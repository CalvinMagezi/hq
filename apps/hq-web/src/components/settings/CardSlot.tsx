import { useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'
import { useEffect, useState } from 'react'
import { BACKUP_STATUS_KEY, fetchBackupStatus } from '~/lib/backupStatus'
import { useNewerHq, reloadForNewer } from '~/lib/newerHq'
import { ageWords, backupNeedsLook, pickCard, type BackupStatus, type CardId } from '~/lib/settingsGroups'
import { isSharedComputer } from '~/lib/sharedComputer'
import { enablePush, getPushState } from '~/lib/webPush'
import { t } from '~/strings'

/** Whether this browser could receive notifications but has them off (permission still to ask). */
export function useNotificationsOff(): boolean {
  const [off, setOff] = useState(false)
  useEffect(() => {
    if (isSharedComputer()) return
    getPushState()
      .then((s) => setOff(s === 'off' && Notification.permission === 'default'))
      .catch(() => undefined)
  }, [])
  return off
}

export function useBackupStatus(): BackupStatus | null {
  return useQuery({ queryKey: BACKUP_STATUS_KEY, queryFn: fetchBackupStatus }).data ?? null
}

/** One card, pure: the words of `card` and its one button. */
export function CardView({ card, backup, onAct }: { card: CardId; backup: BackupStatus | null; onAct: () => void }) {
  const body =
    card === 'newer'
      ? t('card_newer_body')
      : card === 'notif_off'
        ? t('card_notif_off_body')
        : backup?.last_ok_at
          ? t('card_backups_body', { age: ageWords(backup.last_ok_at) })
          : t('card_backups_body_never')
  const act = card === 'newer' ? t('card_newer_act') : card === 'notif_off' ? t('card_notif_off_act') : t('card_backups_act')
  const title = card === 'newer' ? t('card_newer_title') : card === 'notif_off' ? t('card_notif_off_title') : t('card_backups_title')
  return (
    <div className="a-card needs" role="status" style={{ margin: '0 16px' }}>
      <div className="a-card-head">
        <b>{title}</b>
      </div>
      <span className="a-muted">{body}</span>
      <div className="a-card-actions">
        <button type="button" className="a-btn secondary" onClick={onAct}>
          {act}
        </button>
      </div>
    </div>
  )
}

/** The one card on top of Settings: what needs a look, or nothing. */
export function CardSlot({ notificationsOff }: { notificationsOff: boolean }) {
  const navigate = useNavigate()
  const newerRunning = useNewerHq()
  const backup = useBackupStatus()
  const card = pickCard({ newerRunning, notificationsOff, backupNeedsLook: backupNeedsLook(backup) })
  if (!card) return null
  const act = () => {
    if (card === 'newer') void reloadForNewer()
    else if (card === 'notif_off') void enablePush().then(() => navigate({ to: '/settings/browser' }))
    else void navigate({ to: '/settings/backup' })
  }
  return <CardView card={card} backup={backup} onAct={act} />
}
