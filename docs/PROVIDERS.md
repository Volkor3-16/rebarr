# Provider Research

A lot of the sites are supplied from https://everythingmoe.com/section/manga
If you're here because you're hoping for your favourite site to be added, make a new issue :)


| Provider Name | Domain                  | Status      | Details |
|---------------|-------------------------|-------------|---------|
| Weeb Central  | weebcentral.com         | **WORKING** | Stock provider, ai did the whole thing in seconds. |
| TCB Scans     | tcbonepiecechapters.com | **WORKING** | No search, but all series on one page, wordpress scraping is easy. |
| MangaDex      | mangadex.org            | BROKEN      | Searching Works, chapter list and pages does not. |
| AsuraComic    | asuracomic.net          | **WORKING** | Formatting and info kinda sloppy. |
| Comix         | comix.to                | **WORKING** | NextJS app, works by injecting scripts instad of normal selector scraping. |
| AllManga      | allmanga.to             | BROKEN      | Large Library, uses graphql to fuck us over a bit.  |
| MangaBall     | mangaball.net           | **WORKING** | Massive scanlator hub, Working but incredibly low quality metadata and rips. |
| Atsumaru      | atsu.moe                | **WORKING** | Decent Aggregator, well organised library. |
| Mangago       | www.mangago.zone        | N/A         | Large library (esp yaoi smh) long lived site |
| Cubari        | cubari.moe              | N/A         | Supports multiple sites scraped, popular |
| VyManga       | vymanga.com             | N/A         | Giant library, but bad quality. |
| WeebDex       | weebdex.org/            | N/A         | Highest image quality, for the relatively smaller library size. No Official Rips |
| MangaTaro     | mangataro.org           | N/A         | Medium-small lib, some scanlators use it. |
| MangaCloud    | mangacloud.org          | N/A         | Well curated, but small library. New site |
| MangaKatana   | mangakatana.com         | N/A         | Good sized lib, batch downloads. |
| KaliScan      | kaliscan.com            | N/A         | Giant Library, but bad search and slow updates. |
| MangaBuddy    | mangabuddy.me           | N/A         | Large library, lots of alt domains |
| Mangakakalot  | ?                       | N/A         | My personal addition. Low quality, slow but large library. Great backup |

## WeebCentral

Working fine with basic matching

## TCB Scans

Luke wanted it for one piece. I'm happy to add it.
It was a simple wordpress site, all series listed on one page, so no fancy searching needed.

## MangaDex

This has some annoying API. I asked claude to try it out, but it failed too. Since most of their library is shit now, I don't mind this not working for now.

## AsuraComic

Nothing too fancy, did have a bit of chatgpt copy-paste help.

## Comix

Working Example: https://github.com/manga-download/haruneko/blob/master/web/src/engine/websites/ComixTo.ts

- 2026-03-08: All done. This provider works very different than the others so far.
  Searching is the same, but we inject javascript into the site to extract chapter and page info.
  Duplicates are removed based on the **NEWEST** chapter of the number, hopefully automatically ignoring the slop mtls and going for officials or higher quality releases.

## MangaBall

- curl is 403'd, would need JS
- 2026-03-08: Working on this now.

## AllManga

- GraphQL api
- But images are hosted on a domain that blocks direct downloads.
- Adapted from https://github.com/keiyoushi/extensions-source/blob/2f988d8c75e01f717706acc0b9d3917370425667/src/en/allanime/src/eu/kanade/tachiyomi/extension/en/allanime/AllManga.kt

## MangaFire

- has some bullshit drm protection. (forced breakpoints and automatic reloading when devtools is open)
- uses some image scrambling with canvas to load images. what the fuck.

Working example: https://github.com/manga-download/haruneko/blob/master/web/src/engine/websites/MangaFire.ts

## Atsumaru

