use common::{CreateDeploymentRequest, CreateDeploymentResponse, ImageEntry, MyQuota, PvcEntry, TemplateEntry};
use leptos::prelude::*;
use leptos::tachys::dom::event_target_value;
use leptos::task::spawn_local;

use crate::api;
use crate::result_banner::{ErrorBanner};

use crate::env_editor::{EnvVars, EnvVarsEditor};
use crate::format::{fixed_request_note, quota_summary};

/// A successful launch's (message, proxy_path) pair.
type LaunchResult = Result<(String, Option<String>), String>;

#[component]
pub fn CreateDeploymentTab(is_admin: bool) -> impl IntoView {
    let images: RwSignal<Vec<ImageEntry>> = RwSignal::new(Vec::new());
    let images_error = RwSignal::new(None::<String>);
    let templates: RwSignal<Vec<TemplateEntry>> = RwSignal::new(Vec::new());
    let templates_error = RwSignal::new(None::<String>);

    let selected_template_id = RwSignal::new(String::new());
    let selected_template_name = RwSignal::new(None::<String>);

    let image = RwSignal::new(String::new());
    let replicas = RwSignal::new("1".to_string());
    let container_port = RwSignal::new(String::new());
    let readiness_path = RwSignal::new(String::new());
    let cpu_request = RwSignal::new(String::new());
    let cpu_limit = RwSignal::new(String::new());
    let memory_request = RwSignal::new(String::new());
    let memory_limit = RwSignal::new(String::new());
    let accelerator_type = RwSignal::new(String::new());
    let accelerator_count = RwSignal::new(String::new());
    let env_vars = EnvVars::new();
    let args_text = RwSignal::new(String::new());
    let model = RwSignal::new(String::new());
    let context_length = RwSignal::new(String::new());
    let quantization = RwSignal::new(String::new());
    let served_model_name = RwSignal::new(String::new());
    let gpu_memory_utilization = RwSignal::new(String::new());
    let dtype = RwSignal::new(String::new());
    let volume_claim_name = RwSignal::new(String::new());
    let volume_mount_path = RwSignal::new(String::new());
    let volume_sub_path = RwSignal::new(String::new());
    let pvcs: RwSignal<Vec<PvcEntry>> = RwSignal::new(Vec::new());
    let notes = RwSignal::new(None::<String>);
    let secret_env_key = RwSignal::new(None::<String>);
    let proxy_enabled = RwSignal::new(false);
    let strip_prefix = RwSignal::new(false);
    // Off by default: per-deployment proxy origins are the intended access
    // path now, and a public LoadBalancer Service is the thing most likely
    // to sit stuck at <pending> on a cluster with no LB controller.
    let public_service = RwSignal::new(false);

    let submitting = RwSignal::new(false);
    let result: RwSignal<Option<LaunchResult>> = RwSignal::new(None);

    let my_quota: RwSignal<Option<MyQuota>> = RwSignal::new(None);
    // Defaults to showing request fields until the real setting loads, so
    // the form doesn't flash from "with requests" to "without" on a slow
    // connection - matches this app's existing behavior before quotas
    // existed.
    let expose_requests = move || my_quota.get().map(|q| q.expose_resource_requests).unwrap_or(true);
    // Same before-load-defaults-to-true reasoning as expose_requests above.
    // Admins are always allowed regardless of the setting — checked here
    // too (not just server-side) so the "Custom" option/free-text image
    // editing isn't hidden from the one role it was never meant to gate.
    let allow_custom = move || is_admin || my_quota.get().map(|q| q.allow_custom_images).unwrap_or(true);
    // The model-serving fields below (model, context length, quantization,
    // ...) exist only to be substituted into `args` — `{{model}}` and
    // friends, see `substitute_placeholders` in backend/src/deployments.rs.
    // A template whose args never mention a placeholder (JupyterLab,
    // RStudio) would collect the value and then silently discard it, so
    // don't ask for it at all.
    //
    // Keyed off the args themselves rather than a hardcoded vLLM/SGLang
    // image list: a new engine template gets the right fields with no
    // frontend change, and editing `args` to add a placeholder reveals its
    // field immediately.
    let uses = move |placeholder: &'static str| move || args_text.get().contains(placeholder);

    spawn_local(async move {
        if let Ok(quota) = api::get_json::<MyQuota>("/api/quota/me").await {
            my_quota.set(Some(quota));
        }
    });

    spawn_local(async move {
        match api::get_json::<Vec<ImageEntry>>("/api/images").await {
            Ok(list) => images.set(list),
            Err(err) => images_error.set(Some(format!("failed to load images: {err}"))),
        }
    });

    spawn_local(async move {
        match api::get_json::<Vec<TemplateEntry>>("/api/templates").await {
            Ok(list) => templates.set(list),
            Err(err) => templates_error.set(Some(format!("failed to load templates: {err}"))),
        }
    });

    spawn_local(async move {
        if let Ok(list) = api::get_json::<Vec<PvcEntry>>("/api/pvcs").await {
            pvcs.set(list);
        }
    });

    let apply_template = move |t: &TemplateEntry| {
        selected_template_name.set(Some(t.name.clone()));
        notes.set(if t.notes.trim().is_empty() { None } else { Some(t.notes.clone()) });
        image.set(t.image.clone());
        container_port.set(t.container_port.map(|p| p.to_string()).unwrap_or_default());
        readiness_path.set(t.readiness_path.clone());
        cpu_request.set(t.cpu_request.clone());
        cpu_limit.set(t.cpu_limit.clone());
        memory_request.set(t.memory_request.clone());
        memory_limit.set(t.memory_limit.clone());
        accelerator_type.set(t.accelerator_type.clone());
        accelerator_count.set(t.accelerator_count.map(|c| c.to_string()).unwrap_or_default());
        // A secret env var is auto-generated at launch, not typed in — never show it as an editable row.
        let env: Vec<(String, String)> =
            t.env.iter().filter(|(k, _)| Some(k) != t.secret_env_key.as_ref()).cloned().collect();
        env_vars.set_from(&env);
        args_text.set(t.args.join("\n"));
        model.set(t.model.clone());
        context_length.set(t.context_length.map(|n| n.to_string()).unwrap_or_default());
        quantization.set(t.quantization.clone());
        served_model_name.set(t.served_model_name.clone());
        gpu_memory_utilization.set(t.gpu_memory_utilization.map(|f| f.to_string()).unwrap_or_default());
        dtype.set(t.dtype.clone());
        volume_claim_name.set(t.volume_claim_name.clone());
        volume_mount_path.set(t.volume_mount_path.clone());
        volume_sub_path.set(t.volume_sub_path.clone());
        secret_env_key.set(t.secret_env_key.clone());
        proxy_enabled.set(t.proxy_enabled);
        strip_prefix.set(t.strip_prefix);
        public_service.set(t.public_service);
    };

    let reset_to_custom = move || {
        selected_template_name.set(None);
        notes.set(None);
        image.set(String::new());
        container_port.set(String::new());
        readiness_path.set(String::new());
        cpu_request.set(String::new());
        cpu_limit.set(String::new());
        memory_request.set(String::new());
        memory_limit.set(String::new());
        accelerator_type.set(String::new());
        accelerator_count.set(String::new());
        env_vars.set_from(&[]);
        args_text.set(String::new());
        model.set(String::new());
        context_length.set(String::new());
        quantization.set(String::new());
        served_model_name.set(String::new());
        gpu_memory_utilization.set(String::new());
        dtype.set(String::new());
        volume_claim_name.set(String::new());
        volume_mount_path.set(String::new());
        volume_sub_path.set(String::new());
        secret_env_key.set(None);
        proxy_enabled.set(false);
        strip_prefix.set(false);
        public_service.set(false);
    };

    let on_template_change = move |ev: web_sys::Event| {
        let value = event_target_value(&ev);
        selected_template_id.set(value.clone());
        if value.is_empty() {
            reset_to_custom();
            return;
        }
        if let Ok(id) = value.parse::<i32>()
            && let Some(t) = templates.get().iter().find(|t| t.id == id) {
                apply_template(t);
            }
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        if submitting.get() {
            return;
        }

        let accel_type = accelerator_type.get();
        let accel_count = if accel_type.is_empty() {
            None
        } else {
            accelerator_count.get().trim().parse::<i64>().ok()
        };
        let args: Vec<String> = args_text.get().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();

        let req = CreateDeploymentRequest {
            template_name: selected_template_name.get(),
            image: image.get(),
            replicas: replicas.get().trim().parse().unwrap_or(1),
            cpu_request: if expose_requests() { non_empty(cpu_request.get()) } else { None },
            cpu_limit: non_empty(cpu_limit.get()),
            memory_request: if expose_requests() { non_empty(memory_request.get()) } else { None },
            memory_limit: non_empty(memory_limit.get()),
            accelerator_type: if accel_type.is_empty() { None } else { Some(accel_type) },
            accelerator_count: accel_count,
            container_port: container_port.get().trim().parse().ok(),
            readiness_path: non_empty(readiness_path.get()),
            env: env_vars.to_pairs(),
            args,
            model: non_empty(model.get()),
            context_length: context_length.get().trim().parse().ok(),
            quantization: non_empty(quantization.get()),
            served_model_name: non_empty(served_model_name.get()),
            gpu_memory_utilization: gpu_memory_utilization.get().trim().parse().ok(),
            dtype: non_empty(dtype.get()),
            volume_claim_name: non_empty(volume_claim_name.get()),
            volume_mount_path: non_empty(volume_mount_path.get()),
            volume_sub_path: non_empty(volume_sub_path.get()),
            generate_secret_for: secret_env_key.get(),
            enable_proxy: proxy_enabled.get(),
            strip_prefix: strip_prefix.get(),
            public_service: public_service.get(),
        };

        submitting.set(true);
        result.set(None);
        spawn_local(async move {
            let outcome = submit(req).await;
            submitting.set(false);
            result.set(Some(outcome));
        });
    };

    view! {
        <div class="tab-panel">
            <ErrorBanner error=images_error />
            <ErrorBanner error=templates_error />

            <form class="deploy-form" on:submit=on_submit>
                <label>
                    "Template"
                    <select prop:value=move || selected_template_id.get() on:change=on_template_change>
                        {move || allow_custom().then(|| view! { <option value="">"Custom"</option> })}
                        <For each=move || templates.get() key=|t| t.id let(t)>
                            <option value=t.id.to_string()>{t.name.clone()}</option>
                        </For>
                    </select>
                </label>

                {move || notes.get().map(|n| view! { <div class="template-notes">{n}</div> })}
                {move || {
                    secret_env_key
                        .get()
                        .map(|key| {
                            view! {
                                <div class="template-notes">
                                    {format!(
                                        "A random {key} will be generated automatically and shown here after launch — no need to set one yourself.",
                                    )}
                                </div>
                            }
                        })
                }}
                {move || {
                    let text = if proxy_enabled.get() {
                        if public_service.get() {
                            Some("Also opens through Helve directly — no separate login needed via that route.")
                        } else {
                            Some(
                                "Opens through Helve only — no public IP, no login of its own; Helve's own login is the only way in.",
                            )
                        }
                    } else if !public_service.get() {
                        Some(
                            "Internal only — no public IP. Reachable only from inside the cluster (e.g. other tooling pods), not from outside it.",
                        )
                    } else {
                        None
                    };
                    text.map(|text| view! { <div class="template-notes">{text}</div> })
                }}

                <label>
                    "Image"
                    <input
                        type="text"
                        list="image-catalog"
                        required=true
                        maxlength="512"
                        placeholder="e.g. nginx:stable"
                        prop:value=move || image.get()
                        prop:disabled=move || !allow_custom()
                        on:input=move |ev| image.set(event_target_value(&ev))
                    />
                    <datalist id="image-catalog">
                        <For each=move || images.get() key=|img| img.id let(img)>
                            <option value=img.image.clone()>{img.name.clone()}</option>
                        </For>
                    </datalist>
                    {move || {
                        (!allow_custom())
                            .then(|| view! { <div class="hint">"An admin has restricted image selection to the catalog and existing templates — pick a Template above."</div> })
                    }}
                </label>

                <label>
                    "Replicas"
                    <input
                        type="number"
                        min="0"
                        step="1"
                        prop:value=move || replicas.get()
                        on:input=move |ev| replicas.set(event_target_value(&ev))
                    />
                </label>

                <label>
                    "Container port (optional)"
                    <input
                        type="number"
                        min="1"
                        max="65535"
                        step="1"
                        placeholder="e.g. 8080 — creates a LoadBalancer Service"
                        prop:value=move || container_port.get()
                        on:input=move |ev| container_port.set(event_target_value(&ev))
                    />
                </label>

                <label>
                    "Readiness probe path (optional)"
                    <input
                        type="text"
                        placeholder="e.g. /health — requires a container port"
                        prop:value=move || readiness_path.get()
                        on:input=move |ev| readiness_path.set(event_target_value(&ev))
                    />
                </label>

                {move || {
                    my_quota.get().map(|q| view! { <div class="template-notes">{quota_summary(&q)}</div> })
                }}
                {move || {
                    my_quota.get().and_then(|q| fixed_request_note(&q)).map(|note| {
                        view! { <div class="template-notes">{note}</div> }
                    })
                }}

                <fieldset>
                    <legend>"CPU"</legend>
                    <Show when=expose_requests>
                        <label>
                            "Request"
                            <input
                                type="text"
                                placeholder="e.g. 250m"
                                prop:value=move || cpu_request.get()
                                on:input=move |ev| cpu_request.set(event_target_value(&ev))
                            />
                        </label>
                    </Show>
                    <label>
                        "Limit"
                        <input
                            type="text"
                            placeholder="e.g. 1"
                            prop:value=move || cpu_limit.get()
                            on:input=move |ev| cpu_limit.set(event_target_value(&ev))
                        />
                    </label>
                </fieldset>

                <fieldset>
                    <legend>"Memory"</legend>
                    <Show when=expose_requests>
                        <label>
                            "Request"
                            <input
                                type="text"
                                placeholder="e.g. 256Mi"
                                prop:value=move || memory_request.get()
                                on:input=move |ev| memory_request.set(event_target_value(&ev))
                            />
                        </label>
                    </Show>
                    <label>
                        "Limit"
                        <input
                            type="text"
                            placeholder="e.g. 512Mi"
                            prop:value=move || memory_limit.get()
                            on:input=move |ev| memory_limit.set(event_target_value(&ev))
                        />
                    </label>
                </fieldset>

                <fieldset>
                    <legend>"Accelerator (optional)"</legend>
                    <label>
                        "Type"
                        <select
                            prop:value=move || accelerator_type.get()
                            on:change=move |ev| accelerator_type.set(event_target_value(&ev))
                        >
                            <option value="">"None"</option>
                            <option value="nvidia.com/gpu">"nvidia.com/gpu"</option>
                            <option value="amd.com/gpu">"amd.com/gpu"</option>
                            <option value="gpu.intel.com/i915">"gpu.intel.com/i915"</option>
                        </select>
                    </label>
                    <label>
                        "Count"
                        <input
                            type="number"
                            min="1"
                            step="1"
                            disabled=move || accelerator_type.get().is_empty()
                            prop:value=move || accelerator_count.get()
                            on:input=move |ev| accelerator_count.set(event_target_value(&ev))
                        />
                    </label>
                </fieldset>

                <fieldset class="env-fieldset">
                    <legend>"Environment variables (optional)"</legend>
                    <EnvVarsEditor vars=env_vars />
                </fieldset>

                <Show when=uses("{{model}}")>
                    <label>
                        "Model (optional)"
                        <input
                            type="text"
                            maxlength="500"
                            placeholder="e.g. meta-llama/Llama-3-8B, or a local path under the mount below"
                            prop:value=move || model.get()
                            on:input=move |ev| model.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <Show when=uses("{{context_length}}")>
                    <label>
                        "Context length (optional)"
                        <input
                            type="number"
                            min="1"
                            step="1"
                            placeholder="e.g. 8192"
                            prop:value=move || context_length.get()
                            on:input=move |ev| context_length.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <Show when=uses("{{quantization}}")>
                    <label>
                        "Quantization (optional)"
                        <input
                            type="text"
                            maxlength="100"
                            placeholder="e.g. awq, gptq, fp8"
                            prop:value=move || quantization.get()
                            on:input=move |ev| quantization.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <Show when=uses("{{served_model_name}}")>
                    <label>
                        "Served model name (optional)"
                        <input
                            type="text"
                            maxlength="200"
                            placeholder="a short name for the OpenAI-compatible API, if different from Model"
                            prop:value=move || served_model_name.get()
                            on:input=move |ev| served_model_name.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <Show when=uses("{{gpu_memory_utilization}}")>
                    <label>
                        "GPU memory utilization (optional)"
                        <input
                            type="number"
                            min="0.01"
                            max="1"
                            step="0.01"
                            placeholder="e.g. 0.9 — fraction of GPU memory to reserve"
                            prop:value=move || gpu_memory_utilization.get()
                            on:input=move |ev| gpu_memory_utilization.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <Show when=uses("{{dtype}}")>
                    <label>
                        "Dtype (optional)"
                        <input
                            type="text"
                            maxlength="50"
                            placeholder="e.g. float16, bfloat16, auto"
                            prop:value=move || dtype.get()
                            on:input=move |ev| dtype.set(event_target_value(&ev))
                        />
                    </label>
                </Show>

                <fieldset>
                    <legend>"Storage mount (optional)"</legend>
                    <label>
                        "Existing PersistentVolumeClaim"
                        <input
                            type="text"
                            list="pvc-catalog"
                            maxlength="63"
                            placeholder="e.g. ollama-models"
                            prop:value=move || volume_claim_name.get()
                            on:input=move |ev| volume_claim_name.set(event_target_value(&ev))
                        />
                        <datalist id="pvc-catalog">
                            <For each=move || pvcs.get() key=|p| p.name.clone() let(p)>
                                <option value=p.name.clone()>{p.capacity.clone().unwrap_or_default()}</option>
                            </For>
                        </datalist>
                    </label>
                    <label>
                        "Mount path"
                        <input
                            type="text"
                            maxlength="512"
                            placeholder="e.g. /mnt/models"
                            disabled=move || volume_claim_name.get().trim().is_empty()
                            prop:value=move || volume_mount_path.get()
                            on:input=move |ev| volume_mount_path.set(event_target_value(&ev))
                        />
                    </label>
                    <label>
                        "Subpath within the claim (optional)"
                        <input
                            type="text"
                            maxlength="512"
                            disabled=move || volume_claim_name.get().trim().is_empty()
                            prop:value=move || volume_sub_path.get()
                            on:input=move |ev| volume_sub_path.set(event_target_value(&ev))
                        />
                    </label>
                </fieldset>

                <label>
                    "Command arguments (optional, one per line)"
                    <textarea
                        rows="2"
                        placeholder="--tensor-parallel-size={{accelerator_count}}"
                        prop:value=move || args_text.get()
                        on:input=move |ev| args_text.set(event_target_value(&ev))
                    ></textarea>
                    <div class="hint">
                        "\"{{name}}\" is this deployment's own generated name, \"{{accelerator_count}}\" is the Accelerator count above (defaults to 1), and \"{{model}}\"/\"{{context_length}}\"/\"{{quantization}}\"/\"{{served_model_name}}\"/\"{{gpu_memory_utilization}}\"/\"{{dtype}}\" are the fields above — a line referencing one of those six is dropped entirely if left blank, rather than sending a broken flag."
                    </div>
                </label>

                <button type="submit" disabled=move || submitting.get()>
                    {move || if submitting.get() { "Launching…" } else { "Launch" }}
                </button>
            </form>

            {move || {
                result.get().map(|res| match res {
                    Ok((msg, proxy_path)) => {
                        view! {
                            <div class="success">
                                {msg}
                                {proxy_path.map(|path| {
                                    view! {
                                        " "
                                        <a class="icon-button" href=path target="_blank">
                                            "Open"
                                        </a>
                                    }
                                })}
                            </div>
                        }
                            .into_any()
                    }
                    Err(msg) => view! { <div class="error">{msg}</div> }.into_any(),
                })
            }}
        </div>
    }
}

fn non_empty(s: String) -> Option<String> {
    let trimmed = s.trim();
    if trimmed.is_empty() { None } else { Some(trimmed.to_string()) }
}

async fn submit(req: CreateDeploymentRequest) -> LaunchResult {
    let created: CreateDeploymentResponse =
        api::post_json("/api/deployments", &req).await.map_err(|err| format!("Failed to create deployment: {err}"))?;
    {
        let mut msg = format!("Created deployment \"{}\" in namespace \"{}\".", created.name, created.namespace);
        if let (Some(service), Some(port)) = (created.service_name, created.container_port) {
            if created.public_service {
                msg.push_str(&format!(
                    " Exposed via Service \"{service}\" on port {port} — check `kubectl get svc -n {}` for its external IP.",
                    created.namespace
                ));
            } else if created.proxy_path.is_some() {
                msg.push_str(&format!(
                    " Service \"{service}\" has no public IP — reachable only through Helve's own login."
                ));
            } else {
                msg.push_str(&format!(
                    " Service \"{service}\" has no public IP — reachable only from inside the cluster."
                ));
            }
        }
        if let Some(secret) = created.secret_value {
            msg.push_str(&format!(" Generated credential: {secret} (also shown on the Pods tab)."));
        }
        Ok((msg, created.proxy_path))
    }
}
