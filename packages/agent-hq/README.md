# agent-hq

Installs [HQ](https://github.com/CalvinMagezi/hq), a local-first AI agent hub (a single Rust binary).

```bash
npx agent-hq
```

On Linux x86_64 it downloads the latest stable release, verifies the minisign signature and SHA-256 checksum against the project's public key, and installs `hq` into `~/.local/bin`. On other platforms it builds from source with `cargo` (install Rust from https://rustup.rs first).

Options: `--channel stable|main`, `--prefix <dir>`, `--from-source`, `--repo owner/name`.

Then:

```bash
hq install     # scaffold your vault and config
hq env         # add an LLM API key
hq doctor
hq chat
```

For an always-on server with signed automatic updates, follow `deploy/README.md` in the repository.
