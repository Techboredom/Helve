use std::collections::HashMap;

use common::{ModelsAccess, PodAccess, PodCredential, PodInfo, Role};
use k8s_openapi::api::core::v1::Service;
use kube::api::{Api, ListParams};
use sqlx::PgPool;

use crate::auth::CurrentUser;
use crate::models_proxy;
use crate::state::AppState;

/// Reads a Service into the address the Pods tab shows. `None` when it
/// exposes no port, since there would be nothing to connect to.
///
/// A `LoadBalancer` whose IP is still pending yields `external_url: None`
/// rather than a half-formed URL — there genuinely is nothing to click yet,
/// and the in-cluster address below still works in the meantime.
fn access_from_service(svc: &Service, namespace: &str) -> Option<PodAccess> {
    let name = svc.metadata.name.as_ref()?;
    let spec = svc.spec.as_ref()?;
    let port = spec.ports.as_ref()?.first()?.port;
    let external_url = if spec.type_.as_deref() == Some("LoadBalancer") {
        svc.status
            .as_ref()
            .and_then(|s| s.load_balancer.as_ref())
            .and_then(|lb| lb.ingress.as_ref())
            .and_then(|ingress| ingress.first())
            // A cloud load balancer publishes a hostname instead of an IP;
            // either is equally usable here.
            .and_then(|i| i.ip.clone().or_else(|| i.hostname.clone()))
            .map(|host| format!("http://{host}:{port}"))
    } else {
        None
    };
    Some(PodAccess { internal: format!("{name}.{namespace}.svc.cluster.local:{port}"), external_url })
}

/// Filters `pods` down to what `user` is allowed to see — everything, for an
/// admin; only pods they launched themselves, for everyone else — and
/// attaches each visible pod's stored credential (if its template generated one).
pub async fn visible_to(pods: Vec<PodInfo>, user: &CurrentUser, state: &AppState) -> Vec<PodInfo> {
    let mut visible: Vec<PodInfo> = if user.role == Role::Admin {
        pods
    } else {
        pods.into_iter().filter(|p| p.owner.as_deref() == Some(user.username.as_str())).collect()
    };

    let deployment_names: Vec<String> = visible.iter().filter_map(|p| p.deployment_name.clone()).collect();
    if deployment_names.is_empty() {
        return visible;
    }

    let secrets = match load_secrets(&state.pg, &deployment_names).await {
        Ok(secrets) => secrets,
        Err(err) => {
            tracing::warn!(error = %err, "failed to load deployment secrets");
            return visible;
        }
    };
    // One list for the whole namespace rather than a get per pod: this runs
    // on every pod-list request, and an N+1 against the apiserver here would
    // scale with how many things are running.
    let access = service_access(state).await;
    for pod in &mut visible {
        if let Some(name) = &pod.deployment_name {
            if let Some(stored) = secrets.get(name) {
                pod.credential = stored.credential.clone();
                pod.proxy_path = stored.proxy_enabled.then(|| state.proxy_url(name));
                pod.models_access = models_access_from(&stored.proxy_token, pod.owner.as_deref(), &stored.engine_slug, &stored.model_slug);
            }
            pod.access = access.get(name).cloned();
        }
    }
    visible
}

/// Every Service in the watched namespace, keyed by name — which is also
/// the deployment name it belongs to (see `create_deployment` in
/// deployments.rs, which names them identically).
///
/// A failure here is logged and treated as "no addresses known": the Pods
/// tab is still useful without them, so it shouldn't fail outright because
/// of a Service lookup.
async fn service_access(state: &AppState) -> HashMap<String, PodAccess> {
    let services: Api<Service> = Api::namespaced(state.client.clone(), &state.namespace);
    match services.list(&ListParams::default()).await {
        Ok(list) => list
            .into_iter()
            .filter_map(|svc| {
                let name = svc.metadata.name.clone()?;
                Some((name, access_from_service(&svc, &state.namespace)?))
            })
            .collect(),
        Err(err) => {
            tracing::warn!(error = %err, "failed to list services; pods will show no direct address");
            HashMap::new()
        }
    }
}

/// Whether `user` is allowed to see `pod` at all — used to filter individual
/// live Upsert events on the WebSocket.
pub fn can_see(pod: &PodInfo, user: &CurrentUser) -> bool {
    user.role == Role::Admin || pod.owner.as_deref() == Some(user.username.as_str())
}

/// Attaches `pod`'s stored credential (and proxy path, if proxy-enabled) in
/// place. Used for single live Upsert events, where a full batch lookup
/// would be overkill.
pub async fn attach_credential(pod: &mut PodInfo, state: &AppState) {
    let Some(name) = &pod.deployment_name else { return };
    if let Ok(Some((env_key, value, proxy_enabled, proxy_token, engine_slug, model_slug))) = sqlx::query_as::<
        _,
        (Option<String>, Option<String>, bool, Option<String>, Option<String>, Option<String>),
    >(
        "SELECT env_key, secret_value, proxy_enabled, proxy_token, engine_slug, model_slug \
         FROM deployment_secrets WHERE deployment_name = $1",
    )
    .bind(name)
    .fetch_optional(&state.pg)
    .await
    {
        pod.credential = credential_from(env_key, value);
        pod.proxy_path = proxy_enabled.then(|| state.proxy_url(name));
        pod.models_access = models_access_from(&proxy_token, pod.owner.as_deref(), &engine_slug, &model_slug);
    }
    // Also here, not just in the batch path above: a live Upsert replaces
    // the whole row in the UI, so leaving this unset would make the address
    // disappear the moment anything about the pod changed.
    let services: Api<Service> = Api::namespaced(state.client.clone(), &state.namespace);
    if let Ok(svc) = services.get(name).await {
        pod.access = access_from_service(&svc, &state.namespace);
    }
}

struct StoredSecret {
    credential: Option<PodCredential>,
    proxy_enabled: bool,
    proxy_token: Option<String>,
    engine_slug: Option<String>,
    model_slug: Option<String>,
}

fn credential_from(env_key: Option<String>, value: Option<String>) -> Option<PodCredential> {
    match (env_key, value) {
        (Some(env_key), Some(value)) => Some(PodCredential { env_key, value }),
        _ => None,
    }
}

/// Builds `PodInfo::models_access` from a `deployment_secrets` row —
/// `None` unless the deployment actually has a `proxy_token`, an owner (it
/// always does, `deployment_secrets.owner_username` is `NOT NULL`, but
/// `pod.owner` is looked up separately from the Kubernetes label and could
/// in principle be missing), and an `engine_slug`.
fn models_access_from(
    proxy_token: &Option<String>,
    owner: Option<&str>,
    engine_slug: &Option<String>,
    model_slug: &Option<String>,
) -> Option<ModelsAccess> {
    let token = proxy_token.clone()?;
    let owner = owner?;
    let engine_slug = engine_slug.as_deref()?;
    Some(ModelsAccess { url: models_proxy::models_url(owner, engine_slug, model_slug.as_deref()), token })
}

/// (deployment_name, env_key, secret_value, proxy_enabled, proxy_token, engine_slug, model_slug)
type SecretRow = (String, Option<String>, Option<String>, bool, Option<String>, Option<String>, Option<String>);

async fn load_secrets(pg: &PgPool, deployment_names: &[String]) -> Result<HashMap<String, StoredSecret>, sqlx::Error> {
    let rows: Vec<SecretRow> = sqlx::query_as(
        "SELECT deployment_name, env_key, secret_value, proxy_enabled, proxy_token, engine_slug, model_slug \
         FROM deployment_secrets WHERE deployment_name = ANY($1)",
    )
    .bind(deployment_names)
    .fetch_all(pg)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(name, env_key, value, proxy_enabled, proxy_token, engine_slug, model_slug)| {
            (name, StoredSecret { credential: credential_from(env_key, value), proxy_enabled, proxy_token, engine_slug, model_slug })
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::api::core::v1::{
        LoadBalancerIngress, LoadBalancerStatus, ServicePort, ServiceSpec, ServiceStatus,
    };
    use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;

    fn service(type_: &str, ports: Vec<i32>, ingress: Option<LoadBalancerIngress>) -> Service {
        Service {
            metadata: ObjectMeta { name: Some("vllm".to_string()), ..Default::default() },
            spec: Some(ServiceSpec {
                type_: Some(type_.to_string()),
                ports: Some(
                    ports.into_iter().map(|port| ServicePort { port, ..Default::default() }).collect(),
                ),
                ..Default::default()
            }),
            status: ingress.map(|i| ServiceStatus {
                load_balancer: Some(LoadBalancerStatus { ingress: Some(vec![i]) }),
                ..Default::default()
            }),
        }
    }

    #[test]
    fn cluster_ip_has_an_internal_address_and_no_link() {
        let access = access_from_service(&service("ClusterIP", vec![8000], None), "helve").unwrap();
        assert_eq!(access.internal, "vllm.helve.svc.cluster.local:8000");
        assert_eq!(access.external_url, None);
    }

    #[test]
    fn load_balancer_with_an_ip_gets_a_link() {
        let ingress = LoadBalancerIngress { ip: Some("192.168.10.7".to_string()), ..Default::default() };
        let access = access_from_service(&service("LoadBalancer", vec![11434], Some(ingress)), "helve").unwrap();
        assert_eq!(access.external_url.as_deref(), Some("http://192.168.10.7:11434"));
        // The in-cluster address is still offered: that's what another pod uses.
        assert_eq!(access.internal, "vllm.helve.svc.cluster.local:11434");
    }

    #[test]
    fn load_balancer_still_awaiting_an_address_offers_no_link() {
        // MetalLB (or a cloud controller) hasn't assigned one yet. Half a URL
        // would be worse than none, and the internal address still works.
        let access = access_from_service(&service("LoadBalancer", vec![80], None), "helve").unwrap();
        assert_eq!(access.external_url, None);
        assert_eq!(access.internal, "vllm.helve.svc.cluster.local:80");
    }

    #[test]
    fn a_load_balancer_hostname_is_used_when_there_is_no_ip() {
        let ingress =
            LoadBalancerIngress { hostname: Some("lb.example.com".to_string()), ..Default::default() };
        let access = access_from_service(&service("LoadBalancer", vec![443], Some(ingress)), "helve").unwrap();
        assert_eq!(access.external_url.as_deref(), Some("http://lb.example.com:443"));
    }

    #[test]
    fn a_service_with_no_ports_has_no_address_at_all() {
        assert!(access_from_service(&service("ClusterIP", vec![], None), "helve").is_none());
    }
}
