**Task lists for helpers and the time report do less work.** A helper that sees only its own tasks now has
the database pick and count them, instead of reading every task and discarding most. The time report reads
all leases in one query instead of one per task, and importing a folder scans the vault once, not twice.
