# Metadata sources

This is the main source of information. We don't trust providers to keep valid data, and matching it properly across sites (tachiyomi title matching anyone?)

In the future, I'd like to move from purely using one metadata provider to supporting many, and using a combination of them.
Anilist already supports and provides a myanimelist id for each item (may not be populated for all though?)
So we could use a mix of both.
either way both are fairly simple, and we can switch between them with minimal code changes.

## AniList

We use [Anilist_Moe](https://github.com/Thunder-Blaze/anilist_moe) as a wrapper for the API.

it supports everything we need, with no accounts needed (so far).

While using it, i've found and reported one bug: https://github.com/Thunder-Blaze/anilist_moe/issues/10
I've worked around this locally, but contributing to open source is nice lmao

## MyAnimeList

We used to use MAL, but in my use with it, it doesn't have as much manga in its database.
