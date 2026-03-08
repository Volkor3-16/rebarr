-- Add monitored flag to Manga (default true — all existing manga become monitored)
ALTER TABLE Manga ADD COLUMN monitored BOOLEAN NOT NULL DEFAULT 1;

-- Global key-value settings store
CREATE TABLE IF NOT EXISTS Settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO Settings (key, value) VALUES ('scan_interval_hours', '6');
