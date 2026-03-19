# Rebarr

Rebarr is a Manga downloader.... made for selfhosting nerds.

I never liked how **all** the other manga scrapers work. They all use the scraped site as an authoratitive source for chapter naming, and metadata. This fucked up my workflow with suwayomi when the site changes the name, and it creates a new series in komga.


The plan is to use AniList as the metadata source, and to automatically* match manga across different sites. Sorta like how sonarr works with multiple indexers.

## Bugs / Dev TODO

- Test downloads for the love of god I need to find a chapter that releases so I can have a full workflow of a new chapter -> download
- Chapter searching doesn't work on the frontend, what did claude even do here?
- Filters don't really work 'downloaded' doesn't do anything
- Downloads aren't as good as I hoped
    - Downloaded chapters should make the whole row change colour a bit
    - Also we sorta just patch on downloaded state, it should listen to the downloaded cbz metadata and not just use the canonical chapter listed. (or do, idk, whatever looks the best, more thinking needs to go into this.)

## Features

- You can add series to your library and download them
- We automatically look on **all**(available) sites and compare what they have
- We automatically compare them, and download the best one (Official rip > Scanlator > unlabelled/unknown)
- the ui works.
- You can monitor/unmonitor series from automatic download. Good for when you've already got a full set and don't need them to download.
- Uses anilist for metadata, saves it into the chapter itself for easy-importing and such.
- New sites are just a .yaml with some html selectors (and maybe some javascript). No rust knowledge needed.
    - Hell half the providers were just me giving chatgpt the yaml schema and an example.
- REST API, so someone with a workable knowledge of frontend design can implement their own (PRs welcome!)

### Minimum Viable Release

- [ ] New Database/new user wizard
    1. Ask to create a library directory (or skip if already set in env)
    2. Ask user to select enabled/disabled providers
    3. Ask user to select default monitor status (do we auto-download newly released chapters?)
    4. Ask user to select download priority ordering (default Official > Named Scanlation Group > Unknown > Aggregator reupload)
    3. Ask user select directoy full of old manga
        - Match and import into DB.
        - Moves, renames, matches files to chapters - exactly like sonarr bulk import
        - Do this for each manga series, let user match and verify if it doesn't match automatically.
- [ ] Automatic upgrade path
    - We should re-download existing chapters if they're a new canonical one. (Upgrade from scan to official)
    - Ignore/warn user of overrides
- [ ] Provider Scores
    - Use this to help decide which providers get used in the case of a conflict (Comix & LHTranslation having the same copy, lhtranslation should be preferred.)
    - Global setting, acts on the entire app where the provider is used.
    - Series setting, acts on the series
    - This should NOT be able to make a trusted scan be more important than an official copy.
- Populate ComicInfo.xml betterer. (https://github.com/anansi-project/comicinfo/blob/main/schema/v2.0/ComicInfo.xsd)
    - Requires saving more details from anilist (Writer/Penciller/Inker/Colorist/Letterer/AgeRating/Community Rating)

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