use common::{AuthConfig, LoginRequest, UserInfo};
use gloo_net::http::Request;
use leptos::prelude::*;
use leptos::tachys::dom::event_target_value;
use leptos::task::spawn_local;

#[component]
pub fn LoginPage(current_user: RwSignal<Option<UserInfo>>) -> impl IntoView {
    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let submitting = RwSignal::new(false);
    let oidc_enabled = RwSignal::new(false);

    spawn_local(async move {
        if let Ok(config) = fetch_auth_config().await {
            oidc_enabled.set(config.oidc_enabled);
        }
    });

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        if submitting.get() {
            return;
        }
        let req = LoginRequest { username: username.get().trim().to_string(), password: password.get() };
        submitting.set(true);
        error.set(None);
        spawn_local(async move {
            match attempt_login(req).await {
                Ok(user) => current_user.set(Some(user)),
                Err(msg) => error.set(Some(msg)),
            }
            submitting.set(false);
        });
    };

    view! {
        <div class="login-screen">
            <form class="login-form" on:submit=on_submit>
                <h1>"Helve"</h1>
                {move || error.get().map(|msg| view! { <div class="error">{msg}</div> })}
                <Show when=move || oidc_enabled.get()>
                    <a class="sso-button" href="/api/auth/oidc/login">
                        "Log in with SSO"
                    </a>
                    <p class="sso-divider">"or"</p>
                </Show>
                <label>
                    "Username"
                    <input
                        type="text"
                        required=true
                        prop:value=move || username.get()
                        on:input=move |ev| username.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    "Password"
                    <input
                        type="password"
                        required=true
                        prop:value=move || password.get()
                        on:input=move |ev| password.set(event_target_value(&ev))
                    />
                </label>
                <button type="submit" disabled=move || submitting.get()>
                    {move || if submitting.get() { "Logging in…" } else { "Log in" }}
                </button>
            </form>
        </div>
    }
}

async fn fetch_auth_config() -> Result<AuthConfig, String> {
    let resp = Request::get("/api/auth/config").send().await.map_err(|err| format!("request failed: {err}"))?;
    resp.json::<AuthConfig>().await.map_err(|err| format!("failed to parse response: {err}"))
}

async fn attempt_login(req: LoginRequest) -> Result<UserInfo, String> {
    let resp = Request::post("/api/login")
        .json(&req)
        .map_err(|err| format!("failed to encode request: {err}"))?
        .send()
        .await
        .map_err(|err| format!("request failed: {err}"))?;

    if resp.ok() {
        resp.json::<UserInfo>().await.map_err(|err| format!("failed to parse response: {err}"))
    } else if resp.status() == 401 {
        Err("Invalid username or password.".to_string())
    } else {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        let message = body.get("error").and_then(|v| v.as_str()).unwrap_or("unknown error");
        Err(format!("Login failed: {message}"))
    }
}
