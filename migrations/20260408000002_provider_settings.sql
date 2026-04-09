-- Replace Providers (score + enabled) with a simpler ProviderSettings (enabled only).
-- The numeric score override is removed: Quality Rules handle all ranking.
-- Enable/disable per provider (globally and per-series) is preserved.

CREATE TABLE ProviderSettings (
    provider_name  TEXT NOT NULL,
    -- NULL = global setting; non-NULL = per-series override
    manga_id       TEXT,
    enabled        INTEGER NOT NULL DEFAULT 1,
    UNIQUE (provider_name, manga_id)
);

-- Migrate any explicitly-disabled providers from the old Providers table.
INSERT OR IGNORE INTO ProviderSettings (provider_name, manga_id, enabled)
SELECT provider_name, manga_id, enabled
FROM Providers
WHERE enabled = 0;

DROP TABLE IF EXISTS Providers;
DROP TABLE IF EXISTS TrustedGroup;
