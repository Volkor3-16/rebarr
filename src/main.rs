use std::sync::Arc;

use dotenvy::dotenv;

mod api;
mod covers;
mod db;
mod downloader;
mod manga;
mod merge;
mod metadata;
mod scraper;
mod web;
mod worker;

use crate::metadata::anilist::ALClient;
use crate::scraper::{
    browser::BrowserPool,
    {ProviderRegistry, ScraperCtx},
};

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();
    env_logger::init();

    // TODO: Move this to save in respective series directory (so komga can use it)
    // root_path/library/Series
    //    poster.ext
    tokio::fs::create_dir_all("./thumbnails")
        .await
        .expect("Failed to create thumbnails directory");

    // Setup DB and API Client
    let pool = db::init("sqlite:rebarr.db").await.expect("DB init failed");
    let al_client = ALClient::new();
    let http_client = reqwest::Client::new();

    // Setup browser scraper
    let browser_pool = BrowserPool::new();
    let registry = Arc::new(
        ProviderRegistry::load()
            .await
            .expect("Failed to load providers"),
    );

    // Pre-warm Chromium if any provider needs it, so errors surface at
    // startup rather than on the first scrape request.
    if registry.browser_providers().next().is_some() {
        log::info!("Pre-warming headless browser for JS-capable providers...");
        match browser_pool.get().await {
            Ok(_) => log::info!("Browser ready."),
            Err(e) => log::warn!("Browser pre-warm failed (will retry on first request): {e}"),
        }
    }

    let mut scraper_ctx = ScraperCtx::new(http_client.clone(), browser_pool);
    scraper_ctx.flaresolverr_url = std::env::var("REBARR_FLARESOLVERR_URL").ok();

    // Background Task Handler start
    let _worker = worker::start(pool.clone(), Arc::clone(&registry), scraper_ctx.clone());
    log::info!("Background task worker started.");

    rocket::build()
        .manage(pool)
        .manage(al_client)
        .manage(http_client)
        .manage(scraper_ctx)
        .manage(Arc::clone(&registry))
        .mount("/", web::routes())
        .mount("/", api::routes())
        .mount("/thumbnails", rocket::fs::FileServer::from("./thumbnails"))
        .launch()
        .await?;

    Ok(())
}
