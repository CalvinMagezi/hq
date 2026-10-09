import { BadRequest, route } from '~/lib/api'
import { hcloud, LABEL_KEY, listAll, MANAGED_BY, ownedSelector } from '~/lib/hetzner'
import { isAdminCidr, isId } from '~/lib/validate'

const SSH_PORT = '22'

interface Server { id: number; name: string; labels: Record<string, string> }

/**
 * Closes public SSH once the server is on the tailnet (no `openFrom`), or reopens it for one admin
 * range when the tailnet path is lost. Only the firewall this wizard made for that server is touched.
 */
export async function POST(req: Request) {
  return route<{ token: string; serverId: number; openFrom?: string }>(req, async ({ token, serverId, openFrom }) => {
    if (!isId(serverId)) throw new BadRequest('Unknown server.')
    if (openFrom !== undefined && !isAdminCidr(openFrom)) throw new BadRequest('Enter your IP as a range, like 203.0.113.7/32.')
    const { server } = await hcloud<{ server: Server }>(token, `/servers/${serverId}`)
    if (server.labels['managed-by'] !== MANAGED_BY['managed-by']) throw new BadRequest('That server was not created by this wizard.')
    const selector = ownedSelector(server.labels[LABEL_KEY] ?? server.name)
    const firewalls = await listAll<{ id: number }>(token, `/firewalls?label_selector=${selector}`, 'firewalls')
    if (firewalls.length === 0) throw new BadRequest('No firewall found for that server.')
    const rules = openFrom
      ? [{ direction: 'in', protocol: 'tcp', port: SSH_PORT, source_ips: [openFrom], description: 'SSH from the admin' }]
      : []
    for (const fw of firewalls) {
      await hcloud(token, `/firewalls/${fw.id}/actions/set_rules`, { method: 'POST', body: { rules } })
    }
    return { sshOpen: Boolean(openFrom) }
  })
}
