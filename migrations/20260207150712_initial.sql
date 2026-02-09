-- All the manga in the library
CREATE TABLE Manga (
    uuid TEXT PRIMARY KEY,
    mal_id UNIQUE,
    title TEXT NOT NULL,
    synopsis TEXT,
    status TEXT NOT NULL,
    chapter_count INTEGER, -- We're gonna need to fill this in later, mal isn't accurate.
    cover_image_url TEXT,
    created_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP DEFAULT CURRENT_TIMESTAMP
)

-- Chapter Table
    -- uuid TEXT PRIMARY KEY,
