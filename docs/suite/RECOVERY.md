# If the phone is lost

The suite keeps three kinds of thing, and each has its own answer. This page says what survives
today, with nothing promised that is not built.

| What | Where it lives | If the phone is lost |
|------|----------------|----------------------|
| The record: notes and their pictures | The vault on the phone | Only as safe as your last copy of the vault that left the phone (below) |
| Originals you sealed into a sink | The sink folder (a card, a drive, another machine) | Safe, and openable with the recovery code |
| Originals you set aside in the trash | The phone's `_trash/` | Gone with the phone, and anyway gone after 30 days |

## The record

`hq backup` makes an encrypted copy of the vault (`docs/BACKUP.md`). The app's "Save a copy of my
Vault" makes a plain zip of the notes, which anyone who holds the file can read, so keep it
somewhere you trust. A copy that stays on the phone does not help if the phone is lost: save one
somewhere else (another device, your computer, a drive) now and then. Restoring it on a new phone
brings back every note and picture.

The suite does not yet send backups to a sink by itself. That is planned (a sink is how a backup
leaves the phone); until then, moving a backup off the phone is a step you do.

## Originals sealed into a sink

`hq suite offload` seals each original under a key made from a recovery code and removes it from
the phone only after reading it back. The sink folder, with the recovery code, is all that is
needed to get an original back on any machine running HQ:

```bash
hq suite sink add card /path/to/the/folder     # on the new machine, if the sink is not registered there
hq suite undo <job>                            # or fetch by hash
hq suite restore-drill card                    # checks a spread of what is stored
```

The code is the key. HQ never keeps it. **Write it down and keep it somewhere that is not the
phone** (a password manager, a sealed envelope, a person you trust). Without it nothing sealed into
the sink can be opened, by you or by anyone, and there is no reset.

Today a sink registered on a new machine needs its folder and code; the registration list itself
lives in the old vault, so the folder path is yours to remember. Key shares (two of three trusted
people restore), which would remove the single point of failure of one code, are not built.

## What is not covered yet

- A drill on a wiped device, from sink to restored files, with the hashes compared. The restore
  drill checks that stored originals open and match; it does not stand in for a real lost-phone
  rehearsal, which the plan requires before M3 is called done.
- A sink on another HQ over the bridge, and one in a cloud folder.
- Sending backups to a sink.
