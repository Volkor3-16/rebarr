# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such.

Rebarr is a sonarr-like manager and scraper for manga and comics*.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Bugs & TODO

I'll remove this when I've got the first public release out, this is just a quick reference for me to see what I need to work on.

- [ ] More providers - 20+ is my goal.. lets see lmfao
- [ ] Maybe have each chromium worker in a window, grid view type shit? sure i made it work fairly nice but ehhhhh i mean does it count unless you can actually SEE it like a stupid little monkey boy????? huh??
- [ ] Count cloudflare errors and log them -> enough of them and we have longer and longer cooldowns and rate limits / just disable provider completely?
- [ ] Honestly same with any provider.. Should have a failed provider list, and their errors. -> optional report bug button???
- [ ] idk but I wanna see the timings of each search task -> actually see with my own eyes the critical path
- [ ] Include the downloaded_at in task queue page and series list (hover over filesize?)
- [ ] Switch between current download mode (only select the best release) and MUST HAVE download mode (pick the best, fallback to 2nd on failure)
- [ ] Does DownloadChapter work in parallel? I know in one task it does, but does a group of DownloadChapter's from 2 providers run at once, or one after? 
- [ ] Visual Task Queue
    - Group by 'provider', show task order for them
        - Sub-Tasks, for the 'checknewchapter' checking `n` providers separately, 
    - im a baby boy who needs a baby ui to make sure my code works
- [ ] Chapter duplicates functionally work but aren't showing in the ui (only way to know is after importing 2 of the same series)
- [ ] Is there a rate limit for anilist api?
    - 90 requests/sec on their end
    - They use `Retry-After`, `X-RateLimit-Reset`, `X-RateLimit-Remaining`, and `X-RateLimit-Limit`
    - We should use the same rate limit system as providers, but much faster
    - https://docs.rs/governor/latest/governor/
- Are page downloads the most request friendly they could be?
    - is scraping?
    - We don't want to impact manga sites with hundreds of instances of rebarr searching 24/7 in the most bullshit inefficient way
- Update frontend to use daisyui components as much as possible https://daisyui.com/components/
- [ ] Chapters start downloading automatically after a initial build chapter list... why?. We have local ones and even if they're not canonicial, we shouldn't automatically override them.
- [ ] Providers `default_score` doesn't actually get used anywhere.
    - We should make it be nice, where it shows the defaults as a greyed out 'default' and allow overriding like normal either globally or per series.

### Providers

- Providers steps shouldn't need a random ass `- open` and then hit another endpoint why have the open step at all?
    - I tried an AI Slop version of this, didn't work correctly, will re-look properly later. this breaks js scripts that need the page open
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
- Nice 'Setup Wizard' that'll help you match and import your existing library
    - Although it's a bit jank, it mostly works if you're careful.
    - That all said, unless you've got a giant collection, its probably better to just redownload what you want.

### Later?

- [ ] Metadata API
    - [ ] MyAnimeList Support (mal_api crate works)
    - [ ] MangaUpdates Support (need to make a crate, or use the worst fucking openapi generated thing ever)
    - [ ] Any other sites can be listed here. It's good to not be stuck with a single metadata service.
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
