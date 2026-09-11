use common::{
    CreateUserRequest, GroupInfo, ResetPasswordRequest, Role, SetNodeLabelRequest, SetSupplementalGroupsRequest, SetUidGidRequest,
    UserInfo,
};
use leptos::prelude::*;
use leptos::tachys::dom::{event_target_checked, event_target_value};
use leptos::task::spawn_local;

use crate::api;
use crate::result_banner::{ErrorBanner, ResultBanner};

#[component]
pub fn UsersTab() -> impl IntoView {
    let users: RwSignal<Vec<UserInfo>> = RwSignal::new(Vec::new());
    let list_error = RwSignal::new(None::<String>);

    let username = RwSignal::new(String::new());
    let password = RwSignal::new(String::new());
    let role = RwSignal::new("user".to_string());

    let saving = RwSignal::new(false);
    let form_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let reset_target: RwSignal<Option<(i32, String)>> = RwSignal::new(None);
    let reset_password_value = RwSignal::new(String::new());
    let reset_saving = RwSignal::new(false);
    let reset_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let label_target: RwSignal<Option<(i32, String)>> = RwSignal::new(None);
    let label_value = RwSignal::new(String::new());
    let label_saving = RwSignal::new(false);
    let label_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let uidgid_target: RwSignal<Option<(i32, String)>> = RwSignal::new(None);
    let uid_value = RwSignal::new(String::new());
    let gid_value = RwSignal::new(String::new());
    let uidgid_saving = RwSignal::new(false);
    let uidgid_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let groups_catalog: RwSignal<Vec<GroupInfo>> = RwSignal::new(Vec::new());
    let groups_target: RwSignal<Option<(i32, String)>> = RwSignal::new(None);
    let groups_selected: RwSignal<Vec<i32>> = RwSignal::new(Vec::new());
    let groups_saving = RwSignal::new(false);
    let groups_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let refresh = move || {
        spawn_local(async move {
            match api::get_json::<Vec<UserInfo>>("/api/users").await {
                Ok(list) => {
                    list_error.set(None);
                    users.set(list);
                }
                Err(err) => list_error.set(Some(format!("failed to load users: {err}"))),
            }
        });
    };
    refresh();

    spawn_local(async move {
        if let Ok(list) = api::get_json::<Vec<GroupInfo>>("/api/groups").await {
            groups_catalog.set(list);
        }
    });

    let delete_user = move |id: i32| {
        let confirmed = web_sys::window()
            .and_then(|w| w.confirm_with_message("Delete this user? This can't be undone.").ok())
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        spawn_local(async move {
            match api::delete(&format!("/api/users/{id}")).await {
                Ok(()) => refresh(),
                Err(err) => list_error.set(Some(format!("failed to delete user: {err}"))),
            }
        });
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        if saving.get() {
            return;
        }
        let req = CreateUserRequest {
            username: username.get().trim().to_string(),
            password: password.get(),
            role: if role.get() == "admin" { Role::Admin } else { Role::User },
        };
        saving.set(true);
        form_result.set(None);
        spawn_local(async move {
            let outcome = create(req).await;
            saving.set(false);
            match outcome {
                Ok(msg) => {
                    form_result.set(Some(Ok(msg)));
                    username.set(String::new());
                    password.set(String::new());
                    role.set("user".to_string());
                    refresh();
                }
                Err(err) => form_result.set(Some(Err(err))),
            }
        });
    };

    view! {
        <div class="tab-panel">
            <ErrorBanner error=list_error />

            <div class="table-wrap">
                <table>
                    <thead>
                        <tr>
                            <th>"Username"</th>
                            <th>"Role"</th>
                            <th>"Auth source"</th>
                            <th>"Node label"</th>
                            <th>"UID/GID"</th>
                            <th>"Supplemental groups"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=move || users.get() key=|u| u.id let(u)>
                            {
                                let id = u.id;
                                let role_label = if u.role == Role::Admin { "admin" } else { "user" };
                                let node_label_display = u.node_label.clone().unwrap_or_else(|| "—".to_string());
                                let uidgid_display = format!(
                                    "{}/{}",
                                    u.uid.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()),
                                    u.gid.map(|v| v.to_string()).unwrap_or_else(|| "—".to_string()),
                                );
                                let reset_username = u.username.clone();
                                let label_username = u.username.clone();
                                let uidgid_username = u.username.clone();
                                let groups_username = u.username.clone();
                                let current_node_label = u.node_label.clone().unwrap_or_default();
                                let current_uid = u.uid.map(|v| v.to_string()).unwrap_or_default();
                                let current_gid = u.gid.map(|v| v.to_string()).unwrap_or_default();
                                let groups_display = if u.supplemental_groups.is_empty() {
                                    "—".to_string()
                                } else {
                                    u.supplemental_groups.iter().map(|g| g.name.clone()).collect::<Vec<_>>().join(", ")
                                };
                                let current_groups: Vec<i32> = u.supplemental_groups.iter().map(|g| g.id).collect();
                                view! {
                                    <tr>
                                        <td>{u.username.clone()}</td>
                                        <td>{role_label}</td>
                                        <td>{u.auth_source.clone()}</td>
                                        <td>{node_label_display}</td>
                                        <td>{uidgid_display}</td>
                                        <td>{groups_display}</td>
                                        <td class="table-actions">
                                            <button
                                                type="button"
                                                class="icon-button"
                                                on:click=move |_| {
                                                    reset_target.set(Some((id, reset_username.clone())));
                                                    reset_password_value.set(String::new());
                                                    reset_result.set(None);
                                                }
                                            >
                                                "Reset password"
                                            </button>
                                            <button
                                                type="button"
                                                class="icon-button"
                                                on:click=move |_| {
                                                    label_target.set(Some((id, label_username.clone())));
                                                    label_value.set(current_node_label.clone());
                                                    label_result.set(None);
                                                }
                                            >
                                                "Node label"
                                            </button>
                                            <button
                                                type="button"
                                                class="icon-button"
                                                on:click=move |_| {
                                                    uidgid_target.set(Some((id, uidgid_username.clone())));
                                                    uid_value.set(current_uid.clone());
                                                    gid_value.set(current_gid.clone());
                                                    uidgid_result.set(None);
                                                }
                                            >
                                                "UID/GID"
                                            </button>
                                            <button
                                                type="button"
                                                class="icon-button"
                                                on:click=move |_| {
                                                    groups_target.set(Some((id, groups_username.clone())));
                                                    groups_selected.set(current_groups.clone());
                                                    groups_result.set(None);
                                                }
                                            >
                                                "Groups"
                                            </button>
                                            <button type="button" class="icon-button" on:click=move |_| delete_user(id)>
                                                "Delete"
                                            </button>
                                        </td>
                                    </tr>
                                }
                            }
                        </For>
                    </tbody>
                </table>
                <Show when=move || users.get().is_empty() && list_error.get().is_none()>
                    <p class="empty">"No users yet."</p>
                </Show>
            </div>

            {move || {
                reset_target
                    .get()
                    .map(|(id, target_username)| {
                        let on_reset_submit = move |ev: web_sys::SubmitEvent| {
                            ev.prevent_default();
                            if reset_saving.get() {
                                return;
                            }
                            let password = reset_password_value.get();
                            reset_saving.set(true);
                            reset_result.set(None);
                            spawn_local(async move {
                                let outcome = reset_password(id, password).await;
                                reset_saving.set(false);
                                match outcome {
                                    Ok(msg) => {
                                        reset_result.set(Some(Ok(msg)));
                                        reset_target.set(None);
                                    }
                                    Err(err) => reset_result.set(Some(Err(err))),
                                }
                            });
                        };
                        view! {
                            <h3 class="section-heading">{format!("Reset password for \"{target_username}\"")}</h3>
                            <form class="deploy-form" on:submit=on_reset_submit>
                                <label>
                                    "New password"
                                    <input
                                        type="password"
                                        required=true
                                        minlength="8"
                                        prop:value=move || reset_password_value.get()
                                        on:input=move |ev| reset_password_value.set(event_target_value(&ev))
                                    />
                                </label>
                                <div class="form-actions">
                                    <button type="submit" disabled=move || reset_saving.get()>
                                        {move || if reset_saving.get() { "Saving…" } else { "Set password" }}
                                    </button>
                                    <button type="button" class="secondary-button" on:click=move |_| reset_target.set(None)>
                                        "Cancel"
                                    </button>
                                </div>
                            </form>
                        }
                    })
            }}
            <ResultBanner result=reset_result />

            {move || {
                label_target
                    .get()
                    .map(|(id, target_username)| {
                        let on_label_submit = move |ev: web_sys::SubmitEvent| {
                            ev.prevent_default();
                            if label_saving.get() {
                                return;
                            }
                            let value = label_value.get();
                            label_saving.set(true);
                            label_result.set(None);
                            spawn_local(async move {
                                let outcome = set_node_label(id, value).await;
                                label_saving.set(false);
                                match outcome {
                                    Ok(msg) => {
                                        label_result.set(Some(Ok(msg)));
                                        label_target.set(None);
                                        refresh();
                                    }
                                    Err(err) => label_result.set(Some(Err(err))),
                                }
                            });
                        };
                        view! {
                            <h3 class="section-heading">{format!("Node label for \"{target_username}\"")}</h3>
                            <form class="deploy-form" on:submit=on_label_submit>
                                <label>
                                    "Label (\"key=value\", e.g. \"node-type=cpu\")"
                                    <input
                                        type="text"
                                        placeholder="leave empty for unrestricted placement"
                                        prop:value=move || label_value.get()
                                        on:input=move |ev| label_value.set(event_target_value(&ev))
                                    />
                                </label>
                                <p class="hint">
                                    "Every future launch from this account is scheduled only onto nodes carrying this label. Existing deployments aren't affected."
                                </p>
                                <div class="form-actions">
                                    <button type="submit" disabled=move || label_saving.get()>
                                        {move || if label_saving.get() { "Saving…" } else { "Save" }}
                                    </button>
                                    <button type="button" class="secondary-button" on:click=move |_| label_target.set(None)>
                                        "Cancel"
                                    </button>
                                </div>
                            </form>
                        }
                    })
            }}
            <ResultBanner result=label_result />

            {move || {
                uidgid_target
                    .get()
                    .map(|(id, target_username)| {
                        let on_uidgid_submit = move |ev: web_sys::SubmitEvent| {
                            ev.prevent_default();
                            if uidgid_saving.get() {
                                return;
                            }
                            let uid = uid_value.get();
                            let gid = gid_value.get();
                            uidgid_saving.set(true);
                            uidgid_result.set(None);
                            spawn_local(async move {
                                let outcome = set_uid_gid(id, uid, gid).await;
                                uidgid_saving.set(false);
                                match outcome {
                                    Ok(msg) => {
                                        uidgid_result.set(Some(Ok(msg)));
                                        uidgid_target.set(None);
                                        refresh();
                                    }
                                    Err(err) => uidgid_result.set(Some(Err(err))),
                                }
                            });
                        };
                        view! {
                            <h3 class="section-heading">{format!("UID/GID for \"{target_username}\"")}</h3>
                            <form class="deploy-form" on:submit=on_uidgid_submit>
                                <label>
                                    "UID"
                                    <input
                                        type="number"
                                        min="1"
                                        placeholder="leave empty for the image's default"
                                        prop:value=move || uid_value.get()
                                        on:input=move |ev| uid_value.set(event_target_value(&ev))
                                    />
                                </label>
                                <label>
                                    "GID"
                                    <input
                                        type="number"
                                        min="1"
                                        placeholder="leave empty for the image's default"
                                        prop:value=move || gid_value.get()
                                        on:input=move |ev| gid_value.set(event_target_value(&ev))
                                    />
                                </label>
                                <p class="hint">
                                    "Every future launch from this account runs its container as this UID/GID (and mounts volumes with matching group ownership). Existing deployments aren't affected. Either field can be set on its own."
                                </p>
                                <div class="form-actions">
                                    <button type="submit" disabled=move || uidgid_saving.get()>
                                        {move || if uidgid_saving.get() { "Saving…" } else { "Save" }}
                                    </button>
                                    <button type="button" class="secondary-button" on:click=move |_| uidgid_target.set(None)>
                                        "Cancel"
                                    </button>
                                </div>
                            </form>
                        }
                    })
            }}
            <ResultBanner result=uidgid_result />

            {move || {
                groups_target
                    .get()
                    .map(|(id, target_username)| {
                        let on_groups_submit = move |ev: web_sys::SubmitEvent| {
                            ev.prevent_default();
                            if groups_saving.get() {
                                return;
                            }
                            let group_ids = groups_selected.get();
                            groups_saving.set(true);
                            groups_result.set(None);
                            spawn_local(async move {
                                let outcome = set_supplemental_groups(id, group_ids).await;
                                groups_saving.set(false);
                                match outcome {
                                    Ok(msg) => {
                                        groups_result.set(Some(Ok(msg)));
                                        groups_target.set(None);
                                        refresh();
                                    }
                                    Err(err) => groups_result.set(Some(Err(err))),
                                }
                            });
                        };
                        view! {
                            <h3 class="section-heading">{format!("Supplemental groups for \"{target_username}\"")}</h3>
                            <form class="deploy-form" on:submit=on_groups_submit>
                                <fieldset>
                                    <legend>"Groups"</legend>
                                    <Show
                                        when=move || !groups_catalog.get().is_empty()
                                        fallback=|| view! { <p class="hint">"No named groups yet — add one on the Groups tab."</p> }
                                    >
                                        <For each=move || groups_catalog.get() key=|g| g.id let(g)>
                                            {
                                                let group_id = g.id;
                                                view! {
                                                    <label class="checkbox">
                                                        <input
                                                            type="checkbox"
                                                            prop:checked=move || groups_selected.get().contains(&group_id)
                                                            on:change=move |ev| {
                                                                let checked = event_target_checked(&ev);
                                                                groups_selected
                                                                    .update(|selected| {
                                                                        if checked {
                                                                            if !selected.contains(&group_id) {
                                                                                selected.push(group_id);
                                                                            }
                                                                        } else {
                                                                            selected.retain(|id| *id != group_id);
                                                                        }
                                                                    });
                                                            }
                                                        />
                                                        {format!("{} (GID {})", g.name, g.gid)}
                                                    </label>
                                                }
                                            }
                                        </For>
                                    </Show>
                                </fieldset>
                                <p class="hint">
                                    "Every future launch from this account adds these as extra GIDs (pod securityContext supplementalGroups) alongside GID above — the POSIX/NFS pattern of belonging to several groups, each granting access to a different share. Existing deployments aren't affected."
                                </p>
                                <div class="form-actions">
                                    <button type="submit" disabled=move || groups_saving.get()>
                                        {move || if groups_saving.get() { "Saving…" } else { "Save" }}
                                    </button>
                                    <button type="button" class="secondary-button" on:click=move |_| groups_target.set(None)>
                                        "Cancel"
                                    </button>
                                </div>
                            </form>
                        }
                    })
            }}
            <ResultBanner result=groups_result />

            <h3 class="section-heading">"New user"</h3>
            <form class="deploy-form" on:submit=on_submit>
                <label>
                    "Username"
                    <input
                        type="text"
                        required=true
                        minlength="3"
                        maxlength="32"
                        prop:value=move || username.get()
                        on:input=move |ev| username.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    "Password"
                    <input
                        type="password"
                        required=true
                        minlength="8"
                        prop:value=move || password.get()
                        on:input=move |ev| password.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    "Role"
                    <select prop:value=move || role.get() on:change=move |ev| role.set(event_target_value(&ev))>
                        <option value="user">"User"</option>
                        <option value="admin">"Admin"</option>
                    </select>
                </label>
                <button type="submit" disabled=move || saving.get()>
                    {move || if saving.get() { "Creating…" } else { "Create user" }}
                </button>
            </form>

            <ResultBanner result=form_result />
        </div>
    }
}

async fn set_node_label(id: i32, value: String) -> Result<String, String> {
    let trimmed = value.trim();
    let node_label = if trimmed.is_empty() { None } else { Some(trimmed.to_string()) };
    let cleared = node_label.is_none();
    let _: UserInfo = api::put_json(&format!("/api/users/{id}/node-label"), &SetNodeLabelRequest { node_label })
        .await
        .map_err(|err| format!("Failed to save node label: {err}"))?;
    Ok(if cleared { "Node label cleared.".to_string() } else { "Node label saved.".to_string() })
}

async fn set_uid_gid(id: i32, uid: String, gid: String) -> Result<String, String> {
    let uid = uid.trim();
    let gid = gid.trim();
    let uid = if uid.is_empty() { None } else { Some(uid.parse().map_err(|_| "UID must be a whole number".to_string())?) };
    let gid = if gid.is_empty() { None } else { Some(gid.parse().map_err(|_| "GID must be a whole number".to_string())?) };
    let cleared = uid.is_none() && gid.is_none();
    let _: UserInfo = api::put_json(&format!("/api/users/{id}/uid-gid"), &SetUidGidRequest { uid, gid })
        .await
        .map_err(|err| format!("Failed to save UID/GID: {err}"))?;
    Ok(if cleared { "UID/GID cleared.".to_string() } else { "UID/GID saved.".to_string() })
}

async fn set_supplemental_groups(id: i32, group_ids: Vec<i32>) -> Result<String, String> {
    let cleared = group_ids.is_empty();
    let _: UserInfo = api::put_json(&format!("/api/users/{id}/supplemental-groups"), &SetSupplementalGroupsRequest { group_ids })
        .await
        .map_err(|err| format!("Failed to save supplemental groups: {err}"))?;
    Ok(if cleared { "Supplemental groups cleared.".to_string() } else { "Supplemental groups saved.".to_string() })
}

async fn reset_password(id: i32, password: String) -> Result<String, String> {
    api::put_empty(&format!("/api/users/{id}/password"), &ResetPasswordRequest { password })
        .await
        .map_err(|err| format!("Failed to reset password: {err}"))?;
    Ok("Password reset — that account's existing sessions have been logged out.".to_string())
}

async fn create(req: CreateUserRequest) -> Result<String, String> {
    let created: UserInfo = api::post_json("/api/users", &req).await.map_err(|err| format!("Failed to create user: {err}"))?;
    Ok(format!("Created user \"{}\".", created.username))
}
