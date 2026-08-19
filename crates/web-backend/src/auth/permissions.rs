//! Effective permissions: what a user may do, computed from their groups.

use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;

/// One permission grant, already resolved for a specific user.
///
/// Scope is expressed by the two optional fields, matching the storage shape in
/// `group_permissions`:
///
/// - both `None` — global, every resource of every type
/// - `resource_type` set, `resource_id` `None` — every resource of that type
/// - both set — exactly that one resource
///
/// Sent to clients inside the access token so a UI can hide controls the user
/// cannot use. That is a convenience, never a control: the server decides what
/// is permitted, and a client that ignores this learns nothing it could not
/// have learned by trying.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, sqlx::FromRow)]
#[serde(rename_all = "camelCase")]
pub struct PermissionGrant {
    /// The permission key, e.g. `connectors.control`.
    pub key: String,
    /// The kind of resource this grant is limited to, if any.
    pub resource_type: Option<String>,
    /// The single resource this grant is limited to, if any.
    pub resource_id: Option<String>,
}

/// Every grant reaching a user through any of their groups.
///
/// Grants are additive and there is no deny rule, so the union across a user's
/// groups is the whole answer. `DISTINCT` matters: two groups granting the same
/// thing is normal, and the caller should see one grant rather than a duplicate
/// that would then be duplicated again inside a token.
///
/// A deactivated user is not special-cased here — callers must check
/// `is_active` before issuing anything. Keeping that check at the call site
/// means this function answers exactly one question.
pub async fn effective_permissions(
    pool: &SqlitePool,
    user_id: &str,
) -> Result<Vec<PermissionGrant>, sqlx::Error> {
    sqlx::query_as::<_, PermissionGrant>(
        r#"
        SELECT DISTINCT
            gp.permission_key AS key,
            gp.resource_type   AS resource_type,
            gp.resource_id     AS resource_id
        FROM user_groups ug
        JOIN group_permissions gp ON gp.group_id = ug.group_id
        WHERE ug.user_id = ?
        ORDER BY gp.permission_key, gp.resource_type, gp.resource_id
        "#,
    )
    .bind(user_id)
    .fetch_all(pool)
    .await
}
