#!/bin/sh
# Runs the connector TLS ACME test against Pebble, Let's Encrypt's ACME test server, in Docker.
# A diagnostic, not part of the test suite: it needs Docker, network access to pull two images and
# a free local port 5001 (Pebble validates TLS-ALPN-01 there).
#
#   scripts/test-connector-tls-pebble.sh
#
# Pebble resolves every name to this machine through pebble-challtestsrv, dials port 5001 on it and
# validates the challenge the door answers. While the test runs the door listens on every interface of
# this machine (the containers reach it from the Docker network), and it serves only the connector's
# own paths. Pebble's own ports are published on loopback only. The containers and the network are
# removed at the end.
set -u
repo=$(cd "$(dirname "$0")/.." && pwd)
# Pinned by tag and digest (checked 2026-10-22; 2.10.1 is the release of 2026-04-20).
PEBBLE=ghcr.io/letsencrypt/pebble:2.10.1@sha256:ddf230642b1a584f519f32e347de1b05a6e4c1f6c35c1863b33effeab5f78199
CHALLTESTSRV=ghcr.io/letsencrypt/pebble-challtestsrv:2.10.1@sha256:12ce21884def456bcf9786542113949e1f19dc7738d2c70e156c2d0c38a1405b
NET=hq-pebble-net
DOOR_PORT=5001
work=$(mktemp -d)
remove_containers() {
  docker rm -f hq-pebble hq-challtestsrv >/dev/null 2>&1
  docker network rm "$NET" >/dev/null 2>&1
}
cleanup() {
  remove_containers
  rm -rf "$work"
}
trap cleanup EXIT

command -v docker >/dev/null 2>&1 || { echo "docker is not installed" >&2; exit 2; }
docker info >/dev/null 2>&1 || { echo "docker is not running" >&2; exit 2; }
remove_containers
docker network create "$NET" >/dev/null || exit 2

# The address the containers reach this machine at: the door listens here.
host_ip=$(docker run --rm --add-host host.docker.internal:host-gateway debian:bookworm-slim \
  getent ahostsv4 host.docker.internal | awk 'NR == 1 { print $1 }')
[ -n "$host_ip" ] || { echo "could not find this machine's address from a container" >&2; exit 2; }
echo "containers reach this machine at $host_ip"

docker run -d --name hq-challtestsrv --network "$NET" "$CHALLTESTSRV" \
  -defaultIPv4 "$host_ip" -defaultIPv6 "" -http01 "" -https01 "" -tlsalpn01 "" -doh "" >/dev/null || exit 2
docker run -d --name hq-pebble --network "$NET" -p 127.0.0.1:14000:14000 -p 127.0.0.1:15000:15000 \
  -e PEBBLE_VA_NOSLEEP=1 -e PEBBLE_VA_ALWAYS_VALID=0 "$PEBBLE" \
  -config test/config/pebble-config.json -dnsserver hq-challtestsrv:8053 >/dev/null || exit 2

docker cp hq-pebble:/test/certs/pebble.minica.pem "$work/pebble.minica.pem" >/dev/null || exit 2
i=0
until curl -fsS --cacert "$work/pebble.minica.pem" https://localhost:14000/dir >/dev/null 2>&1; do
  i=$((i + 1))
  [ $i -lt 30 ] || { echo "Pebble did not come up" >&2; docker logs hq-pebble | tail -20 >&2; exit 2; }
  sleep 1
done
echo "Pebble is up"

cd "$repo" || exit 2
HQ_PEBBLE_DIRECTORY=https://localhost:14000/dir \
HQ_PEBBLE_MANAGEMENT=https://localhost:15000 \
HQ_PEBBLE_MINICA="$work/pebble.minica.pem" \
HQ_PEBBLE_LISTEN="0.0.0.0:$DOOR_PORT" \
CARGO_PROFILE_DEV_DEBUG=${CARGO_PROFILE_DEV_DEBUG:-0} CARGO_INCREMENTAL=${CARGO_INCREMENTAL:-0} \
  cargo test -p hq-web --lib tls::tests::pebble -- --ignored --nocapture
status=$?
if [ $status -ne 0 ]; then docker logs hq-pebble 2>&1 | tail -30; fi
exit $status
