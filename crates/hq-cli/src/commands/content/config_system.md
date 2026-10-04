---
noteType: system-file
fileName: config
version: 2
pinned: false
---
# Configuration

System-level configuration values. These are read by agents and the daemon at runtime.

| Key | Value | Description |
|-----|-------|-------------|
| DEFAULT_MODEL | anthropic/claude-sonnet-4 | Default LLM for agent sessions |
| orchestration_mode | internal | How tasks are routed (internal, delegated, hybrid) |
| heartbeat_interval | 120 | Seconds between daemon heartbeat cycles |
| max_concurrent_agents | 3 | Maximum parallel sub-agent sessions |
| embedding_batch_size | 10 | Notes embedded per daemon cycle |

Edit values directly in this table. Changes take effect on the next daemon cycle or agent session.
