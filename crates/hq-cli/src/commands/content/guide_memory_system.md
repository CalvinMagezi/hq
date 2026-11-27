---
noteType: guide
fileName: memory-system
version: 2
---
# Memory System Guide

HQ keeps two kinds of memory. They live in different places and behave differently, so it helps to know which is which.

## 1. Your facts: `MEMORY.md`

`_system/MEMORY.md` is a plain markdown file you can read and edit by hand. HQ loads it at the start of every chat.

- Add a fact with `hq memory add "..."`. It goes under "Key Facts".
- Read it with `hq memory show`, or list its bullet points with `hq memory facts`.
- A section called "Agent Insights (Auto-Generated)" is rewritten by HQ. Each line is marked `(inferred)`: it is a guess HQ made from your chats, not something you told it. Edit the rest of the file freely.

Other files in `_system/`: `SOUL.md` (how HQ behaves), `PREFERENCES.md` (your preferences), `CONFIG.md`.

## 2. What HQ picked up from your chats

After a chat turn, HQ may save a short summary of it. These are **not** markdown files. They are rows in HQ's database (`_data/vault.db`), so you cannot open or edit them as notes.

- A turn is only considered if HQ's reply is more than a short answer, and never if the chat read outside content such as a web page or email.
- Secrets that match known shapes (API keys, tokens) are removed before anything is saved. Odd formats can slip through, so do not paste secrets into chat.
- Only your own chats are learned from. A guest's messages, for example a family member on Discord, are not saved and do not see your memory.
- Each chat shows HQ the most important saved summaries. They are picked by importance and age, not by what you just asked. Matching them to your message is planned.

### Consolidation

About every two hours, if at least five new summaries have built up, HQ writes an insight note to `Notebooks/Memories/`. These notes are marked `status: inferred`. The original summaries are kept; consolidation does not merge or delete them.

### Forgetting

Once a day HQ lowers the importance of old summaries and removes the weakest ones after about 60 days, keeping a short copy in `_archive/memories/`.

`hq memory stats` shows how many summaries exist and when consolidation last ran.

## Best practices

1. **Keep `MEMORY.md` short.** It is loaded every time. Put long detail in a note under `Notebooks/`.
2. **Tell HQ what matters.** If something must be remembered, add it with `hq memory add`. Do not rely on HQ picking it up from a chat.
3. **Date your facts.** A date makes it obvious when a fact has gone stale.
4. **Review now and then.** Run `hq memory show` and remove what is no longer true.
