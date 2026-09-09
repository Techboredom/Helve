//! Thin wrappers over `gloo_net` for talking to Helve's own API.
//!
//! Every tab was repeating the same three-step dance — encode, send, then
//! decode either the body or the `{"error": "..."}` the backend returns —
//! which is a lot of surface area for a difference to creep into. These
//! collapse it to one call returning `Result<_, String>`, where the error is
//! already a sentence fit to show the user.

use gloo_net::http::{Request, RequestBuilder, Response};
use serde::de::DeserializeOwned;
use serde::Serialize;

/// Pulls the backend's `{"error": "..."}` message out of a failed response,
/// falling back to the status code when the body isn't the shape we expect
/// (a proxy or the static-file fallback answering instead, say).
async fn error_message(resp: Response) -> String {
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    match body.get("error").and_then(|v| v.as_str()) {
        Some(message) => message.to_string(),
        None => format!("HTTP {status}"),
    }
}

/// `GET`, decoding the response body.
pub async fn get_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = Request::get(url).send().await.map_err(|err| format!("request failed: {err}"))?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>().await.map_err(|err| format!("failed to parse response: {err}"))
}

async fn send_json<B: Serialize>(request: RequestBuilder, body: &B) -> Result<Response, String> {
    request
        .json(body)
        .map_err(|err| format!("failed to encode request: {err}"))?
        .send()
        .await
        .map_err(|err| format!("request failed: {err}"))
}

/// `POST` a body and decode the response.
pub async fn post_json<B: Serialize, T: DeserializeOwned>(url: &str, body: &B) -> Result<T, String> {
    let resp = send_json(Request::post(url), body).await?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>().await.map_err(|err| format!("failed to parse response: {err}"))
}

/// `PUT` a body and decode the response.
pub async fn put_json<B: Serialize, T: DeserializeOwned>(url: &str, body: &B) -> Result<T, String> {
    let resp = send_json(Request::put(url), body).await?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>().await.map_err(|err| format!("failed to parse response: {err}"))
}

/// `PUT` a body where the backend answers with no content.
pub async fn put_empty<B: Serialize>(url: &str, body: &B) -> Result<(), String> {
    let resp = send_json(Request::put(url), body).await?;
    if resp.ok() { Ok(()) } else { Err(error_message(resp).await) }
}

/// `POST` with no request body, decoding the response — e.g. rollback,
/// regenerate-secret: the target is already named in the URL, nothing else
/// to send.
pub async fn post_empty_json<T: DeserializeOwned>(url: &str) -> Result<T, String> {
    let resp = Request::post(url).send().await.map_err(|err| format!("request failed: {err}"))?;
    if !resp.ok() {
        return Err(error_message(resp).await);
    }
    resp.json::<T>().await.map_err(|err| format!("failed to parse response: {err}"))
}

/// `POST` with no request body and no response body — e.g. restart.
pub async fn post_empty(url: &str) -> Result<(), String> {
    let resp = Request::post(url).send().await.map_err(|err| format!("request failed: {err}"))?;
    if resp.ok() { Ok(()) } else { Err(error_message(resp).await) }
}

/// `DELETE`, expecting no content back.
pub async fn delete(url: &str) -> Result<(), String> {
    let resp = Request::delete(url).send().await.map_err(|err| format!("request failed: {err}"))?;
    if resp.ok() { Ok(()) } else { Err(error_message(resp).await) }
}
