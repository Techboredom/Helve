//! Reverse-proxies `/models/{username}/{engine_slug}/...` into a launched
//! model-serving pod (Ollama, vLLM, SGLang), authenticated purely by an
//! `Authorization: Bearer <token>` header — no Helve login, no cookies.
//!
//! Deliberately separate from `proxy.rs`'s `/proxy/` route: that one
//! requires a browser session and a redirect handshake, which API tooling
//! (a coding assistant, a script) can't do. This route authenticates the
//! same way `api_tokens` does (`auth::user_from_api_token`) — except the
//! token here names a *deployment*, not a *user*, since the whole point is
//! reaching a specific model server without ever logging in as anyone.
//!
//! Resolution is always "hash-free lookup of the bearer token in
//! `deployment_secrets`, then check the path against what that row says" —
//! the token is the real authority, the `{username}/{engine_slug}` (and, if
//! the deployment serves a specific model, the leading path segment naming
//! it) path segments are only a consistency check that catches pasting the
//! wrong token into the wrong tool's config. `deployment_secrets.proxy_token`
//! is looked up by plaintext equality (a unique index backs it), the same
//! convention `proxy_auth_tokens`/`proxy_sessions` already use in this
//! table — this is not the hash-and-forget `api_tokens` pattern, since
//! `secret_value` here is already persistently redisplayed on the Pods tab.
//!
//! Reaches the pod the same way `/proxy/` does — see `proxy::connect_and_stream`,
//! shared by both routes — so the same in-cluster-only caveat applies here too.

use axum::extract::{Path, Request, State};
use axum::http;
use axum::response::Response;
use sqlx::FromRow;

use crate::error::ApiError;
use crate::proxy;
use crate::state::AppState;

/// The root-relative `/models/...` URL for a deployment — always
/// root-relative (this route lives on the main app origin only; there's no
/// cookie to scope, so no per-deployment-origin complexity like `/proxy/`
/// has). Shared by `deployments::create_deployment`'s response and
/// `visibility`'s `PodInfo::models_access`.
pub fn models_url(username: &str, engine_slug: &str, model_slug: Option<&str>) -> String {
    match model_slug {
        Some(slug) => format!("/models/{username}/{engine_slug}/{slug}/"),
        None => format!("/models/{username}/{engine_slug}/"),
    }
}

#[derive(FromRow)]
struct ModelsRow {
    deployment_name: String,
    owner_username: String,
    engine_slug: Option<String>,
    model_slug: Option<String>,
    container_port: Option<i32>,
}

/// Handles `/models/{username}/{engine_slug}/{*rest}` — anything with at
/// least one path segment after `engine_slug`.
pub async fn handler(
    Path((username, engine_slug, rest)): Path<(String, String, String)>,
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, ApiError> {
    proxy_request(username, engine_slug, rest, state, req).await
}

/// Handles the bare `/models/{username}/{engine_slug}` and trailing-slash
/// `/models/{username}/{engine_slug}/` forms — matchit's `{*rest}` wildcard
/// requires at least one character after that slash, same reasoning as
/// `proxy::handler_root`.
pub async fn handler_root(
    Path((username, engine_slug)): Path<(String, String)>,
    State(state): State<AppState>,
    req: Request,
) -> Result<Response, ApiError> {
    proxy_request(username, engine_slug, String::new(), state, req).await
}

async fn proxy_request(username: String, engine_slug: String, rest: String, state: AppState, req: Request) -> Result<Response, ApiError> {
    let token = bearer_token(req.headers())?.to_string();

    let not_found = || ApiError::NotFound("no such model deployment".to_string());
    let row: Option<ModelsRow> = sqlx::query_as(
        "SELECT deployment_name, owner_username, engine_slug, model_slug, container_port \
         FROM deployment_secrets WHERE proxy_token = $1",
    )
    .bind(&token)
    .fetch_optional(&state.pg)
    .await?;
    let row = row.ok_or_else(not_found)?;
    if !path_matches_row(&username, &engine_slug, &row.owner_username, row.engine_slug.as_deref()) {
        return Err(not_found());
    }
    let container_port = row
        .container_port
        .ok_or_else(|| ApiError::ProxyUnavailable(format!("{} has no container_port on record", row.deployment_name)))?;
    let forward_rest = strip_model_slug(&rest, row.model_slug.as_deref())?;
    let query = req.uri().query().map(|q| format!("?{q}")).unwrap_or_default();
    let path_and_query = format!("/{forward_rest}{query}");

    let is_upgrade = req.headers().get(http::header::UPGRADE).is_some();
    let outbound_headers = forwarded_headers(req.headers(), is_upgrade);

    proxy::connect_and_stream(&state, &row.deployment_name, container_port, path_and_query, outbound_headers, is_upgrade, req).await
}

/// Extracts the raw token from `Authorization: Bearer <token>` — 400 (not
/// 401/403) on anything else, since this isn't a "your credentials were
/// wrong" case so much as "this request isn't even shaped right" one; a
/// bad/unknown token itself is a 404 (see `proxy_request`'s `not_found`),
/// deliberately not distinguished from a bad path.
fn bearer_token(headers: &http::HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty())
        .ok_or_else(|| ApiError::BadRequest("missing or malformed \"Authorization: Bearer <token>\" header".to_string()))
}

/// Whether the deployment a bearer token resolved to (`owner_username`,
/// `row_engine_slug`) matches the path it was presented at
/// (`username`/`engine_slug`) — the token is the real authority; this is
/// only a consistency check that catches pasting the wrong token into the
/// wrong tool's config, not a security boundary on its own.
fn path_matches_row(username: &str, engine_slug: &str, owner_username: &str, row_engine_slug: Option<&str>) -> bool {
    owner_username == username && row_engine_slug == Some(engine_slug)
}

/// Strips a deployment's `model_slug` from the front of `rest` (the path
/// after `/models/{username}/{engine_slug}/`), when it serves a specific
/// model — a user with enough quota can run two same-engine deployments on
/// two different models at once, so this is what keeps their paths from
/// colliding. `model_slug: None` (nothing to disambiguate) passes `rest`
/// through unchanged. Matches on a whole path segment, not a bare prefix
/// (`"llama-3"` must not match a `rest` starting with `"llama-30"`) —
/// `Err` means the caller's path doesn't name this deployment's model at
/// all, most likely because the right token ended up under the wrong
/// path, or vice versa.
fn strip_model_slug<'a>(rest: &'a str, model_slug: Option<&str>) -> Result<&'a str, ApiError> {
    let Some(slug) = model_slug else { return Ok(rest) };
    let bad_path = || ApiError::BadRequest(format!("path must start with \"{slug}/\" for this deployment's model"));
    match rest.strip_prefix(slug) {
        Some("") => Ok(""),
        Some(after) => after.strip_prefix('/').ok_or_else(bad_path),
        None => Err(bad_path()),
    }
}

/// The header set forwarded to the pod: everything the client sent, minus
/// hop-by-hop headers — unlike `proxy::forwarded_headers`, the caller's own
/// `Authorization` is forwarded through unchanged rather than replaced:
/// it's both the bearer token this route just authenticated with and, for
/// vLLM/SGLang, the same value happens to also satisfy the engine's own
/// `--api-key` check (free defense in depth); Ollama just ignores it
/// downstream. No cookie handling either — this route never touches a
/// Helve session cookie in either direction.
fn forwarded_headers(inbound: &http::HeaderMap, is_upgrade: bool) -> http::HeaderMap {
    let mut out = http::HeaderMap::new();
    for (name, value) in inbound.iter() {
        if proxy::is_hop_by_hop(name, is_upgrade) {
            continue;
        }
        out.append(name.clone(), value.clone());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_the_models_url_with_and_without_a_model_slug() {
        assert_eq!(models_url("alice", "ollama", None), "/models/alice/ollama/");
        assert_eq!(models_url("alice", "vllm", Some("llama-3")), "/models/alice/vllm/llama-3/");
    }

    #[test]
    fn path_matches_row_requires_both_owner_and_engine() {
        assert!(path_matches_row("alice", "vllm", "alice", Some("vllm")));
        assert!(!path_matches_row("alice", "vllm", "bob", Some("vllm")), "wrong owner");
        assert!(!path_matches_row("alice", "vllm", "alice", Some("ollama")), "wrong engine");
        assert!(!path_matches_row("alice", "vllm", "alice", None), "row has no engine_slug at all");
    }

    #[test]
    fn strip_model_slug_passes_through_when_the_deployment_has_no_model() {
        assert_eq!(strip_model_slug("v1/chat/completions", None).ok(), Some("v1/chat/completions"));
        assert_eq!(strip_model_slug("", None).ok(), Some(""));
    }

    #[test]
    fn strip_model_slug_strips_a_matching_leading_segment() {
        assert_eq!(strip_model_slug("llama-3/v1/chat/completions", Some("llama-3")).ok(), Some("v1/chat/completions"));
        // Bare model slug, nothing after it.
        assert_eq!(strip_model_slug("llama-3", Some("llama-3")).ok(), Some(""));
    }

    #[test]
    fn strip_model_slug_rejects_a_partial_segment_match() {
        // "llama-30" must not be treated as starting with "llama-3".
        assert!(strip_model_slug("llama-30/v1/models", Some("llama-3")).is_err());
    }

    #[test]
    fn strip_model_slug_rejects_a_missing_or_wrong_model_segment() {
        assert!(strip_model_slug("v1/chat/completions", Some("llama-3")).is_err());
        assert!(strip_model_slug("", Some("llama-3")).is_err());
        assert!(strip_model_slug("other-model/v1", Some("llama-3")).is_err());
    }
}
