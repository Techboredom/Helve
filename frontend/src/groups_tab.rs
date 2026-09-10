use common::{GroupInfo, SaveGroupRequest};
use leptos::prelude::*;
use leptos::tachys::dom::event_target_value;
use leptos::task::spawn_local;

use crate::api;
use crate::result_banner::{ErrorBanner, ResultBanner};

#[component]
pub fn GroupsTab() -> impl IntoView {
    let groups: RwSignal<Vec<GroupInfo>> = RwSignal::new(Vec::new());
    let list_error = RwSignal::new(None::<String>);

    let editing_id = RwSignal::new(None::<i32>);
    let name = RwSignal::new(String::new());
    let gid = RwSignal::new(String::new());

    let saving = RwSignal::new(false);
    let form_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    let refresh = move || {
        spawn_local(async move {
            match api::get_json::<Vec<GroupInfo>>("/api/groups").await {
                Ok(list) => {
                    list_error.set(None);
                    groups.set(list);
                }
                Err(err) => list_error.set(Some(format!("failed to load groups: {err}"))),
            }
        });
    };
    refresh();

    let clear_form = move || {
        editing_id.set(None);
        name.set(String::new());
        gid.set(String::new());
    };

    let edit_group = move |g: &GroupInfo| {
        editing_id.set(Some(g.id));
        name.set(g.name.clone());
        gid.set(g.gid.to_string());
        form_result.set(None);
    };

    let delete_group = move |id: i32| {
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(
                    "Delete this group? Fails if any user is still assigned it — this can't be undone otherwise.",
                )
                .ok()
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        spawn_local(async move {
            match api::delete(&format!("/api/groups/{id}")).await {
                Ok(()) => {
                    if editing_id.get() == Some(id) {
                        clear_form();
                    }
                    refresh();
                }
                Err(err) => list_error.set(Some(format!("failed to delete group: {err}"))),
            }
        });
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        if saving.get() {
            return;
        }
        let Ok(gid_value) = gid.get().trim().parse::<i32>() else {
            form_result.set(Some(Err("GID must be a whole number".to_string())));
            return;
        };
        let req = SaveGroupRequest { name: name.get().trim().to_string(), gid: gid_value };

        let id = editing_id.get();
        saving.set(true);
        form_result.set(None);
        spawn_local(async move {
            let outcome = match id {
                Some(id) => api::put_json::<_, GroupInfo>(&format!("/api/groups/{id}"), &req).await,
                None => api::post_json::<_, GroupInfo>("/api/groups", &req).await,
            };
            saving.set(false);
            match outcome {
                Ok(_) => {
                    form_result.set(Some(Ok(if id.is_some() { "Group updated.".to_string() } else { "Group created.".to_string() })));
                    clear_form();
                    refresh();
                }
                Err(err) => form_result.set(Some(Err(err))),
            }
        });
    };

    view! {
        <div class="tab-panel">
            <p class="hint">
                "A named group is a display name for a GID — assign users to any number of these from the Users tab \
                 (\"Groups\" button) to add extra GIDs to their launches (pod securityContext supplementalGroups), \
                 and, with \"Inject username/group names\" enabled on a template, to name that GID in the launched \
                 container's own /etc/group instead of showing a bare number."
            </p>

            <ErrorBanner error=list_error />

            <div class="table-wrap">
                <table>
                    <thead>
                        <tr>
                            <th>"Name"</th>
                            <th>"GID"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=move || groups.get() key=|g| g.id let(g)>
                            {
                                let id = g.id;
                                let edit_g = g.clone();
                                view! {
                                    <tr>
                                        <td>{g.name.clone()}</td>
                                        <td>{g.gid}</td>
                                        <td class="table-actions">
                                            <button type="button" class="icon-button" on:click=move |_| edit_group(&edit_g)>
                                                "Edit"
                                            </button>
                                            <button type="button" class="icon-button" on:click=move |_| delete_group(id)>
                                                "Delete"
                                            </button>
                                        </td>
                                    </tr>
                                }
                            }
                        </For>
                    </tbody>
                </table>
            </div>

            <h3 class="section-heading">
                {move || if editing_id.get().is_some() { "Edit group" } else { "New group" }}
            </h3>
            <form class="deploy-form" on:submit=on_submit>
                <label>
                    "Name"
                    <input
                        type="text"
                        required=true
                        minlength="3"
                        maxlength="32"
                        placeholder="e.g. data-team"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>
                <label>
                    "GID"
                    <input
                        type="number"
                        required=true
                        min="1"
                        prop:value=move || gid.get()
                        on:input=move |ev| gid.set(event_target_value(&ev))
                    />
                </label>
                <div class="form-actions">
                    <button type="submit" disabled=move || saving.get()>
                        {move || if saving.get() { "Saving…" } else { "Save" }}
                    </button>
                    <Show when=move || editing_id.get().is_some()>
                        <button type="button" class="secondary-button" on:click=move |_| clear_form()>
                            "Cancel"
                        </button>
                    </Show>
                </div>
            </form>

            <ResultBanner result=form_result />
        </div>
    }
}
