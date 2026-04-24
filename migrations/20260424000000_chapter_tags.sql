CREATE TABLE ChapterTags (
    chapter_id  TEXT NOT NULL REFERENCES Chapters(uuid) ON DELETE CASCADE,
    tag         TEXT NOT NULL,
    created_at  INTEGER NOT NULL DEFAULT (unixepoch()),
    PRIMARY KEY (chapter_id, tag)
);

CREATE INDEX idx_chapter_tags_chapter ON ChapterTags(chapter_id);
