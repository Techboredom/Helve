mod auth;
mod deployments;
mod error;
mod events;
mod images;
mod logs;
mod proxy;
mod quota;
mod resources;
mod state;
mod templates;
mod tokens;
mod users;
mod validate;
mod visibility;
mod watch;
mod ws;

use std::net::SocketAddr;
use std::time::Duration;

use axum::routing::{any, get, post, put};
use axum::Router;
use clap::Parser;
use kube::Client;
use sqlx::postgres::PgPoolOptions;
use tower_http::services::{ServeDir, ServeFile};
use tower_http::trace::TraceLayer;

use state::AppState;

/// A small dashboard that shows the pods running in a Kubernetes namespace,
/// with their basic resource requests (CPU, memory, accelerators).
#[derive(Parser, Debug)]
#[command(version, about)]
struct Args {
    /// Namespace to watch.
    #[arg(long, env = "NAMESPACE")]
    namespace: String,

    /// Address to bind the HTTP server to.
    #[arg(long, env = "BIND_ADDR", default_value = "0.0.0.0:3000")]
    bind_addr: String,

    /// Directory containing the built frontend (trunk build output) to serve as static files.
    #[arg(long, env = "STATIC_DIR", default_value = "frontend/dist")]
    static_dir: String,

    /// Postgres connection string backing the container image catalog.
    #[arg(long, env = "DATABASE_URL")]
    database_url: String,

    /// Password for a one-time bootstrap "admin" account, created only if the
    /// `users` table is empty. Ignored once any user exists.
    #[arg(long, env = "ADMIN_BOOTSTRAP_PASSWORD")]
    admin_bootstrap_password: Option<String>,

    /// Public origin this app is served from, e.g.
    /// "https://helve.example.com". Set this whenever the app is
    /// served over HTTPS: its scheme is what allows session cookies to be
    /// marked `Secure`. Required by `--proxy-base-domain`.
    #[arg(long, env = "APP_ORIGIN")]
    app_origin: Option<String>,

    /// Base domain for per-deployment proxy origins, e.g.
    /// "proxy.helve.example.com" — a deployment named `foo` is then
    /// served at `foo.proxy.helve.example.com`, on its own origin
    /// rather than on a path under this app's. Needs a wildcard DNS record
    /// (and TLS cert) covering `*.<this domain>`.
    ///
    /// Leaving this unset keeps the legacy same-origin `/proxy/<name>/`
    /// behavior, which lets a proxied app's JavaScript call this app's own
    /// API as whoever is browsing it — acceptable for local development,
    /// not for a shared deployment.
    #[arg(long, env = "PROXY_BASE_DOMAIN", requires = "app_origin")]
    proxy_base_domain: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env().add_directive("info".parse()?))
        .init();

    let args = Args::parse();

    // Tries an in-cluster service account first, then falls back to the local kubeconfig.
    let client = Client::try_default().await?;

    // idle_timeout below sqlx's own default (10 minutes) deliberately: this
    // connects through PgBouncer (postgres/pooler.yaml in Helve-Deploy) in
    // transaction mode, and PgBouncer's own server_idle_timeout - 600s,
    // confirmed live in its generated config, since CNPG sets no override -
    // is the *same* 10 minutes sqlx defaults to. Two clocks racing on an
    // identical interval means the app sometimes loses: it hands out a
    // connection PgBouncer already closed server-side a moment earlier,
    // caught reactively by sqlx's own test-before-acquire ping (on by
    // default) as "ping on idle connection returned error", surfacing as a
    // several-second readyz 503 roughly every ~10 minutes before the pool
    // recovers on its own. Recycling client-side well before PgBouncer's
    // timeout removes the race entirely rather than just tolerating it -
    // confirmed via the pooler's own live pgbouncer.ini, not assumed from
    // PgBouncer's documented default.
    let pg = PgPoolOptions::new()
        .max_connections(5)
        .idle_timeout(Duration::from_secs(240))
        .connect(&args.database_url)
        .await?;
    sqlx::migrate!().run(&pg).await?;
    bootstrap_admin(&pg, args.admin_bootstrap_password.as_deref()).await?;

    // `requires` on both flags means clap rejects setting one without the
    // other, so this zip either yields both or neither.
    let proxy_origin = args.app_origin.as_ref().zip(args.proxy_base_domain.as_ref()).map(|(app_origin, base_domain)| {
        state::ProxyOrigin {
            app_origin: app_origin.trim_end_matches('/').to_string(),
            base_domain: base_domain.trim_start_matches('.').to_string(),
        }
    });
    match &proxy_origin {
        Some(origin) => tracing::info!(
            base_domain = %origin.base_domain,
            app_origin = %origin.app_origin,
            "serving proxied deployments on per-deployment origins"
        ),
        None => tracing::warn!(
            "PROXY_BASE_DOMAIN is not set — proxied apps are served same-origin at /proxy/<name>/, \
             where their own JavaScript can call this app's API as whoever is browsing them; \
             intended for local development only"
        ),
    }

    let app_origin = args.app_origin.as_ref().map(|origin| origin.trim_end_matches('/').to_string());
    if app_origin.as_deref().map(|o| o.starts_with("http://")).unwrap_or(true) {
        tracing::warn!(
            "APP_ORIGIN is unset or not https — session cookies can't be marked Secure, so they travel in \
             cleartext over any plain-HTTP hop"
        );
    }

    let state = AppState::new(args.namespace.clone(), client.clone(), pg, app_origin, proxy_origin);
    tokio::spawn(watch::run(state.clone(), client));
    tokio::spawn(prune_expired_credentials(state.clone()));

    let index_html = format!("{}/index.html", args.static_dir);
    let static_service = ServeDir::new(&args.static_dir).fallback(ServeFile::new(&index_html));

    let app = Router::new()
        // Unauthenticated on purpose: these are for the kubelet, which
        // presents no session. They must also stay outside /api/, since
        // everything there requires a login.
        .route("/healthz", get(healthz))
        .route("/readyz", get(readyz))
        .route("/api/login", post(auth::login))
        .route("/api/logout", post(auth::logout))
        .route("/api/me", get(auth::me))
        .route("/api/me/password", put(auth::change_password))
        .route("/api/sessions", get(auth::list_sessions))
        .route("/api/users", get(users::list_users).post(users::create_user))
        .route("/api/users/{id}", axum::routing::delete(users::delete_user))
        .route("/api/users/{id}/password", put(users::reset_password))
        .route("/api/users/{id}/node-label", put(users::set_node_label))
        .route("/api/users/{id}/uid-gid", put(users::set_uid_gid))
        .route("/api/tokens", get(tokens::list_tokens).post(tokens::create_token))
        .route("/api/tokens/{id}", axum::routing::delete(tokens::delete_token))
        .route("/api/pods", get(ws::list_pods))
        .route("/api/images", get(images::list_images).post(images::create_image))
        .route("/api/images/{id}", put(images::update_image).delete(images::delete_image))
        .route("/api/templates", get(templates::list_templates).post(templates::create_template))
        .route("/api/templates/{id}", put(templates::update_template).delete(templates::delete_template))
        .route("/api/deployments", post(deployments::create_deployment))
        .route(
            "/api/deployments/{name}",
            get(deployments::get_deployment).put(deployments::update_deployment).delete(deployments::delete_deployment),
        )
        .route("/api/deployments/{name}/restart", post(deployments::restart_deployment))
        .route("/api/deployments/{name}/rollback", post(deployments::rollback_deployment))
        .route("/api/deployments/{name}/regenerate-secret", post(deployments::regenerate_secret))
        .route("/api/launches", get(deployments::list_launches))
        .route("/api/pvcs", get(deployments::list_pvcs))
        .route("/api/quota/me", get(quota::my_quota))
        .route("/api/quota/settings", get(quota::get_settings).put(quota::update_settings))
        .route("/api/quota/users", get(quota::list_user_quotas))
        .route("/api/quota/users/{id}", put(quota::set_user_quota).delete(quota::clear_user_quota))
        .route("/api/pods/{name}/logs", get(logs::get_pod_logs))
        .route("/api/pods/{name}/events", get(events::get_pod_events))
        .route("/ws", get(ws::ws_handler))
        // The bare/trailing-slash routes exist because matchit's `{*rest}`
        // wildcard requires at least one character after the slash — without
        // them, every "Open" link (which points at the bare
        // "/proxy/<name>/") would silently miss this route entirely and hit
        // the SPA fallback below instead. See proxy::handler_root.
        .route("/proxy/{deployment_name}", any(proxy::handler_root))
        .route("/proxy/{deployment_name}/", any(proxy::handler_root))
        .route("/proxy/{deployment_name}/{*rest}", any(proxy::handler))
        // Entry point for a proxy origin that has no session of its own yet:
        // it lands here, on the app origin, where the caller's session cookie
        // actually exists. See proxy::start_proxy_auth.
        .route("/proxy-auth", get(proxy::start_proxy_auth))
        .fallback_service(static_service)
        // No CORS layer: the frontend is served by this same process, so
        // every call it makes is same-origin. `trunk serve` proxies to the
        // backend server-side during development, which CORS never sees
        // either. Adding a permissive policy would only widen what other
        // sites can do with a logged-in browser.
        .layer(TraceLayer::new_for_http())
        .with_state(state.clone())
        // Outermost, so a request to a deployment's own origin is served as
        // that deployment and never reaches the routes above at all.
        .layer(axum::middleware::from_fn_with_state(state, proxy::dispatch_by_host));

    let addr: SocketAddr = args.bind_addr.parse()?;
    tracing::info!(%addr, namespace = %args.namespace, "starting server");
    let listener = tokio::net::TcpListener::bind(addr).await?;
    // `ConnectInfo` (used by auth::login to record a login's source IP in
    // session_log) requires the connect-info-aware make-service.
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(shutdown_signal())
        .await?;

    Ok(())
}

/// Resolves once Kubernetes asks this pod to stop (`SIGTERM`) or a local
/// `Ctrl+C`. Only after this resolves does axum stop accepting *new*
/// connections and start waiting for in-flight ones to finish — without it,
/// the default Rust runtime behavior on `SIGTERM` is to just exit
/// immediately, silently dropping whatever requests were mid-flight.
///
/// Covers plain HTTP requests (login, launch, the REST API) — a real
/// improvement for a rolling restart, since those now finish instead of
/// getting reset. It does **not** extend to connections already upgraded to
/// a raw byte stream (the pod-watch `/ws` WebSocket, or a proxied
/// deployment's tunneled WebSocket in `proxy.rs`): once upgraded, those run
/// on a detached task outside hyper's own request bookkeeping, so an
/// already-open Jupyter kernel session or live Pods-tab connection still
/// gets cut when the process actually exits. Solving that would mean
/// tracking upgraded connections and waiting on them too — out of scope
/// here; the client-side reconnect/retry behavior for those is what
/// actually matters for those cases.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received, pausing before draining in-flight requests");
    // Kubernetes removes a Terminating pod from its Service's endpoints
    // asynchronously — there's a real (if usually brief) window where a new
    // connection can still land here right after this signal arrives. This
    // image has no shell (distroless), so it can't use the usual preStop
    // `sleep` hook to cover that window; doing the same wait in-process
    // here, before we actually stop accepting new connections below,
    // accomplishes the same thing without needing one.
    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
    tracing::info!("done waiting, no longer accepting new connections");
}

/// Liveness: is this process still serving HTTP at all? Deliberately checks
/// nothing else — a liveness probe that depended on Postgres would restart a
/// perfectly healthy app every time the database hiccuped, which is the
/// opposite of what restarting is for.
async fn healthz() -> &'static str {
    "ok"
}

/// Readiness: should this pod receive traffic? Unlike liveness, this *does*
/// check Postgres, because an instance that can't reach it can't serve a
/// single useful request — better to drop out of the Service's endpoints
/// until it can.
async fn readyz(
    axum::extract::State(state): axum::extract::State<AppState>,
) -> (axum::http::StatusCode, &'static str) {
    // Bounded explicitly: with Postgres unreachable the pool blocks waiting
    // for a connection until its own (much longer) acquire timeout, so the
    // probe would hang rather than answer. A probe that times out is failed
    // either way, but answering promptly keeps the reason in our logs
    // instead of only in the kubelet's.
    const READY_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
    let unavailable = (axum::http::StatusCode::SERVICE_UNAVAILABLE, "database not reachable");
    match tokio::time::timeout(READY_TIMEOUT, sqlx::query("SELECT 1").execute(&state.pg)).await {
        Ok(Ok(_)) => (axum::http::StatusCode::OK, "ok"),
        Ok(Err(err)) => {
            tracing::warn!(error = %err, "readiness check failed");
            unavailable
        }
        Err(_) => {
            tracing::warn!("readiness check timed out waiting for the database");
            unavailable
        }
    }
}

/// Deletes credentials that have already expired. Nothing reads them once
/// they're past `expires_at` — every lookup filters on it — so this is purely
/// to stop the tables growing without bound.
///
/// Deliberately limited to expired *credentials*. The `session_log` and
/// `launch_log` audit tables are left alone: how long to keep those is a
/// retention decision (they record who logged in from where), not something
/// to silently discard here.
async fn prune_expired_credentials(state: AppState) {
    const PRUNE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(3600);
    let mut ticker = tokio::time::interval(PRUNE_INTERVAL);
    loop {
        ticker.tick().await;
        // Written out rather than looped over a table name, because sqlx
        // (rightly) refuses to take SQL built at runtime.
        let statements: [(&str, _); 3] = [
            ("sessions", sqlx::query("DELETE FROM sessions WHERE expires_at < now()")),
            ("proxy_auth_tokens", sqlx::query("DELETE FROM proxy_auth_tokens WHERE expires_at < now()")),
            ("proxy_sessions", sqlx::query("DELETE FROM proxy_sessions WHERE expires_at < now()")),
        ];
        for (table, statement) in statements {
            match statement.execute(&state.pg).await {
                Ok(result) if result.rows_affected() > 0 => {
                    tracing::info!(table, removed = result.rows_affected(), "pruned expired rows");
                }
                Ok(_) => {}
                Err(err) => tracing::warn!(table, error = %err, "failed to prune expired rows"),
            }
        }
    }
}

/// Creates the initial "admin" account if (and only if) no users exist yet.
/// Without this there'd be no way to log in at all on a fresh database.
async fn bootstrap_admin(pg: &sqlx::PgPool, bootstrap_password: Option<&str>) -> anyhow::Result<()> {
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(pg).await?;
    if user_count > 0 {
        return Ok(());
    }
    let Some(password) = bootstrap_password else {
        tracing::warn!(
            "no users exist yet and ADMIN_BOOTSTRAP_PASSWORD is not set — the app has no way to log in until a user is created"
        );
        return Ok(());
    };
    let password_hash = auth::hash_password(password).map_err(|_| anyhow::anyhow!("failed to hash bootstrap password"))?;
    sqlx::query("INSERT INTO users (username, password_hash, role) VALUES ('admin', $1, 'admin')")
        .bind(&password_hash)
        .execute(pg)
        .await?;
    tracing::info!("created initial admin account (username: admin)");
    Ok(())
}
