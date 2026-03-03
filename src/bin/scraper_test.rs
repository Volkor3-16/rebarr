/// Manual integration test for a single provider.
///
/// Usage:
///   cargo run --bin scraper_test -- "manga title"
///
/// What it does:
///   1. Loads providers from ./providers/ (or REBARR_PROVIDERS_DIR)
///   2. Searches WeebCentral for the given query
///   3. Takes the first result, lists all chapters
///   4. Fetches pages for chapter 1
///   5. Downloads all pages to ./test_dl/ch_<number>/
use std::{path::PathBuf, process};

use rebarr::scraper::{browser::BrowserPool, ProviderRegistry, ScraperCtx};

#[tokio::main]
async fn main() {
    env_logger::builder()
        .filter_level(log::LevelFilter::Info)
        .parse_default_env()
        .init();

    let query = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("Usage: scraper_test <search query>");
        eprintln!("Example: cargo run --bin scraper_test -- \"berserk\"");
        process::exit(1);
    });

    // Build context
    let http = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36")
        .build()
        .expect("failed to build HTTP client");
    let ctx = ScraperCtx::new(http, BrowserPool::new());

    // Load providers
    log::info!("Loading providers...");
    let registry = ProviderRegistry::load().await.unwrap_or_else(|e| {
        eprintln!("Failed to load providers: {e}");
        process::exit(1);
    });

    if registry.is_empty() {
        eprintln!("No providers loaded. Make sure ./providers/ contains YAML files.");
        process::exit(1);
    }

    // Find WeebCentral
    let provider = registry
        .by_score()
        .into_iter()
        .find(|p| p.name() == "WeebCentral")
        .unwrap_or_else(|| {
            eprintln!("WeebCentral provider not found. Check providers/weebcentral.yaml exists.");
            process::exit(1);
        })
        .clone();

    // Search
    log::info!("Searching WeebCentral for {query:?}...");
    let results = provider.search(&ctx, &query).await.unwrap_or_else(|e| {
        eprintln!("Search failed: {e}");
        process::exit(1);
    });

    if results.is_empty() {
        eprintln!("No results found for {query:?}");
        process::exit(1);
    }

    println!("\nSearch results:");
    for (i, r) in results.iter().enumerate() {
        println!("  [{i}] {} — {}", r.title, r.url);
    }

    let manga = &results[0];
    println!("\nUsing: {} ({})\n", manga.title, manga.url);

    // Get chapters
    log::info!("Fetching chapter list...");
    let chapters = provider
        .chapters(&ctx, &manga.url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("chapters() failed: {e}");
            process::exit(1);
        });

    println!("Found {} chapters:", chapters.len());
    for ch in chapters.iter().take(10) {
        let title = ch.title.as_deref().unwrap_or("(no title)");
        let scanlator = ch.scanlator_group.as_deref().unwrap_or("Unknown");
        let url = ch.url.as_deref().unwrap_or("(no url how tf???)");
        println!("{}. {} ({}) [{}]", ch.number, title, scanlator, url);
    }
    if chapters.len() > 10 {
        println!("  ... and {} more", chapters.len() - 10);
    }

    // Get first chapter's URL
    let ch1 = &chapters[0];
    let chapter_url = ch1.url.as_ref().unwrap_or_else(|| {
        eprintln!(
            "Chapter {} has no URL — check the 'url' field in weebcentral.yaml chapters.list.fields",
            ch1.number
        );
        process::exit(1);
    });

    println!("\nFetching pages for chapter {}...", ch1.number);
    log::info!("Calling pages() with URL: {chapter_url}");

    let pages = provider
        .pages(&ctx, chapter_url)
        .await
        .unwrap_or_else(|e| {
            eprintln!("pages() failed: {e}");
            process::exit(1);
        });

    println!("Found {} pages", pages.len());
    for p in pages.iter().take(5) {
        println!("  Page {} — {}", p.index, p.url);
    }
    if pages.len() > 5 {
        println!("  ... and {} more", pages.len() - 5);
    }

    if pages.is_empty() {
        eprintln!("No pages found — check the 'pages' extract config in weebcentral.yaml");
        process::exit(1);
    }

    // Download pages
    // let out_dir = PathBuf::from(format!("./test_dl/ch_{:.0}", ch1.number));
    // tokio::fs::create_dir_all(&out_dir)
    //     .await
    //     .expect("failed to create output directory");

    // println!("\nDownloading {} pages to {}/ ...", pages.len(), out_dir.display());

    // let mut downloaded = 0usize;
    // for page in &pages {
    //     let ext = page
    //         .url
    //         .rsplit('.')
    //         .next()
    //         .filter(|e| e.len() <= 4 && !e.contains('/'))
    //         .unwrap_or("jpg");
    //     let filename = format!("{:03}.{ext}", page.index);
    //     let path = out_dir.join(&filename);

    //     match ctx.http.get(&page.url).send().await {
    //         Ok(resp) => match resp.bytes().await {
    //             Ok(bytes) => {
    //                 tokio::fs::write(&path, &bytes)
    //                     .await
    //                     .expect("failed to write page file");
    //                 log::info!("  [{}/{}] {} → {}", page.index, pages.len(), page.url, path.display());
    //                 downloaded += 1;
    //             }
    //             Err(e) => eprintln!("  Page {}: failed to read bytes: {e}", page.index),
    //         },
    //         Err(e) => eprintln!("  Page {}: HTTP error: {e}", page.index),
    //     }
    // }

    // println!(
    //     "\nDone. Downloaded {}/{} pages to {}/",
    //     downloaded,
    //     pages.len(),
    //     out_dir.display()
    // );
}
