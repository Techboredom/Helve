# Security policy

Helve authenticates users, holds their session cookies, proxies arbitrary
workloads on their behalf, and stores generated credentials (API keys,
notebook tokens) for the apps it launches. Treat a vulnerability here as
something that could expose another user's session, another user's proxied
app, or the cluster it runs on.

## Supported versions

Only the latest tagged release is supported. Security fixes are made
against the `main` branch and released as a new tag; there is no long-term
support branch for older releases. Upgrade to the latest release before
reporting an issue, to confirm it's still present.

## Reporting a vulnerability

Do not open a public GitHub issue for a suspected vulnerability. Instead,
email **johnscod@gmail.com** with:

- A description of the issue and its impact.
- Steps to reproduce, or a proof-of-concept if you have one.
- The version/commit you tested against.

You should get an acknowledgment within a few days. Once a fix is
available, it will be released and the report credited (unless you'd
rather stay anonymous), with a note in `CHANGELOG.md`.

## Scope

In scope: the `backend`/`frontend`/`common` Rust code, the Helm chart in
`charts/helve/`, and the container image built from `Dockerfile`.

Out of scope: vulnerabilities in the base images this project depends on
(`gcr.io/distroless/cc-debian12`, `rust`) — report those upstream — and
misconfiguration of a specific deployment (e.g. running without TLS, or
without setting `PROXY_BASE_DOMAIN`) that the chart's own guards or the
README already document as insecure.
