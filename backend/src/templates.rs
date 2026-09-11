use axum::extract::{Path, State};
use axum::Json;
use common::{SaveTemplateRequest, TemplateEntry};
use sqlx::types::Json as SqlxJson;
use sqlx::{AssertSqlSafe, FromRow};

use crate::auth::{AdminUser, CurrentUser};
use crate::error::ApiError;
use crate::state::AppState;
use crate::validate;

#[derive(FromRow)]
struct TemplateRow {
    id: i32,
    name: String,
    image: String,
    container_port: Option<i32>,
    cpu_request: String,
    cpu_limit: String,
    memory_request: String,
    memory_limit: String,
    accelerator_type: String,
    accelerator_count: Option<i64>,
    env: SqlxJson<Vec<(String, String)>>,
    args: Vec<String>,
    model: String,
    context_length: Option<i64>,
    quantization: String,
    served_model_name: String,
    gpu_memory_utilization: Option<f64>,
    dtype: String,
    volume_claim_name: String,
    volume_mount_path: String,
    volume_sub_path: String,
    home_mount_path: String,
    inject_identity_files: bool,
    notes: String,
    secret_env_key: Option<String>,
    proxy_enabled: bool,
    strip_prefix: bool,
    public_service: bool,
    readiness_path: String,
    engine_slug: Option<String>,
    api_proxy_enabled: bool,
}

impl From<TemplateRow> for TemplateEntry {
    fn from(row: TemplateRow) -> Self {
        TemplateEntry {
            id: row.id,
            name: row.name,
            image: row.image,
            container_port: row.container_port,
            cpu_request: row.cpu_request,
            cpu_limit: row.cpu_limit,
            memory_request: row.memory_request,
            memory_limit: row.memory_limit,
            accelerator_type: row.accelerator_type,
            accelerator_count: row.accelerator_count,
            env: row.env.0,
            args: row.args,
            model: row.model,
            context_length: row.context_length,
            quantization: row.quantization,
            served_model_name: row.served_model_name,
            gpu_memory_utilization: row.gpu_memory_utilization,
            dtype: row.dtype,
            volume_claim_name: row.volume_claim_name,
            volume_mount_path: row.volume_mount_path,
            volume_sub_path: row.volume_sub_path,
            home_mount_path: row.home_mount_path,
            inject_identity_files: row.inject_identity_files,
            notes: row.notes,
            secret_env_key: row.secret_env_key,
            proxy_enabled: row.proxy_enabled,
            strip_prefix: row.strip_prefix,
            public_service: row.public_service,
            readiness_path: row.readiness_path,
            engine_slug: row.engine_slug,
            api_proxy_enabled: row.api_proxy_enabled,
        }
    }
}

// `SELECT_COLUMNS` is a compile-time constant, never user input, so interpolating it
// into these queries with `AssertSqlSafe` below is not a SQL-injection risk.
const SELECT_COLUMNS: &str = "id, name, image, container_port, cpu_request, cpu_limit, memory_request, \
     memory_limit, accelerator_type, accelerator_count, env, args, model, context_length, quantization, \
     served_model_name, gpu_memory_utilization, dtype, volume_claim_name, volume_mount_path, \
     volume_sub_path, home_mount_path, inject_identity_files, notes, secret_env_key, proxy_enabled, \
     strip_prefix, public_service, readiness_path, engine_slug, api_proxy_enabled";

pub async fn list_templates(
    _user: CurrentUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<TemplateEntry>>, ApiError> {
    let sql = format!("SELECT {SELECT_COLUMNS} FROM templates ORDER BY name");
    let rows: Vec<TemplateRow> = sqlx::query_as(AssertSqlSafe(sql)).fetch_all(&state.pg).await?;
    Ok(Json(rows.into_iter().map(TemplateEntry::from).collect()))
}

pub async fn create_template(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(req): Json<SaveTemplateRequest>,
) -> Result<Json<TemplateEntry>, ApiError> {
    validate_request(&req)?;
    let sql = format!(
        "INSERT INTO templates (name, image, container_port, cpu_request, cpu_limit, memory_request, \
         memory_limit, accelerator_type, accelerator_count, env, args, model, context_length, quantization, \
         served_model_name, gpu_memory_utilization, dtype, volume_claim_name, volume_mount_path, \
         volume_sub_path, home_mount_path, inject_identity_files, notes, secret_env_key, proxy_enabled, \
         strip_prefix, public_service, readiness_path, engine_slug, api_proxy_enabled) \
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16, $17, $18, $19, \
                 $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, $30) \
         RETURNING {SELECT_COLUMNS}"
    );
    let row: TemplateRow = sqlx::query_as(AssertSqlSafe(sql))
        .bind(&req.name)
        .bind(&req.image)
        .bind(req.container_port)
        .bind(&req.cpu_request)
        .bind(&req.cpu_limit)
        .bind(&req.memory_request)
        .bind(&req.memory_limit)
        .bind(&req.accelerator_type)
        .bind(req.accelerator_count)
        .bind(SqlxJson(&req.env))
        .bind(&req.args)
        .bind(&req.model)
        .bind(req.context_length)
        .bind(&req.quantization)
        .bind(&req.served_model_name)
        .bind(req.gpu_memory_utilization)
        .bind(&req.dtype)
        .bind(&req.volume_claim_name)
        .bind(&req.volume_mount_path)
        .bind(&req.volume_sub_path)
        .bind(&req.home_mount_path)
        .bind(req.inject_identity_files)
        .bind(&req.notes)
        .bind(&req.secret_env_key)
        .bind(req.proxy_enabled)
        .bind(req.strip_prefix)
        .bind(req.public_service)
        .bind(&req.readiness_path)
        .bind(&req.engine_slug)
        .bind(req.api_proxy_enabled)
        .fetch_one(&state.pg)
        .await?;
    Ok(Json(row.into()))
}

pub async fn update_template(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(req): Json<SaveTemplateRequest>,
) -> Result<Json<TemplateEntry>, ApiError> {
    validate_request(&req)?;
    let sql = format!(
        "UPDATE templates SET name = $1, image = $2, container_port = $3, cpu_request = $4, cpu_limit = $5, \
         memory_request = $6, memory_limit = $7, accelerator_type = $8, accelerator_count = $9, env = $10, \
         args = $11, model = $12, context_length = $13, quantization = $14, served_model_name = $15, \
         gpu_memory_utilization = $16, dtype = $17, volume_claim_name = $18, volume_mount_path = $19, \
         volume_sub_path = $20, home_mount_path = $21, inject_identity_files = $22, notes = $23, \
         secret_env_key = $24, proxy_enabled = $25, strip_prefix = $26, public_service = $27, \
         readiness_path = $28, engine_slug = $29, api_proxy_enabled = $30 \
         WHERE id = $31 \
         RETURNING {SELECT_COLUMNS}"
    );
    let row: Option<TemplateRow> = sqlx::query_as(AssertSqlSafe(sql))
        .bind(&req.name)
        .bind(&req.image)
        .bind(req.container_port)
        .bind(&req.cpu_request)
        .bind(&req.cpu_limit)
        .bind(&req.memory_request)
        .bind(&req.memory_limit)
        .bind(&req.accelerator_type)
        .bind(req.accelerator_count)
        .bind(SqlxJson(&req.env))
        .bind(&req.args)
        .bind(&req.model)
        .bind(req.context_length)
        .bind(&req.quantization)
        .bind(&req.served_model_name)
        .bind(req.gpu_memory_utilization)
        .bind(&req.dtype)
        .bind(&req.volume_claim_name)
        .bind(&req.volume_mount_path)
        .bind(&req.volume_sub_path)
        .bind(&req.home_mount_path)
        .bind(req.inject_identity_files)
        .bind(&req.notes)
        .bind(&req.secret_env_key)
        .bind(req.proxy_enabled)
        .bind(req.strip_prefix)
        .bind(req.public_service)
        .bind(&req.readiness_path)
        .bind(&req.engine_slug)
        .bind(req.api_proxy_enabled)
        .bind(id)
        .fetch_optional(&state.pg)
        .await?;

    let row = row.ok_or_else(|| ApiError::BadRequest(format!("template {id} not found")))?;
    Ok(Json(row.into()))
}

pub async fn delete_template(_admin: AdminUser, State(state): State<AppState>, Path(id): Path<i32>) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM templates WHERE id = $1").bind(id).execute(&state.pg).await?;
    Ok(())
}

fn validate_request(req: &SaveTemplateRequest) -> Result<(), ApiError> {
    validate::label("name", &req.name, 100)?;
    validate::image_ref(&req.image)?;
    if let Some(port) = req.container_port {
        validate::container_port(port)?;
    }
    for (field, value) in [
        ("cpu_request", &req.cpu_request),
        ("cpu_limit", &req.cpu_limit),
        ("memory_request", &req.memory_request),
        ("memory_limit", &req.memory_limit),
    ] {
        validate::quantity(field, value)?;
    }
    for (key, _) in &req.env {
        if !key.trim().is_empty() {
            validate::env_key(key)?;
        }
    }
    validate::bounded_list("env", &req.env.iter().map(|(_, v)| v.clone()).collect::<Vec<_>>(), 50, 4096)?;
    validate::bounded_list("args", &req.args, 50, 1024)?;
    validate::optional_text("model", &req.model, 500)?;
    validate::optional_text("quantization", &req.quantization, 100)?;
    validate::optional_text("served_model_name", &req.served_model_name, 200)?;
    validate::optional_text("dtype", &req.dtype, 50)?;
    if let Some(n) = req.context_length
        && n <= 0
    {
        return Err(ApiError::BadRequest("context_length: must be positive".to_string()));
    }
    if let Some(f) = req.gpu_memory_utilization {
        validate::fraction("gpu_memory_utilization", f)?;
    }
    validate::volume_mount(&req.volume_claim_name, &req.volume_mount_path)?;
    validate::home_mount_path(&req.home_mount_path)?;
    if req.notes.chars().count() > 2000 {
        return Err(ApiError::BadRequest("notes: must be at most 2000 characters".to_string()));
    }
    if let Some(key) = &req.secret_env_key {
        validate::env_key(key)?;
    }
    if !req.readiness_path.is_empty() {
        validate::http_path("readiness_path", &req.readiness_path)?;
        if req.container_port.is_none() {
            return Err(ApiError::BadRequest("readiness_path requires container_port".to_string()));
        }
    }
    if req.api_proxy_enabled {
        let engine_slug = req
            .engine_slug
            .as_deref()
            .filter(|s| !s.is_empty())
            .ok_or_else(|| ApiError::BadRequest("api_proxy_enabled requires engine_slug".to_string()))?;
        validate::slug("engine_slug", engine_slug)?;
    }
    Ok(())
}
