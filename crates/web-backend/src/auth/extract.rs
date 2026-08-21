//! Authentication and authorization for handlers.
//!
//! Two layers, deliberately separate:
//!
//! - [`AuthenticatedUser`] answers *who is this*, rejecting with 401 when the
//!   answer is nobody. A handler taking it can assume a valid caller.
//! - [`RequirePermission`] additionally answers *may they*, rejecting with 403.
//!
//! The status codes are not interchangeable. 401 means "you are not
//! authenticated, credentials might help"; 403 means "you are authenticated and
//! the answer is still no". Returning 401 for a permission failure makes a
//! client retry the login it already completed.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::Response;

use crate::auth::permissions::PermissionGrant;
use crate::auth::tokens::{verify_access_token, AccessClaims};
use crate::error::ErrorBody;
use crate::routes::auth::bearer_token;
use crate::state::AppState;

/// The verified caller, extracted from the `Authorization` header.
///
/// Carries the access token's claims — id, username, and the grants that were
/// effective when the token was issued. Those grants can be up to the token's
/// 15-minute lifetime out of date; that staleness is the price of not reading
/// the database on every request, and is bounded by that lifetime. A permission
/// change takes effect for a user on their next refresh.
#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub claims: AccessClaims,
}

impl AuthenticatedUser {
    /// The caller's user id.
    pub fn id(&self) -> &str {
        &self.claims.sub
    }

    /// Whether the caller holds `permission` over the named resource.
    ///
    /// Pass `None`/`None` to ask for a global grant. See [`has_permission`] for
    /// the matching rules.
    pub fn can(
        &self,
        permission: &str,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
    ) -> bool {
        has_permission(&self.claims, permission, resource_type, resource_id)
    }

    /// The 403 to return when the caller lacks `permission`, or `None` when
    /// they hold it.
    ///
    /// `Option` rather than `Result` because the "error" is an ordinary
    /// response to hand back, not a failure to propagate — and a `Result` whose
    /// `Err` is a whole HTTP response is both large and misleading. Reads as
    /// one line at the top of resource-scoped handlers, where the resource id
    /// is only known once the path has been parsed:
    ///
    /// ```ignore
    /// if let Some(denied) = caller.deny_unless("connectors.control", ...) {
    ///     return denied;
    /// }
    /// ```
    pub fn deny_unless(
        &self,
        permission: &str,
        resource_type: Option<&str>,
        resource_id: Option<&str>,
    ) -> Option<Response> {
        if self.can(permission, resource_type, resource_id) {
            return None;
        }
        Some(forbidden(permission))
    }
}

/// The 403 body, phrased identically wherever it comes from.
///
/// Naming the missing permission is deliberate. It tells an operator exactly
/// which grant to add, and it reveals nothing an authenticated caller could not
/// already read from their own token.
fn forbidden(permission: &str) -> Response {
    ErrorBody::message(
        StatusCode::FORBIDDEN,
        format!("this action requires the {permission} permission"),
    )
}

impl FromRequestParts<AppState> for AuthenticatedUser {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let Some(token) = bearer_token(&parts.headers) else {
            return Err(ErrorBody::message(
                StatusCode::UNAUTHORIZED,
                "missing or invalid bearer token".to_owned(),
            ));
        };

        // The same verification `/auth/session` performs — algorithm pinned to
        // HS256, signature and expiry checked together.
        match verify_access_token(&state.jwt_secret, token) {
            Ok(claims) => Ok(Self { claims }),
            Err(_) => Err(ErrorBody::message(
                StatusCode::UNAUTHORIZED,
                "invalid or expired access token".to_owned(),
            )),
        }
    }
}

/// Does this caller hold `permission` over the named resource?
///
/// A grant matches when its key matches and its scope covers what was asked
/// for. Scope widens from specific to global:
///
/// | Grant scope | Covers |
/// | --- | --- |
/// | both `None` | every resource of every type, including a global check |
/// | type set, id `None` | every resource of that type |
/// | type and id set | exactly that one resource |
///
/// The asymmetry worth noticing: a **global grant satisfies a scoped check**,
/// because "may control every connector" plainly includes "may control this
/// one". A **scoped grant does not satisfy a global check** — holding
/// `connectors.control` on one connector is not authority over connectors in
/// general, and treating it as such would silently widen every narrow grant
/// into a broad one. That direction is the whole point of scoping, so it is
/// enforced rather than assumed.
pub fn has_permission(
    claims: &AccessClaims,
    permission: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
) -> bool {
    claims
        .permissions
        .iter()
        .any(|grant| grant_covers(grant, permission, resource_type, resource_id))
}

fn grant_covers(
    grant: &PermissionGrant,
    permission: &str,
    resource_type: Option<&str>,
    resource_id: Option<&str>,
) -> bool {
    if grant.key != permission {
        return false;
    }

    match (grant.resource_type.as_deref(), grant.resource_id.as_deref()) {
        // Global grant: covers anything asked of this permission.
        (None, None) => true,

        // Type-wide grant: covers any resource of that type, but not a global
        // check, and not a different type.
        (Some(granted_type), None) => resource_type == Some(granted_type),

        // Single-resource grant: only that exact resource.
        (Some(granted_type), Some(granted_id)) => {
            resource_type == Some(granted_type) && resource_id == Some(granted_id)
        }

        // A grant with an id but no type cannot be interpreted: an id alone
        // does not say what it identifies. The unique index makes this
        // unreachable in practice; treating it as matching nothing fails
        // closed, which is the only safe direction for an authorization check.
        (None, Some(_)) => false,
    }
}

/// A permission key, named by a type so routes can declare it in a signature.
///
/// Zero-sized markers rather than a string parameter because extractors carry
/// no runtime arguments, and const generics cannot hold a `&'static str` on the
/// pinned toolchain. The practical gain is that a route's requirement is part
/// of its handler signature — visible in the type, impossible to forget, and
/// checked before the handler body runs.
pub trait Permission {
    /// The key as registered in the `permissions` table.
    const KEY: &'static str;
}

/// May view connectors and their status.
pub struct ConnectorsView;
impl Permission for ConnectorsView {
    const KEY: &'static str = "connectors.view";
}

/// May execute actions on connectors.
pub struct ConnectorsControl;
impl Permission for ConnectorsControl {
    const KEY: &'static str = "connectors.control";
}

/// May add, reconfigure, and remove connector instances.
///
/// Distinct from [`ConnectorsControl`] on purpose: pressing a connector's
/// buttons and deciding which connectors exist at all are different
/// capabilities, and the second one cannot be scoped to a connector because the
/// connector is what is being created or destroyed.
pub struct ConnectorsManage;
impl Permission for ConnectorsManage {
    const KEY: &'static str = "connectors.manage";
}

/// May create, modify, and deactivate accounts.
pub struct UsersManage;
impl Permission for UsersManage {
    const KEY: &'static str = "users.manage";
}

/// May create and modify groups and their grants.
pub struct GroupsManage;
impl Permission for GroupsManage {
    const KEY: &'static str = "groups.manage";
}

/// A caller proven to hold `P` **globally**.
///
/// Declared in the handler signature, so the requirement is enforced before any
/// handler code runs and cannot be forgotten halfway down a function.
///
/// This checks for a *global* grant, which is the right question for endpoints
/// that are not about one resource — user and group administration, and the
/// permission catalog. Resource-scoped endpoints take [`AuthenticatedUser`] and
/// call [`AuthenticatedUser::require`] once the path has told them which
/// resource is involved.
pub struct RequirePermission<P: Permission> {
    /// The caller, already verified and authorized.
    pub user: AuthenticatedUser,
    _permission: std::marker::PhantomData<P>,
}

impl<P: Permission> RequirePermission<P> {
    /// The caller's user id.
    pub fn id(&self) -> &str {
        self.user.id()
    }
}

impl<P: Permission> FromRequestParts<AppState> for RequirePermission<P> {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let user = AuthenticatedUser::from_request_parts(parts, state).await?;

        if !user.can(P::KEY, None, None) {
            return Err(forbidden(P::KEY));
        }

        Ok(Self {
            user,
            _permission: std::marker::PhantomData,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn claims(grants: Vec<PermissionGrant>) -> AccessClaims {
        AccessClaims {
            sub: "user-id".to_owned(),
            username: "someone".to_owned(),
            permissions: grants,
            exp: 0,
            iat: 0,
        }
    }

    fn grant(key: &str, rt: Option<&str>, rid: Option<&str>) -> PermissionGrant {
        PermissionGrant {
            key: key.to_owned(),
            resource_type: rt.map(str::to_owned),
            resource_id: rid.map(str::to_owned),
        }
    }

    #[test]
    fn a_global_grant_covers_every_check() {
        let c = claims(vec![grant("connectors.control", None, None)]);

        assert!(has_permission(&c, "connectors.control", None, None));
        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            None
        ));
        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("mock")
        ));
        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("anything-else")
        ));
    }

    #[test]
    fn a_scoped_grant_does_not_satisfy_a_global_check() {
        // The load-bearing direction: holding a permission over one connector
        // must never read as holding it over connectors in general.
        let c = claims(vec![grant(
            "connectors.control",
            Some("connector"),
            Some("mock"),
        )]);

        assert!(!has_permission(&c, "connectors.control", None, None));
    }

    #[test]
    fn a_single_resource_grant_covers_only_that_resource() {
        let c = claims(vec![grant(
            "connectors.control",
            Some("connector"),
            Some("mock"),
        )]);

        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("mock")
        ));
        assert!(!has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("other")
        ));
        // Same id, different type: not a match.
        assert!(!has_permission(
            &c,
            "connectors.control",
            Some("dashboard"),
            Some("mock")
        ));
    }

    #[test]
    fn a_type_wide_grant_covers_any_id_of_that_type_but_not_a_global_check() {
        let c = claims(vec![grant("connectors.control", Some("connector"), None)]);

        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("mock")
        ));
        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("other")
        ));
        assert!(has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            None
        ));
        assert!(!has_permission(&c, "connectors.control", None, None));
        assert!(!has_permission(
            &c,
            "connectors.control",
            Some("dashboard"),
            Some("mock")
        ));
    }

    #[test]
    fn a_different_permission_key_never_matches() {
        let c = claims(vec![grant("connectors.view", None, None)]);

        assert!(!has_permission(&c, "connectors.control", None, None));
        assert!(!has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("mock")
        ));
    }

    #[test]
    fn no_grants_means_no_permission() {
        let c = claims(vec![]);
        assert!(!has_permission(&c, "connectors.view", None, None));
    }

    #[test]
    fn an_id_without_a_type_fails_closed() {
        // Uninterpretable, and the unique index makes it unreachable — but an
        // authorization check must never fail open on data it cannot read.
        let c = claims(vec![grant("connectors.control", None, Some("mock"))]);

        assert!(!has_permission(&c, "connectors.control", None, None));
        assert!(!has_permission(
            &c,
            "connectors.control",
            Some("connector"),
            Some("mock")
        ));
    }
}
