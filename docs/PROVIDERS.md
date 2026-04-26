# Provider Research

A lot of the sites are supplied from https://everythingmoe.com/section/manga
If you're here because you're hoping for your favourite site to be added, make a new issue :) (or even better, a PR!)

| Provider Name  | Domain                  | Status      | Details |
|----------------|-------------------------|-------------|---------|
| Weeb Central   | weebcentral.com         | **WORKING** | Stock provider, ai did the whole thing in seconds. |
| TCB Scans      | tcbonepiecechapters.com | **WORKING** | No search, but all series on one page, wordpress scraping is easy. |
| MangaDex       | mangadex.org            | **WORKING** | Searching Works, chapter list and pages does not. |
| AsuraComic     | asuracomic.net          | BROKEN      | These cunts re-order their pages, and tinker with the site to stop scraper. Their scans are available elsewhere. |
| Comix          | comix.to                | **WORKING** | Large library, graphql |
| AllManga       | allmanga.to             | **WORKING** | Large Library, uses graphql to fuck us over a bit.  |
| MangaBall      | mangaball.net           | **WORKING** | Massive scanlator hub, Working but incredibly low quality metadata and rips. |
| Atsumaru       | atsu.moe                | **WORKING** | Decent Aggregator, well organised library. |
| Mangago        | www.mangago.zone        | N/A         | Large library (esp yaoi smh) long lived site |
| Cubari         | cubari.moe              | N/A         | Supports multiple sites scraped, popular |
| VyManga        | vymanga.com             | N/A         | Giant library, but bad quality. |
| MangaTaro      | mangataro.org           | **WORKING** | Medium-small lib, some scanlators use it. Requires persisted `manga_id` from search. |
| MangaCloud     | mangacloud.org          | **WORKING** | Well curated, but small library. New site |
| MangaKatana    | mangakatana.com         | N/A         | Good sized lib, batch downloads. |
| KaliScan       | kaliscan.com            | N/A         | Giant Library, but bad search and slow updates. |
| MangaBuddy     | mangabuddy.me           | N/A         | Large library, lots of alt domains |
| Mangakakalot   | mangakakalot.gg         | **WORKING** | My personal addition. Low quality, slow but large library. Great backup, but broken due to cloudflare blocks, will fix when they lower blocks |
| Kagane         | ?                       | N/A         | Decent Library, good tagging, good quality. Worth it. (and lots of hentai for the gooners)|
| MangaFire      | ?                       | N/A         | Older library, meh quality and tags. |
| MangaHub       | mangahub.io             | **WORKING** | Large library, uses names of taken down sites. |
| ReadComicOnline| ?                       | N/A         | Good site for Western Comics |
| MangaPlus      | https://mangaplus.shueisha.co.jp | N/A | Official? |
| UTOON          | ?                       | N/A         | They do a LOT of scans |
| RawSakura      | https://rawsakura.org   | N/A         | It has raws, maybe in the future we can auto-MTL it. I have series i wanna read that are nearing 10 years old. |
| SirenScans     | https://sirenscans.com  | **WORKING** | Shitty Paid ones, but direct scans. |
| RitharScans    | https://ritharscans.com | **WORKING** | Shitty paid one too, but its the only available for some |

requests_per_minute - How many requests you can make to this provider (for any action)

# How do I make my own provider?

Aight okay, go to the site, open dev-tools and search for anything.
- Make a copy of a existing provider
- If it's a fancy one that auto-loads results, check the network tab and find the request it does - almost always a json result
    - Most manga sites do this, and this means you'll do a graphql step (or fetch and json parsing). Either way, paste the request (curl) and response into notes
    - You'll likely be able to do the same for chapter lists, so do the same thing and try to find the json chapter list result. Paste in the same notes
    - For page list, it's a 50/50 shot on either graphql or html scraping.
- If It's a boring one, you'll have to scrape the html with selectors, either way, paste the html blocks into notes
    - You'll do the same thing with the html tags on the chapter list
        - And the page list.
- Put the notes + example provider into your favourite LLM and ask it nicely to do it all for you (or do it yourself i guess)
    - Debug and test, its very likely broken.
    1. Ensure searching works (cli command)
    2. Ensure chapter list works (same command)
    3. Ensure pages downloading works (add `-d`)
    4. Ensure the dates, scanlator groups, titles, and whatever else is right... assuming the provider.. provides them.
- If none of that works, check keiyoushi & haruneko for working examples and 'inspiration'.
    - https://github.com/manga-download/haruneko/tree/master/web/src/engine/websites
    - https://github.com/keiyoushi/extensions-source/tree/main/src/en/

## Relative Dates?

Sometimes providers won't have a date at all, some have like 3 different ones, some have 'relative' dates. (`2 years ago`)
For relative dates, we can to some awful javascript conversion to unixtime.

For any other format, we can give the normal date format string

# Provider Details

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

## WeebDex

RIP WeebDex. I removed them from the provider list. Most of their content was copied from MangaDex, so it wasn't super important.

## MangaTaro

These guys are fucky, they have a server-side token and timestamp, and if they don't match, it fails.
Thankfully, they've really nicely given us a `generateToken()` function! Which gives us exactly the two things we need to get into their API!


# Shit providers list

You shouldn't be charging for chapters you stole. Groups like these are fucking things over for everyone, free scanlations are already legally gray, charging for them is definitely illegal.

Kakao, if you're reading this, go after these cunts first ;)

- Art Lapsa: Deleting their previously uploaded chapters, charging for them. Oh yeah, and it's mostly MTL slop.
- Asmodeus Scans: Charging for their chapters.
- Athrea Scans: Charging for their MTL chapters.
- Diva Scans
- Luna Toons
- GalaxyDegenScans: Their patreon vote-paywall where they sit on translated chapters for months is cringe.
    - Einherjar Scan: They disbanded and went on to form GDS
- UTOON
- Reaper Scans: bahahahahaha imagine selling chapters and then getting a cease and desist.
- Asura Scans: please daddy keep charging me for chapters you censor and snipe from other groups
- Philia Scans: "I didn't know it was a crime to paywall - continues to paywall"
- Diva Scans
- Luna Toon
- RESET Scans
- Siren Scans
