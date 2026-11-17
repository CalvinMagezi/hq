import { useEffect, useState } from 'react'
import { t } from '~/strings'

interface Props {
  title: string
  /** Without it the title is only read: a chat that does not exist yet has nothing to rename. */
  onRename?: (title: string) => void
}

/** A chat's title in the head. Pressing it makes it a field: Enter or leaving keeps the name, Escape leaves it as it was. */
export function ChatTitle({ title, onRename }: Props) {
  const [editing, setEditing] = useState(false)
  const [text, setText] = useState(title)
  useEffect(() => setText(title), [title])
  if (!onRename) {
    return <h2 className="a-h3">{title}</h2>
  }
  const finish = (keep: boolean) => {
    setEditing(false)
    const next = text.trim()
    if (keep && next !== '' && next !== title) onRename(next)
    else setText(title)
  }
  if (editing) {
    return (
      <input
        autoFocus
        className="hq-title-input a-h3"
        aria-label={t('chat_rename_label')}
        value={text}
        maxLength={120}
        onChange={(e) => setText(e.target.value)}
        onFocus={(e) => e.currentTarget.select()}
        onBlur={() => finish(true)}
        onKeyDown={(e) => {
          if (e.key === 'Enter') {
            e.preventDefault()
            finish(true)
          } else if (e.key === 'Escape') {
            e.preventDefault()
            finish(false)
          }
        }}
      />
    )
  }
  return (
    <h2 className="a-h3">
      <button type="button" className="hq-title-btn" title={t('chat_rename_label')} onClick={() => setEditing(true)}>
        {title}
      </button>
    </h2>
  )
}
