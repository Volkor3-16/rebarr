# Provider Research

## Comix

- I'm not sure, haven't looked at it. but this easily has the largest collection.

Working Example: https://github.com/manga-download/haruneko/blob/master/web/src/engine/websites/ComixTo.ts

## WeebCentral

Working fine with basic curl and matches.

## MangaBall

- curl is 403'd, would need JS

## AllManga

- "Error Page" when curling.
- I got myself blocked from the api lmao

## MangaFire

- has some bullshit drm protection. (forced breakpoints and automatic reloading when devtools is open)
- uses some image scrambling with canvas to load images. what the fuck.

Working example: https://github.com/manga-download/haruneko/blob/master/web/src/engine/websites/MangaFire.ts

