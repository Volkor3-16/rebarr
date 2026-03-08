-- Deduplicate existing Chapter rows: for each (manga_id, number_sort) keep one row,
-- preferring Downloaded status, then smallest uuid as tiebreaker.
DELETE FROM Chapter WHERE rowid NOT IN (
    SELECT rowid FROM (
        SELECT rowid,
               ROW_NUMBER() OVER (
                   PARTITION BY manga_id, number_sort
                   ORDER BY CASE download_status WHEN 'Downloaded' THEN 0 ELSE 1 END, uuid
               ) AS rn
        FROM Chapter
    ) WHERE rn = 1
);

-- Enforce one logical chapter per (manga, number) going forward.
-- INSERT OR IGNORE in upsert_from_scrape will now correctly skip duplicates.
CREATE UNIQUE INDEX uq_chapter_manga_number ON Chapter(manga_id, number_sort);
