export interface DNode {
  x: number;
  y: number;
  w: number;
  h: number;
  title?: string;
  sub?: string[];
  kind?: 'box' | 'accent' | 'store' | 'group';
}
export interface DEdge {
  pts: [number, number][];
  dashed?: boolean;
  both?: boolean;
  label?: { x: number; y: number; text: string; anchor?: 'start' | 'middle' | 'end' };
}
export interface DiagramSpec {
  id: string;
  title: string;
  summary: string;
  caption: string;
  alt: string[];
  width: number;
  height: number;
  nodes: DNode[];
  edges: DEdge[];
}

const g = (x: number, y: number, w: number, h: number, title: string): DNode => ({ x, y, w, h, title, kind: 'group' });

export const processModel: DiagramSpec = {
  id: 'dg-process',
  title: 'Process model: one binary, several roles',
  summary: 'The hq binary at the top, five roles below it, and the vault underneath that every role reads and writes.',
  caption:
    'Every role is a subcommand of the same binary. hq start runs the long-lived components, while an MCP client launches hq mcp-serve itself.',
  alt: [
    'One binary, hq, about 58 MB, built from the hq-cli crate.',
    'Five roles come out of it: terminal chat (hq chat), the daemon, the chat relays for Discord and Telegram, the web server that carries REST, the chat socket, the MCP endpoint and the PWA, and the MCP stdio server (hq mcp-serve).',
    'All five read and write the same vault: markdown files plus one SQLite database.',
  ],
  width: 768,
  height: 330,
  nodes: [
    { x: 254, y: 12, w: 260, h: 52, kind: 'accent', title: 'hq', sub: ['one Rust binary, about 58 MB'] },
    { x: 20, y: 110, w: 136, h: 76, title: 'Terminal chat', sub: ['hq chat'] },
    { x: 168, y: 110, w: 136, h: 76, title: 'Daemon', sub: ['hq start daemon'] },
    { x: 316, y: 110, w: 136, h: 76, title: 'Chat relays', sub: ['Discord, Telegram'] },
    { x: 464, y: 110, w: 136, h: 76, title: 'Web server', sub: ['REST, /ws, /mcp,', 'the PWA'] },
    { x: 612, y: 110, w: 136, h: 76, title: 'MCP server', sub: ['hq mcp-serve', '(stdio)'] },
    { x: 20, y: 246, w: 728, h: 66, kind: 'store', title: 'The vault', sub: ['markdown files in .vault/ and one SQLite file, .vault/_data/vault.db'] },
  ],
  edges: [88, 236, 384, 532, 680].flatMap((cx): DEdge[] => [
    { pts: [[384, 64], [384, 87], [cx, 87], [cx, 110]] },
    { pts: [[cx, 186], [cx, 246]], both: true },
  ]),
};

export const crateMap: DiagramSpec = {
  id: 'dg-crates',
  title: 'Crate map',
  summary: 'Sixteen crates in five bands: foundations, intelligence, services, the agent runtime and the CLI.',
  caption: 'The 16 crates of the Cargo workspace, grouped by role. This is a map of responsibilities, not a dependency graph.',
  alt: [
    'Foundations: hq-core (types, config, errors, token counters), hq-vault (notes, tasks, frontmatter), hq-db (SQLite, FTS5, embeddings) and hq-llm (providers and backend chains).',
    'Intelligence: hq-agent (session loop, sub-agents, governance, context engine), hq-memory (consolidator, forgetter), hq-tools (about 100 tools) and hq-convert (document conversion).',
    'Services: hq-mcp (the 2-tool gateway), hq-daemon, hq-relay (Discord and Telegram), hq-web (REST, WebSocket, PWA) and hq-update (the signed updater).',
    'Agent runtime: hq-host (the built-in host for long-lived coding agents) and hq-sandbox (the process sandbox policy for one agent).',
    'Entry point: hq-cli, which builds the hq binary.',
  ],
  width: 768,
  height: 510,
  nodes: [
    g(10, 10, 748, 90, 'Foundations'),
    { x: 30, y: 36, w: 168, h: 52, title: 'hq-core', sub: ['types, config, errors'] },
    { x: 210, y: 36, w: 168, h: 52, title: 'hq-vault', sub: ['notes, tasks, frontmatter'] },
    { x: 390, y: 36, w: 168, h: 52, title: 'hq-db', sub: ['SQLite, FTS5, embeddings'] },
    { x: 570, y: 36, w: 168, h: 52, title: 'hq-llm', sub: ['providers, backend chains'] },
    g(10, 110, 748, 90, 'Intelligence'),
    { x: 30, y: 136, w: 168, h: 52, title: 'hq-agent', sub: ['session loop, governance'] },
    { x: 210, y: 136, w: 168, h: 52, title: 'hq-memory', sub: ['consolidate, forget'] },
    { x: 390, y: 136, w: 168, h: 52, title: 'hq-tools', sub: ['about 100 tools'] },
    { x: 570, y: 136, w: 168, h: 52, title: 'hq-convert', sub: ['documents, OCR, brands'] },
    g(10, 210, 748, 90, 'Services'),
    { x: 30, y: 236, w: 132, h: 52, title: 'hq-mcp', sub: ['2-tool gateway'] },
    { x: 174, y: 236, w: 132, h: 52, title: 'hq-daemon', sub: ['scheduler, worker'] },
    { x: 318, y: 236, w: 132, h: 52, title: 'hq-relay', sub: ['Discord, Telegram'] },
    { x: 462, y: 236, w: 132, h: 52, title: 'hq-web', sub: ['REST, WS, PWA'] },
    { x: 606, y: 236, w: 132, h: 52, title: 'hq-update', sub: ['signed updater'] },
    g(10, 310, 748, 90, 'Agent runtime'),
    { x: 30, y: 336, w: 348, h: 52, title: 'hq-host', sub: ['pty panes, control socket'] },
    { x: 390, y: 336, w: 348, h: 52, title: 'hq-sandbox', sub: ['sandbox-exec and bwrap policy'] },
    g(10, 410, 748, 90, 'Entry point'),
    { x: 30, y: 436, w: 708, h: 52, kind: 'accent', title: 'hq-cli', sub: ['builds the hq binary: clap commands, start, install, doctor, update'] },
  ],
  edges: [],
};

export const vaultMemory: DiagramSpec = {
  id: 'dg-vault',
  title: 'The vault and the context engine',
  summary: 'The vault directories on the left feed a five-layer context frame on the right. The daemon memory tier writes back to the vault.',
  caption:
    'The vault is the center. The context engine reads it into token-budgeted frames, and the daemon memory tier keeps it tidy.',
  alt: [
    'Vault: _system/ (SOUL, MEMORY, CRON-SCHEDULE and other system markdown), _threads/ (conversation history), Notebooks/ (notes, projects, knowledge), skills/ and _data/vault.db (SQLite in WAL mode, with FTS5 and embeddings).',
    'Context engine: five layers per frame. 1 System (SOUL plus harness instructions), 2 UserMessage (the current turn), 3 Memory (long-term facts), 4 Thread (recent messages, older ones compacted), 5 Injections (pinned notes and search results).',
    'Surplus tokens cascade between layers: thread 50 percent, injections 35 percent, memory 15 percent.',
    'The daemon memory tier runs consolidation, embeddings and note tagging, and forgetting, and writes the results back to the vault.',
  ],
  width: 768,
  height: 410,
  nodes: [
    g(10, 10, 330, 276, 'Vault (.vault/)'),
    { x: 30, y: 38, w: 290, h: 40, title: '_system/', sub: ['SOUL, MEMORY, CRON-SCHEDULE'] },
    { x: 30, y: 86, w: 290, h: 40, title: '_threads/', sub: ['conversation history'] },
    { x: 30, y: 134, w: 290, h: 40, title: 'Notebooks/', sub: ['notes, projects, knowledge'] },
    { x: 30, y: 182, w: 290, h: 40, title: 'skills/', sub: ['markdown skills'] },
    { x: 30, y: 230, w: 290, h: 40, kind: 'store', title: '_data/vault.db', sub: ['SQLite, FTS5 search, embeddings'] },
    g(410, 10, 348, 276, 'Context engine, five layers'),
    { x: 430, y: 38, w: 308, h: 40, title: '1 System', sub: ['SOUL and harness instructions'] },
    { x: 430, y: 86, w: 308, h: 40, title: '2 UserMessage', sub: ['the current turn'] },
    { x: 430, y: 134, w: 308, h: 40, title: '3 Memory', sub: ['long-term facts'] },
    { x: 430, y: 182, w: 308, h: 40, title: '4 Thread', sub: ['recent messages, older ones compacted'] },
    { x: 430, y: 230, w: 308, h: 40, title: '5 Injections', sub: ['pinned notes and search results'] },
    g(10, 320, 748, 80, 'Daemon memory tier'),
    { x: 30, y: 344, w: 228, h: 44, title: 'Consolidation' },
    { x: 270, y: 344, w: 228, h: 44, title: 'Embeddings and tagging' },
    { x: 510, y: 344, w: 228, h: 44, title: 'Forgetting' },
  ],
  edges: [
    { pts: [[340, 148], [410, 148]], label: { x: 375, y: 138, text: 'read' } },
    { pts: [[174, 320], [174, 286]], label: { x: 184, y: 306, text: 'writes back', anchor: 'start' } },
  ],
};

export const daemon: DiagramSpec = {
  id: 'dg-daemon',
  title: 'The daemon scheduler',
  summary: 'A five-second tick fans out to three tiers of tasks. Two loops run beside the scheduler, and a status file is written after every tick.',
  caption: 'One tick every 5 seconds, three tiers of interval tasks, two side loops and a status file you can read.',
  alt: [
    'A scheduler tick fires every 5 seconds.',
    'Fast tier: approvals, harness-session supervision, heartbeat, value-bus delivery and email poll.',
    'Memory tier: consolidation, embeddings, note tagging and forgetting.',
    'Maintenance tier: vault health, thread rotation, clean-up, the disk watchdog and SQLite vacuum.',
    'Beside the scheduler run the agent worker, which triages inbound events and email, and the machine profile, which records the host CLIs in MACHINE.md.',
    'DAEMON-STATUS.md at the vault root is written after every tick.',
  ],
  width: 768,
  height: 380,
  nodes: [
    { x: 244, y: 10, w: 280, h: 50, kind: 'accent', title: 'Scheduler tick', sub: ['every 5 seconds'] },
    { x: 20, y: 108, w: 232, h: 128, title: 'Fast tier', sub: ['approvals', 'harness-session supervision', 'heartbeat', 'value-bus delivery', 'email poll'] },
    { x: 268, y: 108, w: 232, h: 128, title: 'Memory tier', sub: ['consolidation', 'embeddings', 'note tagging', 'forgetting'] },
    { x: 516, y: 108, w: 232, h: 128, title: 'Maintenance tier', sub: ['vault health, thread rotation', 'clean-up, disk watchdog', 'SQLite vacuum'] },
    { x: 20, y: 290, w: 232, h: 60, kind: 'store', title: 'DAEMON-STATUS.md', sub: ['written after every tick'] },
    g(268, 266, 480, 100, 'Runs beside the scheduler'),
    { x: 284, y: 296, w: 220, h: 56, title: 'Agent worker', sub: ['triages events and email'] },
    { x: 516, y: 296, w: 216, h: 56, title: 'Machine profile', sub: ['records host CLIs'] },
  ],
  edges: [136, 384, 632].map((cx): DEdge => ({ pts: [[384, 60], [384, 84], [cx, 84], [cx, 108]] })).concat([
    { pts: [[136, 236], [136, 290]], dashed: true },
  ]),
};

export const channels: DiagramSpec = {
  id: 'dg-channels',
  title: 'Channels converge on one agent session',
  summary: 'Discord and Telegram go through the relay, the PWA through the web server, and the terminal runs in process. All reach one agent session and one vault.',
  caption: 'Four ways in, one agent loop, one vault. That is why a thread started on your phone continues in the terminal.',
  alt: [
    'Discord and Telegram connect to the hq-relay crate, the unified bot, which handles cancel, background turns and watches.',
    'The web PWA connects to hq-web over a WebSocket and REST.',
    'The terminal chat (hq chat) runs the agent in process.',
    'All four paths reach one agent session in hq-agent, which reads and writes one vault holding the shared thread history and memory.',
  ],
  width: 768,
  height: 340,
  nodes: [
    { x: 20, y: 24, w: 170, h: 52, title: 'Discord' },
    { x: 20, y: 94, w: 170, h: 52, title: 'Telegram' },
    { x: 20, y: 174, w: 170, h: 52, title: 'Web PWA' },
    { x: 20, y: 254, w: 170, h: 52, title: 'Terminal', sub: ['hq chat'] },
    { x: 270, y: 34, w: 210, h: 100, title: 'Relay', sub: ['unified bot, !cancel,', 'background turns, /watch'] },
    { x: 270, y: 168, w: 210, h: 64, title: 'Web server', sub: ['WebSocket and REST'] },
    { x: 550, y: 100, w: 198, h: 110, kind: 'accent', title: 'Agent session', sub: ['one loop, one memory,', 'one thread history'] },
    { x: 550, y: 260, w: 198, h: 56, kind: 'store', title: 'Vault', sub: ['threads and memory'] },
  ],
  edges: [
    { pts: [[190, 50], [230, 50], [230, 70], [270, 70]] },
    { pts: [[190, 120], [230, 120], [230, 100], [270, 100]] },
    { pts: [[480, 84], [515, 84], [515, 125], [550, 125]] },
    { pts: [[190, 200], [270, 200]] },
    { pts: [[480, 200], [515, 200], [515, 150], [550, 150]] },
    { pts: [[190, 280], [530, 280], [530, 190], [550, 190]], label: { x: 360, y: 272, text: 'in process, no server needed' } },
    { pts: [[649, 210], [649, 260]], both: true },
  ],
};

export const mcpGateway: DiagramSpec = {
  id: 'dg-mcp',
  title: 'The MCP server and its two-tool gateway',
  summary: 'MCP clients reach a gateway with two tools, which fronts a registry of about a hundred tools. Remote MCP servers are bridged in.',
  caption: 'Clients see two tools no matter how many HQ has. hq_discover finds, hq_call runs.',
  alt: [
    'MCP clients such as Claude Code, Cursor, VS Code, Copilot and OpenCode connect over stdio (hq mcp-serve) or over HTTP at /mcp. HTTP needs an API key and refuses requests without one.',
    'The gateway exposes exactly two tools: hq_discover and hq_call.',
    'Behind it sits the tool registry of about 100 tools: vault, tasks, harness sessions, sub-agents, web and more.',
    'Remote MCP servers listed under remote_mcp in the config are bridged into the registry as name_discover and name_call.',
  ],
  width: 768,
  height: 316,
  nodes: [
    { x: 20, y: 52, w: 168, h: 112, title: 'MCP clients', sub: ['Claude Code, Cursor,', 'VS Code, Copilot,', 'OpenCode, others'] },
    { x: 226, y: 52, w: 152, h: 112, title: 'Transport', sub: ['stdio: mcp-serve', 'HTTP: /mcp', 'API key required'] },
    { x: 416, y: 40, w: 150, h: 136, kind: 'accent', title: '2-tool gateway', sub: ['hq_discover', 'hq_call'] },
    { x: 604, y: 40, w: 144, h: 136, kind: 'store', title: 'Tool registry', sub: ['about 100 tools', 'vault, tasks,', 'sessions, web...'] },
    { x: 416, y: 236, w: 332, h: 56, title: 'Remote MCP servers', sub: ['bridged in as <name>_discover and <name>_call'] },
  ],
  edges: [
    { pts: [[188, 108], [226, 108]], both: true },
    { pts: [[378, 108], [416, 108]], both: true },
    { pts: [[566, 108], [604, 108]], both: true },
    { pts: [[676, 236], [676, 176]], label: { x: 666, y: 212, text: 'tools', anchor: 'end' } },
  ],
};

export const herdr: DiagramSpec = {
  id: 'dg-herdr',
  title: 'Supervised coding-agent sessions over Herdr',
  summary: 'HQ drives Herdr on a local host directly and on a remote host through a restricted ssh gate. The supervisor polls every host once a minute.',
  caption:
    'A host is any machine running Herdr. HQ treats an unreachable host as unknown, never as exited.',
  alt: [
    'On the HQ side: the harness_session tools start and steer sessions, the supervisor polls each host once a minute, the registry holds one row per session, and task comments and chat updates report what happens.',
    'Local host: HQ calls the herdr CLI directly. Herdr owns the panes where the coding agents run, for example Claude Code, Codex, OpenCode, Cursor, Copilot CLI, Kimi and Qwen.',
    'Remote host, for example a laptop: HQ connects over ssh with a dedicated key that is restricted to one forced command, the gate. The gate passes only the herdr CLI. Herdr then runs the agents.',
    'If a remote host sleeps or leaves the tailnet, its sessions stay running and watching resumes when it answers.',
  ],
  width: 768,
  height: 410,
  nodes: [
    g(10, 10, 250, 390, 'HQ'),
    { x: 30, y: 40, w: 210, h: 66, title: 'harness_session tools', sub: ['spawn, send, wait, logs'] },
    { x: 30, y: 122, w: 210, h: 66, title: 'Supervisor', sub: ['polls every host each minute'] },
    { x: 30, y: 204, w: 210, h: 66, kind: 'store', title: 'Session registry', sub: ['one row per session'] },
    { x: 30, y: 286, w: 210, h: 90, title: 'Reports', sub: ['task comments, web chat,', 'relay alerts when an', 'agent blocks or finishes'] },
    g(330, 10, 428, 140, 'Host: local'),
    { x: 350, y: 44, w: 120, h: 80, title: 'Herdr', sub: ['herdr CLI'] },
    { x: 510, y: 44, w: 228, h: 80, title: 'Agents in panes', sub: ['Claude Code, Codex, OpenCode,', 'Cursor, Copilot CLI, Kimi...'] },
    g(330, 180, 428, 220, 'Host: remote, for example a laptop'),
    { x: 350, y: 214, w: 110, h: 80, title: 'ssh gate', sub: ['dedicated key,', 'forced command'] },
    { x: 480, y: 214, w: 100, h: 80, title: 'Herdr' },
    { x: 600, y: 214, w: 138, h: 80, title: 'Agents', sub: ['in panes'] },
    { x: 350, y: 322, w: 388, h: 60, title: 'Host asleep or off the tailnet', sub: ['sessions stay running, watching resumes'] },
  ],
  edges: [
    { pts: [[260, 80], [350, 80]], label: { x: 305, y: 70, text: 'local' } },
    { pts: [[260, 254], [350, 254]], label: { x: 305, y: 244, text: 'ssh' } },
    { pts: [[470, 84], [510, 84]] },
    { pts: [[460, 254], [480, 254]] },
    { pts: [[580, 254], [600, 254]] },
  ],
};

export const governance: DiagramSpec = {
  id: 'dg-governance',
  title: 'What stands between a tool call and its effect',
  summary: 'Governance rules run before any tool. For the bash tool, three more layers follow: text checks, an environment allowlist and an OS sandbox.',
  caption: 'The first row denies dangerous effects before a tool runs. The second row is the boundary for shell commands.',
  alt: [
    'Row one, governance, runs for every tool call. 1 The model proposes a tool call, possibly steered by untrusted content. 2 Credential rules refuse SSH keys, cloud credentials and HQ config always. 3 The taint tracker marks a session once it has read untrusted content such as web pages, mail, documents or notes. 4 After taint, secret files and outbound network from bash are denied.',
    'Row two applies to the bash tool. 5 Text checks catch honest mistakes and cheap tricks but are not the boundary. 6 The environment allowlist starts the child with an empty environment plus a short list. 7 The OS sandbox is bubblewrap on Linux and sandbox-exec on macOS. 8 The command runs with secret files masked.',
  ],
  width: 800,
  height: 340,
  nodes: [
    g(10, 10, 780, 130, 'Governance, every tool call'),
    { x: 20, y: 38, w: 172, h: 86, title: '1 Tool call', sub: ['chosen by the model,', 'maybe steered by', 'untrusted content'] },
    { x: 216, y: 38, w: 172, h: 86, title: '2 Credential rules', sub: ['ssh, cloud, HQ config', 'refused, always'] },
    { x: 412, y: 38, w: 172, h: 86, title: '3 Taint tracker', sub: ['web, mail, docs, notes', 'mark the session'] },
    { x: 608, y: 38, w: 172, h: 86, title: '4 After taint', sub: ['secret files and bash', 'network are denied'] },
    g(10, 190, 780, 140, 'Bash tool only'),
    { x: 20, y: 222, w: 172, h: 86, title: '5 Text checks', sub: ['speed bumps,', 'not the boundary'] },
    { x: 216, y: 222, w: 172, h: 86, title: '6 Env allowlist', sub: ['child starts with an', 'empty environment'] },
    { x: 412, y: 222, w: 172, h: 86, kind: 'accent', title: '7 OS sandbox', sub: ['bubblewrap on Linux,', 'sandbox-exec on macOS'] },
    { x: 608, y: 222, w: 172, h: 86, kind: 'store', title: '8 Command runs', sub: ['secret files masked'] },
  ],
  edges: [
    { pts: [[192, 81], [216, 81]] },
    { pts: [[388, 81], [412, 81]] },
    { pts: [[584, 81], [608, 81]] },
    { pts: [[694, 124], [694, 166], [106, 166], [106, 222]], label: { x: 400, y: 160, text: 'then, for bash' } },
    { pts: [[192, 265], [216, 265]] },
    { pts: [[388, 265], [412, 265]] },
    { pts: [[584, 265], [608, 265]] },
  ],
};

export const releasePipeline: DiagramSpec = {
  id: 'dg-release',
  title: 'Signed release and pull-based updater',
  summary: 'CI builds and signs a release. An instance pulls it on a timer, verifies every signature and hash, swaps, checks health and rolls back on failure.',
  caption: 'The build side holds the signing key and no credentials for any instance. Each instance holds only the public key.',
  alt: [
    'Build side: a merge to main triggers tests, then a build job with no secrets, then a sign job that is the only place the signing key lives, then a GitHub release with the binary, web files, manifest.json and its minisign signature, plus a signed channel pointer.',
    'Instance side, in order: a systemd timer starts the updater every 10 minutes. It verifies the channel pointer signature and the manifest signature and hash. It downloads artifacts and checks SHA-256 and archive safety. It snapshots the database with VACUUM INTO. It runs the staged binary with --version as the service user and checks the build. It swaps the binary and web files by rename. It restarts the service and polls /health for the new git sha.',
    'Outcome: on success it runs hq install --upgrade. On failure it rolls back the binary, web files and, if needed, the database, blocks that version for 24 hours and sends an alert.',
    'The instance pulls from the release over HTTPS. The build side never connects to the instance.',
  ],
  width: 768,
  height: 420,
  nodes: [
    g(10, 10, 262, 400, 'Build side (CI)'),
    { x: 30, y: 40, w: 222, h: 64, title: 'Merge to main' },
    { x: 30, y: 126, w: 222, h: 64, title: 'Test, then build', sub: ['no secrets in these jobs'] },
    { x: 30, y: 212, w: 222, h: 64, kind: 'accent', title: 'Sign', sub: ['the only job with the key'] },
    { x: 30, y: 298, w: 222, h: 92, kind: 'store', title: 'GitHub release', sub: ['binary, web files,', 'manifest + .minisig,', 'signed channel pointer'] },
    g(300, 10, 458, 400, 'Instance (systemd timer, every 10 minutes)'),
    { x: 316, y: 40, w: 200, h: 64, title: '1 Verify', sub: ['pointer and manifest', 'signatures, hashes'] },
    { x: 316, y: 126, w: 200, h: 64, title: '2 Download', sub: ['SHA-256 and strict', 'archive checks'] },
    { x: 316, y: 212, w: 200, h: 64, title: '3 Snapshot', sub: ['database, VACUUM INTO'] },
    { x: 316, y: 298, w: 200, h: 64, title: '4 Staged check', sub: ['--version as service', 'user must match'] },
    { x: 548, y: 298, w: 194, h: 64, title: '5 Swap', sub: ['binary and web files,', 'by atomic rename'] },
    { x: 548, y: 212, w: 194, h: 64, title: '6 Restart', sub: ['poll /health for the', 'new git sha'] },
    { x: 548, y: 40, w: 194, h: 150, kind: 'accent', title: 'Outcome', sub: ['healthy: install --upgrade', 'failed: roll back,', 'block that version,', 'send an alert'] },
  ],
  edges: [
    { pts: [[141, 104], [141, 126]] },
    { pts: [[141, 190], [141, 212]] },
    { pts: [[141, 276], [141, 298]] },
    { pts: [[316, 72], [292, 72], [292, 344], [252, 344]], dashed: true, label: { x: 288, y: 396, text: 'pulls over HTTPS', anchor: 'start' } },
    { pts: [[416, 104], [416, 126]] },
    { pts: [[416, 190], [416, 212]] },
    { pts: [[416, 276], [416, 298]] },
    { pts: [[516, 330], [548, 330]] },
    { pts: [[645, 298], [645, 276]] },
    { pts: [[645, 212], [645, 190]] },
  ],
};

export const topologies: DiagramSpec = {
  id: 'dg-topologies',
  title: 'Two deployment shapes',
  summary: 'Left: everything on one laptop, bound to loopback. Right: HQ on a VPS, reached over Tailscale only, updating itself from signed releases, optionally driving Herdr on a laptop.',
  caption: 'Nothing here needs a public port for the web UI. Only /mcp may ever be published, and only through a proxy that exposes /mcp and /health.',
  alt: [
    'Laptop only: a browser or terminal talks to hq bound to 127.0.0.1 on port 5678. hq reads and writes the vault on disk and drives a local Herdr with coding agents.',
    'VPS plus Tailscale: a device on your tailnet reaches tailscale serve, which terminates HTTPS on the tailnet. It forwards to Caddy bound to 127.0.0.1:4749, which forwards to hq on 127.0.0.1:5678. The updater timer pulls signed releases. HQ can drive Herdr on a laptop over a restricted ssh key.',
  ],
  width: 768,
  height: 430,
  nodes: [
    g(10, 10, 350, 410, 'A. Laptop only'),
    { x: 30, y: 44, w: 310, h: 56, title: 'Browser or terminal', sub: ['http://127.0.0.1:5678, hq chat'] },
    { x: 30, y: 132, w: 310, h: 64, kind: 'accent', title: 'hq', sub: ['web_bind 127.0.0.1 (default)'] },
    { x: 30, y: 240, w: 146, h: 64, kind: 'store', title: 'Vault', sub: ['on your disk'] },
    { x: 194, y: 240, w: 146, h: 64, title: 'Herdr', sub: ['local coding agents'] },
    g(400, 10, 358, 410, 'B. VPS plus Tailscale'),
    { x: 420, y: 44, w: 318, h: 52, title: 'Tailnet device', sub: ['phone, laptop browser'] },
    { x: 420, y: 116, w: 318, h: 52, title: 'tailscale serve', sub: ['HTTPS, tailnet only'] },
    { x: 420, y: 188, w: 318, h: 52, title: 'Caddy', sub: ['127.0.0.1:4749, loopback only'] },
    { x: 420, y: 260, w: 318, h: 56, kind: 'accent', title: 'hq on the VPS', sub: ['127.0.0.1:5678'] },
    { x: 420, y: 350, w: 150, h: 56, title: 'Updater timer', sub: ['signed releases'] },
    { x: 588, y: 350, w: 150, h: 56, title: 'Laptop Herdr', sub: ['over gated ssh'] },
  ],
  edges: [
    { pts: [[185, 100], [185, 132]], both: true },
    { pts: [[103, 196], [103, 240]], both: true },
    { pts: [[267, 196], [267, 240]], both: true },
    { pts: [[579, 96], [579, 116]] },
    { pts: [[579, 168], [579, 188]] },
    { pts: [[579, 240], [579, 260]] },
    { pts: [[495, 350], [495, 316]], dashed: true },
    { pts: [[663, 316], [663, 350]], dashed: true },
  ],
};
