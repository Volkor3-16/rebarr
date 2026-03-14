use std::sync::Arc;

use dotenvy::dotenv;

mod api;
mod db;
mod manga;
mod http;
mod scraper;
mod scheduler;
mod library;

use crate::api::{api_routes, frontend_routes};
use crate::http::anilist::ALClient;
use crate::scheduler::worker;
use crate::scraper::{
    browser::BrowserPool,
    {ProviderRegistry, ScraperCtx},
};


#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();
    env_logger::init();

    // Setup DB and API Client
    let db_url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "sqlite:rebarr.db".to_string());
    let pool = db::init(&db_url).await.expect("DB init failed");

    // Reset any tasks that were stuck Running when the server last stopped
    match db::task::reset_running_tasks(&pool).await {
        Ok(0) => {}
        Ok(n) => log::warn!("Reset {n} stuck Running task(s) to Pending."),
        Err(e) => log::error!("Failed to reset running tasks: {e}"),
    }
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
        .mount("/", frontend_routes())
        .mount("/", api_routes())
        .launch()
        .await?;

    Ok(())
}
