# Rebarr

Rebarr is a Manga downloader.... made for selfhosting nerds.

I never liked how **all** the other manga scrapers work. They all use the scraped site as an authoratitive source for chapter naming, and metadata. This fucked up my workflow with suwayomi when the site changes the name, and it creates a new series in komga.


The plan is to use AniList as the metadata source, and to automatically* match manga across different sites. Sorta like how sonarr works with multiple indexers.

## Dev TODO

- [x] Setup base structure
- [ ] Get AniList API working
    - [x] Generate OpenAPI Spec code thing
    - [x] Figure out how to use it
    - [x] Use it
- [x] Convert Anilist response into manga struct
    - To do this, i need to have a way for the user to 'select' one.
    - So I think we just hard-code one for now to get the code working, and then have the selection later on.
- [x] DB Stuffs
- [x] Web API
- [ ] Scraper integration
    - [x] YAML provider config system (`src/scraper/`)
    - [x] Provider trait: search, chapters, pages
    - [x] Headless browser pool (chromiumoxide) for JS-heavy sites
    - [x] `scraper_test` binary — proves providers work end-to-end
    - [ ] **DB: `MangaProvider` table** (new migration)
        - Stores `(manga_id, provider_name, provider_url, last_synced_at)`
        - Cache of "this manga lives at this URL on this provider"
        - Avoids re-searching on every sync
    - [ ] **DB: Chapter layer** (`src/db/chapter.rs`)
        - Add `download_status` to `Chapter` struct (column exists in DB, missing from Rust type)
        - `insert` / `upsert_from_scrape` / `get_all_for_manga` / `set_status`
    - [ ] **Scan + merge flow** (`src/scraper/merge.rs` or similar)
        - For each provider (by score): search for manga URL if not in `MangaProvider`
        - Title matching: try all `MangaAlias` titles; auto-accept if score is high enough
        - For each cached provider URL: scrape chapter list
        - Upsert chapters into DB — new ones get `Missing`; already-`Downloaded` untouched
        - Update `chapter_count` (highest number seen) and `downloaded_count` on `Manga`
    - [ ] **API routes** (wired into `src/api.rs`, use managed `ProviderRegistry` + `ScraperCtx`)
        - `GET  /api/providers` — list providers (name, score, needs_browser)
        - `POST /api/manga/<id>/scan` — trigger scan + merge for one manga
        - `POST /api/manga/<id>/provider` — manually set or override provider URL
        - `GET  /api/manga/<id>/chapters` — list chapters with download_status
        - `POST /api/manga/<id>/chapters/<num>/download` — queue or directly download
    - [ ] **Download pipeline** (`src/downloader.rs`)
        - Given manga + chapter: scrape pages from provider → download images → save to disk
        - Output format: CBZ at `<library_root>/<manga_path>/Chapter <num>.cbz`
        - Mark chapter `Downloaded`, bump `downloaded_count` on `Manga`
        - Provider fallback: on failure try next provider by score
    - [ ] **Task queue worker** (background tokio task)
        - Poll `Task` table for `Pending` tasks (table already in DB)
        - Per-provider rate limiting using `rate_limit` from YAML `ProviderDef`
        - Task types: `ScanLibrary`, `CheckNewChapter`, `DownloadChapter`
        - Searching/scanning tasks take priority over background downloads

## Features

### Minimum Viable Release

- [x] AniList api metadata
    - [ ] Manual user entered fallback for manga not on AniList.
- [ ] Independant indexer support
- [ ] Komga `ComicInfo.xml` creation
- [ ] Queue system for scraping
    - [ ] Separate queue for each scraped site
    - [ ] Searching sites should take priority over background downloads (thanks tranga for not doing this and pissing me off enough to start this project)
- [ ] a bad looking webui

### Maximum Viable Release (in order of importance)

- [ ] Scriptable scraper (People shouldn't need to learn rust to add new sites)
- [ ] A nice looking webui
- [ ] Losslessly convert images to webp/avif/whatever
- [ ] Work with non-manga comics?
- [ ] Komga server 'emulation' (for mihon/tachiyomi extensions)
    - [ ] User system
    - [ ] read-list
    - [ ] Scrobbling to mal? (do we need this? most programs already have some form of support... BUT this would be better since automatic matching with mal ids we already use)


## Installation

i got nothing here atm lol

but once it's done, just copy the docker compose and change the stuff to what makes sense for ur setup


### Dev Install

Requires rust/cargo and whatever else i add

1. `cargo run --release`

thats it :)


## Thanks

- AniList for their nice API and database, even if they don't have a nicely updated 'chatper count' entry.
- Kakao Entertainment, for ruining the entire scanlation community enough to warrant me making this. You do realise shutting down managa/manwha sites that's not available for sale traditionally is a failing on **your** part right? You're making it harder for people to even find out about your content you're trying to sell.
- All the other companies that also don't realise that.


Please do note that this program was NOT entirely vibecoded, but I did use a little bit to help plan and implement some of the basics. It's a nice tool for rubber duck debugging too, and just to clarify, all LLM Generation was done on a Local Machine (lie), entirely powered with solar.
