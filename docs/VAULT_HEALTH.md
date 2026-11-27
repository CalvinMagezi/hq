---
audience: owner
order: 45
summary: "How HQ checks your notes for broken links and repeats, what it shows, and what it never does."
updated: 2026-11-26
---

# The notes check

Notes pile up. Links break when a note is renamed or deleted, the same text gets saved twice,
and a note's properties get damaged. None of it hurts at first, then searches and answers get
worse because HQ reads the same notes you do. The notes check finds these things and tells you
where they are.

## What it looks for

- **Links that lead to a note that is not there.** `[[Quarterly Reveiw]]` when the note is
  called `Quarterly Review`. HQ suggests the near match when exactly one is close.
- **Links that could mean more than one note.** `[[Plan]]` when two notes are called `Plan`.
  HQ never guesses which one you meant.
- **Links to a heading that is gone.** `[[Plan#Goals]]` when `Plan` has no `Goals` heading.
- **Notes with the same text.** Spacing and capitals do not hide them. Very short notes are not
  compared.
- **Properties HQ cannot read.** The block at the top of a note that is never closed, that is
  not valid, or that appears twice.
- **Notes nothing links to**, and **empty notes**. These are listed but are not counted as
  something to fix.

Links inside code blocks and code spans are code, not links. Links to the web, to a heading in
the same note, and to pictures and files in your vault are not counted as broken.

## Where you see it

- **Notes home** shows a short card, only when something needs a look. It is gone when your
  notes are in order.
- **Review** (from the card) lists each item with its note and line. **Open** takes you to the
  note. **Ignore** leaves one item out from then on. **Check now** looks again straight away.
- **`hq vault health`** prints the same list in a terminal. Add `--json` for a script.
- **`hq doctor`** has one line about it.
- **In a chat**, ask "Check my notes for broken links". Chats also warn when a note they just
  wrote holds a link that leads nowhere, so it can be fixed on the spot.

HQ checks every few hours. It also keeps the list in `_system/VAULT-HEALTH.md`, which it
rewrites each time.

## What it never does

- It **changes no note**. This version only reports. You fix a link by editing the note.
- It uses **no AI model and no network**. It reads your notes on your own device.
- What it found stays in HQ's own database on this device. It is not sent anywhere.

Turn it off with `daemon.vault_health: false` in your config.

## Moving and renaming keep links working

When you move or rename a note in HQ, every link that reached it is rewritten to follow it:
plain `[[Name]]` links, links with a heading, a folder, a `.md` ending, embeds, and markdown
links. A note that moves to another folder keeps its own relative links working. Moving a whole
folder fixes links into it too.

If a link could mean either of two notes with the same name, HQ leaves it alone and lists it in
the result of the move, instead of pointing it at the wrong one.

## Limits

- It reads notes under `Notebooks/`. Notes larger than 2 MB are not read; they are counted
  under "not checked".
- A note renamed outside HQ (in another editor or by a sync tool) leaves its links broken. They
  show up here as links to a note that is not there.
- Checking twenty thousand notes with a quarter of a million links takes about half a second.
