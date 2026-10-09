import { route } from '~/lib/api'
import { listAll, MANAGED_SELECTOR } from '~/lib/hetzner'

interface Location { name: string; city: string; country: string }
interface SshKey { id: number; name: string; fingerprint: string }
interface ServerType {
  id: number
  name: string
  description: string
  cores: number
  memory: number
  disk: number
  architecture: string
  deprecated?: boolean
  locations?: { name: string; deprecation: unknown }[]
  prices: { location: string; price_monthly: { gross: string } }[]
}
interface Server { id: number; name: string; status: string; public_net: { ipv4: { ip: string } | null } }

const MIN_MEMORY_GB = 2

/** Hetzner reports retirement per location; the old top-level `deprecated` flag is honoured too. */
function offeredAt(t: ServerType, location: string): boolean {
  const entry = t.locations?.find((l) => l.name === location)
  return !t.deprecated && !entry?.deprecation
}

/** What the wizard form needs: places, sizes with prices, the project's SSH keys, and servers this wizard made. */
export async function POST(req: Request) {
  return route<{ token: string }>(req, async ({ token }) => {
    const [locations, types, keys, servers] = await Promise.all([
      listAll<Location>(token, '/locations', 'locations'),
      listAll<ServerType>(token, '/server_types', 'server_types'),
      listAll<SshKey>(token, '/ssh_keys', 'ssh_keys'),
      listAll<Server>(token, `/servers?label_selector=${encodeURIComponent(MANAGED_SELECTOR)}`, 'servers'),
    ])
    const forwarded = req.headers.get('x-forwarded-for')?.split(',')[0]?.trim() ?? req.headers.get('x-real-ip') ?? ''
    return {
      callerIp: forwarded,
      locations: locations.map((l) => ({ name: l.name, label: `${l.city}, ${l.country}` })),
      serverTypes: types
        .filter((t) => !t.deprecated && t.memory >= MIN_MEMORY_GB)
        .map((t) => ({
          name: t.name,
          description: t.description,
          cores: t.cores,
          memoryGb: t.memory,
          diskGb: t.disk,
          arch: t.architecture,
          monthlyGross: Object.fromEntries(t.prices.filter((p) => offeredAt(t, p.location)).map((p) => [p.location, p.price_monthly.gross])),
        })),
      sshKeys: keys.map((k) => ({ id: k.id, name: k.name })),
      existing: servers.map((s) => ({ id: s.id, name: s.name, status: s.status, ip: s.public_net.ipv4?.ip ?? null })),
    }
  })
}
