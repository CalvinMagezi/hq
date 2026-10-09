# HQ on Windows

There are two ways to run HQ on Windows. Pick by what your computer allows.

| | Full HQ (WSL2) | HQ Lite (native) |
|---|---|---|
| For | Your own PC, or any PC that can run WSL2 | Work computers that block WSL2, virtualization or installers |
| You get | Everything: coding agents, chat bots, every tool | Web app, tasks, notes, search, VS Code agents over MCP |
| Needs | WSL2 (an administrator turns it on once) | Nothing: one program in your user profile, no administrator |
| Not available | | Coding agents, the sandbox, chat bots, anything that sends your notes to another service |
| Guide | this page | [HQ_LITE.md](HQ_LITE.md) and [CORPORATE_WORKSTATION.md](CORPORATE_WORKSTATION.md) |

**Full HQ is the default and the better choice wherever it works.** Lite exists for the computers
where it cannot.

## The installer picks with you

In PowerShell (a normal user is fine; it asks before doing anything):

```
irm https://agent-hq.online/install.ps1 | iex
```

It first looks, without changing anything, at whether WSL2 works here, whether you are an
administrator and whether virtualization is on. It recommends an edition and says why, and you can
choose the other. `-Edition full` or `-Edition lite` skips the question, `-DryRun` shows what it
would do, and `-Yes` accepts the defaults:

```
& ([scriptblock]::Create((irm https://agent-hq.online/install.ps1))) -Edition lite -DryRun
```

For Full HQ it runs the steps in sections 1 and 2 below for you, each shown first and each
skippable, and stops after the one that needs a restart. For HQ Lite it downloads the zip, checks
its SHA-256, unpacks it under `%LOCALAPPDATA%\hq-lite` and starts it once to see whether your
computer lets it run. If a policy (AppLocker, Windows Defender Application Control, Smart App
Control) refuses, it removes the file and lists what you can do instead. It does not try to get
past the policy. It changes no system setting, never elevates itself, and Lite builds are not
code-signed yet, so the checksum shows the download is intact, not who made it.

The rest of this page is Full HQ by hand.

# Full HQ in WSL2

HQ, its coding-agent host and the agents it runs (Claude Code, Codex and the rest) all run inside
WSL2, which is a normal Ubuntu on your Windows machine. You use HQ from your Windows browser as
usual. Everything below happens once.

Two ways to use a Windows machine:

- **HQ runs on this PC.** Install HQ inside WSL2, open the web app in your Windows browser.
  Coding agents run in the same WSL2 install.
- **This PC runs the agents for an HQ somewhere else** (an HQ on a server, say). Install the
  host inside WSL2 and pair it with that HQ. Pairing is the same for every machine; see
  [JOIN_A_MACHINE.md](JOIN_A_MACHINE.md).

You can do both on one PC.

## 1. Set up WSL2 (once)

In PowerShell as a normal user:

```
wsl --install -d Ubuntu
```

Restart if Windows asks, then open **Ubuntu** from the Start menu and create your Linux user. Then
make systemd the init system, which lets HQ's host run as a service. In the Ubuntu shell:

```
printf '[boot]\nsystemd=true\n' | sudo tee /etc/wsl.conf
```

and in PowerShell run `wsl --shutdown`, then open Ubuntu again.

Install what HQ needs, still in Ubuntu:

```
sudo apt update && sudo apt install -y curl openssh-server bubblewrap
sudo systemctl enable --now ssh
sudo loginctl enable-linger "$USER"
```

- `bubblewrap` is the sandbox agents run in. Ubuntu 24.04 blocks the user namespaces it needs, so
  also run `sudo sysctl -w kernel.apparmor_restrict_unprivileged_userns=0` and keep it after reboots
  with `echo 'kernel.apparmor_restrict_unprivileged_userns = 0' | sudo tee /etc/sysctl.d/60-hq.conf`.
- `openssh-server` is only needed when another machine's HQ will drive agents here.
- `enable-linger` keeps HQ's host running when no Ubuntu window is open.

Keep your projects in the Linux file system (`~/projects`), not under `/mnt/c`: it is much faster
and agents handle permissions correctly there. Open them from Windows with
`\\wsl$\Ubuntu\home\<you>\projects` if you want to browse them in Explorer.

## 2. Install HQ (once)

In Ubuntu:

```
curl -fsSL https://agent-hq.online/install.sh | bash
export PATH="$HOME/.local/bin:$PATH"
hq install
```

Then follow the normal [quick start](../README.md#quick-start): add a model key with `hq env`, start
with `hq start all`, and open **http://localhost:5678** in your Windows browser (WSL2 forwards
localhost for you). To keep HQ running in the background, run `hq start all` from a systemd user
service as described in [deploy/README.md](../deploy/README.md).

## 3. Coding agents on this PC

Install the agent inside Ubuntu and sign in there, for example Claude Code (see its own install
instructions), then run:

```
hq host install
```

That starts the built-in host as a service. HQ on this PC uses it with no further setup: start a
session from the web app or `hq sessions spawn claude-code --cwd ~/projects/my-app`.

Agents started from the Workbench page run in `~/Documents/HQ` inside Ubuntu, which `hq host
install` and `hq host join` create. Open it from Windows Explorer at
`\\wsl$\Ubuntu\home\<you>\Documents\HQ` (the Workbench page shows the exact path). It stays in the
Linux file system on purpose: agents work much faster there and permissions behave correctly.

## 4. Let an HQ on another machine use this PC

Both machines need [Tailscale](https://tailscale.com). WSL2 has its own network, so install it
**inside Ubuntu** (`curl -fsSL https://tailscale.com/install.sh | sh`, then
`sudo tailscale up --hostname <name>-wsl`); that Ubuntu is its own node on your tailnet and its
address is the one the other HQ connects to. Then pair as for any machine:

1. In Ubuntu: `hq host join` and copy the join code it prints.
2. On the HQ: `hq host add <code>` (or have its agent call the `host_add` tool). It prints one command
   starting with `hq host authorize`.
3. In Ubuntu: run that command.
4. On the HQ: `hq host check <name>`.

## Limits and troubleshooting

- WSL2 stops when its last window closes unless something keeps it alive; `enable-linger` and the
  host service do that. If a session says the host is unreachable, open Ubuntu once and run
  `hq host status`.
- `cannot sandbox the agent` when starting a session: `bubblewrap` is missing, or the sysctl in step 1
  is not set. `bwrap --ro-bind / / true` should print nothing.
- A coding agent sandboxed on Linux can read and write only its project, its own config directory
  and the temporary directories. Paths that do not exist yet (a `.mcp.json` a project lacks) cannot be
  protected; see [AGENT_HOST.md](AGENT_HOST.md).
- Windows itself is not reachable from the agents: they see only Ubuntu. Files on `C:\` are under
  `/mnt/c`, which the sandbox treats as read-only system area unless you add them in
  `agent_host.sandbox.writable`.
