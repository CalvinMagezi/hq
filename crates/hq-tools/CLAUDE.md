# HQ tools, harness sessions and the features built on them

Notes for working under `crates/hq-tools/`, moved out of the root `CLAUDE.md` so an assistant loads them only when it is in this area. The root file has the rules that apply everywhere.

## Entry points

| What | Path |
|------|------|
| Coding-agent runtime (built-in host) | `crates/hq-host/`, `crates/hq-tools/src/agent_host/`, `crates/hq-tools/src/harness_session/`, `docs/AGENT_HOST.md` |
| Task-linked and chat-driven harness sessions | `crates/hq-tools/src/harness_session/mission.rs`, `crates/hq-web/src/session_driver.rs`, `crates/hq-web/src/sessions_api.rs`, "Sessions that work on a task" and "Watching and driving from a web chat" in `docs/AGENT_SESSIONS.md` |
| Device actions (asking the owner's phone to do something: the calendar tools today, contacts and messages next): the pending-action store, the link to the phone, the risk-class table governance reads, the one agent wrapper, the phone routes | `crates/hq-tools/src/device_actions/` (`store.rs`, `link.rs`, `spec.rs` the `DeviceToolSpec` table and `RiskClass`, `fake.rs` the test-support fake domain), `crates/hq-tools/src/calendar/` (the first domain), `crates/hq-agent/src/device_tool.rs`, `crates/hq-web/src/device_actions_api.rs`, `docs/DEVICE_ACTIONS.md`, `docs/CALENDAR_CREATION.md` |
| Lane quotas (what is left of each coding-agent lane's allowance, read on the host that holds the login, stored, with pace; `hq quota`, `lane_quotas`, `GET /api/lane-quotas`, daemon task `lane-quotas`, config `quota:`) | `crates/hq-host/src/quota/` (readers, `host.quota`), `crates/hq-core/src/quota.rs` and `config/quota.rs` (types and config), `HostBackend::quota` in `crates/hq-tools/src/agent_host/` (native and scripted), `crates/hq-tools/src/lane_quotas/` (`mod.rs` read, store, stale fallback and the tool; `pace.rs` the pure pace maths), `crates/hq-db/src/lane_quota_samples.rs` (`sql/129_lane_quota_samples.sql`), `crates/hq-web/src/lane_quotas_api.rs`, `crates/hq-cli/src/commands/quota.rs`, `tasks_periodic/lane_quotas.rs`, `docs/LANE_QUOTAS.md`. Owner-only data. |
| Asking HQ from an MCP client (`hq_ask`, `hq_ask_result`) | `crates/hq-tools/src/ask.rs` (tools and the `AskRunner` seam), `crates/hq-web/src/ws/ask.rs` (runner, read-only enforcement via the permission preset), `crates/hq-db/src/ask_requests.rs`, `docs/MCP_ASK.md` |
| GitHub without a shell: `github_read`, `github_clone` (a hardened tarball fetch with a base record when neither git nor gh is installed), `github_write` (an `hq/` branch and a pull request for repositories in `github_write.repos`, token `HQ_GITHUB_WRITE_TOKEN`) | `crates/hq-tools/src/github/` (`mod.rs`, `tarball.rs`, `changes.rs`, `write.rs`), `crates/hq-core/src/config/github_write.rs`, `docs/plans/edge-coding-worker.md` |
| HQ Coder jobs from chat: `hq_coder` (start, status, list, steer, stop, retry, feedback, budget; owner's chat only, everything but reading waits for approval), `coder_context` (a worker's only door into the vault: notes linked to its job), one budget and credit cap per job, retries after a kill, the phone and web job controls (`POST /api/coder/{action}`) | `crates/hq-tools/src/github/coder.rs`, `dispatch.rs`, `crates/hq-db/src/coder_jobs.rs`, `crates/hq-agent/src/session/job_budget.rs`, `crates/hq-web/src/coder_api.rs`, `apps/android/.../ui/helpers/CoderJobSheet.kt`, `apps/hq-web/src/components/sessions/OwnRunCard.tsx`, `docs/plans/hq-coder-sprint.md` |
| Sites a chat may fetch without asking (allow a whole site, three approvals in a chat, three chats) | `crates/hq-db/src/fetch_sites.rs`, `fetch_site` in `crates/hq-agent/src/governance/tainted_calls.rs`, `note_site_answer` in `crates/hq-web/src/notifications_api.rs`, `crates/hq-web/src/fetch_sites_api.rs` |
| HQ Coder: the person's own GitHub App by manifest (`hq github connect|status|disconnect`), scoped installation tokens, the repository check, the first-task yes and per-task budget. Owner-only; never a tool or an MCP route | `crates/hq-tools/src/github/app.rs` (manifest, `state`, key store, JWT, tokens), `repos.rs` (scan and flags), `dispatch.rs` (`allowed_repos`), `crates/hq-web/src/github_connect_api.rs`, `crates/hq-cli/src/commands/github.rs`, `apps/android/.../ui/helpers/GithubSheet.kt`, `apps/hq-web/src/components/sessions/GithubConnect.tsx`, `docs/plans/mobile-helpers.md` |
| Shared links summarised into their notes (`enrich_url` on `POST /api/note/create`, daemon task `link-capture`, off with `daemon.link_capture: false`) | `crates/hq-tools/src/link_capture.rs`, `crates/hq-db/src/link_captures.rs`, `crates/hq-cli/src/commands/start/daemon/tasks_periodic/link_capture.rs`, `web_fetch_direct` in `crates/hq-tools-web/src/web/fetch.rs`, `crates/hq-convert/src/ingest/keep.rs` (Google Keep export), `crates/hq-tools/src/collection_tip.rs` (weekly tip, daemon task `collection-tip`, off with `daemon.collection_tip: false`), `docs/plans/link-collection.md` |
| Files the owner is handed (`file_deliver`, the typed `files` entry every export returns, `Exports/<yyyy-mm>/`; exports stay in the vault and are never cache) | `crates/hq-tools/src/deliver.rs`, `crates/hq-tool-api/src/files.rs` (the `files` entry and `Exports/` names), `crates/hq-tools/src/convert.rs` (`confine_to_vault`) |
| Authoring and exporting office files (`office_create`: a native sheet, doc or slides note; `office_export`: xlsx, csv, docx, pptx, pdf into `Exports/<yyyy-mm>/`, with a one-line fidelity statement) | `crates/hq-tools-office/src/author.rs`, `crates/suite/hq-suite-adapter/src/author.rs`, `crates/suite/hq-suite-xlsx/` (formula safety: `formula_is_safe`), `crates/suite/hq-suite-calc` (`Workbook::formula_parses`) |
| Task verification (HQ completes finished tasks from evidence, measures its own accuracy, `hq verify`) | `crates/hq-tools/src/tasks/verify/` (`pipeline.rs` gather, gate, review, finish and the veto window; `dispatch.rs` the loop behind the daemon task `verify-dispatch`; `collect.rs`, `artifact.rs`, `probe.rs` the evidence; `reviewer.rs`, `canary.rs`; `release_check.rs` the check at handover; `sweep.rs` plan, run, apply and undo; `second_opinion.rs`), `crates/hq-db/src/tasks/` (`verification.rs` verdicts and the guard on the edge into `complete`, `waits.rs`, `rework.rs`, `calibration.rs`, `veto.rs`, `sweeps.rs`, `briefing.rs`, `reverify.rs`, `shadow.rs` and `rollout.rs` the numbers that gate each rollout step), `VerificationConfig` in `crates/hq-core/src/config/tasks.rs`, daemon tasks `verify-dispatch`, `verify-audit` and `verify-briefing`, `crates/hq-cli/src/commands/verify.rs`, `crates/hq-web/src/verification_api.rs` (the owner-only REST routes), `apps/hq-web/src/components/tasks/verification/` and `apps/hq-web/src/lib/verificationApi.ts`, `docs/TASK_VERIFICATION.md`, `docs/plans/task-verification.md`. Owner-only by design: sweeps, audits, vetoes and second opinions are not agent tools and not reachable with any MCP key. |
| Helper setup (install the free coding helpers on a connected computer, start their sign-in, know which are ready; `helper_setup` tool, first-run offer, Helpers page panel) | `crates/hq-tools/src/helper_setup/` (`catalog.rs` the fixed commands, `runner.rs`, `service.rs`, `mod.rs`), `HostBackend::run_command`, `crates/hq-db/src/helper_status.rs` (`sql/138_helper_setup.sql`), `crates/hq-web/src/helper_setup_api.rs`, `tasks_periodic/helper_setup_offer.rs` (`daemon.helper_setup_offer`), `apps/hq-web/src/components/sessions/HelperSetup.tsx`, `docs/HELPER_SETUP.md`. The commands are fixed in the catalog; nothing a model or request says becomes a command. |
| Sandbox grants (`sandbox_request`: a helper asks HQ for a path, port or host its strict sandbox denies; HQ approves under a task's access profile, asks the owner, or refuses; `hq grants`) | `crates/hq-core/src/grants.rs` (`classify`, the pure decision), `crates/hq-tools/src/harness_session/sandbox_request.rs` (broker, tool, restart), `sandbox_access.rs` (the owner's standing task, repository and starter answers), `crates/hq-db/src/sandbox_grants.rs` (`sql/148_sandbox_grants.sql`), `LaunchRequest.grants` and `agent_host/sandbox.rs::extend_plan`, `crates/hq-cli/src/commands/grants.rs`, `docs/plans/delegated-sandbox-grants.md`, "Asking for sandbox access" in `docs/AGENT_SESSIONS.md` |
| Native task management | `crates/hq-db/src/tasks.rs` (shared write surface: spaces > initiatives > tasks > comments), `crates/hq-tools/src/tasks/` (MCP tools), `crates/hq-web/src/tasks_api.rs` (REST + WS broadcast), `apps/hq-web/src/routes/tasks.tsx` (UI). Tag-based agent routing pushes to `_mailboxes/<tag>/` synchronously via `hq_core::mailbox::notify_tagged_agents`. Archiving for the view (`shelved_at`, `crates/hq-db/src/tasks/shelf.rs`, `tools_shelf.rs`) is display only and separate from `archived_at`, which is a removal: nothing outside a list that asks for `Shelf::Exclude` may read it. Sub-tasks, start dates, soft dependencies and List/Board/Timeline views: `docs/plans/native-tasks.md`. |
| Web Push and `notify_owner` (`web_push:` config) | `crates/hq-tools/src/notify.rs` (tool), `crates/hq-web/src/web_push/` (VAPID, RFC 8291 encryption, dispatcher, `/api/notifications/push/*`), `crates/hq-db/src/push_subscriptions.rs`, `apps/hq-web/src/lib/webPush.ts`, `apps/hq-web/src/sw.ts`, `docs/WEB_PUSH.md` |
| Note provenance, review state and the task activity view | `crates/hq-db/src/provenance.rs` (tables `note_provenance`, `note_review_events`), `crates/hq-tools/src/provenance.rs` (who is calling, the `Recorder`), `crates/hq-tools/src/vault/review.rs` (`note_review`, `note_provenance`), `crates/hq-tools/src/tasks/activity.rs` (`task_activity`), `crates/hq-web/src/tasks_api.rs` (REST), `apps/hq-web/src/components/tasks/TaskActivity.tsx` and `InitiativeBriefing.tsx` |
| Context packets (source-linked vault excerpts, built from a task id, with provenance and gaps) and their evaluation | `crates/hq-memory/src/context_packet.rs` (`build_packet`, `verify_citations`), `crates/hq-memory/src/context_task.rs` (the task record and its sources), `crates/hq-tools/src/vault/context.rs` (`context_packet`), `crates/hq-memory/tests/context_packet_eval.rs` (`cargo test -p hq-memory --test context_packet_eval`), `docs/architecture/vault-context-packets.md` |
| Skills HQ ships (`hq skills sync`, `skills.builtin.off`): embedded in the binary, kept in the vault at every start, always loaded from the embedded text | `crates/hq-tools/src/skills/builtin.rs`, `crates/hq-tools/assets/skills/`, `crates/hq-core/src/config/skills.rs`, "Skills HQ ships" in `docs/SKILL_FLYWHEEL.md` |
| Skill evidence, outcomes and fixtures (`hq skills evidence`) | `crates/hq-db/src/skill_invocations.rs`, `skill_outcomes.rs`, `skill_evidence.rs`, `crates/hq-tools/src/skills/fixtures.rs`, `evidence_proposal.rs`, `crates/hq-cli/src/commands/skills_evidence.rs`, `docs/SKILL_FLYWHEEL.md` |
| The notes check as a service and a tool (`hq_tools::vault_health`: scan, store, ignore, read back; `vault_health`: read-only, one note or all; the write tools' `warnings` for links that lead nowhere) | `crates/hq-tools/src/vault_health.rs`, `crates/hq-tools/src/vault/health.rs`, `crates/hq-tools/src/vault/write.rs` (`link_warnings`), `hq_vault::health`, `docs/VAULT_HEALTH.md`. Not in the Spark allowlist: it reads every note. Repairs are never a tool. |

## Rules

- **Coding-agent runtime**: long-lived coding agents (Claude Code, Codex, Cursor,
  Pi, OpenCode, Copilot CLI, Kimi, Qwen, Antigravity) run in HQ's built-in host
  (`hq host`, crate `hq-host`). General-purpose agents (Hermes, OpenClaw) run
  there too as a second harness category
  (`crates/hq-tools/src/harness_session/category.rs`): every session has a
  `purpose` (`code`, `monitor`, `research`, `review`, `general`), a
  general-purpose agent refuses `code`, a monitor or reviewer never moves its
  task, and any session given sessions to `observe` (at spawn, or at run time with
  `harness_session_subscribe`) hears their finished turns, blocks
  and exits through the task-thread messages the supervisor delivers. They get
  no HQ token. The **Agentic Stack** (`stack:` config,
  `crates/hq-tools/src/harness_session/stack.rs`, daemon task `stack-dispatch`,
  `hq stack`) names such agents as specialists that pull the tasks assigned to
  them and run each as one one-shot job (`hermes chat --oneshot -q "<brief>"`); the
  process ending puts the task into review (`mission::Event::RunEnded`). A pane
  whose process ended counts as gone (`AgentInfo::exited`). "General-purpose
  agents" in `docs/AGENT_SESSIONS.md`, plan in `docs/plans/agentic-stack.md`. `crates/hq-tools/src/agent_host/`
  (to be renamed) holds the client: `NativeBackend` talks to the host on this
  machine, or to one on another machine over SSH through a key pinned to
  `hq host gate`. Agents run in a process sandbox whose profile is
  `agent_host.sandbox.profile`: `open` (default) walls off HQ's own files (host tokens, `~/.hq`, the vault's `_data`,
  the config file and `*.env` beside it; the vault folders HQ loads, such as `_system` and
  `skills`, are read-only), other processes' environments, other Claude accounts and
  credential stores unless `hide_credentials: false`, and leaves the rest of the machine and
  its network to the agent;
  `strict` adds an egress allowlist and read and write allowlists (`crates/hq-host/src/sandbox.rs`).
  `hq host join` / `hq host add` / the `host_add` tool pair a machine.
  `harness_session_*`, the read-only `host_list`/`host_agents`/`host_read`
  tools and `host_send` sit on top. The host sends state changes as events; the
  supervisor treats an unreachable host as "unknown" (never "exited"), and alerts
  when an agent blocks. A coding agent's trust dialog defaults to "No, exit", so
  blocked launches are reported, never auto-answered. Setup, security model and
  pairing a machine: `docs/AGENT_HOST.md`, `docs/JOIN_A_MACHINE.md`.
- **The owner hears from HQ, not from helpers.** A helper a chat drives wakes that chat when
  it blocks; the owner's inbox gets an item only if the prompt outlives
  `agent_host.driver_grace_minutes` or nobody drives the session
  (`session_supervisor/alerts.rs`, `raise_unanswered`). The owner can silence a topic
  (helpers, tasks, tips, upkeep, email) or pause them all (`hq_db::inbox_prefs`,
  `/api/notifications/silences`): silenced items still land in the inbox, with no push, no
  phone notice and no badge count. A source nobody listed is HQ's own topic, which can never
  be silenced, so a new kind of approval is never hidden by accident.
- **Capabilities and the adoption digest**: `crates/hq-tools/assets/capabilities.yaml` lists what
  HQ can do in the person's words; `hq_tools::capabilities` resolves it against the install (model
  key, OCR, `gws`, registered tools) and what the owner's sessions used. It feeds the empty chat's
  chips (`GET /api/capabilities`, web and Android), a short usage-free "Things you can offer"
  block in the owner's own sessions, and the weekly device-only `adoption-digest` daemon task
  (`crates/hq-cli/src/commands/start/daemon/tasks_periodic/adoption_digest.rs`, queries in
  `hq_db::adoption`). `hq adoption` prints the same report and `hq offers replay` runs the phase 2
  offer detectors (`hq_tools::capabilities::offers`) over past chats, counts only
  (`crates/hq-cli/src/commands/adoption.rs`); `scripts/phone-hq.sh` reads both from a phone over the bridge
  (`GET /api/adoption`, `GET /api/offers/replay`, `crates/hq-web/src/adoption_api.rs`) with a device token.
  A reply in the web or phone chat may carry one offer (`chat_messages.meta.offer`, chosen on the
  device by `offers::for_turn`, answered through `POST /api/offers/answer`,
  `crates/hq-web/src/offers_api.rs`, kept in `hq_db::offers`); the model never sees it. When no rule fires and the turn worked on a task, `hq_tools::capabilities::next_step` may offer one next step by comparing the task with invented examples (`assets/next_step_examples.jsonl`) through the on-device embedder; no language model takes part. The
  `due-reminders` daemon task reminds about tasks made in chat. A new catalog tool must be registered (`crates/hq-mcp/tests/capability_catalog.rs`).
  Plan: `docs/plans/simple-hq.md`. Off with `daemon.adoption_digest: false`.

- **Argument checks are opt-in**: a tool that returns `true` from `HqTool::checks_arguments` has its
  arguments checked against `parameters()` before `execute` (`hq_tool_api::arg_check`), through
  `validate_and_execute`, which the MCP gateway and native sessions both use. A wrong, missing or
  mistyped argument gets a specific error with the closest real name and the call shape; `"5"` for 5 is
  fixed. Opt in only when the schema lists every argument `execute` reads: one it does not list is
  refused. Runtime `_hq_*` keys are never checked. Today: `session_search`, `tool_output_read`.

## Skills and document generation (optional)

HQ has its own markdown skills under `.vault/skills/`, validated by
`hq skills validate` and loaded into HQ sessions by hint match (see
`crates/hq-tools/src/skills/`). Skill self-improvement is one mechanism,
`hq_agent::skill_review`: after a session with enough tool calls, one LLM call
proposes up to three edits to skills HQ minted or the owner adopted
(`hq skills adopt|unadopt <name>` sets `managed: true`). Edits go through
`hq_tools::skill_edit`, are scanned for instruction overrides and opaque
payloads, and are audited in `skills/_audit.jsonl`. `hq skills revert <name>`
restores the newest archive and `hq skills history <name>` prints the trail.
`governance.skills_write_approval` (default off) stages every change in
`skills/_proposed/` for `hq skills approve|reject <name>`.

Skill evidence and tests: `skill_invocations` records the skill hash, task, purpose and
outcome of every load (ids and fixed values only, never prompt or document text),
`hq_db::skill_outcomes::settle` scores outcomes from observable acceptance rather than the
session exit, and `skills/<name>/fixtures.yaml` holds regression fixtures that gate edits. See
`docs/SKILL_FLYWHEEL.md`.

Document generation (docx, pptx, xlsx, pdf) is optional and depends on external
skills you install yourself. Brands are vault content, not a compiled-in list:
`hq_convert::brand::discover_brands` scans `Notebooks/Projects/*/Branding/` at
call time, so `brand_assets_list`, `brand_assets_add` and the `brand` parameter
of `convert_from_markdown` offer only the brands registered in that vault. Add
one with `Notebooks/Projects/<Name>/Branding/brand.yaml`.
`hq_convert::brand::BrandKit` (`crates/hq-convert/src/brand.rs`) resolves a brand
and `outbound.rs` uses it to pick a per-brand pandoc reference doc. The
`prose_lint` tool (`crates/hq-tools/src/prose_lint.rs`) is a mechanical check for
AI-sounding prose (`hq_core::prose_quality::SlopDetector`).
