-- Add ComicInfo fields to Manga table
-- This migration adds support for storing all ComicInfo metadata fields in the database

-- Add new columns for ComicInfo fields
ALTER TABLE Manga ADD COLUMN writer TEXT;
ALTER TABLE Manga ADD COLUMN penciller TEXT;
ALTER TABLE Manga ADD COLUMN inker TEXT;
ALTER TABLE Manga ADD COLUMN colorist TEXT;
ALTER TABLE Manga ADD COLUMN letterer TEXT;
ALTER TABLE Manga ADD COLUMN editor TEXT;
ALTER TABLE Manga ADD COLUMN translator TEXT;
ALTER TABLE Manga ADD COLUMN genre TEXT;
ALTER TABLE Manga ADD COLUMN community_rating INTEGER;
ALTER TABLE Manga ADD COLUMN start_month INTEGER;
ALTER TABLE Manga ADD COLUMN start_day INTEGER;

-- Note: existing other_titles column remains as JSON for backward compatibility
-- The Manga struct now handles both old JSON format and new Synonym format