-- The first untrusted source a chat thread read (a tool name). Once set, every later turn of
-- the thread starts tainted with it, so a calendar or web read in one message still holds the
-- next message to the tainted-session rules.
ALTER TABLE chat_threads ADD COLUMN taint_source TEXT;
