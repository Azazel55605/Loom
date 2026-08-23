//! Authenticated connector-status WebSocket.
//!
//! Browsers cannot set an `Authorization` header during the WebSocket
//! handshake, so this one route accepts the short-lived access token as a
//! percent-encoded `token` query parameter. Refresh tokens never belong here.
//! See `docs/adr/0012-connector-status-push.md` for the trade-off.

use std::collections::HashSet;

use axum::extract::ws::rejection::WebSocketUpgradeRejection;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::auth::extract::{has_permission, ConnectorsView, Permission};
use crate::auth::tokens::verify_access_token;
use crate::connectors::runtime::{ConnectorStatusSnapshot, ConnectorStatusUpdate};
use crate::error::ErrorBody;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
pub(super) struct SocketAuth {
    #[serde(default)]
    token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ClientMessage {
    Subscribe { instance_ids: Vec<Uuid> },
    Unsubscribe { instance_ids: Vec<Uuid> },
}

#[derive(Debug, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ServerMessage {
    Status {
        instance_id: Uuid,
        #[serde(flatten)]
        snapshot: ConnectorStatusSnapshot,
    },
}

#[derive(Debug, Default)]
struct Subscriptions(HashSet<Uuid>);

impl Subscriptions {
    fn apply(&mut self, message: ClientMessage) {
        match message {
            ClientMessage::Subscribe { instance_ids } => self.0.extend(instance_ids),
            ClientMessage::Unsubscribe { instance_ids } => {
                for id in instance_ids {
                    self.0.remove(&id);
                }
            }
        }
    }

    fn wants(&self, update: &ConnectorStatusUpdate) -> bool {
        self.0.contains(&update.instance_id)
    }
}

/// `GET /ws`
///
/// Requires a valid access token and a global `connectors.view` grant. The
/// reverse proxy exposes this as `/api/ws`, consistent with the HTTP API.
pub(super) async fn connector_status_socket(
    State(state): State<AppState>,
    Query(auth): Query<SocketAuth>,
    ws: Result<WebSocketUpgrade, WebSocketUpgradeRejection>,
) -> Response {
    let Some(token) = auth.token.as_deref().filter(|token| !token.is_empty()) else {
        return ErrorBody::message(StatusCode::UNAUTHORIZED, "missing access token");
    };

    let claims = match verify_access_token(&state.jwt_secret, token) {
        Ok(claims) => claims,
        Err(_) => {
            return ErrorBody::message(StatusCode::UNAUTHORIZED, "invalid or expired access token")
        }
    };

    if !has_permission(&claims, ConnectorsView::KEY, None, None) {
        return ErrorBody::message(
            StatusCode::FORBIDDEN,
            "this connection requires the connectors.view permission",
        );
    }

    let ws = match ws {
        Ok(ws) => ws,
        Err(rejection) => return rejection.into_response(),
    };
    let updates = state.connectors.subscribe_statuses();
    ws.on_upgrade(move |socket| handle_socket(socket, updates))
}

async fn handle_socket(
    mut socket: WebSocket,
    mut updates: broadcast::Receiver<ConnectorStatusUpdate>,
) {
    let mut subscriptions = Subscriptions::default();

    loop {
        tokio::select! {
            incoming = socket.recv() => {
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if let Ok(message) = serde_json::from_str(text.as_str()) {
                            subscriptions.apply(message);
                        }
                    }
                    Some(Ok(Message::Close(_))) | None | Some(Err(_)) => break,
                    Some(Ok(_)) => {}
                }
            }
            update = updates.recv() => {
                let update = match update {
                    Ok(update) => update,
                    // A lagged client will receive the next periodic snapshot;
                    // one slow connection must not affect the poller.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                };

                if !subscriptions.wants(&update) {
                    continue;
                }

                let message = ServerMessage::Status {
                    instance_id: update.instance_id,
                    snapshot: update.snapshot,
                };
                let Ok(serialized) = serde_json::to_string(&message) else {
                    tracing::error!("failed to serialize a connector status update");
                    continue;
                };

                if socket.send(Message::Text(serialized.into())).await.is_err() {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::connectors::runtime::PendingOperation;
    use chrono::Utc;
    use loom_core::connector::ConnectorStatus;

    fn update(id: Uuid) -> ConnectorStatusUpdate {
        ConnectorStatusUpdate {
            instance_id: id,
            snapshot: ConnectorStatusSnapshot {
                status: Some(ConnectorStatus::healthy()),
                status_error: None,
                pending_operation: None,
                diagnosis: None,
            },
        }
    }

    #[test]
    fn subscribed_instances_receive_updates_and_unsubscribed_instances_do_not() {
        let subscribed = Uuid::new_v4();
        let other = Uuid::new_v4();
        let mut subscriptions = Subscriptions::default();

        subscriptions.apply(ClientMessage::Subscribe {
            instance_ids: vec![subscribed],
        });
        assert!(subscriptions.wants(&update(subscribed)));
        assert!(!subscriptions.wants(&update(other)));

        subscriptions.apply(ClientMessage::Unsubscribe {
            instance_ids: vec![subscribed],
        });
        assert!(!subscriptions.wants(&update(subscribed)));
    }

    #[test]
    fn outgoing_status_messages_match_the_documented_wire_shape() {
        let id = Uuid::new_v4();
        let message = ServerMessage::Status {
            instance_id: id,
            snapshot: update(id).snapshot,
        };
        let json = serde_json::to_value(message).expect("serializable update");

        assert_eq!(json["type"], "status");
        assert_eq!(json["instanceId"], id.to_string());
        assert!(json["status"].is_object());
        assert!(json.get("statusError").is_none());
        // Both overlay fields are present as explicit nulls rather than
        // omitted. A client destructures them on every frame, and a key that
        // appears only sometimes is a key that gets read as `undefined` by
        // something that meant to read `null`.
        assert_eq!(json["pendingOperation"], serde_json::Value::Null);
        assert_eq!(json["diagnosis"], serde_json::Value::Null);
    }

    /// The overlay is what makes a restart legible, so its wire shape is worth
    /// pinning: a client renders `actionLabel` verbatim.
    #[test]
    fn a_pending_operation_is_pushed_with_its_label_and_start_time() {
        let id = Uuid::new_v4();
        let started_at = Utc::now();
        let mut snapshot = update(id).snapshot;
        snapshot.pending_operation = Some(PendingOperation {
            action_label: "Restart".to_owned(),
            started_at,
        });
        snapshot.diagnosis = Some("Host `192.0.2.10` is unreachable on port `2375`.".to_owned());

        let json = serde_json::to_value(ServerMessage::Status {
            instance_id: id,
            snapshot,
        })
        .expect("serializable update");

        assert_eq!(json["pendingOperation"]["actionLabel"], "Restart");
        assert_eq!(
            json["pendingOperation"]["startedAt"],
            serde_json::to_value(started_at).expect("rfc 3339")
        );
        assert!(json["diagnosis"]
            .as_str()
            .is_some_and(|d| d.contains("2375")));
    }
}
