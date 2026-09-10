use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Basic resource and status info for a single pod, aggregated across its containers.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PodInfo {
    pub name: String,
    pub namespace: String,
    pub node: Option<String>,
    pub phase: String,
    pub ready_containers: u32,
    pub total_containers: u32,
    pub restarts: i32,
    /// RFC3339 pod start time; the frontend derives "age" from this at render time.
    pub start_time: Option<String>,
    pub containers: Vec<ContainerStatusInfo>,
    pub cpu_request_millicores: Option<i64>,
    pub cpu_limit_millicores: Option<i64>,
    pub memory_request_bytes: Option<i64>,
    pub memory_limit_bytes: Option<i64>,
    /// e.g. "nvidia.com/gpu" -> 1, "amd.com/gpu" -> 2, summed across containers.
    pub accelerators: BTreeMap<String, i64>,
    /// Username of whoever launched this pod via the Launch tab, if any
    /// (pods that predate this feature, or weren't launched through Helve,
    /// have no owner label and so show `None`).
    pub owner: Option<String>,
    /// The stable Deployment name (the `app` label), unlike the pod's own
    /// name which gets a random suffix and changes across restarts.
    pub deployment_name: Option<String>,
    /// The auto-generated login credential for this instance (JupyterLab
    /// token, RStudio password, vLLM API key, ...), if its template has one.
    /// Only populated for pods the requester is allowed to see.
    pub credential: Option<PodCredential>,
    /// If its template is proxy-enabled, the URL that opens it through
    /// Helve itself with the credential already injected — no login
    /// prompt, no public IP. A full origin
    /// (`https://<deployment-name>.<proxy base domain>/`) when
    /// per-deployment proxy origins are configured, otherwise the legacy
    /// root-relative `/proxy/<deployment-name>/`.
    pub proxy_path: Option<String>,
    /// How to reach this deployment's own Service, if it has one. Distinct
    /// from `proxy_path`: that goes through Helve and requires an Helve
    /// session, which is right for a browser but unusable for a program
    /// (a coding tool pointed at vLLM's OpenAI-compatible API can't log
    /// in). This is the direct address.
    pub access: Option<PodAccess>,
}

/// Where a deployment's Service can actually be reached.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodAccess {
    /// In-cluster DNS address (`<service>.<namespace>.svc.cluster.local:<port>`).
    /// Always present when a Service exists, but only resolvable from
    /// inside the cluster — never a browser link.
    pub internal: String,
    /// Browser-reachable `http://<external-ip>:<port>`, present only for a
    /// `LoadBalancer` Service that has actually been assigned an address.
    /// A `LoadBalancer` still pending an IP yields `None`, which is
    /// correct: there is nothing to click yet.
    pub external_url: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodCredential {
    pub env_key: String,
    pub value: String,
}

/// Per-container status, surfaced so the UI can explain *why* a pod isn't healthy
/// (waiting/terminated reason + message) without a separate round trip.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ContainerStatusInfo {
    pub name: String,
    pub image: String,
    pub ready: bool,
    pub restart_count: i32,
    /// "running" | "waiting" | "terminated" | "unknown"
    pub state: String,
    /// e.g. "CrashLoopBackOff", "ImagePullBackOff", "OOMKilled".
    pub reason: Option<String>,
    pub message: Option<String>,
    pub exit_code: Option<i32>,
}

/// Messages sent from backend to frontend over the pods WebSocket.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PodEvent {
    /// Full state, sent once when a client connects.
    Snapshot { pods: Vec<PodInfo> },
    /// A pod was added or changed.
    Upsert { pod: Box<PodInfo> },
    /// A pod was deleted, identified by name (unique within the watched namespace).
    Delete { name: String },
}

/// A container image available to pick from in the "create deployment" form,
/// backed by a row in the `images` Postgres table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ImageEntry {
    pub id: i32,
    pub name: String,
    pub image: String,
    pub description: String,
}

/// Submitted by the Images admin tab to create or update a catalog entry.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveImageRequest {
    pub name: String,
    pub image: String,
    pub description: String,
}

/// Submitted by the "create deployment" form. There's no user-chosen name —
/// the backend generates one (`<username>-<instance-type>-<random>`, see
/// deployments.rs::create_deployment) so launching never requires picking
/// something unique yourself.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateDeploymentRequest {
    /// Name of the template this was launched from, if any (`None` for a
    /// Custom launch) — used both for `launch_log` and as the auto-generated
    /// name's "instance type" segment (falling back to a slug of `image`
    /// for a Custom launch, which has no template name to use instead).
    #[serde(default)]
    pub template_name: Option<String>,
    pub image: String,
    pub replicas: i32,
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub memory_request: Option<String>,
    pub memory_limit: Option<String>,
    /// Accelerator resource name, e.g. "nvidia.com/gpu" or "amd.com/gpu".
    pub accelerator_type: Option<String>,
    pub accelerator_count: Option<i64>,
    /// If set, a `Service` (type LoadBalancer) is also created exposing this
    /// container port, since there's no ingress controller in the cluster yet.
    #[serde(default)]
    pub container_port: Option<i32>,
    /// Extra environment variables. Entries with an empty value are dropped,
    /// so an app's own default behavior (e.g. an auto-generated password
    /// logged at startup) still applies unless a value is explicitly set.
    #[serde(default)]
    pub env: Vec<(String, String)>,
    /// Extra container command-line arguments. `{{name}}` is always
    /// substituted with the deployment's own auto-generated name, and
    /// `{{accelerator_count}}` with `accelerator_count` above (defaulting to
    /// `1` if unset) — e.g. vLLM's `--tensor-parallel-size={{accelerator_count}}`.
    /// `{{model}}`, `{{context_length}}`, `{{quantization}}`,
    /// `{{served_model_name}}`, `{{gpu_memory_utilization}}`, and
    /// `{{dtype}}` (all below) are each substituted the same way *if set* —
    /// but if the corresponding field is unset, the whole line containing
    /// that placeholder is dropped entirely, rather than substituting an
    /// empty value and sending a broken `--flag=` with nothing after the
    /// `=`. This lets a template's args always list an optional flag on its
    /// own line and have it just not appear when that field is left blank.
    #[serde(default)]
    pub args: Vec<String>,
    /// Substituted for `{{model}}` in `args` — see `SaveTemplateRequest::model`.
    /// A Hugging Face model ID and a local path under `volume_mount_path`
    /// below both just substitute in as a plain string; nothing here
    /// distinguishes the two.
    #[serde(default)]
    pub model: Option<String>,
    /// Substituted for `{{context_length}}` in `args` — e.g. vLLM's
    /// `--max-model-len`. A plain token count, not vLLM-specific itself.
    #[serde(default)]
    pub context_length: Option<i64>,
    /// Substituted for `{{quantization}}` in `args` — e.g. vLLM's
    /// `--quantization` (`"awq"`, `"gptq"`, `"fp8"`, ...). Free text: there's
    /// no single fixed set of methods across engines/versions.
    #[serde(default)]
    pub quantization: Option<String>,
    /// Substituted for `{{served_model_name}}` in `args` — the name exposed
    /// via the OpenAI-compatible API, separate from `model` above (which is
    /// often a long, ugly Hugging Face ID).
    #[serde(default)]
    pub served_model_name: Option<String>,
    /// Substituted for `{{gpu_memory_utilization}}` in `args` — the
    /// fraction of GPU memory one instance reserves (vLLM defaults this to
    /// `0.9` on its own if never set). Must be in `(0.0, 1.0]` if set.
    #[serde(default)]
    pub gpu_memory_utilization: Option<f64>,
    /// Substituted for `{{dtype}}` in `args` — e.g. `"float16"`,
    /// `"bfloat16"`, `"auto"`. Free text, same reasoning as `quantization`.
    #[serde(default)]
    pub dtype: Option<String>,
    /// Name of an existing `PersistentVolumeClaim` (provisioned out-of-band
    /// — Helve never creates one itself, e.g. a shared model cache) to
    /// mount into the container. Requires `volume_mount_path`; rejected
    /// with 400 if no such claim exists in the namespace.
    #[serde(default)]
    pub volume_claim_name: Option<String>,
    /// Where to mount `volume_claim_name` inside the container. Required
    /// if `volume_claim_name` is set, meaningless otherwise.
    #[serde(default)]
    pub volume_mount_path: Option<String>,
    /// Mounts only this subdirectory of the claim rather than its root.
    /// Optional even when `volume_claim_name`/`volume_mount_path` are set.
    #[serde(default)]
    pub volume_sub_path: Option<String>,
    /// Where to mount this user's home directory inside the container, e.g.
    /// "/home/jovyan" — independent of `volume_claim_name` above, so both
    /// can be mounted together (a home directory plus a shared model
    /// cache). Rejected with 400 if set but the backend has no home-drive
    /// mode configured (`HOME_DRIVES_HOST_BASE_PATH` or
    /// `HOME_DRIVES_STORAGE_CLASS`) — see backend/src/state.rs::HomeDrives.
    #[serde(default)]
    pub home_mount_path: Option<String>,
    /// If set, the backend generates a random value and sets it as this env
    /// var (overriding any same-keyed entry in `env`), instead of the user
    /// typing one in — e.g. `"JUPYTER_TOKEN"`. Comes from the selected
    /// template's `secret_env_key`.
    #[serde(default)]
    pub generate_secret_for: Option<String>,
    /// If set, the app is also reachable via Helve's own `/proxy/<name>/`
    /// route, which injects the generated credential automatically (if any).
    /// Requires `container_port` to be set. Comes from the selected
    /// template's `proxy_enabled`.
    #[serde(default)]
    pub enable_proxy: bool,
    /// Whether the proxy should forward the full `/proxy/<name>/...` path to
    /// the container as-is (`false`, e.g. JupyterLab's `base_url`) or strip
    /// that prefix first (`true`, e.g. RStudio's `www-root-path`, which only
    /// stamps the prefix onto outgoing redirects/cookies and still expects
    /// requests at the bare path). Only meaningful when `enable_proxy` is
    /// set. Comes from the selected template's `strip_prefix`.
    #[serde(default)]
    pub strip_prefix: bool,
    /// Whether Launch creates a public `LoadBalancer` Service (`true`,
    /// default) or a `ClusterIP`-only one (`false`) — must be `false` for
    /// any app with no auth of its own, since Helve's proxy ownership
    /// check then becomes the only thing gating access. Comes from the
    /// selected template's `public_service`.
    #[serde(default = "default_true")]
    pub public_service: bool,
    /// HTTP path for a `readinessProbe` against `container_port` (e.g.
    /// `"/health"`) — without one, Kubernetes considers the container Ready
    /// the instant its process starts, which for a slow-loading LLM server
    /// means traffic (and the Pods tab's "Ready" status) arrives well
    /// before it can actually answer requests. Empty means no probe.
    /// Requires `container_port`. Comes from the selected template's
    /// `readiness_path`.
    #[serde(default)]
    pub readiness_path: Option<String>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateDeploymentResponse {
    pub name: String,
    pub namespace: String,
    /// Present if `container_port` was set and a matching Service was created.
    pub service_name: Option<String>,
    pub container_port: Option<i32>,
    /// The generated value, if `generate_secret_for` was set.
    pub secret_value: Option<String>,
    /// Present if `enable_proxy` was set — the root-relative path that opens
    /// this deployment through Helve with the credential already injected.
    pub proxy_path: Option<String>,
    /// Whether `service_name` (if any) is a public `LoadBalancer` or a
    /// `ClusterIP`-only Service — the frontend uses this to avoid telling
    /// someone to go check `kubectl get svc` for an external IP that
    /// doesn't exist.
    pub public_service: bool,
}

/// Returned by `POST /api/deployments/{name}/regenerate-secret`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RegenerateSecretResponse {
    pub secret_value: String,
}

/// Current editable state of a running Deployment, returned by `GET
/// /api/deployments/{name}` to pre-fill the Pods tab's manage panel.
/// Image, container port, accelerator, and args are fixed at launch time —
/// changing those is a delete + relaunch, not an edit.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct DeploymentDetail {
    pub name: String,
    pub replicas: i32,
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub memory_request: Option<String>,
    pub memory_limit: Option<String>,
    /// User-editable env vars — excludes the auto-generated secret's entry,
    /// if any (see `generated_secret_key`).
    pub env: Vec<(String, String)>,
    /// The env var key managed by an auto-generated secret (e.g.
    /// `"JUPYTER_TOKEN"`), if this deployment has one. Shown read-only in
    /// the manage panel rather than as an editable row, since its value is
    /// generated server-side and would otherwise be silently overwritten or
    /// blanked by a resubmit of `env`.
    pub generated_secret_key: Option<String>,
}

/// Submitted to `PUT /api/deployments/{name}` to scale and/or update
/// resources and env vars on an existing Deployment.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct UpdateDeploymentRequest {
    pub replicas: i32,
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub memory_request: Option<String>,
    pub memory_limit: Option<String>,
    pub env: Vec<(String, String)>,
}

/// A Kubernetes Event involving a specific pod (scheduling failures, image pull
/// errors, OOM kills, etc. all surface here, often before it's visible any other way).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PodEventInfo {
    /// "Normal" or "Warning".
    pub type_: String,
    pub reason: String,
    pub message: String,
    pub count: i32,
    /// RFC3339 timestamp of the most recent occurrence, if known.
    pub last_seen: Option<String>,
}

/// A workload template (Ollama, JupyterLab, etc.), stored in the `templates`
/// table and managed from the Templates admin tab. Selecting one on the
/// Launch tab pre-fills a `CreateDeploymentRequest` with these values; every
/// field stays editable afterward.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TemplateEntry {
    pub id: i32,
    pub name: String,
    pub image: String,
    pub container_port: Option<i32>,
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub accelerator_type: String,
    pub accelerator_count: Option<i64>,
    /// `(key, value)` pairs. A blank value is scaffolding for the launcher to
    /// fill in — see `CreateDeploymentRequest::env`.
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    /// Substituted for `{{model}}` in `args` at launch time — see
    /// `CreateDeploymentRequest::model`. Empty means this template doesn't
    /// use the placeholder.
    pub model: String,
    /// Substituted for `{{context_length}}` — see `CreateDeploymentRequest::context_length`.
    pub context_length: Option<i64>,
    /// Substituted for `{{quantization}}` — see `CreateDeploymentRequest::quantization`.
    pub quantization: String,
    /// Substituted for `{{served_model_name}}` — see `CreateDeploymentRequest::served_model_name`.
    pub served_model_name: String,
    /// Substituted for `{{gpu_memory_utilization}}` — see `CreateDeploymentRequest::gpu_memory_utilization`.
    pub gpu_memory_utilization: Option<f64>,
    /// Substituted for `{{dtype}}` — see `CreateDeploymentRequest::dtype`.
    pub dtype: String,
    /// Name of an existing PersistentVolumeClaim to mount, or empty for
    /// none. Set together with `volume_mount_path`, or not at all.
    pub volume_claim_name: String,
    pub volume_mount_path: String,
    /// Optional even when the two above are set.
    pub volume_sub_path: String,
    /// See `CreateDeploymentRequest::home_mount_path`. Empty means this
    /// template doesn't use one.
    pub home_mount_path: String,
    pub notes: String,
    /// If set, launching this template generates a random value for this env
    /// var automatically instead of showing it as an editable field — e.g.
    /// JupyterLab's `"JUPYTER_TOKEN"`, RStudio's `"PASSWORD"`.
    pub secret_env_key: Option<String>,
    /// If true, this app is also reachable via Helve's `/proxy/<name>/`
    /// route, with `secret_env_key`'s generated value injected automatically
    /// (if any) — no separate login for that path.
    pub proxy_enabled: bool,
    /// Whether the proxy strips the `/proxy/<name>/` prefix before
    /// forwarding to the container. See `CreateDeploymentRequest::strip_prefix`.
    pub strip_prefix: bool,
    /// Whether Launch creates a public `LoadBalancer` Service (default) or
    /// a `ClusterIP`-only one. Must be `false` for templates with no
    /// `secret_env_key` and no other auth of their own (e.g. RStudio run
    /// with `DISABLE_AUTH=true`), since Helve's own login is then the only
    /// gate.
    pub public_service: bool,
    /// See `CreateDeploymentRequest::readiness_path`. Empty means no probe.
    pub readiness_path: String,
}

/// Submitted by the Templates admin tab to create or update a template.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SaveTemplateRequest {
    pub name: String,
    pub image: String,
    pub container_port: Option<i32>,
    pub cpu_request: String,
    pub cpu_limit: String,
    pub memory_request: String,
    pub memory_limit: String,
    pub accelerator_type: String,
    pub accelerator_count: Option<i64>,
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
    pub model: String,
    pub context_length: Option<i64>,
    pub quantization: String,
    pub served_model_name: String,
    pub gpu_memory_utilization: Option<f64>,
    pub dtype: String,
    pub volume_claim_name: String,
    pub volume_mount_path: String,
    pub volume_sub_path: String,
    pub home_mount_path: String,
    pub notes: String,
    pub secret_env_key: Option<String>,
    pub proxy_enabled: bool,
    pub strip_prefix: bool,
    pub public_service: bool,
    pub readiness_path: String,
}

/// An existing `PersistentVolumeClaim` in the watched namespace, for the
/// Launch/Templates forms' storage-mount fields — `GET /api/pvcs`, any
/// logged-in user (matches the Images catalog's visibility level).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PvcEntry {
    pub name: String,
    /// e.g. "2Ti" — `None` if the claim isn't Bound yet.
    pub capacity: Option<String>,
}

/// The two account classes. Admins can manage templates and accounts; both
/// classes can view pods and launch deployments.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    User,
}

/// The logged-in user, as returned by `GET /api/me`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserInfo {
    pub id: i32,
    pub username: String,
    pub role: Role,
    /// An admin-set "key=value" node label. When present, every Deployment
    /// this user launches gets a matching `nodeSelector`, pinning their
    /// workloads to nodes carrying that label. `None` means unrestricted.
    pub node_label: Option<String>,
    /// Admin-set UID/GID, if any. When present, every Deployment this user
    /// launches gets a matching pod `securityContext`
    /// (`runAsUser`/`runAsGroup`/`fsGroup`), so files it creates on shared
    /// storage are owned by this account instead of whatever the container
    /// image defaults to. `None` means the image's own default.
    pub uid: Option<i32>,
    pub gid: Option<i32>,
}

/// Submitted by the Users admin tab to set or clear a user's node label.
/// `node_label: None` clears it back to unrestricted placement.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SetNodeLabelRequest {
    pub node_label: Option<String>,
}

/// Submitted by the Users admin tab to set or clear a user's UID/GID.
/// Either left `None` clears that half back to the image's own default —
/// they're independent, not all-or-nothing.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SetUidGidRequest {
    pub uid: Option<i32>,
    pub gid: Option<i32>,
}

/// Submitted by the API Tokens admin tab to mint a new token for the
/// calling admin's own account — `name` is just a human label (e.g. "CI
/// automation") to tell tokens apart later, since the raw value itself is
/// never shown again after creation.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CreateApiTokenRequest {
    pub name: String,
}

/// Returned once, by `POST /api/tokens` only — the one and only time the
/// raw token value is ever available. Send it as `Authorization: Bearer
/// <token>` on subsequent API calls instead of logging in for a session
/// cookie; it authenticates as whichever account created it, with that
/// account's own role and permissions.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApiTokenCreated {
    pub id: i32,
    pub name: String,
    pub token: String,
    pub created_at: String,
}

/// A previously-created API token, as listed by `GET /api/tokens` — never
/// the raw value again, only enough to tell it apart and decide whether
/// it's still in use.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ApiTokenEntry {
    pub id: i32,
    pub name: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

/// Submitted by the Users admin tab to create an account.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreateUserRequest {
    pub username: String,
    pub password: String,
    pub role: Role,
}

/// Submitted by the Users admin tab to reset another account's password.
/// Requires no proof of the old one — the admin role is the authorization.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ResetPasswordRequest {
    pub password: String,
}

/// Submitted by the logged-in user themselves to change their own password.
/// Requires `current_password` to match, unlike an admin's reset.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ChangePasswordRequest {
    pub current_password: String,
    pub new_password: String,
}

/// One row of `GET /api/sessions` — a past login. `username` is always
/// present; the frontend hides that column for non-admins the same way
/// `PodInfo::owner` is hidden on the Pods tab, since a `user`-role account's
/// query is already server-side filtered to just their own rows anyway.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SessionLogEntry {
    pub username: String,
    /// RFC3339-ish timestamp string (Postgres's default `timestamptz` text form).
    pub created_at: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
}

/// One row of `GET /api/launches` — a past Launch-tab submission, kept for
/// support/metrics ("who launched JupyterLab with what resources"). Same
/// admin-vs-own visibility split as `SessionLogEntry`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LaunchLogEntry {
    pub username: String,
    pub created_at: String,
    pub deployment_name: String,
    pub template_name: Option<String>,
    pub image: String,
    pub replicas: i32,
    pub cpu_request: Option<String>,
    pub cpu_limit: Option<String>,
    pub memory_request: Option<String>,
    pub memory_limit: Option<String>,
    pub accelerator_type: Option<String>,
    pub accelerator_count: Option<i64>,
    pub container_port: Option<i32>,
    /// Same `(key, value)` shape as `TemplateEntry::env`. Any value matching
    /// the launch's `generate_secret_for` key was redacted before this was
    /// ever written to the database — see `deployments.rs`.
    pub env: Vec<(String, String)>,
    pub args: Vec<String>,
}

/// A CPU/memory/GPU limit triple, `None` meaning unlimited for that
/// dimension. Shared shape for both the global default and a per-user
/// override — see `common::MyQuota`/`UserQuotaEntry`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaLimits {
    pub cpu_limit: Option<String>,
    pub memory_limit: Option<String>,
    pub gpu_limit: Option<i32>,
}

/// The cluster-wide default quota (`GET`/`PUT /api/quota/settings`,
/// admin-only to write). Applies to any user with no override row of their
/// own in `user_quotas`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct QuotaSettings {
    #[serde(flatten)]
    pub limits: QuotaLimits,
    /// Whether the Launch tab and the Pods tab's manage panel show
    /// separate CPU/memory *request* fields at all, independent of the
    /// quota limits themselves. When `false`, only limits are shown/sent,
    /// and a fixed request is substituted server-side (see below) instead
    /// of leaving it for Kubernetes to default to match the limit.
    pub expose_resource_requests: bool,
    /// Applied to every launch/edit's CPU/memory request in place of
    /// whatever the user would have set, but only while
    /// `expose_resource_requests` is `false` — irrelevant otherwise, since
    /// the user sets their own request directly in that mode. `None`
    /// means "leave the request unset", letting Kubernetes default it to
    /// match the limit.
    pub fixed_cpu_request: Option<String>,
    pub fixed_memory_request: Option<String>,
    /// Whether a non-admin may launch an image that isn't already a row in
    /// the Images catalog or an existing Template's own image — i.e.
    /// whether the Launch tab's "Custom" option and free-text image editing
    /// are available to them at all. Admins are always exempt. Defaults to
    /// `true` (current/original behavior, matching the `quota_settings`
    /// column default) — frontend code reading this before it's loaded
    /// should do the same, the way `expose_resource_requests` already does,
    /// rather than trust this struct's derived `Default` (which would give
    /// `false`).
    pub allow_custom_images: bool,
}

/// Returned by `GET /api/quota/me` — the caller's own effective quota,
/// current usage, and whether request fields should be shown at all.
/// Backs the Launch tab and the Pods tab's manage panel.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct MyQuota {
    /// The effective limits: the caller's `user_quotas` override if they
    /// have one, otherwise the global `quota_settings` default. Always
    /// unlimited (all `None`) for an admin, who is exempt from enforcement.
    pub limits: QuotaLimits,
    pub is_override: bool,
    pub expose_resource_requests: bool,
    /// Purely informational — the frontend never sends these back, the
    /// backend applies them server-side. See `QuotaSettings::fixed_cpu_request`.
    pub fixed_cpu_request: Option<String>,
    pub fixed_memory_request: Option<String>,
    pub used_cpu_millicores: i64,
    pub used_memory_bytes: i64,
    pub used_gpu_count: i64,
    /// See `QuotaSettings::allow_custom_images` — always `true` for an
    /// admin, who is exempt. Same before-load-defaults-to-true caveat.
    pub allow_custom_images: bool,
}

/// One row of the Quotas admin tab's per-user table (`GET /api/quota/users`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct UserQuotaEntry {
    pub user_id: i32,
    pub username: String,
    /// `None` if this user has no override and is bound by the global default.
    pub quota_override: Option<QuotaLimits>,
    pub used_cpu_millicores: i64,
    pub used_memory_bytes: i64,
    pub used_gpu_count: i64,
}
