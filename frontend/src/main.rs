mod account;
mod activity_tab;
mod api;
mod api_tokens_tab;
mod create_deployment_tab;
mod deployment_manage;
mod env_editor;
mod format;
mod images_tab;
mod login;
mod pod_detail;
mod pods_tab;
mod quotas_tab;
mod result_banner;
mod templates_tab;
mod theme;
mod users_tab;
mod ws;

use account::ChangePasswordPanel;
use activity_tab::ActivityTab;
use api_tokens_tab::ApiTokensTab;
use common::{Role, UserInfo};
use create_deployment_tab::CreateDeploymentTab;
use gloo_net::http::Request;
use images_tab::ImagesTab;
use leptos::prelude::*;
use leptos::task::spawn_local;
use login::LoginPage;
use pods_tab::PodsTab;
use quotas_tab::QuotasTab;
use templates_tab::TemplatesTab;
use users_tab::UsersTab;

fn main() {
    console_error_panic_hook::set_once();
    leptos::mount::mount_to_body(App);
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Pods,
    Launch,
    Activity,
    Templates,
    Images,
    Users,
    Quotas,
    ApiTokens,
}

#[component]
fn App() -> impl IntoView {
    let theme = theme::init_theme();
    let current_user: RwSignal<Option<UserInfo>> = RwSignal::new(None);
    let checking = RwSignal::new(true);

    spawn_local(async move {
        if let Ok(resp) = Request::get("/api/me").send().await
            && resp.ok()
                && let Ok(user) = resp.json::<UserInfo>().await {
                    current_user.set(Some(user));
                }
        checking.set(false);
    });

    view! {
        {move || {
            if checking.get() {
                view! { <div class="loading-screen">"Loading…"</div> }.into_any()
            } else {
                match current_user.get() {
                    None => view! { <LoginPage current_user=current_user /> }.into_any(),
                    Some(user) => view! { <AppShell user=user current_user=current_user theme=theme /> }.into_any(),
                }
            }
        }}
    }
}

#[component]
fn AppShell(user: UserInfo, current_user: RwSignal<Option<UserInfo>>, theme: RwSignal<String>) -> impl IntoView {
    let tab = RwSignal::new(Tab::Pods);
    let is_admin = user.role == Role::Admin;
    let username = user.username.clone();
    let show_change_password = RwSignal::new(false);

    let logout = move |_| {
        spawn_local(async move {
            let _ = Request::post("/api/logout").send().await;
            current_user.set(None);
        });
    };

    view! {
        <main>
            <header>
<svg class="brand-mark" viewBox="47 1 87 180" fill="none" role="img" aria-labelledby="header-mark-title">
                    <title id="header-mark-title">"Helve"</title>
                    <g fill="currentColor">
                        <path d="M62 1c5 0 10 2 10 10 1 13 0 23-4 35-4 14-3 26 2 40 5 16 8 24 11 38s5 22 7 32 1 19-6 22-15 0-18-8 0-16 4-22c-2-10-4-16-6-26-3-14-7-24-9-36-3-16-6-30-4-42 1-13-1-19 0-25 .5-8 0-14 5-17Z"/>
                        <path d="m51 14 23-2c20 1 36-2 47-5 8 3 13 12 13 26 0 17-9 33-23 40-13-11-24-22-35-27-8 0-17-1-25-2Z"/>
                    </g>
                </svg>
                <h1>"Helve"</h1>
                <nav class="tabs">
                    <button
                        class="tab-button"
                        class:active=move || tab.get() == Tab::Pods
                        on:click=move |_| tab.set(Tab::Pods)
                    >
                        "Pods"
                    </button>
                    <button
                        class="tab-button"
                        class:active=move || tab.get() == Tab::Launch
                        on:click=move |_| tab.set(Tab::Launch)
                    >
                        "Launch"
                    </button>
                    <button
                        class="tab-button"
                        class:active=move || tab.get() == Tab::Activity
                        on:click=move |_| tab.set(Tab::Activity)
                    >
                        "Activity"
                    </button>
                    <Show when=move || is_admin>
                        <button
                            class="tab-button"
                            class:active=move || tab.get() == Tab::Templates
                            on:click=move |_| tab.set(Tab::Templates)
                        >
                            "Templates"
                        </button>
                        <button
                            class="tab-button"
                            class:active=move || tab.get() == Tab::Images
                            on:click=move |_| tab.set(Tab::Images)
                        >
                            "Images"
                        </button>
                        <button
                            class="tab-button"
                            class:active=move || tab.get() == Tab::Users
                            on:click=move |_| tab.set(Tab::Users)
                        >
                            "Users"
                        </button>
                        <button
                            class="tab-button"
                            class:active=move || tab.get() == Tab::Quotas
                            on:click=move |_| tab.set(Tab::Quotas)
                        >
                            "Quotas"
                        </button>
                        <button
                            class="tab-button"
                            class:active=move || tab.get() == Tab::ApiTokens
                            on:click=move |_| tab.set(Tab::ApiTokens)
                        >
                            "API Tokens"
                        </button>
                    </Show>
                </nav>
                <div class="user-info">
                    <span class="username">{username}</span>
                    <button
                        class="icon-button"
                        on:click=move |_| theme::toggle(theme)
                    >
                        {move || if theme.get() == "light" { "Dark theme" } else { "Light theme" }}
                    </button>
                    <button class="icon-button" on:click=move |_| show_change_password.set(true)>
                        "Change password"
                    </button>
                    <button class="icon-button" on:click=logout>
                        "Log out"
                    </button>
                </div>
            </header>

            <Show when=move || show_change_password.get()>
                <ChangePasswordPanel open=show_change_password />
            </Show>

            <div class:hidden=move || tab.get() != Tab::Pods>
                <PodsTab is_admin=is_admin />
            </div>
            <div class:hidden=move || tab.get() != Tab::Launch>
                <CreateDeploymentTab is_admin=is_admin />
            </div>
            <div class:hidden=move || tab.get() != Tab::Activity>
                <ActivityTab is_admin=is_admin />
            </div>
            <Show when=move || is_admin>
                <div class:hidden=move || tab.get() != Tab::Templates>
                    <TemplatesTab />
                </div>
                <div class:hidden=move || tab.get() != Tab::Images>
                    <ImagesTab />
                </div>
                <div class:hidden=move || tab.get() != Tab::Users>
                    <UsersTab />
                </div>
                <div class:hidden=move || tab.get() != Tab::Quotas>
                    <QuotasTab />
                </div>
                <div class:hidden=move || tab.get() != Tab::ApiTokens>
                    <ApiTokensTab />
                </div>
            </Show>
        </main>
    }
}
