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
    - [x] **DB: `MangaProvider` table** (new migration)
        - Stores `(manga_id, provider_name, provider_url, last_synced_at)`
        - Cache of "this manga lives at this URL on this provider"
        - Avoids re-searching on every sync
    - [x] **DB: Chapter layer** (`src/db/chapter.rs`)
        - Add `download_status` to `Chapter` struct (column exists in DB, missing from Rust type)
        - `insert` / `upsert_from_scrape` / `get_all_for_manga` / `set_status`
    - [x] **Scan + merge flow** (`src/scraper/merge.rs` or similar)
        - For each provider (by score): search for manga URL if not in `MangaProvider`
        - Title matching: try all `MangaAlias` titles; auto-accept if score is high enough
        - For each cached provider URL: scrape chapter list
        - Upsert chapters into DB — new ones get `Missing`; already-`Downloaded` untouched
        - Update `chapter_count` (highest number seen) and `downloaded_count` on `Manga`
    - [x] **API routes** (wired into `src/api.rs`, use managed `ProviderRegistry` + `ScraperCtx`)
        - `GET  /api/providers` — list providers (name, score, needs_browser)
        - `POST /api/manga/<id>/scan` — trigger scan + merge for one manga
        - `POST /api/manga/<id>/provider` — manually set or override provider URL
        - `GET  /api/manga/<id>/chapters` — list chapters with download_status
        - `POST /api/manga/<id>/chapters/<num>/download` — queue or directly download
    - [x] **Download pipeline** (`src/downloader.rs`)
        - Given manga + chapter: scrape pages from provider → download images → save to disk
        - Output format: CBZ at `<library_root>/<manga_path>/Chapter <num>.cbz`
        - Mark chapter `Downloaded`, bump `downloaded_count` on `Manga`
        - Provider fallback: on failure try next provider by score
    - [x] **Task queue worker** (background tokio task)
        - Poll `Task` table for `Pending` tasks (table already in DB)
        - Per-provider rate limiting using `rate_limit` from YAML `ProviderDef`
        - Task types: `ScanLibrary`, `CheckNewChapter`, `DownloadChapter`
        - Searching/scanning tasks take priority over background downloads
    - [ ] Integrate with the app itself
        - [x] Task worker enqueues a scan on `POST /api/manga/<id>/scan`
        - [ ] Auto-scan when adding manga to library (currently manual via scan endpoint)
        - [ ] Prompt user if provider match score is below threshold (currently auto-accepts at ≥0.85)
        - [ ] 'Monitor' toggle on manga (auto-download missing chapters on next scheduled scan)
        - [ ] Show a list of chapters in UI (API ready: `GET /api/manga/<id>/chapters`, UI not wired)
        - [ ] Allow the user to queue downloads from UI (API ready: `POST /api/manga/<id>/chapters/<num>/download`, no buttons)
        - [ ] Manual download selection:
            - Show all providers that have a chapter (scanlator/image stats)
            - Allow user to pick provider per chapter
            - For whole-series: prioritise providers with full collections

## Features

### Minimum Viable Release

- [ ] Metadata API
    - [x] AniList Support
    - [ ] Manual Entry support (fallback)
- [ ] Library Support
    - [x] Multiple Libraries
    - [x] Download coverphoto/thumbnail when adding/importing to library (saved to `./thumbnails/` — TODO: move to series folder)
    - [ ] Ability to scan for existing series / chapters (detect existing CBZ files on disk)
    - [ ] Ability to manage existing series / chapters
    - [ ] Ability to optimise existing chapters (convert to lossless webp)
    - [ ] Generate a standalone `ComicInfo.xml` on add/import (CBZ creation writes a stub, but no standalone import step yet)
- [ ] Downloading Chapters
    - [x] Basic Provider implementation (CSS/JSON declarative extraction, normal HTTP)
    - [x] Basic Scraper testing (`src/bin/scraper_test.rs`)
        - [ ] Use and expand on this as we add providers
            - Ideally this could be spun into a cli version of the downloader.
    - [x] Queued Downloads
        - [x] Task/queue system implementation (`src/worker.rs`, `src/db/task.rs`)
        - [x] Separate queue for each provider (because rate limits are site specific :p)
        - [x] Doesn't hit rate limits (`rate_limit_rpm` from YAML enforced in worker)
        - [x] Automatic retry with backoffs on download failure (exponential: 2^attempt minutes, max 3 attempts)
        - [x] Priority system (Searching > Manual Downloads > Automatic Downloads) (thanks tranga for not doing this and pissing me off enough to start this project)
    - [x] Advanced Provider implementation
        - [x] Fancy lua scripting & JS Handling inside chromium instance (mlua 0.10 + chromiumoxide 0.9)
    - [ ] flaresolverr support
    - [ ] Configurable Workflows for automatically optimising chapters (lossless webp conversion)
- [x] REST API 
- [x] a bad looking webui frontend
    - [x] Create and view libraries
    - [x] Search for manga
    - [x] Add Manga from search
    - [x] View Manga
    - [x] Download Manga
    - [x] Change Library Settings
    - [ ] Manually Add Manga
    - [ ] live updating task status and history
    - [ ] View Logs

### Maximum Viable Release (in order of importance)

This is in addition to the above.

- [ ] Metadata API
    - [ ] MyAnimeList Support (mal_api crate works)
    - [ ] MangaUpdates Support (need to make a crate, or use the worst fucking openapi generated thing ever)
    - [ ] Any other sites can be listed here. It's good to not be stuck with a single metadata service.
- [ ] Storage Backends
    - [ ] S3 Storage?
    - I'm not sure what else we'd need.
- [ ] A nice looking webui
    - [ ] Basic Auth
    - [ ] oauth/oidc/whatever fancy system (would link with komga emulation)
- [ ] Import workflows
    - [ ] Losslessly convert pages to webp/whatever (uses https://lib.rs/crates/compress_comics)
    - [ ] Detect watermarks (and remove them?)
    - [ ] Detect Low Quality images
    - [ ] Detect and remove scanlator pages where they have 4 pages of random fucking memes seriously just have one at most.
- [ ] Work with non-manga comics?
- [ ] Komga server 'emulation' (for mihon/tachiyomi extensions)
    - [ ] User system
    - [ ] read-list
    - [ ] Scrobbling to mal? (do we need this? most programs already have some form of support... BUT this would be better since automatic matching with mal ids we already use)
- [ ] Notification System
    - Webhooks on events
        - Download completed
        - Download failures
        - Other problems
- [ ] Metrics (because i love grafana graphs)

## Installation

i got nothing here atm lol

but once it's done, just copy the docker compose and change the stuff to what makes sense for ur setup


### Dev Install

Requires rust/cargo and whatever else i add

1. `cargo run --bin rebarr`

for testing a provider, use

`cargo run --bin scraper_test -- "mangasearchterm"`


## Thanks

- All the other companies that also don't realise that.
- My mates claude subscription that i'm borrowing lmfao.


## Copyright Holders

If you are a copyright holder and don't like this, perhaps you should have thought about that before you and ur cunty mates decided to shut down the only place (legal or illegal) where you can find your content. Making it harder for people to find out about your content isn't a good way to make money. (oh and pissing off everyone in the community).

Sure, you can shut me down and get me to stop my work, but honestly this software isn't even that good, and you're not going to **stop** piracy.

### The fuck you list

This is a bit of a old-man-screams-at-cloud thing, but it's nice to vent :)

- Kakao Entertainment.  
  Thanks for killing Tachiyomi you cunts, you guys gonna try and get firefox shut down for enabling access to piracy sites?  
  It's really cool how you exist to leech money from the artists, and then you cancel their series and their revenue sharing. ***"At this point, I’d rather you pirate Sound of Bread than give Tapas a single cent."*** - author of Sound of Bread  
  It's also really cool how you force artists to work until they miscarry. total normal ethical behaviour.  