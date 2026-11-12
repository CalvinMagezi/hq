# Offers: Later, a remembered Not now, and cooldowns

Server side of the proactive layer (`crates/hq-db/src/offers.rs`, `crates/hq-web/src/offers_api.rs`,
`crates/hq-tools/src/capabilities/`, migration 128). Everything here is adoption data: it stays on
the device and no model is ever told any of it (`docs/security/ADOPTION_DATA.md`, guarded by
`crates/hq-tools/tests/adoption_stays_local.rs`).

## Answers to an offer

`POST /api/offers/answer { message_id, answer }` takes `yes`, `not_now`, `later` or `scheduled`
(with `at`, next steps only). The answered offer leaves the reply's saved copy; 404 when that reply
has no open offer.

- **Later** parks the offer (`offer_later`) and leaves one quiet inbox item: source
  `scheduled_offer` (device-only, so never web-pushed), kind `fyi`, not owner-only, titled "Still
  want to {what it does}?". The item and the parking row are written in one transaction; if the
  item cannot be written the offer stays open (500) and a retry works.
  That item is the one resurfacing. It carries the action `open` (label "Bring it back", in
  `metadata.extra.open_label`) next to `dismiss`: `actions` is `["open","dismiss"]` for these items
  only. `POST /api/notifications/{id}/action {action:"open"}` engages the item and brings the
  offer back to the end of its chat at once (`open` on any other item is a 400). The daemon task
  `scheduled-offers` settles the same thing every minute as a fallback. Dismissing the item, or
  letting it expire (7 days), drops the offer. A client without the `open` button never brings the
  offer back. There is no second resurfacing: whoever deletes the parking row owns the return.
- An offer that came back (from Later or a schedule) carries `later_used: true` in `meta.offer`.
  Clients hide the Later button on it, and `later` on it is a 409. It can still be answered Yes
  or Not now.
- A returned offer is dropped, not shown, if its type has since been hidden, paused or set to
  Never. Its inbox item is settled either way.
- Answering a Later offer from the chat instead: `POST /api/offers/later/open { message_id }`
  brings it back now and settles the inbox item. 404 when nothing is parked for that reply.

## Not now, per type

A type is an offer kind (`dated_todo`, `next_look_up`) or, for a pill, a capability id from
`capabilities.yaml`. `hq_db::offers::KIND_CAPABILITY` maps each kind to its capability (a test in
`hq-tools` keeps it in step with the detectors and next steps). Answers are kept under the
capability id, so Not now, Never or Fewer on either side applies to both, and the summary has one
row per type. An id that maps to nothing and is not in the catalog is a 400.

| Rule | Value | Constant |
|------|-------|----------|
| One Not now hides the type | 7 days | `NOT_NOW_HIDE_SECS` |
| Not nows in a row that pause it | 3 (a Yes resets the run) | `NOT_NOW_RUN_TO_PAUSE` |
| The pause | 30 days, then the count starts again | `PAUSE_SECS` |
| Fewer like this | a Not now hides twice as long, for 30 days | `FEWER_SECS`, `FEWER_FACTOR` |
| Never show | until a Clear | `offer_type_memory.never` |

A later Not now (an older open offer answered after a pause started) never shortens a longer
hide. The 30-day pause is not doubled by Fewer.

`POST /api/offers/never { type }` and `POST /api/offers/fewer { type }` set the last two; an
unknown type is a 400. Pills for a hidden or never type drop out of `GET /api/capabilities`
suggestions (the capability is still listed).

## Cooldowns

`hq_tools::capabilities::offers::may_offer` runs before any choosing: no offer in a voice turn
(`source: voice` or a client id starting `voice-`), none while an owner approval is pending in the
thread, none after a failed turn, and at most two an hour (`MAX_OFFERS_PER_HOUR`). One offer per
reply is the shape of the chooser. The older spacing (a day between any two, three days for the
same type, `ANY_OFFER_GAP_SECS` and `SAME_OFFER_GAP_SECS`) still applies on top and is the calm
default. Offers that come back from Later or a schedule are marked `resurfaced` in `offer_log` and
count toward neither the hourly cap nor that spacing, so a returned offer never blocks a fresh one.
The ws turn skips the chooser altogether when the turn failed.

## Settings

`GET /api/offers/summary` returns, per type, `shown`, `tapped` (yes), `not_now`, `later` and
`never` for the last 30 days, with the type's catalog wording as `says`. `POST /api/offers/clear`
deletes the memory, parked Later offers and scheduled steps, and stamps every log row
`cleared_at`: they stop counting in the summary and in Not now counts, but keep feeding the pacing
(hourly cap, days between offers). An offer still open under a reply keeps working. A scheduled
step counts as a Yes in the summary; Later is its own count.

## Pills

`GET /api/capabilities` returns at most 3 suggestions, untried first. `says` and `example` in
`capabilities.yaml` are at most 40 characters; a test fails on longer ones.
