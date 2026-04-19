CREATE TABLE LibrarySuggestionCandidate (
    library_id              TEXT NOT NULL REFERENCES Library(uuid) ON DELETE CASCADE,
    target_anilist_id       INTEGER NOT NULL,
    title                   TEXT NOT NULL,
    cover_url               TEXT,
    media_format            TEXT,
    publishing_status       TEXT,
    tags_json               TEXT,
    community_rating        INTEGER,
    popularity              INTEGER,
    favourites              INTEGER,
    total_occurrences       INTEGER NOT NULL DEFAULT 0,
    recommendation_occurrences INTEGER NOT NULL DEFAULT 0,
    relation_occurrences    INTEGER NOT NULL DEFAULT 0,
    weighted_score          REAL NOT NULL DEFAULT 0,
    hidden                  INTEGER NOT NULL DEFAULT 0,
    hidden_at               INTEGER,
    refreshed_at            INTEGER NOT NULL,
    PRIMARY KEY (library_id, target_anilist_id)
);

CREATE INDEX idx_library_suggestion_candidate_library
ON LibrarySuggestionCandidate(library_id, hidden, weighted_score DESC, total_occurrences DESC, target_anilist_id ASC);

CREATE TABLE LibrarySuggestionSource (
    id                  INTEGER PRIMARY KEY AUTOINCREMENT,
    library_id          TEXT NOT NULL REFERENCES Library(uuid) ON DELETE CASCADE,
    source_manga_id     TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    target_anilist_id   INTEGER NOT NULL,
    source_kind         TEXT NOT NULL CHECK (source_kind IN ('Recommendation', 'Relation')),
    relation_type       TEXT,
    context             TEXT,
    rating              INTEGER,
    created_at          INTEGER NOT NULL
);

CREATE INDEX idx_library_suggestion_source_library_target
ON LibrarySuggestionSource(library_id, target_anilist_id, source_manga_id);

CREATE INDEX idx_library_suggestion_source_library_source
ON LibrarySuggestionSource(library_id, source_manga_id);

CREATE TABLE Task_new (
    uuid         TEXT PRIMARY KEY,
    task_type    TEXT NOT NULL CHECK (task_type IN (
                     'ScanLibrary', 'BuildFullChapterList', 'RefreshMetadata',
                     'CheckNewChapter', 'SyncProviderChapters', 'DownloadChapter',
                     'ScanDisk', 'OptimiseChapter', 'Backup', 'RefreshSuggestions'
                 )),
    status       TEXT NOT NULL DEFAULT 'Pending'
                     CHECK (status IN ('Pending', 'Running', 'Completed', 'Failed', 'Cancelled')),
    library_id   TEXT REFERENCES Library(uuid) ON DELETE CASCADE,
    manga_id     TEXT REFERENCES Manga(uuid)   ON DELETE CASCADE,
    chapter_id   TEXT REFERENCES Chapters(uuid) ON DELETE CASCADE,
    priority     INTEGER NOT NULL DEFAULT 10,
    payload      TEXT,
    attempt      INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error   TEXT,
    created_at   INTEGER,
    updated_at   INTEGER,
    run_after    INTEGER,
    queue        TEXT NOT NULL DEFAULT 'system'
);

INSERT INTO Task_new SELECT * FROM Task;

DROP TABLE Task;

ALTER TABLE Task_new RENAME TO Task;

CREATE INDEX idx_task_worker          ON Task(status, priority, run_after);
CREATE INDEX idx_task_manga_id        ON Task(manga_id);
CREATE INDEX idx_task_chapter_id      ON Task(chapter_id);
CREATE INDEX idx_task_queue_priority  ON Task(queue, status, priority, run_after);
