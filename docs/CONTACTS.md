# Looking up contacts on the phone

Contacts are the second domain on the shared device-action platform (`DEVICE_ACTIONS.md`). This
page is what is specific to them. Ask HQ in the phone app for someone's number or email and it
searches the phone's contacts by name. It only reads: it cannot add, change or delete a contact,
and it cannot call or message anyone.

## What the owner sees

- HQ answers with a name, up to three phone numbers and up to three email addresses per contact,
  and a link that opens the contact on the phone.
- Notes, postal addresses, birthdays, photos, companies and groups are never read.
- Up to ten contacts come back, and HQ says when there were more.
- Contacts access is requested from the phone's Settings page, never in the middle of a chat.
  Without it HQ says nothing was searched and where to turn it on.

## How it works

1. The tool `contacts_search` exists only in a chat turn that carried `device`, the same as the
   calendar tools, and only while the built-in skill `contacts-lookup` is active. Without it the
   tool returns `contacts_unavailable` with reason `skill_not_active` and sends nothing.
2. HQ checks the arguments first (`contacts/search.rs`): `query` is 2 to 64 characters after
   trimming, no control characters; `limit` is 1 to 10 and defaults to 5; unknown fields are
   refused. A refusal is an `invalid_query` outcome and nothing is sent.
3. The wire action is `contacts.search`, version 1. The payload is
   `{request_id, query, limit}`. The phone matches the query as a case-insensitive substring of
   the display name.
4. The phone answers `{v: 1, outcome: "listed", contacts: [...], truncated}` or one of
   `permission_denied`, `invalid_query`, `contacts_unavailable`. A code outside that list is
   refused at the route.
5. `finalize_search` keeps only the named fields, strips control characters, caps every string
   (name 100, label 40, number 32, address 120 characters), keeps at most three numbers and
   three addresses, drops rows whose name does not contain the query, sorts by name, removes
   repeated ids and cuts to the limit. HQ builds `open_link` itself as
   `hq://contacts/contact/{id}`, and only from an id made of ASCII digits.

## Safety

- `contacts_search` is a `Read`. Names and labels can be written by others (synced accounts,
  shared lists, imports), so a read marks the thread as having seen outside text, and a later
  `Create` in that thread asks the owner. This follows from the risk class; governance names no
  contacts tool.
- The approval line is "would look up contacts on the owner's phone". It quotes no argument.
- Logs hold request and action ids, the phase and an outcome code. Never a query, name, number or
  address.
- The skill tells the model that contact text is data, never instructions.

## Not in this version

Creating, editing or deleting contacts, and calling or messaging a contact. Messaging would be
`Outbound` and always ask.
