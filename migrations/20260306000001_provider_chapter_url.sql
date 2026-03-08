-- Cache chapter page URLs per provider so downloads don't need to re-scrape the full chapter list.
CREATE TABLE IF NOT EXISTS ProviderChapterUrl (
    manga_id             TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    provider_name        TEXT NOT NULL,
    chapter_number_sort  REAL NOT NULL,
    chapter_url          TEXT NOT NULL,
    PRIMARY KEY (manga_id, provider_name, chapter_number_sort)
);
