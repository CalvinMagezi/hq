#!/bin/sh
# Runs the real `hq net tunnel run` as a device against a real hub (kernel WireGuard, nftables, the
# real systemd units and applier, set up with the real `hq net` commands) in a throwaway privileged
# container, and checks what the device and the hub do (inside.sh): handshake, the serve and reach
# maps, the hub's firewall against the device's own source check, a network change, a lost link,
# revoke and re-add, a killed and restarted device, a restarted hub, a flood, and one connection held
# across rekeys. It is a diagnostic, not part of the test suite or the required CI checks: it needs
# Docker with a Linux kernel that has WireGuard and nf_tables, privileged containers, cgroup v2 and
# network access to fetch the Rust toolchain and base images.
#
#   deploy/net/tunnel-test/run.sh                  # all checks, about 15 minutes
#   FAST=1 deploy/net/tunnel-test/run.sh           # one rekey instead of three
#   SOAK=1 DURATION=3600 deploy/net/tunnel-test/run.sh   # also an hour with 170 ms and 1 percent loss
#   HQ_BIN=/path/to/linux/hq deploy/net/tunnel-test/run.sh   # skip the build of hq
#   PRUNE=1 deploy/net/tunnel-test/run.sh          # also remove the cargo cache volume afterwards
#
# The Linux `hq` (a debug build with --features tunnel, so the numbers it gives are a debug build's)
# is built in a container whose cargo and target directories live in the volume hqnet-cargo, one
# target directory per checkout so a build of another worktree is never mistaken for this one's, so
# only the first run is slow (about 3 minutes and 4 GB on arm64). The test container has no network
# but its own and is removed at the end, with the image it ran; the base images and the volume stay
# (PRUNE=1 removes the volume). The exit status is the number of failed checks. Logs of the streams
# and the soak are copied to results/ next to this script, which each run empties first.
set -u
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
case $(docker info --format '{{.Architecture}}' 2>/dev/null) in
  aarch64|arm64) platform=linux/arm64 ;;
  x86_64|amd64) platform=linux/amd64 ;;
  *) echo "cannot tell the Docker architecture; is Docker running?" >&2; exit 2 ;;
esac
base=${BASE:-debian:bookworm-slim}
tag=hqtun-test-image-$$
name=hqtun-test-$$
key=$(printf %s "$repo" | cksum | cut -d' ' -f1)
work=$(mktemp -d)
results=$here/results
mkdir -p "$results"
rm -f "$results"/*
cleanup() {
  docker rm -f "$name" >/dev/null 2>&1
  docker rmi "$tag" >/dev/null 2>&1
  [ -n "${PRUNE:-}" ] && docker volume rm hqnet-cargo >/dev/null 2>&1
  rm -rf "$work"
}
trap cleanup EXIT

echo "building the applier (release, no default features) for $platform"
docker run --rm --platform "$platform" -e CARGO_INCREMENTAL=0 -e CARGO_TARGET_DIR=/cache/target-applier-$key -e CARGO_HOME=/cache/cargo \
  -v hqnet-cargo:/cache -v "$repo":/src:ro -v "$work":/out -w /src rust:1-slim-bookworm \
  sh -c 'cargo build --release --locked -p hq-net --no-default-features --bin hq-net-apply && cp /cache/target-applier-'"$key"'/release/hq-net-apply /out/' \
  >"$work/build-applier.log" 2>&1 || { tail -20 "$work/build-applier.log"; exit 2; }

if [ -n "${HQ_BIN:-}" ]; then
  cp "$HQ_BIN" "$work/hq"
else
  echo "building hq with the tunnel feature for $platform (first run: minutes)"
  docker run --rm --platform "$platform" -e CARGO_INCREMENTAL=0 -e CARGO_PROFILE_DEV_DEBUG=0 -e CARGO_TARGET_DIR=/cache/target-$key -e CARGO_HOME=/cache/cargo \
    -v hqnet-cargo:/cache -v "$repo":/src:ro -v "$work":/out -w /src rust:1-slim-bookworm \
    sh -c 'apt-get update -qq >/dev/null && apt-get install -y -qq build-essential perl pkg-config >/dev/null 2>&1; cargo build --locked -p hq-cli --features tunnel && cp /cache/target-'"$key"'/debug/hq /out/hq' \
    >"$work/build-hq.log" 2>&1 || { tail -20 "$work/build-hq.log"; exit 2; }
fi

docker build -q --platform "$platform" --build-arg BASE="$base" -t "$tag" "$repo/deploy/net/test" >/dev/null || { echo "image build failed"; exit 2; }
docker run -d --name "$name" --platform "$platform" --privileged --network none --cgroupns=host \
  -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
  -v "$repo/deploy/net":/deploy:ro -v "$work":/out:ro -v "$results":/results \
  -v "$here/inside.sh":/inside.sh:ro -v "$here/helper.py":/helper.py:ro -v "$repo/deploy/net/test/hub-lib.sh":/hub-lib.sh:ro \
  "$tag" >/dev/null
i=0
while [ $i -lt 40 ]; do
  state=$(docker exec "$name" systemctl is-system-running 2>/dev/null)
  [ "$state" = running ] || [ "$state" = degraded ] && break
  i=$((i + 1)); sleep 2
done
docker exec -e FAST="${FAST:-}" -e SOAK="${SOAK:-}" -e DURATION="${DURATION:-3600}" -e REKEY_SECS="${REKEY_SECS:-400}" "$name" bash /inside.sh
failures=$?
echo "logs: $results"
exit $failures
