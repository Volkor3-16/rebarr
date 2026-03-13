
# Tasks

- SearchSeries - Just calls the scraper's search function (from yaml)
- Build Chapter List - Builds a list of all chapters from each provider (if its got it)
- Update Chapter List - Searches through all chapters from each provider, adding new chapters (done in scoring order)
    - This doesn't re-search for chapters, it just takes the series url and checks for new chapter. If it fails, try later but continue.
- ScoreProviders - Scores each provider depending on (in order of priority):
    1. Matching language
    2. Full chapter > partial/split chapters (1 > 1.1 and 1.2)
    3. Scanlator
        - Normal Tiers of Official > Trusted Scanlator > Scanlator > Unknown
        - Tiebreakers of 'most recent' (for fixed releases?)
            - Maybe we can have it download a page from each chapter in the tie, and compare for quality (res/compression/size)
- DownloadChapter - Downloads the chapters pages, builds the cbz.
- OptimiseChapter - re encodes the chapter's pages into lossless webp.


## SearchSeries

Finds the series on all configured providers

Takes in a string with the name of the series, and outputs the following:

`series_id` - the internal id of the series, just in case
`provider` - the name of the provider, just in case
`series_url` - the url of the series page.
`failure_reason` - if there's an error, return the reason why here.

Only runs when:
- The series is first added
- When the user manually triggers it
- If all providers return failures?

## BuildChapterList

Builds a full index of available chapters online.

Takes in all provider series_urls

Outputs a list of providers and their available chapters.


## UpdateChapterList

Rebuilds a incremental index of available chapters online.

Takes in the full ChapterList as it exists.

1. For each provider that has valid chapters, grab the full chapter list
2. Insert new chapters into the chapterlist/db

## Score Providers

This decides which providers are the best to download from... per series.
If the user has set a custom provider scoring on the series, we don't need to do this.

We rank on a few metrics (in order of importance)
- Newest chapter (higher number, higher score)
- Chapter coverage (more chapter, higher score)
- Quality (requires downloading one page per provider?)
    - No need to do this at the moment. It's massively annoying for a slight gain. 
- Release speed (requires date scraping)
    - No need to do this at the moment. It's incredibly complicated and the first two are a little bit more important.
    - This calculates which provider added the chapter first, and score it depending on the oldest

## DownloadChapter

Fetch images and produce a .cbz

1. Fetch Page List from scraper
2. Download images 
3. Sort pages?
4. Chuck in a 'cbz' (zip file with .xml data)
5. Save to disk and mark as downloaded.

## OptimiseChapter

Reduces disk usage by more efficiently encoding chapter pages.

1. Extract CBZ
2. Encode images according to settings (default lossless webp)
3. Rebuild CBZ

## Create Metadata

Creates/updates the metadata on disk for a given series.

1. Reload metadata from anilist
2. Download thumbnail if missing
3. Rebuild ComicInfo.xml

## SelectProviderForChapter

This decides what provider to use for each chapter download.

1. For each chapter, find the highest scoring provider
2. loop until done.