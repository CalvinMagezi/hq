import { BadRequest, route } from '~/lib/api'
import { HetznerError, hcloud, LABEL_KEY, listAll, MANAGED_BY, ownedSelector } from '~/lib/hetzner'
import { isId, isServerName } from '~/lib/validate'

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

async function fetchServer(token: string, id: number): Promise<Server | null> {
  try {
    return (await hcloud<{ server: Server }>(token, `/servers/${id}`)).server
  } catch (e) {
    if (e instanceof HetznerError && e.status === 404) return null
    throw e
  }
}

/** Removes the firewall and SSH key the wizard made for `name`, unless a server with that name is still running. */
async function cleanup(token: string, name: string): Promise<boolean> {
  const owner = ownedSelector(name)
  const live = await listAll<{ id: number }>(token, `/servers?label_selector=${owner}`, 'servers')
  if (live.length > 0) return false
  const firewalls = await listAll<{ id: number }>(token, `/firewalls?label_selector=${owner}`, 'firewalls')
  const keys = await listAll<{ id: number }>(token, `/ssh_keys?label_selector=${owner}`, 'ssh_keys')
  const removed = await Promise.all([
    ...firewalls.map((f) => hcloud(token, `/firewalls/${f.id}`, { method: 'DELETE' }).then(() => true, () => false)),
    ...keys.map((k) => hcloud(token, `/ssh_keys/${k.id}`, { method: 'DELETE' }).then(() => true, () => false)),
  ])
  return removed.every(Boolean)
}

/**
 * Deletes a server this wizard created, then its firewall and SSH key. The caller must type the server's
 * name, and servers without the wizard's label are refused, so a stray id cannot delete anything else.
 * Hetzner finishes deleting after this request may have given up waiting, so calling again with the same
 * id is safe: once the server is gone it only finishes the cleanup.
 */
export async function POST(req: Request) {
  return route<{ token: string; serverId: number; confirmName: string }>(req, async ({ token, serverId, confirmName }) => {
    if (!isId(serverId)) throw new BadRequest('Unknown server.')
    if (!isServerName(confirmName)) throw new BadRequest('Type the server name exactly to confirm.')
    const server = await fetchServer(token, serverId)
    if (!server) return { deleted: true, firewallsRemoved: await cleanup(token, confirmName) }
    if (server.labels['managed-by'] !== MANAGED_BY['managed-by']) throw new BadRequest('That server was not created by this wizard.')
    if (confirmName !== server.name) throw new BadRequest('Type the server name exactly to confirm.')
    const del = await hcloud<{ action: { id: number } }>(token, `/servers/${serverId}`, { method: 'DELETE' })
    const gone = await waitForAction(token, del.action.id).catch(() => false)
    if (!gone) return { deleted: false, pending: true }
    return { deleted: true, firewallsRemoved: await cleanup(token, server.labels[LABEL_KEY] ?? server.name) }
  })
}
