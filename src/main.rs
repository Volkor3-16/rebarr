use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dotenvy::dotenv;
use rocket::fs::FileServer;
use rocket_okapi::rapidoc::{make_rapidoc, RapiDocConfig};
use rocket_okapi::settings::UrlObject;
use rocket_okapi::swagger_ui::{make_swagger_ui, SwaggerUIConfig};
use tokio_util::sync::CancellationToken;
use rebarr::api::{extra_routes, frontend_routes, openapi_routes};
use rebarr::db;
use rebarr::http::{ALClient, AniListMetadata, WebhookDispatcher};
use rebarr::scheduler::{CancelMap, start_worker};
use rebarr::scraper::{
    browser::BrowserPool,
    executor::ProviderExecutor,
    {ProviderRegistry, ScraperCtx},
};
use tracing::{error, info, warn};

#[rocket::main]
async fn main() -> Result<(), Box<rocket::Error>> {
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
    let al_metadata = AniListMetadata::new();
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
    let shutdown_token = CancellationToken::new();
    let cancel_map: CancelMap = Arc::new(Mutex::new(HashMap::new()));
    let worker_handle = start_worker(
        pool.clone(),
        Arc::clone(&registry),
        scraper_ctx.clone(),
        Arc::clone(&cancel_map),
        shutdown_token.clone(),
    );
    info!("Background task worker started.");

    let rocket = rocket::build()
        .manage(pool)
        .manage(al_client)
        .manage(al_metadata)
        .manage(http_client)
        .manage(scraper_ctx.clone())
        .manage(Arc::clone(&registry))
        .manage(cancel_map)
        .mount("/", frontend_routes())
        .mount("/", openapi_routes())
        .mount("/", extra_routes())
        .mount("/web", FileServer::from("web"))
        .mount(
            "/swagger-ui/",
            make_swagger_ui(&SwaggerUIConfig {
                url: "../openapi.json".to_owned(),
                ..Default::default()
            }),
        )
        .mount(
            "/rapidoc/",
            make_rapidoc(&RapiDocConfig {
                title: Some("Rebarr API Documentation".to_owned()),
                general: rocket_okapi::rapidoc::GeneralConfig {
                    spec_urls: vec![UrlObject::new("General", "../openapi.json")],
                    ..Default::default()
                },
                ..Default::default()
            }),
        )
        .ignite()
        .await?;

    // Get Rocket's shutdown handle and spawn a task to cancel workers early
    let shutdown_handle = rocket.shutdown();
    let shutdown_token_clone = shutdown_token.clone();
    tokio::spawn(async move {
        shutdown_handle.await;
        info!("Rocket shutdown signal received, cancelling background workers...");
        shutdown_token_clone.cancel();
    });

    // Launch Rocket
    rocket.launch().await?;

    // Graceful shutdown: wait for workers to finish (token already cancelled)
    info!("Waiting for background workers to finish...");
    let _ = tokio::time::timeout(Duration::from_secs(5), worker_handle).await;

    // Clean up browser pool if running
    scraper_ctx.browser.reset().await;
    info!("Shutdown complete.");

    Ok(())
}
