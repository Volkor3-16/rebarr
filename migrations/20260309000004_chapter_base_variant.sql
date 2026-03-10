-- Split chapter number into base + variant for proper grouping of split/extra chapters.
-- chapter_base: the main chapter number (e.g. 12 for "12", "12.5", "12a")
-- chapter_variant: sub-part index (0 = full chapter, 1..4 = split parts a/b/c/d or .1/.2,
--                  5+ = extra/bonus chapters like .5)
-- is_extra: true if this is a bonus/extra chapter (heuristic: decimal >= 0.5)
ALTER TABLE Chapter ADD COLUMN chapter_base REAL NOT NULL DEFAULT 0;
ALTER TABLE Chapter ADD COLUMN chapter_variant INTEGER NOT NULL DEFAULT 0;
ALTER TABLE Chapter ADD COLUMN is_extra INTEGER NOT NULL DEFAULT 0;  -- SQLite bool as 0/1

-- Backfill existing rows from number_sort.
-- chapter_base = integer part; chapter_variant = round(fractional * 10); is_extra = frac >= 0.5
UPDATE Chapter SET
    chapter_base    = CAST(number_sort AS INTEGER),
    chapter_variant = CAST(ROUND((number_sort - CAST(number_sort AS INTEGER)) * 10) AS INTEGER),
    is_extra        = CASE WHEN (number_sort - CAST(number_sort AS INTEGER)) >= 0.5 THEN 1 ELSE 0 END;
