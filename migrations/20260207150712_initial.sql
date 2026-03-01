-- =============================================================================
-- Rebarr Initial Schema
-- =============================================================================
-- NOTE: SQLite does not enforce foreign keys by default.
-- Run `PRAGMA foreign_keys = ON;` per-connection in your pool setup.


-- =============================================================================
-- Library
-- Root directories the scanner manages.
-- library_type maps the MangaType enum (Rust field `r#type`).
-- =============================================================================
CREATE TABLE Library (
    uuid         TEXT PRIMARY KEY,
    library_type TEXT NOT NULL CHECK (library_type IN ('Comics', 'Manga')),
    root_path    TEXT NOT NULL
);


-- =============================================================================
-- Manga
-- One row per series. MangaMetadata fields are flattened here directly;
-- metadata columns can be refreshed independently via targeted UPDATE.
-- =============================================================================
CREATE TABLE Manga (
    uuid                TEXT PRIMARY KEY,
    library_id          TEXT NOT NULL REFERENCES Library(uuid) ON DELETE CASCADE,

    -- External IDs
    anilist_id          INTEGER,
    mal_id              INTEGER,

    -- Filesystem
    relative_path       TEXT NOT NULL,

    -- Flattened MangaMetadata fields
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

    -- Counts (chapter_count = AniList-reported value, often inaccurate per code comments)
    chapter_count       INTEGER,
    downloaded_count    INTEGER,

    -- Metadata provenance
    metadata_source     TEXT NOT NULL DEFAULT 'Local'
                            CHECK (metadata_source IN ('AniList', 'Local')),

    -- Thumbnail URL cache for web UI
    thumbnail_url       TEXT,

    -- Timestamps (strftime used instead of CURRENT_TIMESTAMP so SQLx/chrono
    -- can parse the ISO 8601 T/Z format directly)
    created_at          TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    metadata_updated_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

CREATE INDEX idx_manga_library_id ON Manga(library_id);
CREATE INDEX idx_manga_anilist_id ON Manga(anilist_id);
CREATE INDEX idx_manga_mal_id     ON Manga(mal_id);


-- =============================================================================
-- MangaTag
-- Normalized storage for MangaMetadata.tags: Vec<String>.
-- Composite PK prevents duplicate tags per manga.
-- =============================================================================
CREATE TABLE MangaTag (
    manga_id TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (manga_id, tag)
);


-- =============================================================================
-- Chapter
-- Logical chapter records only — provider chapter data (URLs, scrape info)
-- is transient and resolved at runtime, never persisted.
-- =============================================================================
CREATE TABLE Chapter (
    uuid            TEXT PRIMARY KEY,
    manga_id        TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,

    -- Chapter identity
    number_raw      TEXT NOT NULL,  -- raw string from provider (e.g. "12.5")
    number_sort     REAL NOT NULL,  -- parsed float for ordering
    title           TEXT,
    volume          INTEGER,
    scanlator_group TEXT,

    -- Download lifecycle
    download_status TEXT NOT NULL DEFAULT 'Missing'
                        CHECK (download_status IN ('Missing', 'Downloading', 'Downloaded', 'Failed')),
    downloaded_at   TEXT            -- set when download_status = 'Downloaded'
);

CREATE INDEX idx_chapter_manga_id          ON Chapter(manga_id);
CREATE INDEX idx_chapter_manga_number_sort ON Chapter(manga_id, number_sort);


-- =============================================================================
-- MangaAlias
-- Alternate titles for a manga from various sources (MangaAlias struct).
-- AliasSource::Site(String) uses alias_source_site for the payload;
-- a CHECK constraint enforces it is set iff alias_source = 'Site'.
-- =============================================================================
CREATE TABLE MangaAlias (
    manga_id          TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    alias_source      TEXT NOT NULL CHECK (alias_source IN ('MyAnimeList', 'Site', 'Manual')),
    alias_source_site TEXT,         -- site name; only when alias_source = 'Site'
    title             TEXT NOT NULL,

    CHECK (
        (alias_source = 'Site'  AND alias_source_site IS NOT NULL)
        OR
        (alias_source != 'Site' AND alias_source_site IS NULL)
    ),

    PRIMARY KEY (manga_id, alias_source, alias_source_site, title)
);

CREATE INDEX idx_manga_alias_manga_id ON MangaAlias(manga_id);


-- =============================================================================
-- Task
-- Background task queue for the worker system (TaskType enum).
-- Subject FKs are nullable because different tasks target different subjects:
--   ScanLibrary     -> library_id set
--   RefreshAniList  -> manga_id set
--   CheckNewChapter -> manga_id set
--   DownloadChapter -> manga_id + chapter_id set
--   Backup          -> all may be NULL
-- =============================================================================
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
    chapter_id   TEXT REFERENCES Chapter(uuid) ON DELETE CASCADE,

    -- Lower number = higher priority (e.g. 0 = critical, 10 = background)
    priority     INTEGER NOT NULL DEFAULT 10,

    -- Overflow payload for future task types or extra parameters
    payload      TEXT,

    -- Retry tracking
    attempt      INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error   TEXT,

    -- Scheduling (run_after enables delayed retry with backoff)
    created_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    updated_at   TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now')),
    run_after    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ', 'now'))
);

-- Composite index for the worker's polling query:
-- WHERE status = 'Pending' AND run_after <= ? ORDER BY priority ASC, run_after ASC
CREATE INDEX idx_task_worker     ON Task(status, priority, run_after);
CREATE INDEX idx_task_manga_id   ON Task(manga_id);
CREATE INDEX idx_task_chapter_id ON Task(chapter_id);
