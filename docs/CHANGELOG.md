# Changelog

This a informal changelog so i can keep track of what im doing.

## 2026-03-15

- Improved Manga searching
    - We now save the synonyms (other names) for each manga. While searching for chapters, if we don't find a result, we try its other names before giving up.
    - Internally, we removed `title_og`, `title_roman`, and merged them into `other_titles`
    - `other_titles` is the Anilist `synonyms` + `title_og` + `title_roman`
    - We now filter out non-Manga from the Anilist search results.
        - Made an issue upstream: https://github.com/Thunder-Blaze/anilist_moe/issues/10
    - We now properly escape searching. things with ' and " and other fucked up characters work now.
    - We now show the synposis in the search results (the first 150 characters anyway), helps when searching a lot.
    - Fixed a bug with how searching works: previously any results in the search (even non-matching) would end a search early, now we continue until all are exhausted, or no actual matches are found.
- Updated Comix provider to use their search endpoint correctly (sort by most relevant first...oops!)
- The frontend has been updated accordingly, we now show other titles in the same nice bubble thing as tags.
- We've disabled a bunch of needless logging by default. You can still use `RUST_LOG` or `.env` to override.
- We've also cleaned up the frontend's table, made it easier to see chapters and their variants, along with actioning them.

## 2026-03-13

- Added language and date scraping from providers (if supported)
- Cleaned up the DB, removing unused old stuffs
    - This one is huge, I completely deleted the Chapter & ProviderChapterURL table and replaced it with Chapters & CanonicalChapters
    - Also re-wrote half the db functions since they need to point to the right thing.
- Frontend changes to fix up chapter grouping (maybe, i haven't checked yet)

## 2026-03-10

- Chapters list now have dropdowns on each chapter, showing all available providers for a chapter
    - Users can now 'pin' a variant chapter to be the default, download checks that first.
- Scoring is re-written to use tiers.
- Split chapter handling
    - Chapters no longer use floats, but `chapter_base` and `chapter_variant` (and `is_extra`).
- Renamed scraper_test to cli, added future ideas of a entirely CLI version of the scraper/downloader.
- Refactored the whole thing, moved stuff into their proper place.

## 2026-03-08

- MVR completion pass
- RefreshAniList task: re-fetches metadata from AniList and updates DB + cover image
- ScanDisk task: scans library directory for existing CBZ files and marks chapters Downloaded
- OptimiseChapter task: re-encodes chapter images to WebP, rebuilds CBZ
- Mark as Downloaded: new API endpoint + UI button for manually marking chapters
- Refresh Metadata, Scan Disk, Optimise buttons in series view UI
- README MVR checklist updated
- Updated scraper_test to be more helpful with output.
- New working providers (total of 5 working providers!):
    - AsuraComic (Asura Scans)
    - Comix
    - MangaBall
- New not-working providers:
    - AllManga
    - MangaDex

## 2026-03-06

- Moved scraper/browser engine from chromiumoxide to eoka, because its simpler and bypasses cloudflare.
- Implemented tcbsans (because brother loves his one piece)
- scraper_test actually matches on the title now.
- Implemented Cancelling Tasks
- Working Queue system with chapter download queues.

## 2026-03-03

- Vibecoded up a scraper system
- Along with the provider yaml
- WeebCentral Works! (when testing manually)
- Started work on queue/tasks and integration with the main app.

## 2026-03-02

- Fixed bugs
    - Chapter counts were being saved in the db from anilist
    - Tags weren't being scraped (or just saved?)
    - Thumbnails only should be used for searching, download the full-sized cover when adding to library.
    - When adding a series, the folder name was based off the anilist id, not the title.
    - Cleaned up formatting and some easy rust warnings

## 2026-03-01

- Grok add yogurt to my database
- Grok add yogurt to my web app

## 2026-02-28

- Rewrote to use Anilist instead of MAL or MangaUpdates
- Vibe-planned out how scraping chapters would work
- (vibe)wrote conversion from anilist to internal manga types
- Started thinking about how rocket web stuff

## 2026-02-10

- Imagine loosing the entire project and needing to rewrite it again from half-done backups.