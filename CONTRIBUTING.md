# Contributing to Agent-HQ

Pull requests to this repository are limited to the maintainer, who sets the
project's direction. Bug reports and ideas are welcome as issues, and forking is
encouraged: HQ is MIT licensed, so you can build and run your own version. This
file covers setup and the checks every pull request must pass. By participating you agree to the
[Code of Conduct](CODE_OF_CONDUCT.md).

## Reporting bugs and security issues

- Bugs and feature requests: open a GitHub issue using the templates.
- Security problems: do not open an issue. Follow [`SECURITY.md`](SECURITY.md).

## Dev setup

Requirements: Rust 1.89 or newer (edition 2024), a C toolchain and OpenSSL
headers on Linux (`build-essential pkg-config libssl-dev`), and
[bun](https://bun.sh) for the web UI. Ollama, SearxNG and Herdr are optional; web search works without them.

```bash
git clone https://github.com/CalvinMagezi/hq.git
cd hq
cargo build -p hq-cli                  # debug build of the hq binary
cargo run -p hq-cli -- install         # scaffold a vault and ~/.hq/config.yaml
cargo run -p hq-cli -- doctor          # verify setup
cd apps/hq-web && bun install && bun run build   # web UI
```

`CLAUDE.md` and `AGENTS.md` describe the layout, entry points and conventions in
more detail (they are also the instructions AI assistants read in this repo).
Copy `.env.example` for local environment variables. Never commit real keys,
tokens, chat ids, hostnames or personal data; use placeholders such as
`example.com`.

## Checks before you open a PR

All of these must pass:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd apps/hq-web && bun run build        # when you touched the web UI
```

Notes:

- `cargo test --workspace` stops at the first failing test binary, so read the
  totals. Run `cargo test -p <crate>` while iterating.
- Tests that need external state (a real vault, Ollama, the network, an
  authenticated harness binary) must be `#[ignore]` with a reason and an opt-in
  command. They are diagnostics, not gates.
- If you touch shell scripts under `deploy/` or `scripts/`, run `shellcheck` and
  `bash -n` on them.

## Pull request expectations

- Branch from `main`, keep the change focused, and explain the why in the
  description. Link the issue if there is one.
- Add or update tests for behavior changes. Fix the root cause rather than the
  one call site a bug report names.
- Follow the surrounding code style. Named constants instead of magic numbers,
  error paths before happy paths, no new abstraction for a single call site.
- Update docs and `CHANGELOG.md` when behavior, config or CLI output changes.
- Security-relevant changes (auth, sandbox, governance, updater, anything that
  parses untrusted input) get extra scrutiny. Describe the threat you considered.
- Keep commits readable. Squash noise before review if asked.

## Optional integrations

HQ works without Discord, Telegram, Google Workspace, Herdr, Ollama or SearxNG
(`web_search` has a built-in keyless engine pool).
Code that depends on one must degrade cleanly when it is absent, and must never
make an instance-specific service a default.

## License

By contributing you agree that your contributions are licensed under the
[MIT License](LICENSE).
