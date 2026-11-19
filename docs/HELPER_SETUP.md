---
audience: owner
order: 40
summary: "Install the free coding helpers on a connected computer and see which are ready."
updated: 2026-11-19
---

# Helper setup

A person who has just installed HQ usually has no coding helper (Cursor, Codex, Antigravity and
the others) and no idea which ones they could use for free. Helper setup closes that gap: HQ looks
at the computer it has a host on, installs the helpers the person wants with each vendor's own
installer, opens each sign-in page on that computer, and from then on knows which helpers are
installed and signed in.

It works whether HQ runs on the computer itself or on a phone with a laptop paired to it. Everything
runs through the host (`HostBackend::run_command`), the same path a helper session takes.

## What the person sees

1. HQ finds one connected computer with none of the catalog's helpers and leaves one inbox item,
   "Set up coding helpers on <computer>" (daemon task `helper-setup-offer`, every half hour, off
   with `daemon.helper_setup_offer: false`). A dismissed offer does not come back.
2. They approve it once. HQ installs what is missing and starts each sign-in. Pages open on that
   computer; they sign in on each vendor's own page. A computer with no browser shows the sign-in
   link in the app instead.
3. An inbox note says what is ready. The Helpers page shows each helper as Ready, Installed but not
   signed in, or Not installed, with Set up, Sign in and Check again buttons.

The same flow is the `helper_setup` tool (`status`, `check`, `setup`, `install`, `login`), so a chat
can offer it when a job needs a helper and none is ready. `harness_session_spawn` names it when a
helper's program is missing.

## How it stays safe without twenty prompts

- **One approval per batch.** `helper_setup` with a changing action (`setup`, `install`, `login`)
  asks the owner once, in every permission mode but bypass, and never runs in a session nobody can
  ask. The card is written from the catalog (what will be downloaded and from where), not from the
  model's words. The approval is bound to the exact arguments: which computer, which helpers.
  Nothing else is asked per helper or per step. The web app's buttons need no inbox approval: the
  owner pressing the button is the approval.
- **The commands are fixed.** `crates/hq-tools/src/helper_setup/catalog.rs` holds every install and
  sign-in command. A request, a chat or a model names a computer, an action and helper ids; it never
  supplies a command, and a test checks the recipes are fixed strings.
- **It does not choose a computer for the owner.** With more than one connected computer the call
  names which; with none it says so.
- **One setup at a time per computer.** A second tap says one is already running.
- **Reports are readings.** `helper_status` keeps whether a helper was installed and signed in when
  last checked, with the time. A sign-in HQ cannot ask about (Antigravity has no status command) is
  recorded as unknown until its login run succeeds.
- **Output is cleaned.** What a program prints is stored with secret-shaped text removed
  (`hq_core::redact::redact_secrets`), cut to the last lines, and only `https://` links are lifted
  out as sign-in links.

## What is installed, and from where

| Helper | Install | Sign in | Asks "signed in?" |
| --- | --- | --- | --- |
| Cursor | `curl https://cursor.com/install -fsS \| bash` (Cursor's documented installer) | `cursor-agent login` | `cursor-agent status` |
| Codex | `brew install --cask codex`, else `npm install -g --prefix "$HOME/.local" @openai/codex` | `codex login` | `codex login status` |
| Antigravity | `curl -fsSL https://antigravity.google/cli/install.sh \| bash` | first run: `agy -p "say hi"` | none |

These run the vendors' code on the owner's computer, which is why the card says so. Re-check each
line against the vendor's current documentation before a release: the Antigravity macOS and Linux
installer address and the Codex status command were read from third-party guides, not the vendors'
own pages, and the "signed in?" commands' exit codes have not been run against live accounts. The
check treats an exit status other than zero, or output saying "not logged in" and the like, as
signed out.

**Free tiers.** HQ never promises one. The card says each helper can be tried with a free account
and that the vendor sets and changes the limits. Cursor and Antigravity have been described by
third parties as starting free; whether a free ChatGPT account includes Codex is up to OpenAI.

## Limits to know

- Unix hosts only (macOS, Linux, WSL2). Windows without WSL2 has no host.
- A helper installed into `~/.local/bin` may not be on the PATH of a host the system started.
  HQ looks for it the way a login shell would, and if the host itself cannot see it yet the status
  says "Restart HQ there".
- A host that only starts sandboxed agents runs the install in the same sandbox as an agent. The
  `open` profile (the default) allows it; `strict` blocks the download, and the step fails with what
  it printed.
- A sign-in not finished in five minutes is stopped; press Sign in again.
- Nothing here reads a helper's credentials. The sign-in happens on the vendor's page.

## Where it lives

`crates/hq-tools/src/helper_setup/` (`catalog.rs`, `runner.rs` runs one program in a host pane and
follows it, `service.rs`, `mod.rs` the tool, the background start and the offer),
`HostBackend::run_command` in `crates/hq-tools/src/agent_host/`, `crates/hq-db/src/helper_status.rs`
(`sql/138_helper_setup.sql`), `crates/hq-web/src/helper_setup_api.rs`,
`apps/hq-web/src/components/sessions/HelperSetup.tsx`, daemon task
`crates/hq-cli/src/commands/start/daemon/tasks_periodic/helper_setup_offer.rs`. Tests:
`crates/hq-tools/tests/helper_setup.rs` runs real processes in a real in-process host.
