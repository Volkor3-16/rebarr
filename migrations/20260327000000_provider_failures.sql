-- Track provider failures for auto-backoff and auto-disabling
CREATE TABLE IF NOT EXISTS ProviderFailure (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_name TEXT NOT NULL,
    manga_id TEXT NOT NULL,
    failed_at INTEGER NOT NULL,
    error_message TEXT,
    FOREIGN KEY (manga_id) REFERENCES Manga(uuid) ON DELETE CASCADE
);

-- Index for looking up failures by provider + manga
CREATE INDEX IF NOT EXISTS idx_provider_failure_provider_manga
    ON ProviderFailure(provider_name, manga_id);

-- Index for cleaning up old failures
CREATE INDEX IF NOT EXISTS idx_provider_failure_failed_at
    ON ProviderFailure(failed_at);

-- Settings for provider failure handling
INSERT OR IGNORE INTO Settings(key, value) VALUES ('provider_disable_threshold', '5');
INSERT OR IGNORE INTO Settings(key, value) VALUES ('provider_backoff_minutes', '60');
INSERT OR IGNORE INTO Settings(key, value) VALUES ('queue_paused', 'false');