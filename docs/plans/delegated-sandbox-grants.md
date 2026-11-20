# Delegated sandbox grants

Status: design, not built. Builds on the linked-worktree git fix (#679).

## Problem

A strict-sandbox helper that needs something outside its wall (the Gradle cache, a Flutter test
websocket, a package registry, a device) hits a raw OS denial. Today the only ways out are a
host-wide config edit, which widens every strict session, or the owner at a terminal. Helpers stall,
the owner is pulled in for routine things, and nothing is recorded.

Goal: a helper asks HQ for a precise, task-scoped capability. HQ decides deterministically whether to
grant it under a policy the owner set, applies it to that one session, and asks the owner only when
the request is risky, out of policy or unsupported. No bare denials, no dead ends, no host-wide
relaxation.

## Constraints from the code

- A sandbox is fixed at process start. Seatbelt and bubblewrap cannot widen a live process.
  `SandboxSpec` is stored on the host's session record (`crates/hq-host/src/state.rs`), and
  `Host::resume_with` (`host/restore.rs`) restarts from that stored record. A grant therefore means:
  amend the stored spec, restart the same session with its conversation resumed.
- No per-session override exists today. `LaunchRequest` has no sandbox field; the plan comes only from
  `agent_host.sandbox.*` (`agent_host/sandbox.rs`). The host RPC `agent.spawn` already accepts a
  `sandbox` value, so a per-launch spec is a small addition on the client side.
- Caller identity is attested by the MCP gateway (`attest_caller`, `_hq_caller_session`). A helper tool
  can trust which session called and must never take a session id from its arguments.
- Owner approvals already exist: a `ValueItem` of kind `ActionNeeded` with a dedup key, signed on
  approve (`value_items::sign_approvals`), answered through `POST /api/notifications/{id}/action`
  (`folder_request.rs`, `publish.rs`). The new flow reuses that store and those routes.
- The strict policy already refuses a writable root that contains HOME, the host run directory or a
  secret directory (`sandbox.rs` `strict_policy`). Those guards stay the last line of defence for
  every grant.
- Separate policy layers fail differently: the OS sandbox, the egress proxy, and the HQ builder
  command policy (it refused an `env` diagnostic). The broker names which layer denied.

## Model

### Capability types

Effects and resources, not tool names. Flutter, Gradle, Kotlin, npm and cargo are recipes over these.

| Type | Resource | Applied as |
|------|----------|-----------|
| `fs_write` | absolute path | `writable` root |
| `fs_read` | absolute path | `readable` entry |
| `loopback` | port, or any port for tests | `loopback_ports` (macOS) or refusal on bubblewrap |
| `egress` | host and port | `allow` rule on the proxy |
| `recipe` | name from a fixed catalog | command-policy allowance (stage 5) |
| `device` | adb serial plus read or install | brokered, never a sandbox widening (stage 5) |

### Risk classes, decided by a pure function

`classify(request, policy, task_context) -> Approve(grant) | Ask(reason) | Refuse(reason)`.
No model text enters it. Paths are canonicalised first.

- R0, approve under any opted-in profile: inside the task's worktree or repository; declared build
  caches from the profile (`~/.gradle`, `~/.pub-cache`, `~/.cargo`, `~/.npm`, the Android SDK read-only).
- R1, approve only when the profile lists it: loopback for the profile's test ports, egress to the
  profile's registries.
- R2, always ask the owner: any other path under HOME, any host not listed, device install or update,
  anything the classifier cannot map to a supported adapter.
- R3, refuse and never ask: credential directories, `~/.hq`, other agents' state, signing keys,
  publish, deploy, uninstall, wipe, spend, communications. Refusal text says what to do instead
  (for publishing, `harness_session_publish`).

A request cannot widen itself: a grant is a record with a fixed resource set, bound to one session id,
one worktree and one expiry. Asking for more is a new request. After three declined or refused
requests in a session HQ stops forwarding more and tells the helper to finish what it can and report.

### Standing policy and the task opt-in

Owner config `agent_host.grants.profiles`, each a named list of capabilities (for example
`mobile-dev`, `node-build`, `rust-build`), with defaults shipped for the common stacks and nothing
enabled until the owner opts in. A task, or a `harness_session_spawn` call, opts in with
`grant_profile`, bound to a host and a repository or worktree and an expiry. The profile is applied at
spawn, so the common case needs no restart and no ask at all. Absence of a profile keeps today's
behaviour exactly.

Every owner prompt offers a way to teach the policy: Allow for this task, Allow once, Always for this
repository (adds the capability to that repository's standing grant), or Decline. "Always" is how
repeat asks disappear without the owner editing config.

### Flow

1. Helper hits a denial. The helper's startup note says: on any sandbox or policy denial call
   `sandbox_request`, never work around it, never edit sandbox files.
2. `sandbox_request(capability, resources, reason, command?)`. Session identity comes from the
   gateway. `reason` is stored and shown as the helper's words, never used by the classifier.
3. `classify`. Approve: write the grant, amend the stored spec, restart the session with its
   conversation resumed, send it "grant G applied, retry". Ask: file the inbox request, set the task
   to waiting for the owner, tell the helper to park or continue other work. Refuse: reply with the
   reason and the safe alternative.
4. Owner answers on web or phone. Approval signs the request, the broker applies the grant exactly as
   in step 3. Decline returns the reason to the helper.
5. Expiry at session end or TTL, whichever is first. Revoke from any surface amends the spec back and
   restarts the session without the grant, or stops it if it cannot resume.

The helper's own `sandbox_request` call is in flight when the restart is due, so the tool replies
first ("approved, the session restarts when this turn ends"). The broker then waits for the session
to go idle, stops it, amends the spec, resumes it and sends the retry message. Restarting mid-call
would leave the resumed conversation with a dangling tool call.

Agents that cannot resume a conversation (some harnesses) get the grant on their next launch and the
request stays pending with a clear label. The owner sees the difference.

### Denials from every layer

"No bare denials" covers the OS sandbox, the egress proxy and the HQ builder command policy. In P1 a
command-policy denial is routed through the same request path and always resolves to Ask with the
layer named; auto approving named recipes comes in P5. A request whose layer the broker cannot
identify is Ask, never a silent refusal.

### Denial diagnostics

Helpers guess at what to ask for. Add `sandbox_denials`, which returns the recent denied paths and
ports for the caller's session (Seatbelt violations from the unified log, bubblewrap and proxy
refusals from the host), so a request names exact resources. The proxy already logs refused hosts.

### Host seam

New host RPC `agent.amend_sandbox(name, delta)` where `delta` is additive only (writable, readable,
loopback_ports, allow). The host re-validates against the hard guards in `strict_policy` and a ceiling
in `agent_host.grants.max`, so a bug in HQ cannot mint a root over a secret directory. The call is
allowed only on a stopped or waiting session, then `resume_with` applies it. Remote hosts get the same
RPC through the gate. Grants are a database table in HQ and a spec on the host; the host never trusts
HQ for more than the ceiling allows.

### Data and audit

Table `sandbox_grants`: id, session, task, host, repository, capability, resources (json), risk,
decided_by (`policy`, `owner`, `refused`), requested_at, decided_at, applied_at, expires_at,
revoked_at, helper_reason, denial_layer. Every state change is an audit event. Secret values and file
contents are never stored; paths are. Owner decisions reuse signed `ValueItem` approvals.

## UX

One rule: routine things never interrupt the owner, and anything that does shows exactly what,
why, for how long, and what happens if declined.

### Web (apps/hq-web)

- Inbox card for an access request, using `ApprovalCard`. Title in plain words ("Helper mobile-app-1 wants
  to use the Gradle cache"). Body: the exact resource and mode, the task, duration, risk label, and the
  helper's reason clearly marked as the helper's words, truncated and escaped. A line "Why HQ is asking"
  states the policy rule that stopped auto approval. Actions: Allow for this task, Allow once, Always
  for this repository, Decline. An expandable view shows the sandbox before and after.
- The same card appears in chat (`ChatApprovals`) when the owner is driving that session from chat.
- Session detail and `BlockedCallout`: a blocked helper shows "Waiting for access: <resource>" with a
  link, not a generic blocked prompt. Auto approvals appear as quiet chips on the session timeline
  ("HQ allowed Gradle cache, policy mobile-dev") with Revoke.
- Settings, Access section (extends the session folders list in `SettingsViews`): active grants with
  revoke, standing profiles with edit, repository rules learned from "Always", and a history of
  asks, approvals, refusals and revocations.
- Task create and dispatch: an "Allow development access" selector listing profiles, with a preview of
  exactly what the profile grants and for which repository and host. This is the opt-in.
- Strings go in `apps/strings/en.yaml` and the generator, never the generated files.

### Android

- Inbox renders the same item (`InboxScreen`, `InboxWords`) with the same four actions and the same
  wording from the shared strings file.
- The notification carries Allow once only for R0 and R1 through the existing `AlertPoster` and
  `AnswerReceiver` path. R2 opens the app. Web push keeps its no-action-button rule.
- A bottom sheet in the Helpers page (pattern of `GithubSheet`) lists a session's grants and revokes.
- The task dispatch sheet carries the same opt-in selector.
- Unanswered requests follow the existing rule: the owner's inbox gets the item once the driver grace
  passes, silenced topics stay silent, the task shows `waiting_for: owner`.

### What the owner never sees

A prompt for something the policy already covers; raw sandbox error text as the permission UX; a
second prompt for the same capability within a task.

## Stages

- P0 #679, git directories of a linked worktree. Merged first.
- P1 core, backend only: capability types, `classify` with a property-style test table, the
  `sandbox_grants` table, `SandboxDelta`, `agent.amend_sandbox` and resume, `sandbox_request` tool,
  profiles config with `grant_profile` on spawn, audit events, `hq grants list|revoke`.
- P2 owner UX on web: inbox card, session chips, Access settings, task opt-in.
- P3 Android parity.
- P4 denial diagnostics, the helper startup note, docs.
- P5 brokered recipes and device access (adb read, then pinned-serial install), loopback proxy for
  platforms that cannot open ports.
- P6 learning and review: "Always" rules, a weekly summary of what was auto approved.

## Verification

Unit tests for `classify` (every R-class, cross-session reuse, path escape through symlinks and `..`,
R3 refusals, expiry, revocation, the three-refusal stop). Host test that an amended spec reaches the
resumed process and that a root over a secret directory is rejected. Live check on this Mac with a
strict Flutter or cargo session: denial, request, auto grant, restart, retry succeeds. UI checked in a
browser and on the Android emulator. Not verifiable without hardware: bubblewrap loopback and a real
adb install; those are reported as unverified.

## Decisions for the owner

1. Ship defaults for `mobile-dev`, `node-build` and `rust-build`, or only an empty list to fill in.
2. Whether R1 may approve a loopback test with any port. A test runner may pick a random port, so
   exact ports can never match. First check what the runner binds and whether the Seatbelt profile
   can express `localhost:*`, and cover both bind and connect. On bubblewrap a server and client both
   inside the sandbox may already work on its private loopback; verify before calling it unsupported.
3. Session restart on grant is acceptable (the helper pauses a few seconds and resumes).
4. Android notification Allow once for R0 and R1 grants.

## Known risks

Restart loses an in-flight command. Some harnesses cannot resume. Seatbelt denial parsing depends on
macOS log access. Granting a shared cache lets one session affect the cache other sessions read.

## Review findings fixed in P1

An adversarial review of the first P1 diff found, and this branch fixes: a helper could opt itself
into a profile by editing its task's tags (the profile is now an owner-written binding only); a restart
could lose a conversation or kill sub-helpers (it is refused up front and the grant applies at next
launch); an approved grant could be left without its restart (the restart is now a row flag, not a return
value); requests never expired and could be dodged by respelling (expiry, normalised keys and a unique
row per live request); remote hosts used this machine's home (refused); case-insensitive disks and
more credential stores (case-folded compare, longer list); loopback grants did not reach HTTP clients
behind the proxy (private rules added); stored paths were not re-checked at launch (now they are).
Not done: a vault location in the deny list, and checking on a real Mac that `canonicalize` keeps the
typed case.

## Built: standing access (P6, owner-first flow)

Live use showed that editing config and binding a profile per task is not a flow anyone will follow.
The owner's first answer now teaches HQ. The inbox card offers Allow (this session), Allow for this task,
Allow for this repository, and a starter set when the request falls inside one (`suggested_starter`).
Starters ship in `hq_core::grants::STARTER_PROFILES`; profiles in config remain for people who want them.
Rules live in `sandbox_grants` under `task:` and `repo:` scopes and are read against the session
computer's home. The repository is the folder a worktree was made from. A summary endpoint counts
outcomes without paths. Still to do on the UI side: the card choices on web and Android, a rules list
with revoke in Settings > Access, and the weekly summary line.
