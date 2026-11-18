## What and why

<!-- What does this change, and why? Link the issue if there is one. -->

## How it was tested

<!-- Commands run, manual checks, anything not verified. -->

## Checklist

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets -- -D warnings`
- [ ] `cargo test --workspace`
- [ ] `bun run build` in `apps/hq-web` (if the web UI changed)
- [ ] Docs updated, and a `changelog.d/<slug>.<kind>.md` file added (if behavior, config or CLI changed; see `changelog.d/README.md`)
- [ ] No real keys, tokens, hostnames or personal data in the diff
- [ ] Security-relevant change? I described the threat I considered
