import { BadRequest, route } from '~/lib/api'
import { HetznerError, hcloud, LABEL_KEY, MANAGED_BY } from '~/lib/hetzner'
import { renderUserData } from '~/lib/userData'
import { isAdminCidr, isId, isKeyName, isServerName, isSshPublicKey } from '~/lib/validate'

const IMAGE = 'ubuntu-24.04'
const SSH_PORT = '22'

interface Body {
  token: string
  name: string
  location: string
  serverType: string
  adminCidr: string
  sshKeyId?: number
  newSshKey?: { name: string; publicKey: string }
  backups?: boolean
}

interface Created { server: { id: number; name: string; public_net: { ipv4: { ip: string } | null } } }

const PLAIN = /^[a-z0-9][a-z0-9-]{0,30}$/

function check(b: Body) {
  if (!isServerName(b.name)) throw new BadRequest('The name must be lowercase letters, digits and hyphens.')
  if (typeof b.location !== 'string' || !PLAIN.test(b.location)) throw new BadRequest('Pick a location.')
  if (typeof b.serverType !== 'string' || !PLAIN.test(b.serverType)) throw new BadRequest('Pick a server size.')
  if (!isAdminCidr(b.adminCidr)) throw new BadRequest('Enter your IP as a range, like 203.0.113.7/32. Opening SSH to everyone is not allowed.')
  if (b.sshKeyId !== undefined && !isId(b.sshKeyId)) throw new BadRequest('Pick an SSH key.')
  if (b.newSshKey && !(isKeyName(b.newSshKey.name) && isSshPublicKey(b.newSshKey.publicKey))) {
    throw new BadRequest('That is not a one-line SSH public key. Paste the .pub file, never the private key.')
  }
  if (b.sshKeyId === undefined && !b.newSshKey) throw new BadRequest('An SSH key is required to log in to the server.')
}

/**
 * Creates the firewall, then the server. If the server fails, what this call created is removed again,
 * so a failed attempt leaves nothing billing or dangling in the project.
 */
export async function POST(req: Request) {
  return route<Body>(req, async (body, pinned) => {
    check(body)
    const labels = { ...MANAGED_BY, [LABEL_KEY]: body.name }
    let newKeyId: number | null = null
    let firewallId: number | null = null
    try {
      let keyId = body.sshKeyId
      if (body.newSshKey) {
        const key = await hcloud<{ ssh_key: { id: number } }>(body.token, '/ssh_keys', {
          method: 'POST',
          body: { name: body.newSshKey.name, public_key: body.newSshKey.publicKey.trim(), labels },
        })
        newKeyId = key.ssh_key.id
        keyId = newKeyId
      }
      const fw = await hcloud<{ firewall: { id: number } }>(body.token, '/firewalls', {
        method: 'POST',
        body: {
          name: `${body.name}-fw`,
          labels,
          rules: [{ direction: 'in', protocol: 'tcp', port: SSH_PORT, source_ips: [body.adminCidr], description: 'SSH from the admin' }],
        },
      })
      firewallId = fw.firewall.id
      const created = await hcloud<Created>(body.token, '/servers', {
        method: 'POST',
        body: {
          name: body.name,
          server_type: body.serverType,
          location: body.location,
          image: IMAGE,
          ssh_keys: [keyId],
          firewalls: [{ firewall: firewallId }],
          user_data: renderUserData(pinned, body.name),
          labels,
          backups: body.backups === true,
          public_net: { enable_ipv4: true, enable_ipv6: true },
        },
      })
      return { serverId: created.server.id, name: created.server.name, ip: created.server.public_net.ipv4?.ip ?? null }
    } catch (e) {
      // After a timeout the server may exist already, and removing its firewall or key would hurt it.
      if (e instanceof HetznerError && e.code === 'unreachable') {
        throw new HetznerError('Hetzner did not answer in time. Reload this page and check for an existing server before trying again.', 504, 'unreachable')
      }
      if (firewallId !== null) await hcloud(body.token, `/firewalls/${firewallId}`, { method: 'DELETE' }).catch(() => undefined)
      if (newKeyId !== null) await hcloud(body.token, `/ssh_keys/${newKeyId}`, { method: 'DELETE' }).catch(() => undefined)
      throw e
    }
  })
}
