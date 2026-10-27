---
name: hq-status
description: Print a compact status of the connected HQ: health, hosts, running agent sessions, and in-progress, blocked and stale tasks. Run it with /agent-hq:hq-status.
disable-model-invocation: true
---

# HQ status

Gather the facts below with `hq_call(tool, args)` and print one compact summary. If a call fails, say which one and keep going.

1. Health: `system_info`, or the health check the session-start hook ran.
2. Hosts: `host_list`. Note any host that is unreachable.
3. Agents: `harness_session_list`, running sessions only. For each, show the harness, host, label and linked task.
4. Tasks: `task_list` with status `in_progress`, then with status `blocked`. Show id, title and assignee.
5. Stale work: `task_stale`.

Format as short sections with one line per item. End with a single line naming the most urgent thing, for example a blocked task or an unreachable host. If everything is quiet, say so.
