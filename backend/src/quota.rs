use std::collections::HashMap;

use axum::extract::{Path, State};
use axum::Json;
use common::{MyQuota, QuotaLimits, QuotaSettings, Role, UserQuotaEntry};
use k8s_openapi::api::apps::v1::Deployment;
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use kube::api::{Api, ListParams};
use sqlx::PgPool;

use crate::auth::{AdminUser, CurrentUser};
use crate::error::ApiError;
use crate::resources::{parse_count, parse_cpu_millicores, parse_memory_bytes, OWNER_LABEL};
use crate::state::AppState;
use crate::validate;

pub struct GlobalSettings {
    pub limits: QuotaLimits,
    pub expose_resource_requests: bool,
    pub fixed_cpu_request: Option<String>,
    pub fixed_memory_request: Option<String>,
    pub allow_custom_images: bool,
}

impl Default for GlobalSettings {
    fn default() -> Self {
        // Mirrors the column defaults in 0009_add_quotas.sql/
        // 0013_add_image_catalog_restriction.sql — in particular
        // `expose_resource_requests`/`allow_custom_images` are both true, so
        // falling back here can't silently hide fields or block launches for
        // everyone.
        Self {
            limits: QuotaLimits::default(),
            expose_resource_requests: true,
            fixed_cpu_request: None,
            fixed_memory_request: None,
            allow_custom_images: true,
        }
    }
}

type GlobalSettingsRow = (Option<String>, Option<String>, Option<i32>, bool, Option<String>, Option<String>, bool);

pub async fn load_global_settings(pg: &PgPool) -> Result<GlobalSettings, ApiError> {
    // `fetch_optional`, not `fetch_one`: the singleton row is created by the
    // migration, but if it ever went missing every launch would start failing
    // with a 500 rather than falling back to "no quota configured".
    let row: Option<GlobalSettingsRow> = sqlx::query_as(
        "SELECT cpu_limit, memory_limit, gpu_limit, expose_resource_requests, fixed_cpu_request, fixed_memory_request, \
                allow_custom_images \
         FROM quota_settings WHERE id = 1",
    )
    .fetch_optional(pg)
    .await?;
    let Some(row) = row else {
        tracing::warn!("quota_settings row is missing — falling back to unlimited defaults");
        return Ok(GlobalSettings::default());
    };
    Ok(GlobalSettings {
        limits: QuotaLimits { cpu_limit: row.0, memory_limit: row.1, gpu_limit: row.2 },
        expose_resource_requests: row.3,
        fixed_cpu_request: row.4,
        fixed_memory_request: row.5,
        allow_custom_images: row.6,
    })
}

/// Errors if `user` isn't allowed to launch `image` — only relevant when an
/// admin has set `allow_custom_images = false`; admins are always exempt,
/// same as quota enforcement (this exists to stop a `user` account
/// launching arbitrary images, not to constrain someone who already has
/// unrestricted cluster access via their own kubeconfig regardless of what
/// Helve enforces). Only queries the catalog tables when actually needed.
pub async fn check_image_allowed(state: &AppState, user: &CurrentUser, image: &str, global: &GlobalSettings) -> Result<(), ApiError> {
    if user.role == Role::Admin || global.allow_custom_images {
        return Ok(());
    }
    let cataloged: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM images WHERE image = $1) OR EXISTS(SELECT 1 FROM templates WHERE image = $1)")
            .bind(image)
            .fetch_one(&state.pg)
            .await?;
    if cataloged {
        Ok(())
    } else {
        Err(ApiError::BadRequest(format!(
            "\"{image}\" isn't in the image catalog or an existing template — an admin has restricted launches to cataloged images only"
        )))
    }
}

async fn load_user_override(pg: &PgPool, user_id: i32) -> Result<Option<QuotaLimits>, ApiError> {
    let row: Option<(Option<String>, Option<String>, Option<i32>)> =
        sqlx::query_as("SELECT cpu_limit, memory_limit, gpu_limit FROM user_quotas WHERE user_id = $1")
            .bind(user_id)
            .fetch_optional(pg)
            .await?;
    Ok(row.map(|(cpu_limit, memory_limit, gpu_limit)| QuotaLimits { cpu_limit, memory_limit, gpu_limit }))
}

/// `(limits, is_override, expose_resource_requests)` for `user` — an
/// admin's `limits` always come back unlimited, since admins are exempt
/// from enforcement, but `expose_resource_requests` is a UI setting that
/// still applies to everyone.
/// Takes the already-loaded global settings rather than reading them again,
/// so a request that needs both costs one query instead of two.
async fn effective_quota(
    state: &AppState,
    user: &CurrentUser,
    global: &GlobalSettings,
) -> Result<(QuotaLimits, bool), ApiError> {
    if user.role == Role::Admin {
        return Ok((QuotaLimits::default(), false));
    }
    match load_user_override(&state.pg, user.id).await? {
        Some(over) => Ok((over, true)),
        None => Ok((global.limits.clone(), false)),
    }
}

/// One account's total reserved footprint, in the same limit-based terms the
/// quota itself is expressed in.
#[derive(Default, Clone, Copy, PartialEq, Debug)]
pub struct Usage {
    pub cpu_millicores: i64,
    pub memory_bytes: i64,
    pub gpu_count: i64,
}

/// Sums each owner's footprint from **Deployment specs** — desired state —
/// rather than from running pods.
///
/// Counting observed pods is tempting (the watcher already has them) but is
/// wrong in both directions. Pods don't exist for a second or two after a
/// Deployment is created, so a burst of launches all measure a stale, empty
/// cluster and every one of them passes a quota they collectively blow
/// through. And during a rolling update the old and new pods coexist, so a
/// legitimate launch can be rejected against a footprint twice the real one.
/// A Deployment's spec is authoritative the moment it's written, and is
/// exactly what the user is asking to reserve.
///
/// `exclude_deployment` drops one deployment from the totals, so editing an
/// existing one is judged against what it *would* become rather than being
/// double-counted against what it already is.
fn usage_by_owner(deployments: &[Deployment], exclude_deployment: Option<&str>) -> HashMap<String, Usage> {
    let mut by_owner: HashMap<String, Usage> = HashMap::new();
    for deployment in deployments {
        let name = deployment.metadata.name.as_deref().unwrap_or_default();
        if exclude_deployment == Some(name) {
            continue;
        }
        let Some(owner) = deployment.metadata.labels.as_ref().and_then(|l| l.get(OWNER_LABEL)) else {
            // Not launched through Helve, so not attributable to an account.
            continue;
        };
        let replicas = i64::from(deployment.spec.as_ref().and_then(|s| s.replicas).unwrap_or(1));
        let containers =
            deployment.spec.as_ref().and_then(|s| s.template.spec.as_ref()).map(|s| s.containers.as_slice()).unwrap_or(&[]);

        let entry = by_owner.entry(owner.clone()).or_default();
        for limits in containers.iter().filter_map(|c| c.resources.as_ref()).filter_map(|r| r.limits.as_ref()) {
            for (key, quantity) in limits {
                match key.as_str() {
                    "cpu" => entry.cpu_millicores += parse_cpu_millicores(quantity).unwrap_or(0) * replicas,
                    "memory" => entry.memory_bytes += parse_memory_bytes(quantity).unwrap_or(0) * replicas,
                    // Anything else with a limit is an extended resource —
                    // i.e. an accelerator, whatever the vendor prefix.
                    _ => entry.gpu_count += parse_count(quantity).unwrap_or(0) * replicas,
                }
            }
        }
    }
    by_owner
}

/// Every Deployment in the watched namespace, for quota accounting.
async fn list_deployments(state: &AppState) -> Result<Vec<Deployment>, ApiError> {
    let api: Api<Deployment> = Api::namespaced(state.client.clone(), &state.namespace);
    Ok(api.list(&ListParams::default()).await?.items)
}

async fn usage_for(state: &AppState, username: &str, exclude_deployment: Option<&str>) -> Result<Usage, ApiError> {
    let deployments = list_deployments(state).await?;
    Ok(usage_by_owner(&deployments, exclude_deployment).get(username).copied().unwrap_or_default())
}

fn format_gib(bytes: i64) -> String {
    format!("{:.2}Gi", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// Checked by `deployments::create_deployment`/`update_deployment` before
/// touching the cluster. No-op for admins. `additional_*` is the
/// footprint (already multiplied by replicas) the caller is trying to add;
/// `exclude_deployment` should be the deployment being edited, if any, so
/// its own current usage isn't double-counted against the new proposal.
pub async fn check_quota(
    state: &AppState,
    user: &CurrentUser,
    exclude_deployment: Option<&str>,
    additional_cpu_millicores: i64,
    additional_memory_bytes: i64,
    additional_gpu_count: i64,
) -> Result<(), ApiError> {
    if user.role == Role::Admin {
        return Ok(());
    }
    let global = load_global_settings(&state.pg).await?;
    let (limits, _) = effective_quota(state, user, &global).await?;
    let used = usage_for(state, &user.username, exclude_deployment).await?;

    let total_cpu = used.cpu_millicores + additional_cpu_millicores;
    let total_mem = used.memory_bytes + additional_memory_bytes;
    let total_gpu = used.gpu_count + additional_gpu_count;

    if let Some(limit) = &limits.cpu_limit
        && let Some(limit_millicores) = parse_cpu_millicores(&Quantity(limit.clone()))
        && total_cpu > limit_millicores
    {
        return Err(ApiError::BadRequest(format!(
            "CPU limit quota exceeded: this would use {:.2} cores total, your limit is {limit} cores",
            total_cpu as f64 / 1000.0
        )));
    }
    if let Some(limit) = &limits.memory_limit
        && let Some(limit_bytes) = parse_memory_bytes(&Quantity(limit.clone()))
        && total_mem > limit_bytes
    {
        return Err(ApiError::BadRequest(format!(
            "Memory limit quota exceeded: this would use {} total, your limit is {limit}",
            format_gib(total_mem)
        )));
    }
    if let Some(limit) = limits.gpu_limit
        && total_gpu > i64::from(limit)
    {
        return Err(ApiError::BadRequest(format!(
            "GPU quota exceeded: this would use {total_gpu} total, your limit is {limit}"
        )));
    }
    Ok(())
}

fn validate_limits(limits: &QuotaLimits) -> Result<(), ApiError> {
    if let Some(v) = &limits.cpu_limit {
        validate::quantity("cpu_limit", v)?;
    }
    if let Some(v) = &limits.memory_limit {
        validate::quantity("memory_limit", v)?;
    }
    if let Some(v) = limits.gpu_limit
        && v < 0
    {
        return Err(ApiError::BadRequest("gpu_limit: must not be negative".to_string()));
    }
    Ok(())
}

/// The caller's own effective quota + current usage — backs the Launch tab
/// and the Pods tab's manage panel, both of which need
/// `expose_resource_requests` regardless of role.
pub async fn my_quota(user: CurrentUser, State(state): State<AppState>) -> Result<Json<MyQuota>, ApiError> {
    // Fixed requests are a global-only setting (not per-user), and purely
    // informational here — the frontend never sends them back.
    let global = load_global_settings(&state.pg).await?;
    let (limits, is_override) = effective_quota(&state, &user, &global).await?;
    let expose_resource_requests = global.expose_resource_requests;
    // Effective, not raw: an admin is exempt from the restriction, same as
    // they're exempt from quota limits above.
    let allow_custom_images = user.role == Role::Admin || global.allow_custom_images;
    let used = usage_for(&state, &user.username, None).await?;
    Ok(Json(MyQuota {
        limits,
        is_override,
        expose_resource_requests,
        fixed_cpu_request: global.fixed_cpu_request,
        fixed_memory_request: global.fixed_memory_request,
        used_cpu_millicores: used.cpu_millicores,
        used_memory_bytes: used.memory_bytes,
        used_gpu_count: used.gpu_count,
        allow_custom_images,
    }))
}

pub async fn get_settings(_admin: AdminUser, State(state): State<AppState>) -> Result<Json<QuotaSettings>, ApiError> {
    let global = load_global_settings(&state.pg).await?;
    Ok(Json(QuotaSettings {
        limits: global.limits,
        expose_resource_requests: global.expose_resource_requests,
        fixed_cpu_request: global.fixed_cpu_request,
        fixed_memory_request: global.fixed_memory_request,
        allow_custom_images: global.allow_custom_images,
    }))
}

pub async fn update_settings(
    _admin: AdminUser,
    State(state): State<AppState>,
    Json(mut req): Json<QuotaSettings>,
) -> Result<Json<QuotaSettings>, ApiError> {
    // A cleared field arrives as "" from the form; store it as NULL so it
    // reads back as "unset" rather than as an unparseable quantity.
    req.limits.cpu_limit = crate::deployments::normalize_quantity(req.limits.cpu_limit.take());
    req.limits.memory_limit = crate::deployments::normalize_quantity(req.limits.memory_limit.take());
    req.fixed_cpu_request = crate::deployments::normalize_quantity(req.fixed_cpu_request.take());
    req.fixed_memory_request = crate::deployments::normalize_quantity(req.fixed_memory_request.take());
    validate_limits(&req.limits)?;
    if let Some(v) = &req.fixed_cpu_request {
        validate::quantity("fixed_cpu_request", v)?;
    }
    if let Some(v) = &req.fixed_memory_request {
        validate::quantity("fixed_memory_request", v)?;
    }
    sqlx::query(
        "UPDATE quota_settings SET cpu_limit = $1, memory_limit = $2, gpu_limit = $3, \
         expose_resource_requests = $4, fixed_cpu_request = $5, fixed_memory_request = $6, \
         allow_custom_images = $7 WHERE id = 1",
    )
    .bind(&req.limits.cpu_limit)
    .bind(&req.limits.memory_limit)
    .bind(req.limits.gpu_limit)
    .bind(req.expose_resource_requests)
    .bind(&req.fixed_cpu_request)
    .bind(&req.fixed_memory_request)
    .bind(req.allow_custom_images)
    .execute(&state.pg)
    .await?;
    Ok(Json(req))
}

pub async fn list_user_quotas(
    _admin: AdminUser,
    State(state): State<AppState>,
) -> Result<Json<Vec<UserQuotaEntry>>, ApiError> {
    type OverrideRow = (i32, Option<String>, Option<String>, Option<i32>);

    let users: Vec<(i32, String)> = sqlx::query_as("SELECT id, username FROM users ORDER BY username").fetch_all(&state.pg).await?;
    let override_rows: Vec<OverrideRow> =
        sqlx::query_as("SELECT user_id, cpu_limit, memory_limit, gpu_limit FROM user_quotas").fetch_all(&state.pg).await?;
    let overrides: HashMap<i32, QuotaLimits> = override_rows
        .into_iter()
        .map(|(id, cpu_limit, memory_limit, gpu_limit)| (id, QuotaLimits { cpu_limit, memory_limit, gpu_limit }))
        .collect();

    let deployments = list_deployments(&state).await?;
    let usage = usage_by_owner(&deployments, None);
    let entries = users
        .into_iter()
        .map(|(id, username)| {
            let used = usage.get(&username).copied().unwrap_or_default();
            UserQuotaEntry {
                quota_override: overrides.get(&id).cloned(),
                user_id: id,
                username,
                used_cpu_millicores: used.cpu_millicores,
                used_memory_bytes: used.memory_bytes,
                used_gpu_count: used.gpu_count,
            }
        })
        .collect();
    Ok(Json(entries))
}

pub async fn set_user_quota(
    _admin: AdminUser,
    State(state): State<AppState>,
    Path(id): Path<i32>,
    Json(mut req): Json<QuotaLimits>,
) -> Result<Json<QuotaLimits>, ApiError> {
    req.cpu_limit = crate::deployments::normalize_quantity(req.cpu_limit.take());
    req.memory_limit = crate::deployments::normalize_quantity(req.memory_limit.take());
    validate_limits(&req)?;
    sqlx::query(
        "INSERT INTO user_quotas (user_id, cpu_limit, memory_limit, gpu_limit) VALUES ($1, $2, $3, $4) \
         ON CONFLICT (user_id) DO UPDATE SET \
            cpu_limit = EXCLUDED.cpu_limit, memory_limit = EXCLUDED.memory_limit, gpu_limit = EXCLUDED.gpu_limit",
    )
    .bind(id)
    .bind(&req.cpu_limit)
    .bind(&req.memory_limit)
    .bind(req.gpu_limit)
    .execute(&state.pg)
    .await?;
    Ok(Json(req))
}

pub async fn clear_user_quota(_admin: AdminUser, State(state): State<AppState>, Path(id): Path<i32>) -> Result<(), ApiError> {
    sqlx::query("DELETE FROM user_quotas WHERE user_id = $1").bind(id).execute(&state.pg).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::apps::v1::DeploymentSpec;
    use k8s_openapi::api::core::v1::{Container, PodSpec, PodTemplateSpec, ResourceRequirements};
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
    use std::collections::BTreeMap;

    fn deployment(name: &str, owner: Option<&str>, replicas: i32, limits: &[(&str, &str)]) -> Deployment {
        let limits: BTreeMap<String, Quantity> =
            limits.iter().map(|(k, v)| (k.to_string(), Quantity(v.to_string()))).collect();
        let mut labels = BTreeMap::new();
        if let Some(owner) = owner {
            labels.insert(OWNER_LABEL.to_string(), owner.to_string());
        }
        Deployment {
            metadata: ObjectMeta { name: Some(name.to_string()), labels: Some(labels), ..Default::default() },
            spec: Some(DeploymentSpec {
                replicas: Some(replicas),
                template: PodTemplateSpec {
                    spec: Some(PodSpec {
                        containers: vec![Container {
                            name: name.to_string(),
                            resources: Some(ResourceRequirements {
                                limits: (!limits.is_empty()).then_some(limits),
                                ..Default::default()
                            }),
                            ..Default::default()
                        }],
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                ..Default::default()
            }),
            status: None,
        }
    }

    #[test]
    fn sums_limits_multiplied_by_replicas() {
        let deployments = vec![deployment("a", Some("alice"), 3, &[("cpu", "500m"), ("memory", "1Gi")])];
        let usage = usage_by_owner(&deployments, None);
        let alice = usage.get("alice").unwrap();
        assert_eq!(alice.cpu_millicores, 1500);
        assert_eq!(alice.memory_bytes, 3 * 1024 * 1024 * 1024);
        assert_eq!(alice.gpu_count, 0);
    }

    #[test]
    fn keeps_each_owner_separate() {
        let deployments = vec![
            deployment("a", Some("alice"), 1, &[("cpu", "1")]),
            deployment("b", Some("bob"), 1, &[("cpu", "2")]),
            deployment("c", Some("alice"), 2, &[("cpu", "250m")]),
        ];
        let usage = usage_by_owner(&deployments, None);
        assert_eq!(usage.get("alice").unwrap().cpu_millicores, 1000 + 500);
        assert_eq!(usage.get("bob").unwrap().cpu_millicores, 2000);
    }

    #[test]
    fn counts_any_extended_resource_as_gpu_regardless_of_vendor() {
        let deployments = vec![
            deployment("a", Some("alice"), 2, &[("nvidia.com/gpu", "1")]),
            deployment("b", Some("alice"), 1, &[("amd.com/gpu", "3")]),
        ];
        assert_eq!(usage_by_owner(&deployments, None).get("alice").unwrap().gpu_count, 2 + 3);
    }

    #[test]
    fn ignores_deployments_helve_did_not_launch() {
        // No owner label: someone else's workload in this namespace, not
        // attributable to any account.
        let deployments = vec![deployment("stray", None, 5, &[("cpu", "8")])];
        assert!(usage_by_owner(&deployments, None).is_empty());
    }

    #[test]
    fn excluding_a_deployment_drops_only_that_one() {
        // Editing "a" must be judged against what it would become, not
        // double-counted against what it already is.
        let deployments = vec![
            deployment("a", Some("alice"), 1, &[("cpu", "1")]),
            deployment("b", Some("alice"), 1, &[("cpu", "2")]),
        ];
        assert_eq!(usage_by_owner(&deployments, Some("a")).get("alice").unwrap().cpu_millicores, 2000);
        assert_eq!(usage_by_owner(&deployments, Some("b")).get("alice").unwrap().cpu_millicores, 1000);
        assert_eq!(usage_by_owner(&deployments, None).get("alice").unwrap().cpu_millicores, 3000);
    }

    #[test]
    fn treats_a_missing_replica_count_as_one() {
        // Kubernetes defaults spec.replicas to 1 when it is omitted.
        let mut d = deployment("a", Some("alice"), 1, &[("cpu", "1")]);
        d.spec.as_mut().unwrap().replicas = None;
        assert_eq!(usage_by_owner(&[d], None).get("alice").unwrap().cpu_millicores, 1000);
    }

    #[test]
    fn scaling_to_zero_reserves_nothing() {
        let deployments = vec![deployment("a", Some("alice"), 0, &[("cpu", "8"), ("memory", "64Gi")])];
        assert_eq!(usage_by_owner(&deployments, None).get("alice").copied().unwrap_or_default(), Usage::default());
    }

    #[test]
    fn deployments_without_limits_contribute_nothing() {
        let deployments = vec![deployment("a", Some("alice"), 2, &[])];
        assert_eq!(usage_by_owner(&deployments, None).get("alice").copied().unwrap_or_default(), Usage::default());
    }
}
