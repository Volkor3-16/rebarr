# Database Documentation

I'm a little lazy and never bothered with a DB page, so I fucked myself over and let claude randomly add whenever it wanted.  
Now I had to delete half the schema and re-engineer it. yay  
Might as well write what's valid here.

## Library Table

Fairly self-explainatory, stores the libraries.
Libraries are a mostly usued feature (as of yet).
Essentially here for support later on where I have different metadata sources.

- uuid
- library_type
- root_path

## Manga

One 'level' down from Libraries. Stores all the metadata from Anilist/MAL, and a few other things

- uuid                  `Randomly Generated uuid` 
- library_id            `the uuid of the parent library`
- anilist_id            `the anilist id`
- mal_id                `the myanimelist id`
- relative_path         `the path (relative to library's root_path) of the series`
- title                 `english title`
- title_og              `japanese title`
- title_roman           `japanese (romanised) title`
- synopsis              `Description`
- publishing_status     `Completed|Ongoing|Hiatus|Cancelled|NotYetReleased|Unknown`
- start_year            `start year lmao`
- end_year              `end year lmao`
- chapter_count         `Chapters, according to the merged chapter list`
- metadata_source       `Where we grabbed this metadata from`
- thumbnail_url         `URL to the thumbnail (this is downloaded once and never used again lol)`
- created_at            `unix timestamp when the Manga series was added to the library`
- metadata_updated_at   `the last time the metadata was refreshed`
- monitored             `Do we scan for new chapters and download them?`

## Chapters

This replaced two old-crusty-ass tables before.

- uuid                  `Randomly generated`
- manga_id              `the uuid of the parent series`
- chapter_base          `The 'whole number' of the chapter (eg. 1, 2, 3)`
- chapter_variant       `The 'everything after the decimal' of the number (eg. 0.1, 0.2 0.3)`
- title                 `The title of the chapter, if supplied`
- language              `The language of the chapter, defaulting to english`
- scanlator_group       `The scanlator group's name, if supplied`
- provider_name         `The name of the provider we scraped this from`
- chapter_url           `The URL of the chapter itself (for the downloader)`
- download_status       `Missing|Downloading|Downloaded|Failed`
- released_at           `When the provider added the chapter to *their* site`
- downloaded_at         `When we downloaded the chapter`
- scraped_at            `When we first saw the chapter`

### uq_chapter_unique

For any given manga chapter, there can be only one row with this exact combination of:
- manga_id
- chapter_base
- chapter_variant
- language
- provider_name

This makes sure we don't just continually add the same chapter into the Chapters table on every rescan, additionally, helps us keep track.

It also helps us avoid duplicate chapters (which we might want honestly, idk, will address when we run into it)

It's not even really a index lol


### idx_chapter_manga_id

Its a fucking lookup structure. No need to parse every row to check if the manga_id matches.

## CanonicalChapters

This is part 2 of the above. We don't want to scan the Chapters table every time we need to render a series page, or whatever. Lets cache it and only reload it when we re-scan

We save this in a big json array for each series that exists, the array stores the uuid's of the best scoring chapters, the ones we'll auto-download.

Honestly, im not even sure if this is faster. We should benchmark it or smth.

- manga_id              `UUID of the manga`
- canonical_list        `JSON encoded array of post-scoring chapter uuids`
- last_updated          `Might be helpful idk`


## MangaTags

This just stores what series has what tags.
thats it :)

- manga_id
- tag

it has something called a 'composite primary key', effectively making any duplication combination impossible.

## Task

This stores the recent and queued tasks. Honestly much of it is unknown to me, I didn't trust myself to make it work. Claude did it :)

TODO: We should probably clean this up, either have it all payload or have it all task_type.
Also maybe every so often delete ancient task history, if it becomes a problem.

- uuid                  `randomly generated`
- task_type             `ScanLibrary|RefreshAniList|CheckNewChapter|DownloadChapter|Backup`
- status                `Pending|Running|Completed|Failed|Cancelled`
- library_id            `needed for ScanLibrary`
- manga_id              `needed for RefreshAniList,CheckNewChapter,DownloadChapter`
- chapter_id            `needed for DownloadChapter`
- priority              `lower number, higher priority`
- payload               `Could be anything! unused`
- attempt               `counting failures`
- max_attempts          `how many it can do`
- last_error            `what the last error was`
- created_at            `timestamp when task was queued up`
- updated_at            `timestamp when it last changed state? why?`
- run_after             `timestamp when to next run (for exponential backoffs)`

### idx_task_*

-- Composite index for the worker's polling query:
-- WHERE status = 'Pending' AND run_after <= ? ORDER BY priority ASC, run_after ASC

These all just speedup querying for common task stuff.

## TrustedGroup

Curated list of known/trusted scanlation group names (for Tier 2 scoring).
Names are matched case-insensitively at score time.

TODO: idk if I like this. I'd rather have it be saved in a normal text file and loaded up. saves a migration.

## MangaProvider

This exists to map providers and manga, additonally, we use it to enable/disable providers for individual manga series.

- manga_id              `uuid of the manga series`
- enabled               `if we're going to use the provider for searching`
- provider_name         `the name of the provider (sorta its id)`
- provider_url          `the url of the series page/chapter list on a provider`
- last_synced_at        `the timestamp when we last checked for chapters`