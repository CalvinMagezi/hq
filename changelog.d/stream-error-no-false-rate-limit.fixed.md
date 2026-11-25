**A failed stream is no longer mistaken for a rate limit.** An error whose text merely contained the
letters "rate" (such as "generate") skipped the non-streaming fallback. Only real rate-limit errors do now.
