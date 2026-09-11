use crate::error::ApiError;

fn bad(field: &str, reason: &str) -> ApiError {
    ApiError::BadRequest(format!("{field}: {reason}"))
}

/// A Kubernetes DNS-1123 label: lowercase alphanumeric or `-`, must start and
/// end with an alphanumeric character, max 63 chars. Required for anything
/// that becomes a Kubernetes object name (Deployment/Service/Pod name).
pub fn k8s_name(field: &str, value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > 63 {
        return Err(bad(field, "must be 1-63 characters"));
    }
    let bytes = value.as_bytes();
    let is_alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !is_alphanumeric(bytes[0]) || !is_alphanumeric(bytes[bytes.len() - 1]) {
        return Err(bad(field, "must start and end with a lowercase letter or digit"));
    }
    if !bytes.iter().all(|&b| is_alphanumeric(b) || b == b'-') {
        return Err(bad(field, "must be lowercase alphanumeric characters or '-' (a DNS label)"));
    }
    Ok(())
}

/// An optional free-text value (a `{{model}}` substitution: a Hugging Face
/// model ID or a filesystem path) — empty is valid (means "unused"), but
/// anything present is bounded and control-character-free, same spirit as
/// `label` below without the "must not be empty" rule.
pub fn optional_text(field: &str, value: &str, max_len: usize) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.chars().count() > max_len {
        return Err(bad(field, &format!("must be at most {max_len} characters")));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(bad(field, "must not contain control characters"));
    }
    Ok(())
}

/// An HTTP path for a readiness probe (e.g. `"/health"`) — must start with
/// `/`, bounded, and control-character-free. Not a full URL: no scheme,
/// host, or port, since those come from `container_port`/the pod IP instead.
pub fn http_path(field: &str, value: &str) -> Result<(), ApiError> {
    if !value.starts_with('/') {
        return Err(bad(field, "must start with '/'"));
    }
    if value.chars().count() > 200 {
        return Err(bad(field, "must be at most 200 characters"));
    }
    if value.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(bad(field, "must not contain whitespace or control characters"));
    }
    Ok(())
}

/// A fraction in `(0.0, 1.0]` — e.g. `gpu_memory_utilization`. `0.0` itself
/// is rejected (nothing meaningful reserves none of the GPU); the caller
/// only invokes this at all when the value is actually set.
pub fn fraction(field: &str, value: f64) -> Result<(), ApiError> {
    if !(value > 0.0 && value <= 1.0) {
        return Err(bad(field, "must be greater than 0 and at most 1"));
    }
    Ok(())
}

/// A volume mount referencing an existing `PersistentVolumeClaim`:
/// `claim_name`/`mount_path` must both be set or both be blank — a volume
/// with nowhere to mount it, or a mount path with no claim backing it, is
/// never meaningful. Only checks shape here; whether `claim_name` actually
/// exists in the namespace is checked against the live cluster at launch
/// time, not here.
pub fn volume_mount(claim_name: &str, mount_path: &str) -> Result<(), ApiError> {
    let claim_name = claim_name.trim();
    let mount_path = mount_path.trim();
    if claim_name.is_empty() && mount_path.is_empty() {
        return Ok(());
    }
    if claim_name.is_empty() || mount_path.is_empty() {
        return Err(bad("volume", "volume_claim_name and volume_mount_path must both be set, or both left blank"));
    }
    k8s_name("volume_claim_name", claim_name)?;
    if !mount_path.starts_with('/') {
        return Err(bad("volume_mount_path", "must be an absolute path"));
    }
    Ok(())
}

/// Where a home directory mounts inside the container — unlike
/// `volume_mount`, there's no paired claim name to validate alongside it.
/// Empty is always fine (this template doesn't use one); the backend
/// separately rejects a non-empty value if no home-drive mode is
/// configured at all (see `deployments::create_deployment`).
pub fn home_mount_path(value: &str) -> Result<(), ApiError> {
    let value = value.trim();
    if value.is_empty() {
        return Ok(());
    }
    if !value.starts_with('/') {
        return Err(bad("home_mount_path", "must be an absolute path"));
    }
    Ok(())
}

/// A display label (template name, username): just guards against empty/huge/
/// control-character input, not a strict k8s naming scheme.
pub fn label(field: &str, value: &str, max_len: usize) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(bad(field, "must not be empty"));
    }
    if trimmed.chars().count() > max_len {
        return Err(bad(field, &format!("must be at most {max_len} characters")));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(bad(field, "must not contain control characters"));
    }
    Ok(())
}

/// An image reference: non-empty, no whitespace/control characters, capped length.
/// Full OCI reference grammar isn't validated — malformed-but-well-formed-looking
/// values are still caught by the Kubernetes API server at creation time.
pub fn image_ref(value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(bad("image", "must not be empty"));
    }
    if trimmed.len() > 512 {
        return Err(bad("image", "must be at most 512 characters"));
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(bad("image", "must not contain whitespace or control characters"));
    }
    Ok(())
}

/// An admin-set `engine_slug` (e.g. `"ollama"`, `"vllm"`, `"sglang"`) —
/// lowercase alphanumeric-and-hyphen, same DNS-1123-ish grammar as
/// `k8s_name`/`username`, since it becomes a path segment under
/// `/models/<username>/<engine_slug>/...` (`backend/src/models_proxy.rs`).
pub fn slug(field: &str, value: &str) -> Result<(), ApiError> {
    if value.is_empty() || value.len() > 63 {
        return Err(bad(field, "must be 1-63 characters"));
    }
    let bytes = value.as_bytes();
    let is_alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !is_alphanumeric(bytes[0]) || !is_alphanumeric(bytes[bytes.len() - 1]) {
        return Err(bad(field, "must start and end with a lowercase letter or digit"));
    }
    if !bytes.iter().all(|&b| is_alphanumeric(b) || b == b'-') {
        return Err(bad(field, "must be lowercase alphanumeric characters or '-' only"));
    }
    Ok(())
}

pub fn container_port(port: i32) -> Result<(), ApiError> {
    if !(1..=65535).contains(&port) {
        return Err(bad("container_port", "must be between 1 and 65535"));
    }
    Ok(())
}

/// A Kubernetes resource `Quantity` string (CPU/memory), e.g. "500m", "2", "512Mi".
/// A light format check (digits, optional decimal point, optional known unit
/// suffix) — the Kubernetes API server is the authority on full validity.
pub fn quantity(field: &str, value: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Ok(());
    }
    if trimmed.len() > 20 {
        return Err(bad(field, "must be at most 20 characters"));
    }
    const SUFFIXES: &[&str] = &["Ki", "Mi", "Gi", "Ti", "Pi", "Ei", "m", "k", "M", "G", "T", "P", "E"];
    let numeric_part = SUFFIXES.iter().find_map(|s| trimmed.strip_suffix(s)).unwrap_or(trimmed);
    let valid = !numeric_part.is_empty()
        && numeric_part.chars().all(|c| c.is_ascii_digit() || c == '.')
        && numeric_part.matches('.').count() <= 1;
    if !valid {
        return Err(bad(field, "must look like a Kubernetes quantity, e.g. \"500m\", \"2\", \"512Mi\""));
    }
    Ok(())
}

/// An environment variable name: `[A-Za-z_][A-Za-z0-9_]*`, capped length.
pub fn env_key(value: &str) -> Result<(), ApiError> {
    if value.len() > 128 {
        return Err(bad("env key", "must be at most 128 characters"));
    }
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return Err(bad("env key", "must not be empty"));
    };
    if !(first.is_ascii_alphabetic() || first == '_') {
        return Err(bad("env key", "must start with a letter or underscore"));
    }
    if !chars.all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return Err(bad("env key", "must contain only letters, digits, and underscores"));
    }
    Ok(())
}

/// Caps the number and length of a list of freeform strings (env values,
/// command args) to prevent pathological input, e.g. thousands of huge args.
pub fn bounded_list(field: &str, items: &[impl AsRef<str>], max_items: usize, max_len: usize) -> Result<(), ApiError> {
    if items.len() > max_items {
        return Err(bad(field, &format!("must have at most {max_items} entries")));
    }
    if items.iter().any(|item| item.as_ref().len() > max_len) {
        return Err(bad(field, &format!("each entry must be at most {max_len} characters")));
    }
    Ok(())
}

/// Also doubles as a Kubernetes label *value* (the `helve.io/owner` label
/// tracking who launched a Deployment) and, since deployments.rs prefixes
/// every launch's name with it (`<username>-<name>`, so a name only has to
/// be unique among what *that* user has launched, not everyone's), as part
/// of a Deployment/Service *name* itself — hence the same DNS-1123 grammar
/// as validate::k8s_name (lowercase alphanumeric and '-' only, no '.'/'_',
/// no uppercase) rather than the more permissive rule this used to have.
pub fn username(value: &str) -> Result<(), ApiError> {
    if value.len() < 3 || value.len() > 32 {
        return Err(bad("username", "must be 3-32 characters"));
    }
    let bytes = value.as_bytes();
    let is_alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !is_alphanumeric(bytes[0]) || !is_alphanumeric(bytes[bytes.len() - 1]) {
        return Err(bad("username", "must start and end with a lowercase letter or digit"));
    }
    if !bytes.iter().all(|&b| is_alphanumeric(b) || b == b'-') {
        return Err(bad("username", "must be lowercase letters, digits, or '-' only"));
    }
    Ok(())
}

/// A named group's display name (Groups admin tab) — same grammar as
/// `username` (POSIX group names follow the same convention as usernames),
/// and just as load-bearing here: this ends up written verbatim into a
/// generated `/etc/group` line and a shell heredoc
/// (`deployments::identity_files_init_container`), so the restrictive
/// charset is also what keeps that safe, not just cosmetic.
pub fn group_name(value: &str) -> Result<(), ApiError> {
    if value.len() < 3 || value.len() > 32 {
        return Err(bad("name", "must be 3-32 characters"));
    }
    let bytes = value.as_bytes();
    let is_alphanumeric = |b: u8| b.is_ascii_lowercase() || b.is_ascii_digit();
    if !is_alphanumeric(bytes[0]) || !is_alphanumeric(bytes[bytes.len() - 1]) {
        return Err(bad("name", "must start and end with a lowercase letter or digit"));
    }
    if !bytes.iter().all(|&b| is_alphanumeric(b) || b == b'-') {
        return Err(bad("name", "must be lowercase letters, digits, or '-' only"));
    }
    Ok(())
}

/// A Kubernetes node label in `key=value` form, e.g. `node-type=cpu` or
/// `nvidia.com/gpu.product=H100`. The key follows the label-key grammar (an
/// optional DNS-subdomain prefix, `/`, then a DNS-1123-ish name segment); the
/// value follows the label-value grammar (same character set as the name
/// segment, may be empty). This is a practical subset of the full spec, not
/// exhaustive — the Kubernetes API server is the final authority when the
/// nodeSelector is actually applied.
pub fn node_label(value: &str) -> Result<(), ApiError> {
    let bad_label = || bad("node_label", "must look like \"key=value\", e.g. \"node-type=cpu\"");

    fn is_segment(s: &str) -> bool {
        if s.is_empty() || s.len() > 63 {
            return false;
        }
        let bytes = s.as_bytes();
        let is_alnum = |b: u8| b.is_ascii_alphanumeric();
        is_alnum(bytes[0])
            && is_alnum(bytes[bytes.len() - 1])
            && bytes.iter().all(|&b| is_alnum(b) || b == b'-' || b == b'_' || b == b'.')
    }

    let trimmed = value.trim();
    if trimmed.len() > 317 {
        return Err(bad_label());
    }
    let Some((key, val)) = trimmed.split_once('=') else {
        return Err(bad_label());
    };
    let key_ok = match key.split_once('/') {
        Some((prefix, name)) => {
            !prefix.is_empty() && prefix.len() <= 253 && prefix.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.') && is_segment(name)
        }
        None => is_segment(key),
    };
    if !key_ok || (!val.is_empty() && !is_segment(val)) {
        return Err(bad_label());
    }
    Ok(())
}

/// A UID or GID for a launched pod's `securityContext`. Rejects `0` as well
/// as negative values — `0` is root, and assigning it here would defeat the
/// whole point (per-user file ownership on shared storage), so it's almost
/// certainly a mistake rather than something an admin actually meant.
pub fn uid_gid(field: &str, value: i32) -> Result<(), ApiError> {
    if value <= 0 {
        return Err(bad(field, "must be a positive integer (0 is root)"));
    }
    Ok(())
}

/// A user's supplemental-groups assignment — `group_ids` referencing rows
/// in the named-groups registry (`groups.id`), not raw GIDs; the handler
/// checks they actually exist. Just a shape/count check here: real
/// primary-key ids are always positive, and the count cap matches
/// `uid_gid`-based lists elsewhere — nobody legitimately needs more than a
/// handful, though Kubernetes itself would tolerate far more.
pub fn group_ids(values: &[i32]) -> Result<(), ApiError> {
    if values.len() > 32 {
        return Err(bad("group_ids", "must be at most 32 groups"));
    }
    if values.iter().any(|&v| v <= 0) {
        return Err(bad("group_ids", "must all be positive"));
    }
    Ok(())
}

pub fn password(value: &str) -> Result<(), ApiError> {
    if value.len() < 8 {
        return Err(bad("password", "must be at least 8 characters"));
    }
    if value.len() > 256 {
        return Err(bad("password", "must be at most 256 characters"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_ok(result: Result<(), ApiError>) -> bool {
        result.is_ok()
    }

    #[test]
    fn accepts_valid_kubernetes_names() {
        for name in ["a", "my-app", "jupyter01", &"a".repeat(63)] {
            assert!(is_ok(k8s_name("name", name)), "should accept {name}");
        }
    }

    #[test]
    fn rejects_invalid_kubernetes_names() {
        // Uppercase and underscores are the two people actually try; the
        // path-shaped ones would otherwise reach the API server in a URL.
        for name in ["", "My-App", "my_app", "-leading", "trailing-", &"a".repeat(64), "../etc", "a/b"] {
            assert!(!is_ok(k8s_name("name", name)), "should reject {name:?}");
        }
    }

    #[test]
    fn accepts_kubernetes_quantities() {
        for value in ["500m", "2", "0.5", "512Mi", "1Gi", "128974848"] {
            assert!(is_ok(quantity("cpu", value)), "should accept {value}");
        }
    }

    #[test]
    fn rejects_malformed_quantities() {
        for value in ["abc", "1.2.3", "12x", &"1".repeat(21)] {
            assert!(!is_ok(quantity("cpu", value)), "should reject {value:?}");
        }
    }

    #[test]
    fn treats_an_empty_quantity_as_unset() {
        // Callers normalize empty to None before it reaches Kubernetes; see
        // deployments::normalize_quantity.
        assert!(is_ok(quantity("cpu", "")));
    }

    #[test]
    fn env_keys_must_look_like_shell_identifiers() {
        for key in ["PATH", "_private", "JUPYTER_TOKEN", "a1"] {
            assert!(is_ok(env_key(key)), "should accept {key}");
        }
        for key in ["", "1LEADING", "has-dash", "has space", "has=equals"] {
            assert!(!is_ok(env_key(key)), "should reject {key:?}");
        }
    }

    #[test]
    fn container_ports_must_be_in_range() {
        assert!(is_ok(container_port(1)));
        assert!(is_ok(container_port(8888)));
        assert!(is_ok(container_port(65535)));
        assert!(!is_ok(container_port(0)));
        assert!(!is_ok(container_port(-1)));
        assert!(!is_ok(container_port(65536)));
    }

    #[test]
    fn usernames_double_as_kubernetes_names_and_label_values() {
        for name in ["alice", "bob-smith", "a-b-c", "abc"] {
            assert!(is_ok(username(name)), "should accept {name}");
        }
        // Too short/long, not starting/ending alphanumeric, or containing
        // characters valid in a label value but not in a k8s *name* (dots,
        // underscores, uppercase) — since a username now also becomes part
        // of a Deployment/Service name (deployments.rs's `<username>-<name>`
        // scoping), not just the helve.io/owner label value.
        for name in ["ab", &"a".repeat(33), "-alice", "alice-", "_alice", "al ice", "al/ice", "bob.smith", "a-b_c", "Alice"] {
            assert!(!is_ok(username(name)), "should reject {name:?}");
        }
    }

    #[test]
    fn passwords_have_a_floor_and_a_ceiling() {
        assert!(!is_ok(password("short")));
        assert!(is_ok(password("longenough")));
        assert!(!is_ok(password(&"x".repeat(257))));
    }

    #[test]
    fn image_refs_reject_whitespace_and_control_characters() {
        assert!(is_ok(image_ref("nginx:alpine")));
        assert!(is_ok(image_ref("ctr.int.example.com:8443/helve/helve:v1")));
        assert!(!is_ok(image_ref("")));
        assert!(!is_ok(image_ref("nginx alpine")));
        assert!(!is_ok(image_ref("nginx\nalpine")));
    }

    #[test]
    fn optional_text_accepts_blank_but_bounds_and_checks_the_rest() {
        assert!(is_ok(optional_text("model", "", 10)));
        assert!(is_ok(optional_text("model", "meta-llama/Llama-3-8B", 500)));
        assert!(!is_ok(optional_text("model", &"x".repeat(11), 10)));
        assert!(!is_ok(optional_text("model", "bad\ntext", 500)));
    }

    #[test]
    fn http_paths_must_start_with_a_slash_and_have_no_whitespace() {
        assert!(is_ok(http_path("readiness_path", "/health")));
        assert!(is_ok(http_path("readiness_path", "/")));
        assert!(!is_ok(http_path("readiness_path", "health")));
        assert!(!is_ok(http_path("readiness_path", "/bad path")));
        assert!(!is_ok(http_path("readiness_path", "/bad\ttab")));
        assert!(!is_ok(http_path("readiness_path", &format!("/{}", "x".repeat(200)))));
    }

    #[test]
    fn fraction_must_be_greater_than_zero_and_at_most_one() {
        assert!(is_ok(fraction("gpu_memory_utilization", 0.9)));
        assert!(is_ok(fraction("gpu_memory_utilization", 1.0)));
        assert!(!is_ok(fraction("gpu_memory_utilization", 0.0)));
        assert!(!is_ok(fraction("gpu_memory_utilization", 1.1)));
        assert!(!is_ok(fraction("gpu_memory_utilization", -0.5)));
    }

    #[test]
    fn volume_mount_requires_both_fields_or_neither() {
        assert!(is_ok(volume_mount("", "")));
        assert!(is_ok(volume_mount("models", "/mnt/models")));
        assert!(!is_ok(volume_mount("models", "")));
        assert!(!is_ok(volume_mount("", "/mnt/models")));
        assert!(!is_ok(volume_mount("Bad_Name", "/mnt/models")), "claim name must be a valid k8s name");
        assert!(!is_ok(volume_mount("models", "relative/path")), "mount path must be absolute");
    }

    #[test]
    fn home_mount_path_allows_empty_and_requires_absolute() {
        assert!(is_ok(home_mount_path("")));
        assert!(is_ok(home_mount_path("/home/jovyan")));
        assert!(!is_ok(home_mount_path("relative/path")));
    }

    #[test]
    fn accepts_well_formed_node_labels() {
        for label in [
            "node-type=cpu",
            "accelerator=amd",
            "kubernetes.io/hostname=node-gpu01",
            "nvidia.com/gpu.product=H100",
            "empty-value=",
        ] {
            assert!(is_ok(node_label(label)), "should accept {label}");
        }
    }

    #[test]
    fn rejects_malformed_node_labels() {
        for label in ["nodetypecpu", "node type=cpu", "=value", "-bad=cpu", "bad-=cpu", &format!("{}=x", "a".repeat(64))] {
            assert!(!is_ok(node_label(label)), "should reject {label:?}");
        }
    }

    #[test]
    fn uid_gid_rejects_root_and_negative_but_accepts_positive() {
        assert!(is_ok(uid_gid("uid", 1000)));
        assert!(is_ok(uid_gid("uid", 1)));
        assert!(!is_ok(uid_gid("uid", 0)));
        assert!(!is_ok(uid_gid("uid", -1)));
    }

    #[test]
    fn group_ids_rejects_nonpositive_and_too_many() {
        assert!(is_ok(group_ids(&[])));
        assert!(is_ok(group_ids(&[1, 2])));
        assert!(!is_ok(group_ids(&[1, 0])));
        assert!(!is_ok(group_ids(&[-1])));
        assert!(!is_ok(group_ids(&(1..=33).collect::<Vec<i32>>())));
    }

    #[test]
    fn slug_accepts_engine_names_and_rejects_bad_ones() {
        for value in ["ollama", "vllm", "sglang", "a", &"a".repeat(63)] {
            assert!(is_ok(slug("engine_slug", value)), "should accept {value}");
        }
        for value in ["", "Ollama", "vllm_api", "-leading", "trailing-", &"a".repeat(64)] {
            assert!(!is_ok(slug("engine_slug", value)), "should reject {value:?}");
        }
    }

    #[test]
    fn group_name_matches_username_grammar() {
        assert!(is_ok(group_name("data-team")));
        assert!(is_ok(group_name("abc")));
        assert!(!is_ok(group_name("ab")), "too short");
        assert!(!is_ok(group_name("Data-Team")), "must be lowercase");
        assert!(!is_ok(group_name("-abc")), "must start alphanumeric");
    }

    #[test]
    fn bounded_list_caps_both_count_and_size() {
        let ok = vec!["a".to_string(), "b".to_string()];
        assert!(is_ok(bounded_list("env", &ok, 5, 10)));
        assert!(!is_ok(bounded_list("env", &ok, 1, 10)));
        assert!(!is_ok(bounded_list("env", &["x".repeat(11)], 5, 10)));
    }
}
