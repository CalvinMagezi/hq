-- A running turn's last known "I'm waiting on someone" signal from
-- report_progress. Persisted (not just carried through the transient
-- ProgressSink) so the startup/6h reconciler can tell "was waiting on your
-- answer to X" apart from "genuinely stranded" after the process that knew
-- it is gone.
ALTER TABLE background_turns ADD COLUMN blocked_on TEXT;
ALTER TABLE background_turns ADD COLUMN blocked_detail TEXT;
