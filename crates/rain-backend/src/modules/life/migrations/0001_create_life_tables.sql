-- CACHE (rebuildable). One row per daily note that existed at the last check.
-- `scope` identifies the vault path, the folder and the format together, so a changed setting
-- never mixes old rows with new ones. `bools` holds ONLY the names and values of the top-level
-- true/false properties of the note, as JSON, for example {"gym":true,"read":false}. No note text.
CREATE TABLE life_note (
  scope    TEXT    NOT NULL,   -- 16 hex characters: hash of lower-cased vault path + "|" + folder + "|" + format
  date     TEXT    NOT NULL,   -- YYYY-MM-DD
  mtime_ns INTEGER NOT NULL,   -- last write time of the file, nanoseconds since 1970
  size     INTEGER NOT NULL,   -- length in bytes
  bools    TEXT    NOT NULL,   -- JSON object, see above
  PRIMARY KEY (scope, date)
) WITHOUT ROWID;

-- HISTORY. One row per vault write Rain made (an audit trail so the user can see what Rain changed).
-- No note text, no property value other than the boolean, no path.
CREATE TABLE life_write (
  id       INTEGER PRIMARY KEY AUTOINCREMENT,
  at       TEXT    NOT NULL,   -- RFC 3339 UTC, as the API
  date     TEXT    NOT NULL,   -- the note's date
  habit_id TEXT    NOT NULL,
  property TEXT    NOT NULL,
  done     INTEGER NOT NULL,   -- 1 or 0
  outcome  TEXT    NOT NULL    -- 'created' | 'updated' | 'unchanged'
);
CREATE INDEX idx_life_write_at ON life_write(at);
