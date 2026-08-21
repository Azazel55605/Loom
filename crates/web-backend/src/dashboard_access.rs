//! Per-dashboard ownership and sharing access control.
//!
//! This path is deliberately independent of `AuthenticatedUser`'s RBAC grants.
//! Group permissions are administrator-managed capabilities such as "may
//! control connectors"; a dashboard role is an end-user-managed relationship
//! to one object. Sharing a dashboard must never grant a connector permission,
//! and holding a connector permission must never reveal a dashboard.

use serde::Serialize;
use sqlx::SqlitePool;

/// The effective relationship between one user and one dashboard.
///
/// Declaration order supplies the access ordering used by `at_least`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum DashboardRole {
    Viewer,
    Editor,
    Owner,
}

impl DashboardRole {
    pub fn at_least(self, required: Self) -> bool {
        self >= required
    }

    fn from_share(value: &str) -> Option<Self> {
        match value {
            "view" => Some(Self::Viewer),
            "edit" => Some(Self::Editor),
            _ => None,
        }
    }
}

/// Resolve the caller's highest role from ownership, a direct user share, or
/// any share to a group they belong to.
pub async fn get_dashboard_role(
    pool: &SqlitePool,
    user_id: &str,
    dashboard_id: &str,
) -> Result<Option<DashboardRole>, sqlx::Error> {
    let owner =
        sqlx::query_scalar::<_, String>("SELECT owner_user_id FROM dashboards WHERE id = ?")
            .bind(dashboard_id)
            .fetch_optional(pool)
            .await?;

    let Some(owner) = owner else {
        return Ok(None);
    };
    if owner == user_id {
        return Ok(Some(DashboardRole::Owner));
    }

    let roles = sqlx::query_scalar::<_, String>(
        "SELECT ds.role \
         FROM dashboard_shares ds \
         LEFT JOIN user_groups ug \
           ON ds.target_type = 'group' \
          AND ds.target_id = ug.group_id \
          AND ug.user_id = ? \
         WHERE ds.dashboard_id = ? \
           AND ((ds.target_type = 'user' AND ds.target_id = ?) \
             OR (ds.target_type = 'group' AND ug.user_id IS NOT NULL))",
    )
    .bind(user_id)
    .bind(dashboard_id)
    .bind(user_id)
    .fetch_all(pool)
    .await?;

    Ok(roles
        .iter()
        .filter_map(|role| DashboardRole::from_share(role))
        .max())
}
