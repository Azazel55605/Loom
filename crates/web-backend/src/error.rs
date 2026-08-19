//! The one error shape every failing response uses.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use loom_core::connector::ConnectorError;
use serde::Serialize;

/// Body of every error response: `{"error": "..."}`, plus `connectorError`
/// when the failure came from a connector.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorBody {
    /// Human-readable summary, safe to show to a user.
    pub error: String,
    /// The originating [`ConnectorError`], when there was one, so a client can
    /// branch on the variant instead of parsing prose.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connector_error: Option<ConnectorError>,
}

impl ErrorBody {
    /// A plain message with a status.
    pub fn message(status: StatusCode, error: impl Into<String>) -> Response {
        (
            status,
            Json(Self {
                error: error.into(),
                connector_error: None,
            }),
        )
            .into_response()
    }

    /// A connector failure, carrying the structured error alongside its text.
    pub fn connector(status: StatusCode, error: ConnectorError) -> Response {
        (
            status,
            Json(Self {
                error: error.to_string(),
                connector_error: Some(error),
            }),
        )
            .into_response()
    }
}

/// Reports an unexpected internal failure without leaking its detail.
///
/// The real error goes to the log, where an operator can see it; the client is
/// told only that something broke. Database errors in particular can carry
/// schema and query text, which is not something to hand to an unauthenticated
/// caller.
pub fn internal_error(context: &str, error: impl std::fmt::Display) -> Response {
    tracing::error!(context, %error, "request failed");
    ErrorBody::message(
        StatusCode::INTERNAL_SERVER_ERROR,
        "an internal error occurred".to_owned(),
    )
}
