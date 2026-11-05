# Adoption data stays on the device

HQ keeps a picture of how the person uses it, so that it can show them what else it can do and so
that the owner can see whether changes make it more useful. That picture is private. It is never
sold, never shared with a third party, never sent to a model provider and never posted anywhere.
It exists only on the machine that runs HQ (a phone or the owner's own server) and is used only by
HQ itself, for the person it describes.

This page says what the data is, where it may go, and what enforces that.

## What counts as adoption data

Anything derived from the person's own activity that describes how they use HQ:

- which capabilities they have used and which they have never tried (`used` in
  `hq_tools::capabilities::Offer`);
- the weekly numbers in `hq_tools::capabilities::adoption::AdoptionReport`: time to first useful
  result, capabilities used, corrections, active days, return in week four;
- the weekly digest item built from them (source `adoption_digest`);
- the offers shown under replies and how the person answered them (`offer_log`, migration 109:
  the offer's kind, when it was shown, Yes or Not now, never any message text), and the 30-day
  mute that two "Not now" answers in a row set.

It is computed on the device from records HQ already keeps there (chat messages and the tool-call
log in `.vault/_data/vault.db`), with no model.

## Where it may be used

| Use | Where it is shown | Leaves the device? |
|---|---|---|
| Ordering the empty chat's chips, untried capabilities first (`GET /api/capabilities`) | the owner's web app and phone app | no |
| The weekly digest in the inbox | the owner's web inbox and phone app | no |
| The offer under a reply, and its answer | the owner's web and phone chats; `offer_log` | no |
| `hq adoption` and `hq offers replay`, the same numbers on the command line | the terminal of the machine HQ runs on | no |
| `GET /api/adoption` and `GET /api/offers/replay`, the same numbers for `scripts/phone-hq.sh` | a computer the owner signed in as a device | counts only, to that computer |

These routes sit behind the web token or a signed-in device token
(`docs/security/WEB_AUTH.md`); MCP keys and claude.ai connector tokens cannot reach it.

## Where it may not go

- **A model provider.** The "Things you can offer" block in the agent's prompt
  (`hq_tools::capabilities::prompt_block`) is built from the catalog and this install only. Its
  signature takes no usage, `capability_block` in `crates/hq-agent/src/builder/prompt.rs` opens no
  database, and `the_models_prompt_does_not_change_with_what_the_person_used` builds one session on
  a fresh vault and one on a vault where every capability was used and checks the block is
  identical.
- **Web push.** A push travels through Google's, Apple's or Mozilla's push service. Items whose
  source is in `hq_core::types::DEVICE_ONLY_SOURCES` are never pushed, even with
  `web_push.push_all: true` (`adoption_data_is_never_pushed_even_with_push_all`).
- **Chat relays.** Telegram and Discord carry chat only (`CLAUDE.md`); nothing in this code path
  posts to them.
- **Anyone else.** HQ has no analytics or telemetry endpoint and none may be added for this data.
  `crates/hq-tools/tests/adoption_stays_local.rs` fails if the modules that read or report
  adoption data import an HTTP client, a model client, the relays, web push, sockets or child
  processes.

## Reading the numbers from a phone

`scripts/phone-hq.sh adoption` and `scripts/phone-hq.sh offers replay` ask the phone's HQ for the
two reports over the bridge (`GET /api/adoption`, `GET /api/offers/replay`, with a device token the
owner hands out from Settings, Devices and can sign out there). The phone computes them; only the
answer crosses, and that answer is counts and the catalog's own wording, never a message.
Do not copy `vault.db` or chat history off the phone to compute these elsewhere: a laptop session,
and above all an AI coding session, would then hold the person's chats, and a coding agent sends
what it reads to its model provider.

## "I can also do this for you"

Telling the person about something they have not tried is the reason this data exists, and it is
done on the device:

- the empty chat already puts untried capabilities first;
- after a reply to the person's own message, HQ runs deterministic detectors on that message
  (`hq_tools::capabilities::offers::for_turn`) and may attach one offer to the reply's saved meta
  (`chat_messages.meta.offer`: its kind, the line, and what Yes sends). The web and phone apps
  draw it under the reply with Yes and Not now (`POST /api/offers/answer`). The model never
  decides an offer and is never told about one: the chat history it is sent holds message text
  only (`an_offer_under_a_reply_never_reaches_the_models_history`). An answer removes the offer
  from the reply and is kept only in `offer_log`.

A model may still offer a capability on its own, from the usage-free block, when the conversation
calls for it.

## What this does not cover

Stated plainly so nobody relies on more than is true:

- The raw records it is computed from (chat messages, tool calls) are the same ones every chat
  turn already uses: the conversation goes to the model the owner chose, as it always has.
  This page is about the derived picture of use, which does not.
- An agent with a shell on a full install can read files on that machine, `vault.db` included.
  Anything it reads goes to its model. Use `governance.bash.sandbox` and the agent sandbox profile
  (`docs/security/BASH_SANDBOX.md`, `docs/AGENT_HOST.md`) to narrow that.
- Backups (`hq backup`) contain the database, encrypted with the owner's key
  (`docs/BACKUP.md`). Where the owner keeps them is the owner's choice.
- HQ is open source. This guarantee is about the code in this repository; a modified copy can do
  anything. The tests above exist so a change here cannot weaken it quietly.

## Changing this

Adding a new kind of adoption data: put its code in one of the files the guard test reads (or add
the file to it), and if it produces inbox items, add their source to `DEVICE_ONLY_SOURCES`. A
report that prints adoption data prints counts and catalog wording only; `hq offers replay`'s
`a_report_serializes_with_counts_only` is the pattern.
Sending any of it anywhere new is a change to this policy, not an implementation detail; it needs
the owner's explicit decision in review and an update to this page.
