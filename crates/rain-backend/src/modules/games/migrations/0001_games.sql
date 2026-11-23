-- History: what Rain itself knows about playing a game. Not a cache.
CREATE TABLE games_play (
  game_key       TEXT PRIMARY KEY,         -- "steam:<appid>", "epic:<AppName>", "dir:<lower-case path>", "custom:<id>"
  last_played_at TEXT,
  play_seconds   INTEGER NOT NULL DEFAULT 0,
  launches       INTEGER NOT NULL DEFAULT 0
);

-- Sessions that are running now (a row is folded into games_play when the process ends, then deleted).
CREATE TABLE games_session (
  id            INTEGER PRIMARY KEY AUTOINCREMENT,
  game_key      TEXT NOT NULL,
  pid           INTEGER NOT NULL,
  process_start INTEGER NOT NULL,          -- FILETIME of the process creation; protects against pid reuse (never in JSON)
  folder        TEXT,                      -- install folder
  started_at    TEXT NOT NULL,
  last_seen_at  TEXT NOT NULL              -- written at launch and at backend stop
);
CREATE INDEX idx_games_session_key ON games_session (game_key);

-- Rebuildable cache: the program-file guess of a folder game.
CREATE TABLE games_guess_cache (
  folder_key TEXT PRIMARY KEY,             -- lower-case normalised folder path
  signature  TEXT NOT NULL,
  exe_rel    TEXT,                         -- path relative to the folder; NULL = nothing found
  score      REAL,
  guessed_at TEXT NOT NULL
);

-- Rebuildable cache: size of a game folder.
CREATE TABLE games_size_cache (
  folder_key  TEXT PRIMARY KEY,
  size_bytes  INTEGER NOT NULL,
  measured_at TEXT NOT NULL,
  signature   TEXT NOT NULL
);
