# helve

A web app for managing compute environments and AI engines on a single
Kubernetes namespace — launch JupyterLab/RStudio environments or LLM
inference engines (Ollama, vLLM, SGLang) with a few clicks, behind a login.

## Prerequisites

- Kubernetes 1.27+.
- An ingress controller (this chart renders a plain `networking.k8s.io/v1`
  `Ingress` — Traefik, ingress-nginx's successor, or anything else that
  honors `ingressClassName` and forwards the `Host` header unmodified;
  the backend dispatches on it, see `proxy::dispatch_by_host`).
- Wildcard DNS for `*.<proxy.baseDomain>` pointed at that ingress
  controller, unless you set `proxy.separateOrigins=false` (not
  recommended — see below).
- [cert-manager](https://cert-manager.io) if you leave `ingress.tls.mode`
  at its default of `certManager`, with an `Issuer`/`ClusterIssuer` that
  can solve a **DNS-01** challenge. This is not optional if
  `proxy.separateOrigins` is true: HTTP-01 cannot issue a certificate for
  a wildcard name (`*.<proxy.baseDomain>`), and that will be the first
  thing a public-internet install hits.
- A Postgres database Helve can run its own migrations against
  (`database.existingSecret`), or the
  [CloudNativePG](https://cloudnative-pg.io) operator installed if you'd
  rather use `database.deploy.enabled=true` for evaluation.

## Quick start

```console
helm install helve oci://ghcr.io/techboredom/charts/helve \
  --namespace helve --create-namespace \
  --set host=helve.example.com \
  --set database.existingSecret=helve-db-app \
  --set ingress.tls.issuerRef.name=letsencrypt \
  --set adminBootstrap.password=<a-temporary-password>
```

This is the minimum that renders: a hostname, a database, and something to
issue a certificate. Everything else has a working default. See
`values.yaml` for the full set, and the table below for what each one
does.

## Evaluating without your own Postgres or cert-manager

```console
helm install helve oci://ghcr.io/techboredom/charts/helve \
  --namespace helve --create-namespace \
  --set host=helve.example.test \
  --set database.deploy.enabled=true \
  --set ingress.tls.enabled=false \
  --set adminBootstrap.password=<a-temporary-password>
```

`database.deploy.enabled=true` is an evaluation-only, single-replica,
no-backups Postgres — don't point it at anything you'd be upset to lose.
`ingress.tls.enabled=false` serves plain HTTP; the app logs a warning at
startup that session cookies can't be marked `Secure` as a result.

## A worked cert-manager + Let's Encrypt example

Helve's wildcard proxy origin (`*.<proxy.baseDomain>`) means Let's
Encrypt's HTTP-01 challenge won't work — you need a DNS-01 solver, which
means a `ClusterIssuer` configured for your DNS provider's API. For
example, with Cloudflare:

```yaml
apiVersion: cert-manager.io/v1
kind: ClusterIssuer
metadata:
  name: letsencrypt
spec:
  acme:
    server: https://acme-v02.api.letsencrypt.org/directory
    email: you@example.com
    privateKeySecretRef:
      name: letsencrypt-account-key
    solvers:
      - dns01:
          cloudflare:
            apiTokenSecretRef:
              name: cloudflare-api-token
              key: api-token
```

Then point the chart at it:

```console
--set ingress.tls.issuerRef.name=letsencrypt --set ingress.tls.issuerRef.kind=ClusterIssuer
```

cert-manager's own docs cover the full list of supported DNS-01 solvers
and their required credentials.

## Values

| Key | Default | Description |
|---|---|---|
| `image.repository` | `ghcr.io/techboredom/helve` | Container image repository. |
| `image.tag` | `""` | Image tag; defaults to `.Chart.AppVersion`. |
| `image.pullPolicy` | `IfNotPresent` | |
| `image.pullSecrets` | `[]` | Names of existing image-pull Secrets. |
| `host` | `""` | **Required.** Public hostname Helve is served from. |
| `replicaCount` | `1` | Safe to raise — see "High availability" below. |
| `terminationGracePeriodSeconds` | `30` | Must exceed the app's own post-`SIGTERM` drain wait (~5s) plus real request completion time. |
| `proxy.baseDomain` | `""` | Base domain for per-deployment proxy origins; defaults to `proxy.<host>`. |
| `proxy.separateOrigins` | `true` | Serve each proxied deployment from its own origin. Recommended; see below before turning off. |
| `proxy.allowSameOriginProxy` | `false` | Must be `true` to set `proxy.separateOrigins=false`. |
| `ingress.enabled` | `true` | |
| `ingress.className` | `""` | |
| `ingress.annotations` | `{}` | |
| `ingress.tls.enabled` | `true` | |
| `ingress.tls.mode` | `certManager` | `certManager`, `existingSecret`, or `none`. |
| `ingress.tls.secretName` | `helve-tls` | |
| `ingress.tls.issuerRef.name` | `""` | **Required when `mode=certManager`.** |
| `ingress.tls.issuerRef.kind` | `ClusterIssuer` | |
| `database.existingSecret` | `""` | Secret holding a Postgres connection string. Either this or `database.deploy.enabled` is required. |
| `database.existingSecretKey` | `uri` | Key within `existingSecret`. |
| `database.deploy.enabled` | `false` | Evaluation-only bundled CloudNativePG Postgres. Not for production data. |
| `database.deploy.storage.size` | `5Gi` | |
| `database.deploy.storage.className` | `""` | Empty uses the cluster's default StorageClass. |
| `homeDrives.mode` | `off` | `off`, `hostPath`, or `pvc`. See "Home directories" below. |
| `homeDrives.hostPath.basePath` | `""` | **Required when `mode=hostPath`.** Path every node already has a shared filesystem mounted at. |
| `homeDrives.pvc.strategy` | `dynamic` | `dynamic` (one PVC per user, created by Helve) or `shared` (one PVC you provision, subPath per user). |
| `homeDrives.pvc.storageClassName` | `""` | **Required when `mode=pvc`, `strategy=dynamic`.** Must support `ReadWriteMany`. |
| `homeDrives.pvc.size` | `10Gi` | Size of each per-user home PVC. Only meaningful with `strategy=dynamic`. |
| `homeDrives.pvc.existingClaimName` | `""` | **Required when `mode=pvc`, `strategy=shared`.** The PVC already provisioned for this. |
| `adminBootstrap.existingSecret` | `""` | Existing Secret (key `password`) for the first-boot admin account. |
| `adminBootstrap.password` | `""` | Convenience alternative to `existingSecret`; ends up in Helm release history. |
| `nodeSelector` | `{}` | |
| `tolerations` | `[]` | |
| `affinity` | `{}` | |
| `serviceAccount.create` | `true` | |
| `serviceAccount.name` | `""` | |
| `serviceAccount.annotations` | `{}` | |
| `rbac.create` | `true` | Namespace-scoped `Role`+`RoleBinding` only, never a `ClusterRole`. |
| `service.type` | `ClusterIP` | |
| `service.port` | `3000` | |
| `resources` | `{requests: {cpu: 50m, memory: 64Mi}, limits: {cpu: 200m, memory: 128Mi}}` | |
| `extraEnv` | `[]` | Extra container env vars, e.g. `[{name: RUST_LOG, value: debug}]`. |

## Home directories

Off by default (`homeDrives.mode: off`). A template opts in individually
by setting its own "Home directory mount path" field (Templates admin
tab, or `home_mount_path` in the API) to where its image expects one, e.g.
`/home/jovyan` for JupyterLab — leave it blank for templates that don't
need one (Ollama, vLLM, SGLang).

Two backends, chosen with `homeDrives.mode`:

- **`hostPath`** — mounts `<homeDrives.hostPath.basePath>/<username>` from
  the node's own filesystem (`DirectoryOrCreate`). Requires every node in
  the cluster to already have the *same* shared filesystem (NFS, CephFS, a
  parallel filesystem, whatever your cluster already has) mounted at that
  path at the OS level — from Kubernetes' point of view the path looks
  identical on any node, so no StorageClass or CSI driver is involved at
  all, and there's no `ReadWriteMany` requirement to satisfy.
- **`pvc`** — one `PersistentVolumeClaim`-backed home directory, in one of
  two strategies (`homeDrives.pvc.strategy`):
  - **`dynamic`** (default) — Helve creates one PVC per user
    (`home-<username>`) from `homeDrives.pvc.storageClassName`, sized
    `homeDrives.pvc.size`, on that user's first home-drive launch. That
    StorageClass **must support `ReadWriteMany`** if a user can ever run
    more than one home-drive environment at once — a `ReadWriteOnce` claim
    only ever attaches to one node at a time, and the chart has no way to
    check which kind a given StorageClass actually provisions before
    creating the claim. This is the one place this chart creates a
    PersistentVolumeClaim itself (see `rbac.yaml`'s comment) — and,
    matching the "Helve never deletes X" caution applied everywhere else
    in this project, it never deletes one either, not even on user
    deletion.
  - **`shared`** — you provision exactly one PVC yourself
    (`homeDrives.pvc.existingClaimName`), the same out-of-band way the
    existing shared-model-cache storage mount already works, and Helve
    mounts that single claim into every user's pod with
    `subPath: <username>` — Kubernetes creates that subdirectory on the
    volume automatically the first time it's mounted, so this needs no
    per-user provisioning step and no `create` RBAC on PVCs at all. The
    tradeoff is one shared capacity pool rather than a size cap per user.
    Works with anything RWX-capable, including a statically-bound
    PV/PVC pair — same as `hostPath` mode, dynamic provisioning isn't
    required, just a shared, writable volume.

File ownership and permissions on whatever the mount resolves to are the
shared filesystem's own concern (its export config, UID mapping, etc.) —
Helve does not `chown` anything itself. This matters more for `hostPath`
than `pvc`: Kubernetes' `fsGroup` (which a per-user UID/GID set on the
Users tab turns into `runAsUser`/`runAsGroup`/`fsGroup` on the pod) does
**not** apply to `hostPath` volumes at all — a directory Kubernetes just
created with `DirectoryOrCreate` is owned by whatever the kubelet itself
runs as, and setting a UID/GID doesn't get that UID write access to it
automatically. `pvc` mode doesn't have this problem for any CSI driver
that declares `fsGroupPolicy: File` — true of most modern ones (Ceph,
NFS-CSI, ...) — since `fsGroup` there does apply.

For `hostPath` specifically, the Users tab's **supplemental groups**
(pod `securityContext` `supplementalGroups`) are the actual fix, not a
workaround: unlike `fsGroup`, they're a property of the *process* rather
than something Kubernetes chowns onto the volume, so they apply
identically no matter what's mounted
([kubernetes/kubernetes#138411](https://github.com/kubernetes/kubernetes/issues/138411)
confirms the same). Configure the shared filesystem's export so the
directories it grants write access to are owned by some GID, then assign
that GID as a supplemental group to whichever users need it — their
containers can then write there regardless of their `runAsUser`. Failing
that, pre-create each user's directory with matching ownership, or leave
`uid`/`gid` unset for users whose images already run fine as whatever
identity the filesystem already grants access to.

## Guards

This chart refuses to render (via a `fail` in `templates/_helpers.tpl`)
rather than produce a broken or quietly-insecure install:

- `host` unset.
- `proxy.separateOrigins=false` without `proxy.allowSameOriginProxy=true`.
- No database configured (`database.existingSecret` unset and
  `database.deploy.enabled=false`).
- `ingress.tls.mode=certManager` with no `ingress.tls.issuerRef.name`.
- `proxy.baseDomain` set to more than one label deeper than `host` when
  using `separateOrigins` + cert-manager TLS — a wildcard certificate only
  ever covers one label, matching `ProxyOrigin::deployment_for_host` in
  the backend.
- `homeDrives.mode=hostPath` with no `homeDrives.hostPath.basePath`.
- `homeDrives.mode=pvc` with no `homeDrives.pvc.storageClassName` or no
  `homeDrives.pvc.size`.

## High availability

`replicaCount` can be raised above the default of 1 — session cookies,
proxy handoff tokens, and quota enforcement all live in Postgres (a
Postgres advisory lock, specifically, not an in-process mutex), so
multiple replicas stay correct rather than each independently
under-enforcing quota or losing the others' sessions.

A rollout is zero-downtime **even at the default of 1 replica**: the
Deployment's `RollingUpdate` strategy (`maxUnavailable: 0, maxSurge: 1`)
brings up a new, ready pod before removing the old one, and the app
drains in-flight HTTP requests on `SIGTERM` rather than dropping them (see
`terminationGracePeriodSeconds` above).

What this doesn't cover: a connection already upgraded to a raw byte
stream — the Pods tab's live-update WebSocket, or a proxied deployment's
tunneled WebSocket (e.g. a JupyterLab kernel session) — isn't drained
gracefully. Those end when their pod does, the same as with any other
rolling restart of a stateful WebSocket server; the client reconnecting
(or the user retrying) is what recovers, not something this chart or the
backend's shutdown handling can paper over.

## Verifying an install

```console
helm test helve -n helve
```

Curls `/readyz` (unauthenticated, checks Postgres connectivity) through
the Service — this doesn't require knowing anything about how to log in.

## Upgrading

`helm upgrade` as usual. Nothing here runs its own migrations job — the
app runs `sqlx::migrate!()` against the database on every startup, so a
new image version migrates the schema itself the moment its first replica
comes up.
