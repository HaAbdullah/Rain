-- Which project folders Rain has seen. Lets a vanished folder be shown as "missing". Losing it
-- only forgets the missing projects; it holds no user data.
CREATE TABLE projects_known (
  path_key      TEXT PRIMARY KEY,          -- lower-case normalised path
  id            TEXT NOT NULL UNIQUE,      -- "proj-<slug>" as first given (keeps the id stable)
  path          TEXT NOT NULL,             -- normalised path, original case
  name          TEXT NOT NULL,             -- folder name
  root_key      TEXT NOT NULL,             -- lower-case normalised root that found it
  first_seen_at TEXT NOT NULL,
  last_seen_at  TEXT NOT NULL
);
CREATE INDEX idx_projects_known_root ON projects_known (root_key);

-- History: when the user opened the project from Rain. Not a cache.
CREATE TABLE projects_opened (
  path_key  TEXT PRIMARY KEY,
  opened_at TEXT NOT NULL,                 -- the last successful open
  opens     INTEGER NOT NULL DEFAULT 1
);

-- Rebuildable cache of git facts.
CREATE TABLE projects_git_cache (
  path_key    TEXT PRIMARY KEY,
  head_sig    TEXT NOT NULL,               -- signature of HEAD, the HEAD reflog and the index
  branch      TEXT NOT NULL,
  dirty       INTEGER,                     -- NULL = unknown
  dirty_at    TEXT,
  commits     TEXT,                        -- JSON array of 14 integers, NULL = unknown
  commits_day TEXT,                        -- the local date (YYYY-MM-DD) the array was computed for
  computed_at TEXT NOT NULL
);
