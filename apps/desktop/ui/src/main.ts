// The control window: three screens over the Rust side. Everything the other device or HQ tells us
// (names, ids, notes) goes into the page with textContent, never as markup.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { t, tn } from '../../../hq-web/src/strings'
import { clock as clockOf, describe, sizeText, stopIsRisky, type Report } from './thisComputer'

interface Settings {
  start_host_on_launch: boolean
  proxy_port: number
  bridge_port: number
  relay: string
  device_name: string
  hq_path: string | null
  look_for_helper_updates: boolean
  try_it_dir: string | null
  usual_start_secs: number | null
  recent_start_secs: number[]
  connection_sheet_day: string | null
  bundle_rename_handled: boolean
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
type Need = 'blocked' | 'done' | 'working' | 'idle' | 'unknown' | 'exited'
interface Agent {
  name: string
  agent: string | null
  need: Need
  title: string
  cwd: string
  quiet_secs: number
}
interface Agents {
  reachable: boolean
  agents: Agent[]
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

// A notice stays until the person closes it: it never times out.
function toast(text: string) {
  $('toast-text').textContent = text
  $('toast').hidden = false
}
$('toast-close').textContent = t('close')
$('toast-close').addEventListener('click', () => ($('toast').hidden = true))

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
  toast(t('copy_details_copied'))
}

// Tabs
const TABS = ['computer', 'devices', 'helpers'] as const
function showTab(name: (typeof TABS)[number]) {
  for (const tab of TABS) {
    $(`tab-${tab}`).hidden = tab !== name
    document
      .querySelector(`nav [data-tab="${tab}"]`)
      ?.setAttribute('aria-selected', String(tab === name))
  }
  // Reading Claude runs its program, so it happens on opening the tab and after a change.
  if (name === 'helpers') void guarded('claude-error', refreshClaude)
  void refresh()
}
document.querySelectorAll<HTMLButtonElement>('nav [data-tab]').forEach((b) =>
  b.addEventListener('click', () => showTab(b.dataset.tab as (typeof TABS)[number])),
)
document.querySelector('[data-tab="computer"]')!.firstChild!.textContent = t('hub_tab_computer')
document.querySelector('[data-tab="devices"]')!.firstChild!.textContent = t('hub_tab_devices')
document.querySelector('[data-tab="helpers"]')!.firstChild!.textContent = `${t('hub_tab_helpers')} `

// This computer
const HUB_WEB_SERVICE = 'web'
let report: Report | undefined
let agentsView: Agents | undefined

async function heldHubForward(): Promise<Forward | undefined> {
  const pairing = await invoke<Pairing>('pairing_snapshot')
  return pairing.forwards.find((f) => f.service === HUB_WEB_SERVICE && f.running_elsewhere)
}

async function hubName(): Promise<string | null> {
  const pairing = await invoke<Pairing>('pairing_snapshot')
  const forward = pairing.forwards.find((f) => f.service === HUB_WEB_SERVICE)
  return pairing.peers.find((p) => p.short_id === forward?.peer_short)?.name ?? null
}

function show(id: string, on: boolean) {
  $(id).hidden = !on
}

function shortPath(path: string): string {
  return path.replace(/^\/Users\/[^/]+/, '~')
}

function tone(el: HTMLElement, value: string) {
  el.className = `a-state ${value}`
}

async function refreshHost() {
  const [r, held, hub, settings] = await Promise.all([
    invoke<Report>('host_status'),
    heldHubForward(),
    hubName(),
    invoke<Settings>('settings_get'),
  ])
  report = r
  const v = describe(r, held !== undefined, hub)
  $('hub-title').textContent = v.title
  const state = $('hub-state')
  state.textContent = v.state
  tone(state, v.tone)
  $('hub-card').className = `a-card${v.tone === 'needs' ? ' needs' : ''}`
  $('hub-lines').replaceChildren(...v.lines.map((l) => el('p', l)))
  const answering = r.state === 'running' || r.state === 'external'
  const ours = r.state === 'running' || r.state === 'starting'
  const going = v.kind === 'on' || v.kind === 'phone_on'
  const noProgram = v.kind === 'not_found'
  show('host-start', !ours && r.state !== 'external' && !noProgram && v.kind !== 'locked' && v.kind !== 'port_held')
  $('host-start').textContent = r.serves_hub ? t('hub_phone_start') : t('hub_start')
  show('open-hq', !noProgram && v.kind !== 'locked')
  $<HTMLButtonElement>('open-hq').disabled = !answering || v.openWhy !== null
  $('hub-open-why').textContent = noProgram || v.kind === 'locked' ? '' : (v.openWhy ?? '')
  show('host-stop', ours && !noProgram)
  $('host-stop').textContent = r.serves_hub ? t('hub_phone_stop') : t('hub_stop_act')
  show('host-restart', v.kind === 'slow')
  show('choose-program', noProgram)
  show('look-again', noProgram || v.kind === 'port_held')
  show('allow-helpers', v.kind === 'locked')
  show('copy-details', v.kind === 'stopped')
  show('paste-row', noProgram)
  if (v.kind === 'not_found') {
    $('hub-lines').append(el('p', t('hub_looked_in'), 'hub-why'))
  }
  show('helpers-row', going && r.helpers.working.length + r.helpers.needs_you.length > 0)
  $('helpers-row-title').textContent = tn('hub_helpers_row', r.helpers.working.length)
  const needs = r.helpers.needs_you.length
  show('helpers-row-state', needs > 0)
  $('helpers-row-state').textContent = needs > 0 ? t('hub_helpers_row_needs', { count: needs }) : ''
  show('first-signin', r.hub_web)
  $('first-signin-title').textContent = t('hub_first_signin_title')
  $('first-signin-note').textContent = t('hub_first_signin_note')
  $('launch-title').textContent = t('hub_launch_title')
  $('launch-note').textContent = `${t('hub_launch_row')}. ${t('hub_launch_note')}`
  $('set-start-on-launch').setAttribute('aria-checked', String(settings.start_host_on_launch))
  $('program-title').textContent = t('hub_program_title')
  $('program-value').textContent = r.program
    ? r.program.version
      ? t('hub_program_value', { path: shortPath(r.program.path), version: r.program.version.replace(/^hq /, '') })
      : t('hub_program_value_unknown', { path: shortPath(r.program.path) })
    : t('hub_not_found_state')
  $('change-program').textContent = t('hub_change')
  $('log-title').textContent = t('hub_log_title')
  $('log-value').textContent = r.last_run
    ? t('hub_log_last_run', { from: clockOf(r.last_run.started_at), to: clockOf(r.last_run.ended_at) })
    : t('hub_log_none')
  $('open-log').textContent = t('hub_log_open')
  $('paste-label').textContent = t('hub_paste_path')
  $('tab-need').textContent = String(needs)
  show('tab-need', needs > 0)
  if (r.almost_full && r.state === 'stopped' && r.free_bytes !== null) {
    $('hub-lines').append(el('p', `${t('reason_almost_full')} ${t('hub_almost_full_fix', { size: sizeText(200 * 1024 * 1024 - r.free_bytes) })}`))
  }
}

const NEED_LABEL: Record<Need, string> = {
  blocked: t('helper_state_needs'),
  done: t('helper_state_finished'),
  working: t('helper_state_working'),
  idle: t('helper_state_ready'),
  unknown: t('helper_state_working'),
  exited: t('helper_state_stopped'),
}
const SECS_PER_MIN = 60

function quiet(seconds: number): string {
  return seconds < SECS_PER_MIN ? '' : `quiet ${Math.floor(seconds / SECS_PER_MIN)} min`
}

async function refreshAgents() {
  const view = await invoke<Agents>('host_agents')
  agentsView = view
  const list = $('agents')
  list.replaceChildren(
    ...view.agents.map((a) => {
      const li = el('li')
      const who = el('span')
      who.append(el('strong', a.name), el('span', ` ${a.agent ?? ''} ${a.title || a.cwd}`, 'muted'))
      const state = el('span', '', 'row')
      state.append(el('span', quiet(a.quiet_secs), 'muted'), el('span', NEED_LABEL[a.need], a.need === 'blocked' ? 'a-state needs' : 'a-state'))
      li.append(who, state)
      return li
    }),
  )
  $('agents-empty').textContent = !view.reachable ? t('hub_log_elsewhere') : view.agents.length === 0 ? t('hub_log_none') : ''
}

// One dialog serves every confirmation. Escape and the quiet button both keep things as they are.
function confirmDialog(opts: { title: string; body: string; keep: string; act: string; danger: boolean }): Promise<boolean> {
  return new Promise((resolve) => {
    $('dlg-title').textContent = opts.title
    $('dlg-body').textContent = opts.body
    const keep = $<HTMLButtonElement>('dlg-keep')
    const act = $<HTMLButtonElement>('dlg-act')
    keep.textContent = opts.keep
    act.textContent = opts.act
    act.className = `a-btn${opts.danger ? ' danger' : ''}`
    show('scrim', true)
    show('dlg', true)
    keep.focus()
    const done = (value: boolean) => {
      show('scrim', false)
      show('dlg', false)
      keep.removeEventListener('click', onKeep)
      act.removeEventListener('click', onAct)
      document.removeEventListener('keydown', onKey)
      resolve(value)
    }
    const onKeep = () => done(false)
    const onAct = () => done(true)
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') done(false)
    }
    keep.addEventListener('click', onKeep)
    act.addEventListener('click', onAct)
    document.addEventListener('keydown', onKey)
  })
}

async function showLog() {
  const log = await invoke<{ path: string; lines: string[]; version: string | null; exit_code: number | null }>('host_log')
  $('sheet-title').textContent = t('hub_log_sheet_title')
  $('sheet-note').textContent = `${t('hub_log_hidden_note')} ${t('hub_log_path', { path: shortPath(log.path) })}`
  $('sheet-log').textContent = log.lines.length ? log.lines.join('\n') : t('hub_log_none')
  $('sheet-close').textContent = t('close')
  show('scrim', true)
  show('sheet', true)
}
function closeSheet() {
  show('scrim', false)
  show('sheet', false)
}
$('sheet-close').addEventListener('click', closeSheet)
$('scrim').addEventListener('click', closeSheet)

async function copyDetails() {
  if (!report) return
  const log = await invoke<{ path: string; lines: string[]; version: string | null; exit_code: number | null }>('host_log')
  const at = report.exited_at ? clockOf(report.exited_at) : ''
  const head = [
    log.exit_code === null ? t('hub_exit_no_code', { time: at }) : t('hub_exit_code', { code: log.exit_code, time: at }),
    t('hub_log_path', { path: shortPath(log.path) }),
    log.version ?? '',
  ].filter(Boolean)
  await copy([...head, '', ...log.lines].join('\n'))
}

async function pickProgram(path?: string) {
  $('program-error').textContent = ''
  try {
    await invoke('pick_program', { path: path ?? null })
  } catch (e) {
    const code = message(e)
    if (code === 'cancelled') return
    $('program-error').textContent =
      code === 'refused' ? t('hub_refused_program') : code === 'not_hq' ? t('hub_not_hq_program') : t('hub_missing_program')
  }
  await refreshHost()
}

$('host-start').addEventListener('click', () =>
  guarded('host-error', async () => {
    await invoke('host_start', { restart: false })
    await refreshHost()
  }),
)
$('host-stop').addEventListener('click', () =>
  guarded('host-error', async () => {
    if (!report) return
    const risky = stopIsRisky(report)
    const names = report.helpers.working.join(', ')
    const phone = report.serves_hub
    const yes = await confirmDialog({
      title: phone ? t('hub_stop_here_title') : t('hub_stop_title'),
      body: phone ? t('hub_stop_here_body') : risky ? t('hub_stop_working', { names }) : t('hub_stop_idle'),
      keep: phone ? t('hub_stop_here_keep') : t('stop_helper_keep'),
      act: phone ? t('hub_phone_stop') : t('hub_stop_act'),
      danger: risky,
    })
    if (!yes) return
    await invoke('host_stop')
    await refreshHost()
  }),
)
$('host-restart').addEventListener('click', () =>
  guarded('host-error', async () => {
    await invoke('host_stop')
    await invoke('host_start', { restart: true })
    await refreshHost()
  }),
)
$('look-again').addEventListener('click', () =>
  guarded('host-error', async () => {
    // Forwards are opened when pairing starts, so a restart tries the port again.
    await invoke('pairing_stop')
    await invoke('pairing_start')
    await refreshHost()
  }),
)
$('choose-program').addEventListener('click', () => void pickProgram())
$('change-program').addEventListener('click', () => void pickProgram())
$('paste-use').addEventListener('click', () => void pickProgram($<HTMLInputElement>('paste-path').value.trim()))
$('open-log').addEventListener('click', () => guarded('host-error', showLog))
$('copy-details').addEventListener('click', () => guarded('host-error', copyDetails))
$('allow-helpers').addEventListener('click', () =>
  guarded('host-error', async () => {
    const yes = await confirmDialog({
      title: t('hub_allow_title'),
      body: t('hub_allow_body'),
      keep: t('hub_allow_keep'),
      act: t('hub_allow_confirm'),
      danger: false,
    })
    if (!yes) return
    await invoke('host_unlock')
    toast(t('hub_allowed_done'))
    await refreshHost()
  }),
)
$('open-hq').addEventListener('click', () => guarded('host-error', () => invoke('open_main_window', { path: '/' })))
$('set-start-on-launch').addEventListener('click', () =>
  guarded('host-error', async () => {
    const settings = await invoke<Settings>('settings_get')
    settings.start_host_on_launch = !settings.start_host_on_launch
    await invoke('settings_save', { settings })
    await refreshHost()
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
        const done = await invoke<{ revoked: boolean; claude_warning: string | null }>(
          'pairing_revoke',
          { id: peer.id },
        )
        toast('Revoked. Its open connections were closed.')
        // The revoke went through; Claude's leftover entry is shown where errors are.
        if (done.claude_warning) $('devices-error').textContent = done.claude_warning
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
    const where = f.running_elsewhere
      ? 'Port in use. Another program holds this port. HQ Hub won\'t send your sign-in there.'
      : 'listening'
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
  const visible = TABS.find((tab) => !$(`tab-${tab}`).hidden)
  const computer = async () => {
    await refreshHost()
    await refreshAgents()
  }
  const work = { computer, devices: refreshDevices, helpers: refreshAgents }[visible ?? 'computer']
  try {
    await work()
  } catch (e) {
    toast(message(e))
  }
}

const POLL_MS = 3000
window.setInterval(() => void refresh(), POLL_MS)
void listen('state-changed', () => void refresh())
void listen<string[]>('claude-entries-moved', (e) => {
  $('claude-moved').textContent = t('hub_claude_moved', { names: e.payload.join(', ') })
  show('claude-moved', true)
})
void invoke<string | null>('startup_warning').then((w) => {
  if (w) $('host-error').textContent = w
})
void refresh()
