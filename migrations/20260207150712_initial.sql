-- Rebarr initial schema
-- Documentation lives at `docs/DATABASE.md`, look there for guidance.

CREATE TABLE Library (
    uuid         TEXT PRIMARY KEY,
    library_type TEXT NOT NULL CHECK (library_type IN ('Comics', 'Manga')),
    root_path    TEXT NOT NULL
);

CREATE TABLE Manga (
    uuid                TEXT PRIMARY KEY,
    library_id          TEXT NOT NULL REFERENCES Library(uuid) ON DELETE CASCADE,
    anilist_id          INTEGER,
    mal_id              INTEGER,
    relative_path       TEXT NOT NULL,
    title               TEXT NOT NULL,
    title_og            TEXT NOT NULL DEFAULT '',
    title_roman         TEXT NOT NULL DEFAULT '',
    synopsis            TEXT,
    publishing_status   TEXT NOT NULL DEFAULT 'Unknown'
                            CHECK (publishing_status IN (
                                'Completed', 'Ongoing', 'Hiatus',
                                'Cancelled', 'NotYetReleased', 'Unknown'
                            )),
    start_year          INTEGER,
    end_year            INTEGER,
    chapter_count       INTEGER,
    downloaded_count    INTEGER,
    metadata_source     TEXT NOT NULL DEFAULT 'Local'
                            CHECK (metadata_source IN ('AniList', 'Local')),
    thumbnail_url       TEXT,
    created_at          INTEGER,
    metadata_updated_at INTEGER,
    monitored           BOOLEAN NOT NULL DEFAULT 1
);

CREATE TABLE Chapters (
    uuid TEXT PRIMARY KEY,
    manga_id TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    chapter_base INTEGER NOT NULL,
    chapter_variant INTEGER NOT NULL DEFAULT 0,
    title TEXT,
    language TEXT NOT NULL DEFAULT 'EN',
    scanlator_group TEXT,
    provider_name TEXT,
    chapter_url TEXT,
    download_status TEXT NOT NULL DEFAULT 'Missing'
                        CHECK (download_status IN ('Missing', 'Downloading', 'Downloaded', 'Failed')),
    released_at INTEGER,
    downloaded_at INTEGER,
    scraped_at INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_chapter_unique
ON Chapters(manga_id, chapter_base, chapter_variant, language, scanlator_group, provider_name);

CREATE INDEX idx_chapter_manga_id ON Chapters(manga_id);

CREATE TABLE CanonicalChapters (
    manga_id TEXT PRIMARY KEY REFERENCES Manga(uuid) ON DELETE CASCADE,
    canonical_list TEXT NOT NULL,
    last_updated INTEGER
);


CREATE TABLE IF NOT EXISTS Settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO Settings (key, value) VALUES ('scan_interval_hours', '6');
INSERT OR IGNORE INTO Settings (key, value) VALUES ('preferred_language', 'en');


CREATE INDEX idx_manga_library_id ON Manga(library_id);
CREATE INDEX idx_manga_anilist_id ON Manga(anilist_id);

CREATE TABLE MangaTags (
    manga_id TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (manga_id, tag)
);

CREATE TABLE Task (
    uuid         TEXT PRIMARY KEY,
    task_type    TEXT NOT NULL CHECK (task_type IN (
                     'ScanLibrary', 'RefreshAniList', 'CheckNewChapter',
                     'DownloadChapter', 'Backup'
                 )),
    status       TEXT NOT NULL DEFAULT 'Pending'
                     CHECK (status IN ('Pending', 'Running', 'Completed', 'Failed', 'Cancelled')),

    -- Subject references (nullable; see design notes above)
    library_id   TEXT REFERENCES Library(uuid) ON DELETE CASCADE,
    manga_id     TEXT REFERENCES Manga(uuid)   ON DELETE CASCADE,
    chapter_id   TEXT REFERENCES Chapters(uuid) ON DELETE CASCADE,

    -- Lower number = higher priority (e.g. 0 = critical, 10 = background)
    priority     INTEGER NOT NULL DEFAULT 10,

    -- Overflow payload for future task types or extra parameters
    payload      TEXT,

    -- Retry tracking
    attempt      INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error   TEXT,

    -- Scheduling (run_after enables delayed retry with backoff)
    created_at   INTEGER,
    updated_at   INTEGER,
    run_after    INTEGER
);

-- Composite index for the worker's polling query:
-- WHERE status = 'Pending' AND run_after <= ? ORDER BY priority ASC, run_after ASC
CREATE INDEX idx_task_worker     ON Task(status, priority, run_after);
CREATE INDEX idx_task_manga_id   ON Task(manga_id);
CREATE INDEX idx_task_chapter_id ON Task(chapter_id);


-- Cache of "this manga lives at this URL on this provider".
-- Avoids re-searching on every sync cycle.
CREATE TABLE IF NOT EXISTS MangaProvider (
    manga_id       TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    enabled        INTEGER NOT NULL,
    provider_name  TEXT NOT NULL,
    provider_url   TEXT,
    last_synced_at INTEGER,
    search_attempted_at  INTEGER,
    PRIMARY KEY (manga_id, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_manga_provider_manga_id ON MangaProvider(manga_id);
