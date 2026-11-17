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

On a new phone or machine, after restoring the vault backup:

```bash
hq suite sink register /path/to/the/folder        # asks for the code; points the sink at this machine's path
hq suite restore-originals --sink card            # asks for the code; puts every original back where it was
```

`restore-originals` reads the restored notes: each one whose original was sealed into the sink
names the file's path and hash, so every original is fetched, checked against its hash and written
back (never over a file that is already there), and the note is marked as having its original again.
It needs no job history. Running it twice finds nothing left to do.

Key shares (two of three trusted people restore), which would remove the single point of failure
of one code, are not built.

## What is not covered yet

- A drill on a wiped phone. The whole path (shrink, offload, encrypted backup, restore into a new
  folder, register the sink, restore the originals, hashes compared) was run with the real `hq` on
  a laptop, including a sink whose folder had moved. The plan's exit asks for it on a wiped device.
- A sink on another HQ over the bridge, and one in a cloud folder.
- Sending backups to a sink.

## A sink kept by a paired machine

A second computer can keep the sealed originals instead of a card or a drive. It stores ciphertext only: the originals are sealed on this phone under the recovery code before they leave, so the other machine never holds the code and cannot open them.

On the machine that will keep them (pair it with HQ first, as for any peer):

```bash
hq suite sink peer-serve ~/hq-sealed --port 5700
```

The store listens only on that machine's loopback address. HQ's pairing exposes the port to the phone as a service, and the phone reaches it through a forward on its own loopback address. On the phone, with the forward in place on `127.0.0.1:5700`:

```bash
hq suite sink add-peer phone-backup 127.0.0.1:5700   # shows the recovery code once
hq suite offload <job> --sink phone-backup
hq suite restore-drill phone-backup
```

`list` says whether the peer answers now. Every original is read back and checked before it counts as stored, exactly as for a folder sink. The store never overwrites a name it already holds, takes only names that are SHA-256 hashes, and refuses a body over 256 MiB.

After a wiped or replaced phone, the vault restores from its backup, and the sink can also be found again from the peer alone: with the forward in place, `hq suite sink register-peer 127.0.0.1:5700` reads the sink's open description from the peer and checks the recovery code, then `hq suite restore-originals --sink phone-backup` puts the originals back.

What is not done: the forward and the pairing are set up by hand today (the service config for `peer-serve`'s port and the forward on the phone), not by one button, and the phone has no screen for it.
