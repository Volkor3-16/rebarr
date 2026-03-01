# Rebarr

Rebarr is a Manga downloader.... made for selfhosting nerds.

I never liked how **all** the other manga scrapers work. They all use the scraped site as an authoratitive source for chapter naming, and metadata. This fucked up my workflow with suwayomi when the site changes the name, and it creates a new series in komga.


The plan is to use myanimelist as a metadata source, and to automatically* match manga across different sites. Sorta like how sonarr works with multiple indexers.

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
- [ ] Web API
- [ ] Scraper
    - [ ] Simple title-matching with aliases?
    - [ ] Scoring? sorta like sonarrs custom scores thing

## Features

### Minimum Viable Release

- [ ] AniList api metadata
    - [ ] Manual user entered fallback for manga not on AniList.
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

but once it's done, just copy the docker compose and change the stuff to what makes sense for ur setup


### Dev Install

Requires rust/cargo and whatever else i add

1. `cargo run --release`

thats it :)


## Thanks

- AniList for their nice API and database, even if they don't have a nicely updated 'chatper count' entry.
- Kakao Entertainment, for ruining the entire scanlation community enough to warrant me making this. You do realise shutting down managa/manwha sites that's not available for sale traditionally is a failing on **your** part right? You're making it harder for people to even find out about your content you're trying to sell.
- All the other companies that also don't realise that.


Please do note that this program was NOT entirely vibecoded, but I did use a little bit to help plan and implement some of the basics. It's a nice tool for rubber duck debugging too, and just to clarify, all LLM Generation was done on a Local Machine (lie), entirely powered with solar.



## Bugs

- Chapter counts are being used from Anilist, they should be None Values until we scrape providers.
- Tags aren't being scraped, or not saved at least.
- Thumbnails work, but when the manga is added to the library, I'd like to download the high resolution cover photo and use that.
