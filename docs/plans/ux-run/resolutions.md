# Resolutions

The last word on every conflict found between the phone boards, the web and hub boards, and the two
rule boards, after the final cross-platform check (5 Nov 2026). The order of authority for building is:

1. this file,
2. `decisions.md`,
3. `rules-shared.md` and `rules-web.md` (one set of rules; `rules-web.md` only adds what a browser or the
   hub needs),
4. the specs under `mobile/` and `web/`.

Where a spec still says something else, build what this file says and fix the spec in the same commit.
Items marked **Owner to confirm** are also listed in `decisions.md`.

## A. Rules

**A1. One set of rules.** `rules-shared.md` and `rules-web.md` are one set. Where they differ, this file
settles it; if this file is silent, `rules-shared.md` wins.

**A2. The failure catalogue** (replaces the second sentences in rules-shared section 2). Every failure
line is: "Couldn't {action}." + the reason sentence + the outcome sentence. The outcome depends only on
whether the action waits and retries by itself (it is in the outbox or retried) or not.

| Reason | Reason sentence | If it waits and retries | If it does not |
|---|---|---|---|
| HQ starting | HQ is still starting. | It will try again by itself. | Try again in a moment. |
| HQ restarting | HQ is restarting. | It will try again by itself. | Try again in a moment. |
| HQ stopped (phone, hub) | HQ has stopped. | It will try again when HQ is back. | Start HQ to try again. |
| No reply in time | HQ didn't answer in time. | (none) | Try again. |
| HQ can't be reached (web; phone talking to a server HQ says "Your server HQ") | Your HQ can't be reached. | It will try again when HQ is back. | Try again when it's back. |
| Signed out | You're signed out of this HQ. (phone: This phone is signed out of your server HQ.) | (none) | Sign in again. |
| Offline | You're offline. | It will try again when you're back online. | Try again when you're back online. |
| AI service down | The AI service isn't answering. | (none) | HQ is fine. Try again soon. |
| Download server | Couldn't reach the download server. | (none) | Check your internet and try again. |
| Download did not match | The download did not match. | (none) | Nothing was kept. Try again. |
| Phone almost full | The phone is almost full. | (none) | Free up {size} and try again. |
| Pairing off | Pairing is off. | (none) | Switch it on and try again. |
| Undo too late | It changed on another device, or too much time passed. | (none) | (none) |

The outbox strip keeps its own words ("Waiting to send"). Each row is a whole-sentence key per action
in the shared catalogue (N19), never assembled from fragments.

**A3. Colour** (adds to rules-shared section 3). The Inbox badge and every needs-you count are amber.
An "on" switch, a checked box, a chosen chip and a chosen card are ink (white on the dark theme), never
green, on phone and web. Matched words in search are bold, never washed in a colour. Charts are data,
not status: blue, sky, white, grey, indigo, in that order, the second series dashed, labelled directly.
This is the one exception to "colour means one thing".

**A4. Notification buttons** are ordered safe first on every surface: "Decline · Allow once · Open" on
the phone (Android 12+, details on, secure lock), "Open" alone on the web and in the hub's system
notices.

**A5. "Show details in notifications"** means different things by surface, on purpose: the phone may show
the request's text, the command included, only while unlocked and never on the lock screen; the web
shows only the item's own first line and never a command, because a browser cannot check a screen lock.

**A6. Undo.** The toast lasts the accessibility timeout on the phone (10 s on Android 8 and 9) and may stay
until closed on the web. Either way Undo calls a restore on HQ, which accepts it for 10 minutes. An Undo
that waits in the outbox and reaches HQ later fails with "Couldn't undo. It changed on another device,
or too much time passed."

**A7. When HQ counts as stopped:** the process is dead, or its start step has not moved for the longer of
60 s and twice its usual start time. No fixed timer. Same on phone and hub.

## B. Words and behaviour shared by phone, web and hub

| Id | Thing | Resolution |
|---|---|---|
| B1 | Approval cards (C2) | On every surface the card quotes the tool and its arguments verbatim from HQ's runtime, then HQ's description under "In HQ's words". Decline left, "Allow once" right; a losing request has a red "Allow {verb}". Notifications stay general (rules-shared section 1). |
| B2 | Sign out everywhere (C4) | "Sign out everywhere?" · "Every other device is signed out. This one stays signed in." · "Keep everyone signed in" · red "Sign out everywhere". |
| B3 | Sign out one device (C5) | Title 'Sign out "{device}"?' · body "It can no longer open HQ until it signs in again." · "Keep signed in" · red "Sign out device" · done line "{device} is signed out." The Inbox card's trigger "Sign it out" is ink; only the dialog's action is red. Devices are named by browser and system ("Chrome on Windows"). |
| B4 | HQ's own runs (C6) | While working: "HQ is working on it". Finished: "Done by HQ". The Helpers group is "HQ's own work". |
| B5 | Balance chip (C7) | "Today 85%"; over the limit "Today 104%" in amber. |
| B6 | Search (C8) | "Show all {N}". hq:// links never carry a query; a web address may keep `?q=` for back and forward only. |
| B7 | While you were away (C9) | The phone's words and sections everywhere: "Finishing soon", "Checked and finished", "Went back to you to finish", "Waiting on you". The web shows it as a link row on Tasks with the phone's first sentence ("HQ checked and finished 4 tasks."); the phone keeps its card. |
| B8 | Marking done (C10) | W33 on both: one task acts at once with "Done: {title} · Undo", unless other tasks wait on it or a check is pending (then it asks). Several: an Ask dialog ("Keep them as they are" · "Mark {N} tasks done"), then "{N} tasks done · Undo". |
| B9 | Stop HQ (C11) | Red only when helpers are working ("Stop HQ?" · "Keep it running" · red "Stop HQ", body names the helpers). With none working it is the Ask kind: "Stop HQ?" · "Keep it running" · green "Stop HQ", body "Chats and scheduled tasks pause until you start it again." |
| B10 | Backup (C12) | D1 and the phone's words: row "Last backup 3 h ago"; card "Last backup 3 d ago. A new one should be made every 24 hours." The web shows amber only when HQ's backup status route reports failed or overdue and the page can start a backup; otherwise grey "Backups are set up where HQ runs." |
| B11 | The HQ window in HQ Hub (C13) | Its web group is "This window". It keeps a saved copy and an outbox, keyed by the HQ's id, labelled "Saved in HQ Hub at 14:32". No web push; Needs you arrives as HQ Hub's system notice. **Confirmed by the owner, 5 Nov 2026.** |
| B12 | The word for a computer (C14) | "This PC" is the hub's tab name only (W44, owner to confirm). Everywhere else: the machine's own name, or "this computer". |
| B13 | Lite (C15) | The gear shows This browser and Other devices. Status line "Connected · HQ Lite". A link to a place Lite lacks opens Notes with "This HQ is HQ Lite. It has no {place}." Lite's Needs you never holds HQ's checks (Lite refuses the verification routes). |
| B14 | The Claude connector (C16) | Row name "Claude on the web"; "connector" only in Technical details. |
| B15 | Messaging apps (C17) | "Messaging apps" everywhere, including the phone's first-run step (was "Chat apps"). The web's Get started: "Let HQ reach you", "Open HQ on your phone", "Messaging apps (optional)". |
| B16 | Saved copy (C18) | One cap on both: 256 KB a place, 2 MB in all, per HQ. Times "at 14:32", dates "on Tue 3 Nov". |
| B17 | Data used (C19) | Monthly on every surface: phone Spending limits › Mobile data ("Data used by HQ this month"); web This browser › Data used ("this month"); the HQ window with the phone as hub: "Counted on your phone this month". |
| B18 | Small words (C20) | "{device} removed."; Disconnect Telegram keeps "To use Telegram again, you set it up from the start."; the Stop helper dialog is word for word "Stop Codex?" · "It stops now. Its work so far is kept." · "Keep it running" · "Stop Codex"; "Take a look" becomes "Open it to see."; the pairing code's "Cancel" becomes "Stop pairing"; counts are "{N} of {M} done" ("9 of 14 checked" becomes "9 of 14 done"); search placeholders say "notes, tasks, chats and helpers"; voice approvals say "You said: …". |
| B19 | Task approvals in notifications | Title "A task needs your OK" (never the task's name, which is content); a stuck helper "A helper is stuck · Open HQ to see which one."; a server question "Your server HQ needs your OK". |

## C. Decisions now applied on both platforms

| Id | Was on | Now also on | What |
|---|---|---|---|
| C-1 | Web | Phone | HQ restores every Undo within 10 minutes (A6). |
| C-2 | Web | Phone | Note edits carry the version they started from; "This note changed somewhere else." with See their version and Keep both. |
| C-3 | Web | Phone | New notes and tasks carry client ids; HQ drops repeats (N1 extended). |
| C-4 | Web | Phone hub | The one-time code screen says "Type this only into HQ on your own device. HQ never asks for it in a chat or a call."; a new sign-in is a Needs you item with "It was me" and "Sign it out"; sign-ins expire after 90 days without use. |
| C-5 | Web | Phone | The lockdown banner on Helpers: "All helpers are stopped. Allow them again in HQ Hub on that computer." |
| C-6 | Web | Phone | The phone's "New helpers: HQ answers their questions" becomes "Let HQ say yes to its requests", with the same rules (off by default, never a losing request, every answer listed). |
| C-7 | Web | Phone | An approval lapses after 24 hours: "This request ran out. Ask again from the helper." |
| C-8 | Web | Phone | HQ sends instants; times show in the phone's zone, naming HQ's zone when it differs; Spending says when its day resets. |
| C-9 | Web | Phone | The phone's web pages use `?embed=1` (both `?embed=1` and `?host=android` are accepted). |
| C-10 | Web | Phone | Note links: `hq://notes/n/{id}` beside the web's `/notes/n/:id`. |
| C-11 | Phone | Web | Notification folding and replay: "{N} more waiting for you", "{N} updates from HQ", waiting items post once when notifications return. |
| C-12 | Phone | Hub | The Connection sheet opens by itself at most once a day; "The phone is almost full" becomes "This computer is almost full" in the hub; the hub draws "Restarting HQ". |
| C-13 | Phone | Web | Word checks run on the web too: rendered text, a lint against literals, 15 words a sentence. |
| C-14 | Phone | Web | The route table marks phone-only Settings screens (assistant, update, speech). |
| C-15 | Both | (later) | Resume a stopped helper and archived helpers come to the phone after this run. A server HQ running Lite, drawn later. |

## D. Phone board conflicts (mobile/README.md section 5)

| Id | Resolution |
|---|---|
| D-1 | Notification words: rules-shared section 1 and B19 above; the details switch lines and channel lines are FixPlainWords'; "Off in Android settings". |
| D-2 | The running notice has no buttons on any version; a tap opens the app (FixOffline wins over FixStartup). |
| D-3 | The signed channel file: `withdrawn` is a list of versionCodes; `notes` is a map from language to up to three lines (`{"en": ["…", "…"]}`); `switches` gains `save_copy` (boolean) and `lock_screen_assist_off_models` (a list of model names, for N21). |
| D-4 | Save a copy lives in `safe/NotesCopy.kt`, used by both the safe-mode screen (its `:safe` process) and Backup's "Save a copy now". The file name uses FixSafeMode's pattern. |
| D-5 | The Connection sheet: its data model and words are wave 0 (`data/Connection`, `ConnectionWords`); its UI is built once in `ui/connection/` in wave 2 with Startup (the start steps, "usually about", "Stopped at", "almost full"); wave 4 extends it with saved copies and server rows. One "almost full" formula: under 200 MB free for the app's files (N15). |
| D-6 | Ready to start: rules-shared section 5 for the order; the assistant card body and the one-machine line are FixAssistantSetup's. |
| D-7 | Plurals: "1 of 3 sub-tasks done" (the count of done is not what pluralises); use plural resources keyed on the total. |
| D-8 | Strings move to resources in wave 0 (the catalogue and the lint); the FixPlainWords rewrites land in wave 1. |
| D-9 | Missing pieces to add while building: `hq://tasks/away` in FixWebPages; "No helpers match that." in FixSearch; a frame for N3 is drawn when wave 4 starts. |
| D-10 | `/api/verification/briefing` is owner-only and the phone signs in as the owner, so it may read it; confirm in `crates/hq-web/src/verification_api.rs` before building and stop if it is limited further. |
| D-11 | Test setup: wave 0 adds Compose UI tests (Robolectric or instrumented), a screenshot test, and an Android lint-checks module for the string rules. Acceptance checks that need them are written there. |
| D-12 | Code names the boards got wrong are corrected in the specs (`SectionLabel` in `ui/notes/NotesHome.kt`, `PhoneFix` in `ui/settings/PhoneWords.kt`; `LocalReselect` and `NotifAccess` are NEW). |

## E. Still owner to confirm

B12 with W44 ("This PC"), and the list in
`decisions.md`.
