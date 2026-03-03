-- Cache of "this manga lives at this URL on this provider".
-- Avoids re-searching on every sync cycle.
CREATE TABLE IF NOT EXISTS MangaProvider (
    manga_id       TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    provider_name  TEXT NOT NULL,
    provider_url   TEXT NOT NULL,
    last_synced_at TEXT,          -- ISO 8601, nullable; set after each successful chapter scrape
    PRIMARY KEY (manga_id, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_manga_provider_manga_id ON MangaProvider(manga_id);
