---
name: finance-import
description: Import the owner's mobile-money messages into their private ledger. Use for where money went.
hints: [finance_import_messages, mobile money, momo, airtel money, where did my money go, my transactions, spending, import my messages]
provenance:
  mintedBy: hq
---

# Finance import

You ask the owner's phone to read their MTN and Airtel Money transaction messages and add them to their private ledger. **You never see the messages.** The phone hands them straight to HQ's ledger, and the tool tells you only counts: how many messages it saw, how many it ignored, how many were transactions, how many rows are new, and how many rows now wait for the owner to confirm. It reads only the mobile-money senders, never other messages. It cannot send or alter a message, and it cannot move money.

## Before you call the tool

1. **Ask only if you must.** The owner is asked to approve every import, so you do not need to ask first. Call the tool when they ask to see, pull in or update their mobile-money records.
2. **Range.** Leave `since_days` out: the phone reads what is new since the last import. Pass it (1 to 365) only when the owner names a period, such as "the last month" (30).
3. **Never ask the owner to paste messages into the chat.** If they offer to, say the import reads them on the phone and nothing needs pasting.

## After it succeeds

Report the counts in one or two plain sentences, then the link. Example: "I found 18 transactions and added 12 new rows. 12 are waiting for you to review." Then show `open_link` as a markdown link labelled "Open ledger". Copy the link exactly; never build one.

- Rows start as **drafts**. Totals count only rows the owner has confirmed, so say the owner needs to review and confirm rows (set status to confirmed in the sheet) before the figures include them.
- Say "from the messages captured". Never present the totals as the owner's whole financial picture: the ledger holds only what the phone's messages contained, and a missing period is not zero.
- If `added` is 0 and `waiting_for_review` is above 0, say nothing new arrived and that rows are still waiting.
- If `transactions` is 0, say no transaction messages were found in that range.

## What you must not do

- Do not quote, guess or reconstruct any amount, name, number or message. You do not have them. If asked what a transaction was, send the owner to the ledger.
- Do not read the ledger yourself. It is in a private folder no tool opens.
- Do not offer lending, credit, scoring, advice on borrowing, or any money movement. This is a record of what happened, nothing more.
- Do not import again to "check". One import per request.

## Reading the outcome

- **imported**: report as above.
- **not_set_up**: the owner has not turned on Finance in the phone's settings. Tell them where: Settings, then Finance. Do not retry until they say it is done.
- **permission_missing**: the app is not allowed to read messages. Tell them to allow it from Settings, Finance, and that this is asked only there, never in a chat.
- **read_failed**: the phone could not read its messages right now; give the reason if there is one.
- **invalid_request**: the arguments were refused; the reason names the problem. Fix it if you can, otherwise ask. Retry once.
- **finance_unavailable**: finance is not available right now; give the reason if there is one.
- **device_unreachable**: the phone did not take the request; say so and do not guess an answer.

## Examples

- "Show me where my mobile money went." Call with no arguments, then report the counts and show the link.
- "Update my transactions for the last two weeks." Call with `since_days` 14.
- The owner pastes a message and asks you to add it. Do not read it into the ledger by hand: say the import reads messages on the phone, and offer to run it.
- The tool returns `not_set_up`. Say Finance is switched off and point to Settings, then Finance.
