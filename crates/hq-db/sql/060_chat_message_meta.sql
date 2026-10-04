-- A web chat reply's tool calls, reasoning and stop flag, as JSON, so a reload
-- shows them. NULL for every other message.
ALTER TABLE chat_messages ADD COLUMN meta TEXT;
