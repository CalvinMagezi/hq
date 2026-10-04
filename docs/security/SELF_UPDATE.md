# Self-update tools

HQ can edit its own source, test it, build it and replace its installed
binary (`self_update_begin`, `self_update_check`, `self_update_install`,
`self_update_rollback`, `self_update_status`). That is a way for an agent turn,
including one steered by injected content, to run new code as the HQ user.

**Production and shared deployments should not enable it.** Update them with
`hq update`, which installs signed releases, or with your deploy pipeline.

## Defaults

- `self_update.enabled` is `false`. When it is off the five tools are not
  registered in native sessions or in the MCP registry (one seam,
  `hq_tools::self_update::create_self_update_tools`), every tool also refuses
  to run, and `hq self-apply` refuses to swap a binary.
- With `require_tests: true` (the default), `self_update_install` needs a
  passing `self_update_check` for the run.

## Owner approval for install

`self_update_install` never ships on the agent's say-so:

1. It stages the working tree, records the git tree hash, and builds the
   release binary (`cargo build --release --locked`). Nothing is installed.
2. It hashes the built `target/release/hq` (SHA-256). If the owner has not
   approved this exact run, tree and binary hash, it files an `action_needed`
   item showing the branch, tree, binary hash and diff stat, and returns
   `awaiting_owner_approval`.
3. The owner approves with the Approve button in Telegram or Discord, in the
   web notifications panel, or with `hq queue approve <id>` (interactive
   terminal only, typed `yes`). Those paths sign the approval with an HMAC
   keyed by `approval.key`, a 0600 file next to the HQ config (outside the vault
   and the database, masked from the bash sandbox). An item whose state was
   edited in the database has no valid signature and is treated as unanswered.
   Dismissing declines it for good.
4. On the next call the tool commits, signs `run | tree | binary hash` with the
   same key, stores the signature on the run and marks it `approved`, then
   starts the detached `hq self-apply`.
5. `hq self-apply` recomputes the tree and the binary hash and verifies the
   signature immediately before the swap. A forged `approved` row without the
   key, or a binary replaced after approval, is refused and reported.

The approval names the run, the tree and the built bytes, so a rebuilt or
swapped binary needs a new approval.

## Limits

- Signatures stop a database-only forgery. An agent with a full shell as the
  same OS user and no sandbox can read `approval.key`, sign for itself, or copy a
  binary over the install path. The barrier against that is the bash sandbox
  (`BASH_SANDBOX.md`, which masks the key file), not this gate. Production
  deployments should use `hq update` (signed releases) and leave self-update off.
- `self_update_check` and the release build run code from the edited tree
  (tests, `build.rs`) before and after approval. Read the branch, not just the
  diff stat, before approving.
- Installed binaries are built locally and are not signed.
