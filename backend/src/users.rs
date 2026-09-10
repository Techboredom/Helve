use axum::extract::{Path, State};
use axum::Json;
use common::{
    CreateUserRequest, ResetPasswordRequest, Role, SetNodeLabelRequest, SetSupplementalGroupsRequest, SetUidGidRequest, UserInfo,
};
use k8s_openapi::api::apps::v1::Deployment;
use kube::api::{Api, ListParams};
use sqlx::FromRow;

use crate::auth::{hash_password, AdminUser};
use crate::error::ApiError;
use crate::resources::OWNER_LABEL;
use crate::state::AppState;
use crate::validate;

#[derive(FromRow)]
struct UserRow {
    id: i32,
    username: String,
    role: String,
    node_label: Option<String>,
    uid: Option<i32>,
    gid: Option<i32>,
    supplemental_groups: Vec<i32>,
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
            supplemental_groups: row.supplemental_groups,
        }
    }
}

pub async fn list_users(_admin: AdminUser, State(state): State<AppState>) -> Result<Json<Vec<UserInfo>>, ApiError> {
    let rows: Vec<UserRow> =
        sqlx::query_as("SELECT id, username, role, node_label, uid, gid, supplemental_groups FROM users ORDER BY username")
            .fetch_all(&state.pg)
            .await?;
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

    let row: UserRow = sqlx::query_as(
        "INSERT INTO users (username, password_hash, role) VALUES ($1, $2, $3) \
         RETURNING id, username, role, node_label, uid, gid, supplemental_groups",
    )
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

    // Deleting the row cascades to their sessions and quota override, but
    // says nothing to Kubernetes: their Deployments would keep running,
    // keep consuming the cluster, and become invisible in the UI, since
    // every view is filtered by an owner that no longer exists. Refuse
    // rather than either leaking workloads or silently destroying them —
    // which of those the admin wants is their call to make, explicitly.
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
    let row: Option<UserRow> =
        sqlx::query_as("UPDATE users SET node_label = $1 WHERE id = $2 RETURNING id, username, role, node_label")
            .bind(&node_label)
            .bind(id)
            .fetch_optional(&state.pg)
            .await?;
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
    let row: Option<UserRow> = sqlx::query_as(
        "UPDATE users SET uid = $1, gid = $2 WHERE id = $3 \
         RETURNING id, username, role, node_label, uid, gid, supplemental_groups",
    )
    .bind(req.uid)
    .bind(req.gid)
    .bind(id)
    .fetch_optional(&state.pg)
    .await?;
    let row = row.ok_or_else(|| ApiError::BadRequest(format!("user {id} not found")))?;
    Ok(Json(row.into()))
}

/// Sets (or, with an empty list, clears) the extra GIDs a user's future
/// launches run their container with — see
/// `deployments::security_context_for`, which reads it off `CurrentUser`
/// at launch time, added to `uid`/`gid` above rather than replacing them.
/// Existing Deployments are untouched; this only affects new launches,
/// same as `set_node_label`/`set_uid_gid`.
pub async fn set_supplemental_groups(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SetSupplementalGroupsRequest>,
) -> Result<Json<UserInfo>, ApiError> {
    validate::supplemental_groups(&req.supplemental_groups)?;
    let row: Option<UserRow> = sqlx::query_as(
        "UPDATE users SET supplemental_groups = $1 WHERE id = $2 \
         RETURNING id, username, role, node_label, uid, gid, supplemental_groups",
    )
    .bind(&req.supplemental_groups)
    .bind(id)
    .fetch_optional(&state.pg)
    .await?;
    let row = row.ok_or_else(|| ApiError::BadRequest(format!("user {id} not found")))?;
    Ok(Json(row.into()))
}
