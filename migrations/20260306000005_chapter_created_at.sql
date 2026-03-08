-- Track when each chapter was first seen by rebarr.
-- Existing rows get the current timestamp as an approximation.
ALTER TABLE Chapter ADD COLUMN created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'));
