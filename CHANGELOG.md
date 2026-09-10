# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Added

- **Home directories.** A template can now set a `home_mount_path` (e.g.
  `/home/jovyan`) to give each user a persistent home directory, mounted
  independently of the existing shared-PVC storage mount. Off by default;
  the backend chooses one of three modes:
  - `hostPath` — mounts `<base path>/<username>` from a filesystem already
    shared across every node at the OS level (NFS, CephFS, a parallel
    filesystem, ...). Needs no StorageClass or CSI driver.
  - `pvc`, `strategy=dynamic` — Helve provisions one PersistentVolumeClaim
    per user from a configured, `ReadWriteMany`-capable StorageClass,
    never deleting it (matching this project's existing "never delete a
    PVC" caution).
  - `pvc`, `strategy=shared` — mounts one PVC you provision yourself into
    every user's pod with `subPath: <username>`, so Helve never creates or
    deletes a PVC at all, at the cost of one shared capacity pool rather
    than a size cap per user.
  See `charts/helve/README.md`'s "Home directories" section.

- **Per-user supplemental groups.** An admin can now assign a user any
  number of extra GIDs (`PUT /api/users/{id}/supplemental-groups`), added
  to the pod `securityContext`'s `supplementalGroups` on top of the
  existing single UID/GID. Unlike `fsGroup`, supplemental groups are a
  property of the process rather than something Kubernetes chowns onto a
  volume, so they're the actual way to grant write access on a `hostPath`
  home directory (see above) — `fsGroup` is explicitly skipped for
  `hostPath` volumes.

## [0.4.1] - 2026-09-09

### Added

- The Helve mark in the app header.

### Changed

- The app's colors follow the Helve palette instead of Aether's.
- Tagged releases also push the Helm chart to this cluster's own Harbor,
  alongside the existing GHCR publish — Harbor is what this cluster's Argo
  CD Application actually pulls the image from, so its chart should live
  there too.

## [0.4.0] - 2026-09-09

The project is renamed **Aether -> Helve**. Entries below this one still say
Aether because that is what shipped at the time; they are left alone rather
than rewritten.

### Changed

- **Everything user-facing is now Helve**: the chart is `charts/helve` and
  publishes to `oci://ghcr.io/techboredom/charts/helve`, the image is
  `ghcr.io/techboredom/helve`, and the UI, README and Helm output follow.

- **Breaking, and deliberately without compatibility shims** — this is a 0.x
  release with one known deployment, and carrying dual-read fallbacks for a
  one-time rename would leave permanent complexity behind:

  | Was | Is | Effect on upgrade |
  |---|---|---|
  | `aether_session` cookie | `helve_session` | Everyone is logged out once |
  | `aether_proxy` cookie | `helve_proxy` | Proxy sessions re-handshake |
  | `aether.io/owner` label | `helve.io/owner` | **Workloads launched by 0.3.x become invisible to their owners** |
  | `aether-theme` in localStorage | `helve-theme` | Theme choice resets to default |
  | chart name `aether` | `helve` | Helm release names are immutable; this is an uninstall and reinstall |
  | default TLS secret `aether-tls` | `helve-tls` | cert-manager issues into the new name |

  Relabel anything already running rather than losing it:

  ```console
  kubectl get deploy,svc -n <ns> -l aether.io/owner \
    -o name | xargs -I{} sh -c \
    'kubectl label {} -n <ns> helve.io/owner=$(kubectl get {} -n <ns> \
       -o jsonpath="{.metadata.labels.aether\\.io/owner}") --overwrite'
  ```

- The Forgejo secret the tag-bump step reads is `HELVE_DEPLOY_TOKEN`, not
  `AETHER_DEPLOY_TOKEN`. It has to be recreated under the new name or that
  workflow fails at the clone.

### Added

- **The app wears the Helve palette.** `frontend/tokens.css` is a verbatim
  copy of the design system's `tokens.css` and is the only place colour is
  defined; `style.css` maps those onto its own names and now contains no hex
  literals at all. Where the app needs a step the brand does not ship — a
  third surface for things that lift off a panel, and a fourth status level
  (`serious`, which the pod-phase mapping uses) — it is derived with
  `color-mix` from tokens that it does, so it re-pitches with the theme
  instead of being pinned to one mode.

  Three things the brand rules forbade came out: the violet glow was a second
  accent hue, the starfield belonged to a name meaning the upper air rather
  than an axe, and the template-notes panel was tinted with the accent, which
  means "you can act on this" and nothing else. The destructive button also
  stopped being a solid red fill — every other status in the app is drawn as
  a tint plus a border, and a filled one needed a white ink forced onto a
  deliberately desaturated red.

- **Archivo and Commit Mono, self-hosted** (`frontend/fonts/`), as the brand
  requires — neither ships with any OS. SIL OFL 1.1, licences alongside.

- `0021_rename_notes_to_helve.sql` updates the two seeded template `notes`
  that name the product in text an admin reads. Written as a new migration
  rather than an edit to 0006/0007: sqlx checksums applied migrations, so
  rewriting one that has already run makes the binary refuse to start with a
  VersionMismatch. Each `UPDATE` is scoped to the exact string those
  migrations wrote, so an admin who has edited the notes keeps their version.

## [0.3.1] - 2026-09-07

### Added

- A favicon and a real page title. The tab previously read "Pods" and had
  the browser's default document icon; it now carries the Aether mark
  (`frontend/aether-mark.svg`, copied to the dist root by trunk) and is
  titled "Aether".

- A background treatment shared with the marketing site, so the two read as
  one product: a fine graph-paper grid behind the whole app, plus drifting
  radial glows and a sparse starfield on the login screen. Driven by theme
  tokens (`--grid-line`, `--star`, `--glow-a/b`) rather than literals, so
  light mode re-pitches the treatment instead of inheriting a dimmed dark
  one — notably `--star: transparent`, since dots on a light background
  read as dust rather than space.

  The glows and starfield are deliberately scoped to the login screen. The
  tabs are dense tables and forms, where a starfield would be decoration
  sitting on top of the thing someone is trying to read; there the grid
  alone fills the gutters. The drift animation is disabled under
  `prefers-reduced-motion`.

## [0.3.0] - 2026-09-07

### Added

- An **Access** column on the Pods tab — placed first, since opening a
  running environment is the most common thing anyone does here and the
  link previously sat at the far right, past the horizontal scroll.
  "Open" now wears the accent colour rather than the same recessive
  outline as every other control. The column shows where a running
  deployment can actually be reached: proxy-enabled templates keep their
  "Open" button (moved into this column); a `LoadBalancer` Service that has
  been assigned an address also gets a "Direct" link; and every
  deployment with a Service shows its in-cluster address
  (`<svc>.<ns>.svc.cluster.local:<port>`) as selectable text. That last
  one is the point for the internal templates — Ollama, vLLM and SGLang
  are `ClusterIP` with no proxy, so previously the UI showed "—" and
  there was no way to find their address without `kubectl`. The proxy
  link can't substitute: it requires an Aether session, which a coding
  tool calling an OpenAI-compatible API doesn't have.
- Tables now show that they scroll horizontally: a permanently visible
  scrollbar (macOS hides overlay scrollbars at rest, which was exactly
  the wrong default for a table wider than its panel) plus a soft edge
  shadow that appears only while there is more content past that edge,
  and disappears at either end so it never misleads.
- The chart's `Role` gains `list` on services, which the Access column needs.
  **Existing installs must upgrade the chart, not just the image** — with
  the old Role the lookup is denied and the column falls back to showing
  no address (logged as a warning; nothing else breaks).

### Fixed

- **Proxied apps were broken on per-deployment origins.** Both proxied
  templates hardcoded the URL prefix they're served under —
  JupyterLab's `--ServerApp.base_url` and RStudio's `www-root-path`, both
  `/proxy/{{name}}/` — which was correct only while a path under Aether's
  own origin was the only way in. On a per-deployment origin the app sits
  at the root, so RStudio redirected to `/proxy/<name>/auth-sign-in` and
  then 404'd its own redirect ("The requested page was not found"), and
  JupyterLab registered its routes under a prefix no request would ever
  carry. A new always-substituted `{{proxy_root_path}}` placeholder
  resolves to `/` when per-deployment origins are configured and
  `/proxy/<name>/` when they aren't, so one template is correct under
  both; migration `0020` rewrites the two seeded templates to use it,
  leaving hand-edited ones alone.

  Deployments launched before this keep the old prefix baked into their
  args and must be relaunched.

### Changed

- The six model-serving fields (model, context length, quantization,
  served model name, GPU memory utilization, dtype) are now shown in the
  Launch and Templates forms only when the current `args` reference the
  matching `{{placeholder}}`. Previously every template displayed all six,
  so a JupyterLab or RStudio launch asked for a model it had no way to
  use — and a value entered there was collected and then silently
  discarded, since these fields only ever feed `args` substitution.
  Keyed off the args rather than a hardcoded vLLM/SGLang image list, so a
  new engine template needs no frontend change.

## [0.2.0] - 2026-09-04

### Added

- Horizontal scaling and zero-downtime rolling restarts: `replicaCount` is
  now a chart value (was hardcoded to 1), with a `RollingUpdate` strategy
  (`maxUnavailable: 0, maxSurge: 1`) so a rollout never drops below full
  capacity, even at the default of one replica.
- Graceful shutdown: the backend now drains in-flight HTTP requests on
  `SIGTERM` instead of dropping them immediately, pausing briefly first to
  give Kubernetes time to remove the terminating pod from Service
  endpoints (in place of a `preStop` hook, since the distroless runtime
  image has no shell to run one).
- A new global admin setting, `allow_custom_images` (Quotas tab, default
  `true`), restricts non-admin launches to an image already in the Images
  catalog or an existing Template's own image when turned off — enforced
  server-side (`POST /api/deployments` 400s otherwise), with the Launch
  tab hiding the "Custom" option and disabling image editing to match.
  Admins are always exempt.
- Dedicated **Model**, **context length**, **quantization**,
  **served model name**, **GPU memory utilization**, and **dtype** fields
  (Launch and Templates forms), substituted for `{{model}}`,
  `{{context_length}}`, `{{quantization}}`, `{{served_model_name}}`,
  `{{gpu_memory_utilization}}`, and `{{dtype}}` in `args` — pull the flags
  every LLM-serving template actually needs edited (vLLM's `--model`/
  `--max-model-len`/`--quantization`/`--served-model-name`/
  `--gpu-memory-utilization`/`--dtype`, SGLang's equivalents) out of the
  free-text args box. `model` works identically for a Hugging Face model
  ID or a local path under a storage mount (below); `gpu_memory_utilization`
  is validated to `(0.0, 1.0]`. All six are genuinely optional: an `args`
  line referencing one of them is dropped entirely if that field is left
  blank, instead of substituting an empty value and sending a broken
  `--flag=` with nothing after the `=`.
- A new `{{accelerator_count}}` args placeholder, so tensor parallelism
  (or any other flag that should track GPU count) matches whatever was
  actually requested instead of needing a second number kept in sync by
  hand — defaults to `1` if no accelerator was requested, and (unlike the
  six above) never dropped. vLLM/SGLang's seeded templates now use all
  seven placeholders instead of a hand-edited placeholder string.
- A storage mount for launches/templates: `volume_claim_name` +
  `volume_mount_path` (+ optional `volume_sub_path`) mount an *existing*
  `PersistentVolumeClaim` into the container — e.g. a shared model cache,
  so `model` can be a local path instead of re-downloading on every
  restart. Aether never creates or deletes a PVC itself; a new
  `GET /api/pvcs` (any logged-in user) lists what already exists in the
  namespace to back the forms' datalist, and the claim is confirmed to
  actually exist before the Deployment is created (400 immediately on a
  typo, rather than a pod stuck `Pending` with an opaque mount-failure
  event). New RBAC verbs: `persistentvolumeclaims` `get`/`list`.
- **Restart**: `POST /api/deployments/{name}/restart` bumps the pod
  template's `kubectl.kubernetes.io/restartedAt` annotation — the same
  convention `kubectl rollout restart` uses — to roll every pod over via
  the existing rolling-update strategy, without scaling to 0 and back up
  by hand.
- **Readiness probes**: a new optional `readiness_path` (Launch and
  Templates forms, requires a container port) attaches an HTTP
  `readinessProbe` to the launched container, with generous timing
  (`periodSeconds=10`, `failureThreshold=60`) to tolerate a slow-loading
  LLM server. Without one, Kubernetes considers a container Ready the
  instant its process starts — for vLLM/SGLang/Ollama that's well before
  the model has actually finished loading and the server can answer
  requests, so both the Pods tab's "Ready" status and any rolling update
  were previously lying about it. The seeded vLLM/SGLang templates default
  to `/health`, Ollama to `/`. New RBAC verb: `apps/replicasets` `get`/`list`
  (needed for rollback, below, not this).
- **Rollback**: `POST /api/deployments/{name}/rollback` reverts a
  Deployment to its previous revision — image, resources, env, and args
  exactly as they were before the last edit — the same mechanism
  `kubectl rollout undo` uses, reading the `ReplicaSet` revision history
  Kubernetes already keeps. 400s if there's no previous revision. Quota is
  re-checked the same way a normal edit is, since a rollback can just as
  easily increase resource usage as decrease it.
- **Credential regeneration**: `POST /api/deployments/{name}/regenerate-secret`
  issues a fresh value for a deployment's auto-generated credential (a
  JupyterLab token, an RStudio password, ...) without deleting and
  relaunching it — updates both `deployment_secrets` and the live
  container's env, then restarts the pod (the same mechanism as Restart,
  above) so a running pod is never left holding a credential Aether itself
  no longer knows. 400s if this deployment has no auto-generated
  credential to begin with.
- **Per-user UID/GID**: an admin can assign a UID and/or GID to a user
  account from the Users tab (`PUT /api/users/{id}/uid-gid`). Every
  Deployment that account launches afterward gets a matching pod
  `securityContext` (`runAsUser`/`runAsGroup`/`fsGroup`), so different
  users' pods can each own their own files on a shared NFS mount instead of
  everything colliding on one identity. UID and GID are independent — set
  or clear either on its own; `null`/`null` (the default) leaves the
  container image's own default untouched. `0` is rejected (that's root).
  Like node placement, fixed at launch time, not retroactive, and not
  editable post-launch.
- **Admin API tokens**: a new API Tokens admin tab lets an admin mint a
  long-lived credential for their own account (`POST /api/tokens`, body
  `{name}`), for scripting against the API without storing a real password
  or re-logging-in for a session cookie. The raw value is returned once,
  at creation, and never again — only its SHA-256 hash is stored. Send it
  as `Authorization: Bearer <token>` instead of a session cookie; it
  authenticates as whichever account created it, with that account's own
  role. `GET /api/tokens` lists an admin's own tokens (name, created/
  last-used timestamps, never the raw value); `DELETE /api/tokens/{id}`
  revokes one immediately. No expiry/rotation yet — revocation is always
  explicit, though deleting the underlying account cascades to its tokens.

### Changed

- Quota enforcement's launch-serializing lock (`AppState::lock_launches`)
  is now a Postgres advisory lock instead of an in-process `Mutex` — the
  in-process version only serialized concurrent requests within a single
  replica, which would have silently under-enforced quota the moment a
  second replica (or an old+new pod overlapping mid-rollout) existed.
- Launching no longer requires picking a name at all: `POST /api/deployments`
  dropped its `name` field entirely and now auto-generates
  `<username>-<instance type>-<random 6-char suffix>` (instance type is a
  slug of `template_name`, or of `image`'s repository component for a
  Custom launch), so nothing ever needs to be unique by anything you'd
  have to think about — not even across your own launches. Usernames are
  now validated against the same DNS-1123 grammar as a Kubernetes name
  (lowercase alphanumeric and `-` only) since a username is now also part
  of one — tighter than the previous rule, which allowed uppercase, `.`,
  and `_`.

## [0.1.1] - 2026-09-03

### Fixed

- `.github/workflows/release.yml` pushed to `ghcr.io/${{ github.repository_owner }}/...`,
  which resolves to this org's actual display name ("Techboredom") — but
  Docker/OCI repository names must be all-lowercase, so both the amd64 and
  arm64 build-and-push-by-digest jobs failed identically on `v0.1.0`.
  Hardcoded lowercase instead.
- The Forgejo-internal build pipeline (`.forgejo/workflows/build.yml`) now
  also builds multi-arch (amd64+arm64) images, via `docker buildx` +
  QEMU emulation on its single runner rather than the public pipeline's
  two native per-arch runners — build time isn't critical there the way
  it is for a tagged public release.

## [0.1.0] - 2026-09-02

First tagged release: packaged for others to run, not just this project's
own cluster.

### Added

- Apache-2.0 `LICENSE` and `NOTICE`, `SECURITY.md`, `CONTRIBUTING.md`.
- A Helm chart (`charts/aether/`) so Aether can be installed on any
  Kubernetes cluster, not just the one it was originally built for.
- A public CI/release pipeline (`.github/workflows/`) building multi-arch
  (amd64+arm64) images and packaging/publishing the Helm chart as an OCI
  artifact on tagged releases.

### Changed

- `public_service` (whether a launched deployment gets a public
  `LoadBalancer` Service) now defaults to `false`. Per-deployment proxy
  origins are the intended access path; a public LoadBalancer with no
  ingress/LB controller in front of it is the thing most likely to look
  broken on a fresh cluster. The option is unchanged otherwise.

### Fixed

- `ProxyOrigin::deployment_for_host` now matches the `Host` header
  case-insensitively.
- README corrected: login rate limiting is implemented (the "known gaps"
  section still listed it as missing, contradicting the security-notes
  section describing the throttle).

[Unreleased]: https://github.com/Techboredom/Helve/compare/v0.4.1...HEAD
[0.4.1]: https://github.com/Techboredom/Helve/releases/tag/v0.4.1
[0.4.0]: https://github.com/Techboredom/Helve/releases/tag/v0.4.0
[0.3.1]: https://github.com/Techboredom/Aether/releases/tag/v0.3.1
[0.3.0]: https://github.com/Techboredom/Aether/releases/tag/v0.3.0
[0.2.0]: https://github.com/Techboredom/Aether/releases/tag/v0.2.0
[0.1.1]: https://github.com/Techboredom/Aether/releases/tag/v0.1.1
[0.1.0]: https://github.com/Techboredom/Aether/releases/tag/v0.1.0
