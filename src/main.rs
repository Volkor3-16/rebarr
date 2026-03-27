use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use dotenvy::dotenv;
use tracing::{error, info, warn};
use rocket::fs::FileServer;

mod api;
mod db;
mod http;
mod importer;
mod library;
mod manga;
mod scheduler;
mod scraper;

use crate::api::{api_routes, frontend_routes};
use crate::http::anilist::ALClient;
use crate::http::webhook::WebhookDispatcher;
use crate::scheduler::worker::{self, CancelMap};
use crate::scraper::{
    browser::BrowserPool,
    executor::ProviderExecutor,
    {ProviderRegistry, ScraperCtx},
};

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    // Setup DB and API Client
    let db_url = std::env::var("DATABASE_URL").unwrap_or_else(|_| "sqlite:rebarr.db".to_string());
    let pool = db::init(&db_url).await.expect("DB init failed");

    // Reset any tasks that were stuck Running when the server last stopped
    match db::task::reset_running_tasks(&pool).await {
        Ok(0) => {}
        Ok(n) => warn!("Reset {n} stuck Running task(s) to Pending."),
        Err(e) => error!("Failed to reset running tasks: {e}"),
    }

    let al_client = ALClient::new();
    let http_client = reqwest::Client::new();
    WebhookDispatcher::new(pool.clone(), http_client.clone()).install();

    // Setup browser scraper
    let browser_pool = BrowserPool::new();
    let registry = Arc::new(
        ProviderRegistry::load()
            .await
            .expect("Failed to load providers"),
    );
    let browser_worker_count = db::settings::get(&pool, "browser_worker_count", "3")
        .await
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(3)
        .clamp(1, 16);
    let executor = Arc::new(ProviderExecutor::new(&registry, browser_worker_count));

    // Pre-warm Chromium if any provider needs it, so errors surface at
    // startup rather than on the first scrape request.
    if registry.browser_providers().next().is_some() {
        info!("Pre-warming headless browser for JS-capable providers...");
        match browser_pool.get().await {
            Ok(_) => info!("Browser ready."),
            Err(e) => warn!("Browser pre-warm failed (will retry on first request): {e}"),
        }
    }

    let scraper_ctx = ScraperCtx::new(http_client.clone(), browser_pool, executor);

    // Background Task Handler start
    let cancel_map: CancelMap = Arc::new(Mutex::new(HashMap::new()));
    let _worker = worker::start(
        pool.clone(),
        Arc::clone(&registry),
        scraper_ctx.clone(),
        Arc::clone(&cancel_map),
    );
    info!("Background task worker started.");

    rocket::build()
        .manage(pool)
        .manage(al_client)
        .manage(http_client)
        .manage(scraper_ctx)
        .manage(Arc::clone(&registry))
        .manage(cancel_map)
        .mount("/", frontend_routes())
        .mount("/", api_routes())
        .mount("/web", FileServer::from("web"))
        .launch()
        .await?;

    Ok(())
}
