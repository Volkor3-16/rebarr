-- Rename AliasSource::MyAnimeList -> AliasSource::AniList in the MangaAlias table.
-- SQLite cannot ALTER a CHECK constraint in place, so we recreate the table.

CREATE TABLE MangaAlias_new (
    manga_id          TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    alias_source      TEXT NOT NULL CHECK (alias_source IN ('AniList', 'Site', 'Manual')),
    alias_source_site TEXT,
    title             TEXT NOT NULL,

    CHECK (
        (alias_source = 'Site'  AND alias_source_site IS NOT NULL)
        OR
        (alias_source != 'Site' AND alias_source_site IS NULL)
    ),

    PRIMARY KEY (manga_id, alias_source, alias_source_site, title)
);

INSERT INTO MangaAlias_new
SELECT
    manga_id,
    CASE WHEN alias_source = 'MyAnimeList' THEN 'AniList' ELSE alias_source END,
    alias_source_site,
    title
FROM MangaAlias;

DROP TABLE MangaAlias;
ALTER TABLE MangaAlias_new RENAME TO MangaAlias;

CREATE INDEX idx_manga_alias_manga_id ON MangaAlias(manga_id);
