-- Idle job runs. Section 03 5.2. Machine history; the definitions are in idle-jobs.toml.
CREATE TABLE idle_state (
  job_id           TEXT PRIMARY KEY,
  last_started_at  TEXT,
  last_ended_at    TEXT,
  last_result      TEXT CHECK (last_result IN ('success', 'failed'))   -- NULL until the first run ends
);

CREATE TABLE idle_run (
  id          TEXT PRIMARY KEY,                           -- irun-20261121-031500-9f3c
  job_id      TEXT NOT NULL,
  started_at  TEXT NOT NULL,
  ended_at    TEXT,
  status      TEXT NOT NULL CHECK (status IN ('running', 'success', 'failed', 'stopped')),
  manual      INTEGER NOT NULL DEFAULT 0,                 -- started by "Run now"
  exit_code   INTEGER
);
CREATE INDEX idx_idle_run_job_started ON idle_run (job_id, started_at DESC);
