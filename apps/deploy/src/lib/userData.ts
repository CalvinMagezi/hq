import { isServerName } from './validate'

const REPO = /^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/
const COMMIT = /^[0-9a-f]{40}$/
const SHA256 = /^[0-9a-f]{64}$/

// Must match deploy/hetzner/cloud-init.yaml exactly; userData.test.ts enforces it.
const TEMPLATE = String.raw`#cloud-config
# Hetzner one-button deploy. Fill the four placeholders below (hcloud.sh does this), then pass the
# result as the server's user-data.
#
# Server user-data is readable by any process on the server through the metadata service and by
# anyone holding the Hetzner API token, so nothing secret goes in here. The SSH public key is
# attached by Hetzner itself, and the web token is generated on the server by bootstrap.sh.
#
#   @REPO@      OWNER/REPO of the repository that publishes HQ releases
#   @REF@       full 40-character commit SHA to fetch bootstrap.sh and the installer from
#   @SHA256@    sha256 of deploy/hetzner/bootstrap.sh at that commit
#   @HOSTNAME@  name this server will use on the tailnet
package_update: true
packages:
  - curl
  - ca-certificates

runcmd:
  - - bash
    - -ec
    - |
      curl --fail --silent --show-error --location --proto '=https' --tlsv1.2 -o /root/bootstrap.sh \
        "https://raw.githubusercontent.com/@REPO@/@REF@/deploy/hetzner/bootstrap.sh"
      echo "@SHA256@  /root/bootstrap.sh" | sha256sum -c -
      bash /root/bootstrap.sh --repo "@REPO@" --ref "@REF@" --hostname "@HOSTNAME@"
`

export interface Pinned {
  repo: string
  ref: string
  sha256: string
}

/** The pinned bootstrap, from the deployment's environment. Null when it is missing or malformed. */
export function pinnedFromEnv(env: Record<string, string | undefined> = process.env): Pinned | null {
  const repo = env.HQ_BOOTSTRAP_REPO ?? 'CalvinMagezi/hq'
  const ref = env.HQ_BOOTSTRAP_REF ?? ''
  const sha256 = env.HQ_BOOTSTRAP_SHA256 ?? ''
  if (!REPO.test(repo) || !COMMIT.test(ref) || !SHA256.test(sha256)) return null
  return { repo, ref, sha256 }
}

export function renderUserData(pinned: Pinned, hostname: string): string {
  if (!isServerName(hostname)) throw new Error('hostname must be a DNS label')
  return TEMPLATE.replaceAll('@REPO@', pinned.repo)
    .replaceAll('@REF@', pinned.ref)
    .replaceAll('@SHA256@', pinned.sha256)
    .replaceAll('@HOSTNAME@', hostname)
}

export const CLOUD_INIT_TEMPLATE = TEMPLATE
