-- Provider score and per-series enable/disable overrides.
--
-- manga_id IS NULL  → global override (applies to all series using that provider)
-- manga_id IS NOT NULL → series-specific override (takes priority over global)
--
-- The 'enabled' column is only meaningful for series-specific rows.
--
-- Two partial unique indexes are used instead of a composite PRIMARY KEY because
-- SQLite allows multiple NULL values in a UNIQUE column (per SQL standard), which
-- would let two global rows for the same provider coexist. Partial indexes solve this.

CREATE TABLE Providers (
    provider_name TEXT NOT NULL,
    manga_id      TEXT REFERENCES Manga(uuid) ON DELETE CASCADE,
    score         INTEGER NOT NULL DEFAULT 0,
    enabled       INTEGER NOT NULL DEFAULT 1
);

-- Exactly one global row per provider
CREATE UNIQUE INDEX providers_global_unique
    ON Providers (provider_name) WHERE manga_id IS NULL;

-- Exactly one series row per (provider, manga) pair
CREATE UNIQUE INDEX providers_series_unique
    ON Providers (provider_name, manga_id) WHERE manga_id IS NOT NULL;
