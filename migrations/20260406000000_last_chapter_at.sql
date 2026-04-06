-- Add last_chapter_at column to Manga table
ALTER TABLE Manga ADD COLUMN last_chapter_at INTEGER;

-- Backfill existing data
UPDATE Manga 
SET last_chapter_at = (
    SELECT MAX(MAX(downloaded_at, released_at, scraped_at))
    FROM Chapters
    WHERE Chapters.manga_id = Manga.uuid
);