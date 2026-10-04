-- Tables whose readers and writers were all removed: the ModelCard and SBLU
-- systems, earn, Hermes, harness quotas, the tracing tables, file locks, link
-- state, and the usage_records mirror of task_outcomes.
DROP TABLE IF EXISTS model_cards;
DROP TABLE IF EXISTS model_synergy;
DROP TABLE IF EXISTS model_card_daily;
DROP TABLE IF EXISTS sblu_training_runs;
DROP TABLE IF EXISTS bounties;
DROP TABLE IF EXISTS earn_usage;
DROP TABLE IF EXISTS hermes_sessions;
DROP TABLE IF EXISTS harness_quotas;
DROP TABLE IF EXISTS span_events;
DROP TABLE IF EXISTS spans;
DROP TABLE IF EXISTS traces;
DROP TABLE IF EXISTS locks;
DROP TABLE IF EXISTS link_state;
DROP TABLE IF EXISTS usage_records;
