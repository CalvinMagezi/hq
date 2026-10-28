# Pinned notes

A note is pinned when its frontmatter has `pinned: true`. Pinned notes show at
the top of the vault in the web app (up to 50), and up to 10 of them are loaded
into the context of every HQ chat turn, wrapped as vault data
(`hq_vault::system::get_system_context_keeping`, the context engine's
injection layer). A pin is therefore a standing note to HQ itself, which is why
who pinned it matters.

## Who pinned it

The rule lives in `hq_db::provenance::pin_suggested_by`: a pin on a version of a
note that an agent wrote (provenance `agent` or `generated`) and the owner has not
accepted is a **suggestion**.

- A suggestion shows in the web app's pinned list with the name of the agent
  that pinned it and a "Keep pin" button. It is not loaded into HQ's context.
- The owner keeping a pin (the button, or pinning the note in the app) records
  an owner write and accepts the version on disk (`review_note` with
  `accepted`), so the pin is trusted from then on.
- An agent editing a pinned note later writes a new version that nobody has
  accepted, so the pin is a suggestion again until the owner keeps it.
- `reviewed` does not count: agents may set it on each other's work.
- A version nobody recorded (edited outside HQ's tools, or pinned before this
  rule) stays trusted.

## Pinning from an agent

`vault_pin` (`{path, pin}`, `pin` defaults to true) sets or clears `pinned`
without touching the body. Every call is recorded as an agent's write, an
unattributed caller included, so a pin made through the tool is always a
suggestion. An agent may unpin its own suggestions but not the owner's pins.
Pinning a note in a system folder through a claude.ai connector waits for the
owner's approval, like any other write there.
