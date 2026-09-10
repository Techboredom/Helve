use axum::extract::{Path, State};
use axum::Json;
use common::{GroupInfo, SaveGroupRequest};
use sqlx::FromRow;

use crate::auth::AdminUser;
use crate::error::ApiError;
use crate::state::AppState;
use crate::validate;

#[derive(FromRow)]
struct GroupRow {
    id: i32,
    name: String,
    gid: i32,
}

impl From<GroupRow> for GroupInfo {
    fn from(row: GroupRow) -> Self {
        GroupInfo { id: row.id, name: row.name, gid: row.gid }
    }
}

/// Admin-only, unlike GET /api/pvcs (visible to any logged-in user to fill
/// a Launch-form datalist) — this registry only ever surfaces in the
/// admin-only Users tab's group picker and the identity-files injection
/// this backs, so there's no non-admin caller that needs it.
pub async fn list_groups(_admin: AdminUser, State(state): State<AppState>) -> Result<Json<Vec<GroupInfo>>, ApiError> {
    let rows: Vec<GroupRow> = sqlx::query_as("SELECT id, name, gid FROM groups ORDER BY name").fetch_all(&state.pg).await?;
    Ok(Json(rows.into_iter().map(GroupInfo::from).collect()))
}

pub async fn create_group(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<SaveGroupRequest>,
) -> Result<Json<GroupInfo>, ApiError> {
    validate::group_name(&req.name)?;
    validate::uid_gid("gid", req.gid)?;
    let row: GroupRow = sqlx::query_as("INSERT INTO groups (name, gid) VALUES ($1, $2) RETURNING id, name, gid")
        .bind(&req.name)
        .bind(req.gid)
        .fetch_one(&state.pg)
        .await
        .map_err(|err| match &err {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                ApiError::BadRequest(format!("a group named \"{}\" or with GID {} already exists", req.name, req.gid))
            }
            _ => ApiError::from(err),
        })?;
    Ok(Json(row.into()))
}

pub async fn update_group(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SaveGroupRequest>,
) -> Result<Json<GroupInfo>, ApiError> {
    validate::group_name(&req.name)?;
    validate::uid_gid("gid", req.gid)?;
    let row: Option<GroupRow> = sqlx::query_as("UPDATE groups SET name = $1, gid = $2 WHERE id = $3 RETURNING id, name, gid")
        .bind(&req.name)
        .bind(req.gid)
        .bind(id)
        .fetch_optional(&state.pg)
        .await
        .map_err(|err| match &err {
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => {
                ApiError::BadRequest(format!("a group named \"{}\" or with GID {} already exists", req.name, req.gid))
            }
            _ => ApiError::from(err),
        })?;
    let row = row.ok_or_else(|| ApiError::BadRequest(format!("group {id} not found")))?;
    Ok(Json(row.into()))
}

/// `ON DELETE RESTRICT` on `user_groups.group_id` means this fails at the
/// database if any user is still assigned this group — surfaced as a clean
/// 400 here rather than the generic 503 ApiError::Sqlx would otherwise
/// produce, same fail-fast-with-a-clear-reason reasoning as everywhere else
/// in this file.
pub async fn delete_group(_admin: AdminUser, State(state): State<AppState>, Path(id): Path<i32>) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM groups WHERE id = $1").bind(id).execute(&state.pg).await.map_err(|err| match &err {
        sqlx::Error::Database(db_err) if db_err.is_foreign_key_violation() => ApiError::BadRequest(
            "this group is still assigned to at least one user — remove it from their supplemental groups first".to_string(),
        ),
        _ => ApiError::from(err),
    })?;
    Ok(())
}
