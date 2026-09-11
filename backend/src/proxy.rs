//! Reverse-proxies `/proxy/{deployment_name}/...` straight into the launched
//! pod, injecting its auto-generated credential (if any) so there's no login
//! prompt — the "JupyterHub-style" part of the auto-generated-credentials
//! feature. Some proxied apps (RStudio run with `DISABLE_AUTH=true`) have no
//! credential at all; for those, Helve's own ownership check below is the
//! *only* gate, so their template must also set `public_service = false`
//! (see `deployments.rs`) so nothing can reach them directly.
//!
//! Reaches the pod via its `Service`'s in-cluster `ClusterIP` (whether that
//! Service is itself a public `LoadBalancer` or a `ClusterIP`-only one makes
//! no difference here — both have a `ClusterIP`). This is the conventional
//! in-cluster design and assumes Helve itself runs in-cluster in
//! production; it does **not** work with the backend running locally
//! against a remote cluster, since a `ClusterIP` isn't routable from outside
//! the cluster network — unlike the rest of this app, this one code path
//! can't be exercised from a local dev machine without actually deploying
//! Helve into the cluster.
//!
//! Only templates with `proxy_enabled` (JupyterLab and RStudio — see
//! `backend/migrations/0005_add_proxy_support.sql` and
//! `0006_add_rstudio_proxy_support.sql`) ever reach this code path.
//! `strip_prefix` controls how each one's path gets forwarded — see the
//! note in the handler below.
//!
//! Two ways in, depending on configuration:
//!
//! * With `PROXY_BASE_DOMAIN` set (how this should be deployed), each
//!   deployment gets its **own origin**, `<name>.<base domain>`, dispatched
//!   by `Host` in [`dispatch_by_host`] before the app's router ever sees the
//!   request. A proxied app is then a different origin from Helve, so its
//!   JavaScript can't call `/api/*` as whoever is browsing it. Since Helve's
//!   host-only session cookie doesn't reach that origin either, it earns its
//!   own via the handshake in [`start_proxy_auth`] / `redeem_auth_token`.
//!   The legacy path below just redirects here.
//! * Without it, the legacy same-origin `/proxy/<name>/` path serves the app
//!   directly. That is a privilege-escalation risk on any shared deployment
//!   (see the README's "Per-deployment proxy origins") and exists only so
//!   local development works without DNS.

use axum::body::{to_bytes, Body};
use axum::extract::{Path, Query, Request, State};
use axum::http;
use axum::http::{HeaderName, StatusCode};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use bytes::Bytes;
use http_body_util::Full;
use hyper_util::rt::TokioIo;
use k8s_openapi::api::core::v1::Service;
use kube::api::Api;
use serde::Deserialize;
use time::Duration;
use tokio::net::TcpStream;

use crate::auth::{generate_token, CurrentUser, SESSION_COOKIE};
use crate::error::ApiError;
use crate::state::AppState;
use common::Role;

const MAX_PROXIED_BODY_BYTES: usize = 20 * 1024 * 1024;

struct ProxyTarget {
    owner_username: String,
    env_key: Option<String>,
    secret_value: Option<String>,
    container_port: i32,
    strip_prefix: bool,
}

/// Cookie granting access to exactly one deployment, set host-only on that
/// deployment's own proxy origin. Deliberately separate from
/// `helve_session`: it authorizes nothing but this one proxied app, so a
/// pod that manages to capture its own is no more powerful than it already
/// was.
pub const PROXY_COOKIE: &str = "helve_proxy";

/// Where a proxy origin redeems the one-time token minted on the app origin.
/// Under `/__helve/` to keep it clear of any path a proxied app might
/// itself serve.
const AUTH_CALLBACK_PATH: &str = "/__helve/auth";

/// The handoff token is used immediately by a redirect, so it only has to
/// survive one round trip — short enough that a copy left in history or a
/// Referer header is dead on arrival.
const AUTH_TOKEN_TTL_SECS: i64 = 30;
const PROXY_SESSION_LIFETIME_HOURS: i64 = 12;

/// Routes a request by its `Host`: anything naming a per-deployment proxy
/// origin is served here, everything else falls through to the app's own
/// router. Installed as the outermost layer, so a proxy origin never reaches
/// `/api/*` or the SPA at all — that separation is the whole point of giving
/// each deployment its own origin.
pub async fn dispatch_by_host(
    State(state): State<AppState>,
    req: Request,
    next: axum::middleware::Next,
) -> Response {
    let Some(origin) = state.proxy_origin.clone() else {
        return next.run(req).await;
    };
    let host = req
        .headers()
        .get(http::header::HOST)
        .and_then(|h| h.to_str().ok())
        .map(str::to_string)
        .or_else(|| req.uri().host().map(str::to_string));
    let Some(deployment) = host.as_deref().and_then(|host| origin.deployment_for_host(host)) else {
        return next.run(req).await;
    };
    serve_proxy_origin(state, deployment, req).await
}

async fn serve_proxy_origin(state: AppState, deployment: String, req: Request) -> Response {
    if req.uri().path() == AUTH_CALLBACK_PATH {
        // Copied out rather than borrowed: `Request`'s body isn't `Sync`, so
        // holding a reference to it across an await would make this whole
        // middleware's future non-`Send`.
        let query = req.uri().query().unwrap_or_default().to_string();
        return redeem_auth_token(&state, &deployment, &query).await.into_response();
    }

    match proxy_session_user(&state, &deployment, req.headers()).await {
        Ok(Some(user)) => proxy_request(user, deployment, String::new(), state, req, true).await.into_response(),
        Ok(None) => start_auth_redirect(&state, &deployment, &req),
        Err(err) => err.into_response(),
    }
}

/// Sends an unauthenticated proxy origin back to the app origin, which is
/// the only host the caller's `helve_session` cookie is ever sent to and so
/// the only place their identity (and ownership of this deployment) can be
/// established.
fn start_auth_redirect(state: &AppState, deployment: &str, req: &Request) -> Response {
    let Some(origin) = &state.proxy_origin else {
        return ApiError::ProxyUnavailable("proxy origins are not configured".to_string()).into_response();
    };
    let next = req.uri().path_and_query().map(|pq| pq.as_str()).unwrap_or("/");
    let url = format!(
        "{}/proxy-auth?deployment={}&next={}",
        origin.app_origin,
        encode_query_value(deployment),
        encode_query_value(next),
    );
    Redirect::temporary(&url).into_response()
}

#[derive(Deserialize)]
pub struct ProxyAuthQuery {
    deployment: String,
    #[serde(default)]
    next: Option<String>,
}

/// `GET /proxy-auth` on the **app** origin: proves who the caller is from
/// their normal session, checks they may open this deployment, and hands
/// them a single-use token to redeem on that deployment's own origin.
pub async fn start_proxy_auth(
    // Taken as a `Result` so an expired session lands on the login page
    // instead of a raw 401 body — this is a URL people reach by following a
    // link or a bookmark, not an API call.
    user: Result<CurrentUser, ApiError>,
    State(state): State<AppState>,
    Query(query): Query<ProxyAuthQuery>,
) -> Result<Response, ApiError> {
    let origin = state
        .proxy_origin
        .as_ref()
        .ok_or_else(|| ApiError::BadRequest("proxy origins are not configured".to_string()))?;

    let user = match user {
        Ok(user) => user,
        Err(_) => return Ok(Redirect::temporary(&format!("{}/", origin.app_origin)).into_response()),
    };

    // Ownership is checked here, on the origin that actually knows who the
    // caller is — the proxy origin downstream only ever sees the token.
    let target = load_target(&state, &query.deployment).await?;
    if user.role != Role::Admin && user.username != target.owner_username {
        return Err(ApiError::Forbidden("you don't own this deployment".to_string()));
    }

    let token = generate_token();
    sqlx::query(
        "INSERT INTO proxy_auth_tokens (token, deployment_name, user_id, expires_at) \
         VALUES ($1, $2, $3, now() + make_interval(secs => $4))",
    )
    .bind(&token)
    .bind(&query.deployment)
    .bind(user.id)
    .bind(AUTH_TOKEN_TTL_SECS as f64)
    .execute(&state.pg)
    .await?;

    let next = safe_next_path(query.next.as_deref().unwrap_or("/"));
    let url = format!(
        "{}{AUTH_CALLBACK_PATH}?token={}&next={}",
        origin.origin_for(&query.deployment),
        encode_query_value(&token),
        encode_query_value(&next),
    );
    Ok(Redirect::temporary(&url).into_response())
}

#[derive(Deserialize)]
struct AuthCallbackQuery {
    token: String,
    #[serde(default)]
    next: Option<String>,
}

/// `GET /__helve/auth` on a **proxy** origin: redeems the one-time token and
/// exchanges it for a cookie scoped to this host and this deployment alone.
async fn redeem_auth_token(state: &AppState, deployment: &str, raw_query: &str) -> Result<Response, ApiError> {
    let query: AuthCallbackQuery = serde_urlencoded::from_str(raw_query)
        .map_err(|_| ApiError::BadRequest("missing or malformed auth token".to_string()))?;

    // Single use: deleting as we read means a token replayed from history, a
    // Referer header, or a log is already spent.
    let row: Option<(String, i32)> = sqlx::query_as(
        "DELETE FROM proxy_auth_tokens WHERE token = $1 AND expires_at > now() \
         RETURNING deployment_name, user_id",
    )
    .bind(&query.token)
    .fetch_optional(&state.pg)
    .await?;

    let Some((token_deployment, user_id)) = row else {
        return Err(ApiError::Forbidden("this login link has expired — reopen the app from Helve".to_string()));
    };
    // A token minted for one deployment must not open another, even though
    // both are served by this same process.
    if token_deployment != deployment {
        return Err(ApiError::Forbidden("this login link is for a different deployment".to_string()));
    }

    let session = generate_token();
    sqlx::query(
        "INSERT INTO proxy_sessions (token, deployment_name, user_id, expires_at) \
         VALUES ($1, $2, $3, now() + make_interval(hours => $4))",
    )
    .bind(&session)
    .bind(deployment)
    .bind(user_id)
    // `hours` takes an int; only `secs` is double precision.
    .bind(PROXY_SESSION_LIFETIME_HOURS as i32)
    .execute(&state.pg)
    .await?;

    // No Domain attribute: the cookie stays host-only, so it is sent to this
    // one deployment's origin and to nothing else under the base domain.
    let secure = state.proxy_origin.as_ref().map(|o| o.is_https()).unwrap_or(false);
    let cookie = Cookie::build((PROXY_COOKIE, session))
        .path("/")
        .http_only(true)
        .secure(secure)
        .same_site(SameSite::Lax)
        .max_age(Duration::hours(PROXY_SESSION_LIFETIME_HOURS))
        .build();

    let next = safe_next_path(query.next.as_deref().unwrap_or("/"));
    Ok((CookieJar::new().add(cookie), Redirect::temporary(&next)).into_response())
}

/// The user behind a proxy origin's own cookie, if it names a live session
/// for *this* deployment. Re-resolved per request rather than trusted from
/// the cookie alone, so deleting a user (or replacing a deployment with a
/// same-named one belonging to someone else) takes effect immediately.
async fn proxy_session_user(
    state: &AppState,
    deployment: &str,
    headers: &http::HeaderMap,
) -> Result<Option<CurrentUser>, ApiError> {
    let Some(token) = headers
        .get(http::header::COOKIE)
        .and_then(|value| value.to_str().ok())
        .and_then(|cookies| cookie_value(cookies, PROXY_COOKIE))
    else {
        return Ok(None);
    };

    let row: Option<(i32,)> = sqlx::query_as(
        "SELECT user_id FROM proxy_sessions WHERE token = $1 AND deployment_name = $2 AND expires_at > now()",
    )
    .bind(&token)
    .bind(deployment)
    .fetch_optional(&state.pg)
    .await?;

    let Some((user_id,)) = row else { return Ok(None) };
    crate::auth::user_by_id(&state.pg, user_id).await
}

/// Handles `/proxy/{deployment_name}/{*rest}` — anything with at least one
/// path segment after the deployment name.
pub async fn handler(
    user: CurrentUser,
    Path((deployment_name, rest)): Path<(String, String)>,
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, ApiError> {
    if let Some(redirect) = redirect_to_own_origin(&state, &deployment_name, &req) {
        return Ok(redirect);
    }
    proxy_request(user, deployment_name, rest, state, req, false).await
}

/// Once each deployment has its own origin, the same-origin path stops
/// serving content and just points at the new location — otherwise the hole
/// per-deployment origins exist to close would stay open right alongside the
/// fix.
fn redirect_to_own_origin(state: &AppState, deployment: &str, req: &Request) -> Option<Response> {
    let origin = state.proxy_origin.as_ref()?;
    let rest = req
        .uri()
        .path()
        .strip_prefix("/proxy/")
        .and_then(|p| p.strip_prefix(deployment))
        .unwrap_or("/");
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let path = if rest.is_empty() { "/" } else { rest };
    Some(Redirect::temporary(&format!("{}{path}{query}", origin.origin_for(deployment))).into_response())
}

/// Percent-encodes a query-string value. Hand-rolled rather than pulling in
/// a URL crate for the two redirects that need it.
fn encode_query_value(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Constrains a `next` parameter to a path on the origin we're redirecting
/// to. Anything else — an absolute URL, or the `//host` protocol-relative
/// form — would turn this into an open redirect.
fn safe_next_path(next: &str) -> String {
    if next.starts_with('/') && !next.starts_with("//") { next.to_string() } else { "/".to_string() }
}

/// The value of one cookie from a `Cookie` header.
fn cookie_value(header: &str, name: &str) -> Option<String> {
    header
        .split(';')
        .map(str::trim)
        .find(|pair| cookie_name(pair) == name)
        .and_then(|pair| pair.split_once('='))
        .map(|(_, value)| value.trim().to_string())
}

/// Handles the bare `/proxy/{deployment_name}` and `/proxy/{deployment_name}/`
/// — matchit's wildcard capture in the route above requires at least one
/// character after that slash, so without this, the *exact* URL every
/// "Open" link points to (no trailing segment) would silently fall through
/// to the frontend's own SPA fallback route instead of ever reaching this
/// module. Equivalent to the wildcard route with an empty `rest`.
pub async fn handler_root(
    user: CurrentUser,
    Path(deployment_name): Path<String>,
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, ApiError> {
    if let Some(redirect) = redirect_to_own_origin(&state, &deployment_name, &req) {
        return Ok(redirect);
    }
    proxy_request(user, deployment_name, String::new(), state, req, false).await
}

async fn proxy_request(
    user: CurrentUser,
    deployment_name: String,
    // Only used when `target.strip_prefix` is set (RStudio's www-root-path
    // model) — see the path-construction note below for why JupyterLab's
    // base_url model needs the full path instead.
    rest: String,
    state: AppState,
    req: Request,
    // True when serving the deployment's own origin, where the app sits at
    // the root and the incoming path is already exactly what the pod should
    // see — no prefix to add or strip either way.
    own_origin: bool,
) -> Result<Response, ApiError> {
    let target = load_target(&state, &deployment_name).await?;
    if user.role != Role::Admin && user.username != target.owner_username {
        return Err(ApiError::Forbidden("you don't own this deployment".to_string()));
    }

    let is_upgrade = req.headers().get(http::header::UPGRADE).is_some();

    // Two different apps, two different expectations: JupyterLab's
    // `base_url` wants the full "/proxy/{name}/..." path forwarded as-is (it
    // registers its own routes under that prefix); RStudio's
    // `www-root-path` is the opposite — it only stamps that prefix onto
    // redirects/cookies sent *back* to the browser, and still expects
    // requests to arrive at the bare path, so it has to be stripped first.
    let path_and_query = if own_origin || !target.strip_prefix {
        req.uri().path_and_query().map(|pq| pq.as_str().to_string()).unwrap_or_else(|| "/".to_string())
    } else {
        match req.uri().query() {
            Some(query) => format!("/{rest}?{query}"),
            None => format!("/{rest}"),
        }
    };
    let credential = target
        .env_key
        .as_deref()
        .zip(target.secret_value.as_deref())
        .and_then(|(env_key, secret_value)| credential_header(env_key, secret_value));
    let outbound_headers = forwarded_headers(req.headers(), is_upgrade, credential.as_deref());

    let mut response =
        connect_and_stream(&state, &deployment_name, target.container_port, path_and_query, outbound_headers, is_upgrade, req)
            .await?;
    drop_session_set_cookie(response.headers_mut());
    Ok(response)
}

/// Connects to `deployment_name`'s pod via its Service's in-cluster
/// `ClusterIP` and forwards `req` into it, streaming the response (including
/// a WebSocket upgrade) back unmodified. `path_and_query` and
/// `outbound_headers` replace the request's own URI and headers — every
/// caller-specific concern (credential injection, cookie stripping/keeping,
/// prefix stripping, path-vs-token consistency checks) is decided by the
/// caller before this is called; this only knows how to reach a pod and
/// speak HTTP/1.1 (with upgrades) to it. Shared by the session-cookie
/// `/proxy/` route above (`proxy_request`) and the bearer-token `/models/`
/// route (`models_proxy.rs`), so the ~100 lines of hyper/TCP plumbing here
/// aren't duplicated between them.
pub(crate) async fn connect_and_stream(
    state: &AppState,
    deployment_name: &str,
    container_port: i32,
    path_and_query: String,
    outbound_headers: http::HeaderMap,
    is_upgrade: bool,
    mut req: Request,
) -> Result<Response, ApiError> {
    let port = container_port as u16;
    let services: Api<Service> = Api::namespaced(state.client.clone(), &state.namespace);
    let service = services
        .get(deployment_name)
        .await
        .map_err(|err| ApiError::ProxyUnavailable(format!("couldn't look up {deployment_name}'s Service: {err}")))?;
    let cluster_ip = service
        .spec
        .and_then(|spec| spec.cluster_ip)
        .filter(|ip| ip != "None")
        .ok_or_else(|| ApiError::ProxyUnavailable(format!("{deployment_name} has no ClusterIP yet")))?;

    let stream = tokio::time::timeout(std::time::Duration::from_secs(5), TcpStream::connect((cluster_ip.as_str(), port)))
        .await
        .map_err(|_| ApiError::ProxyUnavailable(format!("timed out connecting to {deployment_name}")))?
        .map_err(|err| ApiError::ProxyUnavailable(format!("couldn't reach {deployment_name}: {err}")))?;

    let (mut sender, conn) = hyper::client::conn::http1::handshake::<_, Full<Bytes>>(TokioIo::new(stream))
        .await
        .map_err(|err| ApiError::ProxyUnavailable(format!("couldn't connect to {deployment_name}: {err}")))?;
    let conn_deployment_name = deployment_name.to_string();
    tokio::spawn(async move {
        if let Err(err) = conn.with_upgrades().await {
            tracing::warn!(deployment = %conn_deployment_name, %err, "proxy connection to pod ended with error");
        }
    });

    let client_on_upgrade = is_upgrade.then(|| hyper::upgrade::on(&mut req));
    let method = req.method().clone();

    let mut builder = http::Request::builder().method(method).uri(path_and_query);
    if let Some(headers) = builder.headers_mut() {
        *headers = outbound_headers;
    }

    let body = if is_upgrade {
        Full::new(Bytes::new())
    } else {
        let bytes = to_bytes(req.into_body(), MAX_PROXIED_BODY_BYTES)
            .await
            .map_err(|_| ApiError::BadRequest("request body too large".to_string()))?;
        Full::new(bytes)
    };
    let outbound = builder
        .body(body)
        .map_err(|err| ApiError::ProxyUnavailable(format!("couldn't build proxied request: {err}")))?;

    let mut target_response = sender
        .send_request(outbound)
        .await
        .map_err(|err| ApiError::ProxyUnavailable(format!("proxied request to {deployment_name} failed: {err}")))?;

    if is_upgrade && target_response.status() == StatusCode::SWITCHING_PROTOCOLS {
        let target_upgraded = hyper::upgrade::on(&mut target_response)
            .await
            .map_err(|err| ApiError::ProxyUnavailable(format!("upstream upgrade failed: {err}")))?;
        // `is_upgrade` guarantees this was set above.
        let client_on_upgrade = client_on_upgrade.expect("client_on_upgrade set when is_upgrade");
        tokio::spawn(async move {
            match client_on_upgrade.await {
                Ok(client_upgraded) => {
                    let mut client_io = TokioIo::new(client_upgraded);
                    let mut target_io = TokioIo::new(target_upgraded);
                    if let Err(err) = tokio::io::copy_bidirectional(&mut client_io, &mut target_io).await {
                        tracing::warn!(%err, "proxied websocket tunnel ended with error");
                    }
                }
                Err(err) => tracing::warn!(%err, "client-side upgrade handshake failed"),
            }
        });
        let (parts, _) = target_response.into_parts();
        return Ok(Response::from_parts(parts, Body::empty()));
    }

    let (parts, incoming) = target_response.into_parts();
    let mut response = Response::new(Body::new(incoming));
    *response.status_mut() = parts.status;
    for (name, value) in parts.headers.iter() {
        if !is_hop_by_hop(name, false) {
            response.headers_mut().append(name.clone(), value.clone());
        }
    }
    Ok(response)
}

/// The header set forwarded to the pod: everything the client sent, minus
/// hop-by-hop headers, minus the two that would leak the caller's Helve
/// identity (their session cookie and their own `Authorization`), plus
/// whatever credential Helve manages for this deployment.
fn forwarded_headers(inbound: &http::HeaderMap, is_upgrade: bool, credential: Option<&str>) -> http::HeaderMap {
    let mut out = http::HeaderMap::new();
    for (name, value) in inbound.iter() {
        if is_hop_by_hop(name, is_upgrade) {
            continue;
        }
        // The caller's own Authorization never goes upstream — Helve injects
        // whatever credential it manages for this deployment below, and
        // forwarding the client's too would send the pod two conflicting
        // Authorization headers.
        if name == http::header::AUTHORIZATION {
            continue;
        }
        if name == http::header::COOKIE {
            // Everything except Helve's own session cookie is forwarded:
            // proxied apps set and depend on their own cookies (RStudio's
            // session, JupyterLab's XSRF token), and those come back to us
            // on this same origin.
            if let Ok(cookies) = value.to_str()
                && let Some(forwarded) = strip_session_cookie(cookies)
                && let Ok(forwarded) = forwarded.parse()
            {
                out.append(name.clone(), forwarded);
            }
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    if let Some(credential) = credential
        && let Ok(value) = credential.parse()
    {
        out.insert(http::header::AUTHORIZATION, value);
    }
    out
}

/// Cookies belonging to Helve itself, never forwarded to a pod: the app
/// session (which would hand over the caller's whole Helve identity) and
/// the proxy-origin session (which the pod has no use for, and which would
/// otherwise let it re-authenticate as its own visitor).
const HELVE_COOKIES: &[&str] = &[SESSION_COOKIE, PROXY_COOKIE];

/// Removes Helve's own cookies from a `Cookie` header on its way to a
/// proxied pod, keeping every other cookie intact. Returns `None` when
/// nothing is left to forward.
///
/// A proxied pod runs code Helve doesn't control — JupyterLab and RStudio
/// run arbitrary user code by design, and `enable_proxy` can be set on any
/// image — so handing it the caller's session cookie would hand it the
/// caller's Helve identity. That matters most for an admin, who can open
/// *anyone's* proxied app: without this, opening a hostile deployment would
/// leak an admin session token to whoever launched it.
fn strip_session_cookie(header: &str) -> Option<String> {
    let kept: Vec<&str> = header
        .split(';')
        .map(str::trim)
        .filter(|pair| !pair.is_empty())
        .filter(|pair| !HELVE_COOKIES.contains(&cookie_name(pair)))
        .collect();
    (!kept.is_empty()).then(|| kept.join("; "))
}

/// Drops any `Set-Cookie` from a proxied pod that would overwrite Helve's
/// own session cookie — otherwise a hostile pod could pin the caller's
/// browser to a session of its choosing (session fixation), since its
/// responses come back on Helve's own origin.
fn drop_session_set_cookie(headers: &mut http::HeaderMap) {
    if !headers.contains_key(http::header::SET_COOKIE) {
        return;
    }
    let kept: Vec<http::HeaderValue> = headers
        .get_all(http::header::SET_COOKIE)
        .iter()
        .filter(|value| value.to_str().map(|v| !HELVE_COOKIES.contains(&cookie_name(v))).unwrap_or(true))
        .cloned()
        .collect();
    headers.remove(http::header::SET_COOKIE);
    for value in kept {
        headers.append(http::header::SET_COOKIE, value);
    }
}

/// The name part of a `name=value` cookie pair (or of a `Set-Cookie` value,
/// whose attributes trail after the first `;`).
fn cookie_name(pair: &str) -> &str {
    let pair = pair.split(';').next().unwrap_or(pair);
    pair.split('=').next().unwrap_or("").trim()
}

/// (owner_username, env_key, secret_value, proxy_enabled, container_port, strip_prefix)
type ProxyTargetRow = (String, Option<String>, Option<String>, bool, Option<i32>, bool);

async fn load_target(state: &AppState, deployment_name: &str) -> Result<ProxyTarget, ApiError> {
    let row: Option<ProxyTargetRow> = sqlx::query_as(
        "SELECT owner_username, env_key, secret_value, proxy_enabled, container_port, strip_prefix \
         FROM deployment_secrets WHERE deployment_name = $1",
    )
    .bind(deployment_name)
    .fetch_optional(&state.pg)
    .await?;

    let not_found = || ApiError::BadRequest(format!("no proxied deployment named {deployment_name}"));
    let (owner_username, env_key, secret_value, proxy_enabled, container_port, strip_prefix) = row.ok_or_else(not_found)?;
    if !proxy_enabled {
        return Err(not_found());
    }
    let container_port = container_port
        .ok_or_else(|| ApiError::ProxyUnavailable(format!("{deployment_name} has no container_port on record")))?;
    Ok(ProxyTarget { owner_username, env_key, secret_value, container_port, strip_prefix })
}

/// The auth-header convention for each proxied template's generated
/// credential. Adding a new proxy-enabled template later means adding its
/// convention here too, alongside flipping its `proxy_enabled` flag.
fn credential_header(env_key: &str, value: &str) -> Option<String> {
    match env_key {
        "JUPYTER_TOKEN" => Some(format!("token {value}")),
        _ => None,
    }
}

pub(crate) fn is_hop_by_hop(name: &HeaderName, allow_upgrade: bool) -> bool {
    matches!(name.as_str(), "proxy-authenticate" | "proxy-authorization" | "te" | "trailers")
        || (!allow_upgrade && matches!(name.as_str(), "connection" | "keep-alive" | "transfer-encoding" | "upgrade"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds a realistic inbound header set: what a browser actually sends
    /// when an admin clicks "Open" on someone else's proxied deployment.
    fn browser_headers(cookie: &str) -> http::HeaderMap {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::HOST, "helve.example".parse().unwrap());
        headers.insert(http::header::USER_AGENT, "Mozilla/5.0".parse().unwrap());
        headers.insert(http::header::COOKIE, cookie.parse().unwrap());
        headers
    }

    #[test]
    fn session_cookie_never_reaches_the_pod() {
        // The whole point of the fix: a pod runs code Helve doesn't control,
        // so an admin opening a hostile deployment must not hand it their
        // session token.
        let inbound = browser_headers("helve_session=ADMIN_TOKEN; _xsrf=abc");
        let out = forwarded_headers(&inbound, false, None);

        let cookie = out.get(http::header::COOKIE).unwrap().to_str().unwrap();
        assert!(!cookie.contains("ADMIN_TOKEN"), "session token leaked to pod: {cookie}");
        assert!(!cookie.contains("helve_session"));
        assert_eq!(cookie, "_xsrf=abc");
    }

    #[test]
    fn cookie_header_omitted_when_only_the_session_cookie_was_sent() {
        let inbound = browser_headers("helve_session=ADMIN_TOKEN");
        let out = forwarded_headers(&inbound, false, None);
        assert!(!out.contains_key(http::header::COOKIE));
        // Unrelated headers still get through.
        assert_eq!(out.get(http::header::USER_AGENT).unwrap(), "Mozilla/5.0");
    }

    #[test]
    fn callers_authorization_is_replaced_by_helves_own_credential() {
        let mut inbound = browser_headers("helve_session=ADMIN_TOKEN");
        inbound.insert(http::header::AUTHORIZATION, "Bearer CALLER".parse().unwrap());

        let out = forwarded_headers(&inbound, false, Some("token GENERATED"));

        let auth: Vec<&str> = out.get_all(http::header::AUTHORIZATION).iter().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(auth, vec!["token GENERATED"], "expected exactly one Authorization header");
    }

    #[test]
    fn drops_hop_by_hop_headers_but_keeps_upgrade_ones_when_upgrading() {
        let mut inbound = browser_headers("_xsrf=abc");
        inbound.insert(http::header::CONNECTION, "Upgrade".parse().unwrap());
        inbound.insert(http::header::UPGRADE, "websocket".parse().unwrap());

        // A normal request: hop-by-hop headers are stripped.
        let plain = forwarded_headers(&inbound, false, None);
        assert!(!plain.contains_key(http::header::CONNECTION));
        assert!(!plain.contains_key(http::header::UPGRADE));

        // A WebSocket upgrade: they must survive or the tunnel never forms
        // (JupyterLab's kernel connection depends on this).
        let upgraded = forwarded_headers(&inbound, true, None);
        assert_eq!(upgraded.get(http::header::UPGRADE).unwrap(), "websocket");
        assert_eq!(upgraded.get(http::header::CONNECTION).unwrap(), "Upgrade");
    }

    #[test]
    fn requests_without_cookies_are_forwarded_unchanged() {
        let mut inbound = http::HeaderMap::new();
        inbound.insert(http::header::ACCEPT, "text/html".parse().unwrap());
        let out = forwarded_headers(&inbound, false, None);
        assert_eq!(out.get(http::header::ACCEPT).unwrap(), "text/html");
        assert!(!out.contains_key(http::header::COOKIE));
    }

    #[test]
    fn next_path_rejects_anything_that_leaves_this_origin() {
        // An open redirect here would let a crafted "Open" link bounce a
        // logged-in user to an attacker's site straight from Helve.
        assert_eq!(safe_next_path("/lab/tree?a=1"), "/lab/tree?a=1");
        assert_eq!(safe_next_path("//evil.example/path"), "/");
        assert_eq!(safe_next_path("https://evil.example"), "/");
        assert_eq!(safe_next_path("javascript:alert(1)"), "/");
        assert_eq!(safe_next_path(""), "/");
        assert_eq!(safe_next_path("lab"), "/");
    }

    #[test]
    fn query_values_are_percent_encoded() {
        assert_eq!(encode_query_value("/lab/tree?a=1&b=2"), "%2Flab%2Ftree%3Fa%3D1%26b%3D2");
        assert_eq!(encode_query_value("plain-name_1.0~"), "plain-name_1.0~");
        // Anything that could break out of the query parameter must not
        // survive unescaped.
        assert_eq!(encode_query_value("a b"), "a%20b");
    }

    #[test]
    fn reads_one_cookie_out_of_a_header() {
        assert_eq!(cookie_value("a=1; helve_proxy=TOK; b=2", PROXY_COOKIE), Some("TOK".to_string()));
        assert_eq!(cookie_value("helve_proxy=TOK", PROXY_COOKIE), Some("TOK".to_string()));
        assert_eq!(cookie_value("a=1; b=2", PROXY_COOKIE), None);
        // Must not be fooled by a name that merely contains the real one.
        assert_eq!(cookie_value("not_helve_proxy=NOPE", PROXY_COOKIE), None);
    }

    #[test]
    fn proxy_origin_cookie_is_also_kept_from_the_pod() {
        // The pod has no use for it, and it should not be able to replay its
        // own visitor's proxy session.
        let inbound = browser_headers("helve_session=S; helve_proxy=P; keep=1");
        let out = forwarded_headers(&inbound, false, None);
        assert_eq!(out.get(http::header::COOKIE).unwrap(), "keep=1");
    }

    #[test]
    fn strips_only_helves_own_cookie() {
        // The pod still needs its own cookies (RStudio's session, JupyterLab's
        // XSRF token) — only Helve's session may not cross this boundary.
        assert_eq!(
            strip_session_cookie("csrftoken=abc; helve_session=SECRET; _xsrf=def"),
            Some("csrftoken=abc; _xsrf=def".to_string())
        );
    }

    #[test]
    fn strips_session_cookie_in_any_position() {
        for header in [
            "helve_session=SECRET; keep=1",
            "keep=1; helve_session=SECRET",
            "  helve_session=SECRET  ;  keep=1  ",
        ] {
            assert_eq!(strip_session_cookie(header), Some("keep=1".to_string()), "header: {header}");
        }
    }

    #[test]
    fn drops_header_entirely_when_only_session_cookie_present() {
        assert_eq!(strip_session_cookie("helve_session=SECRET"), None);
        assert_eq!(strip_session_cookie(""), None);
    }

    #[test]
    fn leaves_unrelated_cookies_untouched() {
        assert_eq!(strip_session_cookie("a=1; b=2"), Some("a=1; b=2".to_string()));
    }

    #[test]
    fn does_not_match_on_name_substrings() {
        // Guards against a prefix/contains-style check letting the real
        // cookie through, or eating an unrelated one.
        let header = "not_helve_session=keep; helve_session_x=keep2; helve_session=SECRET";
        assert_eq!(
            strip_session_cookie(header),
            Some("not_helve_session=keep; helve_session_x=keep2".to_string())
        );
    }

    #[test]
    fn handles_values_containing_equals() {
        // Base64-ish values contain '='; splitting must not lose them.
        assert_eq!(
            strip_session_cookie("tok=YWJj==; helve_session=SECRET"),
            Some("tok=YWJj==".to_string())
        );
    }

    #[test]
    fn drops_upstream_attempt_to_overwrite_the_session_cookie() {
        let mut headers = http::HeaderMap::new();
        headers.append(http::header::SET_COOKIE, "helve_session=ATTACKER; Path=/".parse().unwrap());
        headers.append(http::header::SET_COOKIE, "rstudio-session=legit; Path=/".parse().unwrap());

        drop_session_set_cookie(&mut headers);

        let remaining: Vec<&str> =
            headers.get_all(http::header::SET_COOKIE).iter().map(|v| v.to_str().unwrap()).collect();
        assert_eq!(remaining, vec!["rstudio-session=legit; Path=/"]);
    }

    #[test]
    fn leaves_responses_without_set_cookie_alone() {
        let mut headers = http::HeaderMap::new();
        headers.insert(http::header::CONTENT_TYPE, "text/html".parse().unwrap());

        drop_session_set_cookie(&mut headers);

        assert_eq!(headers.len(), 1);
        assert!(!headers.contains_key(http::header::SET_COOKIE));
    }
}
