-- User-pinned provider for a specific chapter.
-- When set, downloads always use this provider instead of auto-selecting by tier.
ALTER TABLE Chapter ADD COLUMN preferred_provider TEXT;
