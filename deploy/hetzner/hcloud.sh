#!/usr/bin/env bash
# Creates an Agent HQ server on Hetzner from the terminal, with the same user-data the hosted
# wizard uses. Needs the hcloud CLI, logged in (HCLOUD_TOKEN or an hcloud context).
#
#   bash hcloud.sh --name hq --ssh-key my-key --admin-cidr 203.0.113.7/32 \
#       --type <server-type> --location <location> --repo OWNER/REPO [--ref <40-hex commit>]
#
# List valid values with: hcloud server-type list ; hcloud location list ; hcloud ssh-key list
# The server must be created from a commit that is already pushed, since it downloads it.
set -euo pipefail

IMAGE="ubuntu-24.04"

name="" ssh_key="" cidr="" type="" location="" repo="" ref=""
die() { echo "hcloud.sh: $*" >&2; exit 1; }

while [ $# -gt 0 ]; do
    [ $# -ge 2 ] || die "missing value for $1"
    case "$1" in
        --name) name=$2 ;;
        --ssh-key) ssh_key=$2 ;;
        --admin-cidr) cidr=$2 ;;
        --type) type=$2 ;;
        --location) location=$2 ;;
        --repo) repo=$2 ;;
        --ref) ref=$2 ;;
        *) die "unknown argument $1" ;;
    esac
    shift 2
done

for v in name ssh_key cidr type location repo; do [ -n "${!v}" ] || die "--${v//_/-} is required"; done
[[ "$name" =~ ^[a-z0-9]([a-z0-9-]{0,61}[a-z0-9])?$ ]] || die "--name must be a DNS label"
[[ "$repo" =~ ^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+$ ]] || die "--repo must look like OWNER/REPO"
[[ "$cidr" =~ ^[0-9a-fA-F:.]+/[0-9]+$ ]] || die "--admin-cidr must look like 203.0.113.7/32"
command -v hcloud > /dev/null || die "install the hcloud CLI first"

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
[ -n "$ref" ] || ref="$(git -C "$here" rev-parse HEAD)"
[[ "$ref" =~ ^[0-9a-f]{40}$ ]] || die "--ref must be a full 40-character commit SHA"
sha="$(git -C "$here" show "$ref:deploy/hetzner/bootstrap.sh" | { sha256sum 2> /dev/null || shasum -a 256; } | cut -d' ' -f1)"

userdata="$(mktemp)"
trap 'rm -f "$userdata"' EXIT
sed -e "s|@REPO@|$repo|g" -e "s|@REF@|$ref|g" -e "s|@SHA256@|$sha|g" -e "s|@HOSTNAME@|$name|g" \
    "$here/cloud-init.yaml" > "$userdata"

fw="$name-fw"
hcloud firewall create --name "$fw"
{
    hcloud firewall add-rule "$fw" --direction in --protocol tcp --port 22 --source-ip "$cidr" &&
    hcloud server create --name "$name" --type "$type" --location "$location" --image "$IMAGE" \
    --ssh-key "$ssh_key" --firewall "$fw" --user-data-from-file "$userdata"
} || { hcloud firewall delete "$fw"; die "server create failed; removed firewall $fw"; }

echo "Created $name. Wait a few minutes, then: ssh root@<server-ip> and run: sudo hq-join"
