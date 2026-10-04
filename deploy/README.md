# Deploying Agent-HQ to a server

This directory holds everything needed to run HQ on a Linux VPS: provisioning,
systemd units, Caddy templates and the pull-based updater. The intended model
is a local dev instance you build and test against, and a server instance that
updates itself from signed releases.

Placeholders used below: `<ssh-host>` is an SSH alias for your server,
`hq.example.com` is your public hostname, `hq.example.ts.net` is the server's
Tailscale MagicDNS name, and `<owner>/<repo>` is the GitHub repository that
publishes your releases.

## One-time server setup

1. Provision a VPS (any distro `apt` targets; the script assumes Debian or
   Ubuntu). Add an SSH alias for it, for example in `~/.ssh/config`:
   ```
   Host <ssh-host>
       HostName <vps-ip-or-tailscale-name>
       User root
   ```
2. Run the provisioning script once: `ssh <ssh-host> 'bash -s' < deploy/setup-vps.sh`.
   It installs Tailscale, Caddy, ufw and `gh`, creates an unprivileged `hq`
   user, and writes a starter `/opt/hq/config.yaml` with both chat relays
   disabled (enable them there once you are ready).
3. Install the systemd units: copy `deploy/herdr.service` and
   `deploy/hq.service` to `/etc/systemd/system/`, then
   `systemctl enable --now herdr hq`. Herdr is the runtime for coding-agent
   sessions and runs as its own service so restarting `hq` never takes running
   agents down with it. To also drive Herdr on a laptop over Tailscale, follow
   `docs/HERDR_HARNESS.md`. `hq.service` also reads `/opt/hq/openrouter.env`
   and `/opt/hq/gws.env` when they exist (chmod 600), so provider keys can stay
   out of `config.yaml`; neither is required to start.
4. Optional: GitHub access for `gh` and git over HTTPS. Create a fine-grained
   personal access token limited to the repositories HQ should push to (Contents
   and Pull requests: read and write), then on the server:
   ```
   (umask 077; printf 'GH_TOKEN=%s\n' '<token>' > /opt/hq/gh.env)
   systemctl daemon-reload && systemctl restart hq herdr
   ```
   Both units read the file at start (the leading `-` makes it optional).
   Restarting `herdr` ends running agent sessions, so do it when none are
   mid-task. To let plain `git push` use the token, run once
   `sudo -u hq env HOME=/opt/hq gh auth setup-git --hostname github.com --force`.
   Git commits still need an identity (`git config --global user.name` and
   `user.email` as the `hq` user). `system_info` with check `gh_auth` reports
   what HQ sees.
5. Install a Caddyfile: `deploy/Caddyfile.production` for just the MCP endpoint,
   or `deploy/Caddyfile` for the full stack (MCP plus the web UI). Both are
   templates with `your-domain.com` placeholders; edit before copying to
   `/etc/caddy/Caddyfile` and `systemctl reload caddy`.
6. `tailscale up` on the server so it is reachable from your dev machine over
   the tailnet.

## Pull-based updates (recommended)

An instance pulls signed releases itself. A root-owned systemd timer runs
`hq update --apply`, which verifies a minisign signature, swaps the binary and
web files, restarts, checks `/health` and rolls back on failure. Nothing on the
build side holds a key to the instance.

```
sudo deploy/install.sh --repo <owner>/<repo> --channel stable --pubkey ./update.pub
```

`--repo` is the GitHub repository that publishes the releases, `--channel` is
`main` or `stable`, and `--pubkey` is the minisign public key that signs them.
The script is idempotent: it creates the `hq` user and `/opt/hq`, `/etc/hq`,
`/usr/local/lib/hq`, installs `hq.service`, `herdr.service`, `hq-update.service`
and `hq-update.timer`, writes `/etc/hq/update.conf` and `/etc/hq/update.pub`,
bootstraps the first binary (verified with the `minisign` tool, pinned with
`--bootstrap-sha256`, or supplied with `--bootstrap-binary`), runs the first
`hq update --apply --force` and enables everything. `herdr` itself is not
installed by this script; see `setup-vps.sh`.

A re-run keeps `channel` and `base_url` in an existing `update.conf` unless you
pass them again, keeps edited `hq.service` and `herdr.service` (it prints a diff;
`--force-units` overwrites), and runs the first `hq update --apply --force` only
after a fresh bootstrap of the binary.

Day to day: `hq update --check`, `hq update --pin <version> --apply`,
`hq update --rollback`. Format, trust model and failure handling are in
[`docs/UPDATE_SYSTEM.md`](../docs/UPDATE_SYSTEM.md).

## Building on the server (manual path)

Use this when you are not publishing signed releases. A host should use one
deploy path only: do not mix this with the pull-based updater, which owns the
binary and the web files.

1. Test locally: `cargo test -p <touched-crate>` (or `--workspace`).
2. Build the Linux binary. From an x86_64 Linux machine a plain
   `cargo build -p hq-cli --release` followed by
   `rsync -azP target/release/hq <ssh-host>:/usr/local/bin/hq` is enough. From
   an Apple Silicon Mac, cross-compiling is not supported (rustc segfaults under
   Docker's amd64 emulation), so build natively on the server:
   ```
   # one-time: rustup and build dependencies
   ssh <ssh-host> 'curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y'
   ssh <ssh-host> 'sudo apt install -y build-essential pkg-config libssl-dev'

   # each time: sync the source, build, install, restart. Add `-j <n>` on a
   # small droplet to bound memory; a 2 vCPU / 4 GB box needs about 4 GB of swap.
   rsync -az --delete --exclude target/ --exclude .git/ --exclude node_modules/ \
     --exclude .vault/ --exclude '*.db*' --exclude apps/hq-web/node_modules \
     --exclude .env --exclude '.env.*' \
     ./ <ssh-host>:/root/hq-src/
   ssh <ssh-host> '. "$HOME/.cargo/env" && cd /root/hq-src && cargo build --release -p hq-cli -j 2'
   ssh <ssh-host> 'cp /root/hq-src/target/release/hq /usr/local/bin/hq && systemctl restart hq'
   ```
3. Verify: `./deploy/test-vps-connection.sh https://mcp.example.com <api-key>` runs
   the MCP health check, the `initialize` handshake and a `tools/list` plus
   `hq_discover` call end to end.

## Web UI (PWA) on the server, Tailscale only

`apps/hq-web` builds to a static single-page app (`dist/client`), and
`hq.service` serves it itself from `web/dist` next to the vault
(`/opt/hq/web/dist` here; override with `web_static_dir` in `config.yaml`).
There is no separate web server process: `/`, the PWA's client routes, `/api`,
`/ws` and `/mcp` all come from `:5678`. Caddy only bridges `tailscale serve` to
it, so the UI stays off the public internet. With pull-based updates the web
files ship in the release and need no manual step. Otherwise (bun is needed
wherever you build):

```
ssh <ssh-host> 'cd /root/hq-src/apps/hq-web && bun install && bun run build'
ssh <ssh-host> 'mkdir -p /opt/hq/web/dist && rsync -a --delete /root/hq-src/apps/hq-web/dist/client/ /opt/hq/web/dist/ && chown -R hq:hq /opt/hq/web'
```

1. Install `deploy/Caddyfile.pwa` (one `reverse_proxy` to `:5678`, loopback only
   on `:4749`) and `import` it from whichever Caddyfile is live. It binds only
   `127.0.0.1:4749`, so it cannot collide with the public site. Validate before
   reloading:
   ```
   ssh <ssh-host> 'cp /root/hq-src/deploy/Caddyfile.pwa /etc/caddy/Caddyfile.pwa'
   ssh <ssh-host> 'echo "import Caddyfile.pwa" >> /etc/caddy/Caddyfile && caddy validate --config /etc/caddy/Caddyfile && systemctl reload caddy'
   ```
2. Expose it to the tailnet. The public Caddyfile already owns `:443`, so use an
   alternate HTTPS port for `tailscale serve`:
   ```
   ssh <ssh-host> 'tailscale serve --bg --https=8443 http://127.0.0.1:4749'
   ```
   This gives a stable MagicDNS URL reachable only from tailnet devices:
   `https://hq.example.ts.net:8443/` (confirm the real name with
   `tailscale status`). HTTPS certificates must be enabled for the tailnet.
   `tailscale serve status` shows the live mapping and
   `tailscale serve --https=8443 off` tears it down.

List every origin the UI is served from in `web_allowed_origins` (see
[`docs/security/WEB_AUTH.md`](../docs/security/WEB_AUTH.md)).

## Vault

The server vault (`/opt/hq/.vault`) is the primary copy. There is no sync
script, because an `rsync --delete` from a dev machine would erase notes created
on the server. Read and write the vault through HQ's MCP tools instead.

## What does not carry over from a Mac dev setup

- launchd jobs such as the weekly `scripts/cargo-gc.sh` are macOS scheduling.
  `hq.service` is the systemd equivalent for the daemon itself; any other
  maintenance script needs its own cron or timer on the server.
- Local-only features (today just local Ollama) are off when `config.yaml` sets
  `instance: { instance_type: cloud }`, as the template in `setup-vps.sh` does.
  A `features:` block overrides individual flags.

## Running a second instance

A second server for another person is the same procedure with its own
`<ssh-host>`, its own signing key or channel, and its own vault. Two things to
set per instance: the persona (edit `_system/SOUL.md` in that vault and add
`managed: false` to its frontmatter so `hq install --upgrade` leaves it alone),
and `relay.telegram_authorized_chat_id` before enabling Telegram. Without it the
relay ignores every sender until you run `hq pair` and send `/pair <code>`.
