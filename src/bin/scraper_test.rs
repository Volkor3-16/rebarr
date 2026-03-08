/// Manual integration test for a single provider.
///
/// Usage:
///   cargo run --bin scraper_test -- [OPTIONS] "manga title"
///
/// Options:
///   -p, --provider <name>      Provider to test (default: highest-scored)
///   -d                         Also download the first chapter pages to ./test_dl/
///   -H, --dump-html            Dump page HTML to ./scraper_dump_N.html after each open step
///
/// Examples:
///   cargo run --bin scraper_test -- "berserk"
///   cargo run --bin scraper_test -- -p MangaFire "berserk"
///   cargo run --bin scraper_test -- -p WeebCentral -d "berserk"
///   cargo run --bin scraper_test -- -H "berserk"
///
/// What it does:
///   1. Loads providers from ./providers/ (or REBARR_PROVIDERS_DIR)
///   2. Searches the selected provider for the given query
///   3. Takes the first result, lists all chapters
///   4. Fetches pages for the first chapter
///   5. (With -d) Downloads all pages to ./test_dl/ch_<number>/
use std::process;

use rebarr::scraper::{ProviderRegistry, ScraperCtx, browser::BrowserPool};
use strsim::jaro_winkler;

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    // -------------------------------------------------------------------------
    // Arg parsing
    // -------------------------------------------------------------------------
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut provider_name: Option<String> = None;
    let mut download = false;
    let mut dump_html = false;
    let mut query: Option<String> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-p" | "--provider" => {
                i += 1;
                provider_name = Some(args.get(i).cloned().unwrap_or_else(|| {
                    eprintln!("--provider requires an argument");
                    process::exit(1);
                }));
            }
            "-d" => {
                download = true;
            }
            "-H" | "--dump-html" => {
                dump_html = true;
            }
            flag if flag.starts_with('-') => {
                eprintln!("Unknown flag: {flag}");
                eprintln!("Usage: scraper_test [-p <provider>] [-d] [-H] <query>");
                process::exit(1);
            }
            _ => {
                query = Some(args[i].clone());
            }
        }
        i += 1;
    }

    let query = query.unwrap_or_else(|| {
        eprintln!("Usage: scraper_test [-p <provider>] [-d] [-H] <query>");
        eprintln!("Example: cargo run --bin scraper_test -- -p MangaFire \"berserk\"");
        process::exit(1);
    });

    // -------------------------------------------------------------------------
    // Build context
    // -------------------------------------------------------------------------
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .expect("failed to build HTTP client");
    let mut ctx = ScraperCtx::new(http.clone(), BrowserPool::new());
    ctx.dump_html = dump_html;
    ctx.verbose = true;

    // -------------------------------------------------------------------------
    // Load providers
    // -------------------------------------------------------------------------
    log::info!("Loading providers...");
    let registry = ProviderRegistry::load().await.unwrap_or_else(|e| {
        eprintln!("Failed to load providers: {e}");
        process::exit(1);
    });

    if registry.is_empty() {
        eprintln!("No providers loaded. Make sure ./providers/ contains YAML files.");
        process::exit(1);
    }

    let all = registry.all();

    // List available providers
    println!("Available providers:");
    for p in &all {
        println!("  {}", p.name());
    }
    println!();

    // Select provider
    let provider = if let Some(ref name) = provider_name {
        all.into_iter()
            .find(|p| p.name().eq_ignore_ascii_case(name))
            .unwrap_or_else(|| {
                eprintln!("Provider {name:?} not found. Check the name above.");
                process::exit(1);
            })
    } else {
        all.into_iter().next().unwrap_or_else(|| {
            eprintln!("No providers available.");
            process::exit(1);
        })
    };

    println!("Using provider: {}\n", provider.name());

    // -------------------------------------------------------------------------
    // Search
    // -------------------------------------------------------------------------
    log::info!("Searching {:?} for {query:?}...", provider.name());
    let results = provider.search(&ctx, &query).await.unwrap_or_else(|e| {
        eprintln!("Search failed: {e}");
        process::exit(1);
    });

    if results.is_empty() {
        eprintln!("No results found for {query:?}");
        process::exit(1);
    }

    // Score and sort results by similarity to query
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

    const AUTO_SELECT_THRESHOLD: f64 = 0.90;
    let best_idx = scored[0].0;
    let best_score = scored[0].1;

    let manga = if best_score >= AUTO_SELECT_THRESHOLD {
        println!(
            "\nAuto-selecting best match ({:.0}%): {}",
            best_score * 100.0,
            results[best_idx].title
        );
        &results[best_idx]
    } else {
        print!(
            "\nNo confident match (best: {:.0}%). Enter index [0..{}] or press Enter for [0]: ",
            best_score * 100.0,
            scored.len() - 1
        );
        use std::io::{self, Write as _};
        io::stdout().flush().ok();
        let mut input = String::new();
        io::stdin()
            .read_line(&mut input)
            .expect("failed to read input");
        let chosen_rank: usize = input.trim().parse().unwrap_or(0).min(scored.len() - 1);
        let chosen_orig = scored[chosen_rank].0;
        println!("Selected: {}", results[chosen_orig].title);
        &results[chosen_orig]
    };
    println!();

    // -------------------------------------------------------------------------
    // Chapters
    // -------------------------------------------------------------------------
    log::info!("Fetching chapter list...");
    let chapters = provider
        .chapters(&ctx, &manga.url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("chapters() failed: {e}");
            process::exit(1);
        });

    println!("Found {} chapters:", chapters.len());
    for ch in chapters.iter().take(1000) {
        let title = ch.title.as_deref().unwrap_or("(no title)");
        let scanlator = ch.scanlator_group.as_deref().unwrap_or("—");
        let url = ch.url.as_deref().unwrap_or("(no url)");
        println!("  Ch.{} — {} [{}] {}", ch.number, title, scanlator, url);
    }
    if chapters.len() > 1000 {
        println!("  ... and {} more", chapters.len() - 10);
    }

    // -------------------------------------------------------------------------
    // Pages for first chapter
    // -------------------------------------------------------------------------
    let ch1 = &chapters[0];
    let chapter_url = ch1.url.as_ref().unwrap_or_else(|| {
        eprintln!("Chapter {} has no URL", ch1.number);
        process::exit(1);
    });

    println!("\nFetching pages for chapter {}...", ch1.number);
    log::info!("Calling pages() with URL: {chapter_url}");

    let pages = provider.pages(&ctx, chapter_url).await.unwrap_or_else(|e| {
        eprintln!("pages() failed: {e}");
        process::exit(1);
    });

    println!("Found {} pages:", pages.len());
    for p in pages.iter().take(5) {
        println!("  Page {} — {}", p.index, p.url);
    }
    if pages.len() > 5 {
        println!("  ... and {} more", pages.len() - 5);
    }

    if pages.is_empty() {
        eprintln!("No pages found — check the 'pages' extract config in the provider YAML");
        process::exit(1);
    }

    // -------------------------------------------------------------------------
    // Download (only with -d)
    // -------------------------------------------------------------------------
    if !download {
        println!("\n(Pass -d to also download the pages to ./test_dl/)");
        return;
    }

    let out_dir = std::path::PathBuf::from(format!("./test_dl/ch_{:.0}", ch1.number));
    tokio::fs::create_dir_all(&out_dir)
        .await
        .expect("failed to create output directory");

    println!(
        "\nDownloading {} pages to {}/ ...",
        pages.len(),
        out_dir.display()
    );

    let mut downloaded = 0usize;
    for page in &pages {
        let ext = page
            .url
            .rsplit('.')
            .next()
            .filter(|e| e.len() <= 4 && !e.contains('/'))
            .unwrap_or("jpg");
        let filename = format!("{:03}.{ext}", page.index);
        let path = out_dir.join(&filename);

        match http.get(&page.url).send().await {
            Ok(resp) => match resp.bytes().await {
                Ok(bytes) => {
                    tokio::fs::write(&path, &bytes)
                        .await
                        .expect("failed to write page file");
                    log::info!(
                        "  [{}/{}] {} → {}",
                        page.index,
                        pages.len(),
                        page.url,
                        path.display()
                    );
                    downloaded += 1;
                }
                Err(e) => eprintln!("  Page {}: failed to read bytes: {e}", page.index),
            },
            Err(e) => eprintln!("  Page {}: HTTP error: {e}", page.index),
        }
    }

    println!(
        "\nDone. Downloaded {}/{} pages to {}/",
        downloaded,
        pages.len(),
        out_dir.display()
    );
}
