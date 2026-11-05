# Web wave 4 · 09 · Spending limits (W2, W41, M11, M12)

Boards: **AlignedSpending** (desktop from the balance chip, Add a limit, phone width in every state),
**AlignedSpending2** (tablet, 320, 200%, 2200, the phone's Usage page embedded, the HQ window, Lite),
**AlignedShell** section 4 (the balance chip), AlignedReview M2, M5, M11, M12, Drift 02. Follows
`../../rules-web.md` sections 2, 8 and 9 and `../../rules-shared.md` sections 3 and 4 (spending money:
not green). Where this spec and the rules differ, the rules win.

## 1. Finding and the review items it closes

- **W2.** Usage folds into Settings as the phone's Spending limits page, under How HQ behaves, at
  `/settings/spending`; the balance chip opens it from anywhere.
- **M2.** Over the limit and past the warning point are amber with an icon; within the limit a grey bar with
  ink words; no green or red status, no inline hex colours; the chart in the rules-web 8 palette.
- **M5.** Spending under How HQ behaves; Undo on deleting a limit restores on HQ, with "Couldn't undo. It
  changed on another device, or too much time passed." after the window (`../../resolutions.md` A6).
- **M11.** The reset zone is named: midnight UTC, 03:00 in Nairobi, weeks from Monday.
- **M12.** Rate-limit readings and "Key limit left" kept from `usage.tsx`, with HQ's own record beside what
  the service says.
- **M1.** Connected, the chip only when Connected, "Your HQ can't be reached" with the saved strip, no
  "Checking" chip. **FixRules 3:** Raise the limit is not green, since it spends money.
- **W41.** Spending limits gain a "One model" scope. **Drift 02.** Usage cannot be reached from a phone.

## 2. Before (as built, verified at `e5f9d5f`)

- `apps/hq-web/src/routes/usage.tsx` its own rail place (`AppRail.tsx:45`), not in the phone bar
  (`BottomNav.tsx`); reached on a phone only through the header chip, hidden under 400 px.
- `usage.tsx:85` the form `{name, scope: 'global', period: 'month', limit, action: 'block', downgrade}`;
  `:132` the scope field with the placeholder "global, provider:x, model:x, origin:x"; `:144` actions
  "block", "downgrade", "notify"; `:105` `soft_pct: [80]` fixed; `:186-190` the forecast ("No spend
  recorded yet, so there is nothing to forecast.", "Burn rate now … per hour ({confidence} confidence)");
  `:226` "Key limit left"; `:228-231` rate limits ("{n} of {m} requests").
- `apps/hq-web/src/routes/__root.tsx:113-152` `CopilotChip` (budget, Copilot or OpenRouter label).
- Server: `crates/hq-web/src/budgets_api.rs:21, 42` get and put the whole list;
  `crates/hq-core/src/config/budgets.rs` scopes global, provider, model, origin and `soft_pct`;
  `crates/hq-db/src/usage_ledger.rs:153` `period_bounds()` in UTC (day, week from Monday, month);
  `usage_forecast_api.rs`, `usage_providers_api.rs`, `openrouter_usage_api.rs`, `copilot_usage_api.rs`.

## 3. Decision

**The old Usage page becomes Spending limits under How HQ behaves, with the phone's name and words:
limits first, then the forecast, what used the most and each AI service. One sheet for Add and Edit, in
choices instead of config fields.**

Rejected: a scope typed as `provider:x`; green for within a limit; a red over-limit chip.

## 4. Screens and states

### Desktop, opened from the balance chip (Spending 1)

- **P1 · 1440.** Chip "Today 85%" (amber: past a warning point). Settings list pane on the left with
  "Spending limits · $4.25 of $5.00 today" selected; the detail pane: [Add a limit], "Limits 3", the line
  "Limits reset at midnight UTC, which is 03:00 in Nairobi. Weeks start on Monday."
  - "All AI spending · Each day · $4.25 of $5.00 · 85%" on an amber bar, "Past your warning point · resets
    in 12 h" (amber with an icon), "At the limit: HQ stops using AI until it resets."
  - "OpenRouter only · Each week · $6.10 of $10.00 · 61%" grey bar, "Within the limit · resets Monday, in 4
    days", "At the limit: HQ tells you."
  - "From scheduled tasks · Each month · $12.40 of $20.00 · 62%", "Within the limit · resets 1 Dec, in 26
    days", "At the limit: HQ switches to claude-haiku-5.5."
  - Each limit's menu: Edit (the same sheet as Add), Delete (acts at once with Undo, which puts it back on
    HQ).
- **P2 · Forecast, this month.** "Spent so far · $31.80", "Spending now · $0.42 an hour", "Likely total this
  month · $74.00", "Resets in · 26 days". "At this pace "From scheduled tasks" reaches its limit on 27 Nov,
  before it resets." "Not much history yet, so this is a rough guess. Weekends usually cost less."
- **P3 · What used the most, last 7 days** (a chart in the data palette, labelled directly). "By model":
  "claude-haiku-5.5 · 412 answers · $8.20", "deepseek-flash · 96 answers · $1.10", "qwen-coder · 40 answers
  · $0.30", "gpt-6-luna · on your plan · $0.00". "By kind of work": Chat "188 answers · $5.40", Helpers "61
  answers · $3.10", Telegram "74 answers · $1.20", Scheduled tasks "42 answers · $0.90", Task checks "19
  answers · $0.40".
- **P4 · By AI service.** "GitHub Copilot · Credits on your plan · 912 of 1.5k credits left · Resets 1 Dec.
  At this pace they last until then." "OpenRouter · The service says · $4.20 left"; "Key limit left · The cap
  set on this key at OpenRouter. · $15.80"; "Today $0.31 · This week $1.20 · This month $4.80"; "HQ's own
  record · Today $0.30 · This week $1.18 · This month $4.75"; "Rate limit, from its last answer · 182 of 200
  requests · 38k of 40k tokens left". "Anthropic · Key refused · The service refused the key, so HQ skips
  Anthropic. HQ's own record: $0.00 this month." [Check the key in AI service and keys]. "DeepSeek · $0.42
  today · The service does not report a balance. This is what HQ counted: this week $1.10, this month
  $2.30." "Your own server · llm.example.com · Runs on your own server. Nothing to pay per answer." "3
  answers this month had no known price. These totals may be a little low."

### Add a limit (Spending 2)

- **A1.** One sheet for Add and Edit (side from 768). "Name". "What it covers" as four cards (one per kind of
  limit HQ has, `budgets.rs`: global, provider, model, origin): "Every answer, from anywhere." · "One AI
  service · OpenRouter, Anthropic or another." · "One model · claude-haiku-5.5 or another." · "One kind of
  work" (Chat, Helpers, Telegram, Scheduled tasks, Task checks). Choosing one adds a picker under it. "How
  often it resets" (Each day, Each week, Each month) with "Resets at midnight UTC, 03:00 in Nairobi. Weeks
  start on Monday." "Limit in dollars". "Warn me at" (new in the form; HQ already takes `soft_pct`; the web
  sends 80 today). "At the limit" as three cards: "Stop using AI until it resets · Chats and helpers in this
  limit wait." · "Switch to a cheaper model · You choose which." · "Just tell me · An item in the Inbox.
  Nothing stops." "It applies to every answer straight away." [Cancel] [Save limit].
- **A2 · Checked as you go.** Save stays off until there is a name and a limit above zero. A wrong field
  says why under it, amber with an icon. A name already in use asks to replace that limit.

### Phone width, every state (Spending 3)

- **S1 · Filled.** Limits first, then the forecast, what used the most and each service. The chip is in the
  top bar on every place while Connected, so a phone owner reaches Spending in one tap (Drift 02).
- **S2 · Over a limit.** Chip "Today 104%" amber (`../../resolutions.md` B5). "HQ stopped using AI" / "All AI spending reached $5.20 of $5.00 today. Chats and helpers wait
  until it resets at 03:00, or until you raise the limit." [Raise the limit] (outlined, not green: it spends
  money). The limit "$5.20 of $5.00 · 104%", "Over the limit · resets at 03:00, in 12 h".
- **S3 · Deleted, with Undo.** ""OpenRouter only" deleted · Undo". Undo puts the limit back on HQ within 10
  minutes (needs a restore route).
- **S4 · Too late.** "Couldn't undo. It changed on another device, or too much time passed."
- **S5 · No limits.** "No spending limits" / "No limits are set, so nothing is limited. Add a limit to cap a
  day, a week or a month." [Add a limit]. The chip then shows the first service's balance.
- **S6 · Can't be reached.** The status line says so and the chip hides. "Your HQ can't be reached. Saved in
  this browser at 14:32." Saved values under it; limit menus off: "Changing a limit needs HQ. Your HQ can't
  be reached."
- **S7 · Loading.** "Asking HQ for your limits and spending" and grey shapes. No chip until HQ answers,
  never "Checking".

### Widths, the phone's page, the HQ window, Lite (Spending 2 board)

- **900:** below 1024 the page takes the one pane; Add a limit and Refresh in its header.
- **320:** the chip moves into the More menu with Search and Settings; the two-column cards stack; each fact
  wraps its value under its title.
- **200%:** the phone layout and bottom bar; the Undo toast above the bottom bar.
- **2200:** the page keeps its reading width, centred; the service cards stay in two columns.
- **Embedded (the phone app's Usage page, `?embed=1`):** no rail, no top bar, no bottom bar; the phone draws
  its own header ("Usage · Web page · spend and forecast").
- **The HQ window, showing HQ on your phone (1100):** "These are the limits of HQ on your phone." "$1.10 of
  $3.00 today · of $3.00 · 37%", "$9.80 left", "Today $0.22 · This week $0.90 · This month $3.10". The chip
  is the phone's. The phone's HQ resets at midnight UTC too.
- **Lite:** no Spending limits. Lite does not serve usage or budget routes (`docs/HQ_LITE.md`): no balance
  chip, no Spending row, and `/settings/spending` opens This browser with "This HQ is HQ Lite. It has no
  Spending limits." (`../../decisions.md` WR28, which closes OD-W2). Recommended: no Spending at all
  in Lite, rather than an empty page, since Lite makes no AI calls of its own.

## 5. Exact copy

Every string quoted in section 4 is exact. Keys `spend_*`. The chip: "Copilot 912 left" (normal, line
border, ink); "Today 85%" (past a warning point, amber); "Today 104%" (over the limit, amber; B5); left out when not
known or not Connected. Retired: "Usage", "OpenRouter spend", "Burn rate now", "global, provider:x,
model:x, origin:x", "block", "downgrade", "notify", "Checking", "Over the limit 104%" on the chip, green
within-limit bars, red over-limit.

## 6. Behaviour rules

1. **Address** `/settings/spending`; `/usage` redirects; `hq://web/usage` opens it with `?embed=1`.
2. **The chip** shows only while Connected and HQ knows a balance or a limit: a limit past its warning point
   takes it, in amber, with its words; over a limit stays amber; with no limit it shows the first service's
   balance. It opens Spending limits. Without a chip, Spending limits is still a row in Settings.
3. **Colour:** within the limit a grey bar and ink words; past the warning point and over, amber with an
   icon; money is ink; never green or red. The chart in blue, sky, white, grey, indigo.
4. **The reset zone** is always named (UTC and the browser's time), weeks from Monday (`period_bounds`).
5. **Scopes** map: Every answer › `global`; One AI service › `provider:{id}`; One model › `model:{id}`
   (W41); One kind of work › `origin:{chat|helpers|telegram|scheduled|checks}`.
6. **Warn me at** writes `soft_pct` (default 80).
7. **Delete** acts at once with Undo on HQ (10 minutes); Edit opens the same sheet.
8. **Raise the limit** is outlined: it spends money (D8, rules-shared 3).
9. **HQ's own record beside the service's** for every service that reports one; unpriced answers counted.
10. **Refusals** link to the key page; no status codes.

## 7. Browser and platform notes

- Money formatting through `Intl.NumberFormat` with the currency (USD) and the browser's language.

## 8. Code touchpoints

| File | Change | Exists |
|------|--------|--------|
| `apps/hq-web/src/routes/settings/spending.tsx` | The page (from `routes/usage.tsx`). | NEW |
| `apps/hq-web/src/routes/usage.tsx` | Redirect (85-231 move). | yes |
| `apps/hq-web/src/components/settings/LimitSheet.tsx` | Add and Edit. | NEW |
| `apps/hq-web/src/routes/__root.tsx` | `CopilotChip` rules (113-152). | yes |
| `apps/hq-web/src/lib/usageApi.ts`, `copilotUsageApi.ts`, `openrouterUsageApi.ts` | Data; delete and restore. | yes |
| `crates/hq-web/src/budgets_api.rs`, `crates/hq-core/src/config/budgets.rs` | A restore route with the window; deleted limits kept with `deleted_at`. | yes |

## 9. Server, desktop or release changes

Server: the limit restore route (`../../changes.md`, "Restore routes for every Undo"). None else: the usage
routes exist.

## 10. Acceptance checks (each fails on today's build)

1. `/usage` redirects to `/settings/spending`; the rail has no Usage. Today Usage is a place.
2. The scope is chosen from four cards; no text field accepts `provider:x`. Today the field does.
3. Within-limit bars are grey, past the warning amber; no element is green or red. Today they are coloured.
4. The page names "midnight UTC" and the local time. Today it does not.
5. Deleting a limit shows ""OpenRouter only" deleted · Undo" and Undo restores it. Today it is gone.
6. "Warn me at" writes `soft_pct`. Today it is fixed at 80.
7. Over a limit the chip reads "Today 104%" in amber, past the warning point "Today 85%" (B5).

## 11. Check on a laptop or phone

- On a phone, tap the chip from any place: Spending limits opens.
- Push a limit over 100% in a test HQ: the page leads with "HQ stopped using AI".

## 12. Open decisions it depends on

- **W2, W41, WR2, WR28** (Recommended), `../../resolutions.md` A6, B5.
- Depends on: `../wave-1/06-settings-groups.md`, `../wave-1/03-undo.md`, `../wave-2/04-time-zones.md`,
  `../wave-2/02-embed-and-lite.md`.
