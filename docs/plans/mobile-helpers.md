# Mobile helpers: coding with HQ when the phone is the only computer

Status: brainstorm. Nothing here is built yet beyond what the "Today" column names.

## The problem

Helpers (Claude Code, Codex and the rest) run on a paired computer. With no computer paired, the
phone's start sheet shows one line to run on a computer and a pairing scanner (`HelperWords.PAIR_COMMAND`,
"Helpers" in `docs/HQ_MOBILE.md`). For someone who has no computer, that sheet is a dead end.

That person is common where HQ is built. A student or junior developer in Kampala is far more
likely to own an Android phone than a laptop, pays for data by the bundle, and often has a
mid-range phone with 3 to 4 GB of RAM. They can learn and they can review code, but they cannot
practise every day, and they cannot build the public record (commits, pull requests, a deployed
project) that gets them hired. A sponsor (an employer, a mentor, a programme) can often pay for
model credits long before anyone can pay for a laptop.

The value at scale: every phone that already runs HQ becomes a place to do real, reviewable work
on GitHub, for the cost of model credits. No new hardware.

## What Mobile Harness shows

[Mobile Harness](https://github.com/techjarves/Mobile-Harness) (MIT, an Android app) runs an
Ubuntu 20.04 ARM64 userspace under PRoot and installs Claude Code inside it, with DeepSeek's
harness and Google's Antigravity CLI on demand. Node, npm and git ship in the base; Python, Java,
C and PHP packs download when asked. It has a chat workspace, a terminal (a process bridge, not a
full pty), diffs, and a preview of a local web server, and it can build an APK and hand it to
Android's installer.

What it costs the person, from its own README:

- arm64 only, Android 9 or newer, 4 GB RAM minimum and 8 GB recommended;
- 2.5 GB free for the base, 8 GB recommended, and a large first download ("high-speed Wi-Fi
  recommended");
- PRoot is not a security boundary, heavy builds are slow under ptrace, and Android's battery
  policy throttles background work;
- Antigravity tasks launch with permissions skipped.

So it proves the demand (870 stars) and that a phone can host a real toolchain. It also proves
the point `docs/plans/edge-coding-worker.md` made when it turned PRoot down: the price is paid in
exactly the things the target person is short of, which are RAM, storage and data.

## Three ways to give a phone-only person helpers

Ordered by how well each fits a budget phone on mobile data.

| | Where code runs | What the person needs | Data cost | Today |
|---|---|---|---|---|
| A. HQ codes through GitHub | HQ's own agent on the phone edits a fetched copy; GitHub Actions is the test runner | a GitHub account, a fine-grained token, a model key, a repository with CI | small: a tarball per task, then API calls | built: `github_clone`, `github_write`, the `edge-coding-worker` skill, `edge-worker-dispatch`, run cards on Helpers. Not reachable from the phone (see below) |
| B. A computer in the cloud | a free or cheap Linux box (GitHub Codespaces on a student plan, a small VPS) paired like a laptop | a GitHub or VPS account; the same pairing line | low: only the terminal stream | works in principle with `install.sh --pair`; untested on a Codespace, which sleeps when idle |
| C. Linux on the phone | a companion app such as Mobile Harness, with its own Claude Code | a high-end phone, 8 GB free, Wi-Fi for setup, an Anthropic-compatible key | high at setup | outside HQ; HQ would only link to it |

### A. HQ codes through GitHub (the default for this person)

This is the edge coding worker, and it already does the hard part: fetch without git, edit with
HQ's file tools, publish an `hq/` branch and a pull request, read CI, fix until green, hand over
for review. It fits the problem better than anything else here, because the pull requests land
on the person's own GitHub under their own token: the work is the portfolio.

The gap is setup. The worker refuses everything until `github_write.repos` lists a repository,
and that key lives only in `config.yaml`, which `config_manage` refuses on purpose and which the
phone app never writes for it. A phone owner has no way to turn the worker on. The Tasks quick-add
only offers "Let HQ code" once it is already on (`TaskSheets.kt`, `takesTasks`).

What to build:

1. **A "Code from this phone" card on Helpers** when no computer is paired, beside the pairing
   line rather than instead of it. It says plainly what happens: HQ changes your repository on a
   branch, GitHub tests it, you review and merge.
2. **A phone setup flow for the worker**, owner-only like the rest of `github_write`:
   - paste a fine-grained token into Keys on this phone as `HQ_GITHUB_WRITE_TOKEN`, with the exact
     scopes the plan asks for (Contents and Pull requests read and write, Actions read, no
     Workflows) and a link to GitHub's token page with them preselected;
   - pick repositories from the ones that token can see, which writes `github_write.repos` through
     a new owner-only route (the app already writes `files/hqhome/config.yaml` for backends, so
     the write path exists; the refusal in `config_manage` stays, since that is the model's path,
     not the owner's);
   - a check that each picked repository has a workflow on pull requests, and if not, an offer to
     open a task for "add CI" that the person merges by hand (the worker may not touch `.github/`).
3. **Starting a helper from the Helpers sheet** creates an `edge-task` task in one step (repository,
   what to change, done when), so the person never has to learn the tag.
4. **A first live run** (step S3 of the edge worker plan) on a small Django or React Native
   repository with fast CI, measured on a mid-range phone on mobile data: megabytes per task,
   model cost per merged pull request, time to green.

What it cannot do, said in the card, not discovered later: no shell, so nothing runs on the phone;
no live preview unless the repository deploys previews (Cloudflare Pages, Vercel, Netlify all do
this per pull request for free tiers); and no workflow-file changes.

### B. A computer in the cloud

The pairing line already works on any Linux box. A "Use a computer in the cloud" option on the
same sheet would link a short guide: create a Codespace or a small VPS, paste the line, scan the
code. Open questions before it ships: a Codespace stops after its idle timeout and its free hours
run out, so the host must come back cleanly and the sheet must say "your cloud computer is
asleep", never "unreachable". A sponsor's own server is not the place for someone else's helpers:
they would share the sponsor's files and keys. One small box per person, or nothing.

### C. Linux on the phone

Not something HQ should embed. If a person already runs Mobile Harness or Termux, the useful thing
is the other direction: their Claude Code could reach HQ's MCP endpoint on the phone's loopback
and use the vault and tasks. Android apps share loopback, so this is plausible, but it needs a
key handed to another app and has not been tried. Treat it as a later experiment, and link out
rather than bundle.

## Sponsored credits

A sponsor paying for a person's model use should hand over a key with a hard spending cap, never a
key of their own. OpenRouter keys can carry a credit limit; the person pastes it in first run as
their OpenRouter key and nothing else changes. The sponsor sees spend on their side; HQ's own
`hq usage` shows it on the phone. The person's adoption data stays on the phone as always
(`docs/security/ADOPTION_DATA.md`); a sponsor gets no view into what they do with HQ.

## What success looks like

- A person with only a phone opens Helpers, follows one flow, and has an open pull request on
  their own repository within 15 minutes, on mobile data.
- Data per task stays small enough to fit a daily bundle (measure in S3; target under 20 MB).
- A merged pull request costs a known, small amount of model credit, shown before the task starts.

## Risks

- **A portfolio of work the person did not write.** Every pull request needs their review, and the
  card should say so. Reading and correcting the worker's diff is the practice; merging blind is not.
- **CI as the only test runner** makes a repository with slow or no CI a poor fit. Say so at setup.
- **Sideloading.** The app installs outside the Play Store; Google's developer verification reaches
  Uganda in 2027 (`docs/HQ_MOBILE.md`), and the app id must be registered before then.
- **Token blast radius.** A fine-grained token limited to the picked repositories, no Workflows
  scope, and the existing `github_write` guards. Nothing new here, but the setup flow is where a
  person would otherwise paste a classic token with every scope.
