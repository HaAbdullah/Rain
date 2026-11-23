-- Rebuildable cache: the raw items of the last scan of shell:AppsFolder. Replaced as a whole at every scan.
CREATE TABLE launcher_app (
  key          TEXT PRIMARY KEY,        -- the match key of app-ids.toml
  aumid        TEXT NOT NULL,
  name         TEXT NOT NULL,
  target       TEXT NOT NULL,           -- System.Link.TargetParsingPath, may be empty
  args         TEXT NOT NULL,           -- System.Link.Arguments, may be empty
  target_mtime INTEGER,                 -- seconds since 1970 of the target file, NULL when not a file
  scanned_at   TEXT NOT NULL            -- RFC 3339, same format as the API
) STRICT;

-- History: one row per successful use. Not rebuildable (losing it loses "recent" and "last launched").
CREATE TABLE launcher_launch (
  id          INTEGER PRIMARY KEY AUTOINCREMENT,
  target_type TEXT NOT NULL CHECK (target_type IN ('app', 'pool')),   -- projects and games keep their own history
  target_id   TEXT NOT NULL,
  at          TEXT NOT NULL,            -- RFC 3339 UTC with 3 fraction digits
  run_id      TEXT NOT NULL
) STRICT;
CREATE INDEX idx_launcher_launch_target ON launcher_launch (target_type, target_id, at DESC);
CREATE INDEX idx_launcher_launch_at ON launcher_launch (at DESC);
