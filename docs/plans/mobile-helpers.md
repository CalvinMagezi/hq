# Mobile helpers: coding with HQ when the phone is the only computer

Status: plan. The coding engine is built (the edge coding worker); this plan makes it a standard
part of HQ, switched on and safe by default, and replaces the token setup with a GitHub sign-in.

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
   the owner picked on GitHub when they installed HQ's GitHub App. `github_write.repos` stays as an
   optional narrowing for people who want it in config.
2. `github_write.dispatch_tasks` defaults to `true`, so a task tagged `edge-task` is worked
   without a second switch. Who may tag a task is already restricted (`refuse_queue_write`).
3. Connecting GitHub is a sign-in through GitHub's own pages, described next. The pasted
   `HQ_GITHUB_WRITE_TOKEN` keeps working for people who prefer it.

## Connecting GitHub without making a token

### What we use: a GitHub App and the device flow

GitHub Apps can sign a person in with the
[device flow](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/generating-a-user-access-token-for-a-github-app):
HQ asks GitHub for a short code, the person opens `github.com/login/device`, enters it and presses
Authorize. HQ polls and gets a user access token. The device flow needs only the App's client ID,
which is public, and a token from it is
[refreshed without a client secret](https://docs.github.com/en/apps/creating-github-apps/authenticating-with-a-github-app/refreshing-user-access-tokens).
So nothing secret ships in the binary, no HQ server sits in the middle, and the phone talks to
GitHub directly. `gh auth login` works the same way.

A user token from a GitHub App can only do what all three of these allow at once:

- the permissions the App declares (below);
- what the person can do themselves;
- the repositories the App is installed on, which the person chooses on GitHub's install page
  ("Only select repositories").

That last point is what makes it "request and accept": the repository list is chosen and shown on
GitHub, and removing a repository there takes effect at once, whatever HQ thinks.

Access tokens last eight hours and refresh tokens six months; a refresh replaces both. A stolen
token is short-lived, and a person who stops using HQ is disconnected on their own after six
months.

### What the person sees on the phone

1. **Helpers, Code from this phone, Connect GitHub.** HQ shows an eight-character code with Copy
   and an "Open GitHub" button (a Custom Tab on `github.com/login/device`, already signed in for
   most people). They paste the code and press Authorize.
2. **Pick repositories.** Back in the app, HQ opens the App's install page. The person picks
   repositories and presses Install. On return HQ lists the installations the token can see
   (`GET /user/installations`, then each installation's repositories) and shows the list.
3. **Done.** The sheet shows each repository with one line about its CI (below) and a "Start a
   task" button.

Two confirmations on GitHub, nothing typed but a pasted code, no token in the clipboard. The same
flow serves `hq github connect` on a computer and Settings in the web app, so there is one way to
connect in every edition.

### Why not the alternatives

| Option | Why not the default |
|---|---|
| Fine-grained token by hand (today) | Too many technical steps, and people get it wrong in the unsafe direction (a classic token with every scope). |
| A fine-grained token from a [template URL](https://github.blog/changelog/2025-08-26-template-urls-for-fine-grained-pats-and-updated-permissions-ui/) | GitHub can prefill the name, expiry and permissions, but the person still picks repositories and copies a long-lived secret. Kept as the fallback for organisations that forbid Apps. |
| An OAuth App | Its `repo` scope reaches every private repository the person has. No per-repository limit. |
| The OAuth web flow with a redirect | Needs a client secret, so an HQ server holding it. A server we don't need is one more thing to secure. |
| Each person makes their own App (manifest flow) | Two extra steps and a private key on the phone. Worth it only for a self-hosted fork; see "Self-hosting". |

### The App HQ registers

The project registers one public GitHub App (working name "HQ Coder") once, the way it registers
the Android app id and the release signing key. Its settings:

- **Device flow** enabled. **User token expiration** on.
- **No webhook**, no callback URL, no setup URL. **No private key generated**: HQ never acts as the
  App itself (installation tokens need the key), only as the person.
- **Repository permissions:** Contents read and write, Pull requests read and write, Actions read
  (CI runs and logs), Checks read, Commit statuses read, Metadata read (always present).
- **Not granted:** Workflows (so GitHub itself refuses a push that changes
  `.github/workflows/`, on top of `github_write`'s own refusal), Administration, Secrets,
  Environments, Deployments, Issues and every organisation and account permission.

Its client ID is the default of a new `github.app_client_id` key. A self-hosted fork points that at
its own App.

## Secure by default

What stops this being dangerous while it is on for everyone:

| Control | Default | Why |
|---|---|---|
| Who can connect | The owner only: the phone app, `hq github connect`, owner Settings in the web app. Not a tool, not an MCP route. | A model that could start a device flow could ask the owner to "just enter this code" and take their GitHub. |
| Which repositories | Only those the App is installed on, narrowed by `github_write.repos` if set | The person chose them on GitHub; HQ cannot widen them. |
| What it may push | `hq/` branches and pull requests only; no force, merge or deletion; no `.github/` changes; secret scan of every changed file (all existing `github_write` guards) | Nothing reaches the default branch without the person merging it. |
| Where the token lives | `<hq dir>/github/auth.json`, mode 0600, beside the existing base records that `governance::secrets` denies to every file tool. On Android that is the app's private storage. | The refresh token rotates every eight hours, so it must be a file `hq` can rewrite. A sealed environment variable cannot be rewritten that way. |
| Where the token goes | `api.github.com` and the tarball host only. Never in the environment, so bash and child processes never see it. | Same rule `github_write` has today. |
| Repositories whose CI holds secrets | Flagged at connect and left unpicked until the owner confirms that repository | A pull request from an `hq/` branch in the same repository runs Actions with its secrets before anyone reviews it. The App cannot list secrets, but it can read workflow files (Contents read), so any workflow that names `secrets.` other than `GITHUB_TOKEN` is flagged. |
| Spend | A per-task model budget, shown before a task starts. The task is parked when it runs out | On sponsored credits, one runaway task should not empty the key. |
| Who can queue work | Unchanged: the owner, the owner's MCP clients and HQ's chat. Handoff, task keys and connector grants cannot tag `edge-task` (`refuse_queue_write`) | `dispatch_tasks` being on makes this the gate that matters. |
| Untrusted input | The worker's turn starts under the untrusted policy (no secret reads, no network from bash) and has no shell (existing) | It reads repository text, issues and CI logs. |
| Disconnect | Settings, GitHub, Disconnect: deletes the tokens on the device and opens the App's installation page to remove repositories or uninstall | Revoking a token through GitHub's API needs the client secret we do not hold. Uninstalling the App cuts access at once, and an unused refresh token dies in six months anyway. |

Organisation repositories need an organisation owner to approve the install. That is GitHub's
rule, and the sheet says so instead of failing quietly.

## Where it shows up

- **Helpers, with no computer paired:** "Code from this phone" first, the pairing line second.
  With a computer paired, HQ Coder is still listed as a place to run, as "This phone (HQ)". Its
  runs already appear as cards (`HelperWords.edgeCard`).
- **Tasks quick-add:** "Let HQ code" appears once GitHub is connected (today it waits for config).
- **The card says what it is.** HQ changes your repository on a branch, GitHub tests it, you
  review and merge. Nothing runs on the phone, a live preview needs the repository to deploy
  previews (Cloudflare Pages, Vercel and Netlify do this per pull request on free tiers), and it
  cannot change workflow files.

## Steps

| Step | What | Where |
|---|---|---|
| G1 | Device flow, token store and refresh, `hq github connect | status | disconnect` | `crates/hq-tools/src/github/auth.rs` (new), `crates/hq-cli/src/commands/github.rs` (new) |
| G2 | `github_write` and `github_clone` take the connected token first and `HQ_GITHUB_WRITE_TOKEN` second; the allowlist is the installations, narrowed by `github_write.repos` | `crates/hq-tools/src/github/write.rs`, `crates/hq-core/src/config/github_write.rs` |
| G3 | Owner-only REST routes for the flow (start, poll, installations, disconnect) and the CI-secrets check | `crates/hq-web/src/github_connect_api.rs` (new) |
| G4 | Phone: Connect GitHub on Helpers and in Settings, the repository list, "Code from this phone" | `apps/android/.../ui/helpers/`, `ui/settings/` |
| G5 | Web: the same in Settings and Helpers | `apps/hq-web/src/routes/helpers.tsx`, settings |
| G6 | Flip the defaults (`dispatch_tasks: true`, worker armed without `repos`), the per-task budget, and the worker preset of edge-worker step S3 | config, `edge-worker-dispatch` |
| G7 | First live run on a small Django or React Native repository with fast CI, on a mid-range phone on mobile data: megabytes per task, model cost per merged pull request, time to green | measurements in `docs/plans/` |

G1 and G2 need the App registered (a one-time task for the project owner, done in GitHub's
settings). G1 to G3 can be built and tested against a test App before that.

## Self-hosting

Anyone running a fork registers their own App with the settings above and sets
`github.app_client_id`. A later step can do that from a manifest link (`hq github app create`), so
it takes one confirmation too.

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

- **A portfolio of work the person did not write.** Every pull request needs their review, and the
  card says so. Reading and correcting the diff is the practice; merging blind is not.
- **CI as the only test runner.** A repository with slow or no CI is a poor fit. The repository
  list says so for each one, and offers a task to add CI that the person merges by hand.
- **The project owns an App many people trust.** Its settings page is the place to widen every
  user's access at once. Changing its permissions makes GitHub ask each installer to approve the
  change, which is the backstop. Two-factor authentication and a short list of App managers on the
  owning account are required, and changes go in `docs/security/RELEASE_CHECKLIST.md`.
- **Device-code phishing.** Anyone can send a person a GitHub device code and ask them to approve
  it. The approval page names "HQ Coder"; the phone only shows a code it asked for itself, and
  copy in the app says never to enter a code someone sent you.
- **Sideloading.** The app installs outside the Play Store; Google's developer verification reaches
  Uganda in 2027 (`docs/HQ_MOBILE.md`), and the app id must be registered before then.
