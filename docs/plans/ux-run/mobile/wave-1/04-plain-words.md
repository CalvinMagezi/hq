# Wave 1 · 04 · Plain words, strings in resources, the rendered-text check (finding #11)

Board: **FixPlainWords** ("Fix 11: plain words"), revised after review, 5 Nov 2026.
Area: every `*Words.kt` file, the Kotlin string literals around them, and `res/values`.
Follows rules sections 1, 2 and 7 (`../../rules-shared.md`); where this board differs, the rules win.

Note on waves: FixPlan puts "strings moved to resources" in wave 0 and the plain-word rewrite in wave 1.
Decision N20 says "today's 172 reach zero in wave 0". So the resource move, the lint and its baseline
land in wave 0; the rewritten words (this file's tables) land in wave 1.

## 1. Finding and the review items it closes

**#11 Config words reach the owner.** The app prints HQ's config and code words as if they were
sentences: "global, daily, then block", "all 85%", "Follow every build of main · Only builds the owner
promoted", "hq on hq-oss", "Using n0's public relays".

**Who it hurts, how often.** Every owner who is not a developer (a shop owner in Kampala setting HQ up
on a Galaxy A32), on the screens they open most. The balance chip sits on every tab's top bar, the
helper strip on Tasks, Spending on Settings home. Many of these owners read English as a second
language.

What changed in this revision:

- **Strings move to resources (M12):** whole-sentence keys, plural resources, the phone's own date and
  time format. Words files keep the logic and return resource ids.
- **The word check reads what is drawn:** it renders every fixture and reads every Text node, not just
  Words files. About 170 inline `Text("…")` literals escape today's idea (for example
  `SettingsScreens.kt:1019-1020`, `TasksScreen.kt:205`). A lint stops new ones. (Review #11: "The word
  test reads only Words files; 168 inline Text literals … escape it.")
- **The banned list no longer hits plain English:** "main", "session" and "API" are out. Technical
  details and Copy details are exempt. `{owner}/{repo}` is allowed.
- **ESL rules (rules section 7, M13):** 15 words a sentence at most, no semicolons, no idioms. Checked by
  the same test, then with five owners.
- **Glossary adds** HQ and AgentHQ, "Saved on this phone at {time}", "Connected", "Not now", "Open
  Android settings", "{N} of {M} done".
- **Channels:** the five rules section 1 channels by exact name, with this board's words for how each
  alerts.
- **Fixed in the After frames:** "Live" is "Connected" with a grey dot, no green "Allowed" check,
  failure lines amber with an icon in rules section 2 form, Remove triggers in ink, the backup card as
  FixBackup draws it.
- **Added:** a 360 dp frame at font scale 2.0, the Android versions card, acceptance checks that fail on
  today's build.

## 2. Before (as built, verified at `c04a100`)

Paths under `apps/android/app/src/main/` (`K` = `kotlin/example/hq/spike/`).

- `grep -rE 'Text\(\s*"' K` counts **172** inline literals (matches the board).
- `res/values/strings.xml` holds **6** strings (launcher shortcuts and widgets), no plurals.
- **14** `== 1) "` style plural branches in Kotlin.
- `K/ui/startup/Startup.kt:40` formats the last start with `ofPattern("HH:mm")`; `K/ui/tasks/TasksLogic.kt:144-156`
  uses `"EEE d MMM"`, `"MMM d"`, `"EEEE"`.
- `K/ui/settings/SettingsScreens.kt:1019-1020` inline "Follow every build of main" / "Newest builds,
  before they are promoted" / "Only builds the owner promoted".
- `K/ui/tasks/TasksScreen.kt:205` the strip appends `"${first.actor.ifBlank { first.harness }} on $where"`.
- `K/ui/tasks/TasksLogic.kt:124` `allMeta()` holds "hq is working".
- `K/ui/settings/SettingsWords.kt`: `sandbox` 175, `safetyRows` 206, `aiRows` 301, `appCard` 364,
  `budgetCards` 516, `forecast` 549, `scopeOf` 567 (never called for the title).
- `K/ui/usage/UsageWords.kt`: `statusWords` 105, `budgetChip` 150 (private).
- `K/ui/pairing/PairingWords.kt`: `INTRO` 10, `NOT_REMOVED` 17, `switchLine` 33, `relayLine` 42;
  `K/bridge/BridgeState.kt:76` `BRIDGE_STOPPED = "The bridge stopped. Try again."`.
- `K/ui/helpers/HelperWords.kt:369` `EDGE_AGENT = "HQ edge worker"`.
- `K/ui/firstrun/KeyCheck.kt:15` `Other("Other OpenAI-compatible", "CUSTOM_API_KEY", …)`.
- `K/ui/settings/SettingsScreens.kt:229` home row status `maxLines = 1`, `Ellipsis`.

### Before frames

- **B1 · Spending limits.** Budgets: "All / global, daily, then block / $2.10 of $5.00 (42%) / Within
  budget, resets in 9h"; "Openrouter / provider:openrouter, weekly, then notify / $9.50 of $10.00 (95%) /
  Past the alert threshold, resets in 3 days" (pins A, B). "Forecast, this month": "Spent so far $31.80",
  "Projected month end $74.00". "By AI service": "OpenRouter · $4.20 left · Today $0.31 · This week $1.20
  · This month $4.80".
- **B2 · Tasks, All.** "9 active · Live", chip "all 85%" (pin C). Tabs Today, Upcoming, All; filters
  Active, Stuck 1, Ready to check 1. Strip "1 helper working · hq on hq-oss" (pin D). "In progress 2":
  "T-4 · High · Workstream C: a native view instead of the WebView · hq is working · due Sunday" (pin
  E); "T-3 · Workstream A: first run for a phone-only user · 1 of 3 sub-tasks". [New task].
- **B3 · Backup and about.** "Backup is not set up / If the app is uninstalled or the phone is lost, your
  notes and tasks go with it. Backup today runs from the config file." (pin F). About: "AgentHQ · hub ·
  build 1", "Run mode · Active", "HQ · 0.9.1" (pin G). Updates: "Check for updates · A newer version
  needs Android 29", "Follow every build of main · Only builds the owner promoted · Off" (pin H).
- **B4 · Notifications.** "Allowed. / AgentHQ can post notifications." Channels: "Needs you · High ·
  sound on · heads-up", "Updates · Default · silent", "HQ is running · Low · required while HQ runs"
  (pin I). "Your HQ on a server · Signed in · https://hq.example.com · Disconnect" (pin J). Note:
  "Questions from coding agents on that HQ arrive here, checked once a minute. Disconnect stops that on
  this phone; to end the sign-in itself, sign this phone out in Settings, Devices on that HQ."
- **B5 · Pair a device.** "A device you pair can reach this phone directly, with no account and no VPN.
  Only devices you pair here can." (pin K). "Paired devices: On. Paired devices can reach this phone as
  3f9a1c2e." "Using n0's public relays, which are for development and testing." (pin L). Show a code,
  Scan a code, Paste a link. "Work laptop · a81c03fd · seen 3 min ago · Remove" then red "Could not remove
  it: the bridge is not running. Try again in a moment." (pin M).
- **B6 · Helpers, and the edge worker.** "Helpers · Live · Copilot 912 left", All, Working 2, Finished
  4. "Fix the share sheet title · Working on it · HQ edge worker · on this phone · 6 min · Working on
  T-10" (pin N). "Build the Android app · Working on it · Codex · on your Mac mini · 12 min · Updating
  the build files". [Start a helper].

Where the words come from (pins): **A** the covers line is `scope + ", " + period + ", then " + action`
from the config (`SettingsWords.budgetCards`); `scopeOf()` is never called. **B** the title is the
config's own name with a capital, and "alert threshold" is the config field. **C** the chip is `name + "
" + pct + "%"` (`UsageWords.budgetChip`). **D** the strip is the session's `actor` ("hq") and the last
part of its working folder (`TasksScreen.kt:205`, inline). **E** "hq is working" is a literal in
`TasksLogic.allMeta`. **F** "the config file" appears four times: the backup card, two read-only
notices, the model note. **G** About shows the build flavour "hub" and the run modes Active and
Standby, which Settings calls Background work elsewhere. **H** the update channel names a git branch
and a release step, inline; "Android 29" is an API level (Android 10). **I** channel lines are Android's
importance constants; only three of the five channels exist. **J** "coding agents"; a semicolon and a
25-word sentence. **K, L, M** pairing leaks VPN, relay, n0 (the library's company) and "the bridge"
(the child process); the failure line is red text and a fragment. **N** "HQ edge worker" is the daemon's
name for HQ coding on the phone.

Not drawn, same kind: "The service answered 401." on first run, "Base URL", "Other OpenAI-compatible",
"AI backends", key names like `OPENROUTER_API_KEY`, "3 turns", "Not running yet", raw status words
passed through when unknown.

## 3. Decision

**Translate at the edge, from codes never from text, into sentences that live in resources.**

- **What:** every value from config or the server is mapped to owner words in its `*Words.kt` function,
  by its code (`period == "day"`), with a plain fallback for codes the app does not know. The words
  themselves are string resources.
- **One glossary for the app, web and docs,** so "helper", "AI service" and "Background work" mean one
  thing everywhere.
- **Budgets are titled by what they cover,** not by the name in the config, because the owner never typed
  that name in the app.
- **The check reads the screen, not the source,** because a third of the text never passes a Words file.

Rejected:

- **A "Show technical names" switch.** Two vocabularies double the strings to test. People who want codes
  already have Copy details and Technical details.
- **Rename things in the config and server instead.** The config is a developer contract shared with the
  CLI and the web app. The phone app is the place that talks to the owner.
- **Ban plain words like "main", "session", "API".** They block honest sentences ("the main reason") and
  miss the real leaks, which are code shapes.

## 4. Glossary: the word HQ uses inside, and the word the owner reads

| Inside | Owner reads | Note |
|--------|-------------|------|
| hq, Hq, AgentHQ (in the app's text) | HQ | "AgentHQ" only where Android shows the app's name: launcher, Android settings, permission dialogs, notification header, and steps that point there ("Find AgentHQ in the list"). Never lower-case "hq". |
| last copy, cached, offline copy | Saved on this phone at {time} | "Saved on this phone at 14:32", in the phone's time format. Never "last copy". |
| Live, online, running | Connected | The healthy status word, with a grey dot. Never green. |
| Later, Dismiss, Skip | Not now | For anything that comes back. "Cancel" only in a sheet that edits something. |
| Go to settings, Settings, Permissions | Open Android settings | With one line naming the switch to tap: "Turn on Allow notifications." |
| 1/3, 1 of 3 sub-tasks, 33% | {N} of {M} done | "1 of 3 sub-tasks done". Downloads: "45% · 26 of 57 MB". Starting: "Step 3 of 4". |
| carries on, picks up, needs a look, for good | keeps going, starts again, Check this, completely | No idioms or phrasal verbs that a learner reads word by word. |
| HQ is not answering, HQ didn't reply | a rules section 2 line | "Couldn't {do it}." + the reason sentence + the outcome sentence (`../../resolutions.md` A2), one key per action and reason. |
| agent, coding agent | helper | Any AI HQ runs on a computer to do a job. |
| session, harness session | helper, or chat | A coding run is a helper; a conversation is a chat. The word is not banned in plain English, only as a name for these. |
| harness (claude-code, codex) | the helper's name | Claude, Codex, Copilot, via `HelperWords.agentName`. |
| edge worker | HQ | It is HQ itself working on this phone: "HQ · on this phone". While working "HQ is working on it", finished "Done by HQ", in the Helpers group "HQ's own work" (`../../resolutions.md` B4). |
| host, native, local, cwd, folder | where it runs | "your Mac mini", "this phone". A folder name is never the place. |
| provider, backend | AI service | OpenRouter, GitHub Copilot, Anthropic. Service names stay as the service writes them. |
| anthropic/claude-haiku-5.5 | claude-haiku-5.5 | A model keeps its own name. The service prefix before "/" is dropped. |
| scope global, provider:x, model:x | what it covers | "All AI spending", "OpenRouter only", "claude-haiku-5.5 only", "From Telegram". |
| period day, week, month | each day, each week | "Each day", "Each week", "Each month". On the chip "Today", "This week", "This month". |
| then block, then notify | what HQ does at the limit | "HQ stops using AI at the limit", "HQ tells you at the limit". |
| alert threshold | your warning point | "Past your warning point". |
| config file, config.yaml, settings file | (say what the app can do) | "The app cannot change these yet." Never send an owner to a file they cannot open on a phone. |
| run mode Active, Standby | Background work | "On", "Only when I open the app": the words of the Background work page. |
| main branch, build, promoted, channel | test versions | "Get test versions"; "versions marked ready" for the normal ones. "main" in plain English stays allowed. |
| API level 29, minSdk | Android 10 | 26 = 8, 28 = 9, 29 = 10, 30 = 11, 31 = 12, 32 = 12L, 33 = 13, 34 = 14, 35 = 15, 36 = 16. |
| importance High, Default, Low, Min | how it alerts you | "Sound and pop-up", "Sound", "Silent", "Silent, folded away". |
| relay, n0, iroh | go-between server | Only where the owner chooses it. The company behind the library is never named. |
| bridge | pairing | "Pairing is off" (A2), "Pairing stopped". |
| connector, claude.ai connector | Claude on the web | "connector" only in Technical details (B14). |
| chat apps | Messaging apps | Everywhere, the first-run step included (B15). |
| This PC (a computer) | the machine's own name, or "this computer" | "This PC" is only HQ Hub's tab name (B12). |
| See all {N} | Show all {N} | Search lists (B6). |
| Take a look | Open it to see. | (B18) |
| VPN | (drop it) | Saying what is not needed raises a question the owner never had. |
| OPENROUTER_API_KEY | the service: OpenRouter | Known names map to their service. An unknown name shows as typed. |
| HTTP 401, 404, 429, 5xx | what went wrong | "refused this key", "nothing answered there", "out of credit or too busy", "having trouble". |
| base URL, OpenAI-compatible | service address, another AI service | First run only, for the custom service. |
| turns | answers | "after 3 answers". |
| a raw status word | a plain fallback | An unknown code word is never capitalised and shown: "Other", "Not known yet". |
| bot token | bot token (kept) | Only in Telegram and Discord set-up, where BotFather and Discord use the same word. |
| left alone on purpose | exempt | Copy details, Technical details, the add-key form, commands to type (/pair, /newbot), Android's own setting names, service and model names, task numbers like T-10, repository names like `{owner}/{repo}`. |

## 5. Screens and states (After: the same screens, in the owner's words)

- **A1 · Spending limits.** "All AI spending / Each day · HQ stops using AI at the limit / $2.10 of $5.00
  (42%) / Within the limit · resets in 9 h" (blue bar); "OpenRouter only / Each week · HQ tells you at
  the limit / $9.50 of $10.00 (95%) / Past your warning point · resets in 3 days" (amber bar with icon;
  pin 1). Forecast: "Spent so far $31.80", "Likely total this month $74.00". By AI service unchanged.
- **A2 · Tasks, All.** "9 active · Connected" (grey dot), chip "Today 85%" (pin 2). Strip "1 helper
  working · Claude, on T-4" with a blue dot (pin 3). "T-4 · High · Workstream C: a native view instead of
  the WebView · HQ is working on it · due Sunday"; "T-3 · Workstream A: first run for a phone-only user ·
  1 of 3 sub-tasks done".
- **A3 · Backup and about.** Card "Backup is not set up / Your notes are only on this phone. If you lose
  it, you lose them. The app cannot set up backups yet." [Save a copy now]. About: "AgentHQ · Build 1",
  "Background work · On", "HQ version · 0.9.1" (pin 4). "App updates": "Check for updates · Up to date",
  "Get test versions · You get versions marked ready. · Off" (pin 5). Caption: "FixBackup owns the backup
  card and FixUpdates the App updates group. These are their words."
- **A4 · Notifications.** "Notifications are on / AgentHQ can post notifications." Channels: "Needs you ·
  Sound and pop-up", "Updates · Silent", "App updates · Silent", "Downloads · Silent · shows progress",
  "HQ is running · Silent, folded away · Android needs it while HQ runs" (pin 6). "Show details in
  notifications · Off: they say who and what kind". "Your server HQ · Connected ·
  https://hq.example.com · Disconnect" (ink, pin 7). Note "Questions from helpers on your server HQ come
  here. The phone checks once a minute."
- **A5 · Pair a device.** "A device you pair can reach this phone directly. You do not need an account.
  Only devices you pair here can." "Paired devices: On. Paired devices know this phone as 3f9a1c2e."
  "Uses a free public go-between server when needed. It is fine for a first try." (pin 8). "Work laptop ·
  a81c03fd · seen 3 min ago · Remove" (ink), then amber with icon "Couldn't remove “Work laptop”. Pairing
  is off. Switch it on and try again." (`../../resolutions.md` A2)
- **A6 · Helpers.** "Helpers · Connected · Copilot 912 left". Group "HQ's own work": "Fix the share
  sheet title · HQ is working on it · on this phone · 6 min · Working on T-10" (pin 9; once finished
  "Done by HQ", `../../resolutions.md` B4). Codex card unchanged.
- **A7 · Safety and limits.** Tabs Safety, Time limits, Helpers, Tools. "Read only. The app cannot change
  these yet." (pin 10). "Default computer · Mac mini", "Safe box for helpers · Fenced in, internet limited
  to a list", "Stop idle helpers after · 6 h", "Stop answering a helper that makes no progress after · 3
  answers" with "HQ stops answering its questions for you. The helper itself keeps running." (pin 11).
- **A8 · Keys on this phone, and first run.** "These are the names of the keys sealed on this phone.
  Their values are never shown." Rows "OpenRouter · Replace · Remove", "Brave Search · Replace · Remove"
  (pin 12). "Add or replace a key": label "Exact name HQ looks for", field `OPENROUTER_API_KEY`. "First
  run, when a key is refused": amber "Couldn't check this key. The service refused it. Check that you
  copied all of it." (pin 13).
- **A9 · 360 dp, font scale 2.0, Spending limits** (pin 14). A1's lines all wrap. Caption: "Every line
  wraps. Nothing is cut with "…". The state comes first in each line, so a wrap never hides it."

## 6. Exact copy: every string that changes

| Where | Before | After (the resource's text) | Shows on |
|-------|--------|-----------------------------|----------|
| `ui/settings/SettingsWords.kt` budgetCards covers | global, daily, then block | Each day · HQ stops using AI at the limit | Spending limits |
| budgetCards covers | provider:openrouter, weekly, then notify | Each week · HQ tells you at the limit | Spending limits |
| budgetCards title | All · Openrouter (the config name) | All AI spending · OpenRouter only (from scopeOf, the service's own spelling) | Spending limits |
| budgetCards stand | Within budget, resets in 9h | Within the limit · resets in 9 h | Spending limits |
| budgetCards stand | Past the alert threshold, resets in 3 days | Past your warning point · resets in 3 days (plural resource) | Spending limits |
| scopeOf | Provider openrouter · Model x · From origin | OpenRouter only · x only · From Telegram | Spending limits |
| forecast | Projected month end | Likely total this month | Spending limits |
| appCard line | Not running yet | Not connected yet | Messaging apps |
| appCard, sandbox (unknown) | the raw word, capitalised | Other | Messaging, Safety |
| safetyRows Helpers | … makes no progress after · 3 turns | … makes no progress after · 3 answers | Safety and limits |
| aiRows model line | anthropic/claude-haiku-5.5 · Working | claude-haiku-5.5 · Working | AI service and keys |
| home() Notifications | Needs you on · Updates on | Needs you on · Updates on · App updates on | Settings home |
| `ui/usage/UsageWords.kt` budgetChip label | all 85% | Today 85% · This week 95% · OpenRouter 95% | Top bar, every tab |
| budgetChip title | Budget all: $4.25 of $5.00 daily | Spending limit: $4.25 of $5.00 today, 85% used | TalkBack |
| statusWords ledger_only | No balance from the provider | The service does not report a balance | AI service and keys |
| `SettingsScreens.kt` UpdateGroup title, line 1019 | Follow every build of main | Get test versions | Backup and about |
| UpdateGroup line, off, 1020 | Only builds the owner promoted | You get versions marked ready. | Backup and about |
| UpdateGroup line, on, 1020 | Newest builds, before they are promoted | New versions before they are marked ready. Some may have problems. You can go back only when a regular version is newer. | Backup and about |
| About facts | AgentHQ · hub · build 1 | AgentHQ · Build 1 | Backup and about |
| About facts | Run mode · Active / Standby | Background work · On / Only when I open the app | Backup and about |
| About facts | HQ · 0.9.1 | HQ version · 0.9.1 | Backup and about |
| Group label | Updates | App updates | Backup and about |
| ReadOnly notices (2) | Read only. You can change these in the config file on this phone. | Read only. The app cannot change these yet. | Safety and limits |
| Keys page note | The model is set in the config file on this phone. | The app cannot change which AI HQ uses yet. | AI service and keys |
| ServerHqSection note | Questions from coding agents on that HQ arrive here, checked once a minute. Disconnect stops that on this phone; to end the sign-in itself, sign this phone out in Settings, Devices on that HQ. | Questions from helpers on your server HQ come here. The phone checks once a minute. Disconnect stops that on this phone. To sign this phone out completely, use Other devices on that HQ. | Server HQ |
| `BackupFacts.kt` OFF_BODY | … Backup today runs from the config file. | Your notes are only on this phone. If you lose it, you lose them. The app cannot set up backups yet. (FixBackup owns the states.) | Backup and about |
| `ui/settings/PhoneWords.kt` channelLine Needs you | High · sound on · heads-up | Sound and pop-up | Notifications |
| channelLine Updates | Default · silent | Silent | Notifications |
| channelLine App updates (new) | (no channel) | Silent | Notifications |
| channelLine Downloads (new) | (no channel) | Silent · shows progress | Notifications |
| channelLine HQ is running | Low · required while HQ runs | Silent, folded away · Android needs it while HQ runs | Notifications |
| channelLine off | (none) | Off in Android settings | Notifications |
| `ui/tasks` WorkingStrip, line 205 | 1 helper working · hq on hq-oss | 1 helper working · Claude, on T-4 (plural resource) | Tasks, All |
| WorkingStrip, two or more | 2 helpers working · hq on hq-oss | 2 helpers working · Claude on T-4, and 1 more | Tasks, All |
| allMeta | hq is working · for hq | HQ is working on it · for HQ | Tasks rows |
| allMeta sub-tasks | 1 of 3 sub-tasks | 1 of 3 sub-tasks done (as FixProgress) | Tasks rows |
| dates, `TasksLogic.kt:144-156` | "EEE d MMM", "MMM d" patterns | The phone's own medium date format (`DateTimeFormatter.ofLocalizedDate`) | Tasks |
| time, `Startup.kt:40` | "HH:mm" always | `DateFormat.getTimeFormat(context)`: 2:32 PM or 14:32 as the phone is set | Startup, Saved at |
| QuickAdd pill | For the edge worker · Not for the worker | Let HQ code it · Not for HQ | New task sheet |
| QuickAdd caption | Edge worker | HQ on this phone | New task sheet |
| QuickAdd sentence | HQ works it on this phone within a few minutes and opens a pull request in `<owner>/<repo>`. | HQ starts work on it here in a few minutes. It sends the change to `<owner>/<repo>` on GitHub for you to check. | New task sheet |
| quick add result | Task added for the edge worker | Task added. HQ starts on it in a few minutes. | Snackbar |
| quick add result | Task added, but not for the worker: x is no longer allowed | Task added, but HQ will not code it. x is no longer on its list. | Snackbar |
| `ui/helpers` EDGE_AGENT | HQ edge worker · on this phone | HQ · on this phone, in the group "HQ's own work"; state "HQ is working on it", then "Done by HQ" (B4) | Helpers cards |
| `SettingsWords.kt:230` safety fact row | New helpers: HQ answers their questions | Let HQ say yes to its requests (added by resolutions.md C-6) | Safety and limits |
| Pair a device, the code's button | Cancel | Stop pairing (B18) | Pair a device |
| Paired device removed | Device removed. | {Work laptop} removed. (B18) | Pair a device |
| StopDialog | … and its terminal on your Mac mini closes … | "Stop Codex?" / "It stops now. Its work so far is kept." (#10) | Stop dialog |
| `ui/pairing/PairingWords.kt` INTRO | … with no account and no VPN. Only devices you pair here can. | A device you pair can reach this phone directly. You do not need an account. Only devices you pair here can. | Pair a device |
| switchLine | On. Paired devices can reach this phone as 3f9a1c2e. | On. Paired devices know this phone as 3f9a1c2e. | Pair a device |
| relayLine | Using n0's public relays, which are for development and testing. · Using your relay. · No relay: … | Uses a free public go-between server when needed. It is fine for a first try. · Uses your own go-between server. · Devices must reach each other directly. | Pair a device |
| NOT_REMOVED | Could not remove it: the bridge is not running. Try again in a moment. | Couldn't remove “Work laptop”. Pairing is off. Switch it on and try again. (A2) | Pair a device |
| `bridge/BridgeState.kt` BRIDGE_STOPPED | The bridge stopped. Try again. | Pairing stopped. Try again. | Pairing screens |
| `ui/firstrun` checkFailed | The service answered 401. | One key per reason. 401, 403: "Couldn't check this key. The service refused it. Check that you copied all of it." 404: "… Nothing answered at that address." 429: "… The service is busy, or the account is out of credit." 5xx: "… The service is having trouble. Try again soon." Else: "… The service did not accept it." | First run |
| BASE_URL_LABEL, ERR_URL_* | Base URL · Enter the base URL. | Service address · Enter the service address. | First run |
| Provider Other | Other OpenAI-compatible | Another AI service | First run |
| ERR_CONFIG_HAS_BACKENDS | Your settings file already lists AI backends. Add this service there. | HQ already has its AI services set. The app cannot add this one. | First run |
| StoredKeysPage rows | OPENROUTER_API_KEY | OpenRouter (known names); an unknown name as typed | Keys on this phone |
| KEYS_NAME_LABEL | Name | Exact name HQ looks for | Keys on this phone |
| `update/UpdateLogic.kt` NeedsNewerAndroid | A newer version needs Android 29 | A newer version needs Android 10 or newer. | Backup and about |

Other strings drawn in the After frames: "Notifications are on" (A4 header, was "Allowed."), "Show
details in notifications · Off: they say who and what kind" (A4), "Your server HQ" group (A4), "Safe box
for helpers · Fenced in, internet limited to a list", "Stop idle helpers after · 6 h", "HQ stops answering
its questions for you. The helper itself keeps running." (A7). On Android 13+ the permission line: "Notifications
are off for AgentHQ. Open Android settings and turn on Allow notifications."

### Strings live in resources (M12, rules section 7)

- **Where:** `res/values/strings.xml` and `plurals.xml`, built in wave 0 (`../../resolutions.md` D-8:
  the move to resources, the catalogue and the lint land in wave 0; this board's rewritten words land in
  wave 1). Words files keep the logic
  (which sentence, which reason) and return a resource id with its arguments. They stay pure: tests pass
  a fake lookup.
- **Keys are whole sentences,** one per action and reason, named `area_action_reason`. Translators never
  join fragments. No "Couldn't {verb}." templates.
- **Counts** use plural resources, never `if (n == 1)`. 14 such branches today.
- **Dates and times** use the phone's format and its 12 or 24-hour setting (`DateFormat.getTimeFormat`,
  `ofLocalizedDate`). No fixed patterns in owner text.
- **Literals in composables** are refused by a lint (a custom Android Lint detector on `Text`, button
  labels and `contentDescription` outside previews and tests). The 172 found today by
  `grep -E 'Text\(\s*"'` go into a baseline that only shrinks.

Examples from the board:

```xml
<string name="fail_send_message_starting">Couldn't send your message. HQ is still starting. It will try again by itself.</string>
<string name="saved_on_phone_at">Saved on this phone at %1$s</string>
<plurals name="helpers_working">
  <item quantity="one">%d helper working</item>
  <item quantity="other">%d helpers working</item>
</plurals>
<!-- The quantity is the total {M}, not the count done: getQuantityString(R.plurals.subtasks_done, total, done, total) -->
<plurals name="subtasks_done">
  <item quantity="one">%1$d of %2$d sub-task done</item>
  <item quantity="other">%1$d of %2$d sub-tasks done</item>
</plurals>
```

The plural is keyed on the total `{M}` (`../../resolutions.md` D-7): 3 sub-tasks give "1 of 3 sub-tasks
done" and "0 of 3 sub-tasks done"; only a task with one sub-task reads "1 of 1 sub-task done".

### Plain for English as a second language (rules section 7, M13)

- **15 words a sentence at most.** Counted between ".", "?" and "!", in the rendered text.
- **No semicolons** in owner text. Two sentences instead.
- **No idioms or tricky phrasal verbs:** "carries on", "picks up", "needs a look", "for good", "hang on",
  "on the fly", "heads-up". Replace with "keeps going", "starts again", "Check this", "completely",
  "Wait".
- **One word per thing:** the glossary above, for the app, the web and the docs.
- **Status first:** the state word leads its line ("Overdue · last backup 3 d ago"), so a wrap at large
  text never hides it.
- **Tested with people:** five owners who read English as a second language read A1 to A8 and say back
  what each line means. A line two of them get wrong is rewritten.

## 7. Behaviour rules (numbers match the After pins)

1. **Budget title** from `scopeOf()` ("All AI spending" for global). The covers line is
   "Each day|week|month" plus "HQ stops using AI at the limit" (block) or "HQ tells you at the limit"
   (notify). An unknown action is left out. A bar is blue while within the limit, amber past the warning
   point.
2. **Chip:** global budgets read "Today", "This week", "This month"; a scoped one reads its service or
   model; then the percent. At most 12 characters before the percent, else "Limit 85%". Its spoken label
   is the full sentence. Over a limit the chip is amber with the same words: "Today 104%"
   (`../../resolutions.md` B5).
3. **Strip:** the first helper's `agentName(harness)` and its task id, never the actor or the folder.
   Blue dot while working.
4. **About** uses the Background work page's words. The build flavour is dropped.
5. **Test versions row:** "Get test versions" with On or Off on the right. The line under it is the one
   for the current value.
6. **Channels:** the five rules section 1 names, in that order. The line comes from importance and sound
   together: High with sound "Sound and pop-up"; Default with sound "Sound"; no sound or Low "Silent";
   Min "Silent, folded away"; turned off "Off in Android settings".
7. **Server HQ:** the group is "Your server HQ", the state "Connected". Disconnect is ink text and opens
   the #10 dialog.
8. **Go-between line:** says in plain words who may carry the connection. The library's company is never
   named.
9. **Edge runs** are "HQ". The place stays "on this phone".
10. **Read-only notices** say what the app cannot do, never where a file is.
11. **Counts of turns** are "answers".
12. **Key rows** map known names (the `KeyCheck.Provider` names, `GOOGLE_API_KEY`, `BRAVE_API_KEY`, the
    bot tokens) to the service. The add form keeps the exact name because HQ needs it.
13. **HTTP codes** become a rules section 2 line, amber with an icon. The number goes to Technical
    details only.
14. **Large text:** no owner line has `maxLines = 1`. Lines wrap. The state leads.
15. **"Let HQ say yes to its requests" (added by resolutions.md C-6).** The Safety and limits fact that
    reads `drive_new_watches` is named "Let HQ say yes to its requests", with the web's rules: off by
    default, never a losing request, and every answer HQ gives is listed (under Watching on the web, on
    the helper's page here). The phone shows it read only.
16. **Times and zones (added by resolutions.md C-8).** HQ sends instants. The phone shows every time in
    its own zone and 12 or 24-hour setting; where HQ's own zone matters and differs, it is named
    ("Daily summary at 07:00 London time"). Spending limits say when the day resets ("Resets at midnight
    UTC, 03:00 here" on a phone in Nairobi), as the web does (`../../web/wave-2/04-time-zones.md`).

- **Unknown codes** never pass through: `replaceFirstChar(Char::uppercase)` on a server value is replaced
  by "Other" in `sandbox()`, `appCard()`, `BulkEdit` and `statusWords()`.
- **Exempt from the word check:** nodes inside Technical details, Copy details and the add-key form
  (tagged `technical`), commands to type, service and model names, task numbers, and repository names
  that match `{owner}/{repo}`.

## 8. Android versions

| Version | What differs for this fix |
|---------|---------------------------|
| 8 and 9 (API 26 to 28) | Channels exist from API 26 (the minimum), so all five channel lines show on every version. Plural resources and `DateFormat.is24HourFormat` work the same. |
| 10 and 11 | Nothing new for words. "Open Android settings" opens the channel's own page (`ACTION_CHANNEL_NOTIFICATION_SETTINGS`, API 26). |
| 12 | Nothing new for words. Notification titles follow rules section 1 on every version. |
| 13+ | The notification permission line reads "Notifications are off for AgentHQ. Open Android settings and turn on Allow notifications." From 13 Android offers a language per app (needs `locales_config.xml`). Below 13 the app follows the phone's language. From 14 the font scale goes to 2.0. |

## 9. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `res/values/strings.xml` (6 strings today), `res/values/plurals.xml` | Every owner sentence, with whole-sentence keys. | strings yes, plurals NEW |
| `K/ui/settings/SettingsWords.kt` | `budgetCards`, `scopeOf`, `forecast`, `appCard`, `sandbox`, `safetyRows`, `aiRows`; new `periodWords`, `actionWords`. Return resource ids. | yes |
| `K/ui/usage/UsageWords.kt` | `budgetChip` label and title, `statusWords`. | yes |
| `K/ui/settings/SettingsScreens.kt` | The inline update strings (lines 1019-1020), About facts, the two read-only notices, the model note, `ServerHqSection`, home row `maxLines = 1` (line 229). | yes |
| `K/ui/settings/BackupFacts.kt` | `OFF_BODY` (owned by `02-backup.md`). | yes |
| `K/ui/settings/PhoneWords.kt` | `channelLine`, `importanceWords`, five channels. | yes |
| `K/ui/tasks/TasksScreen.kt` | `WorkingStrip` (line 205, inline today). | yes |
| `K/ui/tasks/TasksLogic.kt` | `allMeta` (124) and its dates (lines 144-156). | yes |
| `K/ui/tasks/TaskSheets.kt`, `TasksViewModel.kt` | Edge pills and sentence; quick add results. | yes |
| `K/ui/startup/Startup.kt:40` | Time format. | yes |
| `K/ui/helpers/HelperWords.kt` | `EDGE_AGENT` (369). | yes |
| `K/ui/helpers/HelperPages.kt` | `StopDialog` (with `03-destructive.md`). | yes |
| `K/ui/pairing/PairingWords.kt` | `INTRO`, `switchLine`, `relayLine`, `NOT_REMOVED`. | yes |
| `K/bridge/BridgeState.kt` | `BRIDGE_STOPPED` (76). | yes |
| `K/ui/firstrun/FirstRunWords.kt`, `KeyCheck.kt` | `Provider.Other` label (line 15); `StoredKeysPage` rows through a new `KeyNames.display(name)`. | yes (`KeyNames` NEW) |
| `app/src/test/.../WordCheckTest.kt` | Renders every fixture, reads the semantics tree. Runs on the Compose UI test setup wave 0 adds (`../../resolutions.md` D-11). | NEW |
| `lint/TextLiteralDetector.kt` | Custom Android Lint detector with a baseline. | NEW (in the lint-checks module wave 0 adds, D-11) |
| `K/ui/settings/SettingsWords.kt` (line 230) | The fact row name "Let HQ say yes to its requests" (C-6). | yes |
| `K/ui/settings/SettingsWords.kt` budget cards, `K/ui/tasks/TasksLogic.kt` | Times in the phone's zone with HQ's zone named when it differs; the reset line (C-8). | yes |

## 10. Server, release or desktop changes

None required. Open question 3 (one catalogue for phone and web) is decided by N19 and needs a generator
step that builds `strings.xml` and the web's strings from one file; see `../../rules-shared.md` and the
web half.

## 11. Acceptance checks (each fails on today's build)

1. From the `s-spend` fixture, Spending shows "All AI spending · Each day · HQ stops using AI at the
   limit". Today it shows "global, daily, then block".
2. With the "all" budget at 85%, the chip reads "Today 85%". Today "all 85%".
3. The Tasks strip from the fixture session reads "1 helper working · Claude, on T-4". Today "hq on
   hq-oss".
4. **Word check:** a Compose test renders every fixture set (Settings, Inbox, Helpers, Chat, Tasks, First
   run, Pairing) and reads every `Text` and `EditableText` in the semantics tree, skipping nodes tagged
   `technical`. It fails on lower-case "hq" as a word, "edge worker", "coding agent", "harness",
   "backend", "provider:", "model:", "config", "yaml", "VPN", "n0", "relay", "bridge", an ALL_CAPS name
   with "_", a file path, an HTTP status number, "Live" as a status, "Later" on a button, a rules
   retired failure line, a semicolon, or a sentence over 15 words. Today it fails on all the Before
   frames.
5. The lint fails a new `Text("…")` literal in main code. Today 172 exist and nothing stops a new one.
6. No Words file or composable contains `== 1) "`. Counts 0, 1, 2 and 5 run through every plural
   resource. Today 14 such branches exist.
7. With the phone set to 12-hour, Startup and "Saved on this phone at" read "2:32 PM". Today
   `Startup.kt:40` prints "14:32".
8. An unknown status, sandbox or action code shows "Other", never itself. Today the raw word shows,
   capitalised.
9. The Notifications page lists Needs you, Updates, App updates, Downloads and HQ is running. Today it
   lists three.
10. At 360 dp and font scale 2.0, no owner line ends in "…". Today home row status is `maxLines = 1` with
    an ellipsis (`SettingsScreens.kt:229`).
11. Five owners who read English as a second language say back A1 to A8 correctly. Today's lines are the
    control.
12. Safety and limits reads "Let HQ say yes to its requests", never "New helpers: HQ answers their
    questions" (added by resolutions.md C-6).
13. With the phone in Africa/Nairobi and HQ in Europe/London, a task due at an instant shows Nairobi
    time, the daily summary names "London time", and Spending reads "Resets at midnight UTC, 03:00
    here" when HQ's day is UTC (added by resolutions.md C-8).
14. `plurals.subtasks_done` with 1 done of 3 renders "1 of 3 sub-tasks done" (D-7).

## 12. Check on device

- The 8.1 emulator and an Android Go phone, for line length at their default font.
- On the Galaxy A32: One UI's large sizes (before Android 14's 2.0 scale). FixPlan wave 1: "large text
  on the A32".
- The five-owner reading test (FixPlan "Still to prove" for #11).

## 13. Open questions and decisions it depends on

Open questions for the owner, with the board's recommendation:

1. **"Get test versions" or a plainer "Try new versions early"?** Recommendation: "Get test versions". It
   says what you get, and "test" warns without a paragraph. The updates board uses whichever is chosen.
   (Decided: D4, "Get test versions", with the can-go-back line.)
2. **Should the go-between line show at all** on the owner's phone, since only a config file changes it?
   Recommendation: keep it, in the plain words drawn in A5, so an owner knows a free public server may
   carry the connection. (Decided: D5, "Yes, in plain words.")
3. **Android resources, or one catalogue shared with the web?** M12 allows either. Recommendation: one
   catalogue file in the repo that generates `strings.xml` and the web's strings, decided before the
   first translation. Until then, Android resources. (Decided: N19, "One catalogue that builds both phone
   and web.")
4. **Lint: fail the build or warn?** Recommendation: fail on new literals, with today's 172 in a baseline
   that must reach zero by the end of wave 0. (Decided: N20.)

How to keep it this way: new owner text goes in a resource, chosen by a Words function, never inline in a
composable; the lint enforces it. Each new fixture runs through the word check by being added to the
fixture list. Server fields added later are mapped by code with a fallback on the day they are first
shown. Reviewers check new words against the glossary; a new term gets a glossary entry first.

Depends on: D4, D5, N19, N20, `../../resolutions.md` A2, B4, B6, B12, B14, B15, B18, C-6, C-8, D-1, D-7,
D-8, D-11; rules sections 1, 2, 7; `05-progress.md` (counts), `02-backup.md` (backup
card), `../wave-2/02-updates.md` (App updates group).
