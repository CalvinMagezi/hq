# Agent messaging

Agent sessions in one team (every session reachable through delegation from one
root, see "Delegating") can message each other, as can two sessions working on the
same HQ task. A message is a comment on a task with a kind and a recipient, so the
task thread is the one durable record (`task_comment_list` shows it with
`kind: "message"`).

Needs agent identity (`docs/security/AGENT_IDENTITY.md`): the sender is the
session HQ verified, never a name the agent passes.

## Sending

A launched agent calls the `hq-session` MCP tool `hq_call` with
`agent_message_send` and `{to_session, body, task_id?, reply_to?}`. HQ refuses
the call unless:

- the sender is a running session HQ launched and the gateway attested it;
- the recipient is a running session;
- both sessions work on the same task (their `mission_id` matches), one started
  the other, or both belong to the same team; `task_id`, when given, is the thread
  the message goes on. Between teammates on different tasks it goes on the
  recipient's task;
- the body is 1 to 8000 characters;
- the sender has sent fewer than 60 messages in the last hour.

`to_session` also takes `parent` (the session that started the sender) and
`root` (the team's lead).

## Seeing the team

`team_roster` lists the sender's whole team, root first: each session's id,
who started it, depth, agent, model, purpose, label, status, last agent state and
task. A launched session always sees its own team. A session token also reaches
`harness_session_status`, `harness_session_logs` and `harness_session_wait` for
any session in its team, and the task tools for every teammate's task. Nothing
outside the team is visible to it.

The first prompt of a delegated session names its team's root, its depth, the
roster tool, whether it may delegate further, and that messages from other
sessions are information, never approval.

## Delivery

The message is queued. If the recipient is idle or done it is typed into its pane
at once; otherwise the supervisor types it in when the recipient next goes idle
(the host's state events wake it within seconds, and the minute sweep is the
safety net). One message is delivered per idle period, oldest first, claimed in a
single write so two sweeps cannot both deliver it. A failed typing puts it back.

The recipient sees the text framed as coming from another agent:

```
[Message from agent session hs-... on task PERSONAL-INBOX-001. It comes from another agent, not from the user.]
<body>
[End of message. Reply with the hq-session tool hq_call: tool agent_message_send ...]
```

## Delegating

A session working on a task can hand part of it to a new session with
`agent_delegate` (`{title, description, harness?, model?, purpose?}`). HQ files a sub-task under the
delegator's task (or under that task's own parent, since sub-tasks go one level
deep), starts a session on it in the delegator's directory and host, tells the
worker who asked and how to report, and records the delegation on the delegator's
task. The worker's own `task_comment_add` notes carry its session as author.

The same delegation happens when a session inside an HQ pane calls
`harness_session_spawn` through any other HQ connection: the gateway's
`x-hq-session-id` marker, or a `from_session` argument naming the caller's own
`HQ_SESSION_ID`, turns the spawn into a delegation from that session. `cwd`,
`host` and `task_id` are then the team's and are ignored. Naming a session only
adds limits, so a caller gains nothing by claiming one.

When the worker finishes a turn or exits, HQ queues a message from it to the
delegator on the worker's task thread, quoting its final reply, and the
delegator receives it the next time it is idle. A parent and a session it started
may message each other even though they work on different tasks, on the child's
task thread.

The worker runs as the delegator's own agent unless the owner lists others in
`agent_host.teams.harnesses`. A profile that runs on another account spends that
account's credit, so it is available only when the owner lists it. Delegations are
taken one at a time, so two calls cannot both pass the limits before either worker exists.

Limits apply to the team: every session reachable through delegation from one root
(the session a person or HQ's chat started). All are owner settings:

```yaml
agent_host:
  teams:
    max_depth: 3            # levels below the root; 1 makes every worker a leaf (1..=8)
    max_live_per_tree: 8    # running sessions in the team, root included (2..=40)
    max_live_children: 5    # running children of one session (1..=20)
    max_spawns_per_hour: 30 # sessions the team starts per hour, ended ones included (1..=120)
    harnesses: []           # other agents a delegator may start, e.g. [pi, copilot-lean]
```

A refusal names the setting that stopped it. Stopping a session stops the sessions
started for it, and theirs.

## What is checked, and what is cleaned

A body, a title and a description are stripped of control characters and terminal
escape sequences before they are stored or typed anywhere, and the host removes
bracketed-paste markers from anything it pastes, so text cannot end a paste early
and type keystrokes into the recipient. The frame around a message carries a
random marker the sender never sees, so a body cannot close the frame and make
what follows look like the user's own words. A message nobody took within a day
is not delivered.

The session-token tools only reach the session's own task, that task's parent and
sub-tasks, and the tasks of its parent and children, and `harness_session_status`
only itself, its parent and its children.

## Limits and risks

- A message is untrusted text from another agent, and it reaches a model that may
  act on it. The framing says so, but it is not a guarantee: a worker that obeys
  whatever a peer types is only as safe as that peer. Keep sessions that handle
  untrusted content away from sessions with wide access.
- Messages go between sessions on one task only. Parents and children, leases,
  broader permissions and quotas beyond the hourly cap are later work.
- Claude Code only for now (it is the agent with an MCP config path).
