# Running HQ in Docker

The official image is `ghcr.io/calvinmagezi/hq`, for `linux/amd64` and `linux/arm64`.
It is assembled from the same signed release artifacts as the install script: the
publish workflow verifies the release signature and checksums first, and nothing is
compiled during the image build. The image holds the `hq` binary, the web UI and
the tools HQ calls at runtime (git, curl, bubblewrap) on Ubuntu 22.04. It runs as
the non-root user `hq` (uid 10001) and keeps all state under `/data`.

## Run it

```bash
docker run -d --name hq --init --restart unless-stopped \
  -p 127.0.0.1:5678:5678 -v hq-data:/data \
  -e HQ_OPENROUTER_API_KEY=... \
  ghcr.io/calvinmagezi/hq
docker logs hq
```

Or with Compose: copy `docker-compose.yml` from the repository root, put your key in a
`.env` file next to it (`HQ_OPENROUTER_API_KEY=...`, mode 600) and run
`docker compose up -d`.

Then open the link from the first-run output (next section). HQ needs at least one LLM
provider key to answer; without one the UI loads but chats fail.

Tags: `<version>` (for example `0.9.1-main.19`) never changes once published, `main` is
the newest build from the main branch, `latest` and `stable` follow the stable channel
(the release most recently promoted with the Promote workflow).

## First run and the web token

On the first start the container scaffolds the vault in `/data/vault`, writes
`/data/config.yaml` and creates a random web token. The log shows this block once:

```
 HQ web UI:   http://localhost:5678/#token=<64 hex characters>
 Web token:   <64 hex characters>
```

Inside the container HQ listens on `0.0.0.0` so the published port works. HQ refuses to
do that without a web token, so a token is always set. `/health` and the static web UI
are open; everything under `/api` and `/ws` answers 401 without
`Authorization: Bearer <token>`. The browser takes the token from the `#token=` link
once and keeps it in local storage; the fragment never reaches a server or its logs.

The token lives in `/data/web-auth.env` (mode 600) and is not printed again. Read it with
`docker exec hq cat /data/web-auth.env`. Note that `docker logs` keeps the first-run
output until the container is removed. To use your own token set `HQ_WEB_AUTH_TOKEN`
(nothing is written or printed then), and to rotate the generated one delete the file and
restart.

## Persistent data

`/data` is a volume: the vault (`/data/vault`, plain markdown plus the SQLite database in
`_data/`), `config.yaml`, the token file and the agent's home directory (`/data/home`).
Removing the container keeps it; `docker volume rm hq-data` deletes it.

With a bind mount the host directory must be writable by uid 10001
(`chown -R 10001:10001 ./hq-data`). The entrypoint says so and exits if it is not.

## Adding an API key

Keys are read from the environment on every start and are never written into
`/data/config.yaml` (the entrypoint removes any key `hq install` recorded). Set
`HQ_OPENROUTER_API_KEY` or `HQ_ANTHROPIC_API_KEY`; any other config field follows the same
rule: `HQ_` plus the upper-case field name, and `__` for nesting (see `.env.example`).
Recreate the container to apply a change (`docker compose up -d` does that).

## Using the CLI

```bash
docker exec hq hq doctor            # health report, including the bash sandbox mode
docker exec hq hq status            # vault and daemon status
```

## Updating

```bash
docker compose pull && docker compose up -d
# or: docker pull ghcr.io/calvinmagezi/hq && docker rm -f hq && docker run ... (same flags)
```

The data volume is reused. On start the entrypoint notices the new HQ version and runs
`hq install --upgrade` once (it refreshes the shipped system files and guides and keeps
your `MEMORY.md`, `PREFERENCES.md` and config), and HQ applies its own database migrations.
Do not run `hq update` inside the container: the image is the update unit. To pin a release, use its
version tag. To go back, run the previous tag; migrations are not reversed, so keep a
backup (below) before upgrading across releases that change the schema.

## Backups

Stop HQ so the SQLite files are consistent, then archive the volume:

```bash
docker stop hq
docker run --rm -v hq-data:/data -v "$PWD":/backup ubuntu:22.04 \
  tar czf /backup/hq-data-$(date +%F).tgz -C /data .
docker start hq
```

Restore into an empty volume with `tar xzf /backup/<file> -C /data` (the files keep uid
10001). The archive holds your token and any config, so store it like a secret.

## The bash sandbox

HQ's agent runs shell commands through a `bash` tool and normally wraps each one in
bubblewrap. Bubblewrap needs to create user and mount namespaces, which the default
container security profile forbids. In this image `bwrap` is installed but fails:

```
bwrap: No permissions to create new namespace, likely because the kernel does not allow
non-privileged user namespaces.
```

HQ's default mode, `required`, would then refuse every bash command. The image therefore
sets `governance.bash.sandbox` to `best_effort` (`HQ_GOVERNANCE__BASH__SANDBOX`): commands run
unwrapped, as the same user as HQ, with HQ's other layers still on: an environment
allowlist (provider keys and the web token are not passed to bash), command policy checks,
and a refusal to read env and config files by name. In this mode those checks are text
matching only. A command that gets past them can read `/data/web-auth.env` and `/data/config.yaml`,
which the bubblewrap sandbox would mask. The container itself is the isolation boundary: the
agent sees only `/data` and the image, as an unprivileged user, with no access to the host
filesystem. It can still use the network the container has.

To get the bubblewrap sandbox inside the container anyway, relax the container profile and
require it:

```bash
docker run ... \
  --security-opt seccomp=unconfined --security-opt apparmor=unconfined \
  --security-opt systempaths=unconfined \
  -e HQ_GOVERNANCE__BASH__SANDBOX=required \
  ghcr.io/calvinmagezi/hq
```

That makes the container weaker (a wider syscall surface) in exchange for a second layer
around each command, so it is a trade, not a plain upgrade. This recipe was tested with
nerdctl and containerd on an Ubuntu 22.04 kernel. Hosts that restrict unprivileged user
namespaces another way (newer Ubuntu releases do, through AppArmor) may still refuse, so
confirm the result: `docker exec hq hq doctor` shows the mode and whether a sandbox
backend works (the "Bash sandbox" section), and with `required` a broken sandbox makes
every bash command fail with a message that says why. Set
`HQ_GOVERNANCE__BASH__SANDBOX=off` to skip the wrapper entirely.

## Reaching HQ from other machines

Keep the port on `127.0.0.1` and put a private tunnel in front of it. With Tailscale on
the Docker host:

```bash
tailscale serve --bg 5678
```

This serves `https://hq.example.ts.net` to your tailnet only, forwarding to the
loopback port. Open `https://hq.example.ts.net/#token=<token>`. The page's own origin is
accepted automatically; if a proxy rewrites the `Host` header, list the public origin
in `web_allowed_origins`. A reverse proxy such as Caddy works the same way: terminate TLS,
forward to `127.0.0.1:5678`, and keep authentication in front.

The MCP endpoint `/mcp` refuses everything until `AGENTHQ_API_KEY` is set, and it has its
own key, separate from the web token.

## Never expose /ws or /api to the internet

The web token is one shared secret over plain HTTP unless something terminates TLS. The
API can read and write the whole vault and run agent work. Do not publish port 5678 on
`0.0.0.0` or a public address (`-p 5678:5678` does exactly that), and do not route `/ws`
or `/api` through a public proxy. Bind to `127.0.0.1` and use Tailscale, a VPN or an
authenticating proxy.

## Maintainers

`.github/workflows/docker.yml` builds the image after a successful Release or Promote run
(or by hand with a release tag). `docker/resolve-release.sh` downloads the release,
verifies it with `scripts/release/verify.sh` and decides the tags. `docker/smoke-test.sh IMAGE`
runs the checks above (health, UI, token enforcement, persistence across a restart) against
any image and is what the workflow runs before pushing. The first push creates the package
as private: make it public once in the package settings so `docker pull` works anonymously.
To build locally, put `hq-linux-amd64.tar.gz`, `hq-linux-arm64.tar.gz`, `hq-web.tar.gz` and a
copy of `docker/entrypoint.sh` in one directory (the release tarballs, renamed) and run
`docker build -f docker/Dockerfile <dir>`.
