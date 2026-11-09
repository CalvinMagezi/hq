# Mobile helpers: coding with HQ when the phone is the only computer

Status: plan. The coding engine is built (the edge coding worker); this plan makes it a standard
part of HQ, switched on and safe by default, and replaces the token setup with a one-tap GitHub
connection that puts no third party, the HQ project included, between a person and their code.

## The problem

Helpers (Claude Code, Codex and the rest) run on a paired computer. With no computer paired, the
phone's start sheet shows one line to run on a computer and a pairing scanner (`HelperWords.PAIR_COMMAND`,
"Helpers" in `docs/HQ_MOBILE.md`). For someone who has no computer, that sheet is a dead end.

That person is common where HQ is built. A student or junior developer in Kampala is far more
likely to own an Android phone than a laptop, pays for data by the bundle, and often has a
mid-range phone with 3 to 4 GB of RAM. They can learn and review code, but they cannot practise
every day or build the public record (commits, pull requests, a deployed project) that gets them
hired. A sponsor can often pay for model credits long before anyone can pay for a laptop.

The value at scale: every phone that runs HQ becomes a place to do real, reviewable work on
GitHub, for the cost of model credits. No new hardware, no toolchain on the phone.

## Decision

**HQ Coder ships with HQ and is on by default.** It is the edge coding worker
(`docs/plans/edge-coding-worker.md`) under a name a person understands, offered on Helpers in
every edition. "On" means armed, not acting: it does nothing until the owner connects GitHub, and
connecting is the consent. The owner never edits a config file or makes a token.

Three things change from today:

1. `github_write` stops being off until `repos` is listed. The allowlist becomes the repositories
   the owner picked on GitHub when they installed their App (below). `github_write.repos` stays as
   an optional narrowing for people who want it in config.
2. `github_write.dispatch_tasks` defaults to `true`, so a task tagged `edge-task` is worked
   without a second switch. Who may tag a task is already restricted (`refuse_queue_write`).
3. Connecting GitHub is one tap on GitHub's own pages, described next. The pasted
   `HQ_GITHUB_WRITE_TOKEN` keeps working for people who prefer it.

## Connecting GitHub: each person makes their own App, in one tap

### The constraint

No account other than the person's own may hold a way into their repositories. A shared App owned
by the project fails this: whoever controls the owning account can generate a private key at any
time and mint tokens for every repository every user picked, with no user asked. One stolen
password would reach everyone. That is not a risk the project takes on, whatever account it lives
under.

### What we use: the GitHub App manifest flow

GitHub lets a page
[register an App from a manifest](https://docs.github.com/en/apps/sharing-github-apps/registering-a-github-app-from-a-manifest):
HQ sends the browser to GitHub with the App's whole definition filled in, the person presses
"Create GitHub App", and GitHub sends the browser back to HQ with a one-time code. HQ trades the
code (within the hour, once) for the new App's id and private key. The App belongs to the person's
own account. The HQ project never sees it, holds nothing and runs nothing.

Then HQ opens the App's install page, the person picks repositories ("Only select repositories")
and presses Install.

From then on HQ signs in as the person's App with its private key and
[mints an installation token](https://docs.github.com/en/rest/apps/apps) when it needs one. That
token lasts an hour, and HQ asks for the least it needs each time: the one repository of the task
and only the permissions of that step. Nothing expires that a person has to renew, so background
work overnight or over a holiday needs no new sign-in.

### What the person sees on the phone

1. **Helpers, Code from this phone, Connect GitHub.** A Custom Tab opens GitHub's "Create GitHub
   App" page, already filled in with the name, the permissions and no webhook. They press Create.
2. **Pick repositories.** GitHub sends them back to HQ (on the phone, HQ's own server on
   `127.0.0.1`, which the browser on the same phone reaches), and HQ opens the install page at
   once. They pick repositories and press Install.
3. **Done.** HQ lists the installed repositories with one line each about CI and branch
   protection (below), and a "Start a task" button.

Two taps on GitHub, nothing to type or copy, no code to read off a screen. On a computer,
`hq github connect` does the same with the local browser. On a server with no browser, it prints
the link; the redirect to the server's loopback then fails in the person's browser, and they paste
the address it shows (it carries the code) back into the terminal.

### The App's definition

What the manifest asks for, and so all an App made this way can ever do:

- **Name** `HQ Coder (<login>)` with a short suffix if taken. **No webhook**, no callback, private
  to the person's account (not installable by others).
- **Repository permissions:** Contents read and write, Pull requests read and write, Actions read
  (CI runs and logs), Checks read, Commit statuses read, Metadata read (always present).
- **Not granted:** Workflows (so GitHub itself refuses a push that changes
  `.github/workflows/`, on top of `github_write`'s own refusal), Administration, Secrets,
  Environments, Deployments, Issues and every organisation and account permission.

### Why not the alternatives

| Option | Why not |
|---|---|
| One App owned by the project, with the device flow | The owning account can mint tokens for every user's repositories. Ruled out. |
| An OAuth App | No central key, but its `repo` scope reaches every private repository the person has, and its tokens do not expire. |
| A fine-grained token by hand (today) | Too many technical steps, a long-lived secret in the clipboard, and people get it wrong in the unsafe direction. |
| A fine-grained token from a [template URL](https://github.blog/changelog/2025-08-26-template-urls-for-fine-grained-pats-and-updated-permissions-ui/) | GitHub prefills the permissions, but the person still picks repositories and copies a token that lasts up to a year. Kept as the fallback for organisations that forbid Apps. |

### Who the work is credited to

Pull requests are opened by `HQ Coder (<login>)[bot]`, so anyone reading the repository can see
which changes HQ proposed and that the person merged them. Commits carry the person's name and
GitHub no-reply address as author, so merged work counts on their profile. Check during G2 how
GitHub credits a squash merge of a bot's pull request.

## Spike results (2026-11-09)

Run before G1 with a throwaway account, a one-file loopback server, Chrome 156 on a Mac, and a real
Chrome Custom Tab on a Samsung Galaxy A32 (Android 13, `adb reverse` carrying the phone's loopback
port to the server). The server recorded field names and permissions only, never a key or secret.

| Question | Answer |
|---|---|
| Does a Custom Tab reach GitHub's manifest page from a loopback page? | Yes. The auto-submitted form lands on "Create GitHub App" with no tap. |
| Does GitHub's redirect to `http://127.0.0.1:<port>/...` load? | Yes, on the Mac and in the Custom Tab, with `code` and `state` in the query. No fallback needed in Chrome. |
| Samsung Internet | **Not attempted**: not installed on the test phone, skipped by the owner's decision. The paste-the-address fallback stays for any browser that refuses the redirect. |
| Is a manifest without a webhook accepted? | Yes, with `hook_attributes` left out and `default_events: []`. The App's webhook shows inactive, and the conversion gives `webhook_secret: null`. |
| Conversion response | HTTP 201 with `id`, `client_id`, `slug`, `node_id`, `owner`, `name`, `description`, `external_url`, `html_url`, `created_at`, `updated_at`, `webhook_secret`, `pem`, `client_secret`, `permissions`, `events`. |
| Key format | PKCS#1 (`BEGIN RSA PRIVATE KEY`), so `ring`'s `RsaKeyPair::from_der` applies and no new crate is needed. |
| Permissions | Granted exactly as the manifest asked, and shown that way on the install page. |
| Return after Install | A manifest `setup_url` brings the Custom Tab back to loopback with `installation_id` and `setup_action=install`. |

What this changes in the design:

- **The install page defaults to "All repositories",** and a link cannot preselect the narrower
  choice. The sheet says "choose Only select repositories" before the Custom Tab opens, and when an
  installation reports `repository_selection: all` HQ says so on the repository list. Every
  repository stays subject to the flagging below whichever was chosen.
- **The install return carries no `state`,** so its `installation_id` proves nothing. HQ ignores it
  and finds the installation with the App JWT (`GET /app/installations`).
- **A Custom Tab cannot send the web token,** and URL tokens are refused. An authenticated
  owner-only POST starts a flow and returns a one-time address; the browser-facing start and return
  routes carry no credential, and the random `state` is their only capability. They do nothing
  without a pending, unexpired flow.
- **The person can rename the App on GitHub's page,** so HQ keeps the `slug` from the conversion
  and never derives it from the name.
- The redirect went to a server reached through `adb reverse`. G7 repeats it against HQ's own
  server on the phone.

Deviations from the plan above, decided while building G1:

- **The App's name is `HQ Coder <6 hex>`, not `HQ Coder (<login>)`.** HQ does not know the login
  until GitHub has made the App, and a long login would pass GitHub's name limit. The random suffix
  also removes the "name taken" case. The person can rename it on the page.
- **`client_secret` is not stored.** Nothing in this design uses it.
- **On a server, `hq github connect` needs an SSH port forward.** The start page is on the server's
  loopback too, so a printed link alone cannot reach it. With the forward the return works as on a
  laptop; pasting the return address remains for a browser that refuses the redirect.
- **One token is not tied to a repository:** listing what the person picked needs a token before
  HQ knows the names. It carries `metadata: read` only, so it can read names and nothing else.
  Every other token covers one repository.
- **A repository HQ could not fully read counts as flagged,** and one added on GitHub after the
  last scan is read before any work starts on it. A confirmation lapses if a new secret or preview
  host appears. With "All repositories" chosen, this is what keeps a new repository from being
  worked on unseen.
- **The first-task confirmation applies to the App only.** Someone on the pasted token listed
  `github_write.repos` by hand, which is already a deliberate choice. A connected App that stopped
  working takes no tasks at all, even with a config list.
- **`hq github` is not part of Lite,** which reaches no other service. On Windows the key is never
  written, because there are no owner-only file modes to protect it; WSL2 works.

## First phone run (G7, 2026-11-09)

Samsung Galaxy A32, Android 13, a debug build beside the real app with its own id and port, a
private test repository with a pytest workflow, Haiku 5.5 through OpenRouter driving and
`openai/gpt-6-luna` as the coding sub-agent. The phone screens (G4) did not exist yet, so the owner
routes were called over `adb forward` against a second server inside the test app, and the
browser steps ran in a real Custom Tab.

| Measure | Result |
|---|---|
| Queue to pull request, first task | 83 seconds of work, after waiting up to two minutes for the worker's tick |
| Second task, after forced Doze | 4 min 30 s; a new token was minted with no new sign-in |
| CI on the pull request | 13 to 14 seconds, green |
| Model cost per merged pull request | $0.0185 for the first task, $0.0105 for the second |
| Data for the first task | about 2.1 MB (0.48 MB in, 1.66 MB out), counting the idle hub in that window |
| Squash merge credit | GitHub lists the App's bot and the person together as authors of the commit |

Checks that passed on the phone: the key and record are mode 0600 in a 0700 folder; an owner route
answers 401 with no token; a repository using `secrets.FOO` is flagged and its task is not taken;
the first task waits for the owner; no `ghs_` token and no model key appear under `files/` or in
`logcat`.

Bugs only the phone found, all fixed with tests: the manifest's homepage was empty for this crate,
and GitHub refused it; the worker skill needed `github_write.repos`, so connecting alone never armed
it; `github_read` had no way into a private repository without `gh`; a deleted App answers 404
rather than 401.

Learned from the run: deleting an App leaves its past pull requests shown as `ghost` on GitHub, so
Disconnect's advice to delete the App has that cost; a person who wants the history can keep the App
and delete only its key.

Not run on a phone, because the model key had expired by then: `allowed_paths` on the coding child,
the per-task budget ending a turn, and a task under the worker preset. All three have unit tests.

After that run the sheet gained the job form, so nobody has to write a tagged task by hand; pressing
Start, with the budget shown, replaces the separate "Allow the first task" button.

Not measured: mobile data on a metered network (the phone was on Wi-Fi), Samsung Internet, and a
full `-Phq.caddy` build (a placeholder was packaged, since Caddy stays off without a Caddyfile).

## Secure by default

| Control | Default | Why |
|---|---|---|
| Who holds a way in | Only the person's own App. The HQ project holds no App, key or token | One compromised account cannot reach other people's code. |
| Who can connect | The owner only: the phone app, `hq github connect`, owner Settings in the web app. Not a tool, not an MCP route. A `state` value ties the returning code to the flow HQ started | A model or a link must not be able to attach an App HQ did not create. |
| Which repositories | Only those the App is installed on, narrowed by `github_write.repos` if set | The person chose them on GitHub; HQ cannot widen them. |
| What a token can do | One repository, the permissions of the step, one hour | A token seen in a log or a crash is nearly worthless. |
| What it may push | `hq/` branches and pull requests only; no force, merge or deletion; no `.github/` changes; secret scan of every changed file (all existing `github_write` guards) | Nothing reaches the default branch without the person merging it. |
| Where the key lives | `<hq dir>/github/app.pem` and `app.json`, mode 0600, beside the base records that `governance::secrets` denies to every file tool. Outside the vault, so `hq backup` does not carry it, and Android cloud backup and device transfer take nothing from there (`data_extraction_rules.xml`) | The private key is the one long-lived credential, so it stays on the device that made it. |
| Where tokens go | `api.github.com` and the tarball host only. Never in the environment, so bash and child processes never see one | Same rule `github_write` has today. |
| Repositories whose CI or previews hold secrets | Flagged at connect and left unpicked until the owner confirms that repository | A pull request from an `hq/` branch runs Actions with the repository's secrets, and Vercel, Netlify or Cloudflare Pages build it with preview variables, before anyone reviews it. The App cannot list secrets, but it can read workflow and preview config files (Contents read). |
| Spend | The first task after connecting asks for confirmation; every task has a model budget, shown before it starts, and is parked when it runs out | `dispatch_tasks` is on by default, and credits may be a sponsor's. |
| Who can queue work | Unchanged: the owner, the owner's MCP clients and HQ's chat. Handoff, task keys and connector grants cannot tag `edge-task` (`refuse_queue_write`) | With dispatch on, this is the gate that matters. |
| Untrusted input | The worker's turn starts under the untrusted policy (no secret reads, no network from bash) and has no shell (existing) | It reads repository text, issues and CI logs. |
| Disconnect | Settings, GitHub, Disconnect: deletes the key and record on the device and opens the App's settings page, where deleting the App (or its key) cuts access at once | The person, not HQ, is the authority on their App. |

Organisation repositories need an App created under that organisation, by one of its owners.
That is GitHub's rule, and the sheet says so instead of failing quietly.

## Where it shows up

- **Helpers, with no computer paired:** "Code from this phone" first, the pairing line second.
  With a computer paired, HQ Coder is still listed as a place to run, as "This phone (HQ)". Its
  runs already appear as cards (`HelperWords.edgeCard`).
- **Tasks quick-add:** "Let HQ code" appears once GitHub is connected (today it waits for config).
- **The card says what it is.** HQ changes your repository on a branch, GitHub tests it, you
  review and merge. Nothing runs on the phone, a live preview needs the repository to deploy
  previews, and it cannot change workflow files.

## Steps

| Step | What | Where |
|---|---|---|
| G1 | The manifest flow (manifest page, `state`, return route, code exchange), key store, App JWT and scoped installation tokens; `hq github connect | status | disconnect` | `crates/hq-tools/src/github/app.rs` (new), `crates/hq-cli/src/commands/github.rs` (new) |
| G2 | `github_write` and `github_clone` take an installation token first and `HQ_GITHUB_WRITE_TOKEN` second; the allowlist is the installation, narrowed by `github_write.repos`; commit authorship | `crates/hq-tools/src/github/write.rs`, `crates/hq-core/src/config/github_write.rs` |
| G3 | Owner-only REST routes for the flow and the repository list with its CI, preview and branch protection checks | `crates/hq-web/src/github_connect_api.rs` (new) |
| G4 | Phone: Connect GitHub on Helpers and in Settings, the repository list, "Code from this phone" | `apps/android/.../ui/helpers/`, `ui/settings/` |
| G5 | Web: the same in Settings and Helpers | `apps/hq-web/src/routes/helpers.tsx`, settings |
| G6 | Flip the defaults (`dispatch_tasks: true`, worker armed without `repos`), the first-task confirmation, the per-task budget, and the worker preset of edge-worker step S3 | config, `edge-worker-dispatch` |
| G7 | First live run on a small Django or React Native repository with fast CI, on a mid-range phone on mobile data: megabytes per task, model cost per merged pull request, time to green | measurements in `docs/plans/` |

Nothing here needs the project to register anything with GitHub, so every step can be built and
tested against a test account.

## Other ways to give a phone-only person helpers

HQ Coder is the default. These stay as options on the same sheet.

**A computer in the cloud.** The pairing line works on any Linux box: a GitHub Codespace on a
student plan, or a small VPS. A Codespace stops when idle and its free hours run out, so the host
must come back cleanly and the sheet must say "your cloud computer is asleep", never
"unreachable". A sponsor's own server is not the place for someone else's helpers: they would
share the sponsor's files and keys. One small box per person, or nothing.

**Linux on the phone.** [Mobile Harness](https://github.com/techjarves/Mobile-Harness) (MIT) runs
Ubuntu under PRoot with Claude Code inside. It proves the demand (870 stars) but asks for what the
target person is short of: 4 GB RAM minimum and 8 GB recommended, 2.5 to 8 GB free, a large first
download, slow builds under ptrace, and no security boundary. HQ should link to it, not embed it.
A later experiment: its Claude Code could reach HQ's MCP endpoint on the phone's loopback and use
the vault and tasks.

## Sponsored credits

A sponsor paying for someone's model use hands over a key with a hard spending cap, never a key of
their own. OpenRouter keys can carry a credit limit; the person pastes it at first run and nothing
else changes. HQ Coder's per-task budget sits under that cap. The person's adoption data stays on
the phone as always (`docs/security/ADOPTION_DATA.md`); a sponsor gets no view into what they do.

## What success looks like

- A person with only a phone opens Helpers and has an open pull request on their own repository
  within 15 minutes, on mobile data, without copying a token.
- Data per task fits a daily bundle (measure in G7; target under 20 MB).
- The model cost of a merged pull request is known and shown before the task starts.

## Risks

- **The key on the phone can write to more than `hq/` branches.** The `hq/`-only rule is HQ's, not
  GitHub's. Someone who takes the private key from the device can push to any branch of the
  installed repositories until the person deletes the key or the App, unless branch protection or
  a ruleset stops it. On GitHub Free those exist for public repositories only, so a free private
  repository has no server-side backstop. This is the same reach today's fine-grained token has.
  The repository list says so per repository and links to branch protection where it is
  available. Android's app sandbox and the file's mode are what guard the key; a rooted or
  compromised phone defeats both.
- **A planted instruction becomes a bad pull request.** The worker reads issues, CI logs and
  repository text, any of which can carry instructions. Nothing merges without the person, but the
  people this is for are the likeliest to merge without reading. The card and the pull request
  body ask for review, and CI and preview secrets are flagged (above).
- **A portfolio of work the person did not write.** Every pull request needs their review.
  Reading and correcting the diff is the practice; merging blind is not.
- **CI as the only test runner.** A repository with slow or no CI is a poor fit. The repository
  list says so for each one, and offers a task to add CI that the person merges by hand.
- **One App per person to look after.** If they delete it on GitHub, HQ sees the next token request
  fail and says "reconnect GitHub", never retries in a loop.
- **Sideloading.** The app installs outside the Play Store; Google's developer verification reaches
  Uganda in 2027 (`docs/HQ_MOBILE.md`), and the app id must be registered before then.
