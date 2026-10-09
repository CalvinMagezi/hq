# Joining a machine to your HQ

A machine that should run coding agents for your HQ (a laptop, a workstation, a WSL2 install on
Windows) pairs with it in four steps. A person or an agent can do all of them; the commands
print what to do next.

You need: the machine and the HQ on the same Tailscale tailnet, an `hq` binary on the machine
(`~/.local/bin/hq`, from a [release](https://github.com/CalvinMagezi/hq/releases)), and the coding
agent you want (for example Claude Code) installed and signed in on the machine.

## Get `hq` on the machine

Pick the newest tag on the [releases page](https://github.com/CalvinMagezi/hq/releases), then
(`linux-x86_64`, `linux-aarch64` or `darwin-aarch64` for the platform):

```
V=0.9.1-main.81
curl -fsSL -o hq.tgz "https://github.com/CalvinMagezi/hq/releases/download/v$V/hq-$V-linux-x86_64.tar.gz"
tar xzf hq.tgz && mkdir -p ~/.local/bin && install -m 755 hq ~/.local/bin/hq
```

Releases ship a `SHA256SUMS` file next to the archives; check the archive against it.

## 1. On the machine

```
hq host join
```

Installs the host as a login service (a LaunchAgent on macOS, a systemd user service on Linux and
WSL2) and prints a join code that starts with `hqjoin1.`. The code holds the machine's name, user and
tailnet address, nothing secret. Give `--addr <tailnet address>` when tailscale cannot be asked, or a
name as the first argument to choose the host name (`hq host join dev-wsl`).

It also creates the machine's HQ folder, `Documents/HQ` under the home directory (on every
operating system, including a headless Ubuntu server that has no `Documents` yet). Agents started
from the Workbench page in the web app run there, or in a folder inside it. The host creates the
folder again each time it starts, so deleting it is harmless.

## 2. On the HQ

```
hq host add hqjoin1.…
```

or have the HQ's agent call the `host_add` tool with the code. It creates a key for the machine,
records it as a host, and prints one command that starts with `hq host authorize`.

## 3. On the machine

Run that `hq host authorize …` command. It lets the HQ's key (and only that key, and only from the
HQ's address) call the host.

## 4. On the HQ

```
hq host check <name>
```

or the `host_check` tool. When it says `reachable`, start a session with `host: <name>`. When it
does not, it names the likely cause.

The key can start any command as you on that machine, like an ssh login, so only pair machines you
control. Agents there run in a sandbox: they cannot read the host's own files or your credentials
directories, and they reach only the sites the HQ allows (`agent_host.sandbox.allow_domains`; denied sites
are logged so you can add what you need).

## Windows

The host runs on Linux and macOS, so on Windows it runs inside WSL2 (Ubuntu). The WSL2 setup,
including the Tailscale and sandbox settings WSL2 needs, is in [WINDOWS.md](WINDOWS.md); once
Ubuntu is ready, follow steps 1 to 4 above inside it.

## If you are an agent setting this up

Run the commands exactly as written, in order, and read each one's output before the next: the join
code goes to the HQ, the `authorize` command comes back to the machine. Do not run
`hq host serve --allow-unsandboxed` or edit `authorized_keys` by hand; `hq host authorize` writes the
pinned line. If `hq host install` refuses the binary's location, copy `hq` to `~/.local/bin` and run it
from there.
