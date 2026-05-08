//! Rebarr CLI platform.

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

use clap::{ArgAction, CommandFactory, Parser, Subcommand};
use indicatif::{MultiProgress, ProgressBar, ProgressDrawTarget, ProgressStyle};
use rebarr::scraper::{
    Provider, ProviderChapterInfo, ProviderRegistry, ProviderSearchResult, ScraperCtx,
    ScraperDebugLevel, browser::BrowserPool, executor::ProviderExecutor,
};
use serde::{Deserialize, Serialize};
use strsim::jaro_winkler;
use tokio_util::sync::CancellationToken;
use tracing::{error, warn};
use tracing_subscriber::fmt::writer::MakeWriter;

static ACTIVE_PROGRESS: OnceLock<Mutex<Option<MultiProgress>>> = OnceLock::new();

#[derive(Parser)]
#[command(name = "rebarr", about = "Rebarr CLI testing and provider tooling")]
struct Cli {
    /// Run Chromium headless. Visible Chromium is the default because Cloudflare can fail headless.
    #[arg(short = 'n', long, global = true)]
    headless: bool,

    /// Dump page HTML to ./scraper_dump_N.html after each open step.
    #[arg(short = 'd', long = "dump-html", global = true)]
    dump_html: bool,

    /// Keep Chromium open after the command exits.
    #[arg(short = 'k', long, global = true)]
    keep_open: bool,

    /// Increase log/debug output. Use -vv for full provider traces.
    #[arg(short = 'v', long = "verbose", global = true, action = ArgAction::Count)]
    verbose: u8,

    #[command(subcommand)]
    command: Option<Cmd>,
}

#[derive(Subcommand)]
enum Cmd {
    /// Search every provider and download one chapter or a chapter range.
    Dl {
        /// Manga title to search for.
        name: String,
        /// Chapter number or range, for example 1 or 1:99.
        chapter: String,
        /// Output directory.
        #[arg(long, default_value = "./downloads")]
        out: String,
        /// Skip the confirmation prompt.
        #[arg(short = 'y', long)]
        yes: bool,
        /// Restrict search to a single provider by name (e.g. MangaDex).
        #[arg(short = 'p', long)]
        provider: Option<String>,
    },

    /// Run provider fixture tests or create a fixture interactively.
    Test {
        /// Provider name, or "all".
        provider: String,
        /// Search term used when creating/updating a fixture interactively.
        search_term: Option<String>,
        /// Fixture directory.
        #[arg(long, default_value = "./test_fixtures")]
        fixtures: String,
    },

    /// Provider management commands.
    Provider {
        #[command(subcommand)]
        command: ProviderCmd,
    },

    /// Future experimental TUI client.
    Cli,
}

#[derive(Subcommand)]
enum ProviderCmd {
    /// List valid providers and invalid provider YAML files.
    List,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
struct ChapterFixture {
    raw_number: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    scanlator_group: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    language: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    date_released: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
struct Fixture {
    provider: String,
    query: String,
    #[serde(default)]
    expected_search_title: String,
    #[serde(default)]
    expected_min_chapters: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    first_chapter: Option<ChapterFixture>,
    #[serde(default)]
    test_chapter_url: String,
    #[serde(default)]
    expected_min_pages: usize,
}

#[derive(Clone)]
struct ProviderCheck {
    provider: Arc<dyn Provider>,
    manga: ProviderSearchResult,
    match_score: f64,
    chapters: Vec<ProviderChapterInfo>,
    matches: Vec<ChapterPick>,
}

#[derive(Clone)]
struct ChapterPick {
    requested: f32,
    chapter: ProviderChapterInfo,
}

struct CliProgress {
    multi: MultiProgress,
    bar: ProgressBar,
}

struct DownloadReport {
    successes: Vec<DownloadSuccess>,
    failures: Vec<DownloadFailure>,
}

struct DownloadSuccess {
    chapter: String,
    requested: String,
    provider: String,
    pages: usize,
    path: PathBuf,
}

struct DownloadFailure {
    chapter: String,
    provider: String,
    reason: String,
}

struct FixtureReport {
    provider: String,
    outcome: FixtureOutcome,
    reasons: Vec<String>,
}

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let filter = match cli.verbose {
        0 => "warn",
        1 => "info",
        _ => "debug",
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(filter)),
        )
        .with_writer(ProgressLogMakeWriter)
        .init();

    if cli.headless {
        unsafe { std::env::set_var("CHROME_HEADLESS", "true") };
    } else {
        unsafe { std::env::set_var("CHROME_HEADLESS", "false") };
    }

    let registry = ProviderRegistry::load().await.unwrap_or_else(|e| {
        error!("Failed to load providers: {e}");
        std::process::exit(1);
    });

    let command = match cli.command {
        Some(command) => command,
        None => {
            Cli::command().print_help().ok();
            println!();
            return;
        }
    };

    if matches!(
        command,
        Cmd::Provider {
            command: ProviderCmd::List
        } | Cmd::Cli
    ) {
        match command {
            Cmd::Provider {
                command: ProviderCmd::List,
            } => cmd_provider_list(&registry),
            Cmd::Cli => {
                eprintln!("rebarr cli is coming soon.");
                std::process::exit(2);
            }
            _ => unreachable!(),
        }
        return;
    }

    if registry.is_empty() {
        error!("No valid providers loaded. Run `rebarr provider list` for details.");
        std::process::exit(1);
    }

    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .expect("failed to build HTTP client");
    let executor = Arc::new(ProviderExecutor::new(&registry, 3));
    let mut ctx = ScraperCtx::new(http, BrowserPool::new(), executor);
    ctx.dump_html = cli.dump_html;
    ctx.set_debug_level(match cli.verbose {
        0 => ScraperDebugLevel::Off,
        1 => ScraperDebugLevel::Summary,
        _ => ScraperDebugLevel::Verbose,
    });

    match command {
        Cmd::Dl {
            name,
            chapter,
            out,
            yes,
            provider,
        } => cmd_dl(&registry, &ctx, &name, &chapter, &out, yes, provider.as_deref()).await,
        Cmd::Test {
            provider,
            search_term,
            fixtures,
        } => {
            cmd_test(
                &registry,
                &ctx,
                &provider,
                search_term.as_deref(),
                Path::new(&fixtures),
                cli.verbose,
            )
            .await
        }
        Cmd::Provider { .. } | Cmd::Cli => unreachable!(),
    }

    if cli.keep_open {
        println!("Chromium left open. Kill this terminal to close it.");
        std::process::exit(0);
    }
}

fn cmd_provider_list(registry: &ProviderRegistry) {
    let all = registry.all();
    println!("Valid providers");
    if all.is_empty() {
        println!("  none");
    } else {
        println!("{:<24} {:>5}  {:<8}  Tags", "Name", "RPM", "Version");
        println!("{}", "-".repeat(78));
        for provider in all {
            let tags = provider
                .tags()
                .iter()
                .map(|tag| format!("{tag:?}"))
                .collect::<Vec<_>>()
                .join(", ");
            println!(
                "{:<24} {:>5}  {:<8}  {}",
                provider.name(),
                provider.rate_limit_rpm(),
                provider.version().unwrap_or("-"),
                tags
            );
        }
    }

    println!("\nInvalid provider YAML");
    if registry.invalid_configs().is_empty() {
        println!("  none");
    } else {
        for invalid in registry.invalid_configs() {
            println!("  {} - {}", invalid.path.display(), invalid.error);
        }
    }
}

async fn cmd_dl(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    query: &str,
    chapter_arg: &str,
    out: &str,
    assume_yes: bool,
    provider_filter: Option<&str>,
) {
    let requested = parse_chapter_arg(chapter_arg).unwrap_or_else(|e| {
        error!("{e}");
        std::process::exit(1);
    });

    if let Some(name) = provider_filter {
        if !registry.all().iter().any(|p| p.name().eq_ignore_ascii_case(name)) {
            error!("Unknown provider {name:?}. Run `rebarr provider list` to see available providers.");
            std::process::exit(1);
        }
    }

    let progress = CliProgress::new("download", requested.len() as u64);
    let checks = collect_provider_checks(registry, ctx, query, &requested, &progress, provider_filter).await;
    progress.clear();
    if checks.is_empty() {
        error!("No provider found usable chapters for {query:?}.");
        std::process::exit(1);
    }

    print_download_alternatives(&checks, requested.len());
    let best = checks.first().expect("checked above").clone();

    if !assume_yes {
        println!(
            "\nBest candidate: {} -> {} chapter(s) from {:?}",
            best.provider.name(),
            best.matches.len(),
            best.manga.title
        );
        if !confirm("Download using this provider? [y/N] ") {
            println!("Cancelled.");
            return;
        }
    }

    let progress = CliProgress::new("download chapters", requested.len() as u64);
    let mut report = DownloadReport {
        successes: Vec::new(),
        failures: Vec::new(),
    };
    for wanted in &requested {
        if !best
            .matches
            .iter()
            .any(|pick| (pick.requested - *wanted).abs() <= 0.01)
        {
            let chapter = format_chapter(*wanted);
            progress.log(format!(
                "{}: requested chapter {} was not found",
                best.provider.name(),
                chapter
            ));
            report.failures.push(DownloadFailure {
                chapter,
                provider: best.provider.name().to_owned(),
                reason: "selected provider did not return this chapter".to_owned(),
            });
            progress.inc(1);
        }
    }
    for pick in &best.matches {
        match download_pick(ctx, &best.provider, &best.manga, pick, out, &progress).await {
            Ok(success) => report.successes.push(success),
            Err(failure) => report.failures.push(failure),
        }
        progress.inc(1);
    }
    progress.clear();
    print_download_summary(&report);
    if !report.failures.is_empty() {
        std::process::exit(1);
    }
}

async fn collect_provider_checks(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    query: &str,
    requested: &[f32],
    progress: &CliProgress,
    provider_filter: Option<&str>,
) -> Vec<ProviderCheck> {
    let all = registry.all();
    let providers: Vec<_> = match provider_filter {
        Some(name) => all.iter().filter(|p| p.name().eq_ignore_ascii_case(name)).cloned().collect(),
        None => all.to_vec(),
    };
    progress.set_length(providers.len() as u64);
    progress.set_task("Providers", format!("searching for {query:?}"));

    let query_lower = query.to_lowercase();
    let mut checks = Vec::new();
    for provider_ref in &providers {
        let provider = Arc::clone(provider_ref);
        progress.set_task(provider.name(), "search");

        let results = match provider.search(ctx, query).await {
            Ok(results) if !results.is_empty() => results,
            Ok(_) => {
                progress.log(format!("{}: no search results", provider.name()));
                progress.inc(1);
                continue;
            }
            Err(e) => {
                progress.log(format!("{}: search failed: {e}", provider.name()));
                progress.inc(1);
                continue;
            }
        };

        let (best_idx, score) = best_match(&results, &query_lower);
        let manga = results[best_idx].clone();
        progress.set_task(provider.name(), format!("chapters for {}", manga.title));
        let chapters = match provider.chapters(ctx, &manga.url, &manga.variables).await {
            Ok(chapters) if !chapters.is_empty() => chapters,
            Ok(_) => {
                progress.log(format!("{}: no chapters returned", provider.name()));
                progress.inc(1);
                continue;
            }
            Err(e) => {
                progress.log(format!("{}: chapters failed: {e}", provider.name()));
                progress.inc(1);
                continue;
            }
        };

        let matches = requested
            .iter()
            .filter_map(|wanted| {
                find_chapter(&chapters, *wanted).map(|chapter| ChapterPick {
                    requested: *wanted,
                    chapter: chapter.clone(),
                })
            })
            .collect::<Vec<_>>();

        if !matches.is_empty() {
            progress.log(format!(
                "{}: found {}/{} requested chapter(s)",
                provider.name(),
                matches.len(),
                requested.len()
            ));
            checks.push(ProviderCheck {
                provider,
                manga,
                match_score: score,
                chapters,
                matches,
            });
        }
        progress.inc(1);
    }

    checks.sort_by(|a, b| {
        b.matches
            .len()
            .cmp(&a.matches.len())
            .then_with(|| b.match_score.partial_cmp(&a.match_score).unwrap())
            .then_with(|| b.chapters.len().cmp(&a.chapters.len()))
    });
    checks
}

fn print_download_alternatives(checks: &[ProviderCheck], requested_count: usize) {
    println!(
        "{:<24} {:>7} {:>9} {:>10}  Match",
        "Provider", "Match%", "Chapters", "Requested"
    );
    println!("{}", "-".repeat(90));
    for check in checks {
        let matched = check
            .matches
            .iter()
            .map(|pick| pick.chapter.raw_number.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        println!(
            "{:<24} {:>6.0}% {:>9} {:>5}/{}     {}",
            check.provider.name(),
            check.match_score * 100.0,
            check.chapters.len(),
            check.matches.len(),
            requested_count,
            matched
        );
    }
}

async fn download_pick(
    ctx: &ScraperCtx,
    provider: &Arc<dyn Provider>,
    manga: &ProviderSearchResult,
    pick: &ChapterPick,
    out: &str,
    progress: &CliProgress,
) -> Result<DownloadSuccess, DownloadFailure> {
    let chapter_label = pick.chapter.raw_number.clone();
    let requested = format_chapter(pick.requested);
    let Some(chapter_url) = pick.chapter.url.as_deref() else {
        return Err(DownloadFailure {
            chapter: chapter_label,
            provider: provider.name().to_owned(),
            reason: "chapter has no URL".to_owned(),
        });
    };

    progress.set_task(
        provider.name(),
        format!("chapter {} resolving pages", pick.chapter.raw_number),
    );

    let pages = match provider.pages(ctx, chapter_url).await {
        Ok(pages) => pages,
        Err(e) => {
            return Err(DownloadFailure {
                chapter: chapter_label,
                provider: provider.name().to_owned(),
                reason: format!("pages() failed: {e}"),
            });
        }
    };
    if pages.is_empty() {
        return Err(DownloadFailure {
            chapter: chapter_label,
            provider: provider.name().to_owned(),
            reason: "no pages found".to_owned(),
        });
    }

    progress.log(format!(
        "{} chapter {}: {} page(s)",
        provider.name(),
        pick.chapter.raw_number,
        pages.len()
    ));
    progress.set_task(
        provider.name(),
        format!(
            "chapter {} downloading {} pages",
            pick.chapter.raw_number,
            pages.len()
        ),
    );
    let image_data = rebarr::scraper::downloader::download_pages_via_browser(
        None,
        None,
        ctx,
        Some(provider.name()),
        &pages,
        chapter_url,
        provider.pages_download_method(),
        CancellationToken::new(),
    )
    .await
    .map_err(|e| DownloadFailure {
        chapter: chapter_label.clone(),
        provider: provider.name().to_owned(),
        reason: format!("download failed: {e}"),
    })?;

    let out_dir = PathBuf::from(out)
        .join(safe_path(provider.name()))
        .join(safe_path(&manga.title))
        .join(format!("Ch.{}", safe_path(&pick.chapter.raw_number)));
    tokio::fs::create_dir_all(&out_dir)
        .await
        .map_err(|e| DownloadFailure {
            chapter: chapter_label.clone(),
            provider: provider.name().to_owned(),
            reason: format!("failed to create output directory: {e}"),
        })?;

    progress.set_task(
        provider.name(),
        format!("chapter {} writing files", pick.chapter.raw_number),
    );
    for (index, data) in &image_data {
        let ext = rebarr::scraper::downloader::image_ext(data);
        let path = out_dir.join(format!("{index:03}.{ext}"));
        tokio::fs::write(&path, data)
            .await
            .map_err(|e| DownloadFailure {
                chapter: chapter_label.clone(),
                provider: provider.name().to_owned(),
                reason: format!("failed to write page {index}: {e}"),
            })?;
    }

    progress.log(format!(
        "{} chapter {}: downloaded to {}",
        provider.name(),
        pick.chapter.raw_number,
        out_dir.display()
    ));

    Ok(DownloadSuccess {
        chapter: chapter_label,
        requested,
        provider: provider.name().to_owned(),
        pages: image_data.len(),
        path: out_dir,
    })
}

async fn cmd_test(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    provider_arg: &str,
    search_term: Option<&str>,
    fixtures_path: &Path,
    verbose: u8,
) {
    let provider_arg_all = provider_arg.eq_ignore_ascii_case("all");
    if search_term.is_some() && provider_arg_all {
        error!("Interactive fixture creation requires a specific provider, not `all`.");
        std::process::exit(1);
    }

    if let Some(query) = search_term {
        let provider = find_provider(registry, provider_arg).unwrap_or_else(|| {
            error!("Provider {provider_arg:?} not found. Run `rebarr provider list`.");
            std::process::exit(1);
        });
        fixture_create_interactive(ctx, &provider, query, fixtures_path).await;
        return;
    }

    fixture_run(registry, ctx, provider_arg, fixtures_path, verbose).await;
}

async fn fixture_create_interactive(
    ctx: &ScraperCtx,
    provider: &Arc<dyn Provider>,
    query: &str,
    fixtures_path: &Path,
) {
    tokio::fs::create_dir_all(fixtures_path)
        .await
        .expect("failed to create fixtures directory");

    let fixture_file = fixtures_path.join(format!("{}.yaml", provider.name()));
    if fixture_file.exists()
        && !confirm(&format!(
            "{} already exists. Replace it? [y/N] ",
            fixture_file.display()
        ))
    {
        println!("Cancelled.");
        return;
    }

    let pb = ProgressBar::new_spinner();
    pb.set_style(spinner_style());
    pb.set_message(format!("searching {} for {query:?}", provider.name()));
    let results = provider.search(ctx, query).await.unwrap_or_else(|e| {
        pb.finish_and_clear();
        error!("Search failed: {e}");
        std::process::exit(1);
    });
    pb.finish_and_clear();

    if results.is_empty() {
        error!("No search results found for {query:?}.");
        std::process::exit(1);
    }

    let manga = prompt_search_result(&results, query);

    let pb = ProgressBar::new_spinner();
    pb.set_style(spinner_style());
    pb.set_message("fetching chapters");
    let chapters = provider
        .chapters(ctx, &manga.url, &manga.variables)
        .await
        .unwrap_or_else(|e| {
            pb.finish_and_clear();
            error!("chapters() failed: {e}");
            std::process::exit(1);
        });
    pb.finish_and_clear();

    if chapters.is_empty() {
        error!("No chapters found for selected result.");
        std::process::exit(1);
    }

    println!("\nFound {} chapters. First 20:", chapters.len());
    for (idx, chapter) in chapters.iter().take(20).enumerate() {
        println!(
            "  [{idx}] Ch.{} - {}",
            chapter.raw_number,
            chapter.title.as_deref().unwrap_or("(no title)")
        );
    }
    let chapter_idx = prompt_index("Fixture page-test chapter index", chapters.len(), 0);
    let test_chapter = &chapters[chapter_idx];
    let chapter_url = test_chapter.url.as_ref().unwrap_or_else(|| {
        error!("Selected chapter has no URL.");
        std::process::exit(1);
    });

    let pb = ProgressBar::new_spinner();
    pb.set_style(spinner_style());
    pb.set_message("fetching pages");
    let pages = provider.pages(ctx, chapter_url).await.unwrap_or_else(|e| {
        pb.finish_and_clear();
        error!("pages() failed: {e}");
        std::process::exit(1);
    });
    pb.finish_and_clear();

    let first = chapters.first().expect("checked non-empty");
    let fixture = Fixture {
        provider: provider.name().to_owned(),
        query: query.to_owned(),
        expected_search_title: manga.title.clone(),
        expected_min_chapters: chapters.len().saturating_sub(10),
        first_chapter: Some(ChapterFixture {
            raw_number: first.raw_number.clone(),
            title: first.title.clone(),
            scanlator_group: first.scanlator_group.clone(),
            language: first.language.clone(),
            date_released: first.date_released,
            url: first.url.clone(),
        }),
        test_chapter_url: chapter_url.clone(),
        expected_min_pages: pages.len().saturating_sub(3).max(1),
    };

    println!("\nFixture preview");
    println!("  provider: {}", fixture.provider);
    println!("  query: {}", fixture.query);
    println!("  expected_search_title: {}", fixture.expected_search_title);
    println!("  expected_min_chapters: {}", fixture.expected_min_chapters);
    println!("  test chapter: Ch.{}", test_chapter.raw_number);
    println!("  expected_min_pages: {}", fixture.expected_min_pages);

    if !confirm("Write this fixture? [y/N] ") {
        println!("Cancelled.");
        return;
    }

    let yaml = serde_yaml::to_string(&fixture).expect("failed to serialize fixture");
    tokio::fs::write(&fixture_file, yaml)
        .await
        .expect("failed to write fixture");
    println!("Wrote {}", fixture_file.display());
}

async fn fixture_run(
    registry: &ProviderRegistry,
    ctx: &ScraperCtx,
    provider_arg: &str,
    fixtures_path: &Path,
    verbose: u8,
) {
    let fixtures = load_fixtures(fixtures_path).await;
    if fixtures.is_empty() {
        println!("No fixture files found in {}.", fixtures_path.display());
        return;
    }

    let mut selected = fixtures;
    if !provider_arg.eq_ignore_ascii_case("all") {
        selected.retain(|fixture| fixture.provider.eq_ignore_ascii_case(provider_arg));
    }

    if selected.is_empty() {
        println!("No fixture found for {provider_arg:?}; skipping.");
        return;
    }

    let progress = CliProgress::new("fixture tests", selected.len() as u64);
    let mut reports = Vec::new();
    for fixture in selected {
        progress.set_task(&fixture.provider, "starting");
        let Some(provider) = find_provider(registry, &fixture.provider) else {
            progress.log(format!("{}: provider not loaded", fixture.provider));
            reports.push(FixtureReport {
                provider: fixture.provider,
                outcome: FixtureOutcome::SeedOnly,
                reasons: vec!["provider not loaded".to_owned()],
            });
            progress.inc(1);
            continue;
        };

        reports.push(validate_fixture(ctx, &provider, &fixture, verbose, &progress).await);
        progress.inc(1);
    }
    progress.clear();
    print_fixture_summary(&reports);
    if reports
        .iter()
        .any(|report| matches!(report.outcome, FixtureOutcome::Fail))
    {
        std::process::exit(1);
    }
}

#[derive(Clone, Copy)]
enum FixtureOutcome {
    Pass,
    Fail,
    SeedOnly,
}

async fn validate_fixture(
    ctx: &ScraperCtx,
    provider: &Arc<dyn Provider>,
    fixture: &Fixture,
    verbose: u8,
    progress: &CliProgress,
) -> FixtureReport {
    progress.set_task(&fixture.provider, "search");

    let results = match provider.search(ctx, &fixture.query).await {
        Ok(results) => results,
        Err(e) => {
            progress.log(format!("{}: search failed", fixture.provider));
            return FixtureReport {
                provider: fixture.provider.clone(),
                outcome: FixtureOutcome::Fail,
                reasons: vec![format!("search: {e}")],
            };
        }
    };

    if results.is_empty() {
        progress.log(format!("{}: no search results", fixture.provider));
        return FixtureReport {
            provider: fixture.provider.clone(),
            outcome: FixtureOutcome::Fail,
            reasons: vec!["search: no results".to_owned()],
        };
    }

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut skipped = 0usize;
    let mut details = Vec::new();

    if fixture.expected_search_title.is_empty() {
        skipped += 1;
        details.push(format!("  ? search: {} results, not seeded", results.len()));
    } else {
        let expected = fixture.expected_search_title.to_lowercase();
        let best = results
            .iter()
            .map(|result| jaro_winkler(&expected, &result.title.to_lowercase()))
            .fold(0.0_f64, f64::max);
        if best >= 0.85 {
            pass += 1;
            details.push(format!("  ok search: {:.0}% match", best * 100.0));
        } else {
            fail += 1;
            details.push(format!("  fail search: best match {:.0}%", best * 100.0));
        }
    }

    let (best_idx, _) = best_match(&results, &fixture.query.to_lowercase());
    let manga = &results[best_idx];
    progress.set_task(&fixture.provider, format!("chapters for {}", manga.title));
    let chapters = match provider.chapters(ctx, &manga.url, &manga.variables).await {
        Ok(chapters) => chapters,
        Err(e) => {
            progress.log(format!("{}: chapters failed", fixture.provider));
            return FixtureReport {
                provider: fixture.provider.clone(),
                outcome: FixtureOutcome::Fail,
                reasons: vec![format!("chapters: {e}")],
            };
        }
    };

    if verbose > 0 {
        progress.log(format!(
            "{}: selected {} - {}",
            fixture.provider, manga.title, manga.url
        ));
    }

    if fixture.expected_min_chapters == 0 {
        skipped += 1;
        details.push(format!("  ? chapters: {}, not seeded", chapters.len()));
    } else if chapters.len() >= fixture.expected_min_chapters {
        pass += 1;
        details.push(format!(
            "  ok chapters: {} >= {}",
            chapters.len(),
            fixture.expected_min_chapters
        ));
    } else {
        fail += 1;
        details.push(format!(
            "  fail chapters: {} < {}",
            chapters.len(),
            fixture.expected_min_chapters
        ));
    }

    if let Some(expected) = &fixture.first_chapter {
        if let Some(live) = chapters.first() {
            if expected.raw_number == live.raw_number
                && expected.title == live.title
                && expected.scanlator_group == live.scanlator_group
                && expected.language == live.language
                && expected.date_released == live.date_released
                && expected.url == live.url
            {
                pass += 1;
                details.push(format!("  ok first chapter: Ch.{}", live.raw_number));
            } else {
                fail += 1;
                details.push(format!(
                    "  fail first chapter: expected Ch.{}",
                    expected.raw_number
                ));
            }
        }
    } else {
        skipped += 1;
        details.push("  ? first chapter: not seeded".to_owned());
    }

    if fixture.test_chapter_url.is_empty() {
        skipped += 1;
        details.push("  ? pages: not seeded".to_owned());
    } else {
        progress.set_task(&fixture.provider, "pages");
        match provider.pages(ctx, &fixture.test_chapter_url).await {
            Ok(pages)
                if fixture.expected_min_pages == 0 || pages.len() >= fixture.expected_min_pages =>
            {
                pass += 1;
                details.push(format!(
                    "  ok pages: {} >= {}",
                    pages.len(),
                    fixture.expected_min_pages
                ));
            }
            Ok(pages) => {
                fail += 1;
                details.push(format!(
                    "  fail pages: {} < {}",
                    pages.len(),
                    fixture.expected_min_pages
                ));
            }
            Err(e) => {
                fail += 1;
                details.push(format!("  fail pages: {e}"));
            }
        }
    }

    let outcome = if fail > 0 {
        progress.log(format!(
            "{}: failed ({pass} passed, {fail} failed, {skipped} skipped)",
            fixture.provider
        ));
        FixtureOutcome::Fail
    } else if pass > 0 {
        progress.log(format!(
            "{}: passed ({pass} checks, {skipped} skipped)",
            fixture.provider
        ));
        FixtureOutcome::Pass
    } else {
        progress.log(format!(
            "{}: seed-only ({skipped} not seeded)",
            fixture.provider
        ));
        FixtureOutcome::SeedOnly
    };

    FixtureReport {
        provider: fixture.provider.clone(),
        outcome,
        reasons: details,
    }
}

async fn load_fixtures(fixtures_path: &Path) -> Vec<Fixture> {
    let mut fixtures = Vec::new();
    let mut dir = match tokio::fs::read_dir(fixtures_path).await {
        Ok(dir) => dir,
        Err(_) => return fixtures,
    };

    while let Ok(Some(entry)) = dir.next_entry().await {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) != Some("yaml") {
            continue;
        }
        match tokio::fs::read_to_string(&path).await {
            Ok(content) => match serde_yaml::from_str::<Fixture>(&content) {
                Ok(fixture) => fixtures.push(fixture),
                Err(e) => warn!("Skipping malformed fixture {}: {e}", path.display()),
            },
            Err(e) => warn!("Failed to read fixture {}: {e}", path.display()),
        }
    }
    fixtures
}

fn find_provider(registry: &ProviderRegistry, name: &str) -> Option<Arc<dyn Provider>> {
    registry
        .all()
        .into_iter()
        .find(|provider| provider.name().eq_ignore_ascii_case(name))
        .map(Arc::clone)
}

fn best_match(results: &[ProviderSearchResult], query_lower: &str) -> (usize, f64) {
    results
        .iter()
        .enumerate()
        .map(|(idx, result)| (idx, jaro_winkler(query_lower, &result.title.to_lowercase())))
        .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
        .unwrap_or((0, 0.0))
}

fn find_chapter(chapters: &[ProviderChapterInfo], requested: f32) -> Option<&ProviderChapterInfo> {
    chapters
        .iter()
        .filter(|chapter| (chapter.number - requested).abs() <= 0.01)
        .min_by(|a, b| a.number.partial_cmp(&b.number).unwrap())
}

fn parse_chapter_arg(raw: &str) -> Result<Vec<f32>, String> {
    if let Some((start, end)) = raw.split_once(':') {
        let start = start
            .parse::<u32>()
            .map_err(|_| format!("Invalid chapter range start: {start:?}"))?;
        let end = end
            .parse::<u32>()
            .map_err(|_| format!("Invalid chapter range end: {end:?}"))?;
        if start > end {
            return Err(format!("Invalid chapter range {raw:?}: start is after end"));
        }
        return Ok((start..=end).map(|value| value as f32).collect());
    }

    let chapter = raw
        .parse::<f32>()
        .map_err(|_| format!("Invalid chapter number: {raw:?}"))?;
    Ok(vec![chapter])
}

fn prompt_search_result(results: &[ProviderSearchResult], query: &str) -> ProviderSearchResult {
    let query_lower = query.to_lowercase();
    let mut scored = results
        .iter()
        .enumerate()
        .map(|(idx, result)| {
            (
                idx,
                jaro_winkler(&query_lower, &result.title.to_lowercase()),
            )
        })
        .collect::<Vec<_>>();
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());

    println!("Search results");
    for (rank, (idx, score)) in scored.iter().take(20).enumerate() {
        let result = &results[*idx];
        println!(
            "  [{rank}] {:>3.0}%  {} - {}",
            score * 100.0,
            result.title,
            result.url
        );
    }

    let chosen = prompt_index("Series index", scored.len().min(20), 0);
    results[scored[chosen].0].clone()
}

fn prompt_index(label: &str, len: usize, default: usize) -> usize {
    print!("{label} [default {default}]: ");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("failed to read input");
    input
        .trim()
        .parse::<usize>()
        .unwrap_or(default)
        .min(len.saturating_sub(1))
}

fn confirm(prompt: &str) -> bool {
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .expect("failed to read input");
    matches!(input.trim().to_lowercase().as_str(), "y" | "yes")
}

fn safe_path(raw: &str) -> String {
    raw.chars()
        .map(|ch| {
            if ch.is_alphanumeric() || ch == ' ' || ch == '-' || ch == '.' {
                ch
            } else {
                '_'
            }
        })
        .collect()
}

fn format_chapter(chapter: f32) -> String {
    if chapter.fract() == 0.0 {
        format!("{}", chapter as i32)
    } else {
        format!("{chapter}")
    }
}

impl CliProgress {
    fn new(label: impl Into<String>, len: u64) -> Self {
        let multi = MultiProgress::with_draw_target(ProgressDrawTarget::stderr_with_hz(10));
        let bar = multi.add(ProgressBar::new(len));
        bar.set_style(action_bar_style());
        bar.set_message(label.into());

        *ACTIVE_PROGRESS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("active progress lock poisoned") = Some(multi.clone());

        Self { multi, bar }
    }

    fn set_task(&self, provider: impl AsRef<str>, task: impl AsRef<str>) {
        self.bar
            .set_message(format!("{}: {}", provider.as_ref(), task.as_ref()));
        self.bar.tick();
    }

    fn set_length(&self, len: u64) {
        self.bar.set_length(len);
    }

    fn inc(&self, delta: u64) {
        self.bar.inc(delta);
    }

    fn log(&self, message: impl AsRef<str>) {
        let _ = self.multi.println(message.as_ref());
    }

    fn clear(&self) {
        self.bar.finish_and_clear();
        *ACTIVE_PROGRESS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("active progress lock poisoned") = None;
    }
}

#[derive(Clone, Copy)]
struct ProgressLogMakeWriter;

impl<'a> MakeWriter<'a> for ProgressLogMakeWriter {
    type Writer = ProgressLogWriter;

    fn make_writer(&'a self) -> Self::Writer {
        ProgressLogWriter { buf: Vec::new() }
    }
}

struct ProgressLogWriter {
    buf: Vec<u8>,
}

impl std::io::Write for ProgressLogWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.flush_inner();
        Ok(())
    }
}

impl Drop for ProgressLogWriter {
    fn drop(&mut self) {
        self.flush_inner();
    }
}

impl ProgressLogWriter {
    fn flush_inner(&mut self) {
        if self.buf.is_empty() {
            return;
        }

        let line = String::from_utf8_lossy(&self.buf).trim_end().to_owned();
        self.buf.clear();
        if line.is_empty() {
            return;
        }

        if let Some(multi) = ACTIVE_PROGRESS
            .get_or_init(|| Mutex::new(None))
            .lock()
            .expect("active progress lock poisoned")
            .clone()
        {
            let _ = multi.println(line);
        } else {
            eprintln!("{line}");
        }
    }
}

fn print_fixture_summary(reports: &[FixtureReport]) {
    let passed = reports
        .iter()
        .filter(|report| matches!(report.outcome, FixtureOutcome::Pass))
        .count();
    let failed = reports
        .iter()
        .filter(|report| matches!(report.outcome, FixtureOutcome::Fail))
        .count();
    let skipped = reports
        .iter()
        .filter(|report| matches!(report.outcome, FixtureOutcome::SeedOnly))
        .count();

    println!();
    println!("{}", bold("Provider Test Summary"));
    println!(
        "{}  {}  {}",
        green(format!("{passed} working")),
        red(format!("{failed} broken")),
        yellow(format!("{skipped} skipped/seed-only"))
    );

    println!("\n{}", green("Providers that work"));
    let mut any = false;
    for report in reports
        .iter()
        .filter(|report| matches!(report.outcome, FixtureOutcome::Pass))
    {
        any = true;
        println!("  {} {}", green("OK"), report.provider);
    }
    if !any {
        println!("  none");
    }

    println!("\n{}", red("Providers that don't"));
    let mut any = false;
    for report in reports
        .iter()
        .filter(|report| matches!(report.outcome, FixtureOutcome::Fail))
    {
        any = true;
        println!("  {} {}", red("FAIL"), report.provider);
        for reason in &report.reasons {
            println!("    {}", reason.trim());
        }
    }
    if !any {
        println!("  none");
    }

    if skipped > 0 {
        println!("\n{}", yellow("Skipped / seed-only"));
        for report in reports
            .iter()
            .filter(|report| matches!(report.outcome, FixtureOutcome::SeedOnly))
        {
            println!("  {} {}", yellow("SKIP"), report.provider);
            for reason in &report.reasons {
                println!("    {}", reason.trim());
            }
        }
    }
}

fn print_download_summary(report: &DownloadReport) {
    println!();
    println!("{}", bold("Download Summary"));
    println!(
        "{}  {}",
        green(format!("{} downloaded", report.successes.len())),
        red(format!("{} failed", report.failures.len()))
    );

    println!("\n{}", green("Downloaded chapters"));
    if report.successes.is_empty() {
        println!("  none");
    } else {
        for success in &report.successes {
            println!(
                "  {} Ch.{} (requested {}) from {} - {} page(s)",
                green("OK"),
                success.chapter,
                success.requested,
                success.provider,
                success.pages
            );
            println!("    {}", success.path.display());
        }
    }

    println!("\n{}", red("Failed chapters"));
    if report.failures.is_empty() {
        println!("  none");
    } else {
        for failure in &report.failures {
            println!(
                "  {} Ch.{} from {}",
                red("FAIL"),
                failure.chapter,
                failure.provider
            );
            println!("    {}", failure.reason);
        }
    }
}

fn bold(value: impl AsRef<str>) -> String {
    format!("\x1b[1m{}\x1b[0m", value.as_ref())
}

fn green(value: impl AsRef<str>) -> String {
    format!("\x1b[32m{}\x1b[0m", value.as_ref())
}

fn red(value: impl AsRef<str>) -> String {
    format!("\x1b[31m{}\x1b[0m", value.as_ref())
}

fn yellow(value: impl AsRef<str>) -> String {
    format!("\x1b[33m{}\x1b[0m", value.as_ref())
}

fn spinner_style() -> ProgressStyle {
    ProgressStyle::with_template("{spinner:.green} {msg}")
        .expect("valid spinner template")
        .tick_strings(&["-", "\\", "|", "/"])
}

fn action_bar_style() -> ProgressStyle {
    ProgressStyle::with_template(
        "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {msg} {pos}/{len}",
    )
    .expect("valid action bar template")
    .progress_chars("=>-")
    .tick_strings(&["-", "\\", "|", "/"])
}
