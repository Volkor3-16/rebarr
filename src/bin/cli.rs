//! Rebarr CLI — scrape & download manga without the web UI.
//!
//! Subcommands:
//!   providers              List all loaded providers with tags and rate limits
//!   test [query]           Test providers: with query → interactive; without → fixture validation
//!   scan <query>           Run all/selected providers and show aggregated results
//!   download <query>       Download a specific chapter from a provider to disk

use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use std::{io::Write as _, path::PathBuf, sync::Arc};
use strsim::jaro_winkler;
use tracing::{error, info, warn};

use rebarr::scraper::{
    browser::BrowserPool, executor::ProviderExecutor, ProviderRegistry, ProviderSearchResult,
    ScraperCtx,
};

// ─── CLI definition ──────────────────────────────────────────────────────────

#[derive(Parser)]
#[command(name = "cli", about = "Rebarr CLI — scrape & download manga without the web UI")]
struct Cli {
    /// Run Chromium in non-headless (visible) mode
    #[arg(short = 'V', long, global = true)]
    visible: bool,

    /// Don't close Chromium on exit (useful for debugging providers)
    #[arg(short = 'k', long, global = true)]
    keep_open: bool,

    /// Dump page HTML to ./scraper_dump_N.html after each open step
    #[arg(short = 'H', long = "dump-html", global = true)]
    dump_html: bool,

    #[command(subcommand)]
    command: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// List all loaded providers with their tags and rate limits
    Providers,

    /// Test providers against fixtures or interactively.
    ///
    /// Without QUERY: runs fixture validation for all providers (or -p <name>).
    /// With QUERY:    runs search → chapters → pages for the given manga title.
    ///
    /// Examples:
    ///   cli test                                    # validate all fixtures
    ///   cli test -p WeebCentral                     # validate one provider
    ///   cli test "berserk"                          # interactive test (first provider)
    ///   cli test -p WeebCentral "berserk"           # interactive test (specific provider)
    ///   cli test --update "berserk" -p WeebCentral  # create/update fixture
    ///   cli test --update                           # refresh all fixtures (uses stored queries)
    Test {
        /// Manga title — if omitted, runs fixture validation instead
        query: Option<String>,
        /// Provider filter (default: first loaded for interactive; all for fixture mode)
        #[arg(short, long)]
        provider: Option<String>,
        /// [Interactive] Download the chapter to ./test_dl/
        #[arg(short, long)]
        download: bool,
        /// [Interactive] Chapter number to test (default: first in list)
        #[arg(short, long)]
        chapter: Option<f32>,
        /// Fixture directory
        #[arg(long, default_value = "./test_fixtures")]
        fixtures: String,
        /// Write/update fixture files from a live scrape instead of testing
        #[arg(long)]
        update: bool,
        /// Print raw scraped data (search results, chapter list, page URLs) at each step
        #[arg(short, long)]
        verbose: bool,
    },

    /// Run all/selected providers and show aggregated chapter results
    Scan {
        /// Manga title to search for
        query: String,
        /// Limit to this provider only (default: all)
        #[arg(short, long)]
        provider: Option<String>,
        /// Auto-select best match without prompting
        #[arg(long)]
        no_interactive: bool,
    },

    /// Download a specific chapter from a provider to disk
    Download {
        /// Manga title to search for
        query: String,
        /// Provider to use (default: first loaded)
        #[arg(short, long)]
        provider: Option<String>,
        /// Chapter number to download
        #[arg(short, long)]
        chapter: f32,
        /// Output directory
        #[arg(long, default_value = "./downloads")]
        out: String,
    },

}

// ─── Fixture format ──────────────────────────────────────────────────────────

/// Snapshot of one chapter's scraped metadata.
/// Every field is `Option` so partial seeds are still valid YAML.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct ChapterFixture {
    raw_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scanlator_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    /// Unix timestamp of the chapter release date.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_released: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

/// Per-provider test fixture.
///
/// Seed files (committed to the repo) contain only `provider` and `query`.
/// Run `test --update` to populate all other fields from a live scrape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Fixture {
    /// Provider name (must match the YAML `name` field exactly).
    provider: String,
    /// Search query used to find this manga on the provider.
    query: String,
    /// Expected best-match title from search (jaro-winkler ≥ 85%).
    /// Empty = not yet seeded; check is skipped.
    #[serde(default)]
    expected_search_title: String,
    /// Minimum acceptable chapter count. 0 = not seeded; check is skipped.
    #[serde(default)]
    expected_min_chapters: usize,
    /// Scraped metadata of the first chapter in the series.
    /// If any field changes between runs, the check fails.
    /// Absent = not yet seeded; check is skipped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first_chapter: Option<ChapterFixture>,
    /// URL of a known chapter to use for page-count validation.
    /// Empty = not yet seeded; check is skipped.
    #[serde(default)]
    test_chapter_url: String,
    /// Minimum acceptable page count for `test_chapter_url`.
    /// 0 = not yet seeded; check is skipped.
    #[serde(default)]
    expected_min_pages: usize,
}

// ─── Helpers ─────────────────────────────────────────────────────────────────

/// Returns the (index, score) of the best-matching result for `query_lower`.
fn best_match(results: &[ProviderSearchResult], query_lower: &str) -> (usize, f64) {
    results
        .iter()
        .enumerate()
        .map(|(i, r)| (i, jaro_winkler(query_lower, &r.title.to_lowercase())))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap_or((0, 0.0))
}

/// Scores and optionally prompts the user to pick a search result.
/// Pass `force_auto = true` to always pick the best match silently.
fn pick_result<'a>(
    results: &'a [ProviderSearchResult],
    query: &str,
    force_auto: bool,
) -> &'a ProviderSearchResult {
    const AUTO_THRESHOLD: f64 = 0.90;
    let query_lower = query.to_lowercase();

    let mut scored: Vec<(usize, f64)> = results
        .iter()
        .enumerate()
        .map(|(i, r)| (i, jaro_winkler(&query_lower, &r.title.to_lowercase())))
        .collect();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!("Search results (sorted by similarity):");
    for (rank, &(orig_idx, score)) in scored.iter().enumerate() {
        let r = &results[orig_idx];
        println!("  [{rank}] ({:.0}%) {} — {}", score * 100.0, r.title, r.url);
    }

    let (best_orig_idx, best_score) = (scored[0].0, scored[0].1);

    if force_auto || best_score >= AUTO_THRESHOLD {
        println!(
            "\nAuto-selecting ({:.0}%): {}",
            best_score * 100.0,
            results[best_orig_idx].title
        );
        return &results[best_orig_idx];
    }

    print!(
        "\nNo confident match (best: {:.0}%). Enter index [0..{}] or Enter for [0]: ",
        best_score * 100.0,
        scored.len() - 1
    );
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("failed to read input");
    let chosen_rank: usize = input.trim().parse().unwrap_or(0).min(scored.len() - 1);
    let chosen_orig = scored[chosen_rank].0;
    println!("Selected: {}", results[chosen_orig].title);
    &results[chosen_orig]
}

/// Truncate a long error string for inline display.
fn short_err(s: &str) -> &str {
    let limit = 60;
    if s.len() > limit { &s[..limit] } else { s }
}

// ─── Entry point ─────────────────────────────────────────────────────────────

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    if cli.visible {
        // Safety: single-threaded before any async work starts.
        unsafe { std::env::set_var("CHROME_HEADLESS", "false") };
    }

    let registry = ProviderRegistry::load().await.unwrap_or_else(|e| {
        error!("Failed to load providers: {e}");
        std::process::exit(1);
    });

    // providers subcommand needs no browser/executor
    if let Cmd::Providers = &cli.command {
        cmd_providers(&registry);
        return;
    }

    if registry.is_empty() {
        error!("No providers loaded. Make sure ./providers/ contains YAML files.");
        std::process::exit(1);
    }

    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .expect("failed to build HTTP client");

    let executor = Arc::new(ProviderExecutor::new(&registry, 3));
    let mut ctx = ScraperCtx::new(http, BrowserPool::new(), executor);
    ctx.dump_html = cli.dump_html;
    ctx.verbose = true;

    match cli.command {
        Cmd::Providers => unreachable!(),
        Cmd::Test { query, provider, download, chapter, fixtures, update, verbose } => {
            cmd_test(
                &registry, &ctx,
                query.as_deref(), provider.as_deref(),
                download, chapter,
                &fixtures, update, verbose,
            ).await;
        }
        Cmd::Scan { query, provider, no_interactive } => {
            cmd_scan(&registry, &ctx, &query, provider.as_deref(), no_interactive).await;
        }
        Cmd::Download { query, provider, chapter, out } => {
            cmd_download(&registry, &ctx, &query, provider.as_deref(), chapter, &out).await;
        }
    }

    if cli.keep_open {
        println!("Chromium left open. Kill this terminal to close it.");
        std::process::exit(0);
    }
}

// ─── providers ───────────────────────────────────────────────────────────────

fn cmd_providers(registry: &ProviderRegistry) {
    let all = registry.all();
    if all.is_empty() {
        println!("No providers loaded (check ./providers/ directory).");
        return;
    }
    println!("{:<22} {:>5}  {:>6}  {:<8}  Tags", "Name", "RPM", "Score", "Version");
    println!("{}", "-".repeat(72));
    for p in &all {
        let tags: Vec<String> = p.tags().iter().map(|t| format!("{t:?}")).collect();
        println!(
            "{:<22} {:>5}  {:>6}  {:<8}  {}",
            p.name(),
            p.rate_limit_rpm(),
            p.default_score(),
            p.version().unwrap_or("—"),
            tags.join(", ")
        );
    }
    println!("\n{} provider(s) loaded.", all.len());
}

// ─── test ────────────────────────────────────────────────────────────────────

#[allow(clippy::too_many_arguments)]
async fn cmd_test(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    query: Option<&str>,
    provider_name: Option<&str>,
    do_download: bool,
    target_chapter: Option<f32>,
    fixtures_dir: &str,
    update: bool,
    verbose: bool,
) {
    // ── Fixture mode (no query) ──────────────────────────────────────────────
    if query.is_none() || update {
        let fixtures_path = PathBuf::from(fixtures_dir);
        if update {
            fixture_update(registry, ctx, provider_name, &fixtures_path, query, verbose).await;
        } else {
            fixture_run(registry, ctx, provider_name, &fixtures_path, verbose).await;
        }
        return;
    }
    let query = query.unwrap();

    // ── Interactive mode (query given) ───────────────────────────────────────
    let all = registry.all();
    let provider = if let Some(name) = provider_name {
        all.into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
            .unwrap_or_else(|| {
                error!("Provider {name:?} not found. Run `providers` to list available providers.");
                std::process::exit(1);
            })
    } else {
        all.into_iter().next().unwrap_or_else(|| {
            error!("No providers loaded.");
            std::process::exit(1);
        })
    };

    println!("Provider: {}\n", provider.name());

    info!("Searching {:?} for {query:?}...", provider.name());
    let results = provider.search(ctx, query).await.unwrap_or_else(|e| {
        error!("Search failed: {e}");
        std::process::exit(1);
    });
    if results.is_empty() {
        error!("No results found for {query:?}");
        std::process::exit(1);
    }

    let manga = pick_result(&results, query, false);
    println!();

    info!("Fetching chapter list...");
    let chapters = provider.chapters(ctx, &manga.url).await.unwrap_or_else(|e| {
        error!("chapters() failed: {e}");
        std::process::exit(1);
    });
    if chapters.is_empty() {
        error!("No chapters found. The provider may require a different manga URL.");
        std::process::exit(1);
    }

    println!("Found {} chapters:", chapters.len());
    for ch in chapters.iter().take(1000) {
        let title = ch.title.as_deref().unwrap_or("(no title)");
        let scanlator = ch.scanlator_group.as_deref().unwrap_or("—");
        let url = ch.url.as_deref().unwrap_or("(no url)");
        let language = ch.language.as_deref().unwrap_or("?");
        let date_str;
        let date = match ch.date_released {
            Some(ts) => {
                date_str = ts.to_string();
                date_str.as_str()
            }
            None => "no date",
        };
        println!(
            " [{}] Ch.{} — {} [{}] ({}) {}",
            language, ch.raw_number, title, scanlator, date, url
        );
    }
    if chapters.len() > 1000 {
        println!("  ... and {} more chapters", chapters.len() - 1000);
    }

    // Pick which chapter to test pages on
    let ch = if let Some(target) = target_chapter {
        chapters
            .iter()
            .min_by(|a, b| {
                (a.number - target)
                    .abs()
                    .partial_cmp(&(b.number - target).abs())
                    .unwrap()
            })
            .unwrap()
    } else {
        &chapters[0]
    };

    let chapter_url = ch.url.as_ref().unwrap_or_else(|| {
        error!("Chapter {} has no URL", ch.number);
        std::process::exit(1);
    });

    println!("\nFetching pages for chapter {}...", ch.raw_number);
    info!("pages() → {chapter_url}");

    let pages = provider.pages(ctx, chapter_url).await.unwrap_or_else(|e| {
        error!("pages() failed: {e}");
        std::process::exit(1);
    });

    println!("Found {} pages:", pages.len());
    for p in pages.iter().take(5) {
        println!("  Page {} — {}", p.index, p.url);
    }
    if pages.len() > 5 {
        println!("  ... and {} more", pages.len() - 5);
    }
    if pages.is_empty() {
        error!("No pages found — check the 'pages' extract config in the provider YAML");
        std::process::exit(1);
    }

    if !do_download {
        println!("\n(Pass -d to also download the pages to ./test_dl/)");
        return;
    }

    let out_dir = PathBuf::from(format!("./test_dl/ch_{}", ch.raw_number));
    tokio::fs::create_dir_all(&out_dir)
        .await
        .expect("failed to create output directory");
    println!(
        "\nDownloading {} pages to {}/ ...",
        pages.len(),
        out_dir.display()
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let image_data = rebarr::scraper::downloader::download_pages_via_browser(
        None,
        None,
        ctx,
        Some(provider.name()),
        &pages,
        provider.page_delay_ms(),
        chapter_url,
        cancel,
    )
    .await
    .unwrap_or_else(|e| {
        error!("Download failed: {e}");
        std::process::exit(1);
    });

    let mut downloaded = 0usize;
    for (index, data) in &image_data {
        let ext = rebarr::scraper::downloader::image_ext(data);
        let path = out_dir.join(format!("{index:03}.{ext}"));
        tokio::fs::write(&path, data)
            .await
            .expect("failed to write page file");
        info!("  [{index}/{}] → {}", image_data.len(), path.display());
        downloaded += 1;
    }
    println!(
        "\nDone. Downloaded {}/{} pages to {}/",
        downloaded,
        pages.len(),
        out_dir.display()
    );
}

// ─── scan ────────────────────────────────────────────────────────────────────

async fn cmd_scan(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    query: &str,
    provider_filter: Option<&str>,
    _no_interactive: bool,
) {
    let all = registry.all();
    let providers: Vec<_> = if let Some(name) = provider_filter {
        all.into_iter()
            .filter(|p| p.name().eq_ignore_ascii_case(name))
            .collect()
    } else {
        all
    };

    if providers.is_empty() {
        error!("No matching providers.");
        return;
    }

    struct Row {
        provider: String,
        match_pct: String,
        chapters: String,
        first: String,
        last: String,
        error: Option<String>,
    }

    println!(
        "Scanning {} provider(s) for {:?}...\n",
        providers.len(),
        query
    );
    let query_lower = query.to_lowercase();
    let mut rows: Vec<Row> = Vec::new();

    for provider in &providers {
        print!("  {:<22} ...", provider.name());
        std::io::stdout().flush().ok();

        // Search
        let results = match provider.search(ctx, query).await {
            Ok(r) if r.is_empty() => {
                println!(" no results");
                rows.push(Row {
                    provider: provider.name().to_owned(),
                    match_pct: "—".into(),
                    chapters: "—".into(),
                    first: "—".into(),
                    last: "—".into(),
                    error: Some("no results".into()),
                });
                continue;
            }
            Ok(r) => r,
            Err(e) => {
                let s = e.to_string();
                println!(" ERR: {}", short_err(&s));
                rows.push(Row {
                    provider: provider.name().to_owned(),
                    match_pct: "—".into(),
                    chapters: "—".into(),
                    first: "—".into(),
                    last: "—".into(),
                    error: Some(format!("search: {}", short_err(&s))),
                });
                continue;
            }
        };

        let (best_idx, best_score) = best_match(&results, &query_lower);
        let manga = &results[best_idx];

        // Chapters
        let chapters = match provider.chapters(ctx, &manga.url).await {
            Ok(c) => c,
            Err(e) => {
                let s = e.to_string();
                println!(
                    " {:.0}% match — ERR(chapters): {}",
                    best_score * 100.0,
                    short_err(&s)
                );
                rows.push(Row {
                    provider: provider.name().to_owned(),
                    match_pct: format!("{:.0}%", best_score * 100.0),
                    chapters: "—".into(),
                    first: "—".into(),
                    last: "—".into(),
                    error: Some(format!("chapters: {}", short_err(&s))),
                });
                continue;
            }
        };

        let first = chapters
            .first()
            .map(|c| format!("Ch.{}", c.chapter_base))
            .unwrap_or_else(|| "—".into());
        let last = chapters
            .last()
            .map(|c| format!("Ch.{}", c.chapter_base))
            .unwrap_or_else(|| "—".into());

        println!(
            " {:.0}% match — {} chapters ({} → {})",
            best_score * 100.0,
            chapters.len(),
            first,
            last
        );

        rows.push(Row {
            provider: provider.name().to_owned(),
            match_pct: format!("{:.0}%", best_score * 100.0),
            chapters: chapters.len().to_string(),
            first,
            last,
            error: None,
        });
    }

    // Summary table
    println!(
        "\n{:<22} {:>7}  {:>9}  {:<10} {:<10}  Error",
        "Provider", "Match%", "Chapters", "First", "Last"
    );
    println!("{}", "-".repeat(78));
    for row in &rows {
        println!(
            "{:<22} {:>7}  {:>9}  {:<10} {:<10}  {}",
            row.provider,
            row.match_pct,
            row.chapters,
            row.first,
            row.last,
            row.error.as_deref().unwrap_or("")
        );
    }
    let ok = rows.iter().filter(|r| r.error.is_none()).count();
    println!("\n{}/{} providers succeeded.", ok, rows.len());
}

// ─── download ────────────────────────────────────────────────────────────────

async fn cmd_download(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    query: &str,
    provider_name: Option<&str>,
    target_chapter: f32,
    out: &str,
) {
    let all = registry.all();
    let provider = if let Some(name) = provider_name {
        all.into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
            .unwrap_or_else(|| {
                error!("Provider {name:?} not found.");
                std::process::exit(1);
            })
    } else {
        all.into_iter().next().unwrap_or_else(|| {
            error!("No providers loaded.");
            std::process::exit(1);
        })
    };

    println!(
        "Provider: {} | Chapter: {} | Query: {:?}\n",
        provider.name(),
        target_chapter,
        query
    );

    info!("Searching for {query:?}...");
    let results = provider.search(ctx, query).await.unwrap_or_else(|e| {
        error!("Search failed: {e}");
        std::process::exit(1);
    });
    if results.is_empty() {
        error!("No results found for {query:?}");
        std::process::exit(1);
    }

    let manga = pick_result(&results, query, true);
    println!();

    info!("Fetching chapter list...");
    let chapters = provider.chapters(ctx, &manga.url).await.unwrap_or_else(|e| {
        error!("chapters() failed: {e}");
        std::process::exit(1);
    });
    if chapters.is_empty() {
        error!("No chapters found.");
        std::process::exit(1);
    }

    // Find the chapter closest to the requested number
    let ch = chapters
        .iter()
        .min_by(|a, b| {
            (a.number - target_chapter)
                .abs()
                .partial_cmp(&(b.number - target_chapter).abs())
                .unwrap()
        })
        .unwrap();

    if (ch.number - target_chapter).abs() > 0.5 {
        error!(
            "No chapter close to {} found (nearest: {})",
            target_chapter, ch.number
        );
        std::process::exit(1);
    }

    let chapter_url = ch.url.as_ref().unwrap_or_else(|| {
        error!("Chapter {} has no URL", ch.number);
        std::process::exit(1);
    });

    println!("Fetching pages for chapter {}...", ch.raw_number);
    let pages = provider.pages(ctx, chapter_url).await.unwrap_or_else(|e| {
        error!("pages() failed: {e}");
        std::process::exit(1);
    });
    if pages.is_empty() {
        error!("No pages found.");
        std::process::exit(1);
    }

    let safe_title: String = manga
        .title
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == ' ' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();

    let out_dir = PathBuf::from(out)
        .join(provider.name())
        .join(&safe_title)
        .join(format!("Ch.{}", ch.raw_number));

    tokio::fs::create_dir_all(&out_dir)
        .await
        .expect("failed to create output directory");
    println!(
        "Downloading {} pages to {}/ ...",
        pages.len(),
        out_dir.display()
    );

    let cancel = tokio_util::sync::CancellationToken::new();
    let image_data = rebarr::scraper::downloader::download_pages_via_browser(
        None,
        None,
        ctx,
        Some(provider.name()),
        &pages,
        provider.page_delay_ms(),
        chapter_url,
        cancel,
    )
    .await
    .unwrap_or_else(|e| {
        error!("Download failed: {e}");
        std::process::exit(1);
    });

    let mut downloaded = 0usize;
    for (index, data) in &image_data {
        let ext = rebarr::scraper::downloader::image_ext(data);
        let path = out_dir.join(format!("{index:03}.{ext}"));
        tokio::fs::write(&path, data)
            .await
            .expect("failed to write page file");
        downloaded += 1;
    }
    println!(
        "Done. Downloaded {}/{} pages to {}/",
        downloaded,
        pages.len(),
        out_dir.display()
    );
}

// ─── fixture helpers ─────────────────────────────────────────────────────────

async fn fixture_update(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    provider_filter: Option<&str>,
    fixtures_path: &PathBuf,
    query_override: Option<&str>, // positional query from CLI, or None to reuse fixture's query
    verbose: bool,
) {
    tokio::fs::create_dir_all(fixtures_path)
        .await
        .expect("failed to create fixtures directory");

    let all = registry.all();
    let providers: Vec<_> = if let Some(name) = provider_filter {
        all.into_iter()
            .filter(|p| p.name().eq_ignore_ascii_case(name))
            .collect()
    } else {
        all
    };

    for provider in &providers {
        let fixture_file = fixtures_path.join(format!("{}.yaml", provider.name()));

        // Determine query: override > existing fixture > skip
        let existing: Option<Fixture> = if fixture_file.exists() {
            tokio::fs::read_to_string(&fixture_file)
                .await
                .ok()
                .and_then(|c| serde_yaml::from_str(&c).ok())
        } else {
            None
        };

        let query = if let Some(q) = query_override {
            q.to_owned()
        } else if let Some(ref f) = existing {
            f.query.clone()
        } else {
            warn!(
                "No fixture file and no query provided for {name}; skipping. \
                 Pass a query as a positional arg (e.g. `test --update \"berserk\" -p {name}`) to create one.",
                name = provider.name()
            );
            continue;
        };

        println!("\nUpdating {} (query: {:?})...", provider.name(), query);

        // Search
        let results = match provider.search(ctx, &query).await {
            Ok(r) if r.is_empty() => {
                error!("  No results — skipping.");
                continue;
            }
            Ok(r) => r,
            Err(e) => {
                error!("  Search failed: {e}");
                continue;
            }
        };
        if verbose {
            println!("  search results ({}):", results.len());
            let q = query.to_lowercase();
            let mut scored: Vec<_> = results.iter()
                .map(|r| (jaro_winkler(&q, &r.title.to_lowercase()), r))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            for (score, r) in scored.iter().take(10) {
                println!("    ({:.0}%) {} — {}", score * 100.0, r.title, r.url);
            }
            if results.len() > 10 {
                println!("    … and {} more", results.len() - 10);
            }
        }
        let (best_idx, _) = best_match(&results, &query.to_lowercase());
        let manga = &results[best_idx];
        println!("  Match: {} — {}", manga.title, manga.url);

        // Chapters
        let chapters = match provider.chapters(ctx, &manga.url).await {
            Ok(c) => c,
            Err(e) => {
                error!("  chapters() failed: {e}");
                continue;
            }
        };
        println!("  {} chapters", chapters.len());
        if verbose {
            for ch in chapters.iter().take(5) {
                println!("    Ch.{:<8} lang={:<4} scanlator={} date={} url={}",
                    ch.raw_number,
                    ch.language.as_deref().unwrap_or("?"),
                    ch.scanlator_group.as_deref().unwrap_or("—"),
                    ch.date_released.map(|d| d.to_string()).as_deref().unwrap_or("—"),
                    ch.url.as_deref().unwrap_or("(no url)"),
                );
            }
            if chapters.len() > 5 {
                println!("    … and {} more", chapters.len() - 5);
            }
        }

        // Pages for first chapter
        let first_ch = match chapters.first() {
            Some(c) => c,
            None => {
                error!("  Chapter list is empty — skipping.");
                continue;
            }
        };
        let chapter_url = match first_ch.url.as_ref() {
            Some(u) => u.clone(),
            None => {
                error!("  First chapter has no URL — skipping.");
                continue;
            }
        };
        let pages = match provider.pages(ctx, &chapter_url).await {
            Ok(p) => p,
            Err(e) => {
                error!("  pages() failed: {e}");
                continue;
            }
        };
        println!("  {} pages for chapter {}", pages.len(), first_ch.raw_number);
        if verbose {
            for p in pages.iter().take(5) {
                println!("    page {:>3} — {}", p.index, p.url);
            }
            if pages.len() > 5 {
                println!("    … and {} more", pages.len() - 5);
            }
        }

        let chapter_fixture = ChapterFixture {
            raw_number: first_ch.raw_number.clone(),
            title: first_ch.title.clone(),
            scanlator_group: first_ch.scanlator_group.clone(),
            language: first_ch.language.clone(),
            date_released: first_ch.date_released,
            url: first_ch.url.clone(),
        };

        let fixture = Fixture {
            provider: provider.name().to_owned(),
            query: query.clone(),
            expected_search_title: manga.title.clone(),
            // Allow 10-chapter variance from the live count
            expected_min_chapters: chapters.len().saturating_sub(10),
            first_chapter: Some(chapter_fixture),
            test_chapter_url: chapter_url,
            // Allow 3-page variance
            expected_min_pages: pages.len().saturating_sub(3).max(1),
        };

        let yaml = serde_yaml::to_string(&fixture).expect("failed to serialize fixture");
        tokio::fs::write(&fixture_file, yaml)
            .await
            .expect("failed to write fixture");
        println!("  Wrote {}", fixture_file.display());
    }
}

async fn fixture_run(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    provider_filter: Option<&str>,
    fixtures_path: &PathBuf,
    verbose: bool,
) {
    if !fixtures_path.exists() {
        error!(
            "Fixtures directory {} does not exist. Run `test --update` to generate fixtures.",
            fixtures_path.display()
        );
        return;
    }

    // Load all fixture files
    let mut dir = match tokio::fs::read_dir(fixtures_path).await {
        Ok(d) => d,
        Err(e) => {
            error!("Failed to read fixtures directory: {e}");
            return;
        }
    };

    let mut fixtures: Vec<Fixture> = Vec::new();
    while let Ok(Some(entry)) = dir.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yaml") {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => match serde_yaml::from_str::<Fixture>(&content) {
                Ok(f) => fixtures.push(f),
                Err(e) => warn!("Skipping malformed fixture {}: {e}", path.display()),
            },
            Err(e) => warn!("Failed to read {}: {e}", path.display()),
        }
    }

    if let Some(name) = provider_filter {
        fixtures.retain(|f| f.provider.eq_ignore_ascii_case(name));
    }

    if fixtures.is_empty() {
        println!(
            "No fixture files found in {}. Run `test --update` to generate them.",
            fixtures_path.display()
        );
        return;
    }

    let all = registry.all();
    let mut total_pass = 0usize;
    let mut total_fail = 0usize;
    let mut total_seed = 0usize; // fixtures with no seeded checks yet

    for fixture in &fixtures {
        let provider = all.iter().find(|p| p.name().eq_ignore_ascii_case(&fixture.provider));
        let provider = match provider {
            Some(p) => p,
            None => {
                println!("{:<22} SKIP (provider not loaded)", fixture.provider);
                continue;
            }
        };

        print!("{:<22} ...", fixture.provider);
        std::io::stdout().flush().ok();

        let mut pass = 0usize;
        let mut fail = 0usize;
        let mut skip = 0usize;
        let mut details: Vec<String> = Vec::new();

        // ── Check 1: search ──
        let results = match provider.search(ctx, &fixture.query).await {
            Ok(r) => r,
            Err(e) => {
                println!(" FAIL");
                println!("  ✗ search: {e}");
                println!("  - chapters: skipped");
                println!("  - pages: skipped");
                total_fail += 1;
                continue;
            }
        };

        if fixture.expected_search_title.is_empty() {
            skip += 1;
            details.push(format!(
                "  ? search: got {} results (not seeded — run `test --update`)",
                results.len()
            ));
        } else {
            let expected_lower = fixture.expected_search_title.to_lowercase();
            let best_score = results
                .iter()
                .map(|r| jaro_winkler(&expected_lower, &r.title.to_lowercase()))
                .fold(0.0_f64, f64::max);

            if best_score >= 0.85 {
                pass += 1;
                details.push(format!(
                    "  ✓ search: found {:?} ({:.0}% match)",
                    fixture.expected_search_title,
                    best_score * 100.0
                ));
            } else {
                fail += 1;
                details.push(format!(
                    "  ✗ search: expected {:?} but best match was {:.0}%",
                    fixture.expected_search_title,
                    best_score * 100.0
                ));
            }
        }

        // Pick best result for chapter check
        let query_lower = fixture.query.to_lowercase();
        if verbose {
            let mut scored: Vec<_> = results.iter()
                .map(|r| (jaro_winkler(&query_lower, &r.title.to_lowercase()), r))
                .collect();
            scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            println!("  search results ({}):", results.len());
            for (score, r) in scored.iter().take(10) {
                println!("    ({:.0}%) {} — {}", score * 100.0, r.title, r.url);
            }
            if results.len() > 10 {
                println!("    … and {} more", results.len() - 10);
            }
        }
        let (best_idx, _) = best_match(&results, &query_lower);
        let manga = &results[best_idx];
        if verbose {
            println!("  selected: {} — {}", manga.title, manga.url);
        }

        // ── Check 2: chapters ──
        let chapters = match provider.chapters(ctx, &manga.url).await {
            Ok(c) => c,
            Err(e) => {
                fail += 1;
                details.push(format!("  ✗ chapters: {e}"));
                details.push("  - pages: skipped".into());
                print_validate_result(pass, fail, skip, &details);
                total_fail += 1;
                continue;
            }
        };

        if verbose {
            println!("  chapters ({}):", chapters.len());
            for ch in chapters.iter().take(5) {
                println!("    Ch.{:<8} lang={:<4} scanlator={} date={} title={} url={}",
                    ch.raw_number,
                    ch.language.as_deref().unwrap_or("?"),
                    ch.scanlator_group.as_deref().unwrap_or("—"),
                    ch.date_released.map(|d| d.to_string()).as_deref().unwrap_or("—"),
                    ch.title.as_deref().unwrap_or("—"),
                    ch.url.as_deref().unwrap_or("(no url)"),
                );
            }
            if chapters.len() > 5 {
                println!("    … and {} more", chapters.len() - 5);
            }
        }

        if fixture.expected_min_chapters == 0 {
            skip += 1;
            details.push(format!(
                "  ? chapters: got {} (not seeded — run `test --update`)",
                chapters.len()
            ));
        } else if chapters.len() >= fixture.expected_min_chapters {
            pass += 1;
            details.push(format!(
                "  ✓ chapters: {} (≥ {})",
                chapters.len(),
                fixture.expected_min_chapters
            ));
        } else {
            fail += 1;
            details.push(format!(
                "  ✗ chapters: {} (expected ≥ {})",
                chapters.len(),
                fixture.expected_min_chapters
            ));
        }

        // ── Check 3: first chapter metadata ──
        if let Some(ref expected) = fixture.first_chapter {
            if let Some(live) = chapters.first() {
                let mut diffs: Vec<String> = Vec::new();
                macro_rules! cmp_field {
                    ($label:expr, $exp:expr, $live:expr) => {
                        if $exp != $live {
                            diffs.push(format!(
                                "      {}: {:?} → {:?}",
                                $label, $exp, $live
                            ));
                        }
                    };
                }
                cmp_field!("raw_number", &expected.raw_number, &live.raw_number);
                cmp_field!("title", &expected.title, &live.title);
                cmp_field!("scanlator_group", &expected.scanlator_group, &live.scanlator_group);
                cmp_field!("language", &expected.language, &live.language);
                cmp_field!("date_released", &expected.date_released, &live.date_released);
                cmp_field!("url", &expected.url, &live.url);

                if diffs.is_empty() {
                    pass += 1;
                    details.push(format!(
                        "  ✓ first chapter: Ch.{} metadata unchanged",
                        live.raw_number
                    ));
                } else {
                    fail += 1;
                    details.push(format!(
                        "  ✗ first chapter: Ch.{} metadata changed:",
                        live.raw_number
                    ));
                    details.extend(diffs);
                }
            }
        } else {
            skip += 1;
            details.push("  ? first chapter: not seeded — run `test --update`".into());
        }

        // ── Check 4: pages ──
        if fixture.test_chapter_url.is_empty() {
            skip += 1;
            details.push("  ? pages: not seeded — run `test --update`".into());
        } else {
            let pages = match provider.pages(ctx, &fixture.test_chapter_url).await {
                Ok(p) => p,
                Err(e) => {
                    fail += 1;
                    details.push(format!("  ✗ pages: {e}"));
                    print_validate_result(pass, fail, skip, &details);
                    total_fail += 1;
                    continue;
                }
            };

            if verbose {
                println!("  pages ({}) for {}:", pages.len(), fixture.test_chapter_url);
                for p in pages.iter().take(5) {
                    println!("    page {:>3} — {}", p.index, p.url);
                }
                if pages.len() > 5 {
                    println!("    … and {} more", pages.len() - 5);
                }
            }

            if fixture.expected_min_pages == 0 || pages.len() >= fixture.expected_min_pages {
                pass += 1;
                details.push(format!(
                    "  ✓ pages: {} (≥ {})",
                    pages.len(),
                    fixture.expected_min_pages
                ));
            } else {
                fail += 1;
                details.push(format!(
                    "  ✗ pages: {} (expected ≥ {})",
                    pages.len(),
                    fixture.expected_min_pages
                ));
            }
        }

        print_validate_result(pass, fail, skip, &details);
        if fail > 0 {
            total_fail += 1;
        } else if pass > 0 {
            total_pass += 1;
        } else {
            total_seed += 1;
        }
        println!();
    }

    let tested = total_pass + total_fail;
    let summary = if total_seed > 0 {
        format!("{total_pass}/{tested} passed, {total_fail} failed, {total_seed} not seeded (run `test --update` to seed them)")
    } else {
        format!("{total_pass}/{tested} passed, {total_fail} failed")
    };
    println!("\n{summary}");
}

fn print_validate_result(pass: usize, fail: usize, skip: usize, details: &[String]) {
    let seeded = pass + fail;
    if fail == 0 && seeded == 0 {
        println!(" SEED ({skip} checks not seeded)");
    } else if fail == 0 {
        let suffix = if skip > 0 { format!(", {skip} not seeded") } else { String::new() };
        println!(" PASS ({pass}/{seeded} checks{suffix})");
    } else {
        println!(" FAIL ({pass}/{seeded} checks, {skip} not seeded)");
    }
    for d in details {
        println!("{d}");
    }
}
