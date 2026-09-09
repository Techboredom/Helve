#!/usr/bin/env bash
# Asserts each fail-fast guard in charts/helve/templates/_helpers.tpl fires
# with its documented message, and that a representative set of otherwise
# valid values combinations render cleanly. Run from the repo root.
set -euo pipefail

CHART=charts/helve

assert_fails() {
  local desc="$1"; shift
  local expect="$1"; shift
  if helm template guard-test "$CHART" "$@" >/tmp/helm-out 2>/tmp/helm-err; then
    echo "FAIL ($desc): expected render to fail, but it succeeded" >&2
    exit 1
  fi
  if ! grep -qF "$expect" /tmp/helm-err; then
    echo "FAIL ($desc): stderr did not contain expected message: $expect" >&2
    echo "--- actual stderr ---" >&2
    cat /tmp/helm-err >&2
    exit 1
  fi
  echo "ok (expected failure): $desc"
}

assert_succeeds() {
  local desc="$1"; shift
  if ! helm template guard-test "$CHART" "$@" >/tmp/helm-out 2>/tmp/helm-err; then
    echo "FAIL ($desc): expected render to succeed, but it failed" >&2
    cat /tmp/helm-err >&2
    exit 1
  fi
  echo "ok (expected success): $desc"
}

BASE=(--set host=helve.example.com --set database.deploy.enabled=true --set ingress.tls.issuerRef.name=test-issuer)

assert_fails "host is required" \
  "host is required"

assert_fails "separateOrigins=false requires an explicit ack" \
  "proxy.allowSameOriginProxy=true" \
  --set host=helve.example.com --set proxy.separateOrigins=false \
  --set database.deploy.enabled=true --set ingress.tls.issuerRef.name=test-issuer

assert_fails "a database must be configured" \
  "no database configured" \
  --set host=helve.example.com --set ingress.tls.issuerRef.name=test-issuer

assert_fails "certManager mode requires an issuerRef" \
  "requires ingress.tls.issuerRef.name" \
  --set host=helve.example.com --set database.deploy.enabled=true

assert_fails "baseDomain more than one label deeper than host" \
  "more than one label deeper" \
  "${BASE[@]}" --set proxy.baseDomain=a.b.helve.example.com

assert_succeeds "full default render (bundled db, cert-manager)" \
  "${BASE[@]}"

assert_succeeds "same-origin proxy with explicit ack" \
  --set host=helve.example.com --set proxy.separateOrigins=false --set proxy.allowSameOriginProxy=true \
  --set database.deploy.enabled=true --set ingress.tls.issuerRef.name=test-issuer

assert_succeeds "existing db secret, TLS disabled" \
  --set host=helve.example.com --set database.existingSecret=helve-db-app --set ingress.tls.enabled=false

assert_succeeds "existing TLS secret instead of cert-manager" \
  --set host=helve.example.com --set database.deploy.enabled=true --set ingress.tls.mode=existingSecret

assert_succeeds "custom baseDomain exactly one label deeper than host" \
  "${BASE[@]}" --set proxy.baseDomain=apps.helve.example.com

echo "all helm guard checks passed"
