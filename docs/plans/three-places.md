# Three places: Notes, Chat, Inbox

Status: phase 1 built (this change). Phases 2 and 3 are proposals.

## The problem

HQ's bar had five places: Notes, Tasks, Chat, Helpers, Inbox. The owner, and the people the
owner gave HQ to, used three of them. Tasks and Helpers were opened rarely, and mostly to check
on something HQ or an agent was doing. They are HQ's workbench, drawn as if they were the
person's.

That costs twice. A new person meets five doors and has to learn which two are not for them.
And because the workbench was a place, its events were treated as the person's business: a
helper that stopped at a prompt went to the owner's inbox and lock screen even while HQ was
driving that helper and could answer the prompt itself.

Who it is for: someone handed HQ on a phone, who writes notes, asks questions, and wants to be
interrupted only when a decision is theirs.

## The rule

1. **A person has three places.** Notes is what they keep. Chat is where they ask. Inbox is
   where HQ asks them.
2. **Tasks and helpers belong to HQ.** They are how HQ tracks and does work. A person can look
   in, but never has to.
3. **The owner hears from HQ, not from HQ's helpers.** A helper's question goes to whoever
   drives it. It reaches the owner when the answer is the owner's to give, or when nobody
   else answered.

## Phase 1 (built)

### The bar

- Web and desktop: `BAR_PLACES` in `apps/hq-web/src/lib/routes.ts` is Notes, Chat, Inbox. Lite
  has no Chat, so its bar is Notes and Inbox. The desktop window shows the web app, so it
  follows.
- Phone: `Place.bar` in `ui/shell/HqShell.kt`.
- Tasks and Helpers keep every address and link (`/tasks/...`, `/helpers/...`,
  `hq://tasks`, `hq://helpers`). They open from an Inbox item, a chat card, search, a
  notification, and on the web from Settings, "What HQ is working on".

### Who is told when a helper stops at a prompt

| The session | Before | Now |
|---|---|---|
| No chat watches it | Inbox item at once | The same |
| A chat watches, Drive off | Chat message and an inbox item | The same |
| A chat drives it | Chat wake and an inbox item | Chat wake only. An inbox item if the prompt is still there after `agent_host.driver_grace_minutes` (default 5) |
| A chat drove it and then closed | Nothing more: the block was already claimed | Inbox item after the grace |

When HQ's driver turn wants to do something that needs the owner (send keys to the helper
from a chat that has read outside text, spawn, hand off), that is an owner approval, written
in HQ's name, and it shows in the chat that asked as well as in the Inbox.

An item about a prompt also leaves the inbox when the session is no longer blocked or has
ended. Before, a "needs an answer" item for a chat-watched session had no expiry and no link
to the dialog, so it stayed in Needs you after the prompt was long gone.

### The inbox as something a person keeps

- **It follows HQ while open.** The web list loaded once per visit, so a request that pushed a
  notification could be missing from a list that was already on screen. It now reloads on
  `inbox:change`, on reconnect and when the tab is shown again. On the phone, the inbox is
  read every few seconds while a reply is running, and once when it ends.
- **A request shows in the chat that asked.** The web already drew the approval card in the
  chat. The phone now does too (`ChatNeeds` in `ui/inbox/InboxScreen.kt`), with the same card
  and buttons as the Inbox.
- **Read.** `value_items.read_at`. An update is new until it is opened or "Mark all as read"
  is pressed. Reading puts nothing away.
- **Clear.** "Clear" on an update, and "Clear all updates", put updates into Done, with Undo.
  The bulk Undo called a route that was never served; it is served now. Done is the archive.
- **Clear this list** on Done hides what is finished (`cleared_at`). Rows are kept, so an
  approval's record and a dismissed item's dedup still hold.
- **Silence.** Per topic (helpers, tasks, tips and digests, upkeep, email) or everything for
  an hour or eight. A silenced item still arrives in the inbox; it sends no push and no phone
  notice, and is left off the badge. HQ's own requests (approvals, a new sign-in, a message
  HQ chose to send) are not a topic and always get through. A source nobody mapped counts as
  HQ's own, so the safe default is to interrupt.
- **The badge** counts the Inbox's requests. It used to add the helpers at a prompt on top,
  which showed one question twice.

### What phase 1 did not do

- The phone's Inbox has no Mark all as read, Clear this list or Silence controls yet. It
  honours a silence set on the web (no notice is posted for a silenced item).
- A question from a server HQ still opens that HQ's own web app from the phone's notice. The
  phone's Inbox lists only the HQ it runs.
- The phone's Settings has no "What HQ is working on" group.

## Phase 2: tasks that agents own (proposal)

Tasks were built as a person's task manager (spaces, boards, timelines) that agents could also
write to. The data says the reverse is true: agents are the main writers and readers.

- **One sentence in, a task out.** A person never fills in a task form. They say what they
  want in Chat; HQ makes the task, picks who works on it, and says so in one line. The task
  page is a record, not an editor.
- **Review reaches the owner only when it is theirs.** `task_ready_for_review` is an
  action-needed item for every finished task. Task verification already completes tasks from
  evidence. Proposal: a task that verification passes is closed with an update, not a request;
  only `task_needs_owner` (verification could not decide) stays a request. Measure first:
  count, over four weeks, how many review requests the owner answered by opening the task
  versus clearing the notice.
- **A daily line, not a board.** One update a day in the Inbox: what HQ finished, what is
  stuck and why. It replaces the briefing, the queue alert and the stale digest, which are
  three items today.
- **Retire the person-facing views nobody opens.** Board and Timeline stay reachable while
  the count of opens is collected (adoption data, device only). If they stay unused for a
  release, remove them rather than maintain them.

Success: the share of inbox requests that are about tasks falls, and no finished task waits
on the owner for more than a day without the owner having been asked something they had to
decide.

## Phase 3: helpers that HQ runs (proposal)

- **Drive is the default, watching is the exception.** A helper started from Chat is driven
  unless the person says otherwise. A helper nobody drives is the case that interrupts the
  owner, so it should be rare and deliberate.
- **Command policies do most of the answering.** Every prompt a policy settles is one HQ does
  not spend a turn on and the owner never sees. Proposal: mine `harness_session_events` for
  prompts the owner or the driver always answered the same way, and offer them as policy
  entries (the owner approves the rule once).
- **Finished and exited are HQ's to read.** For a driven session these are already chat
  wakes. For the rest they are updates. Proposal: fold them into the daily line unless the
  exit left a task blocked.
- **Start a helper from Chat.** "Start a helper" lives on the Helpers page. With the page off
  the bar, the chat needs to offer it (a capability chip and `harness_session_spawn` behind
  owner approval, which exists).

Success: prompts answered without the owner (by policy or by the driver) as a share of all
prompts, and the number of helper items a week in the owner's inbox.

## Risks

- **A request the owner never sees.** Mitigated three ways: the grace is a floor, not a
  filter (an unanswered prompt always reaches the owner); a block is raised at most once
  whatever state its item ends in; silence cannot cover HQ's own topic.
- **Five minutes is a guess.** It is a config key. The number to watch is how many prompts
  the driver clears inside the grace versus how many fall through.
- **Hidden places become forgotten places.** Tasks and Helpers must stay reachable and
  tested while they are off the bar, or they will rot.
