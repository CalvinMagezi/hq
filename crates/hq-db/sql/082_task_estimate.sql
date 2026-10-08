-- Planned effort in minutes, compared with the time leases record. NULL = no estimate.
ALTER TABLE tasks ADD COLUMN estimate_minutes INTEGER CHECK (estimate_minutes IS NULL OR estimate_minutes > 0);
