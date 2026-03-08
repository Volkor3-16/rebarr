This document explains the scraping / provider system.
It's a little complicated, so sit tight.


Overview:

1. Rebarr loads all (valid) provider yamls
2. user clicks around and uses the app
3. when triggered, rebarr searches for the manga on all providers
4. rebarr saves a cached copy of the manga's chapter list (to combine with other providers for final chapter list)
5. when triggered, rebarr downloads each image in a chapter.


To do that, we need each provider to have:


## Search

Searches for the series. 
Requires returning the 
- `url` (that is, any url with a full list of chapters)
- `title` (For matching to the internal name, or manual matching)
- `cover` image (for uhhh.. idk. looking at? idk if we use this anywhere yet.)

## Series

Parses the list of chapters. Requires returning:
- `number_raw` - The raw string that contains the chapter number.
- `url` - a list of chapter urls. (that is, any url with a full list of chapter pages images)
- **Optional:** `scanlator_group` - The name of the scanlator group, or official publisher.
- **Optional:** `title` - The name of the chapter.

## Page

This runs once, and returns a list of all pages (images) in a chapter.

Parses the chapter page.
- `url` - The image url itself. Passed directly to the downloader (or maybe the scorer in the future?)