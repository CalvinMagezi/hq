# Set up HQ for me (a guide for AI agents)

If you are a person: give this file to your AI coding agent (Claude Code, Codex, Cursor or similar) and say "follow AGENTS-SETUP.md to set up HQ for me." You do not need to understand the commands. The agent does the work and asks you only for the few things only you can provide.

If you are the agent: follow the steps in order. Run each command yourself, check the result, and stop to ask the user only where a step says ASK.

HQ is a local-first AI agent hub: one program (`hq`) that runs a chat agent on your machine, keeps notes in a folder of markdown files, and has a web page you open in the browser. Source and docs: https://github.com/CalvinMagezi/hq

## Rules for the agent

1. Never paste, print or log an API key or web token in the chat. When a secret is needed, tell the user to type it into their own terminal (see step 4).
2. Do not use `sudo` unless a step says so. Do not edit files outside `~/.hq`, `~/.local` and the user's chosen vault folder.
3. Explain each step to the user in one plain sentence before you run it, and report the result in one sentence after.
4. If a command fails, read the error, try the fix listed under Troubleshooting, and retry once. If it still fails, stop and show the user the exact error.

## Step 1: check the machine

Run `uname -sm`.

- `Linux x86_64`, `Linux aarch64` and `Darwin arm64` (a Mac with Apple silicon) are supported by the prebuilt installer.
- `Darwin x86_64` (an Intel Mac) has no prebuilt build. Use "Build from source" under Troubleshooting.
- Windows: there is no native build; HQ runs inside WSL2 (Ubuntu). Run `grep -qi microsoft /proc/version && echo wsl`: if it prints `wsl` you are already inside WSL2, continue. If you are in PowerShell or Command Prompt, follow `docs/WINDOWS.md` step 1 (it is `wsl --install -d Ubuntu`, then systemd and the packages), then restart this guide inside the Ubuntu terminal. ASK before running the PowerShell steps, since they may need a restart. In WSL2 keep the vault and projects in the Linux file system (`~/`), not under `/mnt/c`.

## Step 2: install

```bash
curl -fsSL https://agent-hq.online/install.sh | bash
```

The script downloads the latest stable release, checks its signature and checksum, and installs `hq` to `~/.local/bin` and the web page files to `~/.local/share/agent-hq/web`. Then make sure `hq` is found:

```bash
export PATH="$HOME/.local/bin:$PATH"
hq --version
```

If `hq --version` prints a version, continue. Tell the user to add the `export PATH` line to their shell profile (`~/.zshrc` on a Mac, `~/.bashrc` on Linux) so it still works in new terminals, and offer to do it for them.

## Step 3: create the HQ folder and config

```bash
hq install
```

This creates the notes folder (the "vault") and the config file `~/.hq/config.yaml`.

## Step 4: add an AI key (ASK)

HQ needs a key from one AI provider to answer questions. Ask the user which they have: OpenRouter, Anthropic or Google AI. If they have none, suggest OpenRouter (https://openrouter.ai/keys) because one key reaches many models, and tell them to create a key with a small spending limit.

Do not ask the user to paste the key to you. Ask them to run this in their own terminal, so the key never enters the chat:

```bash
hq env
```

`hq env` opens the setup for keys. If it is not interactive on their system, tell them to run `export HQ_OPENROUTER_API_KEY='their-key'` (or `HQ_ANTHROPIC_API_KEY`, `HQ_GOOGLE_AI_API_KEY`) in the terminal where HQ will start, or to paste the key into `~/.hq/config.yaml` themselves.

Cost note to give the user: HQ starts on a low-cost model (`openai/gpt-6-luna` through OpenRouter). Every HQ message sends a large amount of background text to the model, so premium models (Claude, GPT flagship tiers) can cost around ten cents for one short reply. Only switch to one on purpose, with `hq env`, and name a model for a single chat with:

```bash
hq chat -m <model-name>
```

## Step 5: check it works

```bash
hq doctor
```

A line saying the database is "not created yet" is normal before first start. A missing key line means step 4 is not done. Fix anything marked as failed, then run it again.

## Step 6: start HQ and open it

```bash
hq start all
```

This runs HQ in the foreground and serves the web page at http://localhost:5678. Open that address in the browser. Check from another terminal:

```bash
curl -s localhost:5678/health
```

A reply containing `"status":"ok"` means it is running. To keep it running after the terminal closes, ask the user if they want that (ASK). On a Mac, `./scripts/install-hq.sh` from a clone of the repo sets up a background service. On Linux, the server guide in `deploy/README.md` sets up a systemd service with automatic signed updates.

## Step 7: first message

Ask the user to type a short hello into the web page, or run `hq chat` and send one line. If it answers, setup is complete. Tell the user:

- where their notes live (the vault folder `hq install` printed),
- how to start HQ again (`hq start all`),
- how to update (re-run the install command in step 2),
- that HQ is early software (version 0.9.x) and that problems can be reported at https://github.com/CalvinMagezi/hq/issues with the output of `hq doctor` (remove any keys first).

## Optional: coding agents on this machine, or on another one

Do this only if the user wants HQ to run coding agents (Claude Code, Codex and others). The agent must already be installed and signed in on the machine that will run it.

- Same machine as HQ: run `hq host install` (add `bubblewrap` first on Linux and WSL2: `sudo apt install bubblewrap`). Sessions then start on this machine.
- A different machine than the one running HQ (an HQ on a server, a laptop or a Windows PC with WSL2): follow `docs/JOIN_A_MACHINE.md`. In short, `hq host join` on that machine prints a code, `hq host add <code>` on the HQ prints one `hq host authorize` command, run that on the machine, then `hq host check <name>` on the HQ. Both machines need Tailscale.

## Optional: Discord, Telegram, remote access

Do these only if the user asks. Each needs a token the user must create themselves, so walk them through it without seeing the token.

- Discord: see `docs/DISCORD-ACCESS.md`.
- Telegram: see `docs/TELEGRAM-ACCESS.md`.
- Reaching the web page from a phone: use Tailscale (https://tailscale.com) and `tailscale serve`, described in the README under "PWA Dashboard". Never expose port 5678 to the open internet.

## Troubleshooting

- `hq: command not found`: run `export PATH="$HOME/.local/bin:$PATH"` and try again.
- Install script says `bubblewrap` is missing (Linux and WSL2): HQ can still run, but its command sandbox is weaker. Install it with the system package manager (`sudo apt install bubblewrap` on Ubuntu or Debian) after asking the user.
- Port 5678 already in use: another program holds it. Find it with `lsof -i :5678`, stop it with the user's approval, or set a different port in `~/.hq/config.yaml` (`ws_port`).
- Build from source (Intel Mac, unsupported systems): install Rust from https://rustup.rs, then `git clone https://github.com/CalvinMagezi/hq.git && cd hq && cargo build --release -p hq-cli && install -m 755 target/release/hq ~/.local/bin/hq`. The first build takes several minutes. The web page needs bun (https://bun.sh): `cd apps/hq-web && bun install && bun run build`.
- Docker instead of the script: `docker run -d --name hq --restart unless-stopped -p 127.0.0.1:5678:5678 -v hq-data:/data ghcr.io/calvinmagezi/hq`. The first start prints a web token in `docker logs hq`. Details in `docs/DOCKER.md`.
- Anything else: run `hq doctor` and read what it flags, or open an issue.
