# Creating and reading calendar events from the phone

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
- At most 4 actions wait per chat, and the three routes allow 60 calls a minute together.
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
- Calendar text is untrusted. `calendar_list_events` is in `UNTRUSTED_SOURCE_TOOLS`, so after a
  read `calendar_create_event` asks the owner. Logs carry the request id, phase, outcome code and
  counts, never titles. See `docs/security/PROMPT_INJECTION.md`.
