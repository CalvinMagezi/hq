# Forgetting a memory does not offer to delete the chat it came from

**Reason skipped:** the plan had a tick-box "also delete the chat it came from", off by default. HQ has no function that deletes a whole chat thread (`hq_db::chat` can only delete the messages from one point on), and a chat leaves copies in the message index, the folds that summarise it, the stored thread files and their embeddings, so building that correctly is its own change. Forget therefore removes the memory and every copy HQ made of it, and says in its output that the chat still holds what was said there.
**Date:** 2026-11-27
**Where:** `crates/hq-memory/src/forget.rs` (`NOT_REACHED`), `crates/hq-db/src/chat.rs`
**Remove when:** a thread can be deleted with every derived copy of it (messages, `chat_fts`, `thread_folds` and their embeddings, `_threads/`), and `forget` takes an option to call it for the memory's `thread_id`.

The memory already records the chat it was learned in (`memories.thread_id`), so the link is there for when this is built.
