use std::collections::BTreeMap;

use common::{ContainerStatusInfo, PodInfo};
use k8s_openapi::api::core::v1::{ContainerStatus, Pod};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;

/// Label set on launched Deployments/pods/Services recording who launched
/// them, via `CreateDeploymentRequest` — see `backend/src/deployments.rs`.
pub const OWNER_LABEL: &str = "helve.io/owner";

/// Converts a Kubernetes `Pod` into our slimmed-down `PodInfo`, aggregating
/// resource requests/limits (and accelerator resources) across all containers.
pub fn pod_to_info(pod: &Pod) -> PodInfo {
    let name = pod.metadata.name.clone().unwrap_or_default();
    let namespace = pod.metadata.namespace.clone().unwrap_or_default();
    let labels = pod.metadata.labels.as_ref();
    let owner = labels.and_then(|l| l.get(OWNER_LABEL)).cloned();
    let deployment_name = labels.and_then(|l| l.get("app")).cloned();

    let spec = pod.spec.as_ref();
    let status = pod.status.as_ref();

    let node = spec.and_then(|s| s.node_name.clone());
    let phase = status
        .and_then(|s| s.phase.clone())
        .unwrap_or_else(|| "Unknown".to_string());
    // `Time` wraps a `jiff::Timestamp`, whose `Display` impl already emits RFC 3339.
    let start_time = status
        .and_then(|s| s.start_time.as_ref())
        .map(|t| t.0.to_string());

    let total_containers = spec.map(|s| s.containers.len() as u32).unwrap_or(0);
    let containers: Vec<ContainerStatusInfo> = status
        .and_then(|s| s.container_statuses.as_ref())
        .map(|statuses| statuses.iter().map(container_status_info).collect())
        .unwrap_or_default();
    let ready_containers = containers.iter().filter(|c| c.ready).count() as u32;
    let restarts = containers.iter().map(|c| c.restart_count).sum();

    let mut cpu_request_millicores: Option<i64> = None;
    let mut cpu_limit_millicores: Option<i64> = None;
    let mut memory_request_bytes: Option<i64> = None;
    let mut memory_limit_bytes: Option<i64> = None;
    let mut accelerators: BTreeMap<String, i64> = BTreeMap::new();

    if let Some(spec) = spec {
        for container in &spec.containers {
            let Some(resources) = &container.resources else {
                continue;
            };
            if let Some(requests) = &resources.requests {
                for (key, qty) in requests {
                    match key.as_str() {
                        "cpu" => add_opt(&mut cpu_request_millicores, parse_cpu_millicores(qty)),
                        "memory" => add_opt(&mut memory_request_bytes, parse_memory_bytes(qty)),
                        other => {
                            if is_accelerator_resource(other)
                                && let Some(count) = parse_count(qty) {
                                    *accelerators.entry(other.to_string()).or_insert(0) += count;
                                }
                        }
                    }
                }
            }
            if let Some(limits) = &resources.limits {
                for (key, qty) in limits {
                    match key.as_str() {
                        "cpu" => add_opt(&mut cpu_limit_millicores, parse_cpu_millicores(qty)),
                        "memory" => add_opt(&mut memory_limit_bytes, parse_memory_bytes(qty)),
                        other => {
                            // Accelerators only carry a `limits` entry (no separate request) when
                            // the workload didn't set requests explicitly; still surface them.
                            if is_accelerator_resource(other) && !accelerators.contains_key(other)
                                && let Some(count) = parse_count(qty) {
                                    *accelerators.entry(other.to_string()).or_insert(0) += count;
                                }
                        }
                    }
                }
            }
        }
    }

    PodInfo {
        name,
        namespace,
        node,
        phase,
        ready_containers,
        total_containers,
        restarts,
        start_time,
        containers,
        cpu_request_millicores,
        cpu_limit_millicores,
        memory_request_bytes,
        memory_limit_bytes,
        accelerators,
        owner,
        deployment_name,
        credential: None,
        proxy_path: None,
        access: None,
    }
}

/// Extracts the current state (running/waiting/terminated) and, if unhealthy,
/// the reason/message/exit code that explain why — the data behind "why is
/// this pod failing".
fn container_status_info(cs: &ContainerStatus) -> ContainerStatusInfo {
    let (state, reason, message, exit_code) = match cs.state.as_ref() {
        Some(s) if s.running.is_some() => ("running".to_string(), None, None, None),
        Some(s) if s.waiting.is_some() => {
            let waiting = s.waiting.as_ref().unwrap();
            ("waiting".to_string(), waiting.reason.clone(), waiting.message.clone(), None)
        }
        Some(s) if s.terminated.is_some() => {
            let terminated = s.terminated.as_ref().unwrap();
            (
                "terminated".to_string(),
                terminated.reason.clone(),
                terminated.message.clone(),
                Some(terminated.exit_code),
            )
        }
        _ => ("unknown".to_string(), None, None, None),
    };

    ContainerStatusInfo {
        name: cs.name.clone(),
        image: cs.image.clone(),
        ready: cs.ready,
        restart_count: cs.restart_count,
        state,
        reason,
        message,
        exit_code,
    }
}

fn add_opt(acc: &mut Option<i64>, value: Option<i64>) {
    if let Some(v) = value {
        *acc = Some(acc.unwrap_or(0) + v);
    }
}

/// Resource names that represent hardware accelerators (GPUs, etc.), covering the
/// common device-plugin vendors seen in the wild.
fn is_accelerator_resource(name: &str) -> bool {
    const ACCELERATOR_PREFIXES: &[&str] = &[
        "nvidia.com/",
        "amd.com/",
        "gpu.intel.com/",
        "habana.ai/",
        "aws.amazon.com/neuron",
        "google.com/tpu",
    ];
    ACCELERATOR_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Parses a CPU `Quantity` (e.g. "500m", "2", "1500u") into millicores.
pub fn parse_cpu_millicores(q: &Quantity) -> Option<i64> {
    let s = q.0.trim();
    if let Some(stripped) = s.strip_suffix('m') {
        stripped.parse::<f64>().ok().map(|v| v.round() as i64)
    } else if let Some(stripped) = s.strip_suffix('u') {
        stripped.parse::<f64>().ok().map(|v| (v / 1000.0).round() as i64)
    } else if let Some(stripped) = s.strip_suffix('n') {
        stripped.parse::<f64>().ok().map(|v| (v / 1_000_000.0).round() as i64)
    } else {
        s.parse::<f64>().ok().map(|v| (v * 1000.0).round() as i64)
    }
}

/// Parses a memory `Quantity` (e.g. "512Mi", "1Gi", "128974848") into bytes.
pub fn parse_memory_bytes(q: &Quantity) -> Option<i64> {
    let s = q.0.trim();
    const BINARY_SUFFIXES: &[(&str, f64)] = &[
        ("Ki", 1024.0),
        ("Mi", 1024.0 * 1024.0),
        ("Gi", 1024.0 * 1024.0 * 1024.0),
        ("Ti", 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Pi", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
        ("Ei", 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0 * 1024.0),
    ];
    const DECIMAL_SUFFIXES: &[(&str, f64)] = &[
        ("k", 1e3),
        ("M", 1e6),
        ("G", 1e9),
        ("T", 1e12),
        ("P", 1e15),
        ("E", 1e18),
    ];
    for (suffix, factor) in BINARY_SUFFIXES {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.parse::<f64>().ok().map(|v| (v * factor).round() as i64);
        }
    }
    for (suffix, factor) in DECIMAL_SUFFIXES {
        if let Some(stripped) = s.strip_suffix(suffix) {
            return stripped.parse::<f64>().ok().map(|v| (v * factor).round() as i64);
        }
    }
    s.parse::<f64>().ok().map(|v| v.round() as i64)
}

/// Parses an integer-ish `Quantity` used for accelerator counts (e.g. "1", "2").
pub fn parse_count(q: &Quantity) -> Option<i64> {
    let s = q.0.trim();
    s.parse::<i64>().ok().or_else(|| s.parse::<f64>().ok().map(|v| v.round() as i64))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn q(s: &str) -> Quantity {
        Quantity(s.to_string())
    }

    #[test]
    fn parses_cpu_in_every_unit_kubernetes_accepts() {
        assert_eq!(parse_cpu_millicores(&q("500m")), Some(500));
        assert_eq!(parse_cpu_millicores(&q("2")), Some(2000));
        assert_eq!(parse_cpu_millicores(&q("0.5")), Some(500));
        assert_eq!(parse_cpu_millicores(&q("1500u")), Some(2)); // micro -> milli, rounded
        assert_eq!(parse_cpu_millicores(&q("1000000n")), Some(1)); // nano -> milli
        assert_eq!(parse_cpu_millicores(&q("  250m  ")), Some(250));
    }

    #[test]
    fn rejects_cpu_it_cannot_parse() {
        assert_eq!(parse_cpu_millicores(&q("")), None);
        assert_eq!(parse_cpu_millicores(&q("abc")), None);
        assert_eq!(parse_cpu_millicores(&q("m")), None);
    }

    #[test]
    fn parses_binary_memory_suffixes() {
        assert_eq!(parse_memory_bytes(&q("1Ki")), Some(1024));
        assert_eq!(parse_memory_bytes(&q("1Mi")), Some(1024 * 1024));
        assert_eq!(parse_memory_bytes(&q("512Mi")), Some(512 * 1024 * 1024));
        assert_eq!(parse_memory_bytes(&q("1Gi")), Some(1024 * 1024 * 1024));
        assert_eq!(parse_memory_bytes(&q("2Ti")), Some(2 * 1024_i64.pow(4)));
    }

    #[test]
    fn parses_decimal_memory_suffixes_distinctly_from_binary() {
        // The classic mixup: 1M is 1e6, 1Mi is 1048576. Conflating them
        // would silently misreport every quota.
        assert_eq!(parse_memory_bytes(&q("1M")), Some(1_000_000));
        assert_eq!(parse_memory_bytes(&q("1G")), Some(1_000_000_000));
        assert_ne!(parse_memory_bytes(&q("1M")), parse_memory_bytes(&q("1Mi")));
    }

    #[test]
    fn parses_bare_byte_counts() {
        assert_eq!(parse_memory_bytes(&q("128974848")), Some(128_974_848));
        assert_eq!(parse_memory_bytes(&q("")), None);
        assert_eq!(parse_memory_bytes(&q("abc")), None);
    }

    #[test]
    fn parses_accelerator_counts() {
        assert_eq!(parse_count(&q("1")), Some(1));
        assert_eq!(parse_count(&q("8")), Some(8));
        assert_eq!(parse_count(&q("2.0")), Some(2));
        assert_eq!(parse_count(&q("")), None);
    }

    #[test]
    fn recognizes_accelerator_resources_by_vendor_prefix() {
        assert!(is_accelerator_resource("nvidia.com/gpu"));
        assert!(is_accelerator_resource("amd.com/gpu"));
        assert!(is_accelerator_resource("gpu.intel.com/i915"));
        assert!(!is_accelerator_resource("cpu"));
        assert!(!is_accelerator_resource("memory"));
        assert!(!is_accelerator_resource("ephemeral-storage"));
    }
}
