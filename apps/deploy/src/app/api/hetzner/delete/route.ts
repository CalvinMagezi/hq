import { BadRequest, route } from '~/lib/api'
import { hcloud, LABEL_KEY, listAll, MANAGED_BY, ownedSelector } from '~/lib/hetzner'
import { isId } from '~/lib/validate'

const ACTION_POLL_MS = 1_500
const ACTION_POLL_TRIES = 10

interface Server { id: number; name: string; labels: Record<string, string> }

async function waitForAction(token: string, id: number): Promise<boolean> {
  for (let i = 0; i < ACTION_POLL_TRIES; i++) {
    const { action } = await hcloud<{ action: { status: string } }>(token, `/actions/${id}`)
    if (action.status === 'success') return true
    if (action.status === 'error') return false
    await new Promise((r) => setTimeout(r, ACTION_POLL_MS))
  }
  return false
}

/**
 * Deletes a server this wizard created, then its firewall. The caller must type the server's name,
 * and servers without the wizard's label are refused, so a stray id cannot delete anything else.
 */
export async function POST(req: Request) {
  return route<{ token: string; serverId: number; confirmName: string }>(req, async ({ token, serverId, confirmName }) => {
    if (!isId(serverId)) throw new BadRequest('Unknown server.')
    const { server } = await hcloud<{ server: Server }>(token, `/servers/${serverId}`)
    if (server.labels['managed-by'] !== MANAGED_BY['managed-by']) throw new BadRequest('That server was not created by this wizard.')
    if (confirmName !== server.name) throw new BadRequest('Type the server name exactly to confirm.')
    const del = await hcloud<{ action: { id: number } }>(token, `/servers/${serverId}`, { method: 'DELETE' })
    const gone = await waitForAction(token, del.action.id).catch(() => false)
    if (!gone) return { deleted: false, firewallsRemoved: false }
    const owner = ownedSelector(server.labels[LABEL_KEY] ?? server.name)
    const firewalls = await listAll<{ id: number }>(token, `/firewalls?label_selector=${owner}`, 'firewalls')
    const keys = await listAll<{ id: number }>(token, `/ssh_keys?label_selector=${owner}`, 'ssh_keys')
    const removed = await Promise.all([
      ...firewalls.map((f) => hcloud(token, `/firewalls/${f.id}`, { method: 'DELETE' }).then(() => true, () => false)),
      ...keys.map((k) => hcloud(token, `/ssh_keys/${k.id}`, { method: 'DELETE' }).then(() => true, () => false)),
    ])
    return { deleted: true, firewallsRemoved: removed.every(Boolean) }
  })
}
