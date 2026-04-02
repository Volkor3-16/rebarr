/// Tests for provider rate limiting and auto-disable functionality.
///
/// Verifies that:
/// 1. Providers follow rate limits as configured in their YAML
/// 2. Providers are auto-disabled after repeated failures
/// 3. Providers are re-enabled after success
/// 4. Backoff window expiration works correctly
mod helpers;

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use helpers::{test_db, test_ctx};
use rebarr::{
    db::provider_failure as db_provider_failure,
    scraper::{
        PageUrl, Provider, ProviderChapterInfo, ProviderRegistry, ProviderSearchResult, ScraperCtx,
        error::ScraperError,
    },
};

// ---------------------------------------------------------------------------
// Test provider with configurable failure behavior
// ---------------------------------------------------------------------------

struct FailingProvider {
    name: String,
    rate_limit_rpm: u32,
    fail_count: Arc<AtomicUsize>,
    should_fail: bool,
}

impl FailingProvider {
    fn new(name: &str, rate_limit_rpm: u32) -> Self {
        Self {
            name: name.to_owned(),
            rate_limit_rpm,
            fail_count: Arc::new(AtomicUsize::new(0)),
            should_fail: false,
        }
    }

    #[allow(dead_code)]
    fn with_failures(mut self) -> Self {
        self.should_fail = true;
        self
    }

    #[allow(dead_code)]
    fn fail_count(&self) -> usize {
        self.fail_count.load(Ordering::SeqCst)
    }
}

#[async_trait]
impl Provider for FailingProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn needs_browser(&self) -> bool {
        false
    }

    fn rate_limit_rpm(&self) -> u32 {
        self.rate_limit_rpm
    }

    fn max_concurrency(&self) -> u32 {
        1
    }

    async fn search(
        &self,
        _ctx: &ScraperCtx,
        _title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        if self.should_fail {
            self.fail_count.fetch_add(1, Ordering::SeqCst);
            return Err(ScraperError::Parse("intentional failure".to_owned()));
        }
        Ok(vec![ProviderSearchResult {
            title: "Test".to_owned(),
            url: "https://example.com/test".to_owned(),
            cover_url: None,
        }])
    }

    async fn chapters(
        &self,
        _ctx: &ScraperCtx,
        _manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        Ok(vec![])
    }

    async fn pages(
        &self,
        _ctx: &ScraperCtx,
        _chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_registry(providers: Vec<Arc<dyn Provider>>) -> ProviderRegistry {
    ProviderRegistry::from_providers_for_tests(providers)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Test that providers with custom rate_limit_rpm are properly rate limited.
///
/// Creates a provider with rate_limit_rpm=10 (1 request per 6 seconds).
/// Fires 3 requests and verifies they take at least 12 seconds (2 delays).
#[tokio::test]
async fn rate_limit_is_enforced() {
    let provider: Arc<dyn Provider> = Arc::new(FailingProvider::new("limited", 10));
    let registry = make_registry(vec![Arc::clone(&provider)]);
    let ctx = test_ctx(&registry);

    let start = Instant::now();

    // Fire 3 sequential requests - should take at least 12 seconds (2 * 6s delays)
    for _ in 0..3 {
        ctx.executor.search(&ctx, &provider, "test").await.unwrap();
    }

    let elapsed = start.elapsed();
    // Allow some tolerance for timing
    assert!(
        elapsed >= Duration::from_secs(11),
        "Rate limiting not enforced: 3 requests at 10 RPM should take ~12s, took {:?}",
        elapsed
    );
}

/// Test that rate limiting is shared across multiple callers for the same provider.
#[tokio::test]
async fn rate_limit_is_shared_across_callers() {
    let provider: Arc<dyn Provider> = Arc::new(FailingProvider::new("shared", 30));
    let registry = make_registry(vec![Arc::clone(&provider)]);
    let ctx = test_ctx(&registry);

    let start = Instant::now();

    // Fire 2 requests in parallel - should be serialized by rate limiter
    let (a, b) = tokio::join!(
        ctx.executor.search(&ctx, &provider, "a"),
        ctx.executor.search(&ctx, &provider, "b")
    );
    a.unwrap();
    b.unwrap();

    // At 30 RPM, 2 requests should take at least 2 seconds (1 delay between them)
    assert!(
        start.elapsed() >= Duration::from_secs(1),
        "Parallel requests should be rate limited"
    );
}

/// Test that providers are auto-disabled after reaching the failure threshold.
#[tokio::test]
async fn provider_auto_disabled_after_threshold() {
    let pool = test_db().await;
    let provider_name = "failing_provider";

    // Insert a manga first (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga = helpers::insert_manga(&pool, lib.uuid, "Test Manga").await;
    let manga_id = manga.id;

    // Default threshold is 5 failures
    // Record 4 failures - should NOT be disabled
    for i in 0..4 {
        db_provider_failure::record(
            &pool,
            provider_name,
            manga_id,
            Some(&format!("error {i}")),
        )
        .await
        .unwrap();
    }

    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(!disabled, "Provider should not be disabled after 4 failures");

    // Record 5th failure - should now be disabled
    db_provider_failure::record(&pool, provider_name, manga_id, Some("error 5"))
        .await
        .unwrap();

    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(disabled, "Provider should be disabled after 5 failures");
}

/// Test that providers are re-enabled after clearing failures.
#[tokio::test]
async fn provider_reenabled_after_clear() {
    let pool = test_db().await;
    let provider_name = "cleared_provider";

    // Insert a manga first (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga = helpers::insert_manga(&pool, lib.uuid, "Test Manga").await;
    let manga_id = manga.id;

    // Record enough failures to trigger auto-disable
    for i in 0..5 {
        db_provider_failure::record(
            &pool,
            provider_name,
            manga_id,
            Some(&format!("error {i}")),
        )
        .await
        .unwrap();
    }

    // Verify disabled
    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(disabled, "Provider should be disabled");

    // Clear failures (simulating success)
    db_provider_failure::clear_for_manga(&pool, provider_name, manga_id)
        .await
        .unwrap();

    // Verify re-enabled
    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(!disabled, "Provider should be re-enabled after clearing failures");
}

/// Test that failures outside the backoff window don't count toward auto-disable.
#[tokio::test]
async fn old_failures_dont_count() {
    let pool = test_db().await;
    let provider_name = "old_failures_provider";

    // Insert a manga first (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga = helpers::insert_manga(&pool, lib.uuid, "Test Manga").await;
    let manga_id = manga.id;

    // Insert old failures (older than backoff window)
    let old_time = chrono::Utc::now().timestamp() - 7200; // 2 hours ago
    for i in 0..5 {
        sqlx::query(
            "INSERT INTO ProviderFailure (provider_name, manga_id, failed_at, error_message)
             VALUES (?, ?, ?, ?)",
        )
        .bind(provider_name)
        .bind(manga_id.to_string())
        .bind(old_time - i)
        .bind(Some(format!("old error {i}")))
        .execute(&pool)
        .await
        .unwrap();
    }

    // Should NOT be disabled because failures are old
    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(!disabled, "Old failures should not trigger auto-disable");

    // Add one recent failure - still shouldn't be disabled (need 5 recent failures)
    db_provider_failure::record(&pool, provider_name, manga_id, Some("recent error"))
        .await
        .unwrap();

    let disabled = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_id)
        .await
        .unwrap();
    assert!(
        !disabled,
        "Should not be disabled with only 1 recent failure"
    );
}

/// Test that consecutive failures are counted correctly within the backoff window.
#[tokio::test]
async fn consecutive_failures_count() {
    let pool = test_db().await;
    let provider_name = "count_provider";

    // Insert a manga first (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga = helpers::insert_manga(&pool, lib.uuid, "Test Manga").await;
    let manga_id = manga.id;

    // Record 3 failures
    for i in 0..3 {
        db_provider_failure::record(
            &pool,
            provider_name,
            manga_id,
            Some(&format!("error {i}")),
        )
        .await
        .unwrap();
    }

    let count = db_provider_failure::consecutive_failures(
        &pool,
        provider_name,
        manga_id,
        60, // 60 minute backoff
    )
    .await
    .unwrap();

    assert_eq!(count, 3, "Should have 3 consecutive failures");
}

/// Test that different providers have independent failure tracking.
#[tokio::test]
async fn failures_are_per_provider() {
    let pool = test_db().await;

    // Insert a manga first (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga = helpers::insert_manga(&pool, lib.uuid, "Test Manga").await;
    let manga_id = manga.id;

    // Record failures for provider A
    for i in 0..5 {
        db_provider_failure::record(
            &pool,
            "provider_a",
            manga_id,
            Some(&format!("error {i}")),
        )
        .await
        .unwrap();
    }

    // Provider A should be disabled
    let disabled_a = db_provider_failure::is_auto_disabled(&pool, "provider_a", manga_id)
        .await
        .unwrap();
    assert!(disabled_a, "Provider A should be disabled");

    // Provider B should NOT be disabled
    let disabled_b = db_provider_failure::is_auto_disabled(&pool, "provider_b", manga_id)
        .await
        .unwrap();
    assert!(!disabled_b, "Provider B should not be disabled");
}

/// Test that failures are per manga (same provider can be disabled for one manga but not another).
#[tokio::test]
async fn failures_are_per_manga() {
    let pool = test_db().await;
    let provider_name = "shared_provider";

    // Insert two manga (required for foreign key)
    let lib = helpers::insert_library(&pool).await;
    let manga_a = helpers::insert_manga(&pool, lib.uuid, "Manga A").await;
    let manga_b = helpers::insert_manga(&pool, lib.uuid, "Manga B").await;
    let manga_a_id = manga_a.id;
    let manga_b_id = manga_b.id;

    // Record failures for manga A
    for i in 0..5 {
        db_provider_failure::record(
            &pool,
            provider_name,
            manga_a_id,
            Some(&format!("error {i}")),
        )
        .await
        .unwrap();
    }

    // Provider should be disabled for manga A
    let disabled_a = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_a_id)
        .await
        .unwrap();
    assert!(disabled_a, "Provider should be disabled for manga A");

    // Provider should NOT be disabled for manga B
    let disabled_b = db_provider_failure::is_auto_disabled(&pool, provider_name, manga_b_id)
        .await
        .unwrap();
    assert!(
        !disabled_b,
        "Provider should not be disabled for manga B"
    );
}
