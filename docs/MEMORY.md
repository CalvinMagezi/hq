---
audience: owner
order: 75
summary: "What HQ remembers about you, who it remembers, and how to look, correct or delete it."
updated: 2026-11-27
---

# What HQ remembers

HQ saves short notes about what you tell it in chat, so it can help you later without being asked
twice. This page says what is saved, what is not, how a note comes back, and how you correct or
delete one. Nothing here is hidden: every note is listed, and you can delete any of them.

## What gets saved

After a chat, HQ may save one short note about what **you** said: a fact you gave, a decision, a
preference, a correction, or what you asked for. It does not save what HQ answered, unless you
agreed with it.

- **Say "remember that ..."** at the start of a line and HQ keeps that sentence for good. It is
  pinned, so it never ages out, even when it is about health or family.
- **Saying it twice** does not make two notes. HQ counts the repeat on the note it already has.
- **Secrets** that look like keys or passwords are removed before anything is saved.
- **Some subjects are not saved on their own**: health, religion and belief, politics, sexuality,
  ethnicity, immigration, and anything that looks like a password or an account number. Say
  "remember that ..." when you do want one kept.
- **Text that tries to give HQ orders** ("ignore your instructions") is saved but held back until
  you have looked at it.
- **Other people are not remembered.** A family member or guest who chats with HQ is not saved and
  sees none of your notes. A note that came from a shared channel before this was fixed is held
  back until you keep it.

## How a note comes back

HQ looks at your message and brings up the few notes that fit it, at most five and a short block,
never a dump. A greeting or a question about nothing in particular brings up nothing. It does this
on your device with no extra model call and no data leaving it.

`daemon.memory_recall` in `~/.hq/config.yaml` sets how:

| Value | What happens |
|---|---|
| `shadow` (default) | Chats work as before. HQ writes down, as counts only, what it would have brought up. |
| `query` | Each message brings up only the notes that fit it. |
| `recent` | The old way: the most important notes, whatever you said. |
| `off` | Notes from chats are not brought up at all. |

Read the counts with `hq memory recall-report`. It says whether `query` is ready to switch on.
Switching is your decision: it is not done for you.

When the on-device model is already on the machine, HQ can also find a note by what it means, not
only by its words. It never downloads the model for this.

## See it, correct it, delete it

**Settings, What HQ remembers** (web, desktop and the Android app) lists every note with its date, whether it is
pinned, held back, out of date, or over a year old, and how often it was used. Search it, filter
it, and for each note:

- **Edit** the words. The note becomes yours.
- **Pin** it so it never ages out, or unpin it.
- **Out of date**: keep it but stop bringing it up.
- **Keep** a held-back note you vouch for.
- **Forget** it. This deletes the note and every copy HQ made of it: the insight notes drawn from
  it, its lines in `MEMORY.md`, its place on concept pages, and its line in the archive. HQ keeps
  only a fingerprint of the sentence so it is not learned again from the next chat.

The Android page lists the newest 50 notes and offers Pin, Keep, Out of date and Forget; editing and searching are on the web page and in the terminal. The same is in the terminal: `hq memory list|search|view|edit|pin|unpin|keep|outdated|forget|export`.

### Pause saving

The switch **Save new notes** at the top of that page stops HQ saving anything new from your
chats, for example while someone else is using your phone. Notes already saved are still used and
still listed, and you can still delete any of them. It applies at once, even in a chat that is
already open.

### What Forget cannot reach

- **The chat itself.** What you said in the chat is still in that chat's history.
- **Backups** made before you forgot it.

HQ says so each time rather than letting you assume.

## How long notes last

Nothing is deleted on its own. A note that has not been brought up for a year, and is not pinned,
is **put away**: HQ stops bringing it up, but it is still listed and searchable
(`hq memory list --archived`), and using it again (`hq memory keep <number>`) brings it back. HQ
keeps at most 20,000 notes in use. Pinned notes are never put away.

## Languages

Matching works on English text today. Notes in Luganda, Swahili or a mix are saved but are matched
poorly. That gap is recorded in `docs/techdebt/memory-recall-english-only.md`.

## Where the data lives

Notes are rows in the vault's database (`_data/vault.db`), not files. They never leave your
device. The log of what recall did holds note numbers, counts and timings only, never a message or
a note's words, and is deleted after 30 days. How HQ treats what you use it for is in
`docs/security/ADOPTION_DATA.md`.
