//! OIDC (SSO) login: a redirect-based alternative to local password and LDAP
//! login, independent of and simultaneous with both. `AppState::oidc` is
//! `None` unless `OIDC_ISSUER_URL`/`OIDC_CLIENT_ID`/`OIDC_CLIENT_SECRET` were
//! all set at startup, in which case provider discovery has already run
//! once (`Oidc::discover`, called from `main`) and its JWKS are cached by
//! `openidconnect` itself.
//!
//! Two handlers back the flow: [`login`] builds the authorization URL and
//! redirects the browser to the IdP; [`callback`] is where the IdP redirects
//! back with a `code`, which gets exchanged for an ID token, verified, and
//! turned into a Helve session via `auth::establish_session` — the same
//! helper local password and LDAP login both end at.

use std::net::SocketAddr;

use axum::extract::{ConnectInfo, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum::Json;
use axum_extra::extract::cookie::CookieJar;
use base64::Engine;
use common::AuthConfig;
use openidconnect::core::{CoreAuthenticationFlow, CoreClient, CoreProviderMetadata};
use openidconnect::{
    reqwest, AuthorizationCode, ClaimsVerificationError, ClientId, ClientSecret, CsrfToken, IssuerUrl, Nonce, PkceCodeChallenge,
    PkceCodeVerifier, RedirectUrl, Scope, TokenResponse,
};

use crate::auth::establish_session;
use crate::error::ApiError;
use crate::state::{AppState, OidcConfig};
use crate::validate;

/// The discovered provider metadata plus the static config it was built
/// from — held once in `AppState` rather than rediscovered per-request, so
/// a request never blocks on a round trip to the IdP just to find out
/// where its endpoints are.
pub struct Oidc {
    pub config: OidcConfig,
    /// Discovery's raw result, not a `CoreClient` built from it: this
    /// crate's client is a typestate builder whose exact type depends on
    /// which endpoints are configured (`set_redirect_uri` alone changes
    /// it), which makes it awkward to name as a struct field. Cloning this
    /// (cheap — an in-memory struct, no network I/O) and building a fresh
    /// client from it per request, exactly like `discover` does once below,
    /// sidesteps that entirely.
    provider_metadata: CoreProviderMetadata,
    client_id: ClientId,
    client_secret: ClientSecret,
    redirect_url: RedirectUrl,
    http: reqwest::Client,
}

impl Oidc {
    pub async fn discover(config: OidcConfig, redirect_url: String) -> anyhow::Result<Self> {
        let http = reqwest::ClientBuilder::new()
            // Discovery/token/JWKS requests only ever go to the configured
            // issuer's own endpoints. Refusing to follow a redirect is
            // openidconnect's own documented SSRF hardening for this client.
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let provider_metadata = CoreProviderMetadata::discover_async(IssuerUrl::new(config.issuer_url.clone())?, &http).await?;
        let client_id = ClientId::new(config.client_id.clone());
        let client_secret = ClientSecret::new(config.client_secret.clone());
        let redirect_url = RedirectUrl::new(redirect_url)?;
        Ok(Self { config, provider_metadata, client_id, client_secret, redirect_url, http })
    }
}

/// Builds a fresh, fully-configured client from `oidc`'s stored discovery
/// result — no network I/O, just struct assembly, so building it locally
/// wherever it's needed (rather than storing one on `Oidc` — see its
/// doc comment) costs nothing worth caching. Never named as a return
/// type: `openidconnect`'s typestate builder gives a different concrete
/// type depending on which `.set_*_uri` calls were made, so this is a
/// macro rather than a function — a function would have to name that type.
macro_rules! oidc_client {
    ($oidc:expr) => {
        CoreClient::from_provider_metadata($oidc.provider_metadata.clone(), $oidc.client_id.clone(), Some($oidc.client_secret.clone()))
            .set_redirect_uri($oidc.redirect_url.clone())
    };
}

/// Unauthenticated — lets the login page decide whether to show an SSO
/// button before anyone has signed in.
pub async fn auth_config(State(state): State<AppState>) -> Json<AuthConfig> {
    Json(AuthConfig { oidc_enabled: state.oidc.is_some() })
}

/// `GET /api/auth/oidc/login` — a full-page navigation target, not an XHR
/// endpoint (the frontend just points a link/button here), since the whole
/// point is to leave Helve's origin for the IdP's.
pub async fn login(State(state): State<AppState>) -> Result<Response, ApiError> {
    let Some(oidc) = &state.oidc else { return Err(ApiError::BadRequest("SSO is not configured".to_string())) };

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let client = oidc_client!(oidc);
    let (auth_url, csrf_token, nonce) = client
        .authorize_url(CoreAuthenticationFlow::AuthorizationCode, CsrfToken::new_random, Nonce::new_random)
        .add_scope(Scope::new("profile".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    sqlx::query("INSERT INTO oidc_flow_state (state, pkce_verifier, nonce) VALUES ($1, $2, $3)")
        .bind(csrf_token.secret())
        .bind(pkce_verifier.secret())
        .bind(nonce.secret())
        .execute(&state.pg)
        .await?;

    Ok(Redirect::temporary(auth_url.as_str()).into_response())
}

#[derive(serde::Deserialize)]
pub struct CallbackQuery {
    code: String,
    state: String,
}

/// `GET /api/auth/oidc/callback` — where the IdP redirects back with an
/// authorization code. Every failure past the missing-state check is
/// treated as a clean redirect to `/` rather than an error response: from
/// here on, a failure means either a replayed/forged/expired callback (not
/// this server's fault) or an IdP-side hiccup, neither of which should look
/// like Helve itself crashed.
pub async fn callback(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    jar: CookieJar,
    Query(q): Query<CallbackQuery>,
) -> Result<Response, ApiError> {
    let Some(oidc) = &state.oidc else { return Ok(Redirect::temporary("/").into_response()) };

    // Single-use: deleted the instant it's looked up, whether or not
    // everything past this point succeeds — a state value must never be
    // replayable. A row that's missing (already consumed) or older than
    // 10 minutes is treated identically to "no such state".
    let row: Option<(String, String)> = sqlx::query_as(
        "DELETE FROM oidc_flow_state WHERE state = $1 AND created_at > now() - interval '10 minutes' \
         RETURNING pkce_verifier, nonce",
    )
    .bind(&q.state)
    .fetch_optional(&state.pg)
    .await?;
    let Some((pkce_verifier, nonce)) = row else {
        tracing::warn!("OIDC callback with an unknown or expired state — dropping it");
        return Ok(Redirect::temporary("/").into_response());
    };
    let nonce = Nonce::new(nonce);

    let client = oidc_client!(oidc);
    let exchange = match client.exchange_code(AuthorizationCode::new(q.code)) {
        Ok(exchange) => exchange,
        Err(err) => {
            tracing::warn!(error = %err, "failed to build OIDC token exchange request");
            return Ok(Redirect::temporary("/").into_response());
        }
    };
    let token_response = match exchange.set_pkce_verifier(PkceCodeVerifier::new(pkce_verifier)).request_async(&oidc.http).await {
        Ok(resp) => resp,
        Err(err) => {
            tracing::warn!(error = %err, "OIDC code exchange failed");
            return Ok(Redirect::temporary("/").into_response());
        }
    };

    let Some(id_token) = token_response.id_token() else {
        tracing::warn!("OIDC token response had no id_token");
        return Ok(Redirect::temporary("/").into_response());
    };

    let verifier = client.id_token_verifier();
    let claims: Result<_, ClaimsVerificationError> = id_token.claims(&verifier, &nonce);
    let claims = match claims {
        Ok(claims) => claims,
        Err(err) => {
            tracing::warn!(error = %err, "OIDC id_token verification failed");
            return Ok(Redirect::temporary("/").into_response());
        }
    };
    let subject = claims.subject().as_str().to_string();

    // `username_claim`/`groups_claim` name arbitrary, admin-configured
    // claims that openidconnect's typed `IdTokenClaims` has no field for —
    // read straight from the token's own JSON payload instead. Safe to
    // trust without a second signature check: `id_token.claims()` above
    // already verified this exact token's signature: no bytes here were
    // ever in question.
    let raw_claims = match decode_jwt_payload(&id_token.to_string()) {
        Ok(v) => v,
        Err(err) => {
            tracing::warn!(error = %err, "failed to decode OIDC id_token payload");
            return Ok(Redirect::temporary("/").into_response());
        }
    };
    let (claimed_username, is_admin) = extract_identity(&raw_claims, &oidc.config);

    let user_id = resolve_user(&state, &oidc.config, &subject, claimed_username.as_deref(), is_admin).await?;
    let (jar, _user) = establish_session(&state, user_id, addr, &headers, jar).await?;
    Ok((jar, Redirect::temporary("/")).into_response())
}

/// Pulls `username_claim`/`groups_claim` (both admin-configured claim
/// *names*, so a generic `serde_json::Value` lookup rather than a typed
/// accessor) out of a decoded ID token payload, and reduces group
/// membership to a single `is_admin` bool. A missing/non-array groups
/// claim, or no `admin_group` configured at all, is indistinguishable from
/// "no matching group" — both yield `false`, never a silent default to
/// `true`.
fn extract_identity(raw_claims: &serde_json::Value, config: &OidcConfig) -> (Option<String>, bool) {
    let claimed_username = raw_claims.get(&config.username_claim).and_then(|v| v.as_str()).map(str::to_string);
    let groups: Vec<&str> = raw_claims.get(&config.groups_claim).and_then(|v| v.as_array()).map_or_else(Vec::new, |items| {
        items.iter().filter_map(|v| v.as_str()).collect()
    });
    let is_admin = config.admin_group.as_deref().is_some_and(|g| groups.contains(&g));
    (claimed_username, is_admin)
}

/// Base64url-decodes an already-signature-verified compact JWT's payload
/// segment back to JSON, purely to reach claims the typed `IdTokenClaims`
/// struct doesn't have fields for (see `callback` above).
fn decode_jwt_payload(compact: &str) -> Result<serde_json::Value, String> {
    let payload = compact.split('.').nth(1).ok_or("malformed ID token (not three dot-separated segments)")?;
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(payload).map_err(|err| err.to_string())?;
    serde_json::from_slice(&bytes).map_err(|err| err.to_string())
}

/// Resolves a verified OIDC identity (`subject`, plus whatever the
/// configured claims yielded) to a Helve user id, provisioning or linking
/// an account as `config.auto_provision` allows. See `OidcConfig::auto_provision`
/// and `LdapConfig::auto_provision` for the shared reasoning; the one
/// OIDC-specific wrinkle is the pre-created-account linking path below,
/// which only exists when auto-provisioning is off.
async fn resolve_user(
    state: &AppState,
    config: &OidcConfig,
    subject: &str,
    claimed_username: Option<&str>,
    is_admin: bool,
) -> Result<i32, ApiError> {
    let sync_role = |user_id: i32| async move {
        if config.admin_group.is_some() {
            let role = if is_admin { "admin" } else { "user" };
            sqlx::query("UPDATE users SET role = $1 WHERE id = $2").bind(role).bind(user_id).execute(&state.pg).await?;
        }
        Ok::<(), ApiError>(())
    };

    // Every login after the first is identified by `sub` alone — this is
    // the only OIDC-spec-guaranteed-stable identifier, unlike email/username.
    if let Some(id) = sqlx::query_scalar::<_, i32>("SELECT id FROM users WHERE oidc_subject = $1").bind(subject).fetch_optional(&state.pg).await? {
        sync_role(id).await?;
        return Ok(id);
    }

    let Some(username) = claimed_username else {
        return Err(ApiError::Forbidden(format!(
            "SSO login succeeded, but the configured username claim (\"{}\") was missing from the ID token",
            config.username_claim
        )));
    };

    if !config.auto_provision {
        // The one case where an *existing* local account gets linked to a
        // new subject: it was pre-created by an admin specifically to be
        // claimed this way, and has never been linked to any subject yet.
        // This never happens when auto_provision is true (see below) — an
        // account otherwise reachable by guessing/registering a matching
        // username must never be silently annexed by a new SSO identity.
        let existing: Option<i32> =
            sqlx::query_scalar("SELECT id FROM users WHERE username = $1 AND oidc_subject IS NULL").bind(username).fetch_optional(&state.pg).await?;
        let Some(id) = existing else {
            return Err(ApiError::Forbidden(format!(
                "\"{username}\" authenticated via SSO, but no matching pre-created Helve account was found — contact an admin"
            )));
        };
        sqlx::query("UPDATE users SET oidc_subject = $1 WHERE id = $2").bind(subject).bind(id).execute(&state.pg).await?;
        sync_role(id).await?;
        return Ok(id);
    }

    validate::username(username)?;
    let role = if is_admin { "admin" } else { "user" };
    let id: i32 = sqlx::query_scalar("INSERT INTO users (username, role, auth_source, oidc_subject) VALUES ($1, $2, 'oidc', $3) RETURNING id")
        .bind(username)
        .bind(role)
        .bind(subject)
        .fetch_one(&state.pg)
        .await
        .map_err(|err| match err {
            // Never falls back to linking: a username collision with
            // auto-provisioning on is refused outright, not silently
            // attached to whatever account already holds that name.
            sqlx::Error::Database(db_err) if db_err.is_unique_violation() => ApiError::BadRequest(format!(
                "an account named \"{username}\" already exists but isn't linked to this SSO identity — ask an admin to link it (turn off auto-provisioning to enable linking) or rename one of them"
            )),
            other => ApiError::from(other),
        })?;
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config(admin_group: Option<&str>) -> OidcConfig {
        OidcConfig {
            issuer_url: "https://idp.example.com".to_string(),
            client_id: "helve".to_string(),
            client_secret: "secret".to_string(),
            username_claim: "preferred_username".to_string(),
            groups_claim: "groups".to_string(),
            admin_group: admin_group.map(str::to_string),
            auto_provision: true,
        }
    }

    #[test]
    fn reads_the_configured_username_and_groups_claims() {
        let claims = serde_json::json!({
            "sub": "abc123",
            "preferred_username": "alice",
            "groups": ["helve-users", "helve-admins"],
        });
        let (username, is_admin) = extract_identity(&claims, &config(Some("helve-admins")));
        assert_eq!(username.as_deref(), Some("alice"));
        assert!(is_admin);
    }

    #[test]
    fn no_admin_group_configured_never_grants_admin() {
        let claims = serde_json::json!({ "preferred_username": "alice", "groups": ["helve-admins"] });
        let (_, is_admin) = extract_identity(&claims, &config(None));
        assert!(!is_admin, "no admin_group configured must never imply admin, regardless of group membership");
    }

    #[test]
    fn membership_in_a_different_group_does_not_grant_admin() {
        let claims = serde_json::json!({ "preferred_username": "alice", "groups": ["helve-users"] });
        let (_, is_admin) = extract_identity(&claims, &config(Some("helve-admins")));
        assert!(!is_admin);
    }

    #[test]
    fn a_missing_or_malformed_groups_claim_is_treated_as_no_groups() {
        for claims in [serde_json::json!({ "preferred_username": "alice" }), serde_json::json!({ "preferred_username": "alice", "groups": "not-an-array" })]
        {
            let (_, is_admin) = extract_identity(&claims, &config(Some("helve-admins")));
            assert!(!is_admin);
        }
    }

    #[test]
    fn a_missing_username_claim_yields_none_rather_than_a_default() {
        let claims = serde_json::json!({ "sub": "abc123" });
        let (username, _) = extract_identity(&claims, &config(None));
        assert_eq!(username, None);
    }

    #[test]
    fn a_custom_username_claim_name_is_honored() {
        let claims = serde_json::json!({ "upn": "alice@example.com" });
        let mut cfg = config(None);
        cfg.username_claim = "upn".to_string();
        let (username, _) = extract_identity(&claims, &cfg);
        assert_eq!(username.as_deref(), Some("alice@example.com"));
    }

    /// A base64url payload segment encoding `{"sub":"abc123","preferred_username":"alice"}`,
    /// flanked by placeholder header/signature segments — decode_jwt_payload
    /// only ever looks at the middle one.
    #[test]
    fn decodes_the_middle_segment_of_a_compact_jwt() {
        let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"sub":"abc123","preferred_username":"alice"}"#);
        let compact = format!("header.{payload}.signature");
        let decoded = decode_jwt_payload(&compact).expect("should decode");
        assert_eq!(decoded["sub"], "abc123");
        assert_eq!(decoded["preferred_username"], "alice");
    }

    #[test]
    fn rejects_a_token_with_too_few_segments() {
        assert!(decode_jwt_payload("onlyoneseg").is_err());
    }

    #[test]
    fn rejects_a_payload_segment_that_is_not_valid_base64() {
        assert!(decode_jwt_payload("header.not!valid!base64.signature").is_err());
    }
}
