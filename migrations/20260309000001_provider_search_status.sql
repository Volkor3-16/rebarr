-- Make provider_url nullable (NULL = searched but not found) and add search_attempted_at.
-- Requires table recreation in SQLite since we can't drop NOT NULL constraints via ALTER TABLE.
PRAGMA foreign_keys=OFF;

CREATE TABLE MangaProvider_new (
    manga_id             TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    provider_name        TEXT NOT NULL,
    provider_url         TEXT,          -- NULL means "searched but not found"
    last_synced_at       TEXT,          -- ISO 8601; set after each successful chapter scrape
    provider_score       REAL NOT NULL DEFAULT 0,
    score_override       REAL,
    search_attempted_at  TEXT,          -- ISO 8601; set on every search attempt (found or not)
    PRIMARY KEY (manga_id, provider_name)
);

INSERT INTO MangaProvider_new
    SELECT manga_id, provider_name, provider_url, last_synced_at,
           provider_score, score_override, last_synced_at
    FROM MangaProvider;

DROP TABLE MangaProvider;
ALTER TABLE MangaProvider_new RENAME TO MangaProvider;

CREATE INDEX IF NOT EXISTS idx_manga_provider_manga_id ON MangaProvider(manga_id);

PRAGMA foreign_keys=ON;
