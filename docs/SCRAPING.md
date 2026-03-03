# Scraping dev-yapping

- Scraping providers should be a simple text based config (html tags and whatever pointing to the images are saved here.)
- Scrape tasks are done with proper cooloff times and ratelimiting (per site)
- Queues should have per-site 'channels' and rate limits set from above.
- Ideally we steal tachiyomi's extension methods, not the code but the /how/

## What needs to be added, per provider

1. Search 'api' and parsing of response
2. Listing chapters and parsing of response
3. Downloading pages

In the provider config, this is split into 4 parts.

- The main info (name, base url, rate limits and such).
- The search section (to tell the scraper how to search and how to read results)
- The chapters section (to list and parse chapters provided)
- The pages section (to retrieve and store page images per chapter)


This is in relative order of what needs to happen. Since we can't view chapters without knowing the path to view them.
However, we can cache most of this, so we don't end up needing to hit the search every single time.

Once we've found the manga url/ series url, we save it, and we can go from there to check for new chapters (only requires listing chapters once per check)