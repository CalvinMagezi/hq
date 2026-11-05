/** Display names for what a person sees: a helper's kind and the computer it runs on. */
const AGENT_NAMES: Record<string, string> = {
  'claude-code': 'Claude Code',
  claude: 'Claude',
  codex: 'Codex',
  cursor: 'Cursor',
  pi: 'Pi',
  opencode: 'OpenCode',
  kimi: 'Kimi',
  qwen: 'Qwen',
  antigravity: 'Antigravity',
  'github-copilot': 'GitHub Copilot',
  hermes: 'Hermes',
  openclaw: 'OpenClaw',
}

/** A helper's kind as its display name; an unknown kind keeps its own name. */
export const agentName = (harness: string) => AGENT_NAMES[harness] ?? harness

/** The host id the server uses for the computer HQ itself runs on. */
export const isThisComputer = (host: string) => host === 'native' || host === 'local'

export const computerName = (host: string) => (isThisComputer(host) ? 'This computer' : host)
