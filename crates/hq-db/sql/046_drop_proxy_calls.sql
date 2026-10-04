-- The sub-task fleet dispatch system (multi-harness routing, self-learning
-- quality feedback) is retired: HQ is its own single harness everywhere
-- (web, Telegram, Discord, CLI), with only LLM provider-level fallback
-- remaining. proxy_calls fed the retired HarnessRouter's learned-quality
-- scoring and DISPATCH-POLICY.md; nothing reads or writes it anymore.
DROP TABLE IF EXISTS proxy_calls;
