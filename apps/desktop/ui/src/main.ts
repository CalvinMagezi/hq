// The control window: three screens over the Rust side. Everything the other device or HQ tells us
// (names, ids, notes) goes into the page with textContent, never as markup.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'

type HostStatus =
  | { state: 'stopped' }
  | { state: 'starting'; pid: number }
  | { state: 'running'; pid: number }
  | { state: 'external' }
  | { state: 'exited'; code: number | null }

interface Settings {
  start_host_on_launch: boolean
  proxy_port: number
  bridge_port: number
  relay: string
  device_name: string
  hq_path: string | null
}

interface Qr {
  size: number
  rows: string[]
}
interface Ticket {
  link: string
  qr: Qr | null
  expires_at: number
}
interface Pending {
  id: number
  name: string
  joiner_id: string
  joiner_short: string
  expires_at: number
  tries_left: number
}
interface Peer {
  id: string
  short_id: string
  name: string
  paired_at: number
  last_seen_at: number | null
}
interface Forward {
  service: string
  peer_short: string
  listen: string
  running_elsewhere: boolean
}
interface Pairing {
  running: boolean
  host: { id: string; short_id: string; name: string; addrs: string[] } | null
  services: string[]
  forwards: Forward[]
  ticket: Ticket | null
  pending: Pending[]
  peers: Peer[]
  peers_error: string | null
}
interface ClaudeHub {
  name: string
  lines: [string, string]
  added: boolean
  failed: boolean
}
type Submitted =
  | { result: 'accepted' | 'declined' | 'too_late' | 'not_digits' | 'unknown' }
  | { result: 'wrong_digits'; tries_left: number }

const $ = <T extends HTMLElement>(id: string): T => {
  const node = document.getElementById(id)
  if (!node) throw new Error(`missing #${id}`)
  return node as T
}

function el<K extends keyof HTMLElementTagNameMap>(
  tag: K,
  text = '',
  className = '',
): HTMLElementTagNameMap[K] {
  const node = document.createElement(tag)
  if (text) node.textContent = text
  if (className) node.className = className
  return node
}

function button(label: string, onClick: () => void): HTMLButtonElement {
  const b = el('button', label)
  b.type = 'button'
  b.addEventListener('click', onClick)
  return b
}

function message(e: unknown): string {
  return typeof e === 'string' ? e : e instanceof Error ? e.message : 'That did not work.'
}

let toastTimer: number | undefined
function toast(text: string) {
  $('toast').textContent = text
  window.clearTimeout(toastTimer)
  toastTimer = window.setTimeout(() => ($('toast').textContent = ''), 3000)
}

async function guarded(errorId: string, work: () => Promise<void>) {
  try {
    $(errorId).textContent = ''
    await work()
  } catch (e) {
    $(errorId).textContent = message(e)
  }
}

function clock(seconds: number): string {
  return new Date(seconds * 1000).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })
}

async function copy(text: string) {
  await navigator.clipboard.writeText(text)
  toast('Copied')
}

// Tabs
const TABS = ['host', 'devices', 'claude'] as const
function showTab(name: (typeof TABS)[number]) {
  for (const tab of TABS) {
    $(`tab-${tab}`).hidden = tab !== name
    document
      .querySelector(`nav [data-tab="${tab}"]`)
      ?.setAttribute('aria-selected', String(tab === name))
  }
  // Reading Claude runs its program, so it happens on opening the tab and after a change.
  if (name === 'claude') void guarded('claude-error', refreshClaude)
  void refresh()
}
document.querySelectorAll<HTMLButtonElement>('nav [data-tab]').forEach((b) =>
  b.addEventListener('click', () => showTab(b.dataset.tab as (typeof TABS)[number])),
)

// Host
const HOST_LABEL: Record<HostStatus['state'], string> = {
  stopped: 'Stopped',
  starting: 'Starting',
  running: 'Running',
  external: 'Running elsewhere',
  exited: 'Exited',
}

async function refreshHost() {
  const status = await invoke<HostStatus>('host_status')
  $('host-state').textContent = HOST_LABEL[status.state]
  let detail = ''
  if (status.state === 'external') detail = 'Started outside this app, so this app will not stop it.'
  if (status.state === 'exited') detail = `Ended by itself (code ${status.code ?? 'unknown'}). See ~/.hq/desktop/host.log.`
  if (status.state === 'running' || status.state === 'starting') detail = `Started by this app (process ${status.pid}).`
  $('host-detail').textContent = detail
  const ours = status.state === 'running' || status.state === 'starting'
  const answering = ours || status.state === 'external'
  $<HTMLButtonElement>('host-start').disabled = answering
  $<HTMLButtonElement>('host-stop').disabled = !ours
  $<HTMLButtonElement>('open-hq').disabled = !answering
  const settings = await invoke<Settings>('settings_get')
  $<HTMLInputElement>('set-start-on-launch').checked = settings.start_host_on_launch
}

$('host-start').addEventListener('click', () =>
  guarded('host-error', async () => {
    await invoke('host_start')
    await refreshHost()
  }),
)
$('host-stop').addEventListener('click', () =>
  guarded('host-error', async () => {
    await invoke('host_stop')
    await refreshHost()
  }),
)
$('open-hq').addEventListener('click', () => guarded('host-error', () => invoke('open_main_window')))
$('set-start-on-launch').addEventListener('change', (e) =>
  guarded('host-error', async () => {
    const settings = await invoke<Settings>('settings_get')
    settings.start_host_on_launch = (e.target as HTMLInputElement).checked
    await invoke('settings_save', { settings })
  }),
)

// Devices
function drawQr(canvas: HTMLCanvasElement, qr: Qr) {
  const quiet = 2
  const cells = qr.size + quiet * 2
  const scale = Math.max(1, Math.floor(canvas.width / cells))
  canvas.width = cells * scale
  canvas.height = cells * scale
  const ctx = canvas.getContext('2d')
  if (!ctx) return
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--qr-light')
  ctx.fillRect(0, 0, canvas.width, canvas.height)
  ctx.fillStyle = getComputedStyle(document.documentElement).getPropertyValue('--qr-dark')
  qr.rows.forEach((row, y) =>
    [...row].forEach((cell, x) => {
      if (cell === '1') ctx.fillRect((x + quiet) * scale, (y + quiet) * scale, scale, scale)
    }),
  )
}

// The screen refreshes every few seconds; a list is rebuilt only when it changed, so a half-typed
// digit or an armed "Confirm revoke" is not wiped under the person's hands.
const rendered = new Map<string, string>()
function unchanged(key: string, signature: unknown): boolean {
  const next = JSON.stringify(signature)
  if (rendered.get(key) === next) return true
  rendered.set(key, next)
  return false
}

function renderPending(list: Pending[]) {
  if (unchanged('pending', list.map((p) => [p.id, p.tries_left]))) return
  const host = $('pending')
  host.replaceChildren()
  for (const p of list) {
    const card = el('div', '', 'card')
    card.append(el('p', `"${p.name}" wants to pair (id ${p.joiner_short}).`))
    card.append(
      el('p', 'Type the six digits shown on that device. It pairs only if they match.', 'muted'),
    )
    const input = el('input')
    input.type = 'text'
    input.inputMode = 'numeric'
    input.autocomplete = 'off'
    input.maxLength = 8
    input.setAttribute('aria-label', 'Six digits shown on the other device')
    const send = async () => {
      const outcome = await invoke<Submitted>('pairing_submit', { id: p.id, typed: input.value })
      input.value = ''
      if (outcome.result === 'wrong_digits') toast(`Not the right digits. ${outcome.tries_left} tries left.`)
      else if (outcome.result === 'accepted') toast('Accepted. It shows in the list once the other device finishes.')
      else if (outcome.result === 'declined') toast('Too many wrong tries: the device was declined.')
      else if (outcome.result === 'not_digits') toast('Type the six digits.')
      else toast('That device is no longer waiting.')
      await refreshDevices()
    }
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void guarded('devices-error', send)
    })
    const row = el('p', '', 'row')
    row.append(
      input,
      button('Pair', () => void guarded('devices-error', send)),
      button('Decline', () =>
        void guarded('devices-error', async () => {
          await invoke('pairing_decline', { id: p.id })
          await refreshDevices()
        }),
      ),
    )
    card.append(row, el('p', `${p.tries_left} tries left. Waiting until ${clock(p.expires_at)}.`, 'muted'))
    host.append(card)
  }
}

function renderPeers(peers: Peer[]) {
  if (unchanged('peers', peers.map((p) => [p.id, p.name]))) return
  const list = $('peers')
  list.replaceChildren()
  $('peers-empty').hidden = peers.length > 0
  for (const peer of peers) {
    const item = el('li')
    const label = el('span')
    label.append(el('strong', peer.name), el('span', `  id ${peer.short_id}`, 'muted'))
    let armed = false
    const revoke = button('Revoke', () => {
      if (!armed) {
        armed = true
        revoke.textContent = 'Confirm revoke'
        return
      }
      void guarded('devices-error', async () => {
        await invoke('pairing_revoke', { id: peer.id })
        toast('Revoked. Its open connections were closed.')
        await refreshDevices()
      })
    })
    item.append(label, revoke)
    list.append(item)
  }
}

function renderForwards(state: Pairing) {
  if (unchanged('forwards', [state.services, state.forwards])) return
  const list = $('forwards')
  list.replaceChildren()
  for (const name of state.services) list.append(el('li', `This computer offers its ${name} service to paired devices.`))
  for (const f of state.forwards) {
    const where = f.running_elsewhere ? 'running elsewhere' : 'listening'
    list.append(el('li', `The ${f.service} service of device ${f.peer_short} is on ${f.listen} (${where}).`))
  }
  $('forwards-block').hidden = list.children.length === 0
}

async function refreshDevices() {
  const state = await invoke<Pairing>('pairing_snapshot')
  $('devices-off').hidden = state.running
  $('devices-on-block').hidden = !state.running
  if (state.host) {
    $('this-name').textContent = state.host.name
    $('this-id').textContent = state.host.short_id
  }
  const ticket = $('ticket')
  ticket.hidden = !state.ticket
  if (state.ticket) {
    $('ticket-link').textContent = state.ticket.link
    $('ticket-expires').textContent = `at ${clock(state.ticket.expires_at)}`
    if (state.ticket.qr) drawQr($<HTMLCanvasElement>('qr'), state.ticket.qr)
  }
  renderForwards(state)
  renderPending(state.pending)
  renderPeers(state.peers)
  if (state.peers_error) $('devices-error').textContent = state.peers_error
}

$('devices-on').addEventListener('click', () =>
  guarded('devices-error', async () => {
    await invoke('pairing_start')
    await refreshDevices()
  }),
)
$('devices-off-btn').addEventListener('click', () =>
  guarded('devices-error', async () => {
    await invoke('pairing_stop')
    await refreshDevices()
  }),
)
$('show-code').addEventListener('click', () =>
  guarded('devices-error', async () => {
    await invoke('pairing_begin')
    await refreshDevices()
  }),
)
$('cancel-ticket').addEventListener('click', () =>
  guarded('devices-error', async () => {
    await invoke('pairing_cancel')
    await refreshDevices()
  }),
)
$('copy-link').addEventListener('click', () => void copy($('ticket-link').textContent ?? ''))

// Claude
function renderClaude(hubs: ClaudeHub[]) {
  const list = $('claude-hubs')
  list.replaceChildren()
  $('claude-empty').hidden = hubs.length > 0
  for (const hub of hubs) {
    const item = el('li')
    const label = el('span')
    label.append(el('strong', hub.name))
    for (const line of hub.lines) label.append(el('br'), el('span', line, 'muted'))
    const change = (add: boolean) =>
      void guarded('claude-error', async () => {
        const done = await invoke<ClaudeHub>('claude_change', { hub: hub.name, add })
        if (done.failed) $('claude-error').textContent = done.lines.join(' ')
        else toast(add ? 'Added. Restart Claude to see it.' : 'Removed.')
        await refreshClaude()
      })
    item.append(label, button(hub.added ? 'Remove' : 'Add', () => change(!hub.added)))
    list.append(item)
  }
}

async function refreshClaude() {
  renderClaude(await invoke<ClaudeHub[]>('claude_status'))
}

// Refresh
async function refresh() {
  const visible = TABS.find((t) => !$(`tab-${t}`).hidden)
  const work = { host: refreshHost, devices: refreshDevices, claude: async () => {} }[visible ?? 'host']
  try {
    await work()
  } catch (e) {
    toast(message(e))
  }
}

const POLL_MS = 3000
window.setInterval(() => void refresh(), POLL_MS)
void listen('state-changed', () => void refresh())
void invoke<string | null>('startup_warning').then((w) => {
  if (w) $('host-error').textContent = w
})
void refresh()
