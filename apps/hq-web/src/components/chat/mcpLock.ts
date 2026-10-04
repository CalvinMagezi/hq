import type { ThreadMessage } from '~/store/threadStore'

/** The server refuses to edit or regenerate a question an MCP client asked, or the reply to it. */
export function isMcpLocked(messages: ThreadMessage[], index: number): boolean {
  const m = messages[index]
  if (!m) return false
  if (m.role === 'user') return Boolean(m.viaMcp)
  const asked = messages[index - 1]
  return asked?.role === 'user' && Boolean(asked.viaMcp)
}
