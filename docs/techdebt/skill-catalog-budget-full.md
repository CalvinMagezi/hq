# The skill catalog budget is full

**Reason skipped:** the built-in skill descriptions total 4,096 bytes, the limit, because they ship in every system prompt. Adding `finance-import` took two descriptions down to 115 and 152 characters to fit.
**Date:** 2026-11-21
**Where:** `crates/hq-tools/src/skills/builtin_tests.rs`, `crates/hq-tools/assets/skills/`
**Remove when:** the next built-in skill is added: load the calendar, contacts and finance skills only for phone chats, or raise the budget on purpose.
