import type { ReactNode } from 'react'
import type { HqSettings } from '~/lib/settingsApi'
import { ageWords, backupNeedsLook, type BackupStatus } from '~/lib/settingsGroups'
import { formatApprovedText, type FolderGrant } from '~/lib/sessionFoldersApi'
import { t, tn } from '~/strings'

export type SafetyTab = 'safety' | 'helpers' | 'time' | 'tools'
export const SAFETY_TABS: SafetyTab[] = ['safety', 'helpers', 'time', 'tools']
const TAB_KEY = {
  safety: 'settings_safety_tab',
  helpers: 'settings_helpers_tab',
  time: 'settings_time_tab',
  tools: 'settings_tools_tab',
} as const

const SECS_PER_MINUTE = 60

function Line({ label, sub, children }: { label: string; sub?: string; children: ReactNode }) {
  return (
    <div className="a-card">
      <div className="flex items-baseline justify-between gap-4">
        <b>{label}</b>
        <span className="text-right">{children}</span>
      </div>
      {sub && <p className="a-muted a-body-sm">{sub}</p>}
    </div>
  )
}

const yesNo = (v: boolean) => t(v ? 'settings_yes' : 'settings_no')
const onOff = (v: boolean) => t(v ? 'settings_on' : 'settings_off')

/** The safe box in plain words: what it does, never the config names. */
export function safeBoxValue(h: NonNullable<HqSettings['agent_host']>): { value: string; body: string } {
  if (h.sandbox_mode !== 'process') return { value: t('settings_off'), body: t('settings_helpers_box_off') }
  if (h.sandbox_profile === 'open') return { value: t('settings_helpers_box_open_value'), body: t('settings_helpers_box_open') }
  return { value: t('settings_helpers_box_strict_value'), body: t('settings_helpers_box_strict') }
}

function SafetyTabBody({ s }: { s: HqSettings }) {
  const box = s.safety.bash_sandbox
  const boxWord = box === 'required' ? t('settings_safe_box_required') : box === 'best_effort' ? t('settings_safe_box_best_effort') : t('settings_safe_box_off')
  return (
    <>
      <Line label={t('settings_safe_box')} sub={t('settings_safe_box_sub')}>{boxWord}</Line>
      <Line label={t('settings_net_commands')} sub={t('settings_net_commands_sub')}>{yesNo(s.safety.bash_network)}</Line>
      <Line label={t('settings_skills_ask')} sub={t('settings_skills_ask_sub')}>{yesNo(s.safety.skills_write_approval)}</Line>
      <Line label={t('settings_origins')} sub={t('settings_origins_sub')}>
        {s.safety.web_allowed_origins.length ? s.safety.web_allowed_origins.join(', ') : t('settings_origins_none')}
      </Line>
    </>
  )
}

function HelpersTabBody({ h }: { h: HqSettings['agent_host'] }) {
  if (!h) return <p className="a-muted">{t('settings_helpers_not_reported')}</p>
  const box = safeBoxValue(h)
  const hidden = h.sandbox_hide_credentials !== false
  return (
    <>
      <Line label={t('settings_helpers_box')} sub={box.body}>{box.value}</Line>
      <Line label={t('settings_helpers_passwords')} sub={t('settings_helpers_passwords_sub')}>{yesNo(hidden)}</Line>
      <Line label={t('settings_helpers_default')}>{h.default_host}</Line>
      <Line label={t('settings_helpers_computers')} sub={t('settings_helpers_computers_sub')}>
        {h.hosts.length ? h.hosts.join(', ') : t('settings_helpers_computers_only')}
      </Line>
      <Line label={t('settings_helpers_idle')}>
        {h.idle_reap_hours ? t('settings_helpers_idle_value', { n: h.idle_reap_hours }) : t('settings_helpers_idle_never')}
      </Line>
      <Line label={t('settings_helpers_answer')}>{onOff(h.drive_new_watches)}</Line>
      <Line label={t('settings_helpers_check')}>{t('settings_helpers_check_value', { n: h.driver_checkin_minutes })}</Line>
      <Line label={t('settings_helpers_budget')}>{h.driver_nudge_budget}</Line>
      <Line label={t('settings_helpers_stuck')} sub={t('settings_helpers_stuck_sub')}>
        {tn('settings_helpers_stuck_value', h.driver_no_progress_limit)}
      </Line>
      <Line label={t('settings_helpers_extra_sites')} sub={t('settings_helpers_extra_sites_sub')}>
        {h.sandbox_profile !== 'open' && h.sandbox_extra_domains ? h.sandbox_extra_domains : t('settings_helpers_extra_never')}
      </Line>
    </>
  )
}

function TimeTabBody({ s }: { s: HqSettings }) {
  const l = s.limits
  return (
    <>
      <Line label={t('settings_time_reply')} sub={t('settings_time_reply_sub')}>{`${Math.round(l.turn_ack_timeout_secs / SECS_PER_MINUTE)} min`}</Line>
      <Line label={t('settings_time_background')}>{tn('settings_time_background_value', l.background_turn_max_days)}</Line>
      <Line label={t('settings_time_checkin')}>
        {l.background_progress_secs === null ? t('settings_time_default') : `${Math.round(l.background_progress_secs / SECS_PER_MINUTE)} min`}
      </Line>
    </>
  )
}

function ToolsTabBody({ s }: { s: HqSettings }) {
  const i = s.integrations
  return (
    <>
      <Line label={t('settings_tools_connected')}>
        {i.remote_mcp.length ? i.remote_mcp.map((m) => m.name).join(', ') : t('settings_tools_none')}
      </Line>
      <Line label={t('settings_tools_search')}>{i.searxng_host ?? t('settings_tools_search_builtin')}</Line>
      <Line label={t('settings_tools_disk')}>{onOff(i.disk_watchdog_enabled)}</Line>
    </>
  )
}

/** Safety and limits as four read-only tabs; nothing here changes from the web. */
export function SafetyView({ s, tab, onTab }: { s: HqSettings; tab: SafetyTab; onTab: (t: SafetyTab) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <div role="tablist" className="a-seg">
        {SAFETY_TABS.map((k) => (
          <button key={k} type="button" role="tab" aria-selected={k === tab} onClick={() => onTab(k)}>
            {t(TAB_KEY[k])}
          </button>
        ))}
      </div>
      <p className="a-muted a-body-sm">{t('settings_read_only_web')}</p>
      {tab === 'safety' && <SafetyTabBody s={s} />}
      {tab === 'helpers' && <HelpersTabBody h={s.agent_host} />}
      {tab === 'time' && <TimeTabBody s={s} />}
      {tab === 'tools' && <ToolsTabBody s={s} />}
    </div>
  )
}

export function FoldersView({ grants, canChange, busy, onRevoke, nowMs }: { grants: FolderGrant[]; canChange: boolean; busy: boolean; onRevoke: (g: FolderGrant) => void; nowMs?: number }) {
  return (
    <div className="flex flex-col gap-3">
      <p className="a-muted a-body-sm">{t('settings_folders_intro')}</p>
      <p className="a-muted a-body-sm">{t('settings_folders_hq_note')}</p>
      {grants.length === 0 ? (
        <div className="a-card">
          <b>{t('settings_folders_empty_title')}</b>
          <p className="a-muted">{t('settings_folders_empty_body')}</p>
        </div>
      ) : (
        grants.map((g) => (
          <div key={g.id} className="a-card">
            <b className="font-mono">{g.path}</b>
            <span className="a-muted">{g.host}</span>
            <span className="a-muted">{formatApprovedText(g.approved_at, g.requested_by, nowMs)}</span>
            <button type="button" className="a-btn secondary" disabled={busy || !canChange} onClick={() => onRevoke(g)}>
              {t('settings_folders_revoke')}
            </button>
            {!canChange && <span className="a-muted a-body-sm">{t('settings_folders_revoke_needs')}</span>}
          </div>
        ))
      )}
    </div>
  )
}

/** Amber only when a backup failed or is overdue and this page could start one; else grey. */
export function BackupView({ status, canStart, version }: { status: BackupStatus | null; canStart: boolean; version: string | null }) {
  const known = status?.configured === true
  const age = status?.last_ok_at ? ageWords(status.last_ok_at) : null
  const look = backupNeedsLook(status)
  const amber = look && canStart
  let body: string
  if (!known) body = t('settings_backup_unknown_body')
  else if (status?.failed) body = t('settings_backup_failed')
  else if (status?.overdue && age) body = t('settings_backup_overdue', { age })
  else body = age ? t('settings_backup_on_body', { age }) : t('settings_backup_on')
  return (
    <div className="flex flex-col gap-3">
      <div className="a-card">
        <b>{known ? t('settings_backup_on') : t('settings_backup_unknown_title')}</b>
        <span className={amber ? 'a-warn-t' : 'a-muted'}>{body}</span>
        {version && <span className="a-muted">{t('ver_hq_line', { version })}</span>}
      </div>
    </div>
  )
}
