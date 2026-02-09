# Rebarr

Rebarr is a Manga downloader.... made for selfhosting nerds.

I never liked how **all** the other manga scrapers work. They all use the scraped site as an authoratitive source for chapter naming, and metadata. This fucked up my workflow with suwayomi when the site changes the name, and it creates a new series in komga.


The plan is to use myanimelist as a metadata source, and to automatically* match manga across different sites. Sorta like how sonarr works with multiple indexers.

## Dev TODO

- [x] Setup base structure
- [x] Get MAL API working
- [x] Convert MalManga response into manga struct
    - To do this, i need to have a way for the user to 'select' one.
    - So I think we just hard-code one for now to get the code working, and then have the selection later on.
- [ ] DB Stuffs
- [ ] Web API
- [ ] Scraper
    - [ ] Simple title-matching with aliases?
    - [ ] Scoring? sorta like sonarrs custom scores thing

## Features

### Minimum Viable Release

- [ ] MAL api metadata (will require API Key!)
    - [ ] Manual user entered fallback for manga not on MAL.
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

but once it's done, just copy the docker compose and change the stuff


### Dev Install

Requires rust/cargo

1. `cargo run --release`

thats it :)
