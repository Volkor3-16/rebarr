# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such.

Rebarr is a sonarr-like manager and scraper for manga and comics.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Bugs & TODO

I'll remove this when I've got the first public release out, this is just a quick reference for me to see what I need to work on.

### Backend

### Providers

- Providers steps shouldn't need a random ass `- open` and then hit another endpoint why have the open step at all?
- [ ] Get Mangago working
- [ ] Get MangaHub working (no chapters returned)

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
- Kakao Entertainment (Thanks for being so shit I made this out of spite.)
