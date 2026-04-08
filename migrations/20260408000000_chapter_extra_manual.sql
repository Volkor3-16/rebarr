-- Add is_extra_manual to distinguish user-set extras from auto-classified ones.
-- NULL = auto-determined by scanner
-- 0   = user explicitly said NOT extra (survives future scans)
-- 1   = user explicitly said IS extra (survives future scans)
ALTER TABLE Chapters ADD COLUMN is_extra_manual INTEGER;
