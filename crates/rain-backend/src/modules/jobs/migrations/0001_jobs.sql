-- Scheduled job runs, their log lines, and the scheduler's memory. Section 03 5.1.
-- Machine history: losing rain.db loses these, never a job definition (those are TOML).
CREATE TABLE jobs_run (
  id                 TEXT PRIMARY KEY,                    -- run-20261121-093000-ab12
  job_id             TEXT NOT NULL,                       -- the id in jobs.toml
  trigger_kind       TEXT NOT NULL CHECK (trigger_kind IN ('schedule', 'manual', 'catchup')),
  status             TEXT NOT NULL CHECK (status IN ('running', 'success', 'failed', 'idle')),
  started_at         TEXT NOT NULL,
  ended_at           TEXT,
  duration_ms        INTEGER,
  exit_code          INTEGER,                             -- the real exit code only
  claude_session_id  TEXT,                                -- set when a Claude job reported its session
  log_lines          INTEGER NOT NULL DEFAULT 0,          -- lines stored in jobs_run_log
  log_omitted        INTEGER NOT NULL DEFAULT 0           -- lines dropped by the cap
);
CREATE INDEX idx_jobs_run_job_started ON jobs_run (job_id, started_at DESC);
CREATE INDEX idx_jobs_run_status ON jobs_run (status);

-- Log lines of scheduled runs AND of idle runs (idle run ids start with "irun-").
CREATE TABLE jobs_run_log (
  run_id  TEXT    NOT NULL,
  seq     INTEGER NOT NULL,
  ts      TEXT    NOT NULL,
  level   TEXT    NOT NULL CHECK (level IN ('info', 'warn', 'error')),
  text    TEXT    NOT NULL,                               -- at most 2000 characters
  PRIMARY KEY (run_id, seq)
) WITHOUT ROWID;

-- Every occurrence of a job up to this instant has been handled (run, skipped, or deliberately not run).
CREATE TABLE jobs_state (
  job_id         TEXT PRIMARY KEY,
  handled_until  TEXT NOT NULL,
  updated_at     TEXT NOT NULL
);
