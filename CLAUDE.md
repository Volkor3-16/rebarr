# Rebarr: Manga Library Manager – Summary for Agents

## Purpose:
A self-hosted manga library manager, similar to Sonarr but for manga. Designed for long-term maintainability and resilient downloads across multiple providers.

## Core Concepts

### Manga & Metadata

- Manga: Represents a series in the library, includes library location, chapter counts, and metadata source.
- Metadata: Contains titles, synopsis, tags, publishing status, and start/end years. Separate from Manga to allow independent refreshes.

## Chapters

- Logical Chapters: One per manga + chapter number, tracking download status, title, volume, and scanlator group.
- Provider Chapters: Temporary structures from scraping; indicate which providers have which chapters.
- Rules:
    - Only one version per chapter is ever downloaded.
    - Chapters from multiple providers are merged for availability, but the internal chapter remains singular.
    - Download URLs are handled at runtime and not persisted.

## Providers

- Multiple providers are supported.
- Each (enabled) provider is searched and checked for valid chapters
- Handles provider failures and DMCA takedowns gracefully.

## Scraping & Merge Flow

- Scrape Providers: Collect chapter lists from each provider.
- Merge Chapters:
    - For each chapter number not yet downloaded, find the highest-scored available provider.
    - Map this provider chapter to the internal chapter record.

- Update Manga Stats:
    - `chapter_count` = highest chapter number known.
    - `downloaded_count` = count of chapters actually downloaded.

## Design Principles

- Decouple metadata from library storage.
- Keep logical chapters separate from provider-specific availability.
- Maintain deterministic mapping between internal chapters and provider data.
- Only download chapters manually or when new chapters appear; no automatic upgrades.
- Designed for long-term self-hosting with maintainability and future provider addition in mind.

## Key Takeaways

- Chapters are single-version entities; providers are fallback sources.
- `chapter_count` and `downloaded_count` are derived from merged chapters.
- Download URLs are not persisted; scraping/downloader runtime resolves them.
- System resilient to provider failure, DMCA takedowns, and multiple providers.
- Metadata refreshes are independent of chapter downloads.