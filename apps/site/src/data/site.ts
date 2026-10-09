export const SITE_URL = 'https://agent-hq.online';
export const REPO = 'https://github.com/CalvinMagezi/hq';
export const REPO_BLOB = `${REPO}/blob/main`;
export const LATEST_VERSION_NOTE = '0.9.x';

export const links = {
  repo: REPO,
  releases: `${REPO}/releases`,
  changelog: `${REPO_BLOB}/CHANGELOG.md`,
  license: `${REPO_BLOB}/LICENSE`,
  securityPolicy: `${REPO_BLOB}/SECURITY.md`,
  reportVulnerability: `${REPO}/security/advisories/new`,
  issues: `${REPO}/issues`,
  contributing: `${REPO_BLOB}/CONTRIBUTING.md`,
  readme: `${REPO_BLOB}/README.md`,
  deployReadme: `${REPO_BLOB}/deploy/README.md`,
  hetznerDocs: `${REPO_BLOB}/docs/HETZNER.md`,
  updateSystem: `${REPO_BLOB}/docs/UPDATE_SYSTEM.md`,
  agentSessions: `${REPO_BLOB}/docs/AGENT_SESSIONS.md`,
  windows: `${REPO_BLOB}/docs/WINDOWS.md`,
  joinMachine: `${REPO_BLOB}/docs/JOIN_A_MACHINE.md`,
  mcpAsk: `${REPO_BLOB}/docs/MCP_ASK.md`,
  nativeTasks: `${REPO_BLOB}/docs/plans/native-tasks.md`,
  officialKey: `${REPO_BLOB}/release/minisign.pub`,
  sec: {
    webAuth: `${REPO_BLOB}/docs/security/WEB_AUTH.md`,
    bashSandbox: `${REPO_BLOB}/docs/security/BASH_SANDBOX.md`,
    promptInjection: `${REPO_BLOB}/docs/security/PROMPT_INJECTION.md`,
    webFetch: `${REPO_BLOB}/docs/security/WEB_FETCH.md`,
    selfUpdate: `${REPO_BLOB}/docs/security/SELF_UPDATE.md`,
    agentEnv: `${REPO_BLOB}/docs/security/AGENT_ENV_EXPOSURE.md`,
    identity: `${REPO_BLOB}/docs/security/IDENTITY_SCOPE.md`,
    releaseChecklist: `${REPO_BLOB}/docs/security/RELEASE_CHECKLIST.md`,
  },
  bun: 'https://bun.sh',
  rustup: 'https://rustup.rs',
  minisign: 'https://jedisct1.github.io/minisign/',
  tailscaleServe: 'https://tailscale.com/kb/1242/tailscale-serve',
} as const;

export const nav = [
  { href: '/', label: 'Home' },
  { href: '/architecture/', label: 'Architecture' },
  { href: '/compare/', label: 'Compare' },
  { href: '/install/', label: 'Install' },
  { href: '/security/', label: 'Security' },
] as const;
