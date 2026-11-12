# Device actions: asking the owner's phone to do something

A device action is a request HQ sends to the owner's phone and waits for: put an event on the
calendar, read the day's events, look up a contact, later send a message. The calendar is the
first domain and contacts the second. This page is the platform they share and how to add the
next one. What each does with it is in `CALENDAR_CREATION.md` and `CONTACTS.md`.

## The pieces

| Piece | Where | What it owns |
|-------|-------|--------------|
| Store | `crates/hq-tools/src/device_actions/store.rs` | Open actions in memory: random single-use ids, ack and result windows, replay of a repeated request id, a cap on waiting actions per chat. |
| Link | `device_actions/link.rs` | One chat turn's connection to its phone: the phone's clock and zone, the way to push a frame, and the open, wait, settle round trip. |
| Spec table | `device_actions/spec.rs` | One `DeviceToolSpec` per tool, with its risk class, and the lookups governance and the result route use. |
| Domain | `crates/hq-tools/src/calendar/`, `contacts/` | Validators, payload builder, result filter, outcome vocabulary and the specs. |
| Routes | `crates/hq-web/src/device_actions_api.rs` | `POST /api/device-actions/{id}/ack`, `.../result` and `GET /api/device-actions/pending`. Domain-blind: they judge a result by its spec. |
| Agent wrapper | `crates/hq-agent/src/device_tool.rs` | One `DeviceActionTool` that turns any spec into an `AgentTool`. |
| Phone | `apps/android/` | One module per domain that answers a `device_action` frame by its `name`. |

Domains today:

| Domain | Tools | Wire actions | Risk | Skill | Page |
|--------|-------|--------------|------|-------|------|
| Calendar | `calendar_create_event`, `calendar_update_event`, `calendar_delete_event`, `calendar_list_events` | `calendar.create`, `.update`, `.delete`, `.list` | Create, Modify, Destructive, Read | `calendar-creation`, `calendar-editing`, `calendar-reading` | `CALENDAR_CREATION.md` |
| Contacts | `contacts_search` | `contacts.search` | Read | `contacts-lookup` | `CONTACTS.md` |

## Risk classes

The risk class is the only thing governance knows about a device tool. It never names one.

| Class | Meaning | Taints the thread | Asks the owner | With nobody to ask |
|-------|---------|-------------------|----------------|--------------------|
| `Read` | Returns text an outsider could have written | Yes, and the mark carries into later turns of the chat | No | Runs (but see below) |
| `Create` | Adds something new | No | Only once the thread is tainted | Does not run when tainted |
| `Modify` | Changes something that exists | No | Always | Does not run |
| `Destructive` | Removes something | No | Always | Does not run |
| `Outbound` | Sends something to another person | No | Always | Does not run |

Every device tool needs a live user turn, so an unattended session (a watch firing, a daemon
task) is never offered one of any class. The "does not run" column is the second line of defence,
for a session that has a tool but no approval store.

The approval line comes from the spec's `describe`: a "would ..." sentence that quotes only
cleaned, shortened text, because arguments are untrusted. The inbox card also shows the exact
arguments.

## Adding a domain

1. **Outcome vocabulary.** A module of string codes for what the phone may report, plus HQ's own
   (`device_unreachable` is shared and comes from `device_actions::unreachable`). One list of
   `phone_outcomes` per tool. A code outside the list is refused at the route.
2. **Validators.** Check arguments in HQ, against the phone's zone and clock
   (`DeviceLink::phone`), before anything is sent. Return an outcome, never an error. Reject
   unknown fields. Do all date arithmetic here, so the phone receives instants.
3. **Payload builder.** A JSON object that always carries a fresh `request_id`. A repeated tool
   call id with the same `request_id` returns the first answer instead of acting twice.
4. **Result filter.** `finalize(body, payload)` keeps only the fields the contract names, cleans
   every phone-supplied string, and compares the result with the payload. Anything it cannot
   verify becomes the domain's "unverified" outcome, never success.
5. **Skill.** A built-in skill in `crates/hq-tools/assets/skills/` and an entry in
   `skills/builtin.rs`. The spec names it in `skill`; without it active the tool returns the
   domain's "unavailable" outcome with reason `skill_not_active` and sends nothing.
6. **Specs.** One `DeviceToolSpec` const per tool: `name`, `wire_action` (`domain.verb`), `risk`,
   `skill`, `phone_outcomes`, `description`, `parameters`, `describe`, `finalize`, `unavailable`,
   `lost` (what the model gets when no answer came: a create may have happened, a read has nothing
   to report) and `run`. `run` validates, builds the payload and calls
   `DeviceLink::round_trip(&SPEC, tool_call_id, payload)`. Add the consts to a `SPECS` slice and
   list that slice in `DOMAINS` in `spec.rs`. Nothing else changes: governance, the routes, the
   agent wrapper and the builder read the table.
7. **Android.** A module that handles the new `name` values: check the arguments again, ack, act
   through the platform API, read back what was written, report, and keep a small ledger of
   recent request ids so a retry returns the first result. Ask for the permission in Settings,
   never in the middle of a chat.
8. **Tests.** The domain's own, and a case in `device_actions/tests.rs`. The fake domain in
   `device_actions/fake.rs` (feature `test-support`) shows that a new class of tool needs no
   edit to governance.

## Safety rules

- An action id is 128 random bits, valid for one action. It is the only handle the phone holds.
- At most 4 actions wait per chat; the three routes allow 60 calls a minute together.
- Phone-supplied strings reach the model only after being cut to one line and 200 characters.
- Nothing is stored on disk by HQ. A restart drops open actions.
- Logs hold request and action ids, the phase and an outcome code. Never a title, name, number or
  message body.
- Once a thread has read outside text, a `Create` asks the owner. A `Read` is itself an
  untrusted source. See `docs/security/PROMPT_INJECTION.md`.
- A tool of a class that asks is never listed as safe to skip: add it to the table with the right
  class and it follows the rule.

## Other apps

The same table is meant for contacts, messages and calls. Where a domain has an effect on another
person (a message, a call), its tools are `Outbound`. The design note is in the vault under
`Notebooks/Projects/Agent-HQ/Native-Device-Capabilities-Architecture.md`.
