-- One row per game mode run that reached the wind-down. History, not cache: losing it loses only the recap.
CREATE TABLE gamemode_record (
  id                  INTEGER PRIMARY KEY AUTOINCREMENT,
  started_at          TEXT    NOT NULL,                    -- RFC 3339, when the wind-down began
  finished_at         TEXT,                                -- NULL while running, and for good if the backend died
  outcome             TEXT    NOT NULL CHECK (outcome IN ('running', 'completed', 'incomplete')),
  countdown_seconds   INTEGER NOT NULL,
  waited              INTEGER NOT NULL DEFAULT 0,          -- 1: the user chose Wait
  forced              INTEGER NOT NULL DEFAULT 0,          -- 1: something was interrupted
  then_launch_game_id TEXT,
  then_launch_title   TEXT,
  launch_result       TEXT CHECK (launch_result IN ('started', 'failed')),
  launch_error        TEXT,
  seen_at             TEXT                                 -- NULL until the recap notice was shown
);
CREATE INDEX idx_gamemode_record_started ON gamemode_record (started_at);

CREATE TABLE gamemode_record_item (
  record_id INTEGER NOT NULL REFERENCES gamemode_record (id) ON DELETE CASCADE,
  seq       INTEGER NOT NULL,
  kind      TEXT    NOT NULL CHECK (kind IN ('session', 'job', 'idleJob', 'service', 'window')),
  name      TEXT    NOT NULL,                              -- a job name, or the label of a group
  outcome   TEXT    NOT NULL CHECK (outcome IN ('stopped', 'forced', 'failed', 'leftRunning')),
  count     INTEGER NOT NULL DEFAULT 1,                    -- sessions: how many were left running
  PRIMARY KEY (record_id, seq)
);
