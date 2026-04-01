# Changelog

This a informal changelog so i can keep track of what im doing.

## 2026-04-01

- Queue Changes:
    - Sequential cancelled tasks are grouped together and expandable with a click
    - Downloading chapters in bulk will now order by oldest first
    - Cancelling Downloading Chapter tasks in the queue now set the chapter entry back to 'Missing'.
    - Selecting tasks in queue will no longer automatically get unselected every queue refresh.
- Merge sequential cancelled tasks into a expandable table in the queue.
- Added version number and changelog in the frontend.
- Disabling a provider immediately recalculates the canonical list (and shows it on the frontend)
- Pages that download wrong (not an image) will be automatically failed
    - and depending on your download mode, will retry another provider or fail completely.

## 2026-03-29

- Setup Wizard has been reworked
    - `/setup` url
    - uses at least slightly more daisyui/tailwind stuff
    - You can now import your existing series
        - We (you and the server) will match and copy over the manga to your new library
- Local (Manually imported) chapters are now 'tier 0', so they won't be automatically replaced when you search all providers.

## 2026-03-27

- Cover photo improvements
    - Its a bit bigger (100px -> 160px)
    - Its a bit better (hover over it to change it)
    - We can handle manual series with no photo now
- Provider list on the series is collapsible
- Added a 'mini-header' that lets you download chapters without scrolling through the list too much
- Added a 'mini-footer' that shows up when you select chapters with even more options
- You can shift-click the chapter selection boxes!
- Added a on row hover download button for chapters
- Keyboard shortcuts! s to select hovered chapter, d to download, a to select all, and Escape to clear!
- Added a shitload of tests and fix up every warning from clippy!
- Dumb and stupid (but automatic) clownflare bypass.

## 2026-03-26

- Potentially better canonical chapter downloading. Maybe?
- Discord webhooks.... maybe?
- Providers will automatically overwrite on each restart, **Additional** files won't be touched (remember: you can disable providers from the settings page)
- There's a setting to automatically unmonitor series that get flagged as 'Completed' on anilist (when refreshing metadata)
- Webhooks!
    - For a basic 'DownloadChapter - Completed' discord hook: `{"embeds":[{"title":"{{task_type}} — {{status}}","description":"{{manga_title}} Ch.{{chapter_number_raw}}"}]}`
    - werks for me
- Add a message making promises I'm not sure I can keep (a comic reader frontend)
- Fix weebdex scanlator group and released at time.

## 2026-03-25

- Series Tasks now look better on the frontend
    - Progress bars, loading tips / messages, logging
    - Added yogurt (snark)
- You can now delete series (either from db or both db and chapters-on-disk)
- You can now middle click series on the homepage.
- Reimplemented (yet again) the scraper/queue system
    - We run a worker for each provider, that can run in parallel to drastically improve scanning chapters.
    - Also set it up for the future to potentially run multiple browsers at once, for even faster speedups (but more ram usage :s )

## 2026-03-24

- Tested out openai codex by implementing better cloudflare bypassing
    - (when in docker) We run chromium inside a dummy x server, which is exposed on the /desktop endpoint
    - This required adding nginx and a bunch of vnc stuff.
    - If we get cloudflare checked, we can manually click the checkbox (or hope that the bypasser clicks it for us)
- Added much better status information for tasks.
    - Provider search gives you info about what provider is searching, and with what title
    - Disk Scans have some info, but I can't see it because it runs too fast lmao
    - Chapter downloads have per-page download progress shown
- Mangakakalot provider is now working, since we can bypass cloudflare properly
- Fix mangadex issues with scanlators and official publisher stuff

## 2026-03-23

- Add date scraping because i forgot to do that for the providers added yesterday
- Simplify docker build pipeline (removed arm64 builds.. my poor poor garage orange pi....)
- Docker now runs a virtual desktop stack (`Xvfb + x11vnc + noVNC`) with a new `/desktop` web view.
- Split `scan_manga()` into like 5 different functions, make it betterer and simples
- Bypass clownflare again! (imagine having a bad useragent and all the stealth stuff disabled)
- Reduce memory usage by actually exiting browser sessions after we're done with them.. lol
- Implement pagination for graphql steps

## 2026-03-22

- Added WeebDex provider
- Added MangaBuddy provider
- Added MangaHub provider
- Tested and Added MangaDex Provider
    - No longer crashes when you search for something that links to the publisher!
- AllManga provider is working
- Added better provider handling

## 2026-03-21

- Scandisk will mark deleted chapters as missing again
    - Any chapters marked as 'Local' will be removed completely, since they're not automatically grabbed by the system.
- Added Bulk chapter importing
- Rewrote Comicinfo.xml, now includes internal rebarr metadata for sharing and reimport later.
- Fixed up a few pain points I had with the frontend (tooltips and such)

## 2026-03-19

- Scandisk actually now scans the disk and flags already-downloaded chapters as downloaded.
- Also added a filesize thing we're not using yet
- Changed UUID's to be deterministic, instead of random. Future us can share chapters and have everything match up
    - ipfs backend!?!?!?!
- Implemented chapter upgrades (for when a official release upgrades over a scanlator copy automatically)
- Changed provider score colours a bit
- Searching & Filtering works now!
- Add more debugging logs
- Replaced some stuff with tailwind/daisyUI so I can come back and use it properly
- Refactored api module into a bunch of different files.
- Implemented a bunch of shit i forgor


## 2026-03-18
- Fixed up docker builds with gitlab-ci.
- Truncation of long titles
- Activity log on the top bar should work properly now
- Chapter downloader now actually downloads chapters where the user overrode the provider.
- More debug logging. very helpful for testing!

## 2026-03-17

- Replaced frontend almost completely
    - Similar base, just split into files and a more sensible structure.
    - Dark/Light theme (depending on browser preferences/override)
    - Series view improved substantially
    - Home view looks nicer
    - Github looking quick chapter status view
- We now restart the embedded browser when it dies (often)
- Claude re-wrote the rate limiting / queue system
    - Rate limiting during page downloads (with page_delay_ms)
    - Cancelling chapters won't just get the task stuck, stops the download mid-state and resets to 'Missing'
    - Reset buttons on chapters flagged as Failed
- Merged testing migrations

## 2026-03-16

- Added LHTranslation provider
- Added scanlator column to chapterview
- Fix chapters again lol, extras should show in the main chapter table

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
