# n8n Integration — Retired 2026-08-10

**Status: fully removed from the codebase.** This is a historical record, not
a description of anything currently running. Do not follow any command or
code reference below — every one of them refers to code that no longer
exists in this repo.

## What this was

n8n was HQ's event-ingress layer: it sensed external events (email arrival,
a GitHub PR, an RSS item, a calendar reminder) and POSTed them to HQ, which
did the actual reasoning. The native `hq-workflow` crate was a separate,
parallel workflow-DAG engine for multi-step automations (`workflow_api.rs`,
a Runs tab and cron triggers in `apps/hq-desktop`). Both existed because the
daemon's own scheduler is purely time-based and cannot react to "an email
arrived."

## Why it was retired

The owner decided on 2026-08-10 to drop both n8n and `hq-workflow` in favor of
cron-style daemon tasks and in-process connectors — fewer moving parts than
running a second workflow engine alongside the daemon's own scheduler.

## What replaced it

- **Email triage/reply**: native, in `crates/hq-daemon/src/email_triage.rs`
  (triage) and `crates/hq-daemon/src/email_send.rs` (send — resolves each
  company's `gws` or `imap_smtp` connector and sends in-process, no HTTP
  round-trip to anything).
- **Email ingest**: `crates/hq-web/src/gmail_ingest.rs` (Pub/Sub push,
  primary) and `crates/hq-cli/.../tasks_periodic/email_ingest.rs` (a 15-minute
  polling fallback for companies without Pub/Sub configured).

## What was actually deleted (2026-08-10, this pass)

- The `hq-workflow` crate — already out of the workspace `members` list and
  had zero live dependents; the crate directory itself is now gone.
- `crates/hq-web/src/n8n_hooks.rs` (the `/hooks/n8n` ingress route) and
  `crates/hq-tools/src/n8n.rs` (the `n8n_trigger` egress tool) — removed in
  commit `b8efb47a`, same day as the retirement decision.
- `crates/hq-web/src/{workflow_api,workflow_run_store,workflow_trigger,
  ws_workflow}.rs` — four files that referenced `hq_workflow::` but were
  never declared as `pub mod` in `hq-web`'s `lib.rs`, so they hadn't compiled
  into the binary for some time before this cleanup made that explicit by
  deleting them.
- `apps/hq-desktop`'s Workflows UI (`routes/workflows/`, the Runs tab, the
  cron-trigger config panel, and their exclusive support files
  `lib/workflow-ws.ts`, `lib/workflow-yaml.ts`, `server/workflows.ts`) — its
  backend had already been gone for a while, so every fetch it made had been
  silently failing.
- `docs/workflows/{email-triage,inbox-lieutenant-send}.yaml` — reference
  YAML definitions for an engine that no longer exists.

## What's still outstanding

- **The owner's private workflow suite** (17 real business
  automations — invoicing, payment-check, monthly P&L, outreach, content
  planning) has not been ported. It needs infrastructure this repo doesn't
  have natively yet: a PDF-generation path (n8n shelled to `wkhtmltopdf`,
  not installed on this machine) and a Google Drive upload connector (no
  native client exists in `hq-tools`/`hq-daemon`; `hq-calendar`'s existing
  OAuth2/PKCE flow is a real head start for building one).
- **Whether n8n itself is still running anywhere** could not be verified as
  part of the cleanup. It is not part of this repository, and nothing here
  depends on it.
