#!/usr/bin/env bash
# Creates a CA-trusted buildx builder, parameterized by a unique suffix
# (the arch it's about to build for) plus the calling workflow run's own
# ID, so builds from different runs can never collide on the same
# host-mode runner.
#
# The run ID is the part that actually matters: this workflow triggers on
# both a push to main *and* a "vX.Y.Z" tag push (build.yml's `on:`), and a
# release is exactly a branch commit immediately followed by pushing a tag
# that points at it - two independent workflow runs for the same commit,
# with no ordering guarantee between them on this one physical runner.
# Before the run ID was part of it, both runs used the identical
# "helve-builder-amd64" name and "/tmp/helve-buildx-amd64" workdir: one
# run's `rm -rf "$WORKDIR" && mkdir -p "$WORKDIR"` (below) could delete or
# truncate the other's in-flight buildkitd.toml between it being written
# and `docker buildx create --buildkitd-config` reading it back - "file
# missing right after being created" (see this project's own history of
# that exact failure class), just triggered by two *workflow runs* racing
# instead of two *architectures* racing within one run.
#
# Usage: setup-buildx.sh <suffix> <run-id>   e.g. setup-buildx.sh amd64 "$GITHUB_RUN_ID"
set -euo pipefail

SUFFIX="$1"
RUN_ID="${2:-local}"
BUILDER_NAME="helve-builder-${SUFFIX}-${RUN_ID}"
WORKDIR="/tmp/helve-buildx-${SUFFIX}-${RUN_ID}"

docker run --rm --privileged tonistiigi/binfmt --install all

# Where the host trusts this CA from is not assumed to be one fixed path -
# this project has hit CA trust living in a different place on every
# surface it's touched (k8s node containerd, the Docker daemon, and this
# host's own Arch trust-anchors rather than certs.d). Search by subject
# rather than a specific filename.
CA_SRC=""
for candidate in \
  /etc/docker/certs.d/ctr.int.techboredom.com:8443/ca.crt \
  /etc/ssl/certs/*.pem \
  /etc/pki/ca-trust/source/anchors/*.crt \
  /usr/local/share/ca-certificates/*.crt
do
  [ -f "$candidate" ] || continue
  if openssl x509 -in "$candidate" -noout -subject 2>/dev/null | grep -q 'O=LocalCA'; then
    CA_SRC="$candidate"
    break
  fi
done

if [ -z "$CA_SRC" ]; then
  echo "Could not find the internal CA (subject O=LocalCA) anywhere this runner checked."
  echo "docker login succeeding means the host trusts it from *somewhere* - add that"
  echo "location to the candidate list above."
  exit 1
fi

rm -rf "$WORKDIR" && mkdir -p "$WORKDIR"
# -L dereferences: this host's copy is a symlink (Arch's trust-anchor
# extraction), and buildx needs to read real bytes here, not a link.
cp -L "$CA_SRC" "$WORKDIR/internal-ca.crt"

# --driver-opt image=<name> needs a real, pullable reference - a purely
# local, never-pushed tag isn't reliably used even when `docker images`
# shows it built and present (moby/moby#49453). DOCKER_BUILDKIT=0 forces
# the classic, non-buildx builder for this one build+push - deliberately
# not routed through the builder this script is about to create (which
# doesn't trust Harbor yet; fixing that is the entire point) or whatever
# builder happens to be ambient. Uses the plain Docker daemon's own push
# instead, which already trusts Harbor via this host's system
# trust-anchors, independent of anything buildx/buildkit-related.
#
# Shared content across both suffixes (same Dockerfile, same bytes) but
# each invocation still builds and pushes its own copy rather than
# coordinating with the other - redundant if both run concurrently, but
# the whole thing costs a couple of seconds against an already-pulled
# base image, and avoiding a shared step here is what keeps this script
# fully self-contained per job.
cat > "$WORKDIR/Dockerfile" <<'DOCKERFILE'
FROM moby/buildkit:buildx-stable-1
COPY internal-ca.crt /usr/local/share/ca-certificates/internal-ca.crt
DOCKERFILE

BUILDKIT_IMAGE="${REGISTRY}/helve/buildkit-with-ca:latest"
DOCKER_BUILDKIT=0 docker build -t "$BUILDKIT_IMAGE" "$WORKDIR"
docker push "$BUILDKIT_IMAGE"

# The directive BuildKit's registry client actually consults for its own
# push path - still needed even with the cert baked into the image's OS
# trust store, per moby/buildkit#5576: BuildKit's registry client doesn't
# reliably fall back to the OS cert pool regardless of what's in it.
#
# Read from THIS HOST by buildx at create time, confirmed directly from a
# real create's own logged output - not from inside the image built
# above, which is a separate, independently-necessary fix for a different
# problem (image resolution, not registry trust).
cat > "$WORKDIR/buildkitd.toml" <<TOML
[registry."ctr.int.techboredom.com:8443"]
  ca = ["${WORKDIR}/internal-ca.crt"]
TOML

# Recreated every run rather than reused: both --driver-opt image= and
# --buildkitd-config only take effect at container creation.
docker buildx rm "$BUILDER_NAME" 2>/dev/null || true
docker buildx create --name "$BUILDER_NAME" --driver docker-container \
  --driver-opt image="$BUILDKIT_IMAGE" \
  --buildkitd-config "$WORKDIR/buildkitd.toml" \
  --use
docker buildx inspect --bootstrap

echo "builder_name=${BUILDER_NAME}"
echo "workdir=${WORKDIR}"
