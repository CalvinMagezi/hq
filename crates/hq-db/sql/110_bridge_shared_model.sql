-- A machine's shared model, on the hub (`hq/share/1`, docs/PAIRING.md "Sharing a model"). On the
-- setup row, so revoking the machine removes it with the rest. NULL model: nothing shared.
ALTER TABLE bridge_setup ADD COLUMN shared_model TEXT;
-- 1 when the machine's server says the model takes tool calls, 0 when it says it does not, NULL
-- when it did not say.
ALTER TABLE bridge_setup ADD COLUMN shared_tools INTEGER;
-- The context the machine's server gives the model, when it knew.
ALTER TABLE bridge_setup ADD COLUMN shared_context INTEGER;
