-- Pre-computed token count for context engine knapsack allocator.
-- NULL means not yet computed; callers fall back to count_tokens_fast().
ALTER TABLE vault_cache ADD COLUMN token_count INTEGER;
