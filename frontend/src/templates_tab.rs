use common::{PvcEntry, SaveTemplateRequest, TemplateEntry};
use leptos::prelude::*;
use leptos::tachys::dom::{event_target_checked, event_target_value};
use leptos::task::spawn_local;

use crate::api;
use crate::result_banner::{ErrorBanner, ResultBanner};

use crate::env_editor::{EnvVars, EnvVarsEditor};
use crate::format;

#[component]
pub fn TemplatesTab() -> impl IntoView {
    let templates: RwSignal<Vec<TemplateEntry>> = RwSignal::new(Vec::new());
    let list_error = RwSignal::new(None::<String>);

    let editing_id = RwSignal::new(None::<i32>);
    let name = RwSignal::new(String::new());
    let image = RwSignal::new(String::new());
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
    let home_mount_path = RwSignal::new(String::new());
    let inject_identity_files = RwSignal::new(false);
    let pvcs: RwSignal<Vec<PvcEntry>> = RwSignal::new(Vec::new());
    let notes_text = RwSignal::new(String::new());
    let secret_env_key = RwSignal::new(String::new());
    let proxy_enabled = RwSignal::new(false);
    let strip_prefix = RwSignal::new(false);
    // Off by default; see the matching default in create_deployment_tab.rs.
    let public_service = RwSignal::new(false);

    let saving = RwSignal::new(false);
    let form_result: RwSignal<Option<Result<String, String>>> = RwSignal::new(None);

    // Show a model-serving field only once this template's own args
    // reference its placeholder — same rule, and the same reasoning, as in
    // create_deployment_tab.rs. Type `--model={{model}}` into Args below
    // and the Model field appears.
    let uses = move |placeholder: &'static str| move || args_text.get().contains(placeholder);

    let refresh = move || {
        spawn_local(async move {
            match api::get_json::<Vec<TemplateEntry>>("/api/templates").await {
                Ok(list) => {
                    list_error.set(None);
                    templates.set(list);
                }
                Err(err) => list_error.set(Some(format!("failed to load templates: {err}"))),
            }
        });
    };
    refresh();

    spawn_local(async move {
        if let Ok(list) = api::get_json::<Vec<PvcEntry>>("/api/pvcs").await {
            pvcs.set(list);
        }
    });

    let clear_form = move || {
        editing_id.set(None);
        name.set(String::new());
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
        home_mount_path.set(String::new());
        inject_identity_files.set(false);
        notes_text.set(String::new());
        secret_env_key.set(String::new());
        proxy_enabled.set(false);
        strip_prefix.set(false);
        public_service.set(false);
    };

    let load_into_form = move |t: &TemplateEntry| {
        editing_id.set(Some(t.id));
        name.set(t.name.clone());
        image.set(t.image.clone());
        container_port.set(t.container_port.map(|p| p.to_string()).unwrap_or_default());
        readiness_path.set(t.readiness_path.clone());
        cpu_request.set(t.cpu_request.clone());
        cpu_limit.set(t.cpu_limit.clone());
        memory_request.set(t.memory_request.clone());
        memory_limit.set(t.memory_limit.clone());
        accelerator_type.set(t.accelerator_type.clone());
        accelerator_count.set(t.accelerator_count.map(|c| c.to_string()).unwrap_or_default());
        env_vars.set_from(&t.env);
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
        home_mount_path.set(t.home_mount_path.clone());
        inject_identity_files.set(t.inject_identity_files);
        notes_text.set(t.notes.clone());
        secret_env_key.set(t.secret_env_key.clone().unwrap_or_default());
        proxy_enabled.set(t.proxy_enabled);
        strip_prefix.set(t.strip_prefix);
        public_service.set(t.public_service);
        form_result.set(None);
    };

    let delete_template = move |id: i32| {
        let confirmed = web_sys::window()
            .and_then(|w| w.confirm_with_message("Delete this template? This can't be undone.").ok())
            .unwrap_or(false);
        if !confirmed {
            return;
        }
        spawn_local(async move {
            match api::delete(&format!("/api/templates/{id}")).await {
                Ok(()) => {
                    if editing_id.get() == Some(id) {
                        clear_form();
                    }
                    refresh();
                }
                Err(err) => list_error.set(Some(format!("failed to delete template: {err}"))),
            }
        });
    };

    let on_submit = move |ev: web_sys::SubmitEvent| {
        ev.prevent_default();
        if saving.get() {
            return;
        }

        let accel_type = accelerator_type.get();
        let accel_count = if accel_type.is_empty() { None } else { accelerator_count.get().trim().parse::<i64>().ok() };
        let args: Vec<String> = args_text.get().lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();

        let req = SaveTemplateRequest {
            name: name.get().trim().to_string(),
            image: image.get().trim().to_string(),
            container_port: container_port.get().trim().parse().ok(),
            readiness_path: readiness_path.get().trim().to_string(),
            cpu_request: cpu_request.get().trim().to_string(),
            cpu_limit: cpu_limit.get().trim().to_string(),
            memory_request: memory_request.get().trim().to_string(),
            memory_limit: memory_limit.get().trim().to_string(),
            accelerator_type: accel_type,
            accelerator_count: accel_count,
            env: env_vars.to_pairs(),
            args,
            model: model.get().trim().to_string(),
            context_length: context_length.get().trim().parse().ok(),
            quantization: quantization.get().trim().to_string(),
            served_model_name: served_model_name.get().trim().to_string(),
            gpu_memory_utilization: gpu_memory_utilization.get().trim().parse().ok(),
            dtype: dtype.get().trim().to_string(),
            volume_claim_name: volume_claim_name.get().trim().to_string(),
            volume_mount_path: volume_mount_path.get().trim().to_string(),
            volume_sub_path: volume_sub_path.get().trim().to_string(),
            home_mount_path: home_mount_path.get().trim().to_string(),
            inject_identity_files: inject_identity_files.get(),
            notes: notes_text.get(),
            secret_env_key: {
                let key = secret_env_key.get().trim().to_string();
                if key.is_empty() { None } else { Some(key) }
            },
            proxy_enabled: proxy_enabled.get(),
            strip_prefix: strip_prefix.get(),
            public_service: public_service.get(),
        };

        let id = editing_id.get();
        saving.set(true);
        form_result.set(None);
        spawn_local(async move {
            let outcome = save(id, req).await;
            saving.set(false);
            match outcome {
                Ok(msg) => {
                    form_result.set(Some(Ok(msg)));
                    clear_form();
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
                            <th>"Name"</th>
                            <th>"Image"</th>
                            <th>"Port"</th>
                            <th>"CPU"</th>
                            <th>"Memory"</th>
                            <th>"Accelerator"</th>
                            <th>"Secret"</th>
                            <th>"Proxy"</th>
                            <th>"Public"</th>
                            <th></th>
                        </tr>
                    </thead>
                    <tbody>
                        <For each=move || templates.get() key=|t| t.id let(t)>
                            {
                                let t_for_edit = t.clone();
                                let id = t.id;
                                view! {
                                    <tr>
                                        <td>{t.name.clone()}</td>
                                        <td>{t.image.clone()}</td>
                                        <td>{t.container_port.map(|p| p.to_string()).unwrap_or_else(|| "—".into())}</td>
                                        <td>{format!("{} / {}", or_dash(&t.cpu_request), or_dash(&t.cpu_limit))}</td>
                                        <td>{format!("{} / {}", or_dash(&t.memory_request), or_dash(&t.memory_limit))}</td>
                                        <td>{format::accelerator_summary(&t.accelerator_type, t.accelerator_count)}</td>
                                        <td>{t.secret_env_key.clone().unwrap_or_else(|| "—".into())}</td>
                                        <td>{if t.proxy_enabled { "yes" } else { "—" }}</td>
                                        <td>{if t.public_service { "yes" } else { "no" }}</td>
                                        <td class="table-actions">
                                            <button type="button" class="icon-button" on:click=move |_| load_into_form(&t_for_edit)>
                                                "Edit"
                                            </button>
                                            <button type="button" class="icon-button" on:click=move |_| delete_template(id)>
                                                "Delete"
                                            </button>
                                        </td>
                                    </tr>
                                }
                            }
                        </For>
                    </tbody>
                </table>
                <Show when=move || templates.get().is_empty() && list_error.get().is_none()>
                    <p class="empty">"No templates yet — add one below."</p>
                </Show>
            </div>

            <h3 class="section-heading">
                {move || if editing_id.get().is_some() { "Edit template" } else { "New template" }}
            </h3>

            <form class="deploy-form" on:submit=on_submit>
                <label>
                    "Name"
                    <input
                        type="text"
                        required=true
                        maxlength="100"
                        prop:value=move || name.get()
                        on:input=move |ev| name.set(event_target_value(&ev))
                    />
                </label>

                <label>
                    "Image"
                    <input
                        type="text"
                        required=true
                        maxlength="512"
                        placeholder="e.g. ollama/ollama:latest"
                        prop:value=move || image.get()
                        on:input=move |ev| image.set(event_target_value(&ev))
                    />
                </label>

                <label>
                    "Container port (optional)"
                    <input
                        type="number"
                        min="1"
                        max="65535"
                        step="1"
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

                <fieldset>
                    <legend>"CPU"</legend>
                    <label>
                        "Request"
                        <input
                            type="text"
                            placeholder="e.g. 250m"
                            prop:value=move || cpu_request.get()
                            on:input=move |ev| cpu_request.set(event_target_value(&ev))
                        />
                    </label>
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
                    <label>
                        "Request"
                        <input
                            type="text"
                            placeholder="e.g. 256Mi"
                            prop:value=move || memory_request.get()
                            on:input=move |ev| memory_request.set(event_target_value(&ev))
                        />
                    </label>
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
                    <legend>"Environment variable defaults (optional)"</legend>
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
                    "Home directory mount path (optional)"
                    <input
                        type="text"
                        maxlength="512"
                        placeholder="e.g. /home/jovyan"
                        prop:value=move || home_mount_path.get()
                        on:input=move |ev| home_mount_path.set(event_target_value(&ev))
                    />
                    <div class="hint">
                        "Only takes effect if this deployment of Helve has a home-drive mode configured; leave blank for templates (Ollama, vLLM, SGLang) that don't need one."
                    </div>
                </label>

                <label class="checkbox">
                    <input
                        type="checkbox"
                        prop:checked=move || inject_identity_files.get()
                        on:change=move |ev| inject_identity_files.set(event_target_checked(&ev))
                    />
                    "Inject username/group names into /etc/passwd and /etc/group (so a shell or file listing shows names instead of bare UID/GID numbers) — requires the image to have a POSIX shell, and only takes effect for a launching user with a UID, GID, or supplemental group actually assigned"
                </label>

                <label>
                    "Command arguments (optional, one per line)"
                    <textarea
                        rows="2"
                        placeholder="--tensor-parallel-size={{accelerator_count}}"
                        prop:value=move || args_text.get()
                        on:input=move |ev| args_text.set(event_target_value(&ev))
                    ></textarea>
                    <div class="hint">
                        "\"{{name}}\" is this deployment's own generated name, \"{{accelerator_count}}\" is however many accelerators were requested (defaults to 1), and \"{{model}}\"/\"{{context_length}}\"/\"{{quantization}}\"/\"{{served_model_name}}\"/\"{{gpu_memory_utilization}}\"/\"{{dtype}}\" are the fields above — a line referencing one of those six is dropped entirely if left blank, rather than sending a broken flag."
                    </div>
                </label>

                <label>
                    "Notes (shown when this template is selected on the Launch tab)"
                    <textarea
                        rows="2"
                        maxlength="2000"
                        prop:value=move || notes_text.get()
                        on:input=move |ev| notes_text.set(event_target_value(&ev))
                    ></textarea>
                </label>

                <label>
                    "Auto-generate a secret for this env var (optional)"
                    <input
                        type="text"
                        maxlength="128"
                        placeholder="e.g. JUPYTER_TOKEN, PASSWORD, VLLM_API_KEY"
                        prop:value=move || secret_env_key.get()
                        on:input=move |ev| secret_env_key.set(event_target_value(&ev))
                    />
                </label>

                <label class="checkbox">
                    <input
                        type="checkbox"
                        prop:checked=move || proxy_enabled.get()
                        on:change=move |ev| proxy_enabled.set(event_target_checked(&ev))
                    />
                    "Also reachable via Helve's own /proxy/<name>/ route (injects the generated secret above, if any)"
                </label>

                <label class="checkbox">
                    <input
                        type="checkbox"
                        disabled=move || !proxy_enabled.get()
                        prop:checked=move || strip_prefix.get()
                        on:change=move |ev| strip_prefix.set(event_target_checked(&ev))
                    />
                    "Strip the /proxy/<name>/ prefix before forwarding (for apps like RStudio that expect to run at the root path, unlike JupyterLab)"
                </label>

                <label class="checkbox">
                    <input
                        type="checkbox"
                        prop:checked=move || public_service.get()
                        on:change=move |ev| public_service.set(event_target_checked(&ev))
                    />
                    "Public LoadBalancer Service (uncheck to make it internal — reachable only from inside the cluster, e.g. by other tooling pods, or via Helve's proxy for apps with no auth of their own)"
                </label>

                <div class="form-actions">
                    <button type="submit" disabled=move || saving.get()>
                        {move || {
                            if saving.get() {
                                "Saving…"
                            } else if editing_id.get().is_some() {
                                "Save changes"
                            } else {
                                "Create template"
                            }
                        }}
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

fn or_dash(s: &str) -> &str {
    if s.trim().is_empty() { "—" } else { s }
}

async fn save(id: Option<i32>, req: SaveTemplateRequest) -> Result<String, String> {
    let saved: TemplateEntry = match id {
        Some(id) => api::put_json(&format!("/api/templates/{id}"), &req).await,
        None => api::post_json("/api/templates", &req).await,
    }
    .map_err(|err| format!("Failed to save template: {err}"))?;
    Ok(format!("Saved template \"{}\".", saved.name))
}
