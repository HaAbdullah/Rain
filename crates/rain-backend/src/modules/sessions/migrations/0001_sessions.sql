-- Sessions that ended, so the "Done" and "Failed" groups and their transcripts survive a restart
-- of the backend. Machine history, not a cache: nothing here can be rebuilt. Open permission
-- requests are never stored.
CREATE TABLE sessions_ended (
  id                TEXT PRIMARY KEY,                     -- the Claude session id
  title             TEXT NOT NULL,
  cwd               TEXT NOT NULL,
  project_id        TEXT,
  branch            TEXT,
  model             TEXT NOT NULL,
  host              TEXT NOT NULL,
  status            TEXT NOT NULL CHECK (status IN ('success', 'failed')),
  started_at        TEXT NOT NULL,
  ended_at          TEXT NOT NULL,
  last_activity_at  TEXT NOT NULL,
  tokens            INTEGER NOT NULL DEFAULT 0,
  last_message      TEXT NOT NULL DEFAULT '',
  transcript_path   TEXT,                                 -- the path Claude Code gave; only ever read
  recent_json       TEXT NOT NULL DEFAULT '[]'            -- the last 8 events as JSON
);
CREATE INDEX idx_sessions_ended_ended_at ON sessions_ended (ended_at DESC);
