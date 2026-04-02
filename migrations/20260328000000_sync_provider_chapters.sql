-- Rename CheckNewChapter → SyncProviderChapters in the Task table CHECK constraint.
-- SQLite doesn't support ALTER TABLE ... DROP/ADD CONSTRAINT, so we recreate the table.

-- Step 1: Create new table with updated CHECK constraint
CREATE TABLE Task_new (
    uuid         TEXT PRIMARY KEY,
    task_type    TEXT NOT NULL CHECK (task_type IN (
                     'ScanLibrary', 'BuildFullChapterList', 'RefreshMetadata',
                     'CheckNewChapter', 'SyncProviderChapters', 'DownloadChapter',
                     'ScanDisk', 'OptimiseChapter', 'Backup'
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

-- Step 2: Copy data
INSERT INTO Task_new SELECT * FROM Task;

-- Step 3: Drop old table
DROP TABLE Task;

-- Step 4: Rename new table
ALTER TABLE Task_new RENAME TO Task;

-- Step 5: Recreate indexes
CREATE INDEX idx_task_worker          ON Task(status, priority, run_after);
CREATE INDEX idx_task_manga_id        ON Task(manga_id);
CREATE INDEX idx_task_chapter_id      ON Task(chapter_id);
CREATE INDEX idx_task_queue_priority  ON Task(queue, status, priority, run_after);