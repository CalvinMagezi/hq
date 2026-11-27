---
audience: owner
order: 45
summary: "How HQ checks your notes for broken links and repeats, fixes what it is sure of, and how to undo it."
updated: 2026-11-27
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
- **Leave the rest alone** is for a large, old collection with thousands of broken links you do
  not mean to tidy. One tap sets aside everything that is here now, except what HQ can fix,
  so the card counts only what is new from then on and the repairs stay on offer. The card
  shows it once ten or more problems have no repair, and Review shows it whenever any do. The
  card and Review say how many are set aside, and **Count them again** brings them all back
  (that also brings back anything you ignored one by one). It changes no note. HQ never does
  this for you: a number that quietly went away would hide the very links you might want fixed.
- **`hq vault health`** prints the same list in a terminal. Add `--json` for a script.
  `hq vault health --set-aside` and `--count-again` do the two above.
- **`hq doctor`** has one line about it.
- **In a chat**, ask "Check my notes for broken links". Chats also warn when a note they just
  wrote holds a link that leads nowhere, so it can be fixed on the spot.

HQ checks every few hours. It also keeps the list in `_system/VAULT-HEALTH.md`, which it
rewrites each time.

## Fixing

HQ can fix some of what it finds. Every fix is one of two kinds, and nothing else is ever fixed.

- **Certain.** A link whose folder is out of date but whose note name has exactly one note. A
  link to a note that was renamed (HQ remembers a rename it saw for 30 days). A second block of
  properties at the top of a note, merged into the first.
- **Likely.** A misspelt link when exactly one note has a close name. A name that fits several
  notes, settled by the folder the link is in. A note that repeats another word for word:
  it goes to the Trash and links to it point at the one that stays.

Everything else is only listed: a link to a note nothing resembles, a link to a heading that is
gone, two notes that are close but not the same.

`hq vault health` shows what could be fixed. `hq vault health --apply` does the certain ones
(`--tier all` adds the likely ones). Each run saves the original of every note first, in
`_trash/<day>/link-repair/<run>/`, and writes a list of what it did. Undo it with
`hq vault repair --undo <run>`; `hq vault repairs` lists the runs. Undo puts a note back only if
it still reads exactly as the fix left it, so anything you changed afterwards is kept as you
have it. The saved copies are emptied with the Trash after 30 days.

HQ does not fix a note it should not touch, and says which and why: a sheet, deck or dashboard,
a pinned note, a note an assistant wrote that you have not reviewed, notes written by HQ's own
upkeep, anything outside your notes folder, and any note changed in the last ten minutes. A note
that changed after the check is left as it is.

**Automatic fixing is off.** Set `daemon.vault_auto_fix: true` and HQ does the certain fixes
after each check, up to 50 notes a run. It stops by itself if you keep undoing them: once more
than one in twenty of its last runs has been taken back.

## The Inbox and your phone

When something needs a look, HQ puts one note in your Inbox, once a week. It never goes through
a push service, because it names your notes. When HQ is sure about some fixes, one question
goes with it: approve, and HQ fixes them. After a first yes it asks once more whether to do the
certain fixes from now on without asking. Say yes and `daemon.vault_auto_fix` is not needed. The
answer is kept on your device, and you can change it by editing the preference or the config.

The phone shows the same card on Notes, with **Fix** and **Undo**, and a page that lists each
item with its fix. A fix can be undone from the card for a day, and from the list of recent
fixes after that for as long as the saved copies last.

## What it never does

- It **changes a note only when you ask**, or after you have turned on automatic fixing for
  the certain fixes. Everything it changes can be undone.
- It uses **no AI model and no network**. It reads your notes on your own device. Finding notes
  that say almost the same thing uses the small model that runs on your device, and only that
  one: if your notes are embedded by a model on the internet, that check is skipped.
- What it found stays in HQ's own database on this device. It is not sent anywhere.

Turn the check off with `daemon.vault_health: false` in your config.

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
- A note renamed outside HQ (in another editor or by a sync tool) is noticed at the next check,
  when its exact text appears under a new name, and the links to the old name can then be fixed.
  A rename that also changes the text is not recognised.
- Notes that say almost the same thing are compared only when there are at most 6,000 of them.
- Speed, measured on a laptop-class machine with 20,000 notes and 240,000 links: reading and
  resolving every link takes about half a second. Looking for a near-match name for each broken
  link is the slow part, and it depends on how many links are broken, not on how many notes
  there are. With 22,000 broken links, a vault far messier than a real one, the whole check takes
  about ten seconds; with a few hundred it is well under one. The check runs in the background
  and a screen never waits on a scan it did not ask for.
