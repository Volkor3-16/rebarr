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
- Full test of chapter upgrading from site
- Providers that break can sometimes download a empty image and make a corrupt cbz file (all manga)
    - We already know how many pages a chapter /should/ have, we should compare against that to check for potentially failed downloads
- Deleting a file on disk, then running a ScanDisk doesn't flag the file as missing.


#### Manual Provider Matching

Some Manga series (Ruri Dragon for example) have two series with the same name. One, the main series, and one is a oneshot.
Since rebarr matches the first result with a good name match, we can't correct that.
Ideally I'd like a way for a user in the frontend to see each providers search results and manually match it, if there's multiple matches.

### Frontend

#### Downloaded Chapters UI
- Highlight downloaded chapters with a subtle background color on their row (instead of just a small green icon like now)
- Display the downloaded version's metadata in the chapter list (title, scanlator, etc)
- (when theres a chapter downloaded) Store all available online versions as "variants" - accessed via expand/collapse on each chapter row

#### Series Page Improvements
- Add tooltips to all interactive elements (provider names, scanlator groups, chapter info)
- Display scanlator group as a styled pill component
- Improve score display (currently "basic" - needs better visualization)
- Improve "+1 More" pill styling (indicates additional variants exist)
- The 3-dot action menu already exists - ensure it has "Download" and "Extra" options

#### Queue/Activity Page
- Replace current queue page with fullscreen activity view in top bar
- Stream live tracing/logs to this view
- Implement multiple tabs: one per log channel (info, warn, error) or per task type

#### Home Page Sorting
- Implement multiple sort options, each with toggle between ascending/descending:
  - Latest chapter (most recent chapter number)
  - Most recent check (last_checked_at timestamp)
  - First added (created_at timestamp)
  - A-Z (alphabetical by title)
  - Number of chapters (total from providers)
  - Number of chapters downloaded (downloaded_count)

#### System Info
- Display memory usage and basic app stats in top bar next to activity icon

#### Settings Page
- it ugly af

## Features

- You can add series to your library and download them (obviously)
- We automatically look on **all**(available) sites and compare what they have, downloading only the best available.
- You can monitor/unmonitor series from automatic download. Good for when you've already got a full set and don't need them to download.
- Uses anilist for metadata, saves it into the chapter itself for easy-importing and such.
- New sites are just a .yaml with some html selectors (and maybe some javascript). No rust knowledge needed.
    - Hell half the providers were just me giving chatgpt the yaml schema and an example.
- REST API, so someone with a workable knowledge of frontend design can implement their own (PRs welcome!)

### Minimum Viable Release

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

#### Chapter Importing
- User selects a directory containing manga files
- Recursively scan for all .cbz files in directory and subdirectories
- Parse ComicInfo.xml from each cbz for metadata (anilist_id, title, etc)
- Use parsed metadata for automatic series matching (or prompt user)
- Move and rename files to standard library/series/ directory structure
- Same naming scheme as downloaded chapters

#### Automatic Upgrade Path
- Automatically re-download chapters when they upgrade to a higher tier
- Tier order: Official > Trusted Scanlator > Scanlator > Unknown > Aggregator
- Same-tier upgrades (e.g., LHTranslation → Comix) are ignored
- Log all upgrades to activity log - do not prompt user
- User can disable auto-upgrade in settings if desired

#### Extended ComicInfo.xml Support
Populate more fields from AniList (https://github.com/anansi-project/comicinfo/blob/main/schema/v2.0/ComicInfo.xsd):
- Writer, Penciller, Inker, Colorist, Letterer (from AniList staff/studio data)
- AgeRating (map from AniList rating)
- Community Rating (from AniList score)

This enables constructing full Manga struct from ComicInfo.xml in:
- Series directory (series-level metadata)
- Individual cbz files (chapter-level metadata)
- Greatly simplifies the import wizard - can skip AniList lookup if ComicInfo exists

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

### Dev Install

Requires rust/cargo and whatever else i add

1. `cargo run --bin rebarr`

for testing a provider, use

`cargo run --bin scraper_test -- "mangasearchterm"`

## Thanks

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