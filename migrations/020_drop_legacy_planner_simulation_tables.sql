-- Remove planner/simulation-era tables that no longer belong to the
-- companion-first runtime.

DROP TABLE IF EXISTS outcome_observations;
DROP TABLE IF EXISTS action_candidates;
DROP TABLE IF EXISTS simulation_events;
DROP TABLE IF EXISTS simulation_runs;
DROP TABLE IF EXISTS scenario_actors;
DROP TABLE IF EXISTS scenarios;
DROP TABLE IF EXISTS plan_trace_observations;

INSERT INTO schema_version (version) VALUES (20);
