# Rebarr

Rebarr is a Manga downloader.... made for selfhosting nerds.

I never liked how **all** the other manga scrapers work. They all use the scraped site as an authoratitive source for chapter naming, and metadata. This fucked up my workflow with suwayomi when the site changes the name, and it creates a new series in komga.


The plan is to use AniList as the metadata source, and to automatically* match manga across different sites. Sorta like how sonarr works with multiple indexers.

## Bugs / Dev TODO

- [ ] Providers list under chapter list should provide more info
    - Providers that couldn't find anything
    - Provider score breakdowns
    - Direct provider links/metadata to help with matching
- [ ] Better scoring
    - We shouldn't really care which provider it's uploaded to. No 'provider wide scores'
    - We should change it to work on tiers. Tiers are the strict order, we never select a lower source if a better one exists.
        - Tier 1: Official Publisher
        - Tier 2: Known scanlator group (from a 'trusted' community list or something, idk)
        - Tier 3: Unknown scnalator group (for ones that have a group name but we don't know, or just bad naming from a provider)
        - Tier 4: No Group / unlabelled
    - Additionally, the metadata should be ranked too, as to help scoring within a tier. Really here for a tiebreaker.
        - Score presence of:
            - title
            - scanlator group name
            - upload date
            - chapter number (2 v s 2.1 and 2.2)
            - Volume number
        - Combined into a normalised score
            - Then added to the tier scoring.
- [ ] Partial/Split Chapters handling
    - [ ] Split chapter numbers (currently number_sort) into `chapter_base` and `chapter_variant`
        - Somehow handle 2.5 'extras' variant differently from split chapters
        - [ ] Group variants under a chapter_base
        - [ ] Assign a score bonus to full chapters, than a partial/split chapter collection.
        - [ ] Normalise other naming schemes (2a -> 2.1)


## Features
### Minimum Viable Release

- [x] Metadata API
    - [x] AniList Support
    - [x] AniList Refresh (re-fetch metadata on demand)
    - [x] Manual Entry support (fallback)
- [x] Library Support
    - [x] Multiple Libraries
    - [x] Download coverphoto/thumbnail when adding/importing to library
    - [x] Ability to scan for existing series / chapters (detect existing CBZ files on disk)
    - [x] Ability to manage existing series / chapters (Mark as Downloaded, Re-download)
    - [x] Ability to optimise existing chapters (convert to webp)
    - [x] Generate a standalone `ComicInfo.xml` on add/import (CBZ creation writes a stub, but no standalone import step yet)
- [x] Downloading Chapters
    - [x] Basic Provider implementation (CSS/JSON declarative extraction, normal HTTP)
    - [x] Basic Scraper testing (`src/bin/scraper_test.rs`)
        - [x] Use and expand on this as we add providers
            - Ideally this could be spun into a cli version of the downloader.
    - [x] Queued Downloads
        - [x] Task/queue system implementation (`src/worker.rs`, `src/db/task.rs`)
        - [x] Separate queue for each provider (because rate limits are site specific :p)
        - [x] Doesn't hit rate limits (`rate_limit_rpm` from YAML enforced in worker)
        - [x] Automatic retry with backoffs on download failure (exponential: 2^attempt minutes, max 3 attempts)
        - [x] Priority system (Searching > Manual Downloads > Automatic Downloads) (thanks tranga for not doing this and pissing me off enough to start this project)
    - [x] Advanced Provider implementation
    - [x] Configurable Workflows for automatically optimising chapters (webp conversion via OptimiseChapter task)
- [x] REST API
- [x] a bad looking webui frontend
    - [x] Create and view libraries
    - [x] Search for manga
    - [x] Add Manga from search
    - [x] View Manga
    - [x] Download Manga
    - [x] Change Library Settings
    - [x] Manually Add Manga
    - [x] live updating task status and history
    - [x] Queue page (task history + active queue, cancel tasks)
    - [x] Bulk/selective chapter download (checkboxes + "Download All Missing")
    - [x] Monitored toggle per series
    - [x] Scan interval setting (Settings page)
- [x] Queue system
    - [x] Tasks run when needed (auto-scan on manga add)
    - [x] Monitored series — new chapters auto-downloaded on subsequent scans
    - [x] Tasks run automatically on a schedule (configurable interval, default 6h)
        - [x] CheckNewChapter enqueued for all monitored series
    - [x] Task run automatically on trigger
        - New series added:
            - [x] ScanLibrary auto-enqueued on add
        - BuildChapterList Completed
            - [x] ScoreProviders scheduled
    - [x] Stuck "Running" tasks reset to Pending on startup
    - [x] Chapter URLs cached after first scan (downloads skip re-scraping the chapter list)
    - [x] Cancel Pending/Running tasks from the Queue page
- [ ] Content Matching system
    - 
- [ ] New Database/new user wizard
    1. Ask to create a library directory (or skip if already set in env)
    2. Ask user to select enabled/disabled providers
    3. Ask user to select default monitor status (do we auto-download newly released chapters?)
    4. Ask user to select download priority ordering (default Official > Named Scanlation Group > Unknown > Aggregator reupload)
    3. Ask user select directoy full of old manga
        - Match and import into DB.
        - Moves, renames, matches files to chapters - exactly like sonarr bulk import
        - Do this for each manga series, let user match and verify if it doesn't match automatically.

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