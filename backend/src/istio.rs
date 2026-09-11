use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::ServiceAccount;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use kube::api::{Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, PostParams};
use serde_json::json;

use crate::error::ApiError;
use crate::resources::OWNER_LABEL;
use crate::state::{AppState, IstioConfig};

/// The bare SPIFFE-style principal Istio authorizes mTLS peers by — no
/// `spiffe://` prefix, matching how Istio's own examples and internal
/// handling write it.
pub fn principal(trust_domain: &str, namespace: &str, service_account: &str) -> String {
    format!("{trust_domain}/ns/{namespace}/sa/{service_account}")
}

/// Ensures a deterministic, unprivileged ServiceAccount exists for `username`
/// (`owner-<username>`) — reused across every deployment that owner ever
/// launches, purely as the mTLS identity `ensure_authorization_policy` grants
/// access to. Mirrors `deployments::ensure_home_pvc`'s get-then-create shape,
/// tolerating a 409 the same way (a concurrent first-launch race).
pub async fn ensure_owner_service_account(state: &AppState, username: &str) -> Result<String, ApiError> {
    let name = format!("owner-{username}");
    let accounts: Api<ServiceAccount> = Api::namespaced(state.client.clone(), &state.namespace);
    if accounts.get(&name).await.is_ok() {
        return Ok(name);
    }
    let mut labels = BTreeMap::new();
    labels.insert(OWNER_LABEL.to_string(), username.to_string());
    let account = ServiceAccount {
        metadata: ObjectMeta {
            name: Some(name.clone()),
            namespace: Some(state.namespace.clone()),
            labels: Some(labels),
            ..Default::default()
        },
        // No RBAC Role is ever bound to this account — it exists purely as
        // an mTLS identity peg, so there's nothing for a mounted token to
        // grant access to.
        automount_service_account_token: Some(false),
        ..Default::default()
    };
    match accounts.create(&PostParams::default(), &account).await {
        Ok(_) => Ok(name),
        Err(kube::Error::Api(status)) if status.code == 409 => Ok(name),
        Err(err) => Err(err.into()),
    }
}

/// Istio's `AuthorizationPolicy` CRD has no typed Rust binding in this
/// codebase. `from_gvk_with_plural` (not `from_gvk`, which would guess a
/// wrong plural like "authorizationpolicys") is used since there's no live
/// cluster to run API discovery against when this feature is off.
fn authorization_policy_resource() -> ApiResource {
    ApiResource::from_gvk_with_plural(
        &GroupVersionKind::gvk("security.istio.io", "v1", "AuthorizationPolicy"),
        "authorizationpolicies",
    )
}

/// Creates the per-deployment `AuthorizationPolicy` that is this feature's
/// actual enforcement point: scoped via `selector` to just this deployment's
/// pod, it allows inbound traffic only from the Helve backend's own
/// principal (so the reverse proxy keeps working) and this deployment's
/// owner's principal. Every other pod in the namespace — another tenant's —
/// is denied, since Istio never matches an `ALLOW` policy whose rules don't
/// list the caller.
pub async fn ensure_authorization_policy(
    state: &AppState,
    istio: &IstioConfig,
    deployment_name: &str,
    owner_service_account: &str,
) -> Result<(), ApiError> {
    let ar = authorization_policy_resource();
    let backend_principal = principal(&istio.trust_domain, &state.namespace, &istio.backend_service_account);
    let owner_principal = principal(&istio.trust_domain, &state.namespace, owner_service_account);
    let object = DynamicObject::new(deployment_name, &ar).within(&state.namespace).data(json!({
        "spec": {
            "selector": { "matchLabels": { "app": deployment_name } },
            "action": "ALLOW",
            "rules": [{
                "from": [{ "source": { "principals": [backend_principal, owner_principal] } }],
            }],
        }
    }));
    let api: Api<DynamicObject> = Api::namespaced_with(state.client.clone(), &state.namespace, &ar);
    api.create(&PostParams::default(), &object).await?;
    Ok(())
}

/// Deletes a deployment's `AuthorizationPolicy`, tolerating it already being
/// gone — same 404-tolerant shape as `delete_deployment`'s Service cleanup.
pub async fn delete_authorization_policy(state: &AppState, deployment_name: &str) -> Result<(), ApiError> {
    let ar = authorization_policy_resource();
    let api: Api<DynamicObject> = Api::namespaced_with(state.client.clone(), &state.namespace, &ar);
    match api.delete(deployment_name, &DeleteParams::default()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(status)) if status.code == 404 => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn principal_has_no_spiffe_prefix() {
        assert_eq!(principal("cluster.local", "helve", "owner-alice"), "cluster.local/ns/helve/sa/owner-alice");
    }
}
