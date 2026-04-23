use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use dotenvy::dotenv;
use rebarr::api::{extra_routes, frontend_routes, openapi_routes};
use rebarr::db;
use rebarr::db::task::TaskType;
use rebarr::http::{ALClient, AniListMetadata, WebhookDispatcher};
use rebarr::scheduler::{CancelMap, start_worker};
use rebarr::scraper::{
    browser::BrowserPool,
    executor::ProviderExecutor,
    {ProviderRegistry, ScraperCtx},
};
use rocket::fs::FileServer;
use rocket_okapi::rapidoc::{RapiDocConfig, make_rapidoc};
use rocket_okapi::settings::UrlObject;
use rocket_okapi::swagger_ui::{SwaggerUIConfig, make_swagger_ui};
use sqlx::SqlitePool;
use tokio_util::sync::CancellationToken;
use tracing::{error, info, warn};

/// Check each versioned provider's YAML version against the last recorded version in the DB.
/// If the version changed (or is new), enqueue SyncProviderChapters tasks for every manga
/// that already has a URL cached for that provider so their chapter lists are refreshed
/// immediately rather than waiting for the next periodic scheduler cycle.
async fn check_provider_versions(pool: &SqlitePool, registry: &ProviderRegistry) {
    for def in registry.all_defs() {
        let Some(current_version) = &def.version else {
            continue; // unversioned providers opt out
        };

        let stored = db::provider_settings::get_version(pool, &def.name)
            .await
            .ok()
            .flatten();

        if stored.as_deref() == Some(current_version.as_str()) {
            continue; // version unchanged — nothing to do
        }

        let old_display = stored.as_deref().unwrap_or("(none)");
        info!(
            "Provider '{}' version changed: {} -> {}",
            def.name, old_display, current_version
        );

        if let Err(e) = db::provider_settings::set_version(pool, &def.name, current_version).await
        {
            warn!("Failed to store version for '{}': {e}", def.name);
            continue;
        }

        let manga_ids = match db::provider::get_manga_ids_with_found_url(pool, &def.name).await {
            Ok(ids) => ids,
            Err(e) => {
                warn!(
                    "Failed to query manga for provider '{}' version refresh: {e}",
                    def.name
                );
                continue;
            }
        };

        if manga_ids.is_empty() {
            info!(
                "Provider '{}' version changed — no manga to re-sync yet.",
                def.name
            );
            continue;
        }

        let queue = format!("provider:{}", def.name);
        let payload = serde_json::json!({ "provider": def.name }).to_string();
        let mut enqueued = 0usize;

        for manga_id in &manga_ids {
            if db::task::is_pending_in_queue(pool, &queue, *manga_id, TaskType::SyncProviderChapters)
                .await
                .unwrap_or(false)
            {
                continue; // already queued
            }

            match db::task::enqueue_with_payload(
                pool,
                TaskType::SyncProviderChapters,
                Some(*manga_id),
                None,
                7, // between user-triggered (5) and periodic scheduler (10)
                Some(queue.clone()),
                Some(payload.clone()),
            )
            .await
            {
                Ok(_) => enqueued += 1,
                Err(e) => warn!(
                    "Failed to enqueue re-sync for manga {} on provider '{}': {e}",
                    manga_id, def.name
                ),
            }
        }

        info!(
            "Provider '{}' version changed — queued {}/{} re-sync task(s).",
            def.name,
            enqueued,
            manga_ids.len()
        );
    }
}

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

    // Add default quality rules for providers that don't have any rules yet
    if let Err(e) =
        db::quality_rules::ensure_default_provider_rules(&pool, registry.all_defs()).await
    {
        warn!("Failed to ensure default provider quality rules: {}", e);
    }

    // Detect provider YAML version bumps and enqueue re-sync tasks for affected manga
    check_provider_versions(&pool, &registry).await;
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
