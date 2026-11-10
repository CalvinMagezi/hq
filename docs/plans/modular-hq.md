# Modular HQ: a guarded base, modules on top

Status: steps 1 to 4 are built (branch `ccr-d92a93fe-s9n4lf`, commits `e381cbd`, `f6c14bc`,
`79e8132`, `63c795d`). Everything after that is a proposal, and several steps wait on the owner
decisions listed near the end.

## Where this came from

An advisor's note: define the base building blocks for the edge agent, guard that, and run
further experiments as plugins or modules so the base stays small and stable. The owner's
reply: the coding agent and the proactive features should not be packed into one massive
build, and plugins in independent repositories may be the better option because each may need
to scale on its own.

This plan agrees with the first half and argues against part of the second. It was worked out
by mapping the code as it is (nine readers, one subsystem each), writing three competing
designs (phone first, structure first, plugins first), and having three reviewers judge them
for owner protection, for what one maintainer can carry, and for value to a person on a phone.
All three designs and all three reviewers reached the same answer on repositories, which is
why it is stated as a decision here and not as an option.

## The problem

HQ is 25 crates and about 407,000 lines of Rust that build one `hq` binary. Four things are
wrong with its shape, and each was measured.

1. **The agent loop cannot be built without every tool.** `hq-agent` depends on `hq-tools`,
   which holds about 160 tools and links the coding-agent host, document import and note
   export without condition. By `cargo tree`, the loop's dependency closure is 661 crate
   versions, and 297 without `hq-tools`.
2. **Every edition is the same build.** `full`, `lite` and `mobile` are a run-time setting.
   The Android build links slightly more crate versions than the Linux server build (880
   against 868 by `cargo tree`) and leaves out only the public forwarder. Windows Lite links
   both chat-relay SDKs and never starts them.
3. **Every feature edits the same hand-written lists.** Two tool lists that already disagree
   (62 registration statements), a table of 34 timed tasks dispatched by matching strings, 137
   routes in one function, about twenty guard lists of tool names, and one config struct with
   57 sections. `hq-core` changed in 18 of the last 50 merges.
4. **Nothing checked direction.** Base code named module files in 61 places and nothing would
   have failed if that became 62.

Because every merge to `main` becomes a signed release that instances install within minutes,
a base that changes with every experiment means every experiment can reach a phone.

## Who gains, and how much

Stated plainly, because the first guess (a much smaller phone download) turned out to be mostly
wrong.

- **The phone owner on metered data.** What can never run on a phone (the agent-host server,
  the shell sandbox, the connector's TLS door, the private-network hub) is about 55,000 to
  60,000 lines of HQ's own code but only about 37 of 880 third-party crate versions: a few
  percent of the binary. The large weights are document import and export (about 275 crate
  versions, 9.6 MB of fonts), and the phone uses both. Megabytes will come from three places
  that are not the split: measuring (no binary or APK size is recorded anywhere today), taking
  Caddy out of release APKs (about 16 MB by the upstream archive size, not measured here), and
  how often a stable release is promoted. Nothing in this plan is shown to reduce memory or
  start time; nobody has measured those either.
- **The owner as maintainer.** This is where the split pays. A guarded base that experiments
  cannot change, one place to add or remove a feature, and per-crate builds that stop
  compiling Typst to test a task tool.
- **Someone who wants to build on HQ.** Today the only door is a remote MCP server, and the
  client cannot talk to standard servers (no handshake, no event-stream replies, no timeout).
  Fixing that is small and is in phase B.

## What "plugin" was hiding

The word covers three different things, and the evidence supports each to a different degree.

| Meaning | Unit | Verdict |
|---|---|---|
| How code is organised | crates and cargo features | Yes, for everything. This is most of the plan. |
| What runs as its own process | the agent host, the public forwarder, a third-party tool server | Yes, for those three. The first two already run this way. |
| Who owns and releases it | repositories | Not now. One repository, one version, one signed `hq` per platform. |

Why not separate repositories now:

- The updater unpacks exactly one file named `hq`, the signing job lists a fixed set of
  artifacts, and all crates share one version. A second binary (`hq-net-apply`) already exists
  and is left out of releases for that reason.
- Rust has no stable ABI, so a first-party module in another repository is a version pin and a
  paired release for every change that touches both. One person merges about ten changes a
  day.
- Android runs native code only from inside the signed APK. A phone can never download a
  module.
- The guard lists that protect the owner should sit next to the code they guard.

"Independent scaling" has something to scale against in only two places: the public forwarder
(one process, many devices) and the agent host (heavy work on a bigger machine). Both are
already separate processes. A hub is one owner, one vault and one SQLite file. Tasks, HQ
Coder, the session control plane, verification and the digests write that database inside
shared transactions and have nothing to scale against.

**When a piece earns its own repository.** All of these hold: its wire protocol has not
changed shape for 90 days; fewer than 1 in 20 merges over 60 days touch both it and anything
else; a conformance test can run against a released binary; and either a second person needs
merge rights on it or its operators want a slower release cadence. `hq-edge` is the first
candidate. Third-party plugins are different: they share no code with the hub, so they can live
anywhere from the first day.

## The shape

Four tiers. A crate depends only on tiers above its own in this list, or on crates listed
before it in its own tier. `layering.yaml` at the repository root is the source of truth and a
test enforces it.

| Tier | What | Crates today |
|---|---|---|
| contract | What every other crate agrees on. Small, and a change here is a change for everyone | `hq-core`, `hq-contract` |
| kernel | What every build of `hq` links: storage, models, the agent loop and its guards, base tools, the gateway, the scheduler, the web server, signed updates and restore | `hq-vault`, `hq-db`, `hq-llm`, `hq-memory`, `hq-update`, `hq-backup`, `hq-tools`, `hq-agent`, `hq-mcp`, `hq-daemon`, `hq-web` |
| module | What a build may leave out | `hq-sandbox`, `hq-host`, `hq-gate`, `hq-net`, `hq-tunnel`, `hq-convert`, `hq-export`, `hq-bridge`, `hq-edge`, `hq-relay` |
| app | Where the pieces are put together | `hq-cli`, `hq-desktop` |

**The base is the contract and the kernel. It is not small.** Once the modules that still live
inside kernel crates are walled off, it will be roughly 160,000 to 185,000 lines, because
tasks, the web server, sub-agents and the gateway are all things a phone uses. Small here
means the contract tier; for the kernel the promise is stable and guarded. A reader hoping for
a 20,000-line core should know that no honest reading of the code gives one.

Modules, in the project's own words (HQ Coder is HQ's own agent doing GitHub work; helpers are
other coding agents):

| Module | What it is | On a phone | Lives in today |
|---|---|---|---|
| helpers | Session control for other coding agents: host client, session tools, team messaging, supervisor | Yes (drives hosts elsewhere) | `hq-tools`, `hq-web`, `hq-cli` |
| host server | The pty host and the agent sandbox | No | `hq-host`, `hq-sandbox` |
| coder | HQ Coder jobs: the GitHub App, dispatch, write | Yes | `hq-tools`, `hq-web` |
| verify | Task verification (the completion guard itself stays with tasks) | Yes | `hq-tools`, `hq-db`, `hq-web` |
| documents | Import (`hq-convert`) and export (`hq-export`) | Yes, both in use | their crates, linked through `hq-tools` |
| connector | The claude.ai connector: OAuth and the TLS door | No (retired there) | `hq-web` |
| pairing | `hq-bridge` | Yes | its crate |
| net, tunnel, forwarder | `hq-net`, `hq-tunnel`, `hq-edge` | No (nothing on the phone starts the tunnel) | their crates |
| relays | Telegram and Discord | Yes | `hq-relay` |

Tasks are not a module. Lite is tasks plus notes, 65 files import them, and helpers, coder and
verify are all built on them. They are a named layer of the kernel.

**Build flavours** are fixed sets of cargo features on `hq-cli`, one per artifact already
released, so the updater does not change:

| Flavour | Artifact | Leaves out |
|---|---|---|
| full (default, today's build) | server and laptop tarballs, Docker | nothing |
| mobile | Android APK | host server and sandbox, connector, net hub, forwarder, and the tunnel until the app starts it |
| lite | Windows zip | relay SDKs, pairing, helpers, coder, host, connector, net (export, import and `hq verify` stay: the Lite command line allows them) |

No free combinations of features. Each one costs a CI pass, and a feature no build compiled
was already deleted once as rot. The run-time `profile` stays as the policy layer on top:
Lite's egress promise and Mobile's data saver are enforced and tested at run time and should
remain so.

**For other people's code**, the lane is out of process: a skill pack (text, kilobytes, works
on a phone) or a tool server reached over MCP. A plugin gets exactly what a remote MCP server
gets today: no HQ token, no vault access, every call asked, every result marked as outside
text, absent from unattended sessions. No native plugins, no dynamic loading, no
project-hosted registry.

## The guard

`cargo test -p hq-core --test layering` reads `layering.yaml`, every `Cargo.toml` and every
source file under `crates/`. It fails when:

- a crate depends on a crate its tier does not allow, or a module crate on another module
  crate without a declared edge;
- a crate has no tier (a new crate must be classified);
- code outside a module names the files of a module that still lives inside a kernel crate;
- a debt entry is no longer needed. The counts must match exactly, so a number can fall only
  in the change that fixes the code, and can never rise unseen.

Debt recorded on the day it landed: 6 crate edges (`hq-tools` to `hq-host`, `hq-gate`,
`hq-convert`, `hq-export`; `hq-web` to `hq-convert`, `hq-export`) and 61 source lines. Each
entry in the file says what removes it. The source check counts written paths, so it is a
tripwire and not a proof; the crate rule becomes the proof as each module is extracted.

Still to add to the guard: budgets for the phone build (phase A), a rule that a kernel crate
may depend on a module crate only as an optional dependency (phase D), and a short list of
kernel functions module code may not call (approval signing, building a governed registry,
`execute` without `validate_and_execute`).

## Rules that do not move

A split must not weaken any of these, and several steps below exist to make them hold by
construction.

- **Identity is never model-writable.** The markers live in `hq_core::caller`. The gateway and
  a native session both drop every `_hq_*` key a caller sent before setting their own.
- **The guard lists stay in the base.** A module never removes itself from one. The default
  for a tool no list mentions must become strict (see A5); today it is trusted.
- **One schema.** Every migration stays in `hq-db` in one ordered list, and every flavour
  applies all of them, so a phone backup restores on a server. A module owns queries, never
  table definitions. Empty tables cost nothing.
- **Lite and Mobile membership is decided by the kernel.** A module may ask to be in Lite; the
  kernel's pinned lists answer. Egress has no default: a module that sends data out must
  declare where.
- **No module installs its own sign-in or origin check.** Module routes live under
  `/api/<module>/` and inherit the kernel's.
- **Adoption data stays on the device**, and signature verification stays in the base.
- **The session control plane is not a separate process.** It writes task state inside the
  caller's transaction. The host process boundary is the right one and already exists.

## Steps

Sizes: S is a day or less, M a few days, L a week or more. "Change" says whether behaviour
changes for anyone.

### Done

| # | Step | Result |
|---|---|---|
| 1 | Caller markers into the base (`hq_core::caller`) | The one hard edge between base and the coder code is gone. Non-coder modules of `hq-tools` named `harness_session` on about 75 lines and now on one; `hq-mcp` on none |
| 2 | Layering guard (`layering.yaml`, `crates/hq-core/tests/layering.rs`) | Direction is checked on every test run; debt can only shrink. Tried on the real tree: adding `hq-host` to `hq-agent` fails |
| 3 | `hq-contract` holds the tool contract | `HqTool`, `ToolPolicy` and `ToolRegistry` sit below every crate that uses them. Three `hq-agent` files no longer mention `hq-tools`; 129 references to concrete tools remain |
| 4 | The gateway drops every runtime marker a caller sent | A marker added later cannot be forged through `/mcp`. Tests iterate every marker, and one that does not exist yet |

Steps 1 to 3 change no behaviour and make no build smaller. They are what the rest stands on.

### Phase A: measure and pin (all S)

Nothing later can be judged without these, and the snapshots are what make a refactor safe when
every merge ships.

| # | Step | Verifies |
|---|---|---|
| A1 | Record the phone build: `libhq.so` bytes, APK bytes with a per-component table (hq, Caddy, speech libraries, web assets, dex), and the Android crate-version count on every Rust change. Commit a budget file that fails a change when a number rises without the file changing. Record the wall time of the main CI job the same way | Every later claim about megabytes becomes a measurement |
| A2 | One measurement on a 3 to 4 GB phone: idle memory of hq, the bridge child and the app, and time from service start to `/health` | Whether memory and start time are worth targeting at all |
| A3 | A phone contract test: the six commands the Android app spawns, the API paths it calls and the eager tool list are present in the mobile feature set | A cut cannot remove something the phone relies on |
| A4 | Snapshots of today's tool sets and timed tasks, per surface, edition and audience, with behaviour probes and not names only: a restricted audience is refused a note outside its scope; the worker preset is exactly its allowlist; an unattended session holds no live-turn tool; a guest has no remote MCP tool; Lite's tools and tasks equal the pinned lists | Phases C and D can move assembly code without a silent change |
| A5 | Every registered tool appears in a reviewed classification (a test), listed names that match no tool fail, and a tool no list mentions is treated as an outside tool: it marks the session and asks the owner before it changes anything | Closes the fail-open default. No change for today's tools once all are classified |
| A6 | Both tool registries refuse a second tool with a name already registered | Allowlists are keyed by name, so a duplicate inherits another tool's reach |

### Phase B: cuts that each pay on their own

| # | Step | Size | Payoff | Change |
|---|---|---|---|---|
| B1 | Caddy out of release APKs. Debug builds keep it | S | About 16 MB off each install and update, to be confirmed by A1 | Only for an owner who pushes a Caddyfile to a release phone (owner decision 2) |
| B2 | Test that saving config keeps a section the struct does not know, then fix it | S | By reading, `save_patch` rewrites the file from the typed struct and would drop `edge:`. Needed before any module owns its config | Fixes a suspected live bug |
| B3 | Remote MCP client: timeouts, the `initialize` handshake, event-stream replies | M | Removes a hang on the phone-to-paired-hub link, and standard MCP servers start working | Yes, for the better |
| B4 | Timed tasks as a table: each entry carries its function, its editions and whether it costs a model call or data | M | Deletes the string match and three name lists. An unknown task can no longer count as a successful run | None |
| B5 | Three named flavours on `hq-cli`; `hq-net`, `hq-edge` and the direct iroh dependency behind features using the stubs that exist; Android builds `mobile`, Windows builds `lite` | M | First build of `hq-cli` without default features in CI. Relay SDKs leave Windows; net and tunnel leave the phone | The phone loses the unused tunnel command (owner decision 3) |
| B6 | `hq-host` gets a `server` feature. The client, names and hardened git stay always built | M | About 17,600 lines that cannot run on a phone stop being linked there | None on full. A test proves a no-server build's git command still carries its pins |
| B7 | The connector (OAuth and TLS) behind a feature in `hq-web`, with a status route that answers "off" | M | 8,600 lines and the ACME stack leave phone and Windows builds | None on full |
| B8 | A small `hq-edge` binary target for the forwarder's Docker image | S | The one piece that serves many devices stops compiling the whole hub (about 323 crate versions against 868) | None for installed forwarders, which stay on the signed `hq` |

CI for flavours: `cargo check -p hq-cli --no-default-features --features <flavour>` for
`mobile` and `lite` on a Linux host in a parallel job, plus the phone contract test and the
guard tests (governance, gateway allowlists, web sign-in and origin tables, the marker tests,
`adoption_stays_local`) built with each flavour's features. Full lint and the full test run
stay on the default build.

**Checkpoint after phase B.** Write down megabytes saved per step and CI minutes added.
Helpers, HQ Coder and verification stay in both the mobile and full flavours, so everything
after this point is justified by stability and maintainability, not by the phone. The owner
decides whether to continue.

### Phase C: the base stops naming modules

| # | Step | Size | Notes |
|---|---|---|---|
| C1 | Tools register through one list, converted in place a few groups per change, with the A4 snapshot unchanged each time | L, as four changes | The kernel loop keeps the audience work: scope wrappers, owner-only gating and the edition filter. A module's tools reach only the owner's unrestricted sessions unless a kernel list widens that |
| C2 | Governance stops importing module symbols: the few constants it needs move to the contract, approval text becomes a method on the tool trait, protected files become an additive registry | M | Drives `hq-agent -> coder` and most of `hq-agent -> helpers` in `layering.yaml` to zero |
| C3 | Task write guards (`restricted_caller`, the queue-write refusal, the reserved tag, the team access check) move from the coder and helper files into the base task tools | M | They must hold in every flavour. A Lite build without them would let a tasks-scope key plant work that a full instance on the same vault later runs |
| C4 | The one list moves to `hq-cli`; the default registry builder leaves `hq-mcp` | M | From here a module can be a crate above `hq-agent` |
| C5 | Web: a pure router plus a separate start for background loops; module routes merged before the kernel's layers; a hook is added only in the change that deletes the direct call it replaces (task changed, inbox answer, after reply, MCP caller kinds) | L | The inbox key namespace must be fixed first (see "Found along the way", item 2) |

The registration shapes live where their dependencies already are (a tool source beside
`hq-tools`, a task entry with the scheduler, a web part in `hq-web`) and `hq-cli` wires them.
`hq-contract` stays on `hq-core` only; it does not grow a module trait with web and CLI slots.

### Phase D: gate in place, extract on a trigger

| # | Step | Size |
|---|---|---|
| D1 | Teach the layering test that a kernel crate may depend on a module crate only as an optional dependency | S |
| D2 | helpers, coder, verify and connector become feature-gated folders, their walls in `layering.yaml` and their debt numbers driven to zero | L each, staged |
| D3 | Documents: an attachment reader in the base (inline text, pass images), then import and export behind features. Only if A1 says the size forces a choice | L |
| D4 | A module becomes its own crate only on a trigger: a shipped flavour drops it, or a second person owns it. Each extraction is a pure move that changes every call site in one change | L or more |

### Later, each as its own decision

- A base-owned table of tool risk replacing the name lists (A5 is its cheap half).
- Config sections owned by their module (B2 is its precondition). The 57-field struct is
  frozen meanwhile, not broken up.
- Skill packs a phone owner can add (open a file or scan a link with a hash, scanned and shown
  before approval). The one plugin shape that reaches a phone.
- A plugin manifest, per-plugin grants, sandboxed plugin processes and a plugins repository.
  Wait until two real plugins are in use.

### Not doing

Named so they are not rediscovered: separate repositories for first-party modules;
dynamically loaded plugins; a project-hosted plugin registry; per-module migration runners;
breaking up `HqConfig`; one merged tool trait; compile-time gates on the shell, LSP or the
update engine (drop them on Mobile at run time instead); a cargo feature per timed task; a
`chat` feature that removes the agent loop from Lite; separate signed binaries for the host or
forwarder before they can update under the same signature; a fourth "base only" release.

## How to run the split

- At most one split change open at a time.
- A refactor whose snapshot differs is reverted, not fixed forward. Every merge is a signed
  release.
- Phones and servers that matter follow the stable channel for the whole run.
- A device check only on the few changes that alter what the mobile build links.
- A step's size is checked against the map before it starts. Three designers sized the same
  helper extraction as L, L and XL.

## What success looks like

| Measure | Today | Target |
|---|---|---|
| Debt in `layering.yaml` | 6 crate edges, 61 lines | Falls every phase; zero for a module before it is extracted |
| Merges that change `hq-contract`, a guard file, or raise a number in `layering.yaml` or the budget file | not tracked | Rare, and each one deliberate. This replaces "share of merges touching `hq-core`", which cannot fall cheaply because most are config edits |
| Phone build: `libhq.so` and APK bytes, crate versions | not recorded (880 crate versions) | Recorded by A1, then only down |
| Phone contract and snapshots | none | Green on every change |
| `hq-agent`'s dependency closure | 661 crate versions | About 345 once helpers and documents are optional |
| Forwarder image closure | 868 crate versions | About 323 after B8 |
| Files that name modules | two tool lists, the task match, three task-name lists, the router, about twenty guard lists | one list in `hq-cli`, plus the guard data |

Stop rule: if phase B's Rust cuts save less than the number set in owner decision 1, stop
justifying structure by megabytes. Phases C and D continue only if the stability measures are
moving.

## Owner decisions

1. **A size budget for the phone**, in megabytes per APK and per update. Recommendation: set
   it from A1's first measurement and only ratchet down. Promotion cadence of the stable
   channel is a free lever that moves a metered user's cost more than any crate cut.
2. **Caddy out of release APKs?** Recommendation: yes, unless a release phone somewhere runs
   with a pushed Caddyfile.
3. **Drop the userspace tunnel from the phone build until the app starts it?**
   Recommendation: yes. Adding it back is one word in the flavour.
4. **Do document import and PDF export stay on the phone?** Recommendation: yes. The Notes
   screen offers export, and a phone-only owner is the person who most needs to hand someone a
   PDF. Revisit only if A1 shows the cost is out of proportion.
5. **Does Windows Lite keep `hq vault export`, import and `hq verify`?** Recommendation: keep
   them; dropping the relay SDKs alone is safe.
6. **Accept the strict default for a tool no guard list mentions (A5)?** Recommendation: yes.
   With agents writing much of the code, omission is the likely mistake.
7. **Are third-party plugins a goal for the next six months?** Recommendation: not yet beyond
   B3. A plugin story built on a base that is still moving will break its plugins.
8. **Which instances follow `main` while this runs?** Recommendation: the development machine
   only.
9. **Are the repository-split triggers above the ones to be held to?** If yes, start counting
   merges that touch `hq-edge` or `hq-host` together with anything else.

## Found along the way

Not part of the split. Each needs its own change. Items marked "by reading" were not executed.

1. **Possible disclosure gap for restricted audiences (by reading).** The vault scope wrapper
   wraps tools whose names start with `vault_` or `memory_`. `context_packet`, `note_review`
   and `note_provenance` come from the same factory and do not match; the shortcut tools
   (`vault_find` among them) are added after the wrapper; the export tools come from another
   factory. Whether another filter removes them for a family member's Discord session was not
   checked. Needs a test that builds that session's tool set and tries to read a private note.
2. **Inbox approval keys share a namespace with agent notices (by reading).** Approving any
   inbox item signs it, the approval gate reads by dedup key alone, and `notify_owner` lets
   the caller choose the key except for two reserved prefixes, neither of which is the
   owner-approval prefix. A session could file a notice under the key an approval would use,
   and one tap on that notice would let the matching call through. Fix before any module or
   plugin can register an inbox hook: store agent notices under a prefix the caller cannot
   choose, and sign only items from sources that ask.
3. **Guard lists trust a tool they do not mention.** Covered by A5.
4. **Two tests fail when run as root** (`coding::edit::tests`, both rely on a read-only file
   blocking a write). Confirmed in a cloud session. Other tests that use the same approach
   were not checked.
5. **Three call sites run a tool without its validation** (`hq-web/src/vault_api.rs`,
   `hq-web/src/coder_api.rs`, `hq-cli/src/commands/shortcuts.rs` call `execute` directly),
   by reading.
6. **Stale names in guard lists** (by reading): the file-touch tracker matches tool names the
   native file tools do not have, and the Telegram guest list denies five names that exist
   nowhere else.
7. **One harness has its trust dialog answered with Enter at spawn** (antigravity, by
   reading), which `CLAUDE.md`'s "never auto-answered" does not mention.
8. **Stale documents**: the README's crate count and line count; `capability-seams.md` lists
   `remote_mcp.rs` as removed; a CI comment says the bridge feature is off in the workspace
   run when it is in the default set; the runner runbook says releases are manual.
