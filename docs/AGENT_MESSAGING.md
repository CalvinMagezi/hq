# Agent messaging

Two agent sessions working on the same HQ task can message each other. A message
is a comment on that task with a kind and a recipient, so the task thread is the
one durable record (`task_comment_list` shows it with `kind: "message"`).

Needs agent identity (`docs/security/AGENT_IDENTITY.md`): the sender is the
session HQ verified, never a name the agent passes.

## Sending

A launched agent calls the `hq-session` MCP tool `hq_call` with
`agent_message_send` and `{to_session, body, task_id?, reply_to?}`. HQ refuses
the call unless:

- the sender is a running session HQ launched and the gateway attested it;
- the recipient is a running session;
- both sessions work on the same task (their `mission_id` matches), and
  `task_id`, when given, is that task;
- the body is 1 to 8000 characters;
- the sender has sent fewer than 60 messages in the last hour.

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

## Limits and risks

- A message is untrusted text from another agent, and it reaches a model that may
  act on it. The framing says so, but it is not a guarantee: a worker that obeys
  whatever a peer types is only as safe as that peer. Keep sessions that handle
  untrusted content away from sessions with wide access.
- Messages go between sessions on one task only. Parents and children, leases,
  broader permissions and quotas beyond the hourly cap are later work.
- Claude Code only for now (it is the agent with an MCP config path).
