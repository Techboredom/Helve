use std::net::SocketAddr;

use argon2::password_hash::{phc::PasswordHash, PasswordHasher, PasswordVerifier};
use argon2::Argon2;
use axum::extract::{ConnectInfo, FromRequestParts, State};
use axum::http::header::{AUTHORIZATION, USER_AGENT};
use axum::http::request::Parts;
use axum::http::HeaderMap;
use axum::Json;
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use common::{ChangePasswordRequest, GroupInfo, LoginRequest, Role, SessionLogEntry, UserInfo};
use rand::distr::Alphanumeric;
use rand::RngExt;
use sha2::{Digest, Sha256};
use sqlx::types::Json as SqlxJson;
use sqlx::{AssertSqlSafe, FromRow};
use time::Duration;

use crate::error::ApiError;
use crate::state::AppState;
use crate::users::GROUPS_JSON;
use crate::validate;

pub const SESSION_COOKIE: &str = "helve_session";
const SESSION_LIFETIME_DAYS: i64 = 7;

pub fn hash_password(password: &str) -> Result<String, ApiError> {
    Argon2::default()
        .hash_password(password.as_bytes())
        .map(|h| h.to_string())
        .map_err(|err| ApiError::BadRequest(format!("failed to hash password: {err}")))
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    let Ok(parsed) = PasswordHash::new(hash) else { return false };
    Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok()
}

/// A high-entropy random token, used both for session cookies and for
/// auto-generated app credentials (JupyterLab tokens, RStudio passwords, ...).
pub fn generate_token() -> String {
    rand::rng().sample_iter(Alphanumeric).take(48).map(char::from).collect()
}

/// A short random suffix for auto-generated deployment names
/// (`<username>-<instance-type>-<suffix>`, see deployments.rs). Lowercased
/// on purpose — `Alphanumeric` samples upper+lower+digit, but a Kubernetes
/// name only allows lowercase — this isn't a security token, so folding
/// case just to stay in-alphabet costs nothing worth worrying about here,
/// unlike `generate_token` above.
pub fn generate_name_suffix() -> String {
    rand::rng().sample_iter(Alphanumeric).take(6).map(|b| (b as char).to_ascii_lowercase()).collect()
}

/// A high-entropy API token for admin automation (`POST /api/tokens`),
/// prefixed so a leaked value is easy to recognize at a glance or catch
/// with a secret scanner — the same idea as GitHub's `ghp_`/`gho_` etc.
/// Unlike `generate_token`'s session cookies and app credentials, this is
/// explicitly meant to be copied out of a browser once and handed to a
/// script or CI system, so it gets a distinguishing prefix those don't need.
pub fn generate_api_token() -> String {
    format!("aat_{}", generate_token())
}

/// Hashes an API token for storage/lookup (`api_tokens.token_hash`) — a
/// fast, unsalted SHA-256, not `hash_password`'s deliberately-slow argon2.
/// Slow hashing defends against *guessing* a low-entropy secret (a human
/// password); this token is 48 random alphanumeric characters, already
/// far too high-entropy to brute-force, so the only job here is turning "a
/// database dump" into "not immediately a working credential" without
/// adding real latency to every single API-token-authenticated request.
pub fn hash_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

/// The authenticated caller, extracted from the `helve_session` cookie.
/// Any endpoint that takes this as a parameter requires a logged-in user of
/// either role — use `AdminUser` instead for admin-only endpoints.
#[derive(Clone, Debug)]
pub struct CurrentUser {
    pub id: i32,
    pub username: String,
    pub role: Role,
    /// Admin-set "key=value" node label, if any — see `common::UserInfo::node_label`.
    pub node_label: Option<String>,
    /// Admin-set UID/GID, if any — see `common::UserInfo::uid`/`gid`.
    pub uid: Option<i32>,
    pub gid: Option<i32>,
    /// Admin-set named-group membership — see `common::UserInfo::supplemental_groups`.
    pub supplemental_groups: Vec<GroupInfo>,
}

#[derive(FromRow)]
struct SessionUserRow {
    id: i32,
    username: String,
    role: String,
    node_label: Option<String>,
    uid: Option<i32>,
    gid: Option<i32>,
    supplemental_groups: SqlxJson<Vec<GroupInfo>>,
}

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    /// Session cookie first (the browser path); if there's no cookie at all,
    /// falls back to an `Authorization: Bearer <token>` header (the
    /// automation path — see `tokens.rs`). A request carrying a cookie is
    /// never allowed to also fall back to a bearer token if that cookie
    /// turns out to be invalid/expired — that's not a scenario a real
    /// browser or script should ever hit, so there's nothing to gain from
    /// supporting it, only ambiguity to invite.
    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let jar = CookieJar::from_request_parts(parts, state).await.expect("infallible");
        if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) {
            let sql = format!(
                "SELECT u.id, u.username, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups \
                 FROM sessions s JOIN users u ON u.id = s.user_id \
                 WHERE s.token = $1 AND s.expires_at > now()"
            );
            let row: Option<SessionUserRow> =
                sqlx::query_as(AssertSqlSafe(sql)).bind(&token).fetch_optional(&state.pg).await.map_err(ApiError::from)?;

            let row = row.ok_or(ApiError::Unauthorized)?;
            let role = if row.role == "admin" { Role::Admin } else { Role::User };
            return Ok(CurrentUser {
                id: row.id,
                username: row.username,
                role,
                node_label: row.node_label,
                uid: row.uid,
                gid: row.gid,
                supplemental_groups: row.supplemental_groups.0,
            });
        }

        let bearer =
            parts.headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer "));
        if let Some(token) = bearer
            && let Some(user) = user_from_api_token(&state.pg, token).await?
        {
            return Ok(user);
        }
        Err(ApiError::Unauthorized)
    }
}

#[derive(FromRow)]
struct ApiTokenUserRow {
    token_id: i32,
    id: i32,
    username: String,
    role: String,
    node_label: Option<String>,
    uid: Option<i32>,
    gid: Option<i32>,
    supplemental_groups: SqlxJson<Vec<GroupInfo>>,
}

/// Resolves an `Authorization: Bearer <token>` value to the account that
/// created it, and records the attempt in `last_used_at` so an admin can
/// tell a stale token from one still in active use before revoking it.
async fn user_from_api_token(pg: &sqlx::PgPool, token: &str) -> Result<Option<CurrentUser>, ApiError> {
    let hash = hash_token(token);
    let sql = format!(
        "SELECT t.id AS token_id, u.id, u.username, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups \
         FROM api_tokens t JOIN users u ON u.id = t.user_id \
         WHERE t.token_hash = $1"
    );
    let row: Option<ApiTokenUserRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(&hash).fetch_optional(pg).await?;
    let Some(row) = row else { return Ok(None) };

    sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1").bind(row.token_id).execute(pg).await?;
    Ok(Some(CurrentUser {
        id: row.id,
        username: row.username,
        role: if row.role == "admin" { Role::Admin } else { Role::User },
        node_label: row.node_label,
        uid: row.uid,
        gid: row.gid,
        supplemental_groups: row.supplemental_groups.0,
    }))
}

/// Loads a user directly by id, for the paths that establish identity from
/// something other than the `helve_session` cookie — specifically a proxy
/// origin's own session (see `proxy.rs`), which lives on a different host and
/// therefore never receives that cookie.
pub async fn user_by_id(pg: &sqlx::PgPool, id: i32) -> Result<Option<CurrentUser>, ApiError> {
    let sql = format!(
        "SELECT u.id, u.username, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups \
         FROM users u WHERE u.id = $1"
    );
    let row: Option<SessionUserRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(id).fetch_optional(pg).await?;
    Ok(row.map(|row| CurrentUser {
        id: row.id,
        username: row.username,
        role: if row.role == "admin" { Role::Admin } else { Role::User },
        node_label: row.node_label,
        uid: row.uid,
        gid: row.gid,
        supplemental_groups: row.supplemental_groups.0,
    }))
}

/// Same as `CurrentUser`, but rejects non-admins with 403. Use this as the
/// parameter type for admin-only endpoints (Templates writes, Users management).
pub struct AdminUser(pub CurrentUser);

impl FromRequestParts<AppState> for AdminUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let user = CurrentUser::from_request_parts(parts, state).await?;
        if user.role != Role::Admin {
            return Err(ApiError::Forbidden("admin role required".to_string()));
        }
        Ok(AdminUser(user))
    }
}

#[derive(FromRow)]
struct UserAuthRow {
    id: i32,
    username: String,
    password_hash: String,
    role: String,
    node_label: Option<String>,
    uid: Option<i32>,
    gid: Option<i32>,
    supplemental_groups: SqlxJson<Vec<GroupInfo>>,
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Json(req): Json<LoginRequest>,
) -> Result<(CookieJar, Json<UserInfo>), ApiError> {
    // Checked before argon2 runs, so a flood of junk logins can't be turned
    // into a CPU exhaustion attack.
    if state.login_blocked(addr.ip()).await {
        return Err(ApiError::TooManyRequests(
            "too many failed sign-in attempts from this address — wait a few minutes and try again".to_string(),
        ));
    }

    let sql = format!(
        "SELECT u.id, u.username, u.password_hash, u.role, u.node_label, u.uid, u.gid, {GROUPS_JSON} AS supplemental_groups \
         FROM users u WHERE u.username = $1"
    );
    let row: Option<UserAuthRow> = sqlx::query_as(AssertSqlSafe(sql)).bind(&req.username).fetch_optional(&state.pg).await?;

    let row = row.filter(|r| verify_password(&r.password_hash, &req.password));
    let Some(row) = row else {
        state.record_login_failure(addr.ip()).await;
        return Err(ApiError::Unauthorized);
    };
    state.clear_login_failures(addr.ip()).await;

    let token = generate_token();
    sqlx::query("INSERT INTO sessions (token, user_id, expires_at) VALUES ($1, $2, now() + make_interval(days => $3))")
        .bind(&token)
        .bind(row.id)
        .bind(SESSION_LIFETIME_DAYS as i32)
        .execute(&state.pg)
        .await?;

    // Kept for support/metrics ("when did this user last log in, from
    // where, with what browser") — separate from `sessions` above, which is
    // deleted on logout/invalidation and only used for live auth checks.
    let user_agent = headers.get(USER_AGENT).and_then(|v| v.to_str().ok());
    sqlx::query("INSERT INTO session_log (user_id, ip_address, user_agent) VALUES ($1, $2, $3)")
        .bind(row.id)
        .bind(addr.ip().to_string())
        .bind(user_agent)
        .execute(&state.pg)
        .await?;

    let cookie = Cookie::build((SESSION_COOKIE, token))
        .path("/")
        .http_only(true)
        // Only when the app is actually served over HTTPS: a `Secure` cookie
        // is never sent back over plain HTTP, which would lock out a
        // plain-HTTP deployment entirely. See AppState::cookies_secure.
        .secure(state.cookies_secure())
        .same_site(SameSite::Lax)
        .max_age(Duration::days(SESSION_LIFETIME_DAYS))
        .build();

    let role = if row.role == "admin" { Role::Admin } else { Role::User };
    Ok((
        jar.add(cookie),
        Json(UserInfo {
            id: row.id,
            username: row.username,
            role,
            node_label: row.node_label,
            uid: row.uid,
            gid: row.gid,
            supplemental_groups: row.supplemental_groups.0,
        }),
    ))
}

pub async fn logout(State(state): State<AppState>, jar: CookieJar) -> Result<CookieJar, ApiError> {
    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) {
        sqlx::query("DELETE FROM sessions WHERE token = $1").bind(&token).execute(&state.pg).await?;
    }
    // Must match the `path` the cookie was set with (login's Set-Cookie), or the
    // browser treats this as a different cookie and never actually clears the session.
    Ok(jar.remove(Cookie::build((SESSION_COOKIE, "")).path("/").build()))
}

pub async fn me(user: CurrentUser) -> Json<UserInfo> {
    Json(UserInfo {
        id: user.id,
        username: user.username,
        role: user.role,
        node_label: user.node_label,
        uid: user.uid,
        gid: user.gid,
        supplemental_groups: user.supplemental_groups,
    })
}

/// Lets a logged-in user change their own password, proving they know the
/// current one first (unlike an admin's reset). Invalidates every other
/// session for the account, but leaves the one making this request logged in.
pub async fn change_password(
    user: CurrentUser,
    State(state): State<AppState>,
    jar: CookieJar,
    Json(req): Json<ChangePasswordRequest>,
) -> Result<(), ApiError> {
    let current_hash: String =
        sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1").bind(user.id).fetch_one(&state.pg).await?;
    if !verify_password(&current_hash, &req.current_password) {
        return Err(ApiError::BadRequest("current password is incorrect".to_string()));
    }
    validate::password(&req.new_password)?;

    let new_hash = hash_password(&req.new_password)?;
    sqlx::query("UPDATE users SET password_hash = $1 WHERE id = $2").bind(&new_hash).bind(user.id).execute(&state.pg).await?;

    if let Some(token) = jar.get(SESSION_COOKIE).map(|c| c.value().to_string()) {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND token != $2")
            .bind(user.id)
            .bind(&token)
            .execute(&state.pg)
            .await?;
    }
    Ok(())
}

#[derive(FromRow)]
struct SessionLogRow {
    username: String,
    created_at: String,
    ip_address: Option<String>,
    user_agent: Option<String>,
}

impl From<SessionLogRow> for SessionLogEntry {
    fn from(row: SessionLogRow) -> Self {
        SessionLogEntry { username: row.username, created_at: row.created_at, ip_address: row.ip_address, user_agent: row.user_agent }
    }
}

/// Login history: everyone's, for an admin; only your own, for a `user`
/// account — same visibility split as the Pods tab. `username` is always
/// included either way; the frontend just hides that column for non-admins,
/// since a `user`-role response is already server-side filtered to just them.
pub async fn list_sessions(user: CurrentUser, State(state): State<AppState>) -> Result<Json<Vec<SessionLogEntry>>, ApiError> {
    let rows: Vec<SessionLogRow> = if user.role == Role::Admin {
        sqlx::query_as(
            "SELECT u.username, l.created_at::text, l.ip_address, l.user_agent \
             FROM session_log l JOIN users u ON u.id = l.user_id \
             ORDER BY l.created_at DESC LIMIT 200",
        )
        .fetch_all(&state.pg)
        .await?
    } else {
        sqlx::query_as(
            "SELECT u.username, l.created_at::text, l.ip_address, l.user_agent \
             FROM session_log l JOIN users u ON u.id = l.user_id \
             WHERE l.user_id = $1 ORDER BY l.created_at DESC LIMIT 200",
        )
        .bind(user.id)
        .fetch_all(&state.pg)
        .await?
    };
    Ok(Json(rows.into_iter().map(SessionLogEntry::from).collect()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn api_tokens_are_prefixed_and_high_entropy() {
        let token = generate_api_token();
        assert!(token.starts_with("aat_"), "expected an \"aat_\" prefix, got {token}");
        assert_eq!(token.len(), "aat_".len() + 48);
        assert_ne!(generate_api_token(), generate_api_token(), "two tokens should never collide");
    }

    #[test]
    fn token_hashing_is_deterministic_but_not_reversible_by_inspection() {
        let token = "aat_exampletoken";
        assert_eq!(hash_token(token), hash_token(token), "the same token must always hash the same way");
        assert_ne!(hash_token(token), token, "the hash must not just be the input echoed back");
        assert_ne!(hash_token("aat_exampletokex"), hash_token(token), "a one-character difference must change the hash");
    }
}
