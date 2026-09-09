# Contributing

## Prerequisites

- Rust (stable), via [rustup](https://rustup.rs).
- The `wasm32-unknown-unknown` target: `rustup target add wasm32-unknown-unknown`.
- [`trunk`](https://trunkrs.dev) to build the frontend: `cargo install trunk --locked`.
- A Postgres instance for anything backend-related (a throwaway
  `docker run -e POSTGRES_PASSWORD=postgres -p 5432:5432 postgres:16-alpine`
  is enough — migrations run automatically on startup via `sqlx::migrate!()`).
- [Helm](https://helm.sh) 3.x if you're changing `charts/helve/`.

## Building and running

```
cargo build -p backend
cd frontend && trunk build
```

`cargo run -p backend -- --namespace <ns>` needs `NAMESPACE`, `DATABASE_URL`,
and a working kubeconfig (or in-cluster service account) — see the
top-level README's "Running locally" section for the full set of env vars.

## Tests and lints

CI (`.github/workflows/ci.yml`) runs, and your change should pass before
opening a PR:

```
cargo test -p backend -p common
cargo clippy --all-targets -p backend -p common -- -D warnings
cd frontend && cargo clippy --target wasm32-unknown-unknown -- -D warnings
```

`cargo fmt` is **not** enforced — the tree isn't rustfmt-clean throughout,
and a blanket reformat would bury real changes in unrelated diffs. Match
the style of the surrounding code instead of reformatting files you touch.

If you change anything under `charts/helve/`:

```
helm lint charts/helve
helm template charts/helve --set host=example.test
```

## Testing philosophy

This project's standing pattern is to actually exercise a change against a
real (if throwaway) Postgres and a real Kubernetes cluster before calling it
done — unit tests cover pure logic (validation, quota arithmetic, proxy host
matching), but anything touching the k8s API or a live HTTP flow should be
checked by hand too. Clean up any test Deployments/Services you create
against a shared cluster.

## Commit messages and PRs

Describe *why* a change was made, not just what changed — future readers
have the diff already. Keep unrelated changes in separate commits/PRs.
