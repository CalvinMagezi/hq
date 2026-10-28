# Agent HQ MCP server plugin for Claude Code

Adds your [Agent HQ](https://github.com/CalvinMagezi/hq)'s MCP server to Claude Code, which gives a session the `hq_discover` and `hq_call` tools. It is kept apart from the [`agent-hq`](../claude-plugin/README.md) plugin so that people whose HQ already reaches Claude Code through a claude.ai connector do not load the same tools twice.

## Install

```
/plugin marketplace add CalvinMagezi/hq
/plugin install agent-hq-mcp@agent-hq
```

Claude Code asks for your HQ MCP URL and API key, the same two values the `agent-hq` plugin asks for.

Skip this plugin when your HQ is added as a claude.ai connector: Claude Code loads claude.ai connectors by itself.

Sessions that HQ spawns send their session id in the `x-hq-session-id` header, so HQ can tell their calls apart.
