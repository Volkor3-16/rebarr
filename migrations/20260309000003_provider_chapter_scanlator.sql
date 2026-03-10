-- Add scanlator_group to ProviderChapterUrl so tier scoring can use per-chapter group info.
ALTER TABLE ProviderChapterUrl ADD COLUMN scanlator_group TEXT;
