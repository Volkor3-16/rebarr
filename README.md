# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such.

Rebarr is a sonarr-like manager and scraper for manga and comics*.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Bugs & TODO

I'll remove this when I've got the first public release out, this is just a quick reference for me to see what I need to work on.

### Backend

- [ ] Provider repo download system
    - Don't include providers in the system by default (stock rebarr should only work for local management)
    - During setup wizard, ask the user to paste in a repo (or multiple)
    - Automatic updates and all that nice stuff
    - Add more providers
- [ ] Make sure chapters are done nicely, `A Veternarian in Another World` - Local chapters aren't flagged as downloaded, despite being there.
    - Do we have ScanDisk task validate and check this?

### Frontend

- [ ] Update frontend to use daisyui components as much as possible https://daisyui.com/components/
    - The site looks nice as it is, and the import is half broken, this'd be a lot of work... polishing up a poop.
- [ ] Include the downloaded_at in task queue page and series.
- [ ] Have a 'Downloads' Page, where it shows pretty much a condensed version of the queue, where stuff is grouped by series (sequential chapters?)
    - Let users re-order the queue
- [ ] Setup Wizard: Adding 49 series to library… should have some logging or progress.
- [ ] Local chapters (ones where we've downloaded a split chapter (7.1, 7.2, 7.3) are all grouped under a full chapter (7) despite being the local chapter)

### Assend (GraphQL frontend for Moku-like frontend?)

- fuck that shiiiiittttttyyy ass frontend man holy shit just fork a competent one
- why not just pretend to be suwayomi?
    - We'd have to have just one(two?) extension, that is rebarr (and rebarr search)
        - Rebarr: shows your list
            - Downloading or opening will kick you out / load forever / error out and queue up a download with absolute highest priority, and finally load when done.
        - Rebarr (search): searches anilist -> adding to (suwayomi) frontend queues up a rebarr library add and full provider search.

    - This shit should hopefully let it work nicely in suwayomi (and even suwayomi android extension??)
    - https://deepwiki.com/Suwayomi/Suwayomi-Server/2.4-graphql-api
    - Also REST API?
    - Also also OPDS API?

### Providers / Scraper

- Providers steps shouldn't need a random ass `- open` and then hit another endpoint why have the open step at all?
    - I tried an AI Slop version of this, didn't work correctly, will re-look properly later. this breaks js scripts that need the page open
- Use `setBlockedResourceTypes` to block useless requests (some images, CSS, fonts, media, whatever)
- Add adblock to chromium?
- Clownflare challenge polling loop parses the full html every 0.5s, we can clean this up a bit.
- [ ] Get Mangago working
- [ ] Get MangaHub working (no chapters returned)
- [ ] Comix can't handle titles with "The Girl From the Other Side: Siúil, a Rún". The show as "danke-Empire" (is that the uploader? scanlator? the scanlator group is "Official?" so idk.)

## Features

- You can add series to your library and download them (obviously)
- We automatically look on **all**(available) sites and compare what they have, downloading only the best copy.
- Downloaded chapters save all the metadata inside them, so if my awful code breaks something, you can rebuild (most of) the database from that. Also, any manga you share with another rebarr installation will have its metadata shared over. How handy!
- You can monitor/unmonitor series from automatic download. Just like sonarr!
- Uses anilist for metadata, saves it into the chapter itself for easy-importing and such.
- New sites are just a .yaml with some html selectors (and maybe some javascript). No rust knowledge needed.
    - Hell half the providers were just me giving chatgpt the yaml schema and an example.
- CLI tool for testing and debugging providers without touching the database — search, list chapters, download pages, run regression tests against fixture files.
- REST API, so someone with a workable knowledge of frontend design can implement their own (PRs welcome!)
- Nice 'Setup Wizard' that'll help you match and import your existing library
    - Although it's a bit jank, it mostly works if you're careful.
    - That all said, unless you've got a giant collection, its probably better to just redownload what you want.

### Later?

- [ ] Metadata API
    - [ ] MyAnimeList Support (mal_api crate works)
    - [ ] MangaUpdates Support (need to make a crate, or use the worst fucking openapi generated thing ever)
    - [ ] Comic Vine (for western comics - needs user provided API Key)
    - [ ] Any other sites can be listed here. It's good to not be stuck with a single metadata service.
    - [ ] Automatic imports of Anilist genres (auto-add and download all/top/trending of any tag)
    - [ ] Automatic imports of MyAnimeList "Interest Stacks"
    - [ ] Browser Extension to add "add to rebarr" buttons on MyAnimeList / AniList pages
        - Also maybe any MAL/anilist urls pasted anywhere (reddit comments)?
- [ ] Storage Backends
    - [ ] S3 Storage?
    - [ ] IPFS/decentralised 'provider'
- [ ] a frontend that isn't ai slop
- [ ] Import workflows
    - [ ] Losslessly convert pages to webp/whatever (uses https://lib.rs/crates/compress_comics)
    - [ ] Detect watermarks (and remove them?)
    - [ ] Detect Low Quality images
    - [ ] Detect and remove scanlator pages where they have 4 pages of random fucking memes seriously just have one at most.
- [ ] Work with non-manga comics?
- [ ] Komga server 'emulation' (I just wanna read isekai-slop on my phone w/Mihon without running extra software)
    - [ ] User system because komga uses it
        - username can store read history 
            - password is just a key to hand out?
    - [ ] Scrobbling to mal/anilist???
- [ ] Metrics (because i love grafana graphs)
- [ ] Suggestion system
    - Saves suggestions from anilist for each series in library
    - Count occurances over the entire library
    - Deduplicate against series already in library
    - Show them all, in order of how often they show up.
    - Maybe some fancy stuff later, use tags or whatever?
    - We should show relations (prequels/sequels/whatever), sorta like sonarr/radarrs 'collections' feature
    - AI slop suggestions?
    - something else?
- [ ] Tachiyomi/Mihon backup importer (Add libraries)
- [ ] Various site list scrape + importer
- [ ] Fallback mode? use single provider as grand source of metadata?
    - This helps shit like Brainrot Girlfriend, which is only on mangadex?
    - Easier than manually adding and matching i guess.
- wtf even is rootless docker?
- [ ] WebUI for viewing chapters - so we can have the user/automated flagging of pages:
    - FrontCover, InnerCover, Roundup, Story, Advertisment, Editorial, Letters, Preview, BackCover, Other, Deleted?
- [ ] Tell komga to scan for new downloads every so often (`n` download completes?)

## Installation

1. Download / copy `docker-compose.yaml` to your server
2. Edit the docker-compose.yaml to your liking
3. `docker compose up -d`

If you don't trust the docker image i host, build it yourself nerd

### Dev Install

Requires rust/cargo and whatever else i add

1. `CHROME_HEADLESS=false cargo run --bin rebarr`

CHROME_HEADLESS=false is helpful to see the status of the web scraper without the vnc fuckery that exists in docker

for testing and debugging providers, use the `cli` binary:

```
# List all loaded providers
cargo run --bin cli -- providers

# Test a single provider end-to-end (search → chapters → pages)
cargo run --bin cli -- test -p WeebCentral "Berserk"

# Test with visible browser + HTML dumps for debugging selectors
cargo run --bin cli -- -V -k -H test -p WeebCentral "Berserk"

# Also download the first chapter to ./test_dl/
cargo run --bin cli -- test -p WeebCentral -d "Berserk"

# Run all providers against a query and show a comparison table
cargo run --bin cli -- scan "Berserk"

# Run provider fixture tests (regression testing)
cargo run --bin cli -- test              # test all providers against test_fixtures/
cargo run --bin cli -- test WeebCentral  # test one provider
cargo run --bin cli -- test --update     # re-seed all fixtures from live scrape
```

Global flags (`-V` visible browser, `-k` keep open, `-H` dump HTML) go before the subcommand.

## Thanks

- domacikolaci, with his nice Claude subscription that made this project so much easier.
- Dr. Scrotus, for testing the early builds and a few good ideas :)
- Kakao Entertainment (Thanks for being so shit I made this out of spite.)
