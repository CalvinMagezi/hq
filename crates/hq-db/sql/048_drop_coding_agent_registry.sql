-- Coding agents now run through harness_session specs over Herdr. The
-- binary-probe registry and its performance table were only read by the
-- retired SELF.md self-model and the removed coding_agents tools.
DROP TABLE IF EXISTS coding_agent_performance;
DROP TABLE IF EXISTS coding_agent_registry;
