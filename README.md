# Rebarr

WARNING: This is a massively WIP project. Shit will break, the UX will change massively, blah blah no warranties and such. :(

Rebarr is a sonarr-like manager and scraper for manga and comics*.

I never liked how **all** the other manga scrapers work. All that I tried use the scraped site as the authoritative source for metadata. To me, this seems very fragile and relies on awfully designed and maintained manga piracy sites.
In constrast, rebarr uses Anilist and a very fancy (overdesigned) matching system to search and download only the best copies of a chapter over multiple sites.

## Features & Roadmap

Check out [ROADMAP.md](/docs/ROADMAP.md) for everything.

## Installation

1. Download / copy `docker-compose.yaml` to your server
2. Edit the docker-compose.yaml to your liking
3. `docker compose up -d`

If you don't trust the docker image i host, build it yourself nerd

### Dev Install

Requires rust/cargo and whatever else i add

1. `CHROME_HEADLESS=false cargo run --bin rebarr`

CHROME_HEADLESS=false is helpful to see the status of the web scraper without the vnc fuckery that exists in docker

for testing and debugging providers, use the `cli` binary:

```
# List all loaded providers
cargo run --bin cli -- providers

# Test a single provider end-to-end (search → chapters → pages)
cargo run --bin cli -- test -p WeebCentral "Berserk"

# Test with visible browser + HTML dumps for debugging selectors
cargo run --bin cli -- -V -k -H test -p WeebCentral "Berserk"

# Test with full provider request/response trace
cargo run --bin cli -- test -p WeebCentral --verbose "Berserk"

# Also download the first chapter to ./test_dl/
cargo run --bin cli -- test -p WeebCentral -d "Berserk"

# Run all providers against a query and show a comparison table
cargo run --bin cli -- scan "Berserk"

# Run provider fixture tests (regression testing)
cargo run --bin cli -- test              # test all providers against test_fixtures/
cargo run --bin cli -- test WeebCentral  # test one provider
cargo run --bin cli -- test --update     # re-seed all fixtures from live scrape
```

Global flags (`-V` visible browser, `-k` keep open, `-H` dump HTML) go before the subcommand.

`cli test --verbose` is the main provider-debugging workflow. It prints a step-by-step trace of the YAML engine, including expanded requests, response status/body previews, `json_path` extraction, `from_json` transforms, and a final "last trace" summary if the provider fails mid-run.

Example trace shape:

```text
== Search ==
  provider trace:
    [#3 fetch] POST https://api.example.test/graphql -> search_results
      headers: Content-Type: application/json, x-token: abc123
      body: {"query":"{search(...)}"}
      response: 200 OK ok=true final_url=https://api.example.test/graphql (842 bytes)
      response body: {"data":{"search":{"rows":[...]}}}
      json_path data.search.rows -> array [{"title":"Berserk","slug":"berserk"}]
      stored 'search_results' = [{"title":"Berserk","slug":"berserk"}]
```

## Thanks

- domacikolaci, with his nice Claude subscription that made this project so much easier.
- Dr. Scrotus, for testing the early builds and a few good ideas :)
- Kakao Entertainment (Thanks for being so shit I made this out of spite.)
