use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Mutex, Notify};

use crate::scraper::error::ScraperError;
use crate::scraper::{
    PageUrl, Provider, ProviderChapterInfo, ProviderRegistry, ProviderSearchResult, ScraperCtx,
};

#[derive(Debug)]
struct ConcurrencyGateState {
    current: usize,
    limit: usize,
}

#[derive(Debug)]
struct ConcurrencyGate {
    state: Mutex<ConcurrencyGateState>,
    notify: Notify,
}

impl ConcurrencyGate {
    fn new(limit: usize) -> Self {
        Self {
            state: Mutex::new(ConcurrencyGateState {
                current: 0,
                limit: limit.max(1),
            }),
            notify: Notify::new(),
        }
    }

    async fn acquire(self: &Arc<Self>) -> ConcurrencyPermit {
        loop {
            let notified = {
                let mut state = self.state.lock().await;
                if state.current < state.limit {
                    state.current += 1;
                    return ConcurrencyPermit {
                        gate: Arc::clone(self),
                    };
                }
                self.notify.notified()
            };
            notified.await;
        }
    }

    async fn set_limit(&self, limit: usize) {
        let mut state = self.state.lock().await;
        state.limit = limit.max(1);
        drop(state);
        self.notify.notify_waiters();
    }
}

struct ConcurrencyPermit {
    gate: Arc<ConcurrencyGate>,
}

impl Drop for ConcurrencyPermit {
    fn drop(&mut self) {
        if let Ok(mut state) = self.gate.state.try_lock() {
            state.current = state.current.saturating_sub(1);
            drop(state);
            self.gate.notify.notify_one();
            return;
        }

        let gate = Arc::clone(&self.gate);
        tokio::spawn(async move {
            let mut state = gate.state.lock().await;
            state.current = state.current.saturating_sub(1);
            drop(state);
            gate.notify.notify_one();
        });
    }
}

#[derive(Clone, Debug)]
struct RateLimitState {
    next_ready_at: Instant,
    interval: Duration,
}

#[derive(Clone)]
pub struct ProviderExecutor {
    browser_gate: Arc<ConcurrencyGate>,
    provider_gates: Arc<HashMap<String, Arc<ConcurrencyGate>>>,
    rate_limits: Arc<Mutex<HashMap<String, RateLimitState>>>,
}

impl ProviderExecutor {
    pub fn new(registry: &ProviderRegistry, browser_worker_count: usize) -> Self {
        let mut provider_gates = HashMap::new();
        let mut rate_limits = HashMap::new();

        for provider in registry.all() {
            provider_gates.insert(
                provider.name().to_owned(),
                Arc::new(ConcurrencyGate::new(provider.max_concurrency() as usize)),
            );

            let rpm = provider.rate_limit_rpm().max(1) as u64;
            let millis = (60_000u64 / rpm).max(1);
            rate_limits.insert(
                provider.name().to_owned(),
                RateLimitState {
                    next_ready_at: Instant::now(),
                    interval: Duration::from_millis(millis),
                },
            );
        }

        Self {
            browser_gate: Arc::new(ConcurrencyGate::new(browser_worker_count)),
            provider_gates: Arc::new(provider_gates),
            rate_limits: Arc::new(Mutex::new(rate_limits)),
        }
    }

    pub async fn set_browser_worker_count(&self, browser_worker_count: usize) {
        self.browser_gate.set_limit(browser_worker_count).await;
    }

    pub async fn acquire_browser_slot(&self) -> BrowserSlotPermit {
        BrowserSlotPermit {
            _permit: self.browser_gate.acquire().await,
        }
    }

    pub async fn search(
        &self,
        ctx: &ScraperCtx,
        provider: &Arc<dyn Provider>,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        let (_provider_permit, _browser_permit) = self.acquire_provider_slots(provider).await?;
        self.wait_for_rate_limit(provider.name()).await;
        provider.search(ctx, title).await
    }

    pub async fn chapters(
        &self,
        ctx: &ScraperCtx,
        provider: &Arc<dyn Provider>,
        manga_url: &str,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        let (_provider_permit, _browser_permit) = self.acquire_provider_slots(provider).await?;
        self.wait_for_rate_limit(provider.name()).await;
        provider.chapters(ctx, manga_url).await
    }

    pub async fn pages(
        &self,
        ctx: &ScraperCtx,
        provider: &Arc<dyn Provider>,
        chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        let (_provider_permit, _browser_permit) = self.acquire_provider_slots(provider).await?;
        self.wait_for_rate_limit(provider.name()).await;
        provider.pages(ctx, chapter_url).await
    }

    async fn acquire_provider_slots(
        &self,
        provider: &Arc<dyn Provider>,
    ) -> Result<(ConcurrencyPermit, Option<ConcurrencyPermit>), ScraperError> {
        let provider_gate = self
            .provider_gates
            .get(provider.name())
            .cloned()
            .ok_or_else(|| ScraperError::Parse(format!("unknown provider '{}'", provider.name())))?;
        let provider_permit = provider_gate.acquire().await;
        let browser_permit = if provider.needs_browser() {
            Some(self.browser_gate.acquire().await)
        } else {
            None
        };
        Ok((provider_permit, browser_permit))
    }

    async fn wait_for_rate_limit(&self, provider_name: &str) {
        let delay = {
            let mut rate_limits = self.rate_limits.lock().await;
            let Some(state) = rate_limits.get_mut(provider_name) else {
                return;
            };

            let now = Instant::now();
            let scheduled_at = if state.next_ready_at > now {
                state.next_ready_at
            } else {
                now
            };
            state.next_ready_at = scheduled_at + state.interval;
            scheduled_at.saturating_duration_since(now)
        };

        if !delay.is_zero() {
            tokio::time::sleep(delay).await;
        }
    }
}

pub struct BrowserSlotPermit {
    _permit: ConcurrencyPermit,
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    use async_trait::async_trait;
    use tokio::sync::Barrier;

    use super::ProviderExecutor;
    use crate::scraper::browser::BrowserPool;
    use crate::scraper::{
        PageUrl, Provider, ProviderChapterInfo, ProviderRegistry, ProviderSearchResult, ScraperCtx,
    };

    struct FakeProvider {
        name: String,
        needs_browser: bool,
        max_concurrency: u32,
        rate_limit_rpm: u32,
        delay_ms: u64,
        current: Arc<AtomicUsize>,
        max_seen: Arc<AtomicUsize>,
        barrier: Option<Arc<Barrier>>,
    }

    impl FakeProvider {
        fn new(
            name: &str,
            needs_browser: bool,
            max_concurrency: u32,
            rate_limit_rpm: u32,
            delay_ms: u64,
            current: Arc<AtomicUsize>,
            max_seen: Arc<AtomicUsize>,
            barrier: Option<Arc<Barrier>>,
        ) -> Self {
            Self {
                name: name.to_owned(),
                needs_browser,
                max_concurrency,
                rate_limit_rpm,
                delay_ms,
                current,
                max_seen,
                barrier,
            }
        }
    }

    #[async_trait]
    impl Provider for FakeProvider {
        fn name(&self) -> &str {
            &self.name
        }

        fn needs_browser(&self) -> bool {
            self.needs_browser
        }

        fn max_concurrency(&self) -> u32 {
            self.max_concurrency
        }

        fn rate_limit_rpm(&self) -> u32 {
            self.rate_limit_rpm
        }

        async fn search(
            &self,
            _ctx: &ScraperCtx,
            _title: &str,
        ) -> Result<Vec<ProviderSearchResult>, crate::scraper::error::ScraperError> {
            if let Some(barrier) = &self.barrier {
                barrier.wait().await;
            }

            let current = self.current.fetch_add(1, Ordering::SeqCst) + 1;
            loop {
                let seen = self.max_seen.load(Ordering::SeqCst);
                if current <= seen {
                    break;
                }
                if self
                    .max_seen
                    .compare_exchange(seen, current, Ordering::SeqCst, Ordering::SeqCst)
                    .is_ok()
                {
                    break;
                }
            }

            tokio::time::sleep(Duration::from_millis(self.delay_ms)).await;
            self.current.fetch_sub(1, Ordering::SeqCst);
            Ok(vec![ProviderSearchResult {
                title: self.name.clone(),
                url: format!("https://example.com/{}", self.name),
                cover_url: None,
            }])
        }

        async fn chapters(
            &self,
            _ctx: &ScraperCtx,
            _manga_url: &str,
        ) -> Result<Vec<ProviderChapterInfo>, crate::scraper::error::ScraperError> {
            Ok(vec![])
        }

        async fn pages(
            &self,
            _ctx: &ScraperCtx,
            _chapter_url: &str,
        ) -> Result<Vec<PageUrl>, crate::scraper::error::ScraperError> {
            Ok(vec![])
        }
    }

    fn ctx_with_registry(providers: Vec<Arc<dyn Provider>>, browser_workers: usize) -> ScraperCtx {
        let registry = ProviderRegistry::from_providers_for_tests(providers);
        let executor = Arc::new(ProviderExecutor::new(&registry, browser_workers));
        ScraperCtx::new(reqwest::Client::new(), BrowserPool::new(), executor)
    }

    #[tokio::test]
    async fn same_provider_jobs_are_serialized() {
        let current = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "serial",
            false,
            1,
            10_000,
            25,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            None,
        ));
        let ctx = ctx_with_registry(vec![Arc::clone(&provider)], 4);

        let (a, b) = tokio::join!(
            ctx.executor.search(&ctx, &provider, "a"),
            ctx.executor.search(&ctx, &provider, "b")
        );
        a.unwrap();
        b.unwrap();

        assert_eq!(max_seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn different_providers_can_run_in_parallel() {
        let current = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let barrier = Arc::new(Barrier::new(2));
        let one: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "one",
            false,
            1,
            10_000,
            25,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            Some(Arc::clone(&barrier)),
        ));
        let two: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "two",
            false,
            1,
            10_000,
            25,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            Some(Arc::clone(&barrier)),
        ));
        let ctx = ctx_with_registry(vec![Arc::clone(&one), Arc::clone(&two)], 4);

        let (a, b) = tokio::join!(
            ctx.executor.search(&ctx, &one, "a"),
            ctx.executor.search(&ctx, &two, "b")
        );
        a.unwrap();
        b.unwrap();

        assert!(max_seen.load(Ordering::SeqCst) >= 2);
    }

    #[tokio::test]
    async fn global_browser_cap_limits_parallelism() {
        let current = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let one: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "one",
            true,
            2,
            10_000,
            25,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            None,
        ));
        let two: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "two",
            true,
            2,
            10_000,
            25,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            None,
        ));
        let ctx = ctx_with_registry(vec![Arc::clone(&one), Arc::clone(&two)], 1);

        let (a, b) = tokio::join!(
            ctx.executor.search(&ctx, &one, "a"),
            ctx.executor.search(&ctx, &two, "b")
        );
        a.unwrap();
        b.unwrap();

        assert_eq!(max_seen.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn rate_limit_is_shared_across_callers() {
        let current = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let provider: Arc<dyn Provider> = Arc::new(FakeProvider::new(
            "limited",
            false,
            2,
            6_000,
            0,
            Arc::clone(&current),
            Arc::clone(&max_seen),
            None,
        ));
        let ctx = ctx_with_registry(vec![Arc::clone(&provider)], 4);

        let start = Instant::now();
        let (a, b) = tokio::join!(
            ctx.executor.search(&ctx, &provider, "a"),
            ctx.executor.search(&ctx, &provider, "b")
        );
        a.unwrap();
        b.unwrap();

        assert!(start.elapsed() >= Duration::from_millis(8));
    }
}
