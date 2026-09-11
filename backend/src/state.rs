use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{PodEvent, PodInfo};
use kube::Client;
use sqlx::{PgPool, Postgres, Transaction};
use tokio::sync::{broadcast, Mutex, RwLock};

/// Serves each proxied deployment from its own origin
/// (`<name>.<base_domain>`) rather than from a path on Helve's own origin.
/// That separation is what stops a proxied app — which runs code Helve
/// doesn't control — from reaching `/api/*` as whoever is browsing it, since
/// the browser then treats it as a different origin and Helve's host-only
/// session cookie never travels there.
///
/// `None` (neither flag set) keeps the legacy path-based `/proxy/<name>/`
/// behavior, which is same-origin and therefore only appropriate for local
/// development.
#[derive(Clone, Debug)]
pub struct ProxyOrigin {
    /// Where the app itself is served, e.g. `https://helve.example.com`.
    /// Used to send an unauthenticated proxy origin back somewhere the
    /// caller's session cookie actually exists.
    pub app_origin: String,
    /// e.g. `proxy.helve.example.com`, so a deployment named `foo`
    /// is served at `foo.proxy.helve.example.com`.
    pub base_domain: String,
}

impl ProxyOrigin {
    /// The deployment a request's `Host` belongs to, if it names one of our
    /// proxy origins. Matches only a single label in front of the base
    /// domain — `a.b.<base>` is not a deployment, which matters because a
    /// wildcard TLS cert covers exactly one label too.
    pub fn deployment_for_host(&self, host: &str) -> Option<String> {
        // Host may carry a port (`foo.example:8443`); the cert and our
        // routing both key off the hostname alone. Browsers already
        // lowercase the header, but comparing case-insensitively means a
        // non-browser client sending mixed case still matches.
        let host = host.split(':').next().unwrap_or(host).trim_end_matches('.').to_ascii_lowercase();
        let base_domain = self.base_domain.to_ascii_lowercase();
        let name = host.strip_suffix(&base_domain)?.strip_suffix('.')?;
        if name.is_empty() || name.contains('.') {
            return None;
        }
        Some(name.to_string())
    }

    /// The full origin a given deployment is served from.
    pub fn origin_for(&self, deployment: &str) -> String {
        format!("{}://{deployment}.{}", self.scheme(), self.base_domain)
    }

    /// Mirrors the app origin's scheme — the two are always served the same
    /// way, and it decides whether cookies can be marked `Secure`.
    pub fn scheme(&self) -> &str {
        if self.app_origin.starts_with("http://") { "http" } else { "https" }
    }

    pub fn is_https(&self) -> bool {
        self.scheme() == "https"
    }
}

/// How a per-user home directory gets provided to launched environments,
/// mounted at whatever mount path a template's `home_mount_path` names.
/// `None` (no backend flag set) disables the feature entirely — no home
/// volume is ever added, and a template/launch setting `home_mount_path`
/// is rejected with 400 rather than silently ignored.
///
/// `HostPath` and `SharedPvc` both sidestep needing ReadWriteMany storage:
/// the only StorageClasses this project has ever run against are Ceph RBD,
/// which is ReadWriteOnce-only for a `Filesystem`-mode PVC — two of one
/// user's environments landing on different nodes would leave the second
/// stuck `Pending`. `DynamicPvc` requires the operator to actually point
/// `storage_class` at something ReadWriteMany-capable (CephFS, NFS-CSI,
/// etc.); Helve has no way to check that itself before creating the claim.
#[derive(Clone, Debug)]
pub enum HomeDrives {
    /// `<base_path>/<username>` as a `hostPath` volume (`DirectoryOrCreate`).
    /// Requires every node to already have the same shared filesystem
    /// (NFS, CephFS, a parallel filesystem, ...) mounted at `base_path` at
    /// the OS level — from Kubernetes' point of view this looks identical
    /// on any node, so there's no CSI/RWX machinery involved at all.
    HostPath { base_path: String },
    /// One PersistentVolumeClaim per user (`home-<username>`), created on
    /// first use from a ReadWriteMany-capable StorageClass. Never deleted
    /// by Helve, on user deletion or otherwise — same caution as every
    /// other "Helve never deletes X" rule in this codebase, so a mistake
    /// here can't be a data-loss bug.
    DynamicPvc { storage_class: String, size: String },
    /// A single PersistentVolumeClaim, provisioned once out-of-band (the
    /// same way the existing shared-model-cache mount already works —
    /// Helve never creates this one), mounted into every user's pod with
    /// `subPath: <username>`. Kubernetes creates that subdirectory on the
    /// volume automatically the first time it's mounted if it doesn't
    /// already exist, so this needs no per-user provisioning step at all —
    /// just one RWX-capable (or statically-bound) share set up ahead of
    /// time, matching this codebase's default preference for not creating
    /// PVCs itself.
    SharedPvc { claim_name: String },
}

/// LDAP/Active Directory login, tried by `auth::login` as a fallback
/// whenever the local password check fails — see `ldap::authenticate_and_provision`.
/// `None` (the field unset) disables it entirely, leaving local password
/// login as the only path, same `Option<T>`-on-`AppState` shape as
/// [`HomeDrives`].
#[derive(Clone, Debug)]
pub struct LdapConfig {
    /// `ldaps://...` (or `ldap://...` with StartTLS negotiated
    /// separately) — a plain `ldap://` URL sends the bind password
    /// unencrypted and is warned about at startup, the same way
    /// `APP_ORIGIN` not being https is.
    pub url: String,
    /// Service account Helve binds as to search for the user's DN. Needs
    /// no privileges beyond reading the directory.
    pub bind_dn: String,
    pub bind_password: String,
    pub base_dn: String,
    /// e.g. `"(uid={username})"` (OpenLDAP) or `"(sAMAccountName={username})"`
    /// (Active Directory) — `{username}` is substituted with the submitted
    /// username, rejected first if it contains LDAP filter metacharacters.
    pub user_filter: String,
    /// DN of a group whose membership maps to `Role::Admin`. Checked on
    /// every successful LDAP login (not just provisioning), so the
    /// directory stays the source of truth for this once it's set — a
    /// manual role change made from the Users tab is overwritten on that
    /// account's next LDAP login.
    pub admin_group_dn: Option<String>,
    /// Whether a first-time LDAP login with no matching local account
    /// creates one automatically. `false` means an admin must pre-create
    /// the account (any password works — it's immediately overridden by
    /// the LDAP check) before that user can ever log in.
    pub auto_provision: bool,
}

/// OIDC (SSO) login — an independent, simultaneously-usable alternative to
/// [`LdapConfig`] and local password login, not a replacement for either.
/// `None` disables it; the login page then shows no SSO button at all
/// (`GET /api/auth/config`).
#[derive(Clone, Debug)]
pub struct OidcConfig {
    pub issuer_url: String,
    pub client_id: String,
    pub client_secret: String,
    /// Claim to read the Helve username from — e.g. `"preferred_username"`.
    /// Only consulted the first time an account is provisioned/linked;
    /// every login after that is identified by `oidc_subject` alone.
    pub username_claim: String,
    /// Claim carrying group membership, read as a JSON array of strings —
    /// e.g. `"groups"`. Absent or non-array is treated as "no groups".
    pub groups_claim: String,
    /// A group name in `groups_claim`'s value that maps to `Role::Admin`,
    /// re-checked on every login the same way `LdapConfig::admin_group_dn` is.
    pub admin_group: Option<String>,
    /// Same meaning as `LdapConfig::auto_provision`, with one additional
    /// wrinkle: when `false`, a first-ever login for a given `sub` may link
    /// to an *existing* local account matching `username_claim` instead of
    /// being rejected — see `oidc::callback`. When `true`, that linking
    /// never happens (closes an account-takeover-shaped edge case), and an
    /// unrecognized `sub` always creates a new account instead.
    pub auto_provision: bool,
}

/// Istio service-mesh mTLS pod-to-pod tenant isolation — see
/// `backend/src/istio.rs`. `None` (the default) leaves the whole feature
/// off: no owner ServiceAccount is ever created, no `AuthorizationPolicy` is
/// ever created, and launched pods behave exactly as they did before this
/// existed (any pod in the namespace can reach any other's ClusterIP,
/// same as today).
#[derive(Clone, Debug)]
pub struct IstioConfig {
    /// Istio's trust domain, e.g. `"cluster.local"` — the first segment of
    /// every SPIFFE-style principal this feature compares against.
    pub trust_domain: String,
    /// The Helve backend's own ServiceAccount name, always allow-listed
    /// alongside a deployment's owner on every `AuthorizationPolicy` this
    /// creates — otherwise turning this feature on would also cut off
    /// Helve's own reverse-proxy (`/proxy/`, `/models/`) traffic into every
    /// launched pod.
    pub backend_service_account: String,
}

/// How many failed logins from one address, within [`LOGIN_FAILURE_WINDOW`],
/// before further attempts are refused outright.
const MAX_LOGIN_FAILURES: usize = 10;
const LOGIN_FAILURE_WINDOW: Duration = Duration::from_secs(300);

/// Per-source-address failed-login tracking.
///
/// Beyond slowing password guessing, this caps an unauthenticated CPU drain:
/// verifying a password runs argon2, which is expensive *by design*, so an
/// attacker who doesn't care about guessing correctly could otherwise pin the
/// server's cores with a stream of junk logins.
#[derive(Clone)]
pub struct LoginThrottle {
    failures: Arc<Mutex<HashMap<IpAddr, Vec<Instant>>>>,
    max_failures: usize,
    window: Duration,
}

impl Default for LoginThrottle {
    fn default() -> Self {
        Self::new(MAX_LOGIN_FAILURES, LOGIN_FAILURE_WINDOW)
    }
}

impl LoginThrottle {
    pub fn new(max_failures: usize, window: Duration) -> Self {
        Self { failures: Arc::new(Mutex::new(HashMap::new())), max_failures, window }
    }

    /// Whether `ip` has failed enough logins recently to be turned away
    /// without checking its password at all.
    pub async fn blocked(&self, ip: IpAddr) -> bool {
        let mut failures = self.failures.lock().await;
        let Some(recent) = failures.get_mut(&ip) else { return false };
        recent.retain(|at| at.elapsed() < self.window);
        if recent.is_empty() {
            failures.remove(&ip);
            return false;
        }
        recent.len() >= self.max_failures
    }

    pub async fn record_failure(&self, ip: IpAddr) {
        let mut failures = self.failures.lock().await;
        // Swept here as well as in `blocked`, so a stream of one-off source
        // addresses can't grow this map without bound.
        failures.retain(|_, recent| {
            recent.retain(|at| at.elapsed() < self.window);
            !recent.is_empty()
        });
        failures.entry(ip).or_default().push(Instant::now());
    }

    /// Clears an address's history, so a few typos followed by the right
    /// password don't leave someone throttled.
    pub async fn clear(&self, ip: IpAddr) {
        self.failures.lock().await.remove(&ip);
    }
}

#[derive(Clone)]
pub struct AppState {
    pub namespace: String,
    pub client: Client,
    pub pg: PgPool,
    /// Public origin this app is served from, when it's been configured.
    /// Its scheme is what decides whether cookies may be marked `Secure`.
    pub app_origin: Option<String>,
    /// `None` = legacy same-origin `/proxy/<name>/` mode; see [`ProxyOrigin`].
    pub proxy_origin: Option<ProxyOrigin>,
    /// `None` = the home-directory feature is off entirely; see [`HomeDrives`].
    pub home_drives: Option<HomeDrives>,
    /// `None` = LDAP/AD login is off entirely; see [`LdapConfig`].
    pub ldap: Option<LdapConfig>,
    /// `None` = OIDC (SSO) login is off entirely. Holds the already-
    /// discovered provider client (see `oidc::Oidc::discover`, run once at
    /// startup) alongside its `OidcConfig`, wrapped in an `Arc` since the
    /// discovered client itself is neither `Copy` nor cheap to rebuild —
    /// `AppState` as a whole is cloned per-request the way axum's `State`
    /// extractor always does.
    pub oidc: Option<Arc<crate::oidc::Oidc>>,
    /// `None` = Istio pod-to-pod tenant isolation is off entirely; see [`IstioConfig`].
    pub istio: Option<IstioConfig>,
    login_throttle: LoginThrottle,
    pods: Arc<RwLock<HashMap<String, PodInfo>>>,
    events: broadcast::Sender<PodEvent>,
}

impl AppState {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        namespace: String,
        client: Client,
        pg: PgPool,
        app_origin: Option<String>,
        proxy_origin: Option<ProxyOrigin>,
        home_drives: Option<HomeDrives>,
        ldap: Option<LdapConfig>,
        oidc: Option<Arc<crate::oidc::Oidc>>,
        istio: Option<IstioConfig>,
    ) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            namespace,
            client,
            pg,
            app_origin,
            proxy_origin,
            home_drives,
            ldap,
            oidc,
            istio,
            login_throttle: LoginThrottle::default(),
            pods: Arc::new(RwLock::new(HashMap::new())),
            events,
        }
    }

    /// Whether cookies may carry the `Secure` attribute — true only when
    /// this app is served over HTTPS, since a `Secure` cookie is simply never
    /// sent back over plain HTTP and would lock everyone out.
    pub fn cookies_secure(&self) -> bool {
        self.app_origin.as_deref().map(|origin| origin.starts_with("https://")).unwrap_or(false)
    }

    pub async fn login_blocked(&self, ip: IpAddr) -> bool {
        self.login_throttle.blocked(ip).await
    }

    pub async fn record_login_failure(&self, ip: IpAddr) {
        self.login_throttle.record_failure(ip).await;
    }

    pub async fn clear_login_failures(&self, ip: IpAddr) {
        self.login_throttle.clear(ip).await;
    }

    /// Serializes quota-checked writes (launch, scale, edit) across every
    /// Helve replica, not just within one process.
    ///
    /// Quota is enforced by reading current usage and then writing — two
    /// requests interleaving between those steps would both see the
    /// pre-write total and both be allowed, letting a user step over their
    /// quota just by launching twice at once (or, with more than one
    /// replica, by two requests simply landing on different pods). A plain
    /// in-process `Mutex` only ever serialized the first case; a Postgres
    /// advisory lock, shared by every replica through the same database,
    /// covers both. `pg_advisory_xact_lock` ties the lock to this
    /// transaction's lifetime — it releases automatically on commit *or*
    /// rollback, so an early return via `?` anywhere in the caller can't
    /// leak it the way forgetting to call an explicit unlock could.
    ///
    /// The caller holds the returned transaction open across the
    /// Kubernetes write itself (committing only once that succeeds), so the
    /// next caller's read is guaranteed to see it. Global rather than
    /// per-user: these writes are infrequent and short-lived, and one lock
    /// is far easier to reason about than a map of them.
    pub async fn lock_launches(&self) -> Result<Transaction<'_, Postgres>, sqlx::Error> {
        // Arbitrary constant, shared by every replica: Postgres advisory
        // locks are just integers, meaningful only in that the same number
        // means the same lock.
        const LAUNCH_LOCK_KEY: i64 = 727_100_001;
        let mut tx = self.pg.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(LAUNCH_LOCK_KEY).execute(&mut *tx).await?;
        Ok(tx)
    }

    /// The URL that opens `deployment` — an absolute URL on its own origin
    /// when per-deployment origins are configured, else the legacy
    /// same-origin path.
    pub fn proxy_url(&self, deployment: &str) -> String {
        match &self.proxy_origin {
            Some(origin) => format!("{}/", origin.origin_for(deployment)),
            None => format!("/proxy/{deployment}/"),
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<PodEvent> {
        self.events.subscribe()
    }

    pub async fn snapshot(&self) -> Vec<PodInfo> {
        self.pods.read().await.values().cloned().collect()
    }

    /// Inserts or replaces a pod's info and broadcasts the change to any live listeners.
    pub async fn upsert(&self, pod: PodInfo) {
        self.pods.write().await.insert(pod.name.clone(), pod.clone());
        // Ignore send errors: they only mean no WebSocket client is currently connected.
        let _ = self.events.send(PodEvent::Upsert { pod: Box::new(pod) });
    }

    /// Replaces the whole pod set atomically (used after the watcher's initial list completes).
    pub async fn replace_all(&self, pods: Vec<PodInfo>) {
        let mut guard = self.pods.write().await;
        guard.clear();
        for pod in pods {
            guard.insert(pod.name.clone(), pod);
        }
    }

    pub async fn remove(&self, name: &str) {
        self.pods.write().await.remove(name);
        let _ = self.events.send(PodEvent::Delete { name: name.to_string() });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ip(last: u8) -> IpAddr {
        IpAddr::from([192, 168, 10, last])
    }

    #[tokio::test]
    async fn blocks_only_after_the_limit_is_reached() {
        let throttle = LoginThrottle::new(3, Duration::from_secs(300));
        assert!(!throttle.blocked(ip(1)).await);

        throttle.record_failure(ip(1)).await;
        throttle.record_failure(ip(1)).await;
        assert!(!throttle.blocked(ip(1)).await, "under the limit should still be allowed");

        throttle.record_failure(ip(1)).await;
        assert!(throttle.blocked(ip(1)).await);
    }

    #[tokio::test]
    async fn throttling_is_per_address() {
        let throttle = LoginThrottle::new(2, Duration::from_secs(300));
        throttle.record_failure(ip(1)).await;
        throttle.record_failure(ip(1)).await;
        assert!(throttle.blocked(ip(1)).await);
        // One noisy address must not lock everyone else out.
        assert!(!throttle.blocked(ip(2)).await);
    }

    #[tokio::test]
    async fn a_successful_login_clears_the_history() {
        let throttle = LoginThrottle::new(2, Duration::from_secs(300));
        throttle.record_failure(ip(1)).await;
        throttle.record_failure(ip(1)).await;
        assert!(throttle.blocked(ip(1)).await);

        throttle.clear(ip(1)).await;
        assert!(!throttle.blocked(ip(1)).await, "typos then the right password shouldn't leave you locked out");
    }

    #[tokio::test]
    async fn failures_expire_out_of_the_window() {
        let throttle = LoginThrottle::new(1, Duration::from_millis(20));
        throttle.record_failure(ip(1)).await;
        assert!(throttle.blocked(ip(1)).await);

        tokio::time::sleep(Duration::from_millis(40)).await;
        assert!(!throttle.blocked(ip(1)).await, "the lockout should lift once the window passes");
    }

    fn origin() -> ProxyOrigin {
        ProxyOrigin {
            app_origin: "https://helve.example.com".to_string(),
            base_domain: "proxy.helve.example.com".to_string(),
        }
    }

    #[test]
    fn maps_a_single_label_host_to_its_deployment() {
        let o = origin();
        assert_eq!(o.deployment_for_host("foo.proxy.helve.example.com"), Some("foo".to_string()));
        // A port is part of the Host header but not of the name.
        assert_eq!(o.deployment_for_host("foo.proxy.helve.example.com:8443"), Some("foo".to_string()));
        // Trailing dot is a legal absolute FQDN.
        assert_eq!(o.deployment_for_host("foo.proxy.helve.example.com."), Some("foo".to_string()));
    }

    #[test]
    fn rejects_hosts_that_merely_end_with_the_base_domain() {
        let o = origin();
        // The attacker-registered lookalike: suffix matches, but it is a
        // different domain entirely.
        assert_eq!(o.deployment_for_host("evilproxy.helve.example.com"), None);
        assert_eq!(o.deployment_for_host("notproxy.helve.example.com"), None);
    }

    #[test]
    fn matches_a_host_regardless_of_case() {
        let o = origin();
        // Browsers already lowercase the Host header, but a non-browser
        // client sending mixed case must still match.
        assert_eq!(o.deployment_for_host("Foo.Proxy.Helve.Example.Com"), Some("foo".to_string()));
        assert_eq!(o.deployment_for_host("FOO.PROXY.HELVE.EXAMPLE.COM"), Some("foo".to_string()));
    }

    #[test]
    fn rejects_multi_label_prefixes() {
        // A wildcard cert covers one label, so anything deeper would be
        // served without a matching cert — and would let one deployment
        // shadow another's name.
        assert_eq!(origin().deployment_for_host("a.b.proxy.helve.example.com"), None);
    }

    #[test]
    fn rejects_the_base_domain_and_app_origin_themselves() {
        let o = origin();
        assert_eq!(o.deployment_for_host("proxy.helve.example.com"), None);
        assert_eq!(o.deployment_for_host("helve.example.com"), None);
        assert_eq!(o.deployment_for_host("unrelated.example.com"), None);
        assert_eq!(o.deployment_for_host(""), None);
    }

    #[test]
    fn builds_per_deployment_origins_and_urls() {
        let o = origin();
        assert_eq!(o.origin_for("foo"), "https://foo.proxy.helve.example.com");
        assert!(o.is_https());

        let plain = ProxyOrigin { app_origin: "http://localhost:3000".to_string(), ..origin() };
        assert_eq!(plain.scheme(), "http");
        assert!(!plain.is_https());
    }
}
