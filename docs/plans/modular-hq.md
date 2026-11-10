# Modular HQ: a guarded base, modules on top

Status: steps 1 to 4 are built, in the change that adds this file. Everything after that is a
proposal, and several steps wait on the owner decisions listed near the end.

## Where this came from

An advisor's note: define the base building blocks for the edge agent, guard that, and run
further experiments as plugins or modules so the base stays small and stable. The owner's
reply: the coding agent and the proactive features should not be packed into one massive
build, and plugins in independent repositories may be the better option because each may need
to scale on its own.

This plan agrees with the first half and argues against part of the second. It was worked out
by mapping the code as it is (nine readers, one subsystem each), writing three competing
designs (phone first, structure first, plugins first), having three reviewers judge them for
owner protection, for what one maintainer can carry and for value to a person on a phone, and
then having the built steps and this document checked by five more reviewers. All three
designs and all three judges reached the same answer on repositories, which is why it is the
recommendation in owner decision 1 and not one option among several.

## The problem

HQ is 25 crates and about 407,000 lines of Rust, tests included, that build one `hq` binary.
Four things are wrong with its shape. Crate counts below were taken on `main` before step 3
added a crate; each is one higher now.

1. **The agent loop cannot be built without every tool.** `hq-agent` depends on `hq-tools`,
   which holds about 155 tools and links the coding-agent host, document import and note
   export without condition. By `cargo tree`, the loop's dependency closure is 661 crate
   versions, and 286 without `hq-tools`.
2. **On Unix, every edition is the same build.** `full`, `lite` and `mobile` are a run-time
   setting. The Android build links slightly more crate versions than the Linux server build
   (880 against 868) and leaves out only the public forwarder. The Windows build already
   leaves out pairing, the forwarder, the gate and the private network by target, and still
   links both chat-relay SDKs that it never starts.
3. **Every feature edits the same hand-written lists.** Two tool lists that already disagree
   (65 registration statements), a table of 34 timed tasks dispatched by matching strings, 137
   routes in one function, about thirty guard lists of tool names, and one config struct with
   57 top-level fields. In the 49 merges this was measured over, 17 changed `hq-core` and 14
   edited one of the four hand-written assembly lists.
4. **Nothing checked direction.** Base code named module code in 187 places and nothing would
   have failed if that became 188.

Because every merge to `main` becomes a signed release that instances install within minutes,
a base that changes with every experiment means every experiment can reach a phone.

## Who gains, and how much

Stated plainly, because the first guess (a much smaller phone download) turned out to be mostly
wrong.

- **The phone owner on metered data.** What this plan takes out of the phone build (the
  agent-host server and its sandbox, the connector, the private-network code) is about 43,000
  lines of HQ's own code, tests included, and somewhere between 20 and 40 of about 880
  third-party crate versions depending on how they are counted. What share of the binary that
  is has not been measured. The large weights are document import and export (about 275 crate
  versions, 9.6 MB of fonts), and the phone uses both. Megabytes will come from three places
  that are not the split: measuring (each release's channel file records the APK's total size,
  but nothing records `libhq.so`, what each part costs, or checks a size), taking Caddy out of
  release APKs (about 16 MB by the upstream archive size, not measured here), and how often a
  stable release is promoted. Nothing in this plan is shown to reduce memory or start time;
  nobody has measured those.
- **The owner as maintainer.** This is where the split pays: a guarded base that an experiment
  cannot change, and one place to add or remove a feature. It does not buy faster per-crate
  builds unless documents become optional (step D3), which this plan does not recommend for
  the phone.
- **Someone who wants to build on HQ.** Today the only door is a remote MCP server, and the
  client cannot talk to standard servers (no handshake, no event-stream replies, no timeout).
  Fixing that is small and is in phase B. Even after the whole plan, a first-party feature
  that needs config still edits `hq-core` and one that needs a table still edits `hq-db`.

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
changed shape for 90 days (step A7's fixture is what shows this); fewer than 1 in 20 merges
over 60 days touch both it and anything else; a conformance test can run against a released
binary; and either a second person needs merge rights on it or its operators want a slower
release cadence. `hq-edge` is the first candidate. Third-party plugins are different: they
share no code with the hub, so they can live anywhere from the first day.

## The shape

Four tiers. In the contract and kernel tiers a crate depends only on tiers above its own in
this list, or on crates listed before it in its own tier. A module crate depends on the base,
and on another module crate only through a declared edge. `layering.yaml` at the repository
root is the source of truth and a test enforces it.

| Tier | What | Crates today |
|---|---|---|
| contract | What every other crate agrees on. Small, and a change here is a change for everyone | `hq-core`, `hq-contract` |
| kernel | What every build of `hq` links: storage, models, the agent loop and its guards, base tools, the gateway, the scheduler, the web server, signed updates and restore | `hq-vault`, `hq-db`, `hq-llm`, `hq-memory`, `hq-update`, `hq-backup`, `hq-tools`, `hq-agent`, `hq-mcp`, `hq-daemon`, `hq-web` |
| module | What a build may leave out | `hq-sandbox`, `hq-host`, `hq-gate`, `hq-net`, `hq-tunnel`, `hq-convert`, `hq-export`, `hq-bridge`, `hq-edge`, `hq-relay` |
| app | Where the pieces are put together | `hq-cli`, `hq-desktop` |

**The base is the contract and the kernel. It is not small.** With the modules that still live
inside kernel crates walled off, it is roughly 225,000 of the 407,000 lines, because tasks,
the web server, sub-agents and the gateway are all things a phone uses. Small here means the
contract tier; for the kernel the promise is stable and guarded. A reader hoping for a
20,000-line core should know that no honest reading of the code gives one.

Modules, in the project's own words (HQ Coder is HQ's own agent doing GitHub work; helpers are
other coding agents):

| Module | What it is | On a phone | Lives in today |
|---|---|---|---|
| helpers | Session control for other coding agents: host client, session tools, team messaging, supervisor | Yes (drives hosts elsewhere) | `hq-tools`, `hq-web`, `hq-cli`, queries in `hq-db` |
| host server | The pty host and the agent sandbox | No | `hq-host`, `hq-sandbox` |
| coder | HQ Coder jobs: the GitHub App, dispatch, write | Yes | `hq-tools`, `hq-web`, queries in `hq-db` |
| verify | Task verification (the completion guard itself stays with tasks) | Yes | `hq-tools`, `hq-db`, `hq-web` |
| documents | Import (`hq-convert`) and export (`hq-export`) | Yes, both in use | their crates, linked through `hq-tools` and `hq-web` |
| connector | The claude.ai connector: OAuth and the TLS door | Allowed by the profile; nothing in the app starts it | `hq-web` |
| pairing | `hq-bridge` | Yes | its crate |
| net, tunnel, forwarder | `hq-net`, `hq-tunnel`, `hq-edge` | No (the tunnel is linked and nothing starts it) | their crates |
| relays | Telegram and Discord | Yes | `hq-relay` |

Tasks are not a module. Lite is tasks plus notes, 65 files import them, and helpers, coder and
verify are all built on them. They are a named layer of the kernel.

**Build flavours** are fixed sets of cargo features on `hq-cli`, one per artifact already
released, so the updater does not change:

| Flavour | Artifact | Leaves out |
|---|---|---|
| full (default, today's build) | server and laptop tarballs, Docker | the userspace tunnel, off by default today |
| mobile | Android APK | host server and agent sandbox, connector, net hub, forwarder, and the tunnel until the app starts it |
| lite | Windows zip | relay SDKs, helpers, coder, host, connector (pairing and net are already out by target; export, import and `hq verify` stay, because the Lite command line allows them) |

No free combinations of features. Each one costs a CI pass, and a feature no build compiled
was already deleted once as rot. The run-time `profile` stays as the policy layer on top:
Lite's egress promise and Mobile's data saver are enforced and tested at run time and should
remain so.

**Where an experiment goes.** A new first-party idea starts as a module from its first commit:
its own folder or crate, listed in `layering.yaml`, behind a cargo feature that only the
`full` flavour turns on. Its tools need an entry in the classification from step A5 before the
build passes, and that entry is a guard-file edit the owner sees. It reaches a phone only by a
one-word edit to the `mobile` flavour, which triggers the Android build and the budget check.
An experiment that is dropped is a deleted folder and a deleted line. There is no `lab`
flavour until an experiment needs one.

**For other people's code**, the lane is out of process: a skill pack (text, kilobytes, works
on a phone) or a tool server reached over MCP. A plugin gets exactly what a remote MCP server
gets today: no HQ token, no vault access, every call asked, every result marked as outside
text, absent from unattended sessions. No native plugins, no dynamic loading, no
project-hosted registry.

## The guard

`cargo test -p hq-core --test layering` reads `layering.yaml`, asks cargo for the workspace's
crates and their dependencies, and reads every file under `src/` of the contract, kernel and
module crates. It fails when:

- a crate depends on a crate its tier does not allow, or a module crate on another module
  crate without a declared edge;
- a crate has no tier (a new crate must be classified);
- code outside a module names the module's files, or base code names a module crate without
  belonging to a module that may;
- a debt entry is no longer needed. The counts must match exactly, so a number can fall only
  in the change that fixes the code, and a rise is always a visible edit to `layering.yaml`.

Debt recorded on the day it landed: 6 crate edges (`hq-tools` to `hq-host`, `hq-gate`,
`hq-convert`, `hq-export`; `hq-web` to `hq-convert`, `hq-export`) and 187 places in source:
149 where base code names a module's files and 38 where it names a module crate directly.
Each entry in the file says what removes it.

What the count is and is not:

- It reads paths as written: full paths, every path a `use` imports, `self::` and `super::`
  paths, and a child module named bare by its parent. It does not see a name used after it
  was imported, a rename with `as`, a file mounted with `#[path]` or a `tests/` folder, and it
  counts unit tests under `src/`. It is a tripwire, not a proof.
- It leaves out `hq-cli`, where modules are put together. About 77 more lines there name
  helpers, coder or verify files. They are not debt, but step D2 has to gate each one.
- The walls cover `hq-tools` and `hq-web`. Module queries in `hq-db` and module config in
  `hq-core` stay in the base on purpose (see "Rules that do not move").
- A module is ready to be left out when the build without it compiles, not when its numbers
  reach zero.
- Nothing yet stops a change from raising a number together with the code. Step A8 adds that.

Still to add: budgets for the phone build (A3), the check against the merge base (A8), a rule
that a kernel crate may depend on a module crate only as an optional dependency (D1), and a
short list of kernel functions module code may not call (approval signing, building a governed
registry, `execute` without `validate_and_execute`).

## Rules that do not move

A split must not weaken any of these, and several steps below exist to make them hold by
construction.

- **Identity is never model-writable.** The markers live in `hq_core::caller`. A native
  session drops every `_hq_*` key the model sent before setting its own. The gateway does the
  same for its callers, except `_hq_spawned_session`, which can only take reach away.
- **The guard lists stay in the base.** A module never removes itself from one. The default
  for a tool no list mentions must become strict (A5); today it is trusted.
- **One schema.** Every migration stays in `hq-db` in one ordered list, and every flavour
  applies all of them, so a phone backup restores on a server. A module owns queries, never
  table definitions. Empty tables cost nothing.
- **Lite and Mobile membership is decided by the kernel.** A module may ask to be in Lite; the
  kernel's pinned lists answer. Egress has no default: a module that sends data out must
  declare where.
- **No module installs its own sign-in or origin check**, and no module adds a kind of `/mcp`
  credential. Module routes live under `/api/<module>/` and inherit the kernel's checks.
- **Adoption data stays on the device**, and signature verification stays in the base.
- **The session control plane is not a separate process.** It writes task state inside the
  caller's transaction. The host process boundary is the right one and already exists.

## Steps

Sizes: S is a day or less, M a few days, L a week or more. "Change" says whether anything
changes for a person or a caller.

### Done

| # | Step | Result | Change |
|---|---|---|---|
| 1 | Caller markers into the base (`hq_core::caller`) | The markers no longer live in the helpers code. Other `hq-tools` files named `harness_session` on about 77 lines and now on one. In `hq-mcp` the gateway no longer names it; the tool registry still does, until C4 | None |
| 2 | Layering guard (`layering.yaml`, `crates/hq-core/tests/layering.rs`) | Direction is checked on every test run and debt can only shrink. Tried on the real tree: a new dependency, a renamed one and a grouped import of a module file each fail | None |
| 3 | `hq-contract` holds the tool contract | `HqTool`, `ToolPolicy` and `ToolRegistry` sit below every crate that uses them. Three `hq-agent` files no longer mention `hq-tools`; 129 references to concrete tools remain | None. No build gets smaller from this alone |
| 4 | Markers are dropped and passed on by prefix, and a call that cannot carry them is refused | A marker added later cannot be forged through `/mcp`, inside a bulk task item or through the `hq` alias. A call whose arguments are a list, a string or a number is refused: before, it reached the tool with no marker, and a session token or a connector grant was then read as the owner for tools that list things | Yes: an unknown `_hq_*` argument no longer reaches a tool; `hq_call` with non-object `args` is an error; a `_hq_caller_session` typed into the local `hq task` command is dropped |

### Phase A: first, three suspected live defects

Each is its own pull request and none depends on the others. They come first because they are
on `main` today and because later steps stand on them.

| # | Step | Size | Change |
|---|---|---|---|
| A0 | A test that builds a family member's Discord session and tries every note tool against a private note (see "Found along the way", item 1). If anything leaks, close it so the default is refusal: a tool that reads notes and is not known to the scope wrapper is refused for a restricted audience | S to M | Only if a leak is found |
| A1 | A test that saving config keeps a section the struct does not know, then the fix. By reading, `save_patch` rewrites the file from the typed struct, so saving a budget from the web app on a machine that also runs a forwarder would delete its `edge:` section. Also the precondition for any module owning its config | S | Fixes the loss |
| A2 | Inbox approval keys (item 2): agent notices are stored under a prefix the caller cannot choose, only items from sources that ask can be signed as approvals, and a test proves `notify_owner` refuses every prefix an approval is read under | M | An agent can no longer file a notice under an approval's key |

### Phase A, continued: measure and pin

Nothing later can be judged without these, and the snapshots are what make a refactor safe when
every merge ships.

| # | Step | Size | Change |
|---|---|---|---|
| A3 | Record the phone build where it is built: `libhq.so` bytes and a per-component table of the APK (hq, Caddy, speech libraries, web assets, dex) in the Android job summary, and a one-off attribution of `libhq.so` by crate, because the questions this plan sends here are per crate (what do documents, relays and iroh cost). The Windows zip the same way. The Android crate-version count on every Rust pull request with `cargo tree`, which needs no NDK. One line of data cost per month: APK bytes times stable promotions, plus the one-time embedder download. A budget file holds ceilings: crate versions exact; bytes rounded up to the next megabyte, checked on pull requests that build Android and reported on `main` as a warning and an inbox item, never as a failed release. The wall time of the main CI job is recorded the same way | S | None |
| A4 | One measurement on a 3 to 4 GB phone: idle memory of hq, the bridge child and the app, and time from service start to `/health` | S | None |
| A5 | A classification every registered tool must appear in, read by the governance code at run time and checked by a test. A tool it does not mention is treated as an outside tool: it marks the session, asks the owner before it changes anything, and stays out of unattended sessions. Names in a guard list that match no tool fail the test, so the five stale names in the Telegram guest list and the file-touch tracker's wrong names are fixed here. The full `/mcp` key and a paired device still serve every registered tool; that is not closed by this step | M | None for today's tools once all are classified. A new tool with no entry is strict |
| A6 | Snapshots of today's tool sets and timed tasks, per surface, edition and audience, and for tasks with the data saver on and off and the embedder local and remote. Behaviour probes and not names only: the worker preset is exactly its allowlist; an unattended session holds no live-turn tool; a guest has no remote MCP tool; Lite's tools and tasks equal the pinned lists; the restricted-audience probe from A0. The snapshots are tests in `hq-cli`, where every tool is linked, so C4 does not move the test it relies on | M | None |
| A7 | Pin the host wire protocol: a fixture that holds version 1's request and reply lines byte for byte. Phones on stable with a laptop on `main` is the standing arrangement during this plan, so an old client driving a new host is normal | S | None |
| A8 | A CI step compares `layering.yaml` and the budget file with the merge base and fails when a debt number or a ceiling is higher or a debt entry is new, unless the pull request carries a label only the owner applies. The guard files join `CODEOWNERS` for outside contributors | S | None |
| A9 | A phone contract test: the six commands the Android app spawns, the API paths it calls and the eager tool list are present in the mobile feature set | S | None |
| A10 | Both tool registries stop a second tool from taking a name already registered: it is skipped with a warning, never a failed start, since remote tool names come from config and from paired hubs | S | A duplicate no longer silently replaces the first |

### Phase B: cuts that each pay on their own

| # | Step | Size | Payoff | Change |
|---|---|---|---|---|
| B1 | Caddy out of release APKs. Debug builds keep it | S | About 16 MB off each install and update, to be confirmed by A3 | None by any documented route: `adb run-as`, the only described way to place a Caddyfile, works on debug builds only (owner decision 3) |
| B2 | Remote MCP client: timeouts, the `initialize` handshake, event-stream replies | M | Removes an unbounded wait on the phone-to-paired-hub link, and standard MCP servers start working | A server that never answers now fails instead of hanging |
| B3 | Timed tasks as a table: each entry carries its function, its editions, and one of three metered rules (always runs; skipped under the data saver; skipped under the data saver unless the embedder is local, which is inbox triage today) | M | Deletes the string match and three name lists. An unknown task can no longer count as a successful run | None, proven by the A6 task snapshot |
| B4 | Three named flavours on `hq-cli`; `hq-net`, `hq-edge` and the direct iroh dependency behind features using the stubs that exist; Android builds `mobile`, Windows builds `lite`. The Windows job in `ci.yml` builds, tests and smoke-tests `lite` from this step on | M | First build of `hq-cli` without default features in CI. Relay SDKs leave Windows (34 of 769 crate versions); net and tunnel leave the phone | The phone loses the tunnel command nothing starts (owner decision 4) |
| B5 | `hq-host` splits in two. A small client crate with no HQ dependency holds the wire format, the client, the token, folder names and hardened git, and joins the kernel tier before `hq-tools`. `hq-host` keeps the pty server and depends on it | M | About 12,000 lines of non-test code that cannot run on a phone stop being linked there, and the `hq-tools -> hq-host` debt edge goes | None on full, and the A7 fixture is unchanged. A test proves that git run by a build without the server still has every program a repository can name switched off |
| B6 | The connector (OAuth and TLS) behind a feature in `hq-web`, with a status route that answers "off" | M | About 4,350 lines of non-test code and the ACME stack leave phone and Windows builds | None on full. The phone build loses the connector that the Mobile profile allows and the app never starts (owner decision 5) |
| B7 | A small `hq-edge` binary target for the forwarder's Docker image, with a `cargo check` of it in CI since no workflow builds that image | S | The one piece that serves many devices stops compiling the whole hub (about 375 crate versions with `hq edge invite`, which the image needs, against 868) | `docker exec hq-edge hq edge allow|revoke|list|invite` must keep working in the image; installed forwarders stay on the signed `hq` |

A feature that a flavour turns off is off by default in its library crate and turned on by the
flavour in `hq-cli`. Two consequences. Per-crate test runs need the feature named
(`cargo test -p hq-web --features connector`), and `CLAUDE.md` changes in the same pull
request. And each cut is proven in the flavour job: `cargo tree -p hq-cli
--no-default-features --features mobile -i portable-pty` (B5) and `-i instant-acme` (B6) must
print nothing.

CI for flavours: `cargo check -p hq-cli --no-default-features --features <flavour>` on a Linux
host in a parallel job, plus the phone contract test and the guard tests (governance, gateway
allowlists, web sign-in and origin tables, the marker tests, `adoption_stays_local`) built
with each flavour's features. Full lint and the full test run stay on the default build. A
Linux check of `mobile` does not compile Android-only code (about 20 lines in 11 files), and a
Linux check of `lite` covers only the Unix side. The Android build already runs on `main` and
weekly; a failure there means a release without an APK, not a bad one.

**Checkpoint after phase B.** Write down megabytes saved per step and CI minutes added.
Helpers, HQ Coder, verification, documents, pairing and relays stay in the mobile flavour, so
everything after this point is justified by stability and maintainability, not by the phone.
By the sizes below, phases C and D to the end of D2 are at least eight working weeks of one
person, with at most one split change open at a time and the devices that matter on stable
throughout. That is the price the stability measures have to justify, and the owner decides
whether to pay it.

### Phase C: the base stops naming modules

| # | Step | Size | Change |
|---|---|---|---|
| C1 | Tools register through one list, converted in place a few groups per change. The kernel loop keeps the audience work: scope wrappers, owner-only gating and the edition filter. A module's tools reach only the owner's unrestricted sessions unless a kernel list widens that | L, as four changes | None, proven by the A6 snapshot each time |
| C2 | Governance stops importing module symbols: the few constants it needs move to the contract, approval text becomes a method on the tool trait, protected files become an additive registry | M | None |
| C3 | Task write guards (the restricted-caller check, the queue-write refusal, the reserved tag, the team access check) move from the coder and helper files into the base task tools, and the restricted-caller check is inverted: a call is owner-level only when the gateway set a positive marker for the full key | M | The guards hold in every flavour. A Lite build without them would let a tasks-scope key plant work that a full instance on the same vault later runs |
| C4 | The one list moves to `hq-cli`; the default registry builder leaves `hq-mcp` | M | None |
| C5 | Web: a pure router plus a separate start for background loops; module routes merged before the kernel's layers; non-GET module routes require the client header unless a kernel list exempts them; three hooks (task changed, inbox answer, after reply), each added only in the change that deletes the direct call it replaces. Needs A2. The check of who is calling `/mcp` stays a kernel list | L | None |

The registration shapes live where their dependencies already are (a tool source beside
`hq-tools`, a web part in `hq-web`) and `hq-cli` wires them. The task entry type needs a home
below the app tier, since nothing may depend on `hq-cli`; `hq-daemon` is the candidate.
`hq-contract` stays on `hq-core` only and does not grow a module trait with web and CLI slots.

### Phase D: gate in place, extract on a trigger

| # | Step | Size |
|---|---|---|
| D1 | Teach the layering test that a kernel crate may depend on a module crate only as an optional dependency | S |
| D2 | helpers and coder become feature-gated folders, off in `lite`, with their debt driven to zero. Done when `cargo check -p hq-cli --no-default-features --features lite` passes, not when the number is zero. Verification gets the same walls, and a feature only if owner decision 7 drops `hq verify` from Windows Lite, since a feature every flavour turns on has no build that proves the gate. The connector's feature exists since B6; here its last references in the web kernel go | L each for helpers and coder, M for the other two |
| D3 | Documents: an attachment reader in the base (inline text, pass images), then import and export behind features. Only if A3 says the phone must choose, or owner decision 7 drops export from Windows Lite | L |
| D4 | A module becomes its own crate only when a second person owns it, or when its gate has failed to hold (base code named its files again in two changes after its debt reached zero). A flavour dropping it is not a trigger: the gate already does that. Each extraction is a pure move that changes every call site in one change | L or more |

### Later, each as its own decision

- A base-owned table of tool risk replacing the guard lists (A5 is its cheap half). With it,
  a tool with no entry would not be served on any `/mcp` credential, the full key included.
- Config sections owned by their module (A1 is its precondition). New sections keep going
  into `HqConfig` until then.
- Skill packs a phone owner can add: open a file or scan a link with a hash, shown before
  approval. Third-party skill text is instructions, so a pack needs an origin field, is never
  loaded into an unattended or worker session, and loading one marks the session. This is the
  one plugin shape that reaches a phone, and the first step to take if owner decision 9 is yes.
- A plugin manifest, per-plugin grants, sandboxed plugin processes and a plugins repository.
  Wait until two real plugins are in use.
- If web push becomes a module, the kernel hands it only items that may leave the device. The
  adoption guard becomes a rule in the layering test: any file that names the adoption
  queries must be on the guarded list.

### Not doing

Named so they are not rediscovered: separate repositories for first-party modules;
dynamically loaded plugins; a project-hosted plugin registry; per-module migration runners;
breaking up `HqConfig`; one merged tool trait; compile-time gates on the shell, LSP or the
update engine (drop them on Mobile at run time instead); a cargo feature per timed task; a
`chat` feature that removes the agent loop from Lite; a `lab` feature before an experiment
needs one; separate signed binaries for the host or forwarder before they can update under the
same signature; `hq-net-apply` in the signed manifest; a fourth "base only" release.

## How to run the split

- One step per pull request. A change in behaviour never shares a pull request with a
  refactor.
- At most one split change open at a time, and a branch that touches `layering.yaml` must be
  up to date before it merges: two changes that each lower the same number merge cleanly and
  leave `main` failing.
- A refactor whose snapshot differs is reverted, not fixed forward. Every merge is a signed
  release.
- Never raise a number or add a debt entry in `layering.yaml` to make the test pass. Invert
  the dependency.
- Phones and servers that matter follow the stable channel for the whole run. A change to what
  the mobile build links gets a device check, then a small rollout through `promote.yml`'s
  percentage, with `android-withdraw.yml` ready. A phone cannot go back to an older APK, so
  the fix is always a newer build.
- A step's size is checked against the map before it starts. Three designers sized the same
  helper extraction as L, XL and XL.

## What success looks like

| Measure | Today | Target |
|---|---|---|
| Debt in `layering.yaml` | 6 crate edges, 187 places | Unchanged through phases A and B except the host edge (B5); falls from C2 |
| Merges that edit a hand-written assembly list (the two tool lists, the task list, the router) | 14 of the last 49 | Falls through phase C. This is the number that says the split is paying |
| Merges that edit a guard file (governance, the gateway allowlists, sign-in and origin) | 8 of the last 49 | Not above that after A5, although every new tool then edits the classification |
| Phone build: `libhq.so` bytes, APK bytes, crate versions | APK total in each release's channel file; the rest not recorded (880 crate versions) | Recorded by A3, then only down |
| Phone contract and snapshots | none | Green on every change |
| `hq-agent`'s dependency closure | 661 crate versions | About 650 unless documents become optional (D3), then about 325 |
| Forwarder image closure | 868 crate versions | About 375 after B7 |
| Files that name modules | two tool lists, the task match, three task-name lists, the router, about thirty guard lists | one list in `hq-cli`, plus the guard data |

Stop rule. Before B4 the owner sets a floor, in megabytes off the APK, for what B4 to B6
together must save (owner decision 2). If they save less, megabytes are no argument for any
later step. Phases C and D continue only while the assembly-list number falls. A falling debt
count does not count, because it falls whenever the work is done.

## Owner decisions

1. **One repository now**, with the stated triggers for a piece to leave. Recommendation: yes.
   If so, start counting merges that touch `hq-edge` or `hq-host` together with anything else.
2. **Two numbers for the phone**: a ceiling in megabytes per APK, set from A3's first
   measurement and only ratcheted down, and the floor for the stop rule above. How often
   stable is promoted is a free lever that moves a metered user's cost more than any crate
   cut.
3. **Caddy out of release APKs?** Recommendation: yes. No supported route puts a Caddyfile on
   a release phone.
4. **Drop the userspace tunnel from the phone build until the app starts it?**
   Recommendation: yes. After this no released build contains `hq-tunnel`; it is compiled only
   by the CI feature jobs, which must stay, or the feature should be deleted until the app
   work lands.
5. **Take the connector out of the phone build?** The Mobile profile allows it today and the
   app has no screen that starts it. Recommendation: yes, with the tunnel.
6. **Do documents and the chat relays stay on the phone?** Recommendation: yes to both. The
   Notes screen offers export, a phone-only owner is the person who most needs to hand someone
   a PDF, and Telegram is how many people already talk to software. Revisit only if A3 shows a
   cost out of proportion.
7. **Does Windows Lite keep `hq vault export`, import and `hq verify`?** Recommendation: keep
   them; dropping the relay SDKs alone is safe. If export is dropped there, export can go
   behind a feature with `lite` as its off build and no extra CI pass, which is the one route
   by which per-crate builds stop compiling Typst.
8. **Accept the strict default for a tool no guard list mentions (A5)?** Recommendation: yes.
   With agents writing much of the code, omission is the likely mistake.
9. **Are third-party plugins a goal for the next six months?** Recommendation: not beyond B2.
   A plugin story built on a base that is still moving will break its plugins.
10. **During phase C, run the Android build on every Rust pull request** (one NDK build each),
    or accept finding an Android-only break on `main`? And which instances follow `main`:
    recommendation, the development machine only.
11. **A label only the owner applies** as the one way to raise a number in `layering.yaml` or
    the budget file (A8)? Recommendation: yes. Pull requests come from one account, so a
    code-owner rule alone cannot make a second look happen.

## Found along the way

Not part of the split. Items marked "by reading" were not executed.

1. **Possible disclosure gap for restricted audiences (by reading).** The vault scope wrapper
   wraps tools whose names start with `vault_` or `memory_`. `context_packet`, `note_review`
   and `note_provenance` come from the same factory and do not match; the shortcut tools
   (`vault_find` among them) are added after the wrapper; the export tools come from another
   factory. Whether another filter removes them for a family member's Discord session was not
   checked. Step A0 is the test.
2. **Inbox approval keys share a namespace with agent notices (by reading).** Approving any
   inbox item signs it, the approval gate reads the newest item by dedup key alone, and
   `notify_owner` lets the caller choose the key except for two reserved prefixes. None of
   the four prefixes an approval is read under is reserved: `owner-approval:`, `connector:`,
   `folder-grant:` and `self-update-approval-`. No button in the web app or the phone app
   approves a notice, so this is not one tap. A notice filed under an approval's key can be
   approved by the action route called directly, by `approve <token>` in a relay (the tool
   returns the item's id, so a session can name the token it wants typed) or by
   `hq queue approve`. A pending notice under that key also makes the gate wait, so the real
   request is never filed, and clearing it reads as a decline. Step A2 is the fix, and it must
   hold for all four.
3. **Guard lists trust a tool they do not mention.** Covered by A5.
4. **Two tests fail when run as root** (`coding::edit::tests`, both rely on a read-only file
   blocking a write). Confirmed in a cloud session. Other tests that use the same approach
   were not checked.
5. **Three call sites run a tool without its validation** (`hq-web/src/vault_api.rs`,
   `hq-web/src/coder_api.rs`, `hq-cli/src/commands/shortcuts.rs` call `execute` directly),
   by reading.
6. **Stale names in guard lists** (by reading): the file-touch tracker matches tool names the
   native file tools do not have, and the Telegram guest list denies five names that exist
   nowhere else. Fixed in A5.
7. **One harness has its trust dialog answered with Enter at spawn** (antigravity, by
   reading), which `CLAUDE.md`'s "never auto-answered" does not mention.
8. **Stale documents**: the README's crate count and line count; `capability-seams.md` lists
   `remote_mcp.rs` as removed; a CI comment says the bridge feature is off in the workspace
   run when it is in the default set; the runner runbook says releases are manual.
