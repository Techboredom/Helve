# Helve

A web app for managing compute environments and AI engines on a Kubernetes
namespace — launch JupyterLab/RStudio environments or LLM inference engines
(Ollama, vLLM, SGLang) with a few clicks, behind a login.

It's a scoped-down slice of the broader platform described in `SPEC.md`,
covering "deploy a single-namespace workload from a template, behind a
login" — `SPEC.md`'s multi-tenancy, HPA, and ArgoCD/GitOps roadmap items
are still unaddressed. Everything it does need — an ingress controller,
TLS, a StorageClass for persistent storage — is a real requirement of the
Helm chart (see "Deploying to Kubernetes" below), not a placeholder for
something missing.

There are two account roles: **admin** and **user**. Both can use Pods and
Launch; only admins see Templates, Images, Users, Groups, Quotas, and API
Tokens.

- **Pods** — shows the running pods live, with their basic resource info:
  CPU/memory requests and limits, accelerators (GPUs, etc.), status, node,
  restarts, and age. A regular `user` only sees pods they launched
  themselves; an `admin` sees every pod in the namespace plus an **Owner**
  column showing who launched each one. A **Credential** column shows the
  auto-generated login token/API key for templates that have one,
  click-to-select for copying. The **first** column, **Access**, shows
  where a running deployment can actually be reached, in up to three
  forms: for
  proxy-enabled templates (JupyterLab, RStudio) an **Open** link that
  lands you in an already-logged-in session with zero copy-paste (see
  "Ownership, auto-generated credentials, and the reverse proxy" below);
  for a `LoadBalancer` Service that has been assigned an address, a
  **Direct** link to it; and for anything with a Service at all, its
  in-cluster address (`<service>.<namespace>.svc.cluster.local:<port>`)
  as selectable text. That last one is what the internal templates need —
  Ollama, vLLM and SGLang are `ClusterIP` with no proxy, and it's the
  address you point another pod (a coding tool calling an
  OpenAI-compatible API, say) at. The proxy link can't serve that
  purpose: it requires an Helve session, which a program doesn't have.
  A `LoadBalancer` still waiting on an address shows no Direct link
  rather than a half-formed one. Click a row to open its detail panel: per-container
  state and failure reason (`CrashLoopBackOff`, `ImagePullBackOff`, exit
  codes, etc.), recent Kubernetes Events, and a log viewer (container
  picker, tail length, previous-container logs for ones that crashed).
- **Launch** — creates a `Deployment` in that namespace, owned by whichever
  account launched it, and if a container port is given, also a Service
  exposing it — `LoadBalancer` (public, since there's no ingress) by
  default, or `ClusterIP`-only for templates where Helve's own proxy is the
  intended (and, for RStudio, only) way in (JupyterLab, RStudio). There's
  no name field to fill in — the Deployment/Service is auto-named
  `<your username>-<template name>-<random>` (or `<your username>-<image>-
  <random>` for a Custom launch), so nothing you launch ever needs to be
  unique by anything you'd have to think about. The form's
  first field is a **Template** dropdown — Ollama,
  vLLM, SGLang (Intelligence Layer) or JupyterLab, RStudio (Interface
  Layer), or **Custom** — which pre-fills image, port, resource sizing, GPU
  defaults, and any default env vars/args for that template. Every
  pre-filled field stays editable, and Custom picks any image from the
  Postgres-backed image catalog instead. Templates that carry a
  `secret_env_key` (JupyterLab, vLLM) hide that field from the form
  entirely and generate a random value for it at launch time instead —
  shown once in the success message and persistently on the Pods tab, no
  need to invent or type one yourself. JupyterLab and RStudio are also
  reachable by clicking "Open" — Helve proxies straight into an
  already-authenticated (or, for RStudio, auth-free) session.
- **Templates** *(admin only)* — CRUD for the templates the Launch tab
  offers: a table of existing templates (edit/delete) and a form to add a
  new one (same fields as a template pre-fills into Launch, plus notes shown
  when it's selected there, plus an optional "auto-generate a secret for
  this env var" field and a "proxy through Helve" checkbox — see below).
- **Images** *(admin only)* — CRUD for the container-image catalog Custom
  launches and template authoring pick from — see "Image and template
  catalogs" below.
- **Users** *(admin only)* — create accounts (username, password, role),
  delete them, reset any account's password without knowing the old one
  (forces that account to log in again everywhere, on every device), and set
  per-user node placement, UID/GID, and supplemental-group assignment. Local
  accounts are always created by hand here; LDAP/AD and OIDC (SSO) logins can
  also provision accounts automatically — see "SSO (OIDC) and LDAP/AD
  authentication" below.
- **Groups** *(admin only)* — an admin-managed registry of named GIDs,
  assigned to users from the Users tab as supplemental groups — see
  "Per-user UID/GID" below.
- **Quotas** *(admin only)* — the cluster-wide default CPU/memory/GPU limit
  and per-user overrides, plus live usage — see "User quotas" below.
- **API Tokens** *(admin only)* — mint/revoke long-lived bearer tokens for
  scripting against the API without a browser session — see "Admin API
  tokens" below.
- **Activity** — login history (when, from what IP/browser) and launch
  history (what template/image/resources someone launched), kept for
  support/metrics. Same visibility split as Pods: a `user` account sees only
  its own rows, an `admin` sees everyone's with an added Username column.

Any logged-in user (either role) can change their own password via
**Change password** in the header, which does require the current one.

- **Backend**: Rust, [Axum](https://github.com/tokio-rs/axum) +
  [kube-rs](https://kube.rs) + [sqlx](https://github.com/launchbadge/sqlx)
  (Postgres). Watches one namespace via the Kubernetes watch API and serves a
  REST snapshot plus a WebSocket that pushes live pod updates; also serves the
  image/template catalogs, authentication, and creates Deployments on request.
- **Frontend**: Rust, [Leptos](https://leptos.dev) (client-side, compiled to
  WASM with [Trunk](https://trunkrs.dev)). Loads the initial pod snapshot over
  REST, then stays in sync over the WebSocket.

## Quick start

Helve installs via Helm from the published chart:

```console
helm install helve oci://ghcr.io/techboredom/charts/helve \
  --namespace helve --create-namespace \
  --set host=helve.example.com \
  --set database.existingSecret=helve-db-app \
  --set ingress.tls.issuerRef.name=letsencrypt \
  --set adminBootstrap.password=<a-temporary-password>
```

Those four `--set` flags are the ones that matter: a hostname, a database
(or `--set database.deploy.enabled=true` for a throwaway evaluation
Postgres — see below), a cert-manager issuer for TLS, and a first-login
password. Everything else in the chart has a working default.

Just want to see it running, with no Postgres or cert-manager on hand?

```console
helm install helve oci://ghcr.io/techboredom/charts/helve \
  --namespace helve --create-namespace \
  --set host=helve.example.test \
  --set database.deploy.enabled=true \
  --set ingress.tls.enabled=false \
  --set adminBootstrap.password=<a-temporary-password>
```

Log in as `admin` with that password once the pod is `Ready`
(`kubectl get pods -n helve`), then change it — see "Change password" in
the header, both roles can. See `charts/helve/README.md` for the full
values reference, the guards the chart enforces before it will render at
all, and a worked cert-manager + Let's Encrypt example (its DNS-01
requirement, specifically — the wildcard proxy origin below means HTTP-01
can't issue a certificate for it).

The rest of this README covers what the app actually does and how its
pieces fit together, for anyone building or modifying it rather than just
running it.

## Layout

```
common/                    Shared types (PodInfo, TemplateEntry, UserInfo, CreateDeploymentRequest, ...)
backend/                   Axum server, kube-rs watcher, sqlx/Postgres catalogs + auth
backend/migrations/        sqlx migrations, auto-run on startup
backend/src/auth.rs        Password hashing, session cookie, CurrentUser/AdminUser extractors, login/logout/me
backend/src/users.rs       Users admin CRUD (admin-only)
backend/src/ldap.rs        LDAP/AD login (search-then-bind), tried by auth::login as a fallback
backend/src/oidc.rs        OIDC (SSO) login: discovery, PKCE/state/nonce, callback, account provisioning/linking
backend/src/validate.rs    Input validation (k8s names, ports, quantities, env keys, ...)
backend/src/visibility.rs  Per-user pod filtering + credential/proxy-path enrichment (admin sees all, user sees own)
backend/src/proxy.rs       Reverse proxy for proxy-enabled templates (JupyterLab) — ClusterIP connection + credential injection
backend/src/deployments.rs Create/get/update/delete a Deployment, ownership checks, launch history
backend/src/quota.rs       Global/per-user quota settings + enforcement, usage computed from the pod watcher's cache
frontend/src/login.rs      Login page
frontend/src/pods_tab.rs   Pods tab + detail panel
frontend/src/deployment_manage.rs  Scale/edit/delete panel for a Deployment, shown in the pod detail panel
frontend/src/quotas_tab.rs Quotas admin tab (global defaults + per-user overrides)
frontend/src/create_deployment_tab.rs   Launch tab (template dropdown + form)
frontend/src/templates_tab.rs   Templates admin tab (CRUD)
frontend/src/images_tab.rs      Images admin tab (CRUD, backs "Custom" mode on Launch)
frontend/src/users_tab.rs       Users admin tab (CRUD)
frontend/src/env_editor.rs      Shared add/remove env-var-row widget (Launch + Templates)
frontend/src/api.rs             Typed wrappers over gloo_net (encode, send, decode the backend's error body)
frontend/src/result_banner.rs   Shared success/error banner components every form uses
frontend/src/theme.rs           Light/dark theme toggle (data-theme attribute + localStorage)
Dockerfile                 Multi-stage build: compiles both crates, ships a distroless image
charts/helve/             Helm chart — the supported way to install this on your own cluster
.github/workflows/         Public CI (test/lint/helm-lint) and the tagged-release pipeline
SPEC.md                    The broader platform vision this app is a slice of
```

This maintainer's own deployment of Helve (which cluster, which registry,
how it's kept in sync) isn't covered in this README — see "Quick start"
above and `charts/helve/README.md` for installing your own.

## Running locally

Prerequisites:

```
rustup target add wasm32-unknown-unknown
cargo install trunk
```

You'll also need a Postgres instance for the image/template catalogs and
accounts. For local dev, any throwaway instance works — the backend creates
its tables itself on startup:

```
docker run -d -p 5432:5432 -e POSTGRES_PASSWORD=postgres postgres:16-alpine
```

Build the frontend, then run the backend (which also serves the built frontend):

```
cd frontend && trunk build --release && cd ..
cd backend
NAMESPACE=<namespace-to-watch> \
DATABASE_URL=postgres://postgres:postgres@localhost:5432/postgres \
ADMIN_BOOTSTRAP_PASSWORD=<pick-something> \
cargo run --release -- --static-dir ../frontend/dist
```

`ADMIN_BOOTSTRAP_PASSWORD` only matters the *first* time — it creates a
username `admin` account if (and only if) the `users` table is empty. Without
it on a fresh database, the app starts fine but nobody can log in (the
backend logs a warning saying so). It's ignored on every later run once a
user exists.

Open `http://localhost:3000`, log in as `admin`. Auth to the cluster
auto-detects: in-cluster service account first, falling back to your local
kubeconfig (`~/.kube/config`, current context) — the same as `kubectl`.

### Tests

`cargo test -p backend -p common` runs the unit tests. They cover the logic
that's cheap to get subtly wrong and expensive to notice: quantity parsing
(binary vs. decimal suffixes), every input-validation rule, quota accounting
(replicas x limits, per-owner grouping, exclude-self), proxy-origin host
matching, the login throttle, and the proxy's credential stripping. None of
them need a database or a cluster.

Anything that *does* need a cluster — actually launching a Deployment,
reaching a pod through the proxy — is still exercised by hand against the
real one; see "Status & known limitations".

CI runs these plus `clippy -D warnings` (backend, common, and the frontend's
wasm target) before it builds an image, so a failure there stops the build
rather than shipping. `cargo fmt` is *not* enforced — the tree isn't
currently rustfmt-clean, and reformatting it wholesale would bury real
changes in noise.

### Backend configuration

All flags can also be set as environment variables:

| Flag / env var | Default | Description |
|---|---|---|
| `--namespace` / `NAMESPACE` | *(required)* | Namespace to watch and to create Deployments in |
| `--bind-addr` / `BIND_ADDR` | `0.0.0.0:3000` | Address the HTTP server binds to |
| `--static-dir` / `STATIC_DIR` | `frontend/dist` | Directory of the built frontend to serve |
| `--database-url` / `DATABASE_URL` | *(required)* | Postgres connection string for the image/template catalogs and accounts |
| `--admin-bootstrap-password` / `ADMIN_BOOTSTRAP_PASSWORD` | *(none)* | Creates the initial `admin` account on first run only; ignored once any user exists |
| `--app-origin` / `APP_ORIGIN` | *(none)* | Public origin this app is served from, e.g. `https://helve.example.com`. Must be set together with `--proxy-base-domain` |
| `--proxy-base-domain` / `PROXY_BASE_DOMAIN` | *(none)* | Base domain for per-deployment proxy origins, e.g. `proxy.helve.example.com`. Needs wildcard DNS + TLS for `*.<domain>`. **Leaving it unset is only appropriate for local development** — see "Per-deployment proxy origins" below |
| `--ldap-url` / `LDAP_URL` | *(none)* | Turns on LDAP/AD login (with the four flags below it, all required together). e.g. `ldaps://dc1.example.com` — see "SSO (OIDC) and LDAP/AD authentication" below |
| `--ldap-bind-dn` / `LDAP_BIND_DN` | *(none)* | Service account DN Helve binds as to search the directory |
| `--ldap-bind-password` / `LDAP_BIND_PASSWORD` | *(none)* | Password for `--ldap-bind-dn` |
| `--ldap-base-dn` / `LDAP_BASE_DN` | *(none)* | Base DN to search under |
| `--ldap-user-filter` / `LDAP_USER_FILTER` | *(none)* | e.g. `(uid={username})` or `(sAMAccountName={username})` |
| `--ldap-admin-group-dn` / `LDAP_ADMIN_GROUP_DN` | *(none)* | Group DN whose membership maps to the admin role, re-checked every login |
| `--ldap-auto-provision` / `LDAP_AUTO_PROVISION` | `true` | Whether a first-time LDAP login with no matching account creates one automatically |
| `--oidc-issuer-url` / `OIDC_ISSUER_URL` | *(none)* | Turns on OIDC (SSO) login (with the two client flags below it, all required together). Requires `--app-origin` |
| `--oidc-client-id` / `OIDC_CLIENT_ID` | *(none)* | |
| `--oidc-client-secret` / `OIDC_CLIENT_SECRET` | *(none)* | |
| `--oidc-username-claim` / `OIDC_USERNAME_CLAIM` | `preferred_username` | ID token claim to read the Helve username from |
| `--oidc-groups-claim` / `OIDC_GROUPS_CLAIM` | `groups` | ID token claim carrying group membership, read as a JSON array of strings |
| `--oidc-admin-group` / `OIDC_ADMIN_GROUP` | *(none)* | Value in `--oidc-groups-claim` that maps to the admin role, re-checked every login |
| `--oidc-auto-provision` / `OIDC_AUTO_PROVISION` | `true` | Whether a first-time SSO login with no linked account creates one automatically |
| `--extra-root-ca-file` / `EXTRA_ROOT_CA_FILE` | *(none)* | PEM file of extra CA certificates to trust for this app's own outbound HTTPS calls (currently only OIDC discovery/token exchange) — needed when `OIDC_ISSUER_URL` points at a server whose certificate is signed by a private/internal CA. Purely additive; see "SSO (OIDC) and LDAP/AD authentication" below |

### Endpoints

All endpoints below except `POST /api/login` and static assets require
either a valid session cookie (401 if missing/expired) or, if no cookie is
present at all, an `Authorization: Bearer <token>` header naming a valid
API token (see "Admin API tokens" below) — the ones marked *(admin)*
additionally require the `admin` role either way (403 otherwise).

- `POST /api/login` — body `{username, password}`; sets the `helve_session` cookie and returns the logged-in `UserInfo` on success, 401 on bad credentials. If the username/password don't match a local account and LDAP is configured, tries LDAP before giving up — see "SSO (OIDC) and LDAP/AD authentication" below.
- `POST /api/logout` — clears the session (both server-side and the cookie)
- `GET /api/me` — returns the current `UserInfo` (`{id, username, role, auth_source, ...}`), or 401 if not logged in — this is what the frontend polls on load to decide whether to show the login page
- `PUT /api/me/password` — body `{current_password, new_password}`; changes your own password, 400 if `current_password` doesn't match, or if the account has no local password yet (an LDAP/OIDC-provisioned account — ask an admin to set one first). Deletes every other session for your account (`DELETE FROM sessions WHERE user_id = $1 AND token != $2`) but leaves the one making this request logged in.
- `GET /api/auth/config` — unauthenticated; `{oidc_enabled}`. The login page uses this to decide whether to show an SSO button. LDAP needs no equivalent — it's invisible, reusing the same login form.
- `GET /api/auth/oidc/login` — unauthenticated; a full-page-navigation target (not an XHR endpoint) that redirects the browser to the configured IdP. 400 if OIDC isn't configured.
- `GET /api/auth/oidc/callback?code=&state=` — unauthenticated; where the IdP redirects back. Exchanges the code, verifies the ID token, establishes a session, and redirects to `/`. An unrecognized/expired `state` redirects to `/` too rather than erroring — see "SSO (OIDC) and LDAP/AD authentication" below.
- `GET /api/users` *(admin)* — list accounts (id, username, role, auth_source, node_label, uid, gid — never password hashes)
- `POST /api/users` *(admin)* — create an account; body `{username, password, role}` (`role` is `"admin"` or `"user"`); username 3-32 chars, lowercase alphanumeric or `-` only (same grammar as a Kubernetes name — it becomes part of one, see `POST /api/deployments` below), password ≥ 8 chars
- `DELETE /api/users/{id}` *(admin)* — delete an account; an admin can't delete their own account (guards against an easy self-lockout)
- `PUT /api/users/{id}/password` *(admin)* — body `{password}`; resets another account's password without needing the old one — the admin role itself is the authorization. Deletes **all** of that account's sessions (there's no "current session" to preserve, since it isn't the admin's own).
- `PUT /api/users/{id}/node-label` *(admin)* — body `{node_label}`, a `"key=value"` string (e.g. `"node-type=cpu"`) or `null` to clear it. Every Deployment that account launches afterward gets a matching `nodeSelector`, pinning its pods to nodes carrying that label; `null` (the default for a new account) leaves placement unrestricted. Only affects future launches — see "Per-user node placement" below.
- `PUT /api/users/{id}/uid-gid` *(admin)* — body `{uid, gid}`, each either a positive integer or `null` to clear just that one (they're independent). Every Deployment that account launches afterward gets a matching pod `securityContext` (`runAsUser`/`runAsGroup`/`fsGroup`); `null`/`null` (the default for a new account) leaves the container image's own default untouched. `0` is rejected (that's root — assigning it here would defeat the point). Only affects future launches — see "Per-user UID/GID" below.
- `POST /api/tokens` *(admin)* — body `{name}` (a human label, e.g. `"CI automation"`); mints a new API token authenticating as the calling admin. Response is `{id, name, token, created_at}` — `token` is the raw value, and this is the only time it's ever returned; only its SHA-256 hash is stored. See "Admin API tokens" below.
- `GET /api/tokens` *(admin)* — lists the caller's own tokens: `{id, name, created_at, last_used_at}` — never another admin's tokens, never a raw value again.
- `DELETE /api/tokens/{id}` *(admin)* — revokes one of the caller's own tokens, effective immediately. 400 if it doesn't exist or belongs to someone else (same error either way, so this can't be used to probe another account's token ids).
- `GET /api/pods` — JSON snapshot of the current pods in the watched namespace, filtered by role: a `user` only gets pods whose `helve.io/owner` label matches their own username, an `admin` gets all of them (each with its `owner` field populated). Pods for templates with a `secret_env_key` also carry a `credential: {env_key, value}` looked up from `deployment_secrets`, and pods for proxy-enabled templates carry a `proxy_path: "/proxy/<name>/"`.
- `GET /ws` — WebSocket; sends a full snapshot on connect (same per-role filtering and credential enrichment as `GET /api/pods`), then `upsert`/`delete` events as pods change, filtered the same way per-connection
- `GET /api/images` — JSON list of catalog entries from the `images` table (id, name, image, description)
- `GET /api/templates` — JSON list of templates (any logged-in role — needed for the Launch tab's dropdown)
- `POST /api/templates` / `PUT /api/templates/{id}` *(admin)* — create/update a template. Body is a `TemplateEntry` minus `id`: `{name, image, container_port, cpu_request, cpu_limit, memory_request, memory_limit, accelerator_type, accelerator_count, env, args, model, context_length, quantization, served_model_name, gpu_memory_utilization, dtype, volume_claim_name, volume_mount_path, volume_sub_path, notes, secret_env_key, proxy_enabled, strip_prefix, public_service, readiness_path}` — only `name`/`image` are required, everything else defaults to empty/`null`/`false`/`true`. `secret_env_key`, if set, is the env var name (e.g. `JUPYTER_TOKEN`) that Launch should auto-generate instead of showing as an editable field — a proxy-enabled template doesn't need one (RStudio has none). `strip_prefix` only matters when `proxy_enabled` is set (see "the reverse proxy" above). `public_service` is independent of `proxy_enabled` — set it to `false` either for a proxied app with no auth of its own (Helve's login becomes the only way in, e.g. RStudio), or for a plain internal-only service consumed from inside the cluster (e.g. an LLM engine other in-cluster tooling talks to directly, with no browser login to bypass and no proxy involved at all). `model`/`context_length`/`quantization`/`served_model_name`/`gpu_memory_utilization`/`dtype`/`volume_claim_name`/`volume_mount_path`/`volume_sub_path` are described under `POST /api/deployments` below — the template versions are just the pre-filled defaults; empty string (or `null` for `context_length`/`gpu_memory_utilization`) means unset, same convention as `cpu_request` and friends.
- `DELETE /api/templates/{id}` *(admin)* — delete a template
- `GET /api/pvcs` — every `PersistentVolumeClaim` already existing in the watched namespace, `{name, capacity}` (`capacity` `null` if not yet Bound). Any logged-in user, same visibility as the Images catalog. Backs the Launch/Templates forms' storage-mount fields — Helve never creates or deletes a PVC itself, only mounts one that's already there (see "vLLM/SGLang: model, context length, quantization, storage, and readiness" below).
- `POST /api/deployments` — there's no `name` field: the backend generates one, `<username>-<instance type>-<6-char random suffix>`, truncating the instance-type segment as needed to stay within Kubernetes' 63-character name limit. "Instance type" is a slugified `template_name` when given, else a slug of `image`'s repository component (e.g. `nginx:alpine` → `nginx`, `jupyter/base-notebook` → `base-notebook`). The random suffix means a 409 from Kubernetes here would mean that exact suffix collided for you, which should essentially never happen. Every field below that echoes or embeds "the deployment's name" (`{{name}}` substitution, `proxy_path`, `service_name`) means this generated name. Creates a `Deployment` in the watched namespace (labeled `helve.io/owner: <your username>`), and if `container_port` is set, also a Service exposing it — `LoadBalancer` (public, external IP assigned by whatever your cluster's load-balancer implementation is) if `public_service` is true, `ClusterIP`-only otherwise. If the field is omitted this API defaults it to `true`; the Launch tab's own form, in contrast, now defaults its checkbox to off (see "Status & known limitations") — a raw API caller that omits the field still gets the old public-by-default behavior. Body: `{template_name, image, replicas, cpu_request, cpu_limit, memory_request, memory_limit, accelerator_type, accelerator_count, container_port, env, args, model, context_length, quantization, served_model_name, gpu_memory_utilization, dtype, readiness_path, volume_claim_name, volume_mount_path, volume_sub_path, generate_secret_for, enable_proxy, strip_prefix, public_service}` — everything except `image`/`replicas` is optional; `env` is `[[key, value], ...]` pairs (entries with an empty value are dropped, so an image's own default behavior — e.g. an auto-generated password logged at startup — still applies unless you set one); `args` is a list of container command-line arguments — `{{name}}`, `{{proxy_root_path}}`, and `{{accelerator_count}}` are always substituted (the last defaulting to `1` if unset). `{{proxy_root_path}}` is the URL prefix the app is served under, and exists so a template doesn't have to hardcode one: it resolves to `/` when per-deployment proxy origins are configured (the app owns a whole origin and sits at its root) and to `/proxy/<name>/` when they aren't. This is what JupyterLab's `--ServerApp.base_url` and RStudio's `www-root-path` should be set to; hardcoding `/proxy/<name>/` instead breaks the app on a per-deployment origin, and does so in a way that looks like the app itself is broken — RStudio 404s its own redirect, JupyterLab registers routes under a prefix no request carries. `{{model}}`, `{{context_length}}`, `{{quantization}}`, `{{served_model_name}}`, `{{gpu_memory_utilization}}`, and `{{dtype}}` are each substituted from the like-named field *if set* — if that field is unset, the whole `args` line containing the placeholder is dropped entirely rather than sending a broken `--flag=` with nothing after the `=`. `model` is just a plain string, whether it's a Hugging Face ID or a local path under `volume_mount_path`; `context_length` must be positive if set; `gpu_memory_utilization` must be in `(0.0, 1.0]` if set; `quantization`/`served_model_name`/`dtype` are free text. `readiness_path`, if set, attaches an HTTP `readinessProbe` to the container at that path against `container_port` (400 if `container_port` isn't also set) — see "vLLM/SGLang: model, context length, quantization, storage, and readiness" below. `volume_claim_name`, if set, mounts that existing `PersistentVolumeClaim` at `volume_mount_path` (both required together; 400 if no such claim exists), optionally scoped to `volume_sub_path` within it; `generate_secret_for`, if set to an env var name, generates a random value for it (overriding anything with that key in `env`) and stores it in `deployment_secrets`; `enable_proxy`, if `true`, requires `container_port` to be set (400 otherwise) and makes the app also reachable via `GET/POST/... /proxy/<name>/...`, with `strip_prefix` controlling how that route forwards paths (see "the reverse proxy" above); `public_service`, independent of `enable_proxy`, controls whether the Service is a public `LoadBalancer` or `ClusterIP`-only. Response adds `name` (the generated one), `service_name`/`container_port` (both `null` if no port was given), `secret_value` (the generated value, or `null`), `proxy_path` (`"/proxy/<name>/"` if `enable_proxy` was set, else `null`), and `public_service` (echoes the request, so the frontend knows whether to mention an external IP).
- `GET /api/deployments/{name}` — current editable state of a Deployment you own (or, for an admin, any Deployment): `{name, replicas, cpu_request, cpu_limit, memory_request, memory_limit, env, generated_secret_key}`. `env` excludes the auto-generated secret's entry, if any — its key is reported separately as `generated_secret_key` rather than its (regeneratable) value, since it's shown read-only rather than as an editable row. 403 if you don't own it, 404 if it doesn't exist. Backs the Pods tab's manage panel.
- `PUT /api/deployments/{name}` — scales and/or updates resources/env on a Deployment you own (or, for an admin, any Deployment). Body: `{replicas, cpu_request, cpu_limit, memory_request, memory_limit, env}`. Image, container port, accelerator, and args are fixed at launch time — changing those is a delete + relaunch, not an edit. An existing auto-generated secret's env var is carried through untouched regardless of what's submitted in `env` — edits never regenerate or require resubmitting it, since a client may already be using that value. Same validation as create (quantities, env keys, non-negative replicas). Returns the same shape as `GET`.
- `DELETE /api/deployments/{name}` — deletes a Deployment you own (or, for an admin, any Deployment), its Service if it has one, and its `deployment_secrets` row (if any) — the one place in the app that actually cleans up a generated credential rather than leaving it to outlive the deployment that used it. 403 if you don't own it.
- `POST /api/deployments/{name}/restart` — bumps `kubectl.kubernetes.io/restartedAt` on the pod template to now, the same convention `kubectl rollout restart` uses, so the Deployment's existing rolling-update strategy rolls every pod over. No body, no response body. 403 if you don't own it.
- `POST /api/deployments/{name}/rollback` — reverts the Deployment to its previous revision (image, resources, env, args — everything), read from the owning `ReplicaSet`'s revision history, same mechanism as `kubectl rollout undo`. No request body. 400 if there's no previous revision; 403 if you don't own it. Quota is re-checked the same way `PUT` is. Returns the same shape as `GET /api/deployments/{name}`.
- `POST /api/deployments/{name}/regenerate-secret` — issues a fresh value for the Deployment's auto-generated credential, updates `deployment_secrets` and the live container's env, and restarts the pod (same mechanism as `restart` above) so a running pod is never left holding a value Helve itself no longer knows. No request body; response is `{secret_value}`. 400 if this Deployment has no auto-generated credential; 403 if you don't own it.
- `ANY /proxy/{deployment_name}`, `ANY /proxy/{deployment_name}/`, `ANY /proxy/{deployment_name}/{*rest}` — reverse-proxies into a proxy-enabled deployment's pod (`backend/src/proxy.rs`), injecting its generated credential (if any) as the appropriate auth header so there's no login prompt. The first two (bare path / trailing slash, no further segment) are what every "Open" link actually points at; the wildcard one handles everything else the app itself requests once loaded. 403 if you're not that deployment's owner (or an admin); 400 if the deployment isn't proxy-enabled; 502 if the connection to its pod fails or times out (5s). Handles WebSocket upgrades transparently (needed for JupyterLab's kernel connections). See "Ownership, auto-generated credentials, and the reverse proxy" below.
- `GET /api/pods/{name}/logs?container=&tail_lines=&previous=` — plain-text container logs (`container` defaults to the pod's only container if it has one; `tail_lines` defaults to 500; `previous=true` gets the last terminated instance's logs, for a crashed container)
- `GET /api/pods/{name}/events` — JSON list of Kubernetes Events involving that pod (`type_`, `reason`, `message`, `count`, `last_seen`), most recent first — note the apiserver's default Event TTL is short (commonly ~1h), so older pods often have none left
- `GET /api/quota/me` — the caller's own effective quota (their `user_quotas` override if they have one, else the global default), current usage, `expose_resource_requests`, and `allow_custom_images`. Always unlimited limits and `allow_custom_images: true` for an admin (exempt from both enforcement mechanisms), though `expose_resource_requests` still applies to everyone. Backs the Launch tab and the Pods tab's manage panel.
- `GET /api/quota/settings` / `PUT /api/quota/settings` *(admin for PUT; GET requires only login)* — the global default quota: `{cpu_limit, memory_limit, gpu_limit, expose_resource_requests, fixed_cpu_request, fixed_memory_request, allow_custom_images}`. The limit/request fields are quantity strings (e.g. `"4"`/`"16Gi"`) or a plain integer (`gpu_limit`) — `null`/omitted means unlimited for a limit, or "leave unset" for a fixed request. `fixed_cpu_request`/`fixed_memory_request` only take effect while `expose_resource_requests` is `false` (see "User quotas" below). `allow_custom_images` defaults to `true`; set `false` to restrict non-admin launches to an image already in the Images catalog or an existing Template's own image (see "User quotas" below).
- `GET /api/quota/users` *(admin)* — every account's `{user_id, username, quota_override, used_cpu_millicores, used_memory_bytes, used_gpu_count}` — `quota_override` is `null` if that user has no override and is bound by the global default. Backs the Quotas admin tab's table.
- `PUT /api/quota/users/{id}` *(admin)* — sets (or replaces) a user's quota override, same `{cpu_limit, memory_limit, gpu_limit}` shape as the global settings' limits. `DELETE /api/quota/users/{id}` *(admin)* clears it, reverting that user to the global default.
- `GET /proxy-auth?deployment=&next=` — the app-origin half of the proxy handshake. Verifies the caller's session and that they may open `deployment`, then redirects to that deployment's own origin carrying a single-use token. 403 if you don't own it; redirects to the SPA if you aren't logged in (it's a link people follow, not an API call). Only meaningful when `PROXY_BASE_DOMAIN` is set.
- `ANY <name>.<PROXY_BASE_DOMAIN>/*` — everything on a per-deployment proxy origin is forwarded to that deployment's pod, including paths like `/api/...` that would otherwise be Helve's own. `GET /__helve/auth` on that origin is the one exception: it redeems the token above and sets the origin's own `helve_proxy` cookie.
- `GET /healthz` — liveness, no auth. Always 200 while the process is serving; deliberately checks nothing else, since a liveness probe that depended on Postgres would restart a healthy app whenever the database hiccuped.
- `GET /readyz` — readiness, no auth. 200 when Postgres answers, 503 otherwise (bounded at 2s, because an unreachable database makes the pool block rather than fail). Unlike liveness, this *should* fail — an instance that can't reach Postgres can't serve a useful request and should drop out of the Service's endpoints.
- `GET /*` — serves the built frontend (`index.html`, JS, WASM, CSS)

## Image and template catalogs (Postgres)

The `images` table (schema in `backend/migrations/0001_create_images.sql`,
applied automatically on startup via `sqlx::migrate!`) backs the image
catalog used by "Custom" mode on the Launch tab, managed from the **Images**
admin tab — a full CRUD UI for it, same pattern as Templates below. Its
schema, for reference or scripting bulk data:

```sql
CREATE TABLE images (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,        -- display name, e.g. "Ollama (ROCm)"
    image TEXT NOT NULL,       -- image ref, e.g. "ollama/ollama:rocm"
    description TEXT NOT NULL DEFAULT '',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

```sql
INSERT INTO images (name, image, description) VALUES
  ('Ollama (ROCm)', 'ollama/ollama:rocm', 'Ollama with AMD ROCm GPU support');
```

The `templates` table (schema + seed data in
`backend/migrations/0002_create_templates.sql`) backs both the **Template**
dropdown on the Launch tab and the **Templates** admin tab, which is a full
CRUD UI for it — no need to touch SQL directly unless you're restoring/
scripting data:

```sql
CREATE TABLE templates (
    id SERIAL PRIMARY KEY,
    name TEXT NOT NULL,
    image TEXT NOT NULL,
    container_port INTEGER,
    cpu_request TEXT NOT NULL DEFAULT '',
    cpu_limit TEXT NOT NULL DEFAULT '',
    memory_request TEXT NOT NULL DEFAULT '',
    memory_limit TEXT NOT NULL DEFAULT '',
    accelerator_type TEXT NOT NULL DEFAULT '',
    accelerator_count BIGINT,
    env JSONB NOT NULL DEFAULT '[]',    -- [["KEY", "default value"], ...]
    args TEXT[] NOT NULL DEFAULT '{}',  -- ["--model=...", ...]
    notes TEXT NOT NULL DEFAULT '',
    secret_env_key TEXT,                -- e.g. "JUPYTER_TOKEN"; NULL means no auto-generated secret
    proxy_enabled BOOLEAN NOT NULL DEFAULT false,   -- also reachable via Helve's /proxy/<name>/
    strip_prefix BOOLEAN NOT NULL DEFAULT false,    -- see "the reverse proxy" below
    public_service BOOLEAN NOT NULL DEFAULT true,   -- LoadBalancer (true) vs ClusterIP-only (false)
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

It ships seeded with the same five templates (Ollama, vLLM, SGLang,
JupyterLab, RStudio) that used to be hardcoded in the frontend — edit or
delete them from the Templates tab like any other row. JupyterLab
(`JUPYTER_TOKEN`) and vLLM (`VLLM_API_KEY`) are seeded with a
`secret_env_key`; Ollama, SGLang, and RStudio aren't (Ollama and SGLang have
no auth mechanism at all; RStudio runs with its own auth fully disabled —
see below). Both JupyterLab and RStudio are seeded `proxy_enabled`.

`public_service` is a separate, admin-only toggle available on every
template — including Ollama/vLLM/SGLang, which have no `proxy_enabled` at
all. Unchecking it in the Templates tab (or setting it on a per-launch
basis via the API) makes future launches of that template get a
`ClusterIP`-only Service instead of a public `LoadBalancer`: still fully
reachable from anywhere else inside the cluster (e.g. an in-cluster coding
tool calling an LLM engine's API directly), just not from outside it. This
is independent of the reverse proxy — it doesn't require (or imply) auth of
any kind, it's purely about network exposure. Ollama/vLLM/SGLang default to
`public_service = true` (external), matching their behavior before this
toggle existed; flip it per template as needed.

## vLLM/SGLang: model, context length, quantization, storage, and readiness

Launch (and the Templates admin form) has several pieces built
specifically for LLM-serving templates, though none of them are actually
vLLM/SGLang-specific — they're generic primitives any template's `args`
can use:

- **Model**, **context length**, **quantization**, **served model name**,
  **GPU memory utilization**, and **dtype** — dedicated fields, separate
  from the free-text `args` box, substituted for `{{model}}`,
  `{{context_length}}`, `{{quantization}}`, `{{served_model_name}}`,
  `{{gpu_memory_utilization}}`, and `{{dtype}}` respectively. Each field is
  shown in the Launch and Templates forms **only when the current `args`
  actually reference its placeholder**, so a JupyterLab or RStudio launch
  isn't asked for a model it has no way to use. That's keyed off the args
  rather than a hardcoded vLLM/SGLang image list, so a new engine template
  gets the right fields with no frontend change — and typing
  `--model={{model}}` into Args reveals the Model field immediately. A
  value for a placeholder the args never mention would be collected and
  then silently discarded, which is exactly what this avoids. `model`
  doesn't care whether it's a Hugging Face ID or a local filesystem path —
  it's a plain string substitution either way; what makes a path actually
  resolve inside the container is the storage mount below. `served_model_name`
  is the name exposed via the OpenAI-compatible API, kept separate from
  `model` since a Hugging Face ID is often long and not what you want API
  clients to reference. `gpu_memory_utilization` is a fraction in
  `(0.0, 1.0]` (vLLM defaults this to `0.9` — reserving almost all of a
  GPU's memory — if never set, which matters on a multi-tenant,
  quota-limited, shared-GPU cluster). All six are genuinely optional:
  **an `args` line referencing one of them is dropped entirely if that
  field is left blank**, rather than substituting an empty value and
  sending a broken `--flag=` with nothing after the `=`. That's what lets
  a template's args always list `--quantization={{quantization}}` on its
  own line and have it simply not appear for a launch that doesn't set it.
- **Tensor parallelism matches whatever was requested** — there's no
  separate "tensor parallel size" field to keep in sync by hand. `args`
  can reference `{{accelerator_count}}`, substituted with the launch's own
  `accelerator_count` (defaulting to `1` if none was requested, so a
  template whose args always reference it doesn't end up with a
  nonsensical 0 — unlike the six above, this one is never dropped).
  vLLM's full seeded args:
  `--model={{model}} --served-model-name={{served_model_name}}
  --tensor-parallel-size={{accelerator_count}} --max-model-len={{context_length}}
  --quantization={{quantization}} --gpu-memory-utilization={{gpu_memory_utilization}}
  --dtype={{dtype}}`; SGLang's: `--model-path={{model}}
  --served-model-name={{served_model_name}} --host=0.0.0.0 --port=30000
  --tp-size={{accelerator_count}} --context-length={{context_length}}
  --quantization={{quantization}} --mem-fraction-static={{gpu_memory_utilization}}
  --dtype={{dtype}}`.
- **Storage mount** — mounts an *existing* `PersistentVolumeClaim` into the
  container, e.g. a shared model cache, so `model` above can be a local
  path instead of re-downloading from Hugging Face on every restart.
  Helve never creates or deletes a PVC itself — one has to already exist
  in the namespace (provisioned out-of-band, the same way any PV/PVC pair
  is), and `GET /api/pvcs` just lists what's already there to fill a
  datalist. Set together: `volume_claim_name` + `volume_mount_path`
  (400 if no such claim exists — checked against the live cluster before
  the Deployment is created, not left to surface later as a pod stuck
  `Pending` with an opaque mount-failure event), plus an optional
  `volume_sub_path` to scope the mount to one subdirectory of the claim
  rather than its root.
- **Home directory mount** — `home_mount_path`, e.g. `/home/jovyan`,
  mounts a per-user home directory into the container, independent of
  (and mountable alongside) the storage mount above. Only takes effect if
  the backend was started with a home-drive mode configured
  (`HOME_DRIVES_HOST_BASE_PATH`, `HOME_DRIVES_STORAGE_CLASS` +
  `HOME_DRIVES_STORAGE_SIZE`, or `HOME_DRIVES_SHARED_CLAIM`; 400 if
  `home_mount_path` is set without any of them) — see
  `charts/helve/README.md`'s "Home directories" section for the three
  modes and their tradeoffs: `hostPath` needs no `ReadWriteMany` storage
  at all, at the cost of every node needing the same shared filesystem
  already mounted at the OS level; `pvc` mode either has Helve create a
  real PVC per user from a `ReadWriteMany`-capable StorageClass
  (`strategy=dynamic`), or mounts one PVC you provision yourself into
  every user's pod with `subPath: <username>` (`strategy=shared`),
  creating nothing itself.
- **Inject username/group names** — `inject_identity_files`, off by
  default, adds an init container that names the launching user's UID/GID/
  supplemental groups in the container's own `/etc/passwd`/`/etc/group`
  (appended to copies of the image's own, not replacing them), so a shell
  or `ls -l` shows names instead of bare numbers. Requires the image to
  have a POSIX shell; see "Naming UIDs/GIDs inside the container" below.
- **Readiness probe path** — `readiness_path`, e.g. `/health`, attaches an
  HTTP `readinessProbe` to the container against `container_port` (400 if
  `container_port` isn't also set), with generous timing
  (`periodSeconds=10`, `failureThreshold=60` — up to ~10 minutes) to
  tolerate how long an LLM server can take to actually load a model into
  GPU memory. Without one, Kubernetes considers the container Ready the
  instant its process starts, which for these templates is well before
  they can answer a request — the Pods tab's "Ready" status and any
  rolling update would both be lying about it. The seeded vLLM/SGLang
  templates default to `/health` (both engines expose it); Ollama's to
  `/` (any 200 response counts).

None of the storage mount above requires a StorageClass or dynamic
provisioning — a statically-bound `PersistentVolume`/`PersistentVolumeClaim`
pair (NFS-backed, or whatever your cluster already has) works the same as a
dynamically-provisioned one from Helve's perspective, since it only ever
references an existing claim by name. Provision the PVC once, by hand,
then point any number of launches at it. (The home directory mount above is
different: in `pvc` mode specifically, Helve *does* create a
PersistentVolumeClaim itself, dynamically, from whatever StorageClass the
backend was configured with — see `charts/helve/README.md`.)

## Ownership, auto-generated credentials, and the reverse proxy

Every `Deployment`/pod created via Launch is labeled `helve.io/owner:
<username>` (a Kubernetes label, kept separate from the `app: <name>`
selector label so it can't interfere with Service routing). The Pods tab
and its underlying REST/WebSocket endpoints filter on this label: a `user`
account only ever sees pods it launched itself; an `admin` sees everything,
with an extra **Owner** column.

Templates with a `secret_env_key` (JupyterLab, vLLM) don't expose that field
as editable input on the Launch form at all — instead, the backend
generates a random 48-character alphanumeric value (the same generator used
for session tokens), injects it as that env var on the container, and
stores it in a `deployment_secrets` table keyed by the Deployment's name:

```sql
CREATE TABLE deployment_secrets (
    deployment_name TEXT PRIMARY KEY,
    namespace TEXT NOT NULL,
    env_key TEXT,           -- NULL if this deployment has no generated credential at all
    secret_value TEXT,      -- NULL alongside env_key
    owner_username TEXT NOT NULL,
    proxy_enabled BOOLEAN NOT NULL DEFAULT false,
    container_port INTEGER,
    strip_prefix BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

The value is shown once in the Launch success message and persistently in
the Pods tab's Credential column (for whoever can see that pod — the same
ownership filtering applies). Re-launching under the same Deployment name
replaces the stored value. A row exists here for *any* proxy-enabled
deployment, credential or not — see RStudio below for why.

**JupyterLab and RStudio genuinely work like JupyterHub**: both templates
are `proxy_enabled`, meaning they're also reachable via Helve's own
`GET/POST/... /proxy/<name>/{*rest}` route (`backend/src/proxy.rs`, plus two
bare-path variants registered alongside it in `main.rs` — every "Open" link
points at the bare `/proxy/<name>/` with no trailing segment, which
matchit's `{*rest}` wildcard doesn't match on its own). The handler:

1. Checks you own that deployment (or are an admin) — same rule as the Pods
   tab's visibility filtering.
2. Looks up that deployment's Service and connects to its in-cluster
   `ClusterIP` directly (`kube`'s `Api<Service>::get`, RBAC already covers
   `get` on `services`) — whether that Service is itself a public
   `LoadBalancer` or `ClusterIP`-only makes no difference here, both have a
   ClusterIP. This is the conventional in-cluster design, and it assumes
   Helve itself is running **in-cluster** — a `ClusterIP` isn't routable
   from outside the cluster network, so this specific hop can't be
   exercised with the backend running locally against a remote cluster,
   unlike everything else in this app (see "Status & known limitations").
   The connection attempt times out after 5 seconds either way, so a
   stuck/unready pod fails fast (502) rather than hanging the request.
3. Injects an `Authorization` header for the credential, if the deployment
   has one — `token <value>` for `JUPYTER_TOKEN` (Jupyter Server's
   documented convention). No header at all if there's no credential
   (RStudio — see below).
4. Forwards the request path one of two ways, per the template's
   `strip_prefix`: JupyterLab (`false`) wants the full
   `/proxy/<name>/...` path forwarded as-is, since `--ServerApp.base_url`
   registers its own routes under that prefix. RStudio (`true`) is the
   *opposite* — its `www-root-path` setting only stamps that prefix onto
   redirects and cookies sent back to the browser, and it still expects
   requests to arrive at the bare path, so the proxy strips the prefix
   first. (Confirmed by hand against a real `rocker/rstudio` container:
   hitting the prefixed path 404s, while hitting the bare path with
   `www-root-path` set produces a redirect whose `Location` header — and
   whose `Set-Cookie` `Path` attributes — correctly include the prefix,
   because the proxy also forwards the original `Host` header unchanged.)
5. Transparently tunnels WebSocket upgrades too (via `hyper::upgrade` +
   `tokio::io::copy_bidirectional`), which is what makes JupyterLab's kernel
   connections (running notebook cells) work through the proxy, not just
   static pages.

JupyterLab's seeded args are `["start-notebook.sh",
"--ServerApp.base_url=/proxy/{{name}}/"]`, where `{{name}}` is a generic
placeholder substituted with the deployment's own name at launch time (any
template's `args` can use it). Kubernetes' `args` field *replaces* a
container's default command rather than appending to it, which is why the
start script has to be named explicitly — leaving it out makes the
container try (and fail) to `exec` the flag itself as a program. Its
template also sets `public_service = false` — the token-header injection
above is the only auth the proxy adds, so, same as RStudio below, the pod
gets a `ClusterIP`-only Service rather than a public `LoadBalancer`;
otherwise anyone who obtained a raw pod IP could skip Helve's proxy (and
its ownership check) and reach Jupyter directly with no token at all.

**RStudio runs with its own authentication fully disabled** (`env:
[["DISABLE_AUTH", "true"]]`) and relies entirely on Helve's login plus the
ownership check above — there's no credential to generate or inject at all,
and no login prompt to skip, because RStudio simply never asks. This is
only safe because its template also sets `public_service = false`: Launch
creates a `ClusterIP`-only Service for it instead of a public
`LoadBalancer`, so the *only* way to reach it is through Helve's own login
followed by that ownership check — nothing on the network can hit it
directly. Its `args` are a small shell wrapper (`rocker/rstudio`'s
`ENTRYPOINT` is empty, so `args` alone becomes the whole command line):
```
/bin/bash -c 'echo "www-root-path=/proxy/{{name}}/" >> /etc/rstudio/disable_auth_rserver.conf && exec /init'
```
`disable_auth_rserver.conf` is the config file the image's own init script
copies over `rserver.conf` when `DISABLE_AUTH=true` — appending to it
first, then letting `/init` run normally, is what gets `www-root-path` set
without needing a mounted config file or overriding the image's own init
logic.

vLLM is intentionally never proxied: its `VLLM_API_KEY` is meant for
scripted API clients setting their own `Authorization: Bearer <key>`
header, not a browser session — it already matches real
bearer-token-via-header usage without needing a proxy in front of it, and
forcing it through Helve's cookie-based login would only get in the way of
automation.

Each proxied HTTP request currently opens a fresh TCP connection and
HTTP/1.1 handshake to the pod rather than reusing a pooled connection —
correct and simple, but adds latency per request; pooling is a reasonable
future optimization, not a correctness issue.

## Managing running deployments

The Pods tab's detail panel (click any pod row) shows a **Manage** section
for any pod that has a `deployment_name` — i.e. anything launched through
Helve (or carrying an `app` label some other way). It lets you scale
replicas, adjust CPU/memory requests and limits, edit environment
variables, restart, roll back to the previous revision, regenerate an
auto-generated credential, and delete the Deployment (plus its Service, if
any) entirely. Image, container port, accelerator, and args are
intentionally not editable here — changing any of those is a delete +
relaunch through the Launch tab, not an in-place edit.

- **Restart** (`POST /api/deployments/{name}/restart`) bumps the pod
  template's `kubectl.kubernetes.io/restartedAt` annotation — the same
  thing `kubectl rollout restart` does — so every pod rolls over via the
  Deployment's existing rolling-update strategy, without scaling to 0 and
  back up by hand. Useful to pick up a newly-mounted model file or recover
  from a hung process.
- **Roll back** (`POST /api/deployments/{name}/rollback`) reverts the
  Deployment to its previous revision — image, resources, env, and args
  exactly as they were immediately before the last change — reading the
  `ReplicaSet` revision history Kubernetes already keeps for exactly this,
  the same mechanism `kubectl rollout undo` uses. 400s if there's no
  previous revision. Quota is re-checked the same way a normal edit is,
  since a rollback can just as easily increase resource usage as decrease
  it. Only ever undoes the single most recent change — rolling back twice
  in a row toggles between the two most recent revisions, not a longer
  history.
- **Regenerate credential** (`POST /api/deployments/{name}/regenerate-secret`,
  shown only when the Deployment has one) issues a fresh value for an
  auto-generated credential without deleting and relaunching the
  Deployment — updates `deployment_secrets` and the live container's env,
  then restarts the pod the same way Restart does, since an env var change
  only ever takes effect on a fresh pod. 400s if this Deployment has no
  auto-generated credential to begin with.

Authorization is enforced backend-side against the Deployment's own
`helve.io/owner` label (`backend/src/deployments.rs::check_owner`), not
trusted from what the frontend happens to show: an admin can manage any
Deployment, everyone else only their own. In practice a `user`-role account
never even sees a pod it doesn't own to begin with (`visibility.rs`
filters `GET /api/pods` per-role already), so the frontend doesn't
separately re-check ownership before rendering the Manage section — it
just renders whenever `deployment_name` is present, and the backend is the
actual gate for admins looking at someone else's pod.

If the Deployment has an auto-generated credential (`secret_env_key` on
its template), its env var is shown as a read-only note rather than an
editable row, and edits never regenerate or resend it — the backend always
carries the existing stored value through untouched, since resubmitting a
placeholder or omitting it entirely would otherwise silently invalidate a
value someone might already be using. Deleting a Deployment does finally
clean up its `deployment_secrets` row, though — this is the one place a
generated credential doesn't just outlive the workload it was made for
(see "Known limitations" below for the general case).

## User quotas

A single user could otherwise launch enough replicas/resources to occupy
the entire shared cluster. The **Quotas** admin tab sets a cluster-wide
default (CPU limit in cores, memory limit, GPU count — any left blank
means unlimited for that dimension) plus optional per-user overrides; an
override fully replaces the global default for that user across all three
dimensions, rather than overriding just one field at a time. Enforced in
`backend/src/quota.rs::check_quota`, called from both
`create_deployment` and `update_deployment` before either ever touches the
cluster — a launch or edit that would push the *owning user's* total over
their effective quota gets rejected with 400 and a message naming the
exceeded dimension and the numbers involved. **Admins are exempt** —
quotas exist to stop a `user` account from monopolizing shared capacity;
an admin already has unrestricted cluster access via their own kubeconfig
regardless of what Helve enforces.

Quota is checked against resource **limits**, not requests — interactive
workloads are bursty, so it's peak usage that risks starving other users,
not steady-state reservation. GPU quota is a single aggregate count
regardless of accelerator vendor/type (`nvidia.com/gpu`, `amd.com/gpu`, ...
all count toward the same limit) — simplest, and this cluster currently
only has AMD GPUs anyway.

Usage is summed from **Deployment specs** (`replicas × container limits`,
grouped by the `helve.io/owner` label), not from observed pods. Reading
pods is tempting, since the watcher already caches them, but it's wrong in
both directions: pods don't exist until a second or two after a Deployment
is created, so a burst of launches all measure a stale, empty cluster and
every one of them passes a quota they collectively blow through; and during
a rolling update the old and new pods coexist, so a legitimate launch gets
rejected against a footprint twice the real one. A Deployment's spec is
authoritative the moment it's written, and is precisely what the user is
asking to reserve. This is why the app's Role needs `list` on
`apps/deployments`.

Reading usage and then writing is still two steps, so the two are serialized
behind a single lock (`AppState::lock_launches`), held across the Kubernetes
write — otherwise two simultaneous launches would both read the pre-write
total and both be allowed. Launches are infrequent and the lock is held only
for the API call, so one global lock is plenty and much easier to reason
about than a per-user map.

A separate global toggle, **`expose_resource_requests`**, controls whether
the Launch tab and the Pods tab's manage panel show CPU/memory *request*
fields at all, independent of the quota limits themselves. This is
deliberately just a display/input setting, not a quota dimension of its
own. With it off, those fields disappear and the backend substitutes an
admin-configured **fixed request** (`fixed_cpu_request`/
`fixed_memory_request`, also set from the Quotas tab, shown only while
`expose_resource_requests` is off) for every launch and edit instead —
regardless of what limit is set. Left blank, a dimension's request is
simply never set at all, and Kubernetes' own default behavior takes over
(matching a container's request to its limit when a limit is given with
no request — Guaranteed QoS, reserving the full limit). A configured fixed
value avoids that default, letting requests stay low and predictable
(Burstable QoS) even when limits are generous. The Launch tab and manage
panel both show a note naming whatever fixed values are configured, so
users aren't left guessing why the request fields disappeared or what
they're actually getting.

Scaling or editing an existing deployment excludes *that deployment's own*
current usage from the baseline before adding its proposed new footprint,
so raising its own replica count or limits is judged only against what it
would become, not double-counted against what it already is.

A third global toggle, **`allow_custom_images`** (default `true`), governs
*what* a non-admin can launch rather than how much of it. With it off, an
image is only accepted if it already appears in the Images catalog or as
some Template's own `image` — regardless of whether the launch went
through the Template dropdown or the "Custom" option, and regardless of
which specific template (if any) `template_name` named, since the check
is really "is this image already known to Helve" rather than "does it
match the template you picked." The Launch tab hides the "Custom" option
and disables free-text editing of the Image field for non-admins while
this is off; the backend enforces it either way (`POST /api/deployments`
400s naming the image), so this is a real restriction and not just a UI
nicety. Admins are exempt, same as quota limits — the setting exists to
stop a `user` account launching arbitrary images, not to constrain
someone who already has unrestricted cluster access via their own
kubeconfig regardless of what Helve enforces.

## Per-deployment proxy origins

A proxied app runs code Helve doesn't control — JupyterLab and RStudio run
arbitrary user code by design, and `enable_proxy` can be set on any image.
Serving those apps from a path on Helve's own origin (`/proxy/<name>/`, the
original design) means their JavaScript is *same-origin* with the SPA and
`/api/*`, so it can call Helve's API with the browsing user's session cookie
attached automatically. `HttpOnly` is no defence (the JS never reads the
cookie — the browser just sends it) and neither is `SameSite=Lax` (same site).
Because an admin can open anyone's proxied app, that let a `user` account
escalate to admin simply by getting theirs opened. This is the same reason
JupyterHub ships per-user subdomains.

Setting `PROXY_BASE_DOMAIN` (plus `APP_ORIGIN`) gives every deployment its own
origin — `<name>.proxy.helve.example` — so the browser treats it as a
different site entirely. Requests to a proxy origin are dispatched by `Host`
in a middleware sitting *outside* the app's router
(`proxy::dispatch_by_host`), so they never reach `/api/*` or the SPA at all;
everything on that origin, including a path like `/api/pods`, is forwarded to
the pod. The old `/proxy/<name>/` path stops serving content and just
redirects to the new origin, so the hole closes rather than lingering beside
the fix. Host matching accepts exactly one label in front of the base domain,
matching what a wildcard TLS cert covers.

Because Helve's session cookie is host-only, it is never sent to a proxy
origin — which is the point, but means that origin needs its own way to know
who you are. Hence a small handshake, mirroring OAuth's shape:

1. A request to `<name>.proxy…` with no proxy session redirects to
   `/proxy-auth` on the **app** origin — the only host that receives the
   session cookie.
2. There, Helve verifies the session and that the caller may open this
   deployment (owner, or an admin), then mints a single-use token with a 30
   second lifetime and redirects back to the deployment's origin.
3. That origin redeems the token (deleted as it's read, so a copy left in
   history or a `Referer` is already spent), checks it was minted for *this*
   deployment, and sets its own `helve_proxy` cookie — host-only, so it
   belongs to that one subdomain and nothing else under the base domain.
4. Later requests carry that cookie. The user is re-resolved from it on every
   request rather than trusted from the cookie alone, so deleting an account
   (or replacing a deployment with a same-named one owned by someone else)
   takes effect immediately.

The `helve_proxy` cookie authorizes exactly one deployment and nothing else
in Helve, so a pod capturing its own is no more powerful than it already was.
Both it and `helve_session` are stripped from anything forwarded upstream.

Note this also makes the **admin bypass safe again**: an admin can open a
user's app for support, because that app is now cross-origin from `/api`.

**Deploying it** needs a wildcard DNS record for `*.<PROXY_BASE_DOMAIN>` and a
TLS cert covering both that wildcard and the app's own hostname, pointed at
whatever fronts Helve. `APP_ORIGIN`'s scheme decides whether the
`helve_proxy` cookie is marked `Secure`, so serve both over HTTPS.

## Per-user node placement

An admin can pin a user's workloads to a specific subset of nodes by
setting a **node label** on their account from the Users tab —
`"key=value"` (e.g. `node-type=cpu`, or `accelerator=amd` to keep someone
on the AMD GPU node), matching an actual label already on some subset of
the cluster's nodes. Every Deployment that account launches afterward gets
a `nodeSelector` with that single key/value pair
(`backend/src/deployments.rs::node_selector_for`, set on the pod template
in `create_deployment`); the Kubernetes scheduler then refuses to place
its pods anywhere else. Clearing the label (set it to `null`/empty)
returns the account to unrestricted placement for future launches.

Like the image/accelerator/args fields, the selector is fixed at launch
time from whatever the label was *then* — it isn't retroactively applied
to already-running deployments if an admin changes it later, and
`update_deployment` never touches it (only `replicas`/`resources`/`env`
are editable post-launch). This is validated server-side
(`validate::node_label`) as a practical subset of the real Kubernetes
label-key/value grammar, admin-only (`PUT /api/users/{id}/node-label`),
and otherwise invisible to the affected user — no UI surfaces it to them,
since it's a placement decision, not something they need to act on.

## Per-user UID/GID

An admin can assign a **UID and/or GID** to a user's account from the Users
tab. Every Deployment that account launches afterward runs its container
with a matching pod `securityContext` — `runAsUser`/`runAsGroup` so the
process itself runs as that identity, and `fsGroup` so files it creates on
a mounted volume come out group-owned to match
(`backend/src/deployments.rs::security_context_for`, set on the pod
template in `create_deployment`). This is what lets different users share
one NFS-backed `PersistentVolumeClaim` (see "vLLM/SGLang: model, context
length, quantization, storage, and readiness" above) without every file
ending up owned by whatever single UID the container image happens to run
as by default — assign each account its own UID/GID (matching however
ownership is actually set up on the export) and their pods only ever touch
their own files.

UID and GID are independent — set one without the other, or clear either
back to `null` (the default for a new account, meaning the container
image's own default) without touching the other. `0` is rejected
(`validate::uid_gid`) since that's root, and assigning it here would defeat
the entire point. Like node placement above, this is fixed at launch time
from whatever was set *then* — not retroactive to already-running
deployments, and not editable post-launch via `update_deployment`
(`PUT /api/users/{id}/uid-gid`, admin-only). Also like node placement, no
UI surfaces this to the affected user themselves; unlike node placement,
its effects are visible to them indirectly (file ownership on anything
they write to a shared mount), so if that ever needs to be self-service
this would be the first thing to reconsider.

An admin can also assign a user any number of **supplemental groups** —
extra GIDs added to the container's process (pod `securityContext`
`supplementalGroups`, `PUT /api/users/{id}/supplemental-groups`), on top of
the single primary GID above. This is the POSIX/NFS pattern of belonging to
several groups at once, each granting access to a different share, rather
than everything hinging on one primary GID.

Assignment is from the **Groups** admin tab's registry
(`groups`/`user_groups` tables, `backend/src/groups.rs`), not free-typed
GIDs: an admin creates a named group once (a display name plus the real
GID it means, `POST /api/groups`), and any number of users can then be
assigned to it. A GID is meant to be shared by everyone who needs access to
the same thing, so naming it once here — rather than letting each user's
assignment carry its own free-text label — is the only design where
everyone sharing that GID agrees on what it's called. Deleting a group
that's still assigned to a user is refused (`ON DELETE RESTRICT` on
`user_groups.group_id`, surfaced as a 400 rather than the generic 503
`ApiError::Sqlx` would otherwise produce), same "refuse rather than
silently change what someone's launches run as" reasoning as `delete_user`
refusing to delete a user who still owns running Deployments.

It also matters for a reason specific to this codebase: `fsGroup` above
only gets a volume's *files* to come out group-owned as that GID on volume
types that support it — `hostPath` volumes (used by home directories in
`hostPath` mode, see `charts/helve/README.md`) explicitly do **not** get
that chown-on-mount treatment
([kubernetes/kubernetes#138411](https://github.com/kubernetes/kubernetes/issues/138411)).
Supplemental groups have no such carve-out — they're a property of the
*process*, not something Kubernetes has to apply to a volume, so they work
identically no matter what's mounted. If a shared filesystem already grants
write access to files owned by some GID (an NFS export configured that
way, say), adding that GID as a supplemental group is what actually lets a
`hostPath`-mode home directory be writable — `fsGroup` alone can't do it
there.

## Naming UIDs/GIDs inside the container: `inject_identity_files`

None of the above changes what a shell or `ls -l` *inside* the launched
container shows for these UIDs/GIDs — Kubernetes has no mechanism for that
at all, since `/etc/passwd`/`/etc/group` are just files baked into the
image, with no idea what identity a particular launch was actually
assigned. A template with `inject_identity_files: true` (Templates admin
tab) closes that gap: at launch time, if the user has a UID, GID, or any
supplemental groups set at all (a no-op otherwise — nothing to name that
the image's own files don't already cover), an init container is added
that:

1. Runs the *same image* as the main container, so its copy of
   `/etc/passwd`/`/etc/group` is guaranteed format-compatible and whatever
   shell it invokes (`/bin/sh`) is the image's own — this is why the
   feature requires the image to actually have one, and is opt-in per
   template rather than automatic.
2. Copies those two files into a shared `emptyDir`, then **appends** —
   never replaces — an entry naming the launching user's UID and each
   distinct GID they're running with. Appending rather than replacing
   preserves whatever system accounts (`nobody`, a service account the
   entrypoint expects, ...) the image already ships with; a wholesale
   ConfigMap-replace (a more common version of this pattern) would destroy
   them.
3. The main container mounts that `emptyDir` over `/etc/passwd` and
   `/etc/group` via two `subPath` mounts — Kubernetes runs init containers
   to completion before starting the main one, so there's no race between
   the files being written and being read.

The init container deliberately runs as root (its own `SecurityContext`,
not the pod-level one from the UID/GID above) — it only ever touches its
own private, ephemeral volume, and this sidesteps needing `fsGroup` to
happen to be set (it isn't, if only a UID was assigned and not a GID) for
it to be able to write there at all.

Every value that ends up in the generated `/etc/passwd`/`/etc/group` lines
is either a validated username/group name (`validate::username`/
`validate::group_name` — lowercase alphanumeric and `-` only) or a plain
integer, except `home_mount_path`, which is only checked to be an absolute
path. Building the actual shell script (`deployments::identity_files_init_container`)
single-quotes every generated line via `shell_single_quote`, escaping any
embedded single quote with the standard `'\''` trick — unit-tested
directly, including against an actual injection-shaped payload run through
a real shell (not just asserted on the string transformation) — so nothing
in `home_mount_path` can break out of the generated script regardless of
its charset.

## SSO (OIDC) and LDAP/AD authentication

Local password login (`POST /api/login`, argon2-hashed, an opaque session
token in `sessions`) is always available — it's the only way into a
freshly-bootstrapped cluster, and the only path for the bootstrap `admin`
account. Two more, independent login backends can be turned on alongside
it, each tried as a fallback whenever local password login doesn't apply:

- **LDAP/Active Directory** (`LDAP_URL` and friends, or `ldap.enabled` in
  the chart) — no redirect, reuses the existing login form and
  `POST /api/login` entirely; the frontend has no idea it's happening.
  Search-then-bind: Helve binds as a service account
  (`LDAP_BIND_DN`/`LDAP_BIND_PASSWORD`) to search `LDAP_BASE_DN` with
  `LDAP_USER_FILTER` (`{username}` substituted, rejected up front if it
  contains LDAP filter metacharacters) for the submitted username's DN,
  then opens a second connection and binds as *that* DN with the supplied
  password — the actual credential check. `LDAP_URL` should be
  `ldaps://`; a plain `ldap://` URL is warned about at startup, the bind
  password and directory contents otherwise traveling in cleartext.

- **OIDC (SSO)** (`OIDC_ISSUER_URL` and friends, or `oidc.enabled` in the
  chart) — redirect-based: `GET /api/auth/oidc/login` sends the browser to
  the IdP (PKCE + CSRF state + nonce, tracked server-side in
  `oidc_flow_state` rather than in-process, since `replicaCount` can be
  >1 and the callback can land on a different pod than the one that
  issued the redirect), and `GET /api/auth/oidc/callback` exchanges the
  code, verifies the ID token, and establishes a session. The login page
  shows a "Log in with SSO" button whenever `GET /api/auth/config` (the
  one unauthenticated, unauthenticated-by-design signal the frontend
  needs) reports it's on.

Both back-ends share the same two knobs:

- **Auto-provisioning** (`LDAP_AUTO_PROVISION`/`OIDC_AUTO_PROVISION`,
  default `true`) — whether a first-time login with no matching Helve
  account creates one automatically, or is refused with a 403 asking the
  user to contact an admin. Turn it off to require accounts to be
  pre-created (the Users admin tab, any password — it's never checked
  again once the account authenticates externally).
- **Group-to-admin-role mapping**
  (`LDAP_ADMIN_GROUP_DN`/`OIDC_ADMIN_GROUP`) — when set, membership is
  re-checked on **every** login, not just the first, and the account's
  `role` is written to match. The directory/IdP becomes the source of
  truth for that account's role once this is on: a manual role change
  made from the Users tab is overwritten the next time that person logs
  in. An account with no group match, or with no mapping configured at
  all, always ends up `Role::User` — auto-provisioning never silently
  grants `Role::Admin` for lack of a signal either way.

Every account carries an `auth_source` (`local`/`ldap`/`oidc`, shown
read-only on the Users tab) and a nullable `password_hash` — an
LDAP/OIDC-provisioned account has none by default, which is what actually
keeps local login locked out for it (there's no hash to check the
submitted password against), not a separate flag. An admin can still
`PUT /api/users/{id}/password` to give such an account a local
break-glass password without changing its `auth_source`.

OIDC's one extra wrinkle, beyond what LDAP needs: accounts are matched by
the `sub` claim (`oidc_subject`), the only value the spec guarantees is
stable — never by email or username, either of which can change or be
reused. The very first time a given `sub` is seen, though, there's a
choice to make:

- With auto-provisioning **on**, an unrecognized `sub` always creates a
  brand-new account. It never links to an existing local account that
  happens to share a username — that's the account-takeover-shaped edge
  case (anyone who can get a matching username registered at the IdP
  would otherwise be able to annex an existing Helve account) this
  deliberately closes off. A genuine username collision instead fails
  with a clear 400.
- With auto-provisioning **off**, an unrecognized `sub` is allowed to
  link to an existing local account matching the configured
  `OIDC_USERNAME_CLAIM` — but *only* one that has never been linked to
  any `sub` before. This is the intended path for "an admin pre-created
  the account specifically to be claimed via SSO."

`username_claim`/`groups_claim` name arbitrary, IdP-specific claims
(default `preferred_username`/`groups`) that `openidconnect`'s typed
claims struct has no fields for, so they're read directly from the ID
token's own (already signature-verified) JSON payload rather than through
a typed accessor.

See `charts/helve/values.yaml`'s `ldap`/`oidc` blocks for the Helm-level
knobs — `LDAP_BIND_PASSWORD`/`OIDC_CLIENT_SECRET` are Secret references
(`existingSecret`/`existingSecretKey`), never inlined in `values.yaml`,
the same shape as `database.existingSecret`.

**An OIDC provider behind a private/internal CA** needs one more thing:
this app's HTTP client resolves to `rustls` + a fixed, compiled-in list
of public CAs (`webpki-roots`), not the OS trust store — it has no way to
validate a certificate signed by a CA it doesn't already know about, and
doesn't read `SSL_CERT_FILE`/`SSL_CERT_DIR` at runtime the way an
OpenSSL-based client would. Without `EXTRA_ROOT_CA_FILE` set, OIDC
discovery against such a server fails outright at startup with an
`UnknownIssuer` TLS error rather than a vague hang or a runtime 500.
`EXTRA_ROOT_CA_FILE` (or the chart's `caBundle.configMapName`/
`caBundle.configMapKey`, which mounts an existing ConfigMap and points
this env var at it) names a PEM file — one certificate or a whole bundle —
of extra CAs to trust for the backend's own outbound HTTPS calls (today,
only OIDC discovery/token exchange). Purely additive: setting it can
never make a previously-working public issuer stop validating. A
[cert-manager `trust-manager`](https://github.com/cert-manager/trust-manager)
`Bundle` resource (`useDefaultCAs: true`) is a natural source for that
ConfigMap, since its output is already a complete drop-in CA file rather
than just the one custom certificate.

## Admin API tokens

Everything in this API otherwise requires a session cookie — fine for the
browser, but it means scripting against the API (e.g. an automation that
provisions accounts) would otherwise mean storing a real admin password
somewhere and re-running `POST /api/login` yourself. The **API Tokens**
admin tab instead lets an admin mint a long-lived credential for their own
account: `POST /api/tokens` (body `{name}`, just a label to tell tokens
apart later) returns `{id, name, token, created_at}` — `token` is the raw
value, and it is shown exactly this once, right after creation. From then
on, send it as `Authorization: Bearer <token>` instead of a session
cookie; it authenticates as whichever account created it, with that
account's own role — a token minted by an admin has admin permissions,
same as that admin's own session would.

Only the token's SHA-256 hash is ever stored (`api_tokens.token_hash`,
`backend/src/auth.rs::hash_token`) — deliberately not the deliberately-slow
argon2 password hashing used for login (a 48-character random token is
already far too high-entropy to brute-force, so there's nothing to gain
from slow hashing, only needless per-request latency); the point of
hashing at all here is that a database dump alone should never be
replayable as a working credential, which matters more for these than for
`sessions.token` (stored in plaintext) since a token is explicitly meant to
live somewhere outside a browser, indefinitely, rather than expire in 7
days. Every token-authenticated request updates `last_used_at`
(`GET /api/tokens`), so an admin can tell a stale token from one still
genuinely in use before revoking it (`DELETE /api/tokens/{id}`, effective
immediately — no grace period). An admin only ever sees and manages their
own tokens, never another admin's.

If a request carries a session cookie at all, that cookie is the only
thing checked — a stale/expired cookie is never allowed to silently fall
back to a bearer token also present on the same request. That's not a
distinction real traffic should ever hit (a script authenticates with
*either* a cookie *or* a token, never both), so there was nothing to gain
from supporting it, only ambiguity to invite.

## Activity logging

Two append-only tables back the Activity tab, kept for support/metrics
purposes ("when did this user last log in, from where" and "who launched
JupyterLab with what resources") — separate from `sessions` and
`deployment_secrets`, which get deleted/overwritten and are used only for
live auth/proxy checks, not history:

```sql
CREATE TABLE session_log (
    id SERIAL PRIMARY KEY,
    user_id INTEGER NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    ip_address TEXT,
    user_agent TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE TABLE launch_log (
    id SERIAL PRIMARY KEY,
    deployment_name TEXT NOT NULL,
    namespace TEXT NOT NULL,
    owner_username TEXT NOT NULL,
    template_name TEXT,        -- NULL for a Custom launch
    image TEXT NOT NULL,
    replicas INTEGER NOT NULL,
    cpu_request TEXT, cpu_limit TEXT, memory_request TEXT, memory_limit TEXT,
    accelerator_type TEXT, accelerator_count BIGINT,
    container_port INTEGER,
    env JSONB NOT NULL DEFAULT '[]',   -- [[key, value], ...] — see redaction note below
    args TEXT[] NOT NULL DEFAULT '{}',
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
```

- `POST /api/login` records the login's source IP (`ConnectInfo`, real since
  Helve's own frontend sits directly behind its LoadBalancer with no proxy
  in front of itself — unrelated to the `/proxy/` routes above, which proxy
  *to* other apps) and `User-Agent` header into `session_log`.
- `POST /api/deployments` records the full launch request into `launch_log`
  after a successful create — **except** any env value matching
  `generate_secret_for`, which is replaced with the literal string
  `"<generated>"` before it's ever written. This matters because, unlike
  `deployment_secrets` (admin-adjacent, tied to a specific still-running
  deployment), `launch_log` is visible to the launching user themselves
  indefinitely — it must never carry a real credential.
- `GET /api/sessions` / `GET /api/launches` — same visibility rule as Pods:
  an `admin` gets every row (with a `username` field), a `user` account gets
  a query that's already server-side filtered to just their own, not a
  client-side-hidden subset of everyone's.

## Building the container image

Cross-compiling for `linux/amd64` from an arm64 machine (e.g. Apple Silicon)
via QEMU reliably crashes `rustc`, so build this on a native amd64 box for a
single-platform image matching your own machine:

```
docker build -t <registry>/helve/helve:latest .
docker push <registry>/helve/helve:latest
```

`.github/workflows/release.yml` instead produces a genuinely multi-arch
(amd64+arm64) image on every tagged release, built natively on two
per-arch runners rather than via QEMU emulation — emulated Rust
compilation is slow enough (tens of minutes) to make a release
frustrating, so it's worth the extra runner rather than one runner
emitting both platforms.

The image is a distroless (`gcr.io/distroless/cc-debian12:nonroot`) runtime
containing just the compiled backend binary and the built frontend assets —
no shell, runs as a non-root user. It's multi-arch itself, so this
Dockerfile needs no per-arch branching either way.

## Deploying to Kubernetes

Use the Helm chart in `charts/helve/`, published as an OCI artifact on
each tagged release — see "Quick start" above, and `charts/helve/README.md`
for the full values reference, the guards it enforces before rendering,
and a worked cert-manager + Let's Encrypt example. Nothing about it is
specific to any one cluster: the `ServiceAccount`/`Role`/`RoleBinding`,
`Deployment`, `Service`, `Ingress`, and `Certificate` it renders are all
driven entirely by chart values, with no cluster-specific defaults baked
in (domain, registry, node placement, all unset by default).

Postgres doesn't need a manual secret if you already run one elsewhere —
point `database.existingSecret` at a Secret holding its connection string
(any Postgres works; the chart doesn't assume CloudNativePG or anything
else specifically). `database.deploy.enabled=true` deploys a bundled,
evaluation-only single-replica Postgres instead, for trying this out
without a database of your own on hand.

Helve itself can run more than one replica (`replicaCount` in the
chart) — session cookies, proxy handoff tokens, and quota enforcement all
live in Postgres rather than in-process, so replicas don't need to agree
with each other about anything. A rolling restart is zero-downtime even
at the default of one replica: the app drains in-flight HTTP requests on
`SIGTERM` instead of dropping them, and the chart's rollout strategy
never drops below full capacity. See `charts/helve/README.md`, "High
availability", for what this doesn't cover (already-open long-lived
connections — a proxied app session, or the Pods tab's live-update
WebSocket — still end when their pod does).

## Security notes

**Authentication:**

- Passwords are hashed with argon2 (via the `argon2`/`password-hash` crates),
  never stored or logged in plaintext.
- Sessions are opaque random tokens (48 alphanumeric chars from the OS RNG)
  stored server-side in the `sessions` table, sent to the browser as an
  `HttpOnly`, `SameSite=Lax` cookie (`helve_session`) so client-side JS
  (including any XSS) can't read it, and cross-site requests can't ride on
  it. Sessions last 7 days and aren't refreshed on activity; logging out
  deletes the row server-side, not just the cookie.
- The cookie is marked `Secure` when `APP_ORIGIN` is an `https://` URL, and
  not otherwise — a `Secure` cookie is never sent back over plain HTTP, so
  setting it unconditionally would lock out a plain-HTTP deployment entirely.
  Configure `APP_ORIGIN` as soon as the app is served over TLS; until then the
  session token and the login password are only as protected as the network,
  and the backend logs a warning at startup saying so.
- `POST /api/login` is rate limited per source address: 10 failures within 5
  minutes and further attempts are refused with 429 without the password
  being checked at all. That slows guessing, and also caps an unauthenticated
  CPU drain — verifying a password runs argon2, which is expensive by design,
  so junk logins would otherwise be a cheap way to saturate the server. A
  successful login clears the address's history.
- The app-level `admin`/`user` roles gate *application* actions (Templates,
  Users, Launch) and are unrelated to Kubernetes RBAC below, which gates
  what the backend's own ServiceAccount can do to the cluster regardless of
  which human is logged in.
- Every login records its source IP and `User-Agent` into `session_log`
  (see "Activity logging" above), visible to the account it belongs to and
  to admins — a privacy/data-retention tradeoff worth knowing about if this
  is ever used somewhere IP logging needs disclosure.
- LDAP/AD login (see "SSO (OIDC) and LDAP/AD authentication" above)
  rejects a submitted username containing LDAP filter metacharacters
  (`*`, `(`, `)`, `\`, NUL) before it ever reaches the configured search
  filter, and rejects an empty password outright (some directory servers
  treat that as an anonymous bind, which "succeeds" without checking
  anything). `LDAP_URL` not being `ldaps://` is warned about at startup,
  the same way a non-`https` `APP_ORIGIN` is.
- OIDC login uses PKCE + CSRF `state` + a `nonce`, all Postgres-backed
  (`oidc_flow_state`, single-use, ~10-minute expiry) rather than
  in-process, since `replicaCount` can be >1 and the callback can land on
  a different pod than the one that issued the redirect. Accounts are
  matched by the `sub` claim only, never by email/username, and an
  auto-provisioned account is never silently linked to an existing one —
  see the README section above for the exact rule. The HTTP client used
  for discovery/token/JWKS requests never follows redirects.

**Input validation** (`backend/src/validate.rs`), applied to Launch, Templates,
and Users requests server-side (the real boundary) and mirrored as HTML5
attributes client-side for early feedback:

- Deployment/Service names must be valid Kubernetes DNS-1123 labels
  (lowercase alphanumeric + `-`, 1-63 chars) — also closes off any
  path-injection risk from a crafted pod name reaching the Kubernetes API
  through `/api/pods/{name}/logs` or `/events`.
- Container ports must be 1-65535; CPU/memory quantities get a light format
  check (still not full k8s `Quantity` grammar — malformed-but-plausible
  values are caught by the Kubernetes API server itself); env var keys must
  look like real identifiers; env values, args, template names, and image
  refs are all length-capped.
- None of this defends against a *logged-in* user launching something
  legitimately dangerous (arbitrary image, arbitrary args) — that's a
  trust decision inherent to what this app does, not something input
  validation can fix. It only rejects malformed/oversized/injection-shaped
  input.

**Kubernetes RBAC** is minimal and namespaced (no `ClusterRole`): read-only
(`get`/`list`/`watch`) on `pods`, `get` on `pods/log`, `get`/`list` on
`events`, `create`/`get`/`list`/`update`/`delete` on `apps/deployments`, and
`create`/`get`/`delete` on `services` — nothing else. It has no access to
Secrets, ConfigMaps, RBAC objects, or anything outside its own namespace.
The Deployment verbs beyond `create`/`get` are what the Pods tab's manage
panel needs (`update` to scale/edit, `delete` to remove) plus `list` for
quota accounting (see "User quotas"). The reverse proxy needs no RBAC beyond
`get` on `services` (already listed) — it never asks the Kubernetes API to
reach an app, just to look up a Service's ClusterIP, then connects to that
IP directly over plain TCP like any other in-cluster client would.

**Other:**

- Pod logs can contain sensitive application output; any logged-in user
  (either role) can read the logs of anything running in the watched
  namespace, and (via Launch) can create a Service with a public-facing
  LoadBalancer IP — there's no admission control over what gets exposed.
- **Home directories in `hostPath` mode grant a launched pod access to that
  path on whichever node it lands on**, not just the one subdirectory
  Kubernetes' own volume isolation implies — a `hostPath` volume is a real
  node-filesystem access grant, scoped by convention (the path Helve
  constructs) rather than by anything Kubernetes enforces. It also doesn't
  benefit from a per-user UID/GID (Users tab) the way most volumes do:
  Kubernetes' `fsGroup` mechanism, which is what normally grants a
  non-root container write access to a volume it doesn't already own,
  explicitly does not apply to `hostPath` volumes — file ownership and
  permissions there are entirely the shared filesystem's own concern (its
  export config, UID mapping, etc.); Helve does not `chown` anything
  itself. Supplemental groups (also Users tab) are the actual fix here,
  not `fsGroup` — see "Per-user UID/GID" above and `charts/helve/README.md`'s
  "Home directories" section before enabling `homeDrives.mode=hostPath`.
- Templates (Ollama/vLLM/SGLang/JupyterLab/RStudio) are unauthenticated *at
  the app they launch* by default, unrelated to logging into Helve itself.
  Ollama and SGLang have no auto-generated credential (see "Ownership, auto-generated
  credentials, and the reverse proxy" above) — set your own token/password via the
  env var editor if the image supports one, otherwise it's either unauthenticated
  or gets a random value visible only in the pod's own logs. RStudio runs
  with its own auth *deliberately* disabled and no public Service at all —
  Helve's login is the only gate. JupyterLab and vLLM get an auto-generated
  credential instead, stored **in plaintext** in the `deployment_secrets`
  table (no encryption at rest) and visible to the owning user and any admin
  via the Pods tab.
- Pod ownership (`helve.io/owner` label) and the Pods-tab visibility
  filtering it drives are enforced entirely in the Helve backend at read
  time, not via Kubernetes RBAC or admission control — the label itself is
  just metadata anyone with direct `kubectl` access to the namespace can see
  or edit. It restricts what Helve's UI/API surface shows a `user` account,
  not what's actually running in the cluster.
- **The reverse proxy strips Helve's own credentials before forwarding.** A
  proxied pod runs code Helve doesn't control — JupyterLab and RStudio run
  arbitrary user code by design, and `enable_proxy` can be set on any image —
  so `backend/src/proxy.rs::forwarded_headers` removes the caller's
  `helve_session` cookie and their `Authorization` header on the way in, and
  drops any upstream `Set-Cookie` that would overwrite `helve_session` on the
  way back out (which would otherwise let a hostile pod pin the caller's
  browser to a session of its choosing). Every *other* cookie is forwarded
  untouched, because proxied apps set and depend on their own (RStudio's
  session, JupyterLab's XSRF token). Unit-tested in that module.
- **Each proxied deployment is served from its own origin** when
  `PROXY_BASE_DOMAIN` is set — see "Per-deployment proxy origins" below. This
  is what stops a proxied app's JavaScript from calling Helve's own API as
  whoever is browsing it. **With it unset, that hole is open**: `/proxy/<name>/`
  then shares an origin with the SPA and `/api/*`, so a pod's JS can call the
  API with the browsing user's cookie attached automatically (`HttpOnly`
  doesn't help — the JS never reads the cookie, the browser just sends it; nor
  does `SameSite=Lax` — it's the same site), and since an admin can open
  anyone's proxied app, a user could escalate to admin by getting one opened.
  Configure it for any shared deployment; leaving it unset is a local-dev
  convenience only, and the backend logs a warning at startup when it is.
- The container runs as a non-root user with a read-only root filesystem and
  all Linux capabilities dropped.
- **No CORS layer at all.** The frontend is served by this same process, so
  every call it makes is same-origin, and `trunk serve` proxies to the backend
  server-side during development, which CORS never sees either. (A permissive
  policy used to be set here for dev convenience; it only widened what other
  sites could attempt with a logged-in browser.) CSRF protection comes from
  the cookie's `SameSite=Lax`.
- **Internal error detail stays in the logs.** Database and cluster-transport
  failures are logged server-side and answered with a generic message —
  a `sqlx` error quotes SQL and column names straight back at whoever
  triggered it. Kubernetes *API* errors are still passed through, since those
  are the API server's own validation messages ("already exists", "must be no
  more than ..."), which are the most useful thing to show and reveal nothing
  the caller couldn't learn by asking it directly.
- **Expired credentials are pruned hourly** (`sessions`, `proxy_auth_tokens`,
  `proxy_sessions`) — nothing reads them once past `expires_at`, so this only
  stops the tables growing without bound. The `session_log` and `launch_log`
  audit tables are deliberately left alone: how long to keep records of who
  logged in from where is a retention decision, not something to discard
  silently.

## Status & known limitations

Everything above is implemented and has been exercised against the real
cluster (not just locally built) — including the failure-diagnosis path
against a pod that had genuinely been `Failed` for two weeks, the Launch
tab's port/env/args/Service wiring verified with real throwaway Deployments
(one confirmed via its own container logs: args were passed through to the
container and it ran and printed them), full template CRUD (create, edit,
delete, and launching from a DB-backed template) exercised against a
throwaway Postgres, and the full auth flow: bootstrap admin creation,
login/logout, a real `user`-role account confirmed able to reach Pods/Launch
but getting 403s from the Templates/Users write endpoints, and each
validation rule in `backend/src/validate.rs` confirmed to actually reject
its bad input (bad k8s name, out-of-range port, malformed quantity, bad env
key, path-injection-shaped pod name, weak password) via curl. The reverse
proxy was verified against a real `jupyter/base-notebook` pod launched by a
`user`-role account: `/proxy/<name>/` (the bare path every "Open" link
actually uses) served JupyterLab with the token already applied (no login
prompt), a second non-owning user got 403 on that exact path while an admin
could still open it, and — the part most likely to silently break — a real
notebook cell was executed through the proxied WebSocket kernel connection
and returned the correct output, confirming the upgrade-tunneling code path
actually works and isn't just serving static pages. (That "bare path"
qualifier matters: an earlier verification pass only ever tested
`/proxy/<name>/lab`, which masked a real bug where the bare path — with no
segment after the trailing slash — didn't match the route at all and fell
through to the frontend's own SPA fallback; fixed by adding two explicit
routes for it, see `backend/src/proxy.rs::handler_root`.) A parallel
`rocker/rstudio` pod, launched with `DISABLE_AUTH=true` and `public_service:
false`, was confirmed to get a `ClusterIP`-only Service (no external IP at
all) and to correctly reject a non-owner on that same bare path. The
Activity tab's login/launch history was verified the same way: two real
accounts confirmed each only sees their own rows while an admin sees both,
and a launched deployment's `generate_secret_for` value confirmed redacted
to `"<generated>"` in `launch_log` rather than storing the real credential.
The Images admin tab was verified the same way as Templates: full CRUD via
curl (including a `user`-role account confirmed to read the list but get
403 on create), plus a Puppeteer pass creating an entry through the actual
form. The theme toggle was confirmed via Puppeteer: default is dark,
toggling flips `data-theme` and the rendered background color to the
validated light-mode step, the choice persists in `localStorage` across a
reload, and both states were screenshotted to check for layout/contrast
issues. Deployment lifecycle management (scale/edit/delete from the Pods
tab, see "Managing running deployments" above) was verified against the
real cluster: ownership enforcement in every direction (owner, a
non-owning `user`, and admin, tested against `GET`/`PUT`/`DELETE` all
three), a scale+resource+env edit confirmed via `kubectl` to have actually
changed the live Deployment, an auto-generated secret's env var confirmed
byte-for-byte unchanged after an edit that didn't mention it, delete
confirmed to remove the Deployment, its Service, and its
`deployment_secrets` row, and a full Puppeteer pass as a `user`-role
account editing and then deleting a real deployment through the actual
Pods tab UI (including handling the native `confirm()` dialog). User
quotas were verified end-to-end against the real cluster: CPU, memory, and
GPU rejections all confirmed with the exact math checked (e.g. a launch
correctly rejected once existing usage plus its own footprint exceeded the
limit, with the error message's numbers matching by hand), a per-user
override confirmed to both raise a limit and, once cleared, correctly
revert that user to the global default, admin exemption confirmed by
launching wildly over-limit resources as admin with no rejection, the
scale/edit path's exclude-self accounting confirmed correct via exact
arithmetic on a real scale-up attempt, and a full Puppeteer pass covering
the Launch tab's quota summary display, the request-fields toggle actually
hiding/showing the right inputs after being flipped from the Quotas admin
tab, and the per-user usage table rendering real numbers. Fixed requests
were verified via `kubectl`: a deployment launched with a 1-core limit and
no request fields sent came back with its actual request pinned to the
configured fixed value (not defaulted up to match the limit), an edit
raising that same deployment's limit left its request untouched at the
fixed value, and re-enabling `expose_resource_requests` and launching with
an explicit request confirmed that value was honored normally again (no
fixed-request interference once the toggle is back on). Per-user node
placement was verified against the real cluster: setting a test account's
node label to `node-type=cpu` (a label that actually exists on this
cluster's worker nodes) and launching as that account produced a
Deployment whose `spec.template.spec.nodeSelector` was exactly
`{"node-type":"cpu"}` via `kubectl`, with its pod scheduled onto a matching
node; clearing the label and relaunching produced no `nodeSelector` at all;
malformed label values (no `=`, illegal characters) were rejected with 400;
a non-admin account got 403 attempting to set it; and a Puppeteer pass
confirmed the Users tab's new "Node label" column and edit form render and
save correctly. Restart, rollback, readiness probes, and credential
regeneration were all verified against the real cluster: a readiness-probe
launch (`nginx:alpine`, `readiness_path: "/"`) confirmed via `kubectl` to
carry the exact `readinessProbe` (`httpGet`, `failureThreshold: 60`,
`periodSeconds: 10`) and to actually report `ready: false` until nginx
itself started answering; restart confirmed to produce a genuinely new pod
(different name and `startTime`) with the `restartedAt` annotation set;
rollback confirmed by editing a live deployment's CPU/memory limits via
`PUT`, then rolling back and checking the response matched the pre-edit
values exactly, with the readiness probe carried through unchanged, and a
second rollback confirmed to toggle back to the edited values (the same
"undo the last change" semantics as `kubectl rollout undo`); a deployment
with no previous revision confirmed to 400 rather than silently no-op; and
credential regeneration (JupyterLab launch, `JUPYTER_TOKEN`) confirmed to
return a new value different from the one issued at launch, with `kubectl`
confirming the live pod's env var actually held the new value and the pod
had genuinely restarted (`creationTimestamp` of the Deployment vs.
`restartedAt` annotation a few seconds later). Per-user UID/GID was
verified against the real cluster the same way as node placement: `0` and
negative values both rejected with 400 (`uid: must be a positive integer
(0 is root)`), a valid `{uid: 1500, gid: 2000}` set on a test account
confirmed via `kubectl` to produce a Deployment whose
`spec.template.spec.securityContext` was exactly `{runAsUser: 1500,
runAsGroup: 2000, fsGroup: 2000}`, and — going one level past the spec
itself — the *running* pod's own kubelet-reported container status
(`status.containerStatuses[0].user.linux`) confirmed `{uid: 1500, gid:
2000}`, i.e. the container's actual process identity, not just what was
requested. Admin API tokens were verified end-to-end: a token minted via
`POST /api/tokens` with an admin's session cookie was confirmed, via
`psql`, to leave only a SHA-256 hash in `api_tokens.token_hash` — never the
raw value; that same raw value, sent as `Authorization: Bearer <token>`
with **no cookie at all**, successfully authenticated `GET /api/me` and
then created a real user via `POST /api/users` (the exact original use
case); an invalid/bogus bearer token was rejected with 401; a `user`-role
account was rejected with 403 attempting `POST /api/tokens`; `GET
/api/tokens` was confirmed to show `last_used_at` freshly updated after
those calls; and revoking the token (`DELETE /api/tokens/{id}`) was
confirmed to immediately invalidate it (the same bearer token then got 401
on its very next request), with a second revoke attempt on the same
already-gone id cleanly 400ing instead of 500ing. Known gaps, in case they
matter for what you do next:

- **`public_service`'s default is inconsistent between the API and the UI.**
  The Launch tab's own form now defaults its checkbox to off (a public
  `LoadBalancer` with no ingress/LB controller in front of it is the thing
  most likely to look broken on a fresh cluster), but `POST /api/deployments`
  still defaults the field itself to `true` when it's omitted from the
  request body entirely — a script or other direct API caller that doesn't
  send `public_service` explicitly still gets the old public-by-default
  behavior. Always send it explicitly if you're calling the API directly.
- **API tokens never expire on their own** — there's no `expires_at`, no
  automatic rotation, and no "last used more than N days ago, auto-revoke"
  sweep. `last_used_at` gives an admin the information needed to decide a
  token is stale, but revoking it is always a manual, explicit action.
  Deleting the underlying user account does cascade-delete its tokens
  (`ON DELETE CASCADE`), so there's no way for a token to outlive the
  account that created it, at least.
- **Still no "forgot password" self-service flow** — that requires emailing
  a reset link, which this app has no mechanism for (no SMTP config, no
  email field on accounts). An admin can reset a locked-out user's password
  from the Users tab instead (see below), which covers the "I forgot it"
  case even without a self-service link.
- **Quotas aren't retroactive.** Lowering a user's limit (or the global
  default) below what they're already running doesn't touch existing
  deployments — enforcement only ever blocks a *new* launch or edit from
  pushing usage over the limit, never reaches back to shrink or kill
  something already running.
- **Quota usage only counts pods Helve can attribute to an owner.** A
  Deployment created some other way (raw `kubectl apply`, no
  `helve.io/owner` label) doesn't count against anyone's usage and can't
  be blocked by this mechanism at all — quotas only govern what's launched
  through Helve itself.
- **This is a scoped-down slice of `SPEC.md`, not the whole thing.**
  `SPEC.md`'s multi-tenancy, HPA, and ArgoCD/GitOps roadmap items are
  entirely unaddressed; this only covers "deploy a single-namespace
  workload from a template." A launched app has no persistent storage by
  default and loses its state on pod restart, unless its template sets a
  `volume_claim_name` (a shared PVC provisioned out-of-band, see "Image
  and template catalogs" above) or a `home_mount_path` (a per-user home
  directory, see `charts/helve/README.md`'s "Home directories" section);
  Helve itself creates no Gateway/Ingress route for a launched workload at
  all, only an optional `LoadBalancer`/`ClusterIP` Service or its own
  reverse proxy.
- **vLLM/SGLang templates haven't been launched for real** — verified via
  a lightweight substitute (nginx/busybox) exercising the same code path
  (port/env/args/Service), not by actually pulling and running the
  multi-GB vLLM/SGLang images, which would have been slow in this
  environment. Expect to iterate on their default resource sizing once you
  actually run one.
- **No confirmation on template or image edits**, only on delete — saving over an
  existing template's fields is immediate.
- **Single namespace only**, fixed at deploy time via the pod's own
  namespace. No in-app namespace switcher; watching multiple namespaces
  means deploying multiple copies (see "Watching a different namespace").
- **Deployment management covers scale/edit/delete/restart/rollback, not
  everything** — see "Managing running deployments" above. Still no way to
  hand-edit a Service, no pod-level delete/restart independent of its
  Deployment (restart rolls every pod, not one at a time by hand), and
  rollback only ever undoes the single most recent change (like `kubectl
  rollout undo`) — no picker across a longer revision history.
- **`VLLM_API_KEY` is an educated guess, not a confirmed env var name** — it
  hasn't been verified against a real vLLM server run (see the vLLM
  templates gap above). If vLLM ignores it, the generated value shown in the
  UI simply won't do anything.
- **Auto-generated credentials are plaintext in Postgres**, not a Kubernetes
  `Secret` or any encrypted store, and anyone with `deployment_secrets`
  table access can read every credential ever generated, past or present.
  Explicitly deleting a Deployment through the Pods tab's manage panel does
  clean up its row, but a credential still outlives its Deployment if that
  Deployment is instead removed some other way (directly via `kubectl`,
  e.g.) — there's no reconciliation loop that notices and cleans up after
  the fact.
- **JupyterLab and RStudio get true JupyterHub-style transparent auth**
  (reverse proxy + injected credential or, for RStudio, no auth at all —
  click "Open" and you're in). vLLM still just displays a credential for
  copy/paste, deliberately — it's meant for scripted API clients where a
  proxied cookie-auth flow would be more friction, not less. See "Ownership,
  auto-generated credentials, and the reverse proxy" above.
- **RStudio's no-auth mode was verified as thoroughly as possible without
  deploying Helve in-cluster.** Confirmed by hand against a real
  `rocker/rstudio` container: `DISABLE_AUTH=true` + the `www-root-path`
  config line produce the expected redirect/cookie behavior when the
  correct `Host` header is forwarded (which the proxy does). What's *not*
  confirmed is a real browser session completing that flow through Helve's
  actual `/proxy/` route end to end — same ClusterIP-reachability limitation
  as JupyterLab's proxy path (see below), compounded by RStudio's own
  user-agent sniffing making command-line verification less conclusive than
  Puppeteer-based verification was for JupyterLab. Worth a real click-through
  once Helve is deployed in-cluster, before relying on it for anything
  sensitive.
- **The reverse proxy opens a fresh TCP connection per HTTP request**, not a
  pooled/reused one — correctness over performance for this first pass. Fine
  for interactive single-user use; would need pooling before it'd hold up
  under heavier concurrent load.
- **The reverse proxy assumes Helve itself runs in-cluster** — it connects
  to a proxy-enabled deployment's Service via its `ClusterIP`, which isn't
  routable from outside the cluster network. This is the one code path in
  this app that can't be exercised with the backend running locally against
  a remote cluster (this project's usual local-dev pattern); testing it for
  real requires an actual in-cluster install (Helm chart above) — it was
  instead verified by curling a proxy-enabled
  deployment's ClusterIP with the exact header Helve would send, from a
  throwaway pod inside the cluster, to confirm the target app accepts it
  correctly; the Rust-side HTTP/WebSocket-tunneling code was verified
  end-to-end in an earlier revision of this feature that used a different
  (pod-portforward-based) transport, then swapped in place — a small,
  well-contained change (only *how* a byte stream to the pod is obtained
  changed, not what's done with it), but that swap itself hasn't been
  exercised with a real Helve instance actually running in-cluster yet.
- **Light theme covers chart-chrome/ink tokens only** — the header's "Light
  theme"/"Dark theme" toggle (persisted in `localStorage`, defaulting to
  dark) swaps `--page`/`--surface`/`--surface-raised`/`--border`/
  `--border-strong`/`--text-primary`/`--text-secondary`/`--accent` to their
  validated light-mode steps via `:root[data-theme="light"]` in
  `frontend/style.css`. `--text-muted`, `--accent-ink`, and the status
  palette are intentionally identical in both modes (per the design
  system), so they're inherited rather than overridden. Keep new UI work on
  these custom properties rather than introducing new hex values, so it
  themes correctly for free.
- **Cross-compiling `linux/amd64` locally from an arm64 Mac doesn't work**
  (QEMU crashes `rustc`) — this is why the image is built by CI, not on a
  dev machine. If you ever need a local amd64 build, do it on an actual
  amd64 box, not by fighting emulation.
