use axum::extract::{Path, State};
use axum::Json;
use common::{
    CreateUserRequest, GroupInfo, ResetPasswordRequest, Role, SetNodeLabelRequest, SetSupplementalGroupsRequest, SetUidGidRequest,
    UserInfo,
};
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Api, ListParams};
use sqlx::types::Json as SqlxJson;
use sqlx::{AssertSqlSafe, FromRow};

use crate::auth::{hash_password, AdminUser};
use crate::error::ApiError;
use crate::resources::OWNER_LABEL;
use crate::state::AppState;
use crate::validate;

/// A correlated subquery yielding a user's supplemental-groups membership
/// as a JSON array of `{id, name, gid}` objects — reused by every query in
/// this file (and `auth.rs`) that returns a full user row, so
/// `UserInfo::supplemental_groups` is always populated the same way from
/// one definition. A compile-time constant, never user input, so
/// interpolating it into SQL text below (matching `templates.rs`'s
/// `SELECT_COLUMNS` precedent) isn't a SQL-injection risk. Expects the
/// users table to be aliased `u` in whatever statement embeds it — true
/// for a plain `FROM users u`, and also valid Postgres syntax for
/// `INSERT INTO users AS u ... RETURNING` and `UPDATE users AS u ... RETURNING`.
pub(crate) const GROUPS_JSON: &str = "COALESCE((SELECT json_agg(json_build_object('id', g.id, 'name', g.name, 'gid', g.gid) ORDER BY g.name) FROM user_groups ug JOIN groups g ON g.id = ug.group_id WHERE ug.user_id = u.id), '[]'::json)";

#[derive(FromRow)]
struct UserRow {
    id: i32,
    username: String,
    role: String,
    node_label: Option<String>,
    uid: Option<i32>,
    gid: Option<i32>,
    supplemental_groups: SqlxJson<Vec<GroupInfo>>,
    auth_source: String,
}

impl From<UserRow> for UserInfo {
    fn from(row: UserRow) -> Self {
        UserInfo {
            id: row.id,
            username: row.username,
            role: if row.role == "admin" { Role::Admin } else { Role::User },
            node_label: row.node_label,
            uid: row.uid,
            gid: row.gid,
            supplemental_groups: row.supplemental_groups.0,
            auth_source: row.auth_source,
        }
    }
}

pub async fn list_users(_admin: AdminUser, State(state): State<AppState>) -> Result<Json<Vec<UserInfo>>, ApiError> {
    let sql = format!(
        "SELECT u.id, u.username, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups, u.auth_source \
         FROM users u ORDER BY u.username"
    );
    let rows: Vec<UserRow> = sqlx::query_as(AssertSqlSafe(sql)).fetch_all(&state.pg).await?;
    Ok(Json(rows.into_iter().map(UserInfo::from).collect()))
}

pub async fn create_user(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<CreateUserRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    validate::username(&req.username)?;
    validate::password(&req.password)?;

    let password_hash = hash_password(&req.password)?;
    let role_str = if req.role == Role::Admin { "admin" } else { "user" };

    let sql = format!(
        "INSERT INTO users AS u (username, password_hash, role) VALUES ($1, $2, $3) \
         RETURNING id, username, role, node_label, uid, gid, {GROUPS_JSON} AS supplemental_groups, u.auth_source"
    );
    let row: UserRow = sqlx::query_as(AssertSqlSafe(sql))
        .bind(&req.username)
        .bind(&password_hash)
        .bind(role_str)
        .fetch_one(&state.pg)
        .await
        .map_err(|err| match err {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                ApiError::BadRequest(format!("username \"{}\" is already taken", req.username))
            }
            other => ApiError::from(other),
        })?;

    Ok(Json(row.into()))
}

pub async fn delete_user(admin: AdminUser, State(state): State<AppState>, Path(id): Path<i32>) -> Result<(), ApiError> {
    if admin.0.id == id {
        return Err(ApiError::BadRequest("you can't delete your own account".to_string()));
    }

    let username: Option<String> =
        sqlx::query_scalar("SELECT username FROM users WHERE id = $1").bind(id).fetch_optional(&state.pg).await?;
    let Some(username) = username else {
        return Err(ApiError::BadRequest(format!("user {id} not found")));
    };

    // Deleting the row cascades to their sessions, quota override, and
    // group memberships, but says nothing to Kubernetes: their Deployments
    // would keep running, keep consuming the cluster, and become invisible
    // in the UI, since every view is filtered by an owner that no longer
    // exists. Refuse rather than either leaking workloads or silently
    // destroying them — which of those the admin wants is their call to
    // make, explicitly.
    let deployments: Api<Deployment> = Api::namespaced(state.client.clone(), &state.namespace);
    let owned = deployments.list(&ListParams::default().labels(&format!("{OWNER_LABEL}={username}"))).await?;
    if !owned.items.is_empty() {
        let names: Vec<&str> = owned.items.iter().filter_map(|d| d.metadata.name.as_deref()).collect();
        return Err(ApiError::BadRequest(format!(
            "\"{username}\" still has {} running deployment(s): {} — delete them first",
            names.len(),
            names.join(", ")
        )));
    }

    sqlx::query("DELETE FROM users WHERE id = $1").bind(id).execute(&state.pg).await?;
    Ok(())
}

/// An admin can reset any account's password without knowing the old one —
/// the admin role itself is the authorization. All of that account's
/// sessions are invalidated, forcing a fresh login with the new password.
pub async fn reset_password(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<ResetPasswordRequest>,
) -> Result<(), ApiError> {
    validate::password(&req.password)?;
    let password_hash = hash_password(&req.password)?;
    let result = sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2")
        .bind(&password_hash)
        .bind(id)
        .execute(&state.pg)
        .await?;
    if result.rows_affected() == 0 {
        return Err(ApiError::BadRequest(format!("user {id} not found")));
    }
    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&state.pg).await?;
    Ok(())
}

/// Pins (or, with `node_label: None`, unpins) all of a user's future
/// launches to nodes carrying a given "key=value" label — see
/// `deployments::create_deployment`, which reads it off `CurrentUser` at
/// launch time. Existing Deployments are untouched; this only affects new
/// launches, same as every other launch-time-fixed setting in this app.
pub async fn set_node_label(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetNodeLabelRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    if let Some(label) = &req.node_label {
        validate::node_label(label)?;
    }
    let node_label = req.node_label.as_ref().map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    let sql = format!(
        "UPDATE users AS u SET node_label = $1 WHERE u.id = $2 \
         RETURNING id, username, role, node_label, uid, gid, {GROUPS_JSON} AS supplemental_groups, u.auth_source"
    );
    let row: Option<UserRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(&node_label).bind(id).fetch_optional(&state.pg).await?;
    let row = row.ok_or_else(|| ApiError::BadRequest(format!("user {id} not found")))?;
    Ok(Json(row.into()))
}

/// Sets (or, with `uid`/`gid: None`, clears) the UID/GID a user's future
/// launches run their container as — see `deployments::security_context_for`,
/// which reads it off `CurrentUser` at launch time. `uid` and `gid` are
/// independent (either can be set without the other). Existing Deployments
/// are untouched; this only affects new launches, same as `set_node_label`.
pub async fn set_uid_gid(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetUidGidRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    if let Some(uid) = req.uid {
        validate::uid_gid("uid", uid)?;
    }
    if let Some(gid) = req.gid {
        validate::uid_gid("gid", gid)?;
    }
    let sql = format!(
        "UPDATE users AS u SET uid = $1, gid = $2 WHERE u.id = $3 \
         RETURNING id, username, role, node_label, uid, gid, {GROUPS_JSON} AS supplemental_groups, u.auth_source"
    );
    let row: Option<UserRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(req.uid).bind(req.gid).bind(id).fetch_optional(&state.pg).await?;
    let row = row.ok_or_else(|| ApiError::BadRequest(format!("user {id} not found")))?;
    Ok(Json(row.into()))
}

/// Sets (or, with an empty list, clears) which named groups (see
/// `groups.rs`) a user's future launches run their container with — see
/// `deployments::security_context_for`, which reads it off `CurrentUser`
/// at launch time, added to `uid`/`gid` above rather than replacing them.
/// Existing Deployments are untouched; this only affects new launches,
/// same as `set_node_label`/`set_uid_gid`. `req.group_ids` names rows in
/// the groups registry, checked to actually exist before anything is
/// written (same fail-fast-with-a-clear-400 reasoning as the storage
/// mount's claim-existence check in `deployments.rs`) — the alternative,
/// letting `user_groups`'s own foreign key catch it, would only surface as
/// an opaque `ApiError::Sqlx` 503.
pub async fn set_supplemental_groups(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetSupplementalGroupsRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    validate::group_ids(&req.group_ids)?;

    let mut tx = state.pg.begin().await?;

    let exists: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id = $1)").bind(id).fetch_one(&mut *tx).await?;
    if !exists {
        return Err(ApiError::BadRequest(format!("user {id} not found")));
    }

    if !req.group_ids.is_empty() {
        let found: Vec<i32> =
            sqlx::query_scalar("SELECT id FROM groups WHERE id = ANY($1)").bind(&req.group_ids).fetch_all(&mut *tx).await?;
        let missing: Vec<i32> = req.group_ids.iter().filter(|gid| !found.contains(gid)).copied().collect();
        if !missing.is_empty() {
            return Err(ApiError::BadRequest(format!(
                "no such group id(s): {}",
                missing.iter().map(i32::to_string).collect::<Vec<_>>().join(", ")
            )));
        }
    }

    sqlx::query("DELETE FROM user_groups WHERE user_id = $1").bind(id).execute(&mut *tx).await?;
    if !req.group_ids.is_empty() {
        sqlx::query("INSERT INTO user_groups (user_id, group_id) SELECT $1, unnest($2::int[])")
            .bind(id)
            .bind(&req.group_ids)
            .execute(&mut *tx)
            .await?;
    }

    let sql = format!(
        "SELECT u.id, u.username, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups, u.auth_source \
         FROM users u WHERE u.id = $1"
    );
    let row: UserRow = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_one(&mut *tx).await?;

    tx.commit().await?;
    Ok(Json(row.into()))
}
