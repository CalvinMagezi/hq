#!/bin/sh
# Runs the hub's real systemd units and the real applier in throwaway privileged containers, one per
# base image, and checks what they do (inside.sh). It is a diagnostic, not part of the test suite:
# it needs Docker with a Linux kernel that has WireGuard and nf_tables, privileged containers,
# cgroup v2 and network access to fetch the Rust toolchain and base images.
#
#   deploy/net/test/run.sh                      # Debian 12 (systemd 252) and Ubuntu 22.04 (systemd 249)
#   deploy/net/test/run.sh debian:bookworm-slim # just one base image
#   KEEP=1 deploy/net/test/run.sh               # leave the images behind
#
# Containers, images and the build cache it made are removed at the end; the exit status is the
# number of failed checks.
set -u
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../../.." && pwd)
case $(docker info --format '{{.Architecture}}' 2>/dev/null) in
  aarch64|arm64) platform=linux/arm64 ;;
  x86_64|amd64) platform=linux/amd64 ;;
  *) echo "cannot tell the Docker architecture; is Docker running?" >&2; exit 2 ;;
esac
bases=${*:-"debian:bookworm-slim ubuntu:22.04"}
work=$(mktemp -d)
failures=0
cleanup() {
  docker rm -f hqnet-test >/dev/null 2>&1
  if [ -z "${KEEP:-}" ]; then
    for tag in $(docker images --format '{{.Repository}}' | grep '^hqnet-test-'); do docker rmi "$tag" >/dev/null 2>&1; done
    docker builder prune -f >/dev/null 2>&1
  fi
  rm -rf "$work"
}
trap cleanup EXIT

echo "building the applier (release, no default features) for $platform"
docker run --rm --platform "$platform" -e CARGO_INCREMENTAL=0 -e CARGO_TARGET_DIR=/tmp/target -e CARGO_HOME=/tmp/cargo \
  -v "$repo":/src:ro -v "$work":/out -w /src rust:1-slim-bookworm \
  sh -c 'cargo build --release --locked -p hq-net --no-default-features --bin hq-net-apply && cp /tmp/target/release/hq-net-apply /out/' \
  >"$work/build.log" 2>&1 || { tail -20 "$work/build.log"; exit 2; }

for base in $bases; do
  tag="hqnet-test-$(echo "$base" | tr -c 'a-z0-9\n' '-')"
  echo; echo "===== $base"
  docker build -q --platform "$platform" --build-arg BASE="$base" -t "$tag" "$here" >/dev/null || { echo "image build failed"; failures=$((failures + 1)); continue; }
  docker rm -f hqnet-test >/dev/null 2>&1
  docker run -d --name hqnet-test --platform "$platform" --privileged --cgroupns=host \
    -v /sys/fs/cgroup:/sys/fs/cgroup:rw --tmpfs /run --tmpfs /run/lock \
    -v "$repo/deploy/net":/deploy:ro -v "$work":/out:ro -v "$here/inside.sh":/inside.sh:ro -v "$here/hub-lib.sh":/hub-lib.sh:ro "$tag" >/dev/null
  i=0
  while [ $i -lt 40 ]; do
    state=$(docker exec hqnet-test systemctl is-system-running 2>/dev/null)
    [ "$state" = running ] || [ "$state" = degraded ] && break
    i=$((i + 1)); sleep 2
  done
  docker exec hqnet-test bash /inside.sh
  failures=$((failures + $?))
  docker rm -f hqnet-test >/dev/null 2>&1
done
echo; echo "total failures: $failures"
exit $failures
