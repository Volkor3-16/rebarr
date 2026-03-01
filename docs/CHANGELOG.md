# Changelog

This a informal changelog so i can keep track of what im doing.

## 2026-02-28

- Rewrote to use Anilist instead of MAL or MangaUpdates
- Vibe-planned out how scraping chapters would work
- (vibe)wrote conversion from anilist to internal manga types
- Started thinking about how rocket web stuff



====

3. Nope, thats not going to happen. Users can't enable some 'aggressive checking' because this will already be likely too much for the API/scrapers. It's a fine line. Manga are downloaded and then nothing from  then on. Manual re-downloads are again, manual so no need for any flags otherwise.
5.1. If I store the number as a string, it's more difficult to sort. And impossible to try to figure out what chapters are matched on different providers.
5.2. This is why I said i haven't given it much thought. I'm not sure and this gets overwhelming.
6. Yes, thats why chapter_count is stored in manga.
7. YES, the system is strictly allowing only one version of a chapter.
8. Scanlator groups is more just for users to see how consistent their collection is, and for internal scoring of who to download (as that's built)


The whole flow of manga is:
1. add manga to library
2. rebarr downloads metadata from anilist, saving thumbnail
3. rebarr does not download any chapters by default, only new chapters.
    - Downloading is done strange, because I want it to work even if a provider stops working, or gets DMCA'd.
    - My idea at the moment is have a internal (user editable) score for each provider, and the system goes through, and checks the provider for a specified chapter. If missing, it goes on to the next provider. I imagine there's a better way of doing this (like storing a list of all available chapters for each manga from each provider, keeping them in the db and updating them before each series/chapter download.)
    - Additionally, I have no real clue on how we'd get the chapters actually scraped and merged from providers.