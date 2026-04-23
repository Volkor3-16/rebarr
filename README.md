# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such. :(

Rebarr is a sonarr-like manager and scraper for manga and comics*.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Bugs & TODO

I'll remove this when I've got the first public release out, this is just a quick reference for me to see what I need to work on.

### Backend

- Providers should be library specifc -> tag for western, manga,manwha and shit
    - same with metadata client
    - Categories?
- I Was Reincarnated as the 7th Prince So I Can Take My Time Perfecting My Magical Ability
    BuildFullChapterList • 2/3 attempts • 1m ago
    17%
    Finished provider search on TCB Scans
    Error: scraper error: Browser error: task 115865 panicked with message "end byte index 62 is not a char boundary; it is inside '術' (bytes 60..63) of `転生したら第七王子だったので、気ままに魔術を極めます`"
- Build Full Chapter List should:
    - Not just error our when a provider is disabled
    - take priority over anything, even manual downloads
- and automatic instant reqwest mode for  search steps too, auto check providers when searching? on hover?
    - automatically start searching for providers when searching anilist.
    - we can just throw them out/invalidate/cache them separately if we don't follow  through with a library add.
- quality rules for provider tags: Mostly useless since each provider has it's own default score.
- We probably shouldn't include every single metadata instance for the fucken translator group for official releases, you can't see the provider.
- Use a combined metadata thing for titles, save titles from 'all/best' providers, just not save it in the rebarr json entry
- rebarr chapter tags - marking low quality, mtl, whatever else
    - Automatic rules for specific groups, titles whatever?
- [ ]  Chapter 8.1 — The Residence (1) is incorrectly marked as an extra (My Co-worker Is an Eldritch X!)
    - Same with Chapter 14.5 — Oscar Orcus (Arifureta)
- [ ] Cloudflare bypass not working anymore :( - https://www.mangakakalot.gg/manga/i-became-the-strongest-with-the-failure-frame/chapter-29-6

### Frontend

- [ ] Let users re-order the queue
- [ ] Setup Wizard: Adding 49 series to library… should have some logging or progress.
- Count number total number of unique suggestions, show that somewhere idk i just wanna know how quick it updates
- [ ] We can't delete downloaded chapters from providers that have been disabled. the action menu doesn't have any entries.
- [ ] Detect duplicates + old downloads? We shouldn't have more than one cbz of the same chapter. EVER
- Initial Provider Search Rewrite:
    - Show the loading thing, but instead of a log, show a live-updating provider table.
    - This table should replace the existing one, or just be 'part' of it?
- does the worker page show rate limited/disabled providers?
    - Does rate limiting / disabling actually work after enough failures?
- Direct file browser/explorer for each series
    - Lets us see filesize of the whole series (including random ass files), delete them/import them?
- We should be able to click download on chapter variants in the dropdown table, it should automatically override them too.

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

- [ ] Mangadex: Error: scraper error: Parse error: from_json: variable 'chapters_processed' not found
- [ ] WeebCentral: titles are bad sometimes "82-eng-li" & "Ch.011 " & "Vol.7 Chapter 35 "
- [ ] Mangakakalot: We don't get a title from them? is it broke?

- Use `setBlockedResourceTypes` to block useless requests (some images, CSS, fonts, media, whatever)
    - This could work, but you'd have to have a list of stuff TO block, since blocking every 3rd party image would block pages
    - Honestly best bet could be forcing it to use pihole/dns blocking
- Add adblock to chromium?
    - Some investigation: We'd have to fork eoka to allow extra_args to add `--load-extension` to the args
- Clownflare challenge polling loop parses the full html every 0.5s, we can clean this up a bit.
- [ ] Comix can't handle titles with "The Girl From the Other Side: Siúil, a Rún". The show as "danke-Empire" (is that the uploader? scanlator? the scanlator group is "Official?" so idk.)
- [ ] Provider repo download system
    - Don't include providers in the system by default (stock rebarr should only work for local management)
    - During setup wizard, ask the user to paste in a repo (or multiple)
    - Automatic updates and all that nice stuff
    - Add more providers
    - The separate repo should also have a nice CI Pipeline that runs tests for each provider, using text_fixtures, to find broken providers, and alert of new broken ones & a nice 'auto updating' list of what providers work
- Can we just make a wrapper to parse the keiyoushi extensions instead of needing to adapt them all?

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
- [ ] Tachiyomi/Mihon backup importer (Add libraries)
- [ ] Various site list scrape + importer
- [ ] Fallback mode? use single provider as grand source of metadata?
    - This helps shit like Brainrot Girlfriend, which is only on mangadex?
    - Easier than manually adding and matching i guess.
- wtf even is rootless docker?
- [ ] WebUI for viewing chapters - so we can have the user/automated flagging of pages:
    - FrontCover, InnerCover, Roundup, Story, Advertisment, Editorial, Letters, Preview, BackCover, Other, Deleted?
- [ ] Tell komga to scan for new downloads every so often (`n` download completes?)
- [ ] Torrent / Usenet support
    - Most of these releases use Volumes ripped from the publisher, so we'd need to map them somehow.
- [ ] Volume -> Chapter mapping
    - bruh https://github.com/TheIceCreamTroll/VolumeToChapterConverter they have the chapter number in the filenames in the cbz. lmfao
        - For anything that doesn't have this formatting, we force the user to match it and have it submitted to some online api thing or something so other people can use it too? we could always add this in later.

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

# Test with full provider request/response trace
cargo run --bin cli -- test -p WeebCentral --verbose "Berserk"

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

`cli test --verbose` is the main provider-debugging workflow. It prints a step-by-step trace of the YAML engine, including expanded requests, response status/body previews, `json_path` extraction, `from_json` transforms, and a final "last trace" summary if the provider fails mid-run.

Example trace shape:

```text
== Search ==
  provider trace:
    [#3 fetch] POST https://api.example.test/graphql -> search_results
      headers: Content-Type: application/json, x-token: abc123
      body: {"query":"{search(...)}"}
      response: 200 OK ok=true final_url=https://api.example.test/graphql (842 bytes)
      response body: {"data":{"search":{"rows":[...]}}}
      json_path data.search.rows -> array [{"title":"Berserk","slug":"berserk"}]
      stored 'search_results' = [{"title":"Berserk","slug":"berserk"}]
```

## Thanks

- domacikolaci, with his nice Claude subscription that made this project so much easier.
- Dr. Scrotus, for testing the early builds and a few good ideas :)
- Kakao Entertainment (Thanks for being so shit I made this out of spite.)
