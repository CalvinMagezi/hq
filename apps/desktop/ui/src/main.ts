// The control window: three screens over the Rust side. Everything the other device or HQ tells us
// (names, ids, notes) goes into the page with textContent, never as markup.
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import { t, tn, type StringKey } from '../../../hq-web/src/strings'
import { agoText, clock as clockOf, describe, sizeText, stopIsRisky, type Report } from './thisComputer'
import { ACTION_KEY, afterTry, grouped, minutesLeft, nothingFound, rowAction, rowLine, shortHome, stateLabel, tone as stateTone, type HelperView } from './helpers'

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
  share_model: boolean
  shared_model: string | null
}

interface ShareModel {
  name: string
  tools: boolean
}
interface Share {
  on: boolean
  model: string | null
  server: string
  reachable: boolean
  models: ShareModel[]
  context_length: number | null
  warning: string | null
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
interface HelpersReport {
  helpers: HelperView[]
  hubs: ClaudeHub[]
  host_up: boolean
  locked: boolean
  looks_for_updates: boolean
  try_it_dir: string
}
interface TryOutcome {
  state: HelperView['state']
  shown: string
  link_ready: boolean
}
type Need = 'blocked' | 'done' | 'working' | 'idle' | 'unknown' | 'exited'
interface Agent {
  name: string
  agent: string | null
  need: Need
  title: string
  cwd: string
  quiet_secs: number
  age_secs: number
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
function toast(text: string, label?: string, onAct?: () => void) {
  $('toast-text').textContent = text
  const act = $<HTMLButtonElement>('toast-act')
  act.hidden = !label
  act.textContent = label ?? ''
  act.onclick = () => {
    $('toast').hidden = true
    onAct?.()
  }
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

async function copy(text: string, said: string = t('copy_details_copied')) {
  await navigator.clipboard.writeText(text)
  toast(said)
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
  // Asking Ollama about every model is slow, so it happens on opening the tab and on request.
  if (name === 'devices') void guarded('share-warning', refreshShare)
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
  // The header's status line says the same, as the phone's top bar does.
  $('hub-status').textContent = v.state
  $('hub-dot').className = `a-dot${v.tone === 'needs' ? ' warn' : v.tone === 'working' ? ' work' : ''}`
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
  $('log-title').textContent = r.state === 'running' ? t('hub_log_started') : t('hub_log_title')
  $('log-value').textContent = r.state === 'external' ? t('hub_log_elsewhere') : r.last_run
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
async function refreshAgents() {
  const view = await invoke<Agents>('host_agents')
  agentsView = view
  const box = $('hh-work')
  box.replaceChildren()
  if (!view.reachable) {
    $('hh-work').append(el('p', t('hh_hq_off'), 'hub-why'))
    return
  }
  if (report?.serves_hub) {
    const phone = card(t('hh_phone_title'))
    phone.append(el('p', t('hh_phone_body', { who: (await hubName()) ?? t('hub_lockdown_who_phone') }), 'hub-why'))
    box.append(phone)
  }
  if (view.agents.length === 0) {
    box.append(el('p', t('hh_none_work'), 'hub-why'))
    return
  }
  const set = el('div', '', 'a-set hub-set')
  for (const a of view.agents) {
    const row = el('div', '', 'a-li')
    const main = el('div', '', 'a-li-main')
    main.append(el('b', a.name), el('span', `${a.agent ?? ''} ${a.title || shortPath(a.cwd)}`.trim()), el('span', t('hh_started', { name: a.agent ?? a.name, ago: agoText(a.age_secs) }), 'muted'))
    row.append(main, el('span', NEED_LABEL[a.need], a.need === 'blocked' ? 'a-state needs' : 'a-state'))
    // The helper's own page is in the HQ window; a pane here has no session id, so it opens Helpers.
    row.classList.add('hub-hit')
    row.tabIndex = 0
    row.setAttribute('role', 'link')
    const go = () => void guarded('host-error', () => invoke('open_main_window', { path: '/helpers' }))
    row.addEventListener('click', go)
    row.addEventListener('keydown', (e) => (e.key === 'Enter' || e.key === ' ') && (e.preventDefault(), go()))
    set.append(row)
  }
  box.append(groupHead(t('hh_group_work')), set)
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
$('choose-program').textContent = t('hub_choose_program')
$('look-again').textContent = t('hub_look_again')
$('allow-helpers').textContent = t('hub_allow_act')
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
// digit is not wiped under the person's hands.
const rendered = new Map<string, string>()
function unchanged(key: string, signature: unknown): boolean {
  const next = JSON.stringify(signature)
  if (rendered.get(key) === next) return true
  rendered.set(key, next)
  return false
}

// What the person last typed for each waiting device stays on screen until the device is gone.
const outcomes = new Map<number, string>()

function renderPending(list: Pending[]) {
  if (unchanged('pending', list.map((p) => [p.id, p.tries_left, outcomes.get(p.id)]))) return
  const host = $('pending')
  host.replaceChildren()
  for (const p of list) {
    const card = el('div', '', 'a-card')
    card.append(el('b', t('dev_wants', { name: p.name, id: p.joiner_short })))
    card.append(el('p', t('dev_wants_note'), 'hub-why'))
    const input = el('input', '', 'a-field')
    input.type = 'text'
    input.inputMode = 'numeric'
    input.autocomplete = 'off'
    input.maxLength = 8
    input.setAttribute('aria-label', t('dev_digits_label', { name: p.name }))
    const note = el('p', outcomes.get(p.id) ?? '', 'hub-why')
    note.setAttribute('role', 'status')
    const send = async () => {
      const outcome = await invoke<Submitted>('pairing_submit', { id: p.id, typed: input.value })
      input.value = ''
      const line =
        outcome.result === 'wrong_digits'
          ? outcome.tries_left === 1
            ? t('dev_wrong_one')
            : t('dev_wrong', { n: outcome.tries_left })
          : outcome.result === 'accepted'
            ? t('dev_accepted')
            : outcome.result === 'declined'
              ? t('dev_declined')
              : outcome.result === 'not_digits'
                ? t('dev_not_digits')
                : t('dev_gone')
      outcomes.set(p.id, line)
      note.textContent = line
      await refreshDevices()
    }
    input.addEventListener('keydown', (e) => {
      if (e.key === 'Enter') void guarded('devices-error', send)
    })
    const row = el('div', '', 'row')
    const decline = button(t('dev_decline'), () =>
      void guarded('devices-error', async () => {
        await invoke('pairing_decline', { id: p.id })
        await refreshDevices()
      }),
    )
    decline.className = 'a-btn compact secondary'
    const pair = button(t('dev_pair'), () => void guarded('devices-error', send))
    pair.className = 'a-btn compact'
    row.append(decline, input, pair)
    card.append(row, note, el('p', t('dev_wants_wait', { time: clock(p.expires_at) }), 'hub-why'))
    host.append(card)
  }
}

let justPaired = ''
let knownPeers: string[] | undefined

function renderPeers(peers: Peer[]) {
  if (knownPeers) {
    const fresh = peers.find((p) => !knownPeers!.includes(p.id))
    if (fresh) justPaired = t('dev_just_paired', { name: fresh.name, id: fresh.short_id })
  }
  knownPeers = peers.map((p) => p.id)
  $('just-paired').textContent = justPaired
  show('just-paired', justPaired !== '')
  if (unchanged('peers', peers.map((p) => [p.id, p.name, p.last_seen_at !== null]))) return
  const list = $('peers')
  list.replaceChildren()
  $('peers-empty').hidden = peers.length > 0
  for (const peer of peers) {
    const item = el('div', '', 'a-li')
    const label = el('div', '', 'a-li-main')
    label.append(
      el('b', peer.name),
      el('span', peer.last_seen_at === null ? t('dev_row_never', { id: peer.short_id }) : t('dev_row', { id: peer.short_id, seen: clock(peer.last_seen_at) })),
    )
    const remove = button(t('dev_remove'), () =>
      void guarded('devices-error', async () => {
        const yes = await confirmDialog({
          title: t('dev_remove_title', { name: peer.name }),
          body: t('dev_remove_body'),
          keep: t('dev_remove_keep'),
          act: t('dev_remove_act'),
          danger: true,
        })
        if (!yes) return
        try {
          const done = await invoke<{ revoked: boolean; claude_warning: string | null }>('pairing_revoke', { id: peer.id })
          justPaired = ''
          if (done.claude_warning) toast(t('dev_removed_warn', { name: peer.name, warning: done.claude_warning }), t('dev_open_helpers'), () => showTab('helpers'))
          else toast(t('dev_removed', { name: peer.name }))
        } catch {
          $('devices-error').textContent = t('dev_remove_off', { name: peer.name })
        }
        await refreshDevices()
      }),
    )
    remove.className = 'a-btn compact quiet'
    item.append(label, remove)
    list.append(item)
  }
}

function sharedRow(title: string, note: string, tag: string, warn = false): HTMLElement {
  const row = el('div', '', 'a-li')
  const label = el('div', '', 'a-li-main')
  label.append(el('b', title), el('span', note))
  row.append(label, el('span', tag, warn ? 'a-state needs' : 'a-state'))
  return row
}

function renderForwards(state: Pairing) {
  if (unchanged('forwards', [state.services, state.forwards])) return
  const list = $('forwards')
  list.replaceChildren()
  for (const name of state.services) {
    if (name === 'host') list.append(sharedRow(t('dev_shared_helpers'), t('dev_shared_helpers_note'), t('dev_shared_tag')))
    else list.append(sharedRow(name, '', t('dev_shared_tag')))
  }
  for (const f of state.forwards) {
    if (f.running_elsewhere) list.append(sharedRow(f.service, t('dev_port_taken_note'), t('dev_port_taken_tag'), true))
    else list.append(sharedRow(t('dev_shared_web'), t('dev_shared_web_note'), t('dev_shared_tag')))
  }
  $('forwards-block').hidden = false
  if (list.children.length === 0) list.append(el('p', t('dev_nothing_shared'), 'hub-why'))
}

function labelDevices() {
  const text: Record<string, StringKey> = {
    'devices-off-title': 'dev_off_title',
    'devices-off-state': 'dev_off_state',
    'devices-off-note': 'dev_off_note',
    'devices-on': 'dev_switch_on',
    'devices-on-title': 'dev_on_title',
    'devices-on-state': 'dev_on_state',
    'show-code': 'dev_show_code',
    'devices-off-btn': 'dev_switch_off',
    'ticket-title': 'dev_code_title',
    'ticket-note': 'dev_code_note',
    'copy-link': 'dev_copy_link',
    'cancel-ticket': 'dev_stop_pairing',
    'ticket-waiting': 'dev_code_waiting',
    'forwards-title': 'dev_shared_title',
    'peers-title': 'dev_paired_title',
    'peers-empty': 'dev_none',
    'relay-title': 'dev_relay_title',
    'relay-note': 'dev_relay_note',
    'relay-state': 'dev_relay_none',
  }
  for (const [id, key] of Object.entries(text)) $(id).textContent = t(key)
}

async function refreshDevices() {
  const state = await invoke<Pairing>('pairing_snapshot')
  $('devices-off').hidden = state.running
  $('devices-on-block').hidden = !state.running
  if (state.host) $('devices-on-note').textContent = t('dev_on_note', { name: state.host.name, id: state.host.short_id })
  const ticket = $('ticket')
  ticket.hidden = !state.ticket
  if (state.ticket) {
    $('ticket-link').textContent = state.ticket.link
    $('ticket-until').textContent = t('dev_code_until', { time: clock(state.ticket.expires_at) })
    if (state.ticket.qr) drawQr($<HTMLCanvasElement>('qr'), state.ticket.qr)
  }
  renderForwards(state)
  renderPending(state.pending)
  renderPeers(state.peers)
  if (state.peers_error) $('devices-error').textContent = state.peers_error
}

$('devices-on').addEventListener('click', () =>
  guarded('devices-error', async () => {
    try {
      await invoke('pairing_start')
    } catch {
      $('devices-error').textContent = t('dev_port_taken')
      return
    }
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
labelDevices()

// Helpers
let helpersView: HelpersReport | undefined
let detailId: string | null = null
let checking = false
let askedFor: string | null = null
let signinSince: number | null = null
let lastTry: { id: string; at: number; outcome: TryOutcome } | undefined
let checkedAt: number | null = null
const everChecked = () => localStorage.getItem('hh-checked') === '1'

function stateChip(s: HelperView['state']): HTMLElement {
  const tone = stateTone(s)
  return el('span', stateLabel(s), `a-state${tone === 'amber' ? ' needs' : tone === 'blue' ? ' working' : ''}`)
}

function groupHead(title: string, note = ''): HTMLElement {
  const box = el('div')
  box.append(el('div', title, 'a-grp'))
  if (note) box.append(el('p', note, 'hub-why'))
  return box
}

function helperRow(h: HelperView): HTMLElement {
  const row = el('div', '', 'a-li')
  const main = el('div', '', 'a-li-main')
  main.append(el('b', h.name), el('span', rowLine(h, h.new_here)))
  const open = el('button', '', 'hub-row-open')
  open.type = 'button'
  open.append(main)
  open.addEventListener('click', () => void openDetail(h.id))
  row.append(open, stateChip(h.state))
  const act = rowAction(h)
  if (act) {
    const b = button(t(ACTION_KEY[act]), () => void openDetail(h.id))
    b.className = 'a-btn compact secondary'
    row.append(b)
  }
  return row
}

function renderHelpersList(r: HelpersReport) {
  const needsFirst = !everChecked() && !checking
  show('hh-first', needsFirst)
  show('hh-lock', r.locked)
  const none = everChecked() && nothingFound(r.helpers) && !checking
  show('hh-none', none)
  show('hh-check-all', everChecked())
  $('hh-checked').textContent = checkedAt ? t('hh_checked', { time: clockOf(Math.floor(checkedAt / 1000)) }) : t('hh_not_checked')
  const pending = r.helpers.filter((h) => h.state === 'checking').length
  const total = r.helpers.filter((h) => h.group !== 'app').length
  $('hh-checking').textContent = pending > 0 ? `${t('hh_checking_head', { n: total, done: total - pending })} ${t('hh_checking_note')}` : ''
  const groups = $('hh-groups')
  groups.replaceChildren()
  if (needsFirst) return
  const g = grouped(r.helpers)
  const set = (title: string, note: string, rows: HelperView[]) => {
    if (!rows.length) return
    const box = el('div')
    box.append(groupHead(title, note))
    const list = el('div', '', 'a-set hub-set')
    list.append(...rows.map(helperRow))
    box.append(list)
    groups.append(box)
  }
  set(t('hh_group_coding'), '', g.coding)
  set(t('hh_group_general'), t('hh_group_general_note'), g.general)
  set(t('hh_group_apps'), t('hh_group_apps_note'), g.app)
}

function labelHelpers() {
  const text: Record<string, StringKey> = {
    'hh-intro': 'hh_intro',
    'hh-check-all': 'hh_check_all',
    'hh-first-title': 'hh_first_title',
    'hh-first-body': 'hh_first_body',
    'hh-first-body2': 'hh_first_body2',
    'hh-first-act': 'hh_first_act',
    'hh-lock-title': 'hh_lock_title',
    'hh-lock-state': 'hh_lock_state',
    'hh-lock-body': 'hh_lock_body',
    'hh-lock-go': 'hh_lock_go',
    'hh-none-title': 'hh_none_title',
    'hh-none-body': 'hh_none_body',
    'hh-none-copy': 'hh_copy',
    'hh-none-note': 'hh_none_note',
  }
  for (const [id, key] of Object.entries(text)) $(id).textContent = t(key)
  $('hh-none-line').textContent = 'npm install -g @anthropic-ai/claude-code'
}

async function runCheck(id: string | null, again: boolean) {
  checking = true
  localStorage.setItem('hh-checked', '1')
  try {
    const poll = window.setInterval(() => void refreshHelpers(), 600)
    try {
      helpersView = await invoke<HelpersReport>('helpers_check', { id, again })
      checkedAt = Date.now()
    } finally {
      window.clearInterval(poll)
    }
  } finally {
    checking = false
  }
  await refreshHelpers()
}

async function refreshHelpers() {
  const r = await invoke<HelpersReport>('helpers_list')
  helpersView = r
  const needs = r.helpers.filter((h) => rowAction(h) !== null && ['needs_signin', 'needs_you', 'not_connected', 'check_failed'].includes(h.state)).length
  $('tab-need').textContent = String(needs)
  show('tab-need', needs > 0)
  if (detailId) renderDetail(r)
  else {
    renderHelpersList(r)
    await refreshAgents()
  }
}

$('hh-first-act').addEventListener('click', () => void guarded('helpers-error', () => runCheck(null, false)))
$('hh-check-all').addEventListener('click', () => void guarded('helpers-error', () => runCheck(null, true)))
$('hh-lock-go').addEventListener('click', () => showTab('computer'))
$('hh-none-copy').addEventListener('click', () => void copy($('hh-none-line').textContent ?? ''))

async function openDetail(id: string) {
  detailId = id
  show('helpers-list', false)
  show('helper-detail', true)
  await refreshHelpers()
  if (everChecked()) void guarded('helpers-error', () => runCheck(id, false))
}

function closeDetail() {
  detailId = null
  askedFor = null
  show('helper-detail', false)
  show('helpers-list', true)
  void refreshHelpers()
}

function copyLine(line: string): HTMLElement {
  const row = el('p', '', 'row')
  const copyBtn = button(t('hh_copy'), () => void copy(line, t('hh_copied')))
  copyBtn.className = 'a-btn compact secondary'
  row.append(el('code', line), copyBtn)
  return row
}

function card(title: string, state?: HTMLElement): HTMLElement {
  const c = el('div', '', 'a-card')
  const head = el('div', '', 'a-card-head')
  head.append(el('b', title))
  if (state) head.append(state)
  c.append(head)
  return c
}

function connectCard(h: HelperView, r: HelpersReport): HTMLElement {
  const c = card(t('hd_connected_title'), el('span', h.connected ? t('hd_conn_state_on') : t('hd_conn_state_off'), h.connected ? 'a-state' : 'a-state needs'))
  if (h.group === 'general') {
    c.append(el('p', t('hd_conn_none_general'), 'hub-why'))
    return c
  }
  if (!h.can_connect) {
    c.append(el('p', t('hd_conn_none_yet', { name: h.name }), 'hub-why'))
    return c
  }
  c.append(el('b', t('hd_conn_this')), el('p', t('hd_conn_this_note', { name: h.name }), 'hub-why'))
  const act = el('div', '', 'a-card-actions')
  const toggle = button(h.connected ? t('hd_conn_remove') : t('hh_act_connect'), () =>
    void guarded('helpers-error', async () => {
      try {
        if (h.connected) {
          await invoke('helper_disconnect', { id: h.id, hub: null })
          toast(t('hd_conn_toast'), t('hd_undo'), () =>
            void guarded('helpers-error', async () => {
              try {
                await invoke('helper_restore', { id: h.id })
              } catch (e) {
                $('helpers-error').textContent = message(e) || t('hd_undo_failed')
              }
              await refreshHelpers()
            }),
          )
          $('helpers-error').textContent = t('hd_conn_removed', { time: clockOf(Math.floor(Date.now() / 1000)), name: h.name })
        } else {
          await invoke('helper_connect', { id: h.id, hub: null })
          toast(t('hd_conn_added', { name: h.name }))
        }
      } catch (e) {
        $('helpers-error').textContent = t('hd_conn_failed', { error: message(e) })
      }
      await refreshHelpers()
    }),
  )
  toggle.className = h.connected ? 'a-btn secondary' : 'a-btn'
  act.append(toggle)
  c.append(act)
  if (h.id === 'claude-code' || h.id === 'claude-desktop') {
    for (const hub of r.hubs) {
      const line = el('p', '', 'row')
      const label = el('span')
      label.append(el('b', t('hd_conn_paired', { hub: hub.name })), el('br'), el('span', t('hd_conn_paired_note'), 'muted'), el('br'), el('span', hub.lines.join(' · '), 'muted'))
      const b = button(hub.added ? t('hd_conn_remove') : t('hh_act_connect'), () =>
        void guarded('helpers-error', async () => {
          await invoke(hub.added ? 'helper_disconnect' : 'helper_connect', { id: h.id, hub: hub.name })
          await refreshHelpers()
        }),
      )
      b.className = 'a-btn compact secondary'
      line.append(label, b)
      c.append(line)
    }
  }
  return c
}

async function tryIt(h: HelperView, confirmed: boolean, signin: boolean) {
  const outcome = await invoke<TryOutcome>('helper_try', { id: h.id, confirmed, signin })
  lastTry = { id: h.id, at: Date.now(), outcome }
  if (outcome.state === 'needs_you') askedFor = h.id
  if (signin && outcome.link_ready) signinSince = Date.now()
  await refreshHelpers()
}

function tryCard(h: HelperView, r: HelpersReport): HTMLElement {
  const c = card(t('hd_try_title'))
  const last = lastTry && lastTry.id === h.id ? lastTry : undefined
  const time = last ? clockOf(Math.floor(last.at / 1000)) : ''
  const trying = h.state === 'trying'
  let line = t('hd_try_nothing', { name: h.name })
  if (report && report.state !== 'running' && report.state !== 'external') line = t('hh_try_needs_host')
  else if (r.locked) line = report?.serves_hub ? t('hh_phone_try') : t('hd_try_locked')
  else if (trying) line = t('hd_try_trying', { name: h.name })
  else if (last) {
    const s = afterTry(last.outcome)
    line =
      s === 'needs_signin'
        ? t('hd_try_signin', { time })
        : s === 'needs_you'
          ? t('hd_try_stopped')
          : s === 'check_failed' || s === 'program_not_found'
            ? t('hd_try_failed', { name: h.name })
            : t('hd_try_worked', { time, name: h.name })
  }
  c.append(el('p', line, 'hub-why'))
  c.append(el('p', `${t('hd_try_last')}: ${last ? time : t('hd_try_never')}`, 'hub-why'))
  if (h.group === 'general') c.append(el('p', t('hd_try_general_note', { name: h.name }), 'hub-why'))
  if (h.id === 'antigravity') c.append(el('p', t('hd_try_antigravity'), 'hub-why'))
  c.append(el('p', t('hd_try_no_prompt', { name: h.name }), 'hub-why'))
  if (askedFor === h.id && last) {
    c.append(el('p', t('hd_try_asked', { time, name: h.name }), 'hub-why'), el('p', t('hd_try_asked_real', { name: h.name }), 'hub-why'), el('p', t('hd_try_asked_here', { name: h.name }), 'hub-why'))
    c.append(copyLine(t('hd_try_asked_cmd', { dir: r.try_it_dir, program: h.path ? h.path.split('/').pop()! : h.id })))
  }
  const act = el('div', '', 'a-card-actions')
  if (trying) {
    const stop = button(t('hd_try_stop'), () =>
      void guarded('helpers-error', async () => {
        await invoke('helper_try_stop')
        signinSince = null
        await refreshHelpers()
      }),
    )
    stop.className = 'a-btn secondary'
    act.append(stop)
  } else {
    const go = button(last ? t('hd_try_again') : t('hd_try_title'), () =>
      void guarded('helpers-error', async () => {
        askedFor = null
        let confirmed = false
        if (h.group === 'general') {
          confirmed = await confirmDialog({
            title: t('hd_try_general_title', { name: h.name }),
            body: t('hd_try_general_body', { name: h.name }),
            keep: t('hd_try_general_no'),
            act: t('hd_try_general_yes'),
            danger: false,
          })
          if (!confirmed) return
        }
        await tryIt(h, confirmed, false)
      }),
    )
    go.disabled = r.locked || !['ready', 'not_connected', 'update_available', 'needs_signin', 'needs_you'].includes(h.state)
    act.append(go)
  }
  c.append(act)
  return c
}

function signinCard(h: HelperView): HTMLElement | null {
  if (!h.can_sign_in || h.group === 'app') return null
  const trying = h.state === 'trying'
  if (!trying && h.state !== 'needs_signin') return null
  const c = card(t('hd_signin_title'))
  const left = signinSince ? minutesLeft(signinSince, Date.now()) : '10:00'
  c.append(el('p', t('hd_signin_waiting', { left }), 'hub-why'), el('p', t('hd_signin_body', { name: h.name }), 'hub-why'))
  const act = el('div', '', 'a-card-actions')
  const ask = (open: boolean) =>
    void guarded('helpers-error', async () => {
      try {
        await invoke('helper_signin', { id: h.id, open })
        if (!open) toast(t('hd_signin_copied'))
      } catch {
        $('helpers-error').textContent = t('hd_signin_nolink')
      }
    })
  const copyLink = button(t('hd_signin_copy'), () => ask(false))
  const openPage = button(t('hd_signin_open'), () => ask(true))
  copyLink.className = 'a-btn compact'
  openPage.className = 'a-btn compact secondary'
  const stop = button(t('hd_signin_stop'), () =>
    void guarded('helpers-error', async () => {
      await invoke('helper_try_stop')
      signinSince = null
      await refreshHelpers()
    }),
  )
  stop.className = 'a-btn compact quiet'
  act.append(copyLink, openPage, stop)
  c.append(act, el('p', t('hd_signin_note', { name: h.name }), 'hub-why'))
  if (h.state === 'needs_signin' && !trying) {
    const again = button(t('hh_act_signin'), () => void guarded('helpers-error', () => tryIt(h, h.group === 'general', true)))
    again.className = 'a-btn compact'
    act.prepend(again)
  }
  return c
}

function installCard(h: HelperView): HTMLElement {
  const c = card(t('hd_install_title'))
  if (h.install) {
    c.append(el('p', t('hd_install_body'), 'hub-why'), copyLine(h.install), el('p', t('hd_install_signin'), 'hub-why'), el('p', t('hd_install_node'), 'hub-why'))
  } else c.append(el('p', t('hd_install_none'), 'hub-why'))
  return c
}

function renderDetail(r: HelpersReport) {
  const h = r.helpers.find((x) => x.id === detailId)
  const box = $('helper-detail')
  if (!h) return closeDetail()
  if (box.dataset.busy === '1') return
  box.replaceChildren()
  const back = button(`‹ ${t('hd_back')}`, closeDetail)
  back.className = 'a-btn compact quiet'
  const kind = t(h.group === 'coding' ? 'hd_kind_coding' : h.group === 'general' ? 'hd_kind_general' : 'hd_kind_app')
  const head = el('div', '', 'row')
  const title = el('div')
  title.append(el('h2', h.name), el('p', h.version ? t('hd_kind_version', { kind, version: h.version }) : t('hd_kind_plain', { kind }), 'hub-why'))
  head.append(title, stateChip(h.state))
  box.append(back, head)
  const facts = card(h.name)
  facts.firstChild!.remove()
  const version = h.version ? t(h.version_from_package ? 'hd_ver_package' : 'hd_ver_run', { version: h.version }) : t('hd_ver_none')
  const home = r.try_it_dir.replace(/\/\.hq\/.*$/, '')
  facts.append(el('p', `${t('hd_fact_version')}: ${version}`), el('p', `${t('hd_fact_program')}: ${h.path ? shortHome(h.path, home) : t('hd_ver_none')}`, 'muted'))
  box.append(facts)
  if (h.state === 'not_installed' && h.group !== 'app') {
    box.append(installCard(h))
  } else if (h.state === 'program_not_found') {
    const c = card(t('hd_choose'))
    c.append(el('p', t('hd_notfound_body', { name: h.name }), 'hub-why'), el('p', t('hd_refused'), 'hub-why'))
    box.append(c)
  } else {
    box.append(connectCard(h, r))
    if (h.group === 'app') box.append(el('p', t('hd_app_note'), 'hub-why'))
    else {
      const si = signinCard(h)
      if (si) box.append(si)
      box.append(tryCard(h, r))
    }
  }
  const again = button(t('hd_check_again'), () => void guarded('helpers-error', () => runCheck(h.id, true)))
  again.className = 'a-btn compact secondary'
  box.append(again)
  const tech = el('details')
  tech.append(el('summary', t('hd_tech')), el('p', t('hd_tech_looked'), 'hub-why'))
  if (h.path) tech.append(el('p', t('hd_tech_program', { program: h.name, path: shortHome(h.path, home) }), 'hub-why'))
  for (const key of ['hd_try_rule', 'hd_check_rule', 'hd_connect_rule', 'hd_signin_rule'] as const) tech.append(el('p', t(key), 'hub-why'))
  box.append(tech)
}
labelHelpers()

// Refresh
async function refresh() {
  const visible = TABS.find((tab) => !$(`tab-${tab}`).hidden)
  const computer = async () => {
    await refreshHost()
    await refreshAgents()
  }
  const work = { computer, devices: refreshDevices, helpers: refreshHelpers }[visible ?? 'computer']
  try {
    await work()
  } catch (e) {
    toast(visible === 'helpers' ? t('hh_error', { error: message(e) }) : message(e))
  }
}

const POLL_MS = 3000
window.setInterval(() => void refresh(), POLL_MS)
void listen('state-changed', () => void refresh())
void listen<string[]>('claude-entries-moved', (e) => toast(t('hub_claude_moved', { names: e.payload.join(', ') })))
void invoke<string | null>('startup_warning').then((w) => {
  if (w) $('host-error').textContent = w
})
void refresh()

// Sharing the model
function renderShare(share: Share) {
  $('share-on').setAttribute('aria-checked', String(share.on))
  const select = $<HTMLSelectElement>('share-model')
  select.replaceChildren()
  const chosen = share.model ?? share.models[0]?.name ?? null
  for (const m of share.models) {
    const option = el('option', m.tools ? m.name : `${m.name} (no tool calls)`)
    option.value = m.name
    option.selected = m.name === chosen
    select.append(option)
  }
  select.hidden = share.models.length === 0
  const context = share.context_length ? `, ${share.context_length} tokens of context` : ''
  $('share-state').textContent = !share.reachable
    ? ''
    : share.on
      ? `Sharing ${chosen ?? 'the first model'} from ${share.server}${context}.`
      : `Not shared. Ollama answers at ${share.server}.`
  $('share-warning').textContent = share.warning ?? ''
}

// A probe can take seconds; only the newest one may draw, so an older answer never undoes a toggle.
let shareAsked = 0
let shareShown: Share | null = null
async function refreshShare() {
  const asked = ++shareAsked
  const share = await invoke<Share>('share_status')
  if (asked !== shareAsked) return
  shareShown = share
  renderShare(share)
}

/** `flip` is the switch being pressed; a model picked from the list keeps it as it is. */
async function saveShare(flip: boolean) {
  const shown = $('share-on').getAttribute('aria-checked') === 'true'
  const on = flip ? !shown : shown
  // With Ollama away the list is empty: keep the model chosen before rather than clearing it.
  const model = $<HTMLSelectElement>('share-model').value || shareShown?.model || null
  shareAsked++
  await invoke('share_set', { on, model })
  toast(on ? 'Sharing this computer\'s model.' : 'Stopped sharing.')
  await refreshShare()
}

$('share-on').addEventListener('click', () => guarded('share-warning', () => saveShare(true)))
$('share-model').addEventListener('change', () => guarded('share-warning', () => saveShare(false)))
$('share-check').addEventListener('click', () => guarded('share-warning', refreshShare))
