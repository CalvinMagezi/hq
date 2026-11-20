# Personal finance from mobile-money messages

`hq finance` turns the MTN and Airtel transaction messages you choose to export into a ledger you
review, on your own machine. This page is the data-flow review: where a copy of your data can be,
and what keeps it from going further. Status: manual import only. Nothing reads your inbox, and no
automatic capture exists.

## The flow

1. You export messages to a JSON file (a list of `sender`, `date`, `body`). You pick the file.
2. `hq finance import <file>` parses each message into a draft and adds the new ones to
   `_data/finance/ledger.md` in your vault, which is a Sheets note.
3. You open the note in Sheets, set each row's category and flow, and change `status` to
   `confirmed`. Only confirmed rows are totalled.
4. The tiles and chart in the note are computed by the Suite.

## From the phone

Ask HQ in the phone chat where your mobile money went, and HQ calls `finance_import_messages`.

1. The tool is a `Modify`, so you are asked to approve every import. The approval line says what
   will be read: your MTN and Airtel Money transaction messages, from the last N days or since the
   last import.
2. The phone reads only the senders in `hq_finance::parse::TRANSACTION_SENDERS`. The query names
   them, so no other message is loaded.
3. The phone posts the messages to `POST /api/finance/import/{action_id}`. The id is random, single
   use and valid only for an approved, acknowledged action. The route drops any sender the action
   did not name, parses, adds new rows to the ledger and settles the action with five counts.
4. The model is told the counts and a link. It never sees a message, an amount or an id.

The first time, the phone has no permission to read messages. Android grants it only from
Settings, Finance: the chat answers `not_set_up` or `permission_missing` and points there. It is
never requested in the middle of a chat.

## Where a copy can be, and why it goes no further

| Place | Holds | Kept closed by |
|-------|-------|----------------|
| The phone's message store | Raw messages | Read-only: only the allowlisted senders are queried, and the app keeps no copy |
| The import request | Raw messages, in transit to HQ | Single-use action id, signed-in routes only, the MCP key does not pass, body size capped, body not logged. If HQ runs on another machine they cross your paired link, not the internet |
| The export file | Raw messages | You made it and you delete it. HQ never copies it |
| `hq-finance` | Parsed drafts in memory | The crate has no dependency on a model, vault, database or network crate (`tests/boundary.rs`) |
| `_data/finance/ledger.md` | Amounts, dates, ids. No names or phone numbers: the parser does not keep them | `_data` is a private vault folder: `vault_read`, `vault_list` and the other read tools refuse it, and the vault listing skips it |
| Search and embeddings | Nothing | The index reconcile walks `Notebooks/` only (`hq-db/src/search/reconcile.rs`), the vault note walker skips `_data` (`hq-vault/src/notes.rs`) and the vault watcher's scan skips it (`SCAN_SKIP`) |
| Chat and model context | Nothing | Agents reach notes through the vault tools, which refuse `_data` |
| Backups | The ledger, inside the archive | `hq backup` seals the archive with a passphrase or key file (`docs/BACKUP.md`). Copies you make by hand are not covered |
| Logs and command output | Counts only | `hq finance` prints numbers; `validate::Report` has no text fields |
| Repository fixtures | Redacted text | Allowlist redaction, random salt, one-time-code messages dropped (`src/redact.rs`) |

## What this does not protect against

- A phone or laptop that is unlocked, lost without disk encryption, or rooted.
- The vault is a folder of plain files. Anyone who can read your vault folder can read the ledger.
  Use full-disk encryption.
- The Suite's sheet APIs and the owner's signed-in web and phone apps read any vault path,
  including `_data`; that is how the ledger is shown. The boundary against agents is the vault
  tools, not the Suite. Review any new tool that takes a raw path before it ships.
- Deleting a ledger row does not remove it from earlier backups.

## Not built yet

- Capture on the phone (a share target or a file picker). Today you export from a computer.
- Airtel Money has few samples, so its parser is the least tested.
- Transfers between your own wallets, loan fee handling, and fees the message does not state:
  see `docs/techdebt/`.

## Not legal advice

This describes behaviour, not compliance. The plan notes a Uganda legal and privacy review before
any public pilot or any processing of other people's data.
