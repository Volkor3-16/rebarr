-- Rebarr initial schema
-- Documentation lives at `docs/DATABASE.md`, look there for guidance.

CREATE TABLE Library (
    uuid         TEXT PRIMARY KEY,
    library_type TEXT NOT NULL CHECK (library_type IN ('Comics', 'Manga')),
    root_path    TEXT NOT NULL
);

CREATE TABLE Manga (
    uuid                TEXT PRIMARY KEY,
    library_id          TEXT NOT NULL REFERENCES Library(uuid) ON DELETE CASCADE,
    anilist_id          INTEGER,
    mal_id              INTEGER,
    relative_path       TEXT NOT NULL,
    title               TEXT NOT NULL,
    other_titles        TEXT,
    synopsis            TEXT,
    publishing_status   TEXT NOT NULL DEFAULT 'Unknown'
                            CHECK (publishing_status IN (
                                'Completed', 'Ongoing', 'Hiatus',
                                'Cancelled', 'NotYetReleased', 'Unknown'
                            )),
    start_year          INTEGER,
    end_year            INTEGER,
    chapter_count       INTEGER,
    downloaded_count    INTEGER,
    metadata_source     TEXT NOT NULL DEFAULT 'Local'
                            CHECK (metadata_source IN ('AniList', 'Local')),
    thumbnail_url       TEXT,
    created_at          INTEGER,
    metadata_updated_at INTEGER,
    monitored           BOOLEAN NOT NULL DEFAULT 1
);

CREATE TABLE Chapters (
    uuid            TEXT NOT NULL PRIMARY KEY,
    manga_id        TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    chapter_base    INTEGER NOT NULL,
    chapter_variant INTEGER NOT NULL DEFAULT 0,
    is_extra        INTEGER NOT NULL DEFAULT 0,
    title           TEXT,
    language        TEXT    NOT NULL DEFAULT 'EN',
    scanlator_group TEXT,
    provider_name   TEXT,
    chapter_url     TEXT,
    download_status TEXT    NOT NULL DEFAULT 'Missing'
                        CHECK (download_status IN ('Missing', 'Queued', 'Downloading', 'Downloaded', 'Failed')),
    released_at     INTEGER,
    downloaded_at   INTEGER,
    scraped_at      INTEGER
);

CREATE UNIQUE INDEX IF NOT EXISTS uq_chapter_unique
ON Chapters(manga_id, chapter_base, chapter_variant, language, scanlator_group, provider_name);

CREATE INDEX idx_chapter_manga_id ON Chapters(manga_id);

CREATE TABLE CanonicalChapters (
    manga_id           TEXT PRIMARY KEY REFERENCES Manga(uuid) ON DELETE CASCADE,
    canonical_list     TEXT NOT NULL,
    canonical_overrides TEXT,
    last_updated       INTEGER
);


CREATE TABLE IF NOT EXISTS Settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR IGNORE INTO Settings (key, value) VALUES ('scan_interval_hours', '6');
INSERT OR IGNORE INTO Settings (key, value) VALUES ('preferred_language', 'en');
INSERT OR IGNORE INTO Settings (key, value) VALUES ('synonym_filter_languages', 'cmn,vie,rus,kor,tha,spa');


CREATE INDEX idx_manga_library_id ON Manga(library_id);
CREATE INDEX idx_manga_anilist_id ON Manga(anilist_id);

CREATE TABLE MangaTags (
    manga_id TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    tag      TEXT NOT NULL,
    PRIMARY KEY (manga_id, tag)
);

CREATE TABLE Task (
    uuid         TEXT PRIMARY KEY,
    task_type    TEXT NOT NULL CHECK (task_type IN (
                     'ScanLibrary', 'RefreshAniList', 'CheckNewChapter',
                     'DownloadChapter', 'ScanDisk', 'OptimiseChapter', 'Backup'
                 )),
    status       TEXT NOT NULL DEFAULT 'Pending'
                     CHECK (status IN ('Pending', 'Running', 'Completed', 'Failed', 'Cancelled')),

    -- Subject references (nullable; see design notes above)
    library_id   TEXT REFERENCES Library(uuid) ON DELETE CASCADE,
    manga_id     TEXT REFERENCES Manga(uuid)   ON DELETE CASCADE,
    chapter_id   TEXT REFERENCES Chapters(uuid) ON DELETE CASCADE,

    -- Lower number = higher priority (e.g. 0 = critical, 10 = background)
    priority     INTEGER NOT NULL DEFAULT 10,

    -- Overflow payload for future task types or extra parameters
    payload      TEXT,

    -- Retry tracking
    attempt      INTEGER NOT NULL DEFAULT 0,
    max_attempts INTEGER NOT NULL DEFAULT 3,
    last_error   TEXT,

    -- Scheduling (run_after enables delayed retry with backoff)
    created_at   INTEGER,
    updated_at   INTEGER,
    run_after    INTEGER
);

-- Composite index for the worker's polling query:
-- WHERE status = 'Pending' AND run_after <= ? ORDER BY priority ASC, run_after ASC
CREATE INDEX idx_task_worker     ON Task(status, priority, run_after);
CREATE INDEX idx_task_manga_id   ON Task(manga_id);
CREATE INDEX idx_task_chapter_id ON Task(chapter_id);


-- Cache of "this manga lives at this URL on this provider".
-- Avoids re-searching on every sync cycle.
CREATE TABLE IF NOT EXISTS MangaProvider (
    manga_id       TEXT NOT NULL REFERENCES Manga(uuid) ON DELETE CASCADE,
    enabled        INTEGER NOT NULL,
    provider_name  TEXT NOT NULL,
    provider_url   TEXT,
    last_synced_at INTEGER,
    search_attempted_at  INTEGER,
    PRIMARY KEY (manga_id, provider_name)
);
CREATE INDEX IF NOT EXISTS idx_manga_provider_manga_id ON MangaProvider(manga_id);

CREATE TABLE IF NOT EXISTS TrustedGroup (
    name TEXT PRIMARY KEY
);

INSERT OR IGNORE INTO TrustedGroup (name) VALUES
('/A/nimango Scans'),
('/ak/ Scans'),
('Agents of Change'),
('Albedo'),
('Alpha Beta Kappa'),
('Aoitenshi'),
('Aqua Scans'),
('Aquarium Tourmaline'),
('Arite Drop'),
('Atelier du Noi'),
('Pair of 2+'),
('Apollo Team'),
('Arang Scans'),
('Astral Valley Scans'),
('Asura Scans'),
('Black Cat Scans'),
('Blue Sterling Scans'),
('Boredom Society Scans'),
('Chibi Manga'),
('Chillock Scans'),
('Cygnet Scans'),
('Death Toll Scans'),
('Disaster Scans'),
('#Dropout'),
('Edelgarde Scans'),
('Eidetic Memo Scans'),
('#EverydayHeroes Scans'),
('Fallen Angels Scans'),
('Fe Scans'),
('Fire Syndicate'),
('Flame Scans'),
('Flightless Bird Scans'),
('Galaxy Degen Scans'),
('Gemelli Scans'),
('Gourmet Scans'),
('Hachirumi Scans'),
('Harmless Monsters'),
('Hatigarm Scans'),
('Heroz Scans'),
('Hi Wa Mata Noboru'),
('Hokuto No Gun'),
('Hunlight Scans'),
('Immortal Updates'),
('Infrequent Scans'),
('Japanese Unloved Manga'),
('JUM'),
('JoJo''s Colored Adventure Team'),
('Kirei Cake'),
('KS Group'),
('Kyakka Scans'),
('Leviatan Scans'),
('LH Translation'),
('LHTranslation'),
('LHTranslations'),
('Little Miss & Good Sir Scans'),
('LM&GS'),
('Lovesick Alley'),
('lowercase e'),
('Lumine Scans'),
('Lynx Scans'),
('Manga Great'),
('Aloalivn'),
('Manga Sushi'),
('Manga SY'),
('Manhua Plus'),
('Maru Scans'),
('Megchan''s Scanlations'),
('Meraki Scans'),
('Method Scans'),
('Misfits Scans'),
('MMScans'),
('Moe and Friends'),
('Mono Scans'),
('Mystical Merries Scans'),
('NANI? Scans'),
('Not A Scanlation Scans'),
('NASS'),
('Painful Nightz Scans'),
('Peerless Dad Scans'),
('PMScans'),
('Purple Cress'),
('Quick Sand Scans'),
('QSS'),
('Rain of Snow Scans'),
('Ramyun Scans'),
('Random Scans'),
('Reaper Scans'),
('Renascence Scans'),
('Renascans'),
('Reset Scans'),
('Roselia Scanlations'),
('SAWTEAM'),
('Sense Scans'),
('Shadow Madness'),
('Silent Sky Scans'),
('Sleeping Knight'),
('SK'),
('Sleepless Society'),
('Slug Chicks Scans'),
('Speedcat Scans'),
('Spring Palette'),
('Tanooki'),
('TCB Scans'),
('TeruTeru Scans'),
('The Guild'),
('The Nonames Scans'),
('TimelessLeaf'),
('Tonari No Scanlation'),
('TOOR Scans'),
('Tritinia Scans'),
('Tsundere Service Providers'),
('TSP'),
('Twilight Scans'),
('Twisted Hel Scans'),
('Un Team A Caso'),
('Vanguard'),
('Volkan Scans'),
('XuN Scans'),
('Zaibatsu Scans'),
('Zero Scans'),
('BangAQUA'),
('BAP Scans'),
('Beika Street Irregulars'),
('Biamam'),
('Megane Scans'),
('Black Hand Scans'),
('BlastComic Scans'),
('BloodL'),
('Bubble Tea Scans'),
('Café Scanlations'),
('Cafe Scanlations'),
('Cash Money Chiyo'),
('CAT Scans'),
('Champion Scans'),
('ChaosTeam'),
('Cheese Scans'),
('Chibi Manga'),
('Comicola Fansub');
