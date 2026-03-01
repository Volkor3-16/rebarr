-- The place the scanner should look for manga by default.
CREATE TABLE Library (
    uuid TEXT PRIMARY KEY,
    root_path TEXT NOT NULL,
)

-- All the manga in the library
CREATE TABLE Manga (
    uuid TEXT PRIMARY KEY,
    anilist_id TEXT,
    mal_id TEXT,
    path TEXT,
    title TEXT,
    title_og TEXT,
    title_roman TEXT,
    synopsis TEXT,
    status TEXT,
    chapter_count INTEGER, -- We're gonna need to fill this in later, anilist isn't accurate.
    source TEXT,
    thumbnail_url TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    downloaded_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
)

-- Chapter Table
    -- uuid TEXT PRIMARY KEY,
