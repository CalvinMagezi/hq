# Web wave 4 · 08 · Settings: the sub-pages, every state and every width (W34, W36, M12, M16)

Boards: **AlignedSettings** (AI service and keys open, Other devices, phone width), **AlignedSettings2**
(every connection state, one table, widths, 200%, Lite, the HQ window), **AlignedSettingsMore** (Messaging
apps; This browser pages are in wave 1), **AlignedSettingsMore2** (Claude on the web, Folders helpers may
use, Safety and limits, Backup and about, off and empty states, widths), AlignedReview M5, M12, M16, W34,
W36, Drift 11. Follows `../../rules-web.md` sections 2, 3 and 5 and `../../rules-shared.md` sections 4 and
5. Where this spec and the rules differ, the rules win.

`../wave-1/06-settings-groups.md` built the groups, the card slot and This browser. Spending limits is
`09-spending.md`.

## 1. Finding and the review items it closes

- **M16.** Only OpenRouter, Anthropic and Google AI can change here; DeepSeek has no Add. The "checked 2
  Nov" date is gone because HQ does not store it.
- **M12.** The backend's effort and address kept in Which AI answers; the helpers' safe box detail kept
  in plain words (`settings_api.rs`).
- **W34 (owner to confirm).** Claude on the web stays under Your accounts, off by default, with the off
  state drawn; no config keys on screen.
- **W36.** Claude in Your accounts; Folders helpers may use in How HQ behaves; Safety and limits tabs.
- **W09, M4.** Computers by their name ("Mac mini", "Studio PC"); Claude Desktop under HQ Hub's "Apps
  that use HQ".
- **Drift 11.** Approval dates read as dates: "approved 24 Oct", never a bare clock time.
- **W05, W37.** The Backup row is amber only when HQ reports a problem.
- **M1, M2, M3, WB1.** The states table; "Saved in this browser at 14:32"; no "Checking…"; signed out
  shows Sign in only; the web never says "stopped".

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/settings.tsx` one long page of sections (`NAV` at 437); `:207-208` the safe box
  as "open profile", " · credential stores visible", " · {n} extra allowed sites"; `:264` "Set
  mcp_connector.enabled and mcp_connector.public_url in the config, then restart HQ."; `:271` Disconnect
  every Claude client by `confirm()`; `:334` revoke a folder by `confirm()`; `:340` "Session folders".
- `components/settings/ProviderKeysSection.tsx:82-125` Replace or Add; `:103` "A new key is checked with
  the provider before it is saved, and the model choice stays as it is. Keys are never shown again."
- `components/settings/ChatAppsSection.tsx:143` "{Running | status}, seen {relTime}"; `:314, 318` confirms;
  `:367` "Chat apps".
- `lib/sessionFoldersApi.ts:25-31` "approved {clock} for {who}", "Stop allowing sessions in {path} on
  {host}?".
- `crates/hq-web/src/settings_api.rs:31` `editable_key()` (OpenRouter, Anthropic, Google only); `:131-132`
  `sandbox_hide_credentials`, `sandbox_extra_domains`.
- No backup status route; "Copy diagnostics" does not exist on the web today.

## 3. Decision

**Each sub-page is the phone's page with the web's extra rows inside it, in plain words. Every page has an
empty state that says how things get there. Rows from HQ keep their saved values under one strip when HQ is
out of reach; no row says "Checking…".**

Rejected: config keys on screen ("Set mcp_connector.enabled…"); a green "healthy" dot; a per-row
"Couldn't load".

## 4. Screens and states

### AI service and keys (Settings 1)

- **K1 · "Which AI HQ uses".** "Model · gpt-6-luna"; "For chats in Telegram and Discord · Same as above";
  "Only AI that runs on the Mac mini · HQ runs on the Mac mini. With this on, nothing goes to an online
  service."; "HQ cannot change which AI it uses from here yet."
- **K2 · "Which AI answers, in order".** "1 · GitHub Copilot · First choice · gpt-6-luna · effort high · On
  your plan"; "2 · OpenRouter · If the one before is down · claude-haiku-5.5 · Working"; "3 · Your own
  server · If the one before is down · qwen-coder · at llm.example.com"; "4 · DeepSeek · If the one before
  is down · deepseek-flash · No key set" (amber with an icon). The order HQ tries, each with its model, its
  effort when set and its own address when it has one; the address shows the host only, never a path or a
  key.
- **K3 · "Your keys".** "OpenRouter · Saved" [Replace]; "Anthropic · Set where HQ runs. Change it there,
  then restart HQ." (no button); "Google AI · Not set" [Add]. "Other services": "DeepSeek", "Groq",
  "OpenAI, Cerebras, Kimi Code" (listed, never added here). "Only the OpenRouter, Anthropic and Google AI
  keys can change from here. The others are set where HQ runs. A new key is checked with the service before
  it is saved. HQ keeps it where HQ runs and never shows it again." No "checked" date: HQ stores whether a
  key is set, not when it was last checked.
- **K4 · The key sheet** (bottom below 768, side from 768): "Add your Google AI key" / "A key is a long code
  that lets HQ use an AI service. Copy it from your Google AI account, then paste it here." / "Your key"
  (dots) / error amber with an icon "That key didn't work. Check that you copied all of it." / "Where do I
  find my key?" / "HQ keeps the key where it runs and never shows it again." / [Cancel] [Check and save].
  The key is gone with the sheet.

### Messaging apps (Settings more 3)

- **M1 · A card per app.** "Telegram · Running · seen 2 min ago" (grey dot), "Bot · @example_bot", "Your
  account · Linked" [Link a different account] [Disconnect]. "Discord · Not set up · Talk to HQ from
  Discord. It needs a bot you make in Discord, about five minutes." [Set up Discord]. "Changes restart HQ
  for a moment. This page reconnects by itself." "Not connected yet" when HQ has not reported since it
  started. Telegram and Discord carry chats only; nothing here sends notifications.
- **M2 · Set up Telegram.** "Open @BotFather in Telegram." "Send /newbot." "Pick a display name, then a
  username that ends in bot." "BotFather replies with a token. Copy it and paste it below." [Open Telegram]
  "Bot token" [Check the bot] → "Found your bot: @example_bot" (ink with a check) [Save and restart].
  Discord adds the invite link and the Message Content Intent check, as the web does today.
- **M3 · Disconnect asks** (stacked when the pair does not fit on one line, `../../decisions.md` WR27):
  "Disconnect Telegram?" / "HQ stops answering on Telegram and forgets the bot token. To use Telegram
  again, you set it up from the start. HQ restarts for a moment." / [Keep Telegram] [Disconnect Telegram]
  (`../../resolutions.md` B18).
  "Link a different account" asks with the phone's words ("Unlink your Telegram account?" · Keep my account
  · Unlink account).

### Other devices

As `../wave-1/05-sign-in-safety.md` D1 to D7 (the code card, rows by name, Keep signed in, Sign out
everywhere, the operator link).

### Claude on the web (Settings more 2, section 1)

- **C1 · On.** "Claude on claude.ai can use this HQ. It needs HQ to have a public web address. So it is
  off until whoever runs HQ turns it on." (HQ's own words never say "connector" outside Technical
  details, `../../resolutions.md` B14; Claude's own menu names below are quoted as Claude shows them.) "New connections · Open until 14:40" (blue
  countdown) / "In Claude, open Settings, then Connectors. Add a custom connector with this address and
  press Connect. Approve it on the page that opens here." / "Address for Claude" [Copy] / [Close it now].
  "Connected 2": "Claude · claude.ai · Read only · used 2 h ago" [Disconnect]; "Claude for the shop ·
  claude.ai · Tasks only · never used" [Disconnect]; [Disconnect all]. Technical details: "hq_call · allowed
  · 2 h ago", "hq_discover · allowed · 2 h ago", "hq_call · asked you · yesterday". "Claude Desktop and
  Claude Code on a Mac do not need this. They reach HQ through HQ Hub." Disconnect and Disconnect all open
  the losing-action dialog.
- **C2 · Off (the usual case).** "Claude on the web is off" / "Claude on claude.ai can use HQ only when HQ
  has a public web address. Whoever runs your HQ can turn it on where it runs." [How it works]. "On a Mac
  instead · Claude Desktop and Claude Code reach HQ through HQ Hub, with no public address. Open HQ Hub,
  then Apps that use HQ."

### Folders helpers may use (Settings more 2, section 2)

- **F1.** "Folders you let helpers use when Claude starts them. Each covers the folder and everything in it,
  on that computer." "Approved": "shop-site · Mac mini · ~/code/shop-site · approved 24 Oct for Claude"
  [Revoke]; "notes-site · Mac mini · ~/code/notes-site · approved 2 Nov for Claude"; "android · Studio PC ·
  ~/work/android · approved yesterday for Codex". "A helper may always use the HQ folder on the computer it
  runs on. That one is not listed and cannot be revoked here."
- **F2 · Revoke asks.** "Revoke the shop-site folder?" / "Helpers on the Mac mini must ask again before
  they work in ~/code/shop-site. A helper already working there keeps going." / [Keep it approved] [Revoke
  folder]. No Undo: the next request asks again.
- **F3 · Empty.** "No folders approved yet" / "When a helper asks to use a folder, the question comes to your
  Inbox. Allowing it adds the folder here." No button.

### Safety and limits, Backup and about (Settings more 2, section 3)

- **S1 · Tabs** Safety, Time limits, Helpers, Tools (the phone's four, read-only): "Read only. HQ cannot
  change these from here yet."
  - Safety: "Run commands in a safe box · Fences in the commands HQ runs for you, when the computer can. ·
    Required"; "Commands can use the internet · Lets those commands download things and reach websites. ·
    No"; "Ask me before HQ changes its skills · When on, a change to one of HQ's skills waits for your yes.";
    "Addresses allowed to open HQ · The web addresses that may show HQ's screens."
  - Helpers: "Default computer · Mac mini"; "Computers · Where helpers can run"; "Safe box for helpers ·
    Helpers cannot see HQ's own keys, other Claude accounts or other programs' secrets. The rest of the
    computer and the internet stay open to them. · On · open"; "Saved passwords hidden from helpers · The
    computer's password stores stay out of reach. · Yes"; "Stop idle helpers after · 12 h"; "New helpers:
    HQ answers their questions"; "How often HQ checks on helpers · 10 min"; "How many instructions HQ may
    give one helper · 20"; "Stop answering a helper that makes no progress after · HQ stops answering its
    questions for you. The helper itself keeps running. · 3 answers". Strict: "Helpers reach only the
    folders and websites on a list. · On · strict" and "Extra websites helpers may reach · Added to the list
    the strict safe box starts with. · Never" (shown only with the strict box). Off: "Off · helpers reach all
    you can".
  - Time limits: "Longest time for one reply · How long a reply can take before it moves to the background ·
    4 min 30 s"; "How long background work can run · 5 days"; "How often long work checks in · 5 min".
  - Tools: connected tools, the search server and the disk space check; a count opens its list.
- **S2 · Backup and about.** "Backup · Backup is on · Last backup 3 h ago. It runs every day where HQ
  runs." (ink with a grey check, never green; `../../resolutions.md` B10). Failed or overdue, and only
  when the page can start a backup: amber "Last backup 3 d ago. A new one should be made every 24
  hours." with [Back up now]. "About · HQ version · 0.9.2", notes "2,860", "Updates · HQ
  installs signed releases on its own. · By itself". "Support · Copy details · Versions and recent errors,
  with keys blanked". Not reported (today): "Backups are set up where HQ runs" / "This HQ does not tell the
  web about its backups yet." in grey (nothing is known to be wrong).
- **S3 · Failed.** "Couldn't load Safety and limits." / "HQ didn't answer in time. Try again." [Try again];
  codes in Copy details.

### Every state, one table (Settings 2)

| State | Status line | Rows from HQ | This browser rows | Buttons inside a section |
|-------|-------------|--------------|-------------------|--------------------------|
| Connected | Connected, grey dot, balance chip | Current values | Current | On |
| First load | Connected, no chip yet | A grey shape, never "Checking…" | Current | Off until the section loads. A spinner line, "Asking HQ for your messaging apps" |
| Starting | Starting HQ · 12 s, blue | Saved values under "HQ is starting. Saved in this browser at 14:32." | Current | Off: "You can change this once HQ has started." |
| Can't be reached | Your HQ can't be reached, amber | Saved values under "Your HQ can't be reached. Saved in this browser at 14:32." | Current | Off: "Changing this needs HQ. Your HQ can't be reached." |
| Reconnecting | Reconnecting, blue | Current values: only the chat link is retrying | Current | On |
| Offline | You're offline, amber | Saved values under "You're offline. Saved in this browser at 14:32." | Current | Off: "Changing this needs the internet." |
| AI service down | The AI service isn't answering, amber | Current. AI service and keys says so | Current | On |
| Signed out | Signed out of this HQ, amber | Nothing: wiped | Nothing | Sign in only ("This browser kept nothing from before. Sign in to see your settings again.") |
| No such page | Connected | The row says "Not on this HQ" | Current | "HQ answered, but this page is not here. It may be an older HQ." |
| A section failed | Connected | That row: "Couldn't load", amber with an icon | Current | "Couldn't load {the section}." and the reason, with Try again and Copy details |

Inside a section: saved, the strip, then saved values and every button that changes HQ off with one
sentence; never loaded: "Nothing saved in this browser yet" / "These settings come from HQ. Your HQ can't
be reached. It will try again when HQ is back." [Try again now]. A section failed: "Couldn't load your messaging
apps." / "HQ didn't answer in time. Try again." [Try again] [Copy details]. Inside AI service and keys with
HQ away: "Changing a key needs HQ. Your HQ can't be reached."

### Widths, Lite and the HQ window (Settings 2, 3 and 4; Settings more 4 and 5)

- **900 (768 to 1023):** the rail and one pane; the home list fills it; a row opens its section in the same
  pane with a back arrow. With "Notifications are off in this browser" in the slot, the Notifications row is
  amber too ("Off in this browser").
- **320:** the top bar holds the title and the state, which wraps; Search, the chip and the gear in one More
  menu; a second line that does not fit ends in an ellipsis (the section shows it in full); switch rows keep
  a 48 px target; tabs (Safety and limits) scroll sideways, the open one in view; each fact wraps its value
  under its title.
- **200%:** a 1280 window at 640: the phone layout and the bottom bar; toasts above the bottom bar, never
  over the focused field.
- **2200:** the list keeps 360 and the section its reading width, centred.
- **Lite (740, the phone layout with Lite's three places):** "HQ Lite has no other settings here. Keys,
  spending, messaging apps and helpers are not part of HQ Lite." The gear opens This browser (without
  Notifications unless allowed) and Other devices (WR2, B13); `/settings/spending` opens This browser with
  "This HQ is HQ Lite. It has no Spending limits." (WR28).
- **The HQ window (1100), showing HQ on your phone:** "These are the settings of HQ on your phone." Group
  "This window" (`../../resolutions.md` B11): "Shown by HQ Hub on this computer"; "Counted on your phone
  this month · 96 MB" (B17); "Sign out this window · Wipes what this window saved". Rows "OpenRouter · 1 key", "2 signed in", "$1.10 of $3.00 today", "Asks
  before changes outside notes", "HQ version 0.9.1". "Needs you on this computer · HQ Hub shows a notice on
  this computer. Its only button is Open." "Updates · This window has no web push. Your phone shows its own
  notifications." "The phone's own settings, like Background work and Keep running, are on the phone." No
  Install and version (HQ Hub updates itself). Saved copy, outbox and sign-in keyed by HQ's id; the strip
  reads "Saved in HQ Hub at 14:32".

## 5. Exact copy

Every string quoted in section 4 is exact. Keys `settings_*`. Retired: "open profile", "credential stores
visible", "{n} extra allowed sites", "Set mcp_connector.enabled and mcp_connector.public_url in the config,
then restart HQ.", "approved {clock} for", "Stop allowing sessions in {path} on {host}?", "Chat apps",
"Session folders", "Copy diagnostics", "checked {date}", any green status dot.

## 6. Behaviour rules

1. **Editable keys** are exactly `editable_key()`'s three; a key set where HQ runs says so with no button.
2. **A new key is checked before it is saved** (`/api/settings/provider-key`, with the client header).
3. **Which AI answers** shows the chain with model, effort and the host of an address only.
4. **Messaging app health** is a grey dot with words; "Not connected yet" before HQ has reported.
5. **Claude on the web** never shows config keys; Disconnect and Disconnect all are red dialogs.
6. **Folders** show the computer by name, the path in full, the day, and which helper asked; Revoke asks.
7. **Safety and limits** is read-only; the safe box reads in plain words from `settings_api.rs` fields.
8. **Backup** is amber only when HQ's backup status route reports failed or overdue and the page can start
   a backup; otherwise grey, and until a backup route exists the grey "Backups are set up where HQ runs."
   (`../../resolutions.md` B10).
9. **Copy details** blanks keys and includes What HQ noticed.
10. **States** per the table; no row says "Checking…"; one strip per place.

## 7. Browser and platform notes

- Copy buttons use `navigator.clipboard.writeText` (needs a secure context; on plain http on a LAN, fall
  back to a selected text field with "Press Ctrl C to copy").

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/settings/*.tsx` | One file per section (from `settings.tsx` sections). | NEW |
| `apps/hq-web/src/components/settings/ProviderKeysSection.tsx` | K1 to K4 (82-125). | yes |
| `apps/hq-web/src/components/settings/ChatAppsSection.tsx` | M1 to M3 (143, 314-367). | yes |
| `apps/hq-web/src/routes/settings.tsx` | Claude, Folders, Safety, Backup move out (207-340). | yes |
| `apps/hq-web/src/lib/sessionFoldersApi.ts`, `connectorApi.ts` | Date words; dialogs. | yes |
| `crates/hq-web/src/settings_api.rs` | Fields stay; add backup and update status when the routes exist. | yes |

## 9. Server, desktop or release changes

Server: backup status, update status and diagnostics routes (`../../changes.md`); none for the rest.

## 10. Acceptance checks (each fails on today's build)

1. DeepSeek has no Add button; Anthropic set where HQ runs has no button. (Check against
   `editable_key()`.)
2. The safe box reads "On · open" and "Saved passwords hidden from helpers · Yes". Today "open profile ·
   credential stores visible".
3. Claude off shows "Claude on the web is off" with no config key. Today the `mcp_connector` line.
4. A folder approved twelve days ago reads "approved 24 Oct for Claude" (fixture date). Today a clock time.
5. With HQ unreachable, Messaging apps shows saved values under one strip and Disconnect is off with
   "Changing this needs HQ. Your HQ can't be reached." Today the section errors.
6. No owner-facing text on Claude on the web contains "connector" outside Technical details and Claude's
   own quoted menu names; Backup reads "Last backup 3 h ago"; the Disconnect Telegram body contains "To
   use Telegram again, you set it up from the start." (B10, B14, B18).

## 11. Check on a laptop or phone

- Every Settings page at 320, 900 and 1440; with HQ stopped; signed out.

## 12. Open decisions it depends on

- **W34** (owner to confirm). **W36, WR2, WR5, WR27, WR28** (Recommended), `../../resolutions.md` B10,
  B11, B13, B14, B17, B18.
- Depends on: `../wave-1/06-settings-groups.md`, `../wave-1/05-sign-in-safety.md`,
  `../wave-1/02-dialogs-approvals.md`, `09-spending.md`.
