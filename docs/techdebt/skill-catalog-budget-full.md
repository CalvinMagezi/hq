# The skill catalog budget is full

The descriptions of the built-in skills total 4,096 bytes, the limit, because they ship in every system prompt. Adding `finance-import` took two descriptions down to 115 and 152 characters to fit. The next built-in skill will not fit without bundling skills or loading device skills only on a phone chat.

Fix when: the next skill is added. Load the calendar, contacts and finance skills only for phone chats, or raise the budget on purpose.

File: `crates/hq-tools/src/skills/builtin_tests.rs`, `crates/hq-tools/assets/skills/`.
