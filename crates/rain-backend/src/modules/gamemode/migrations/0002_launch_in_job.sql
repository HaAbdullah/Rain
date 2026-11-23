-- 1: the game was started inside a job of Rain and may close together with Rain (the recap says so).
ALTER TABLE gamemode_record ADD COLUMN launched_in_job INTEGER NOT NULL DEFAULT 0;
