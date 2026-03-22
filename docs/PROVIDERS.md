# Provider Research

A lot of the sites are supplied from https://everythingmoe.com/section/manga
If you're here because you're hoping for your favourite site to be added, make a new issue :) (or even better, a PR!)


| Provider Name  | Domain                  | Status      | Details |
|----------------|-------------------------|-------------|---------|
| Weeb Central   | weebcentral.com         | **WORKING** | Stock provider, ai did the whole thing in seconds. |
| TCB Scans      | tcbonepiecechapters.com | **WORKING** | No search, but all series on one page, wordpress scraping is easy. |
| MangaDex       | mangadex.org            | **WORKING** | Searching Works, chapter list and pages does not. |
| AsuraComic     | asuracomic.net          | BROKEN      | These cunts re-order their pages, and tinker with the site to stop scraper. Their scans are available elsewhere. |
| Comix          | comix.to                | **WORKING** | NextJS app, works by injecting scripts instad of normal selector scraping. |
| AllManga       | allmanga.to             | **WORKING** | Large Library, uses graphql to fuck us over a bit.  |
| MangaBall      | mangaball.net           | **WORKING** | Massive scanlator hub, Working but incredibly low quality metadata and rips. |
| Atsumaru       | atsu.moe                | **WORKING** | Decent Aggregator, well organised library. |
| Mangago        | www.mangago.zone        | N/A         | Large library (esp yaoi smh) long lived site |
| Cubari         | cubari.moe              | N/A         | Supports multiple sites scraped, popular |
| VyManga        | vymanga.com             | N/A         | Giant library, but bad quality. |
| WeebDex        | weebdex.org             | **WORKING** | Highest image quality, for the relatively smaller library size. No Official Rips |
| MangaTaro      | mangataro.org           | N/A         | Medium-small lib, some scanlators use it. |
| MangaCloud     | mangacloud.org          | N/A         | Well curated, but small library. New site |
| MangaKatana    | mangakatana.com         | N/A         | Good sized lib, batch downloads. |
| KaliScan       | kaliscan.com            | N/A         | Giant Library, but bad search and slow updates. |
| MangaBuddy     | mangabuddy.me           | N/A         | Large library, lots of alt domains |
| Mangakakalot   | ?                       | BROKEN      | My personal addition. Low quality, slow but large library. Great backup, but broken due to cloudflare blocks, will fix when they lower blocks |
| Kagane         | ?                       | N/A         | Decent Library, good tagging, good quality. Worth it. (and lots of hentai for the gooners)|
| MangaFire      | ?                       | N/A         | Older library, meh quality and tags. |
| MangaHub       | ?                       | N/A         | Large library, uses names of taken down sites. |
| ReadComicOnline| ?                       | N/A         | Good site for Western Comics |


requests_per_minute - How many requests you can make to this provider (for any action)

page_delay_ms - How long to wait between page downloads



## MangaDex

This has some annoying API. I asked claude to try it out, but it failed too. Since most of their library is shit now, I don't mind this not working for now.

## Comix

Was the first provider to use strange injected javascript hacker.
They can be a bit strict on things, and regularly stops working due to cloudflare blocks.

## MangaBall

Working but disabled. This provider is AWFUL. scanlator groups are random pokemon? metadata is just sloppy, image quality is stinky.
You can enable it if you want, but I should have checked library quality before I added this. yikes. I also won't update it if it breaks. Feel free to submit updates if it does though.

## AllManga

- GraphQL api
- But images are hosted on a domain that blocks direct downloads.
- Adapted from https://github.com/keiyoushi/extensions-source/blob/2f988d8c75e01f717706acc0b9d3917370425667/src/en/allanime/src/eu/kanade/tachiyomi/extension/en/allanime/AllManga.kt

## MangaFire

- has some bullshit drm protection. (forced breakpoints and automatic reloading when devtools is open)
- uses some image scrambling with canvas to load images. what the fuck.

Working example: https://github.com/manga-download/haruneko/blob/master/web/src/engine/websites/MangaFire.ts
