# Creating, reading, changing and deleting calendar events on the phone

The calendar is the first domain on the shared device-action platform (the store, the link to the
phone, the risk classes and how to add a domain are in `DEVICE_ACTIONS.md`). This page is what is
specific to the calendar.

Ask HQ in the phone app to put something on your calendar and it is saved straight to the
phone's calendar, with no review screen. A clear request creates the event. A request with a
missing or ambiguous detail gets one short question first.

## What the owner sees

- The event is saved to the calendar chosen in Settings, "Calendar HQ saves events to". HQ never
  picks one for you, even when only one exists.
- A timed event with no length is one hour, and HQ says so in its reply.
- HQ says "saved to your phone calendar". Whether that calendar syncs elsewhere depends on the
  account it belongs to.
- Calendar access (read and write) is requested from the same Settings page, never in the middle
  of a chat. Without it HQ says nothing was created and where to turn it on.
- Attendees, recurrence and reminders are not supported yet. HQ says so instead of dropping them.

## How it works

1. The tool `calendar_create_event` exists only in a chat turn whose message carried `device`
   (`{"tz": "<IANA zone>", "now_utc": "<RFC 3339>"}`), which the Android app adds to every chat
   frame. A phone clock more than 10 minutes off HQ's clock counts as no device. "Now" and the
   time zone always come from the phone, never from the server.
2. The built-in skill `calendar-creation` must be active, or the tool refuses. The skill tells the
   model when to ask a question and how to word each outcome.
3. HQ checks the request (`crates/hq-tools/src/calendar/check.rs`): lengths, formats, end after
   start, a start not more than 5 minutes in the past, a start within 5 years, and local times
   that do not exist or happen twice around a clock change.
4. HQ sends a `device_action` frame on the chat socket. The app answers on
   `POST /api/device-actions/{id}/ack` within 5 seconds and
   `POST /api/device-actions/{id}/result` within 20 more. `GET /api/device-actions/pending`
   lets the app settle an action after a dropped connection.
5. The app checks the request again, resolves the preferred calendar, inserts through Android's
   Calendar Provider, reads the row back and reports. HQ returns one of `created`,
   `clarification_required`, `permission_denied`, `invalid_event`, `calendar_unavailable`,
   `device_unreachable` or `creation_unverified`. Only `created` means the event exists.

All-day events: `start_local` and `end_local` are dates and `end_local` is the last day, inclusive.
The phone stores them as UTC midnight to the next UTC midnight, as the provider requires.

## Safety

- An action id is 128 random bits, valid for one action, and dies with it. It is the only handle
  the phone holds. The `/ws` socket is one shared broadcast, so the id is not tied to a socket.
- At most 4 actions wait per chat, and the three routes allow 60 calls a minute together
  (`DEVICE_ACTIONS.md`, "Safety rules").
- The phone's strings (calendar names, accounts) reach the model only after being cut to one line
  and 200 characters, and at most 8 choices are shown. The skill tells the model they are labels.
- Once a turn has read outside text, `calendar_create_event` asks the owner first
  (`PROMPT_INJECTION.md`, rule 4) and does not run where nobody can be asked.
- Nothing is stored on disk by HQ. The phone keeps a small ledger of recent request ids so a
  retried request returns the first result instead of inserting twice. Clearing app data forgets it.
- Logs hold request and action ids, the phase and an outcome code. Never titles, places,
  descriptions or calendar names.

## Limits

- Real-phone behaviour of the Calendar Provider is covered by the unit tests only through fakes.
  Test on a device after a change here.
- An all-day event on a past date is created without a question.
- A timed event with an explicit end has no maximum length.

## Reading events

Ask what is on your calendar and HQ reads it from the phone with `calendar_list_events`. It is
offered only in a turn that carried `device`, and only while the built-in skill `calendar-reading`
is active. It uses the same `device_action` channel and routes as creation, with the action
`calendar.list` (wire version 1).

- Arguments are optional: `start_local` and `end_local` (phone-local, `YYYY-MM-DDTHH:MM` or
  `YYYY-MM-DD`, end exclusive). The default window is today in the phone's zone. There is no
  maximum span. HQ turns the window into epoch milliseconds with the phone's zone (DST aware), so
  the app does no calendar arithmetic. Payload: `request_id`, `from_ms`, `to_ms`, `zone`, `limit`.
- The app returns `listed` with `events` (`id`, `begin_ms`, `end_ms`, `all_day`, `title`,
  `calendar_label`), `truncated` and `next_from_ms`; or `permission_denied`,
  `calendar_unavailable`, `invalid_event`. It reads every visible calendar, skips deleted events,
  keeps declined ones, and shows a recurring event once per occurrence.
- HQ keeps at most 50 events (`CALENDAR_LIST_MAX_EVENTS`). When more exist the result says
  `truncated` and gives `next_start_local`, the start of the first omitted event in the phone's
  zone. The model sees per event only: a cleaned one-line title (200 characters), local start and
  end (all-day events as dates), the calendar label and `open_link`. Location, notes, guests and
  conference links are neither requested nor passed on.
- An all-day event is stored as UTC midnights, so HQ judges it by dates: its own dates in UTC
  against the window's dates in the phone zone. One that fails is dropped before the cap.
  A timed event must start before the window ends and end after it starts.
- `open_link` is built by HQ from numbers only: `hq://calendar/event/{id}?begin=..&end=..`. An id
  that is not plain digits drops the event. The phone never supplies a URL; the app opens the
  event in the calendar when the link is tapped.
- Calendar text is untrusted. `calendar_list_events` is a `Read` spec, so after a
  read `calendar_create_event` (a `Create` spec) asks the owner, in that turn and in every later turn of the same chat thread. Logs carry the request id, phase, outcome code and
  counts, never titles. See `docs/security/PROMPT_INJECTION.md`.

## Changing and deleting events

`calendar_update_event` and `calendar_delete_event` act on one existing event. They are offered
only in a turn that carried `device`, and only while the built-in skill `calendar-editing` is
active. They use the same channel, routes and timeouts as creation, with the actions
`calendar.update` and `calendar.delete` (wire version 1).

- **Naming the event.** The model gets the event from `calendar_list_events` and passes its
  `open_link`, its `title` and its `start_local` as listed. HQ parses the link strictly
  (`hq://calendar/event/{digits}?begin={digits}&end={digits}`, ids and times at most 18 digits, no
  other parameters; anything else is `invalid_event` with reason `bad_link`) and checks that
  `start_local` equals the link's begin in the phone's zone (an all-day event by UTC date; reason
  `start_mismatch`). A date-only `start_local` means an all-day event.
- **What can change.** `new_title`, `new_start_local` and `new_end_local`, at least one (reason
  `no_changes` otherwise). Nothing else: no place, notes, guests, reminders, recurrence or
  calendar. Timed stays timed and all-day stays all-day (reason `kind_change`). With only a new
  start the length is kept. A new timed start follows the creation rules (not in the past, not
  years away, a repeated local hour asks which one); an all-day end is the last day, inclusive,
  stored as the next UTC midnight. Renaming an event that already started is allowed.
- **Wire.** Delete payload: `request_id`, `event_id`, `begin_ms`, `expected_title`, `zone`.
  Update adds `all_day` (HQ's reading of the event's kind, so the phone can refuse a mismatch) and
  `changes`: `title`, `start_ms`, `end_ms`, already converted by HQ with the phone's zone (DST aware).
  A new start always comes with its end. The phone never receives local times. `expected_title` is
  the title as the phone cleans it; an event listed as "(no title)" is sent with an empty one.
- **Phone checks, in order:** permission, the event exists and is not deleted, the calendar allows
  edits, the event is not recurring, then title and begin still match. It acts, reads the row
  back, and reports `updated` or `deleted` only when the read-back agrees; otherwise
  `update_unverified` or `delete_unverified`. A retried request id returns the first result.
- **Outcomes the model sees:** `updated`, `deleted`, `event_not_found`, `event_changed` (the event
  changed since it was listed), `read_only_calendar`, `recurring_unsupported`, `invalid_event`
  (with a reason), `permission_denied` (with where to turn it on), `calendar_unavailable`, and
  HQ's own `device_unreachable`, `update_unverified`, `delete_unverified`. Only the first two mean
  it happened. A result outside this set is stored as unverified, never as success.
- **Limits.** A recurring event (a rule, an instance or an exception) is refused in this version
  (`recurring_unsupported`); the owner uses the calendar app. Any editable event the owner can read
  may be changed, not only ones HQ created.
- **Approval.** These are `Modify` and `Destructive` specs, so every call asks the owner first, in a clean chat too, and does not run where
  nobody can be asked. The inbox card shows the exact arguments plus a line naming the action,
  the cleaned title and the changes. See `docs/security/OWNER_APPROVAL.md`.
- Logs hold request ids, the action, the phase and an outcome code, never titles.
