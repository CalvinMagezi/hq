'use client'

import { useState } from 'react'

const COPIED_MS = 1_500

export function CopyCommand({ command }: { command: string }) {
  const [state, setState] = useState<'idle' | 'copied' | 'failed'>('idle')
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(command)
      setState('copied')
    } catch {
      setState('failed')
    }
    window.setTimeout(() => setState('idle'), COPIED_MS)
  }
  return (
    <div className="command">
      <code>{command}</code>
      <button type="button" className="quiet" onClick={copy}>
        {state === 'copied' ? 'Copied' : state === 'failed' ? 'Select and copy' : 'Copy'}
      </button>
    </div>
  )
}
