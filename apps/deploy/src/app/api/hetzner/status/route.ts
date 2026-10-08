import { BadRequest, route } from '~/lib/api'
import { hcloud, MANAGED_BY } from '~/lib/hetzner'
import { isId } from '~/lib/validate'

interface Server {
  id: number
  name: string
  status: string
  labels: Record<string, string>
  public_net: { ipv4: { ip: string } | null }
}

/** Hetzner's view of a server this wizard made. HQ itself is tailnet-only, so readiness is not visible from here. */
export async function POST(req: Request) {
  return route<{ token: string; serverId: number }>(req, async ({ token, serverId }) => {
    if (!isId(serverId)) throw new BadRequest('Unknown server.')
    const { server } = await hcloud<{ server: Server }>(token, `/servers/${serverId}`)
    if (server.labels['managed-by'] !== MANAGED_BY['managed-by']) throw new BadRequest('That server was not created by this wizard.')
    return { id: server.id, name: server.name, status: server.status, ip: server.public_net.ipv4?.ip ?? null }
  })
}
