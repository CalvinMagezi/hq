# Release checklist

Readiness only: nothing here authorises a public release. A release is a tag
`v*` on `main`, and the `Security gates` workflow (`.github/workflows/security.yml`)
must be green on that exact tag before anything is published.

## Automated (the workflow blocks on failure)

- [ ] **Secret scan of the full history** (`gitleaks git --log-opts=--all`),
      with only reviewed false positives suppressed inline (`gitleaks:allow`).
- [ ] **Dependency advisories**: `cargo audit` reports no vulnerability.
- [ ] **Tests**: the CI workflow is green for the tagged
      commit (`cargo check` + `cargo test`).

## Manual, recorded in the release notes

- [ ] **Auth defaults.** `/mcp` refuses requests without `AGENTHQ_API_KEY`;
      the web server refuses a non-loopback bind without `web_auth_token`.
- [ ] **Shell isolation.** Bash runs with the environment allowlist, and the
      sandbox mode is documented for each supported platform.
- [ ] **CORS and CSRF.** `web_allowed_origins` is empty by default and the
      origin-guard tests pass.
- [ ] **Token handling.** No token is accepted in a URL (except the Gmail hook
      secret, see `WEB_AUTH.md`).
- [ ] **Secrets.** Every credential ever committed is rotated or revoked,
      and the release artifacts were scanned.
- [ ] **HQ Lite signature.** The newest main release carries
      `hq-lite-*-windows-x86_64.zip`, its `.sha256` and `.sha256.minisig`,
      and `minisign -V -p release/minisign.pub` accepts the signature. The
      key used is the same `MINISIGN_KEY` as the manifest, available only to
      the `release` environment; the Lite build job has no secrets.
- [ ] **Personal defaults.** No owner-specific hostnames, paths or endpoints
      in default config.

Keep the evidence (workflow run URLs, the scan summary) in the release notes,
never secret values.
