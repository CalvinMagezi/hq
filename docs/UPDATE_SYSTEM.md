# Update system

Instances pull signed releases. The build side publishes artifacts and holds no
credentials for any instance. Everything below uses `<owner>/<repo>` for the
GitHub repository that publishes releases; it is configured per instance.

Code: `crates/hq-update/` (library and tests), `crates/hq-cli/src/commands/update.rs`
(the `hq update` subcommand), `deploy/install.sh` and `deploy/update/`.

## How an instance updates

A root-owned systemd timer (`hq-update.timer`, every 10 minutes with up to 2
minutes of random delay) starts `hq-update.service`, which runs
`/usr/local/lib/hq/hq-updater`, a three-line wrapper that execs
`/usr/local/bin/hq update --apply`. `hq.service` keeps `NoNewPrivileges` and
`ProtectSystem=strict`: the only privileged step lives in the updater unit.

`hq update --apply`:

1. Takes an exclusive non-blocking `flock` on `/var/lib/hq-update/update.lock`
   before anything else (exit 75 if another update runs; the unit treats 75 as
   success). Then it reconciles an interrupted earlier run (see "Crash recovery")
   and skips if the installed binary's `--version` differs from the version this
   process started as, so a stale process never acts on an old view.
2. Fetches `channel-<channel>.json` and its `.minisig`, verifies the signature
   against the trusted public key, and requires `manifest_url` to live under
   `<base>/<owner>/<repo>/releases/download/` with a plain path after it: no
   `..` or `%2e`, no empty segments, no backslashes, userinfo, query or fragment.
3. Fetches `manifest.json` and its `.minisig`, verifies the signature, checks
   its SHA-256 against the pointer, checks `schema`, `min_updater_version`, and
   that `manifest.version` equals the pointer's version.
4. Compares versions (semver). Equal or lower: nothing to do. Lower versions
   are never installed without `--pin` or `--rollback`. A version that failed
   on this host earlier is skipped (see "Blocked versions").
5. Downloads the binary and web artifacts with a hard size cap (manifest size
   and the configured limit), verifies each SHA-256 against the signed
   manifest, and unpacks them with a strict tar reader (see "Archives").
6. After the grace period, writes a consistent copy of the vault database with
   `VACUUM INTO` (`/opt/hq/data/update-snapshots/`, newest 5 kept, never one a kept
   backup still needs) and records how many schema migrations were applied. This
   and every other database operation (including pruning old snapshots) runs as
   the `hq` user through the hidden `hq update-db` command, never as root. The
   service user can make a snapshot fail, so that only stops the update when the
   manifest sets `requires_db_snapshot`; otherwise the update continues without a
   restore point and the result and an alert carry a warning.
7. Runs the staged binary as the `hq` user (`setpriv`, else `runuser`; if running
   as root with neither, it refuses) with `--version` and requires the output to
   be exactly `hq <version> (<sha>...` (tokens compared, so `0.9.10` never
   satisfies `0.9.1`) with the first 7 characters of `git_sha`. This is
   the only downloaded code that runs before the swap, and only after the
   SHA-256 and signature checks.
8. Sends `hq notify-restart pre`, waits `notify.grace_secs`, then swaps:
   current binary is copied to `/var/lib/hq-update/bin/hq.1` (older ones shift
   to `hq.2`, `hq.3`, nothing beyond three is kept), the staged binary (created
   next to `/usr/local/bin/hq`, so the same filesystem) is renamed over it, and
   the web tree is renamed into place with the old one kept as `dist.prev`.
9. Restarts through the restarter (`systemctl restart hq`; on macOS the launchd
   label from `launchd_label`) and polls `/health` (6 tries, 5 seconds apart)
   until it reports the new `git_sha`.
10. On success runs `hq install --upgrade` as the new binary,
    unprivileged (vault content migration hook; a failure is logged and reported
    in the result but does not roll back a healthy update), then sends
    `post-ok`.
11. On failure rolls back automatically: stops the service, restores the binary
    and web files, restores the database snapshot only if migrations ran during
    the failed start or the manifest says `requires_db_snapshot` (the database being
    replaced is kept as `vault.db.pre-restore` with its WAL, so writes made after
    the snapshot are recoverable by hand), restarts, waits
    for the previous `git_sha`, sends `post-rolled-back`, blocks the failed
    version and exits 1.

### Blocked versions

A release that reports the wrong build from its staged `--version`, fails its
health check, or has an unusable archive is recorded in
`/var/lib/hq-update/state.json` and skipped for `blocked_ttl_secs` (default 24
hours), so a broken release does not restart the service every ten minutes.
Blocks expire because the service user can provoke failures on purpose. A
staged binary that merely cannot run (exec error, timeout) is never blocked.
A newer release, or an explicit `--pin <version>`, is installed normally. Every
automatic rollback, and every interrupted update that had to be finished or
undone, sends a loud `hq notify-restart alert`.

### Crash recovery

Before the first swap the updater writes an `in_progress` entry (target, previous
build, snapshot, migration count) to `state.json`, which is written to a temp file,
fsynced, renamed, and the directory fsynced. It is cleared once the update is
healthy or rolled back. At the start of every `--apply` or `--rollback`, a leftover
entry means the last run died: if the new binary is installed, the updater repairs
the web tree (a staged `.dist.new` is moved into place; a missing live tree is
restored from `dist.prev`), records the missing backup slot, restarts, and either
confirms health or rolls back (restoring the database only if the migration count
changed). If the old binary is still installed, the entry is dropped and the web
tree repaired. A corrupt `state.json` is renamed to `state.json.corrupt-<time>`
and treated as empty. A run is also cut off after `deadline_secs` (default 900,
below the unit's 20 minute timeout), and each database call has a 120 second
limit; the journal makes both safe. `hq update --rollback`
also blocks the version it leaves.

### Archives

Artifacts are untrusted until verified, and even then only regular files and
directories under the destination are created. The tar reader rejects symlinks,
hard links, devices, absolute paths, `..` components, duplicate paths, more than
`limits.max_web_files` entries, and more than `limits.max_web_unpacked_bytes`
unpacked. The binary archive must hold exactly one regular file named `hq`.
The web archive must contain `index.html`, `sw.js` and `manifest.json`.

## `hq update`

| Flag | Meaning |
|---|---|
| `--check` (default) | Print current vs available. Exit 0 when up to date, 10 when an update is available. |
| `--apply` | Run the flow above. Exit 0 on success or no-op, 1 on failure or automatic rollback, 75 if locked. |
| `--rollback` | Restore the previous binary and web files from the kept copies. Add `--restore-db` to also restore the snapshot taken before that update. |
| `--channel <name>` | Follow `main` or `stable` for this run. |
| `--pin <version>` | Install exactly this version (downgrades allowed). |
| `--force` | With `--apply`: reinstall the channel head even if it equals the running version (used by the installer to lay down the web files). |
| `--dry-run` | Resolve and verify the release, report what would change, change nothing. |
| `--json` | Machine-readable output. |
| `--conf <path>` | Config file (default `/etc/hq/update.conf`). |

`hq update` does not load the vault config, so running it as root never creates
root-owned files under the vault.

## Configuration: `/etc/hq/update.conf` (TOML)

```toml
repo = "<owner>/<repo>"      # required
channel = "stable"           # main | stable
interval = "10m"             # informational, the timer owns the schedule
pin = "0.9.3"                # optional: stay on exactly this version
base_url = "https://github.com"   # https only (plain http for loopback in tests)
pubkey_path = "/etc/hq/other.pub" # optional, must be root-owned and not group/world writable

bin_path = "/usr/local/bin/hq"
web_dist = "/usr/local/share/hq/web/dist"
state_dir = "/var/lib/hq-update"
snapshots_dir = "/opt/hq/data/update-snapshots"  # service-owned: written and restored as the hq user
vault_db = "/opt/hq/.vault/_data/vault.db"
vault_path = "/opt/hq/.vault"
hq_config_path = "/opt/hq/config.yaml"
service_unit = "hq"
launchd_label = ""           # set on macOS instead of service_unit
run_as_user = "hq"
health_url = "http://127.0.0.1:5678/health"
health_tries = 6
health_interval_secs = 5
keep_binaries = 3
snapshots_keep = 5
# max_pointer_age_secs = 604800   # optional: warn when the channel pointer is older than this
blocked_ttl_secs = 86400     # how long a failed release is skipped
deadline_secs = 900          # whole-run limit, below the unit's TimeoutStartSec

[notify]
enabled = true
grace_secs = 8

[limits]
max_manifest_bytes = 1048576
max_binary_bytes = 314572800
max_web_bytes = 104857600
max_web_unpacked_bytes = 419430400
max_web_files = 20000
```

Environment overrides (for testing and one-off runs): `HQ_UPDATE_CONF`,
`HQ_UPDATE_REPO`, `HQ_UPDATE_CHANNEL`, `HQ_UPDATE_PIN`, `HQ_UPDATE_BASE_URL`,
`HQ_UPDATE_STATE_DIR`, `HQ_UPDATE_BIN_PATH`, `HQ_UPDATE_WEB_DIST`,
`HQ_UPDATE_HEALTH_URL`. The public key is deliberately not overridable by
environment.

## Privilege boundary

The `hq` service user runs LLM-driven tools, so it is treated as hostile to the
updater. Everything root swaps lives under root-owned directories: the binary in
`/usr/local/bin`, state and backups in `/var/lib/hq-update`, and the web tree in
`/usr/local/share/hq/web/dist` (the installer points `hq.service` at it with a
drop-in setting `HQ_WEB_STATIC_DIR`). Root never opens or writes a
service-owned path: the vault database is snapshotted, counted and restored by
`hq update-db`, which the updater starts as the `hq` user (`setpriv`/`runuser`,
no new privileges, cleared environment, and `--setsid --bounding-set=-all
--inh-caps=-all` when the installed `setpriv` supports them; missing flags are
logged and skipped). The snapshot directory is service-owned
by design; root only deletes old `vault-*.db` files there. `hq update` and
`hq update-db` skip `.env` files so a stray file in the working directory cannot
redirect a root run. The `/health` probe is unauthenticated loopback HTTP, so a
compromised `hq` user can fake a healthy answer; that does not gain it privilege
but it can defeat the automatic rollback.

## Trust root

Signatures are minisign (Ed25519), verified in-process with the
`minisign-verify` crate (MIT). Nothing shells out for verification. Only
prehashed signatures (the `minisign -S` default; never pass `-l`) are accepted.

The public key is resolved in this order:

1. `pubkey_path` from the config, if set.
2. `/etc/hq/update.pub`, if it exists.
3. The key compiled into the binary: set `HQ_UPDATE_PUBKEY` to the base64 key
   line (`RWQ...`) when building `hq-cli` (forks do this).

A key file must be a regular file owned by root and not writable by group or
others; otherwise the updater refuses to run rather than falling back silently.

Key rotation is a manual, out-of-band step: install the new `update.pub` on
each instance, then publish releases signed by the new key. Releases signed by
a key an instance does not trust are refused. Not mitigated: a compromised
signing key, and a freeze attack: a stale pointer that is still the newest one
the attacker can serve is accepted, and keeps an instance on an old version
(downgrades and replays of older pointers are refused). Setting
`max_pointer_age_secs` in `update.conf` (off by default) makes `--check` and the
apply result warn when `issued_at` is older than that.

## Release format (the contract for CI)

Every build publishes a GitHub release with tag `v<version>` containing:

| Asset | Content |
|---|---|
| `hq-<version>-linux-x86_64.tar.gz` | A tar.gz holding exactly one file, `hq` (a leading `./` is tolerated). |
| `hq-web-<version>.tar.gz` | The contents of `apps/hq-web/dist/client`, with `index.html`, `sw.js` and `manifest.json` at the top level. Optional for releases that do not change the web build. |
| `SHA256SUMS` | `sha256sum` of the other assets, for humans. The updater does not read it. |
| `manifest.json` | Described below. |
| `manifest.json.minisig` | `minisign -S -m manifest.json` signature (prehashed). |

`<version>` is semver without a leading `v`. Convention: stable releases are
`X.Y.Z`; builds of `main` are `X.Y.Z-main.<run_number>` (a semver prerelease, so
`0.9.1-main.57` < `0.9.1` and `0.9.1-main.58` > `0.9.1-main.57`). Versions must
increase monotonically within a channel; the updater only moves forward.

### `manifest.json` (schema 1)

```json
{
  "schema": 1,
  "version": "0.9.1",
  "git_sha": "<40 hex characters, or at least 7>",
  "channel": "main",
  "built_at": "2026-10-04T12:00:00Z",
  "min_updater_version": "0.9.0",
  "requires_db_snapshot": false,
  "artifacts": [
    { "name": "hq-0.9.1-linux-x86_64.tar.gz", "sha256": "<64 lowercase hex>", "size": 52428800 },
    { "name": "hq-web-0.9.1.tar.gz", "sha256": "<64 lowercase hex>", "size": 1048576 }
  ]
}
```

- `schema` must be `1`; any other value is refused as unsupported before any
  other field is read. Unknown extra fields are ignored.
- `git_sha` must be the commit the binary was built from, and the binary must
  have been built with `HQ_GIT_SHA=<that sha>` (or from a git checkout), because
  the updater compares it with `hq --version` and `/health`.
- `min_updater_version`: the oldest running `hq` that may install this release.
- `requires_db_snapshot`: set `true` for releases with schema changes that are
  not safe to run against an old binary, so a rollback always restores the
  pre-update database.
- Artifact names must be plain file names (`A-Za-z0-9._-+`). The binary
  artifact is required; the web artifact is optional.

### Channel pointers

A release with tag `channel-<name>` (`channel-main`, `channel-stable`) holds
two assets, replaced in place whenever the channel moves:

`channel-<name>.json`

```json
{
  "schema": 1,
  "channel": "stable",
  "version": "0.9.1",
  "manifest_url": "https://github.com/<owner>/<repo>/releases/download/v0.9.1/manifest.json",
  "manifest_sha256": "<64 lowercase hex of manifest.json>",
  "issued_at": "2026-10-04T12:00:00Z",
  "seq": 57001
}
```

and `channel-<name>.json.minisig` (signature over the exact bytes of the JSON).
`issued_at` (RFC 3339 UTC) and `seq` (integer, strictly increasing per channel,
for example `GITHUB_RUN_NUMBER*1000+attempt`) are optional in schema 1. An
instance records the highest `seq` it accepted per channel in `state.json`, only
after a successful apply or an up-to-date result, and refuses a pointer with a
lower `seq`, or the same `seq` with a different `manifest_sha256`, as a replay
of an old validly signed pointer. A pointer without `seq` is still accepted
(logged, never ordered). `--pin` and `--rollback` bypass this on purpose and log
it. If `state.json` is lost or corrupt the mark is lost too; versions still only
move forward.

`channel` must equal the channel the instance follows; a pointer signed for one
channel is refused on another. `manifest_url` must start with `<base_url>/<owner>/<repo>/releases/download/`
and carry no query or fragment. The artifacts are fetched from the same
directory as the manifest. Promoting a build to `stable` means signing and
uploading a new `channel-stable.json` that points at an existing release; no
artifact is rebuilt or re-uploaded. `--pin` skips the pointer and reads
`<base_url>/<owner>/<repo>/releases/download/v<version>/manifest.json`.

### What a build must embed

`hq --version` prints `hq <version> (<git sha> <build time>)`. `build.rs` in
`hq-cli` reads `HQ_GIT_SHA` (falls back to `git rev-parse HEAD`, then
`unknown`) and `SOURCE_DATE_EPOCH` (falls back to the current time).
`/health` reports the same `git_sha` and `build_time` next to `version`.

### Signing in CI

```sh
minisign -S -s "$MINISIGN_SECRET_KEY_FILE" -m manifest.json   # writes manifest.json.minisig
minisign -S -s "$MINISIGN_SECRET_KEY_FILE" -m channel-stable.json
```

The secret key lives only in the build environment's secret store; instances
only ever hold the public key.

## One deploy path per host

A host runs exactly one deploy path. A host converted by `install.sh` serves
`/usr/local/share/hq/web/dist` and takes its binary from the updater, so any
push-based deploy (a CI job that rsyncs `/opt/hq/web/dist` or swaps
`/usr/local/bin/hq` over SSH) must be turned off for it before cutover. Note that
`build.rs` embeds the git sha and build time, so every commit produces a different
binary: a byte-compare shortcut that skips restarts for non-Rust commits no longer
works on a push-based path.

## Installing an instance

```sh
sudo deploy/install.sh --repo <owner>/<repo> --channel stable --pubkey ./update.pub
```

See `deploy/README.md`. The first binary is bootstrapped from the channel
(manifest signature checked with the `minisign` tool, or a SHA-256 you pass with
`--bootstrap-sha256`) or taken from `--bootstrap-binary`; the installer then
runs `hq update --apply --force`, which repeats every check in-process and lays
down the web files.

## Operations

```sh
hq update --check                    # 10 means an update is waiting
hq update --apply --dry-run          # verify the release, change nothing
hq update --pin 0.9.2 --apply        # move to an exact version (downgrades allowed)
hq update --rollback [--restore-db]  # previous binary and web, optionally the pre-update database
journalctl -u hq-update.service      # updater logs
systemctl list-timers hq-update.timer
```

State lives in `/var/lib/hq-update`: `state.json`, `bin/hq.{1,2,3}`,
`update.lock` (database snapshots are in `/opt/hq/data/update-snapshots`). The web tree keeps one previous copy next to the
live one as `dist.prev`.

## Limits and known gaps

- The binary swap is one atomic rename. The web swap is two renames (live to
  `dist.prev`, staged to live); a crash between them is repaired by the next run.
- The manifest URL scheme only supports `linux-x86_64` binaries.
- The updater needs systemd for restarts on Linux. The macOS path reuses
  hq-core's launchd kickstart and has no stop step.
- The updater does not write `/opt/hq/deployed.sha` or fast-forward `/opt/hq/src`
  as an older push-based deploy did. Anything that reads those must be retired
  on an updater-managed host.
- A rollback with `--restore-db` loses writes made after the snapshot (the old
  database is kept as `vault.db.pre-restore`).
- Untested against a real systemd host and a real GitHub release; see
  `TECHDEBT.md`.
- `hq install --upgrade` is run without `--non-interactive` on purpose: that
  flag makes `hq install` overwrite `config.yaml`.
