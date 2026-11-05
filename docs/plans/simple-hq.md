# Simple HQ: an assistant that shows what it can do and learns you

Status: phases 0 and 1 built (see "Where the build differs" under Phases); phases 2 to 5 are
proposals. Replaces the first draft, "Proactive HQ" (commit `8a82d5d`), after the adversarial
review below.

## The goal, restated

The first draft asked how HQ could notice things for the owner unprompted. That was the wrong
question. Proactivity is nice to have, many people do not want it, and the version the industry is
chasing (KAIROS, OpenDuet's check-ins, the "Jarvis" picture) is the same rabbit hole HQ already went
down with the dream cycle and the touchpoint generators.

The goal is **simplification for the person at the bottom of the pyramid**: someone handed HQ on a
phone who will never read a capability list, never write a rule, and never tune a setting. For that
person HQ has to do two things on its own:

1. **Bring out its own capabilities**, at the moment they would help, in the person's words, and
   only the ones that work on this install.
2. **Learn the person** (how they like answers, what they track, what they decline) from what they
   do, without being asked to configure anything, and show what it learned so it can be corrected.

Proactivity survives only where it serves one of those two, and only in its smallest form.

## Adversarial review of the first draft

Each part judged on one question: would it make HQ simpler and more useful for someone who did not
build it?

| Part | Verdict | Why |
|---|---|---|
| Signals, rules, a gate, an inbox | **Shrink to the gate** | Solves the owner's problem (many agents, many tasks, too many alerts). A new user has an empty vault: no tasks, no notes, no helpers, so every rule is silent. Cold start defeats it. |
| Rules as vault notes with a validator | **Drop** | A rule language is a power-user feature. "Off and offered" rules hand the configuring back to the user, which is the opposite of the goal. |
| Push budget, quiet hours, coalescing | **Keep, small** | Cheap insurance against becoming annoying. Not a value source on its own. |
| Learned demotion from dismissals | **Keep, widen** | The one part that already learned without being told. But it only learns what to stop; it never learns what to start. |
| Due-date reminders | **Keep, change the entry point** | A person who does not open a Tasks screen never sets a due date. The value is "remind me to collect from the shop on Friday" said in chat, turned into a task with a date, and then reminded. Without the capture, Android's own reminders already do this. |
| Watch prechecks | **Keep as a cost fix** | Real savings for the owner; invisible to a new user. Not part of this goal. |
| Phone signals, deferred model work | **Defer** | Sensible optimisations for a metered connection, no new value. Revisit once there are users on metered data. |
| `similar_to` on the local embedder | **Defer** | Needs a full vault to mean anything. |
| Precision per rule as the success measure | **Replace** | It measures notification quality, not whether HQ became more useful. |

Five of the seven phases in the first draft were infrastructure. The draft's own pushback section
said the bottom-of-pyramid claim was untested; on review, the design did not serve that person at
all.

## What the code says about the real gaps

- **HQ hides almost everything it can do.** About 229 tools are registered (`fn name` across
  `crates/hq-tools` and `crates/hq-agent`). The phone's empty chat offers three fixed chips
  ("Summarise this note", "What needs me today?", "Save a thought as a note",
  `ui/chat/ChatScreen.kt`). The web chat says "Start a conversation" (`MessageList.tsx`). Import a
  PDF or a photo of a receipt, export to Word, run a helper, watch a page, back up: none of it is
  shown to someone who does not already know to ask.
- **The model knows less of HQ than it could, by design.** On Mobile only `MOBILE_EAGER_TOOLS`
  carry schemas and the rest load through `tool_search` (`crates/hq-agent/src/builder/tools.rs`).
  That is right for prompt size, and it means the agent will not suggest a tool it has not loaded.
  Nothing tells it "this person has never used import; a photo just arrived".
- **HQ learns, but invisibly and without structure.** The memory ingester stores summaries with an
  importance score, the turn gate keeps "a lasting fact, decision, or personal preference"
  (`crates/hq-memory/src/turn_gate.rs`), and the querier injects them into every prompt
  (`build_memory_context` in `crates/hq-agent/src/builder/prompt.rs`). A preference is stored like
  any other memory, so it is recalled when relevant rather than applied every time. There is no
  page where the person can see or remove what HQ remembers about them (no memory route in
  `apps/hq-web` or the phone app).
- **The structured profile exists and is never fed.** `_system/USER.md`
  (`crates/hq-daemon/src/user_model.rs`) holds confidence-tagged traits, and outside tests only
  seeded defaults are written. Inbox approvals and dismissals are recorded, and
  `task_engagement_stats` is called only from tests.
- **Capabilities that need a step stay silent until someone finds the setting.** Web push, backup
  and the relays are off by default and nothing suggests them when they would help.
- **The first screen asks for an AI service key** (`docs/FIRST_RUN.md`). For the person this plan
  is for, that is the largest simplification problem in HQ, bigger than anything below. See the
  open questions.

## The design: three loops, all cheap when idle

### 1. One capability catalog

A single list, shipped in the binary like the built-in skills (`crates/hq-tools/assets/`), of what
HQ can do **in the person's terms**, not tool names:

```yaml
- id: receipt-to-record
  says: "Turn a photo of a receipt into a record you can search"
  example: "Here's a receipt from the hardware shop"
  tools: [vault_import, convert_to_markdown]
  needs: [ocr]            # checked against this install
  works_offline: true
- id: remind-me
  says: "Remind you about something on a day you choose"
  example: "Remind me to collect 50,000 from the shop on Friday"
  tools: [task_create]
  needs: []
  works_offline: true
```

`needs` is resolved against the running config and machine (model key present, OCR available,
email connected, push on), so a capability is in one of three states: **ready**, **one step away**
(and the step is a single tap or a single question), or **not on this install** (never shown).

One source, four readers:

- The empty chat on both apps: three to five ready capabilities, chosen by the profile (loop 3),
  not fixed.
- A "What HQ can do" page, grouped by everyday job, each item runnable from its example.
- The agent: a short block in the system prompt listing the ready capabilities this person has not
  used yet, so the model can offer one when it fits, and load its tools.
- `hq doctor` and settings: "one step away" items appear as the step, in plain words.

Size test: the block the agent sees stays under a fixed budget (say 1 KB), so it does not undo the
Mobile prompt diet.

### 2. Offers at the moment of need

Capabilities are discovered by being offered at the moment they would have helped, not by reading a
list. After a turn, a few deterministic detectors look at what just happened (no model call):

| What happened | Offer |
|---|---|
| A photo or document arrived and the reply only described it | keep it as a searchable note |
| The person mentioned a date or a weekday about something to do | a reminder on that day |
| The person pasted a list of things to do | turn it into tasks |
| The same question was asked three times in two weeks | look it up on a schedule (a watch) |
| The person asked for something to send or print | export it as a document |
| A turn needed a capability that is one step away | that one step, in plain words |

At most one offer per turn, shown as a single line under the reply with **Yes** and **Not now**.
**Yes** does it there and then. **Not now** is remembered per offer type; two of them in a row and
that offer stops appearing for 30 days. An offer never interrupts anyone: it is not a notification,
it sits under a reply the person is already reading.

This is the proactive behaviour that serves the goal, and it is the gate from the first draft cut
down to one rule: **offer, once, in context, and remember the answer**.

### 3. A profile HQ learns and shows

A small set of traits that change how HQ behaves every turn, learned from behaviour, never from a
settings form:

| Trait | Learned from |
|---|---|
| Reply length | the person asking for "shorter" or "more detail" |
| Language and register | the languages the person writes in (English, Luganda, Swahili, mixed) |
| Active hours | when they use HQ, which also sets quiet hours for anything that does push |
| What they track | the kinds of notes and tasks they keep making (money owed, stock, school fees, clients) |
| Offers accepted and declined | loop 2 |
| What they dismiss | inbox engagement, the learned demotion from the first draft |
| Stated rules | "always ...", "never ...", "don't ..." said in chat |

Where they live: `_system/USER.md`, the user model HQ already has, with its confidence and
timestamp merge rules. Stated rules come from the turn gate's existing "personal preference"
judgement (or `local_extract` under `memory_extraction: local`) and are promoted from memory to a
trait, so they apply every turn instead of when recalled. Everything else is counters over records
HQ already keeps (chat history, `tool_usage`, `value_items`), computed on the device with no model.

How they are used: a bounded block in the system prompt (at most ten traits), the choice of
capabilities on the empty chat, and quiet hours.

**Shown, not hidden.** A "What HQ has learned about you" page lists each trait in a sentence ("You
like short answers. Learned from 4 times you asked for shorter."), with **Forget** on each. The same
page lists remembered facts from the memory store with a forget button. Learning that the person
cannot see is the part of the Jarvis picture that goes wrong; learning they can see and correct is
an assistant.

### What is kept from the first draft

- Reminders for tasks with a due date, entered from chat through loop 2, one item per task per day.
- A minimal gate for anything that pushes: quiet hours from the learned active hours, at most one
  push an hour folding everything waiting, and the 30-day hide.
- Watch prechecks, as a separate cost fix for the owner.

## Tests every feature here must pass

These are the guard against the dream-cycle pattern. A feature that fails one does not ship.

1. **Works with nothing configured** on a fresh install with an empty vault.
2. **Costs nothing while idle.** No model call on a timer; a model call only inside a turn the
   person started or for something they accepted.
3. **Visible and undoable.** The person can see why HQ did it and turn it off in one tap.
4. **Measurable** with the numbers below.
5. **Nothing leaves the device for it** unless the person asked for the thing that needs it.

## How we will know it worked

- **Time to first useful result**: from first open to the first capability used beyond plain chat.
- **Capabilities used per person per week**, and how many distinct ones in the first month.
- **Offer acceptance** per offer type (accepted over shown). Below a floor, the offer goes.
- **Corrections per week** ("shorter", "no, I meant", repeated instructions). If the profile works,
  this falls.
- **Week-4 return rate** on the pilot.

The first four come from records HQ already keeps; a weekly item in the owner's inbox, built like
`tool-failure-digest`, reports them with no model.

## Who it is for, and the scale of the value

For the owner: HQ stops being a box you have to remember the commands for. For someone who is given
HQ on a phone: the first week shows them, one offer at a time, the five things it can do for their
actual week, in their language, and it stops repeating what they declined. That is the difference
between software that is capable and software that is useful to someone who will never read its
documentation, and it is the only version of HQ that reaches people beyond its builder.

None of the three loops needs a model to decide anything, so their cost does not grow with the
number of people.

## Not doing

- No model on a timer, no daily brief, no "dreaming", no self-model. Retired, and staying retired.
- No rule language for users.
- No notifications for discovery. Offers live under replies, not on the lock screen.
- No learning that is not shown on the "learned about you" page.
- No posting to chat relays (the rule in `CLAUDE.md` stands).

## Phases

Each is one pull request with tests.

**Phase 0: measure.** The weekly report of the numbers above from existing records, so every later
phase has a before and after.

**Phase 1: the capability catalog.** The embedded list, `needs` resolution against config, the
empty-chat chips on web and Android from it, and the agent's unused-capabilities block.

Where the build of phases 0 and 1 differs from the text above:

- The catalog is `crates/hq-tools/assets/capabilities.yaml` (13 entries), read by
  `hq_tools::capabilities`. Only a missing model key is a "one step away" state; anything else a
  capability needs (OCR, the `gws` command, somewhere to run helpers) hides it, because the apps
  cannot take the person through installing software.
- "The person's own sessions" are tool calls recorded with `agent_name = 'hq'`
  (`hq_db::adoption::OWNER_AGENT`): web, the phone app and chat apps. Guests, sub-agents and
  outside MCP clients record other names and are not counted.
- The digest is the daemon task `adoption-digest` (weekly, `daemon.adoption_digest`, not run under
  Lite). Offer acceptance waits for phase 2, which creates offers. Corrections are counted from a
  short list of English phrases (`hq_db::adoption::is_correction`), so the number means "fewer
  misses" only while the list stays the same.
- The chips come from `GET /api/capabilities`: four ready capabilities, untried ones first. With no
  model key there are none on the web (setup comes first) and the phone keeps its fixed three.
- The agent block (at most eight lines, 1 KB) goes only to sessions with the owner's agent name, no
  custom prompt and no parent; `a_mobile_turn_sends_a_small_fixed_prompt` still holds with it.
  It lists what is ready on this install and **nothing about use**: the earlier "has not used
  yet" wording would have sent adoption data to the model provider on every turn. Choosing what
  the person has not tried happens on the device only (the chips now, the offers in phase 2).
- Adoption data stays on the device (`docs/security/ADOPTION_DATA.md`): the digest is
  device-only and never pushed, and a guard test fails if adoption code imports a way out.
- Not built yet: the "What HQ can do" page and the one-step items in `hq doctor` and settings.

**Phase 2: offers.** The post-turn detectors, the one-line offer under a reply, Yes/Not now, and the
30-day memory of declines. The date detector and the reminder ship here. Offers are chosen by HQ
on the device and drawn by the apps; the "I can also do this for you" line for an untried
capability is one of them, so the model is never told what the person has not used.

**Phase 3: the profile.** Promote stated preferences to `USER.md` traits; counters for reply length,
language and active hours; the bounded prompt block.

**Phase 4: "What HQ has learned about you".** The page on web and Android, with Forget for traits
and memories.

**Phase 5: the push gate.** Quiet hours from active hours, hourly coalescing, applied to every push.

## Open questions for the owner

1. **The model key.** A person at the bottom of the pyramid does not have an OpenRouter or Anthropic
   key. Options: an install that works with no model (notes, tasks, reminders, import, search on the
   local embedder) and asks for a key only when a capability needs one; a key the person who sets up
   the phone provides; a small on-device model for the simplest turns. This decides more about reach
   than anything in this plan.
2. **Who is the first pilot user, and in which language?** The catalog's wording and the offers
   should be written for that person's week.
3. **How far should loop 3 go before it asks?** This plan never asks; a single "Did I get that
   right?" the first time a trait is applied may earn trust faster.
