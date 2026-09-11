use std::collections::HashMap;

use common::{MyQuota, PodInfo};
use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::api;

use crate::pod_detail::PodDetailPanel;
use crate::{format, ws};

#[component]
pub fn PodsTab(is_admin: bool) -> impl IntoView {
    let pods: RwSignal<HashMap<String, PodInfo>> = RwSignal::new(HashMap::new());
    let connected = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);
    let selected_pod: RwSignal<Option<String>> = RwSignal::new(None);
    let my_quota: RwSignal<Option<MyQuota>> = RwSignal::new(None);
    // Same setting the Launch tab and manage panel already key off —
    // whether resource *requests* (as opposed to limits) are shown at all.
    // Defaults to showing them until the real setting loads, matching
    // those two.
    let expose_requests = move || my_quota.get().map(|q| q.expose_resource_requests).unwrap_or(true);

    spawn_local(async move {
        match api::get_json::<Vec<PodInfo>>("/api/pods").await {
            Ok(list) => pods.update(|map| {
                for pod in list {
                    map.insert(pod.name.clone(), pod);
                }
            }),
            Err(err) => error.set(Some(format!("failed to load pods: {err}"))),
        }
        ws::run(pods, connected, error).await;
    });

    spawn_local(async move {
        if let Ok(quota) = api::get_json::<MyQuota>("/api/quota/me").await {
            my_quota.set(Some(quota));
        }
    });

    let rows = Memo::new(move |_| {
        let mut list: Vec<PodInfo> = pods.get().into_values().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    });

    let total = Memo::new(move |_| rows.get().len());
    let running = Memo::new(move |_| rows.get().iter().filter(|p| p.phase == "Running").count());
    let pending = Memo::new(move |_| rows.get().iter().filter(|p| p.phase == "Pending").count());
    let failed = Memo::new(move |_| rows.get().iter().filter(|p| p.phase == "Failed").count());

    view! {
        <div class="tab-panel">
            <div class="panel-header">
                <span class="status" class:live=move || connected.get()>
                    {move || if connected.get() { "live" } else { "reconnecting…" }}
                </span>
            </div>

            {move || {
                error
                    .get()
                    .map(|msg| view! { <div class="error">{msg}</div> })
            }}

            <div class="stats">
                <div class="stat-tile">
                    <span class="stat-value">{total}</span>
                    <span class="stat-label">"Total pods"</span>
                </div>
                <div class="stat-tile">
                    <span class="stat-value">
                        <span class="stat-dot good"></span>
                        {running}
                    </span>
                    <span class="stat-label">"Running"</span>
                </div>
                <div class="stat-tile">
                    <span class="stat-value">
                        <span class="stat-dot warning"></span>
                        {pending}
                    </span>
                    <span class="stat-label">"Pending"</span>
                </div>
                <div class="stat-tile">
                    <span class="stat-value">
                        <span class="stat-dot critical"></span>
                        {failed}
                    </span>
                    <span class="stat-label">"Failed"</span>
                </div>
            </div>

            <div class="table-wrap">
                <table>
                    <thead>
                        <tr>
                            // First, not last: opening a running environment is
                            // the most common thing anyone does on this tab, and
                            // at the far right it sat past the horizontal scroll.
                            <th>"Access"</th>
                            <th>"Name"</th>
                            <th class:hidden=!is_admin>"Owner"</th>
                            <th>"Status"</th>
                            <th>"Node"</th>
                            <th>"Restarts"</th>
                            <th>"Age"</th>
                            <th class:hidden=move || !expose_requests()>"CPU request"</th>
                            <th>"CPU limit"</th>
                            <th class:hidden=move || !expose_requests()>"Memory request"</th>
                            <th>"Memory limit"</th>
                            <th>"Accelerators"</th>
                            <th>"Credential"</th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=move || rows.get() key=|pod| pod.name.clone() let(pod)>
                            <PodRow pod=pod selected_pod=selected_pod is_admin=is_admin my_quota=my_quota />
                        </For>
                    </tbody>
                </table>

                <Show when=move || rows.get().is_empty()>
                    <p class="empty">"No pods found in this namespace."</p>
                </Show>
            </div>

            {move || {
                selected_pod
                    .get()
                    .map(|name| view! { <PodDetailPanel pods=pods name=name selected=selected_pod /> })
            }}
        </div>
    }
}

#[component]
fn PodRow(pod: PodInfo, selected_pod: RwSignal<Option<String>>, is_admin: bool, my_quota: RwSignal<Option<MyQuota>>) -> impl IntoView {
    let expose_requests = move || my_quota.get().map(|q| q.expose_resource_requests).unwrap_or(true);
    let ready = format!("{}/{}", pod.ready_containers, pod.total_containers);
    let accelerators = format::accelerators(&pod.accelerators);
    let badge_class = format!("badge {}", format::phase_class(&pod.phase));
    let reason = format::pod_reason(&pod.containers);
    let row_name = pod.name.clone();
    let owner = pod.owner.clone().unwrap_or_else(|| "—".into());
    let credential = pod.credential.clone();
    let proxy_path = pod.proxy_path.clone();
    // Split out up front so the view can use each independently without
    // borrowing `pod.access` across the two closures below.
    let external_url = pod.access.as_ref().and_then(|a| a.external_url.clone());
    let internal_address = pod.access.as_ref().map(|a| a.internal.clone());

    view! {
        <tr class="clickable-row" on:click=move |_| selected_pod.set(Some(row_name.clone()))>
            <td>
                {if proxy_path.is_none() && internal_address.is_none() {
                    view! { "—" }.into_any()
                } else {
                    view! {
                        <div class="credential">
                            {proxy_path.map(|path| {
                                view! {
                                    <a
                                        class="icon-button primary-action"
                                        href=path
                                        target="_blank"
                                        title="Open through Helve — already logged in, no token needed"
                                        on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
                                    >
                                        "Open"
                                    </a>
                                }
                            })}
                            {external_url.map(|url| {
                                let title = format!("Direct to this deployment's own address: {url}");
                                view! {
                                    <a
                                        class="icon-button"
                                        href=url
                                        target="_blank"
                                        title=title
                                        on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
                                    >
                                        "Direct"
                                    </a>
                                }
                            })}
                            // Shown even when one of the links above exists: this is
                            // the address another pod uses, which is what a coding
                            // tool pointed at an OpenAI-compatible API needs. The
                            // proxy link can't serve that purpose — it requires an
                            // Helve session a program doesn't have.
                            {internal_address.map(|addr| {
                                view! {
                                    <code
                                        class="access-internal"
                                        title="In-cluster address — reachable from other pods, not from a browser"
                                        on:click=|ev: leptos::ev::MouseEvent| ev.stop_propagation()
                                    >
                                        {addr}
                                    </code>
                                }
                            })}
                        </div>
                    }
                        .into_any()
                }}
            </td>
            <td>{pod.name.clone()}</td>
            <td class:hidden=!is_admin>{owner}</td>
            <td>
                <span class=badge_class>{pod.phase.clone()}</span>
                " "
                <span class="ready">{ready}</span>
                {reason.map(|r| view! { <div class="reason-hint">{r}</div> })}
            </td>
            <td>{pod.node.clone().unwrap_or_else(|| "—".into())}</td>
            <td>{pod.restarts}</td>
            <td>{format::age(pod.start_time.as_deref())}</td>
            <td class:hidden=move || !expose_requests()>{format::millicores(pod.cpu_request_millicores)}</td>
            <td>{format::millicores(pod.cpu_limit_millicores)}</td>
            <td class:hidden=move || !expose_requests()>{format::bytes(pod.memory_request_bytes)}</td>
            <td>{format::bytes(pod.memory_limit_bytes)}</td>
            <td>{accelerators}</td>
            <td>
                {match credential {
                    None => view! { "—" }.into_any(),
                    Some(cred) => view! {
                        <div class="credential">
                            <span class="credential-key">{cred.env_key}</span>
                            <code class="credential-value" title="Click to select, then copy">
                                {cred.value}
                            </code>
                        </div>
                    }
                        .into_any(),
                }}
            </td>
        </tr>
    }
}
