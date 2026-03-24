# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such.

Rebarr is a sonarr-like manager and scraper for manga and comics.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Bugs & TODO

I'll remove this when I've got the first public release out, this is just a quick reference for me to see what I need to work on.

### Backend
- Full test of new chapter refresh -> chapter downloading
    - 2026-03-20: It refreshed automatically when my pc came out of sleep! noice!
    - 2026-03-23: I've chucked it on the server, i'll import a fuckload of manga for it to try.
- We should save the reason why we upgrade a chapter (so we can debug bad upgrades easily)

### Providers

I think we need to rework how pages are downloaded:
- Downloads report status by page (to show progress in frontend)
- We shouldn't let the browser disconnect and crash? Do we start a new browser session on each task, and close it during completion?
- Providers steps shouldn't need a random ass `- open` and then hit another endpoint why have the open step at all?

- [ ] Get Mangakakalot working
- [ ] Get Mangago working
- [ ] Get AllManga Working

## Features

- You can add series to your library and download them (obviously)
- We automatically look on **all**(available) sites and compare what they have, downloading only the best copy.
- Downloaded chapters save all the metadata inside them, so if my awful code breaks something, you can rebuild (most of) the database from that. Also, any manga you share with another rebarr installation will have its metadata shared over. How handy!
- You can monitor/unmonitor series from automatic download. Just like sonarr!
- Uses anilist for metadata, saves it into the chapter itself for easy-importing and such.
- New sites are just a .yaml with some html selectors (and maybe some javascript). No rust knowledge needed.
    - Hell half the providers were just me giving chatgpt the yaml schema and an example.
- REST API, so someone with a workable knowledge of frontend design can implement their own (PRs welcome!)

### Minimum Viable Release

This is all the stuff I haven't started working on yet, but will be in before a 0.1 release.

#### New Database/User Wizard
First-run modal that shows on fresh install. Saves `wizard_completed` flag to settings table when complete. Users can revisit from settings if needed.

Steps:
1. **Library Setup**: Create first library or skip if configured via environment variables
   - Select default monitor status for new series (monitored vs unmonitored by default)
2. **Provider Configuration**: Enable/disable providers with suggested defaults
   - Show recommended provider scores based on everythingmoe-style trust list
   - User can adjust scores here
3. **Download Priority**: Select preferred tier order (default: Official > Named Scanlation Group > Unknown > Aggregator)
   - Allows filtering to specific scanlators or only official releases
4. **Import Existing Library** (optional): "Do you have an existing rebarr library?" yes/no
   - Scan all subdirectories recursively for .cbz files
   - Process one by one: for each file, parse ComicInfo.xml for metadata
   - Run AniList search with parsed title, show results to user
   - User confirms correct match → runs RefreshMetadata
   - User configures provider enables/disables and aliases
   - Queue BuildFullChapterList and ScanDisk tasks
   - Repeat until all directories processed
5. **Quick Tutorial**: Brief overview of UI


#### Extended ComicInfo.xml Support

- [ ] Add a new task to 'Fix metadata/comicinfo', which goes through all files in all libraries and checks the comicinfo, updating them when needed?

### Maximum Viable Release (in order of importance)

This is in addition to the above.

- [ ] Metadata API
    - [ ] MyAnimeList Support (mal_api crate works)
    - [ ] MangaUpdates Support (need to make a crate, or use the worst fucking openapi generated thing ever)
    - [ ] Any other sites can be listed here. It's good to not be stuck with a single metadata service.
- [ ] Storage Backends
    - [ ] S3 Storage?
    - [ ] IPFS/decentralised 'provider'
- [ ] A nice looking webui
    - [ ] Basic Auth
    - [ ] oauth/oidc/whatever fancy system (would link with komga emulation)
    - [ ] egui frontend, bundled and compiled into wasm, along with a native desktop app.
        - [ ] Chapter reader
            - is there a cbz viewer for egui? (nope lol)
            - gonna have to make one ourselves
        - steal all the hard work i did for mash/yams
- [ ] Import workflows
    - [ ] Losslessly convert pages to webp/whatever (uses https://lib.rs/crates/compress_comics)
    - [ ] Detect watermarks (and remove them?)
    - [ ] Detect Low Quality images
    - [ ] Detect and remove scanlator pages where they have 4 pages of random fucking memes seriously just have one at most.
- [ ] Work with non-manga comics?
- [ ] Komga server 'emulation' (I just wanna read isekai-slop on my phone w/Mihon)
    - [ ] User system
    - [ ] read-list
    - [ ] Scrobbling to mal? (do we need this? most programs already have some form of support... BUT this would be better since automatic matching with mal ids we already use)
- [ ] Notification System
    - Webhooks on events
        - Download completed
        - Download failures
        - Other problems
- [ ] Metrics (because i love grafana graphs)
- [ ] Suggestion system
    - Saves suggestions from anilist for each series in library
    - Count occurances over the entire library
    - Deduplicate against series already in library
    - Show them all, in order of how often they show up.
    - Maybe some fancy stuff later, use tags or whatever?
    - AI slop suggestions?
    - something else?
- [ ] Anti-scraping prevention
    - 'random' useragent (pick a random one from the `n` most common UA's)
    - Seriously how do they figure out im using a automated browser surely we can fix that
- [ ] Tachiyomi/Mihon backup importer (Add libraries)
- [ ] Various site list scrape + importer
- [ ] Fallback mode? use single provider as grand source of metadata?
    - This helps shit like Brainrot Girlfriend, which is only on mangadex?
    - Easier than manually adding and matching i guess.
- wtf even is rootless docker?
- [ ] WebUI for viewing chapters - so we can have the user/automated flagging of pages:
    - FrontCover, InnerCover, Roundup, Story, Advertisment, Editorial, Letters, Preview, BackCover, Other, Deleted?

## Installation

1. Download the following:
    - Any wanted providers from `/providers`
    - `docker-compose.yaml`
2. Edit the docker-compose.yaml to your liking
3. `docker compose up -d`

If you don't trust the docker image i host, build it yourself nerd

### Docker Desktop Viewer

Docker now runs a virtual desktop stack for Chromium:
- `Xvfb` + `x11vnc` + `noVNC`
- `nginx` reverse-proxy so everything stays same-origin on port `8000`

Open `/desktop` in Rebarr to view the browser session.
- Starts in **view-only**
- Click **Unlock Controls** for manual interaction

### Dev Install

Requires rust/cargo and whatever else i add

1. `cargo run --bin rebarr`

for testing a provider, use

`cargo run --bin scraper_test -- "mangasearchterm"`

## Thanks

- domacikolaci, with his nice Claude subscription that made this project so much easier.
- Kakao Entertainment (Thanks for being so shit I made this out of spite.)
