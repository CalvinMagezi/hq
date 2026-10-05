# Release runbook

> Maintainer only. This is the optional fallback for building releases on your own droplet. You do not need it to install or run HQ.

Releases are built and signed by GitHub Actions on **GitHub-hosted ephemeral runners** (`ubuntu-latest`). No server of ours takes part. The self-hosted release runner described under "Fallback" below stays in the repository as a documented fallback if GitHub starts charging for or limiting hosted minutes.

## Release model

`release.yml` (manual by default; uncomment its `push: branches: [main]` trigger to release on every push to `main`, which signs a release per merge, costs hosted minutes per push and needs the branch protection below) has three jobs, each on a fresh runner that is destroyed afterwards:

- `test`: no environment, no secrets. Builds the web app, then `cargo test --workspace --locked` with `HQ_REQUIRE_BUILT_APP=1`.
- `build`: no environment, no secrets. Builds `hq` and the web app, packages the unsigned artifacts and uploads them. It runs repository build code.
- `sign`: `environment: release`. Checks out only `scripts/release` and `release` at the built commit, downloads the artifacts, installs pinned `minisign` 0.12 and `jq` 1.8.2 from release tarballs verified by SHA-256 (`scripts/release/install-tools.sh`, chosen over apt because apt cannot pin the version), signs, verifies, then publishes. It runs no cargo, bun or npm. The key is written to tmpfs with mode 600 and shredded on exit.

No workflow caches anything (no cache action), so nothing a pull request wrote can be restored in a release job. A cache written by a PR could not be restored on `main` anyway, because caches are scoped to the ref that wrote them.

Releases are immutable. `X.Y.Z-main.<run number>` is created once as a draft with every asset attached, then published, and the channel pointer is updated last. A re-run of the same run number fails clearly; start a new run. If a failed run leaves a draft release, delete the draft by hand. Only the `channel-*` pointer releases are replaced in place, and they carry a signed `issued_at` and a strictly increasing `seq` (run number times 1000 plus attempt) so an updater can refuse a replayed older pointer.

Expected time on a 4 vCPU hosted runner (an estimate, not measured): `test` 30 to 50 minutes cold, `build` 35 to 60 minutes cold (thin LTO, one codegen unit, about 700 crates), `sign` under 5 minutes. `test` and `build` run in parallel. Every job has `timeout-minutes: 120`. Measure the first real run and adjust.

To switch to the self-hosted fallback, change `runs-on: ubuntu-latest` to `runs-on: [self-hosted, linux, hq-release]` in the three jobs of `release.yml` and in `promote.yml`. Nothing else changes.

## Threat model

- A hosted runner is ephemeral, so a poisoned workflow or build step cannot persist anything to the next run.
- The signing key exists only in the `sign` job of the `release` environment, which only `main` may deploy to. Jobs that run build code never receive it. Fork pull requests get neither secrets nor the environment.
- Residual risk: a compromised `main` push ships a signed release, and instances will install it. The defenses are therefore review and account security, not the runner: required pull request review, no admin bypass, two-factor authentication for everyone with write access, CODEOWNERS on the release paths, and a ruleset protecting workflow files. Pointers are replay-protected (`seq`), but an attacker who owns `main` and the environment can mint a valid release.
- If the key or `main` is compromised: rotate the key (below), publish a security notice and have users reinstall from a source they verify out of band.

## Required GitHub settings (mandatory, set once)

Settings, Actions, General:
- "Fork pull request workflows from outside collaborators": **Require approval for all outside collaborators**.
- "Workflow permissions": **Read repository contents and packages permissions**. Leave "Allow GitHub Actions to create and approve pull requests" off.
- Allowed actions: restrict to actions in this repository plus the specific pinned ones used (checkout, setup-bun, upload-artifact, download-artifact, taiki-e/install-action, Swatinem/rust-cache). Require actions to be pinned to a full commit SHA if the option is available.

Settings, Environments, `release`:
- Deployment branches and tags: **Selected branches and tags**, `main` only.
- Required reviewers: recommended, so every release run needs a human click.
- "Prevent self-review" on if there is more than one maintainer. Do not allow administrators to bypass protection rules.
- Environment secrets `MINISIGN_KEY` and `MINISIGN_PASSWORD` (omit the password only if the key has none).

Settings, Rules, Rulesets (target branch `main`, enforcement Active, no bypass list for admins):
- Require a pull request with at least one approving review, dismiss stale approvals, require review from Code Owners (`.github/CODEOWNERS` lists `.github/`, `scripts/release/`, `release/`, `deploy/runner/`, the scan scripts and configs).
- Require status checks: the `ci.yml` jobs.
- Block force pushes and deletions. Require signed commits if all maintainers can sign.
- A second ruleset (or a path restriction in the first) that blocks changes to `.github/workflows/**` except through the reviewed pull request path above, so a workflow edit cannot land without code-owner approval.
- Tag ruleset: restrict creating and deleting `v*` and `channel-*` tags to the release workflow's actor.

Account security: two-factor authentication for every account with write or admin access, and fine-grained or no personal access tokens.

## Workflow notes

The `release` and `promote` workflows refuse any ref but `main`, use `workflow_dispatch` (plus an optional `push` to `main` trigger for `release.yml`, commented out by default), and never use `pull_request*` triggers. `release` and `promote` use separate concurrency groups.

## Signing key (owner, on your own machine)

Never generate the key on the runner or in CI.

```bash
minisign -G -p minisign.pub -s minisign.key     # choose a strong password
```

- Commit the public key as `release/minisign.pub` in the repository. `scripts/release/verify.sh` and the release workflows trust this file, and release builds compile its key line into `hq` (`HQ_UPDATE_PUBKEY`). The upstream project already ships its key; a fork replaces it with its own.
- Store `minisign.key` (the secret key file) and its password in a password manager. Keep an offline backup, for example an encrypted USB stick. Losing it means every installed instance must be re-pointed at a new public key.
- You will paste the full contents of `minisign.key` into the `MINISIGN_KEY` environment secret in the Environment settings below.


## First release

Actions, Release, "Run workflow" on `main`. It tests, builds, packages, signs, self-verifies, then publishes `v<X.Y.Z>-main.<run number>` as a prerelease and updates the signed pointer in the `channel-main` release. Promote with Actions, Promote, giving the release tag: it signs a new `channel-stable` pointer at that existing release and changes no artifact.

Verify any release from a laptop:

```bash
scripts/release/verify.sh --repo OWNER/REPO --tag v0.9.1-main.57 --pubkey release/minisign.pub
```


## Minimum updater version

`release/min-updater-version` holds the oldest `hq` allowed to install a release (initially `0.9.0`). Every release copies it into `manifest.json`. It is a floor, not the release's own version, so bumping the workspace version never locks older instances out. Raise it (edit the file, or set the `min_updater_version` input for one run) only when a release truly cannot be installed by an older updater. Doing so strands every instance running an older updater: they refuse the release until someone installs a newer `hq` by hand on each host.


## Private denylist

`PERSONAL_DENYLIST` is a repository secret (one extended regex per line). `ci.yml` uses it only on pushes to `main`, because fork pull requests never receive secrets; pull requests run the in-repo baseline scan. Before publishing a repository, run `bash scripts/personal-data-scan.sh --history` once locally (with `PERSONAL_DENYLIST` exported) to catch anything ever added.


## Key rotation

1. Generate a new key pair (step 1).
2. Install the new public key on every instance out of band (`docs/UPDATE_SYSTEM.md`, "Trust root"). Instances only accept releases signed by a key they trust, so a signed in-band key change does not exist.
3. Then update the `MINISIGN_KEY` and `MINISIGN_PASSWORD` environment secrets and commit the new `release/minisign.pub`.
4. Run Release and Promote to publish pointers signed with the new key.
5. Destroy the old key once no supported version still trusts it.

If the key is compromised, treat every release signed since the compromise as untrusted: rotate immediately, publish a security notice, and have users reinstall from a source they verify out of band.


## Backup

The droplet holds nothing irreplaceable. Back up the minisign key (offline, step 1) and nothing else. Release assets live on GitHub.


## Fallback: self-hosted release runner

Use this only if hosted minutes become unavailable. It is a small dedicated droplet that builds and signs releases, cut off from everything else, and can be rebuilt from `deploy/runner/` in about 20 minutes. The signing key stays in the `release` environment as above; the repository and environment settings above apply unchanged.

### Isolation rules

The runner must never hold anything that reaches an Agent HQ instance:

- no SSH keys or SSH agent for any other host
- not joined to any tailnet or VPN
- no instance secrets, API keys or `.env` files
- its only credentials are the runner's own registration (scoped to one repository) and, through the `release` environment, the minisign signing key and the job's `GITHUB_TOKEN`
- inbound traffic is denied by `ufw`, with SSH open only to an admin CIDR you choose
- only jobs from `main` that pass the `release` environment gate can reach the signing key

If a step in this runbook asks you to add any of the above, stop and rethink it.


### Sizing

At least 4 vCPU, 8 GB RAM and 100 GB disk, CPU-optimized (for example `c-4` on DigitalOcean). The release build links a large LTO binary and needs the memory. A cold build takes tens of minutes, a warm one far less.


### Create the droplet

```bash
REPO=OWNER/REPO
REF=$(git rev-parse origin/main)
SUM=$(git show "$REF:deploy/runner/provision.sh" | sha256sum | cut -d' ' -f1)
ADMIN_CIDR="$(curl -fsS https://api.ipify.org)/32"

sed -e "s#@REPO@#$REPO#g" -e "s#@REF@#$REF#g" -e "s#@SHA256@#$SUM#g" -e "s#@ADMIN_CIDR@#$ADMIN_CIDR#g" \
  deploy/runner/cloud-init.yaml > /tmp/release-runner-init.yaml

doctl compute droplet create release-runner \
  --region <region> --size c-4 --image ubuntu-24-04-x64 \
  --ssh-keys <fingerprint-of-a-key-used-only-for-this-droplet> \
  --user-data-file /tmp/release-runner-init.yaml --wait
```

Use an SSH key that exists only for this droplet. Do not reuse a key that logs into any other machine. Do not put the registration token in user-data: droplet metadata is readable by every process on the box.

`cloud-init` fetches `provision.sh` at the pinned commit, checks its checksum and runs it. Follow progress with `ssh root@<ip> tail -f /var/log/cloud-init-output.log`.

If your admin address changes, SSH is closed. Reach the droplet through the provider console and run `ufw allow from <new-cidr> to any port 22 proto tcp`.


### Repository settings

The required settings are in the main section above. For the fallback, also: self-hosted runners must never serve pull requests from forks. `release` and `promote` trigger only from `main`, and `ci.yml` uses GitHub-hosted runners.

### Register the runner

Generate a registration token (valid for one hour) at repository Settings, Actions, Runners, New self-hosted runner. Keep it out of shell history and argv. On the droplet:

```bash
ssh root@<ip>
export HISTFILE=/dev/null
bash /root/provision.sh --repo OWNER/REPO --register-only      # prompts for the token with echo off
```

For automation, put the token in a file created with `umask 077` and pass `RUNNER_TOKEN_FILE=/path`. The script reads it, then deletes it. The token is handed to `config.sh` through one process's environment (`ACTIONS_RUNNER_INPUT_TOKEN`), never `--token`, so it does not appear in `ps`.

The runner registers at repository level with the label `hq-release` and starts as `hq-release-runner.service`.

**Ephemeral option.** Add `--ephemeral` to register a runner that takes exactly one job and then deregisters. Use it for a per-job or just-in-time model where each job gets a fresh runner or droplet. With a persistent droplet, leave it off, but then the build job's code runs next to nothing secret (the signing key is only in the `sign` job, see below) and a rebuilt droplet remains the recovery path.

Check it: `systemctl status hq-release-runner` and the runner shows Idle in the repository settings.


### Operations

- **Updating the runner or tools**: bump the pinned versions and checksums at the top of `provision.sh`, then rebuild the droplet. Rebuilding beats patching in place.
- **Disk**: a daily cron (`hq-runner-gc`) removes cargo `target` directories above 40 GB when no job is running. The runner workspace is `/home/runner/actions-runner/_work`.
- **OS patches**: `unattended-upgrades` is enabled. Reboot when `/var/run/reboot-required` appears and the runner is idle.
- **Logs**: `journalctl -u hq-release-runner`.


### Decommissioning

1. Remove the runner in repository settings (Actions, Runners, Remove) or, on the droplet, `sudo -u runner ./config.sh remove --token <removal token>`.
2. `doctl compute droplet delete release-runner`.
3. Revoke the droplet-only SSH key.
4. If you are retiring releases entirely, delete the `release` environment and its secrets.
