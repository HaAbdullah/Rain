-- 3: game mode also closes the other programs of the PC, so a recorded step may be of kind
-- 'otherApp'. SQLite cannot change a CHECK, so the table is made again with the same rows.
CREATE TABLE gamemode_record_item_new (
  record_id INTEGER NOT NULL REFERENCES gamemode_record (id) ON DELETE CASCADE,
  seq       INTEGER NOT NULL,
  kind      TEXT    NOT NULL CHECK (kind IN ('session', 'otherApp', 'job', 'idleJob', 'service', 'window')),
  name      TEXT    NOT NULL,                              -- a job or program name, or the label of a group
  outcome   TEXT    NOT NULL CHECK (outcome IN ('stopped', 'forced', 'failed', 'leftRunning')),
  count     INTEGER NOT NULL DEFAULT 1,                    -- sessions: how many were left running
  PRIMARY KEY (record_id, seq)
);
INSERT INTO gamemode_record_item_new (record_id, seq, kind, name, outcome, count)
  SELECT record_id, seq, kind, name, outcome, count FROM gamemode_record_item;
DROP TABLE gamemode_record_item;
ALTER TABLE gamemode_record_item_new RENAME TO gamemode_record_item;
