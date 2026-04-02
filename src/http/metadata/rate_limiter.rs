use std::sync::atomic::{AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use governor::clock::DefaultClock;
use governor::middleware::NoOpMiddleware;
use governor::state::{InMemoryState, NotKeyed};
use governor::Quota;
use nonzero_ext::nonzero;
use reqwest::header::HeaderMap;
use tokio::time::sleep;
use tracing::debug;

/// Default RPM for metadata providers (25 RPM = ~1 req/2.4s, headroom under 30 RPM limit).
const DEFAULT_RPM: u32 = 25;

/// When remaining requests drop below this threshold, add extra delay.
const PROACTIVE_SLOWDOWN_THRESHOLD: u32 = 5;

/// Extra delay in seconds when approaching rate limit.
const PROACTIVE_SLOWDOWN_SECS: u64 = 2;

/// Initial backoff in seconds on 429.
const INITIAL_BACKOFF_SECS: u64 = 2;

/// Maximum backoff in seconds.
const MAX_BACKOFF_SECS: u64 = 60;

type GovernorLimiter = governor::RateLimiter<NotKeyed, InMemoryState, DefaultClock, NoOpMiddleware>;

/// Header-aware rate limiter for metadata API providers.
///
/// Uses `governor` for RPM enforcement and tracks response headers
/// (`X-RateLimit-Remaining`, `X-RateLimit-Reset`, `Retry-After`) to
/// proactively slow down before hitting limits and reactively back off
/// when rate limited.
#[derive(Clone)]
pub struct MetadataRateLimiter {
    name: String,
    governor: Arc<GovernorLimiter>,
    /// Remaining requests from X-RateLimit-Remaining header.
    remaining: Arc<AtomicU32>,
    /// Unix timestamp (seconds) when the rate limit resets (from X-RateLimit-Reset).
    reset_at: Arc<AtomicI64>,
    /// Seconds to wait before retrying (from Retry-After header).
    retry_after: Arc<AtomicU64>,
}

impl MetadataRateLimiter {
    /// Create a new rate limiter with the given RPM.
    pub fn new(name: &str, rpm: u32) -> Self {
        let rpm = rpm.max(1);
        let period = Duration::from_secs(60) / rpm;
        let quota = Quota::with_period(period)
            .expect("valid governor quota")
            .allow_burst(nonzero!(1u32));

        Self {
            name: name.to_owned(),
            governor: Arc::new(GovernorLimiter::direct(quota)),
            remaining: Arc::new(AtomicU32::new(u32::MAX)),
            reset_at: Arc::new(AtomicI64::new(0)),
            retry_after: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create a rate limiter with the default RPM.
    pub fn default_rpm(name: &str) -> Self {
        Self::new(name, DEFAULT_RPM)
    }

    /// Wait until a request is permitted.
    ///
    /// 1. If `Retry-After` is active, sleep for that duration.
    /// 2. If `remaining` is low, add proactive delay.
    /// 3. Use governor to enforce RPM.
    pub async fn wait_for_permit(&self) {
        // Check Retry-After cooldown
        let retry_secs = self.retry_after.load(Ordering::Relaxed);
        if retry_secs > 0 {
            debug!(
                "[metadata:{}] Retry-After active, sleeping {}s",
                self.name, retry_secs
            );
            sleep(Duration::from_secs(retry_secs)).await;
            self.retry_after.store(0, Ordering::Relaxed);
        }

        // Check if we're near the reset time and remaining is very low
        let remaining = self.remaining.load(Ordering::Relaxed);
        if remaining < PROACTIVE_SLOWDOWN_THRESHOLD {
            let reset = self.reset_at.load(Ordering::Relaxed);
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            if reset > now {
                let wait_secs = (reset - now) as u64 + 1;
                debug!(
                    "[metadata:{}] Only {} requests remaining, reset in {}s — waiting",
                    self.name, remaining, wait_secs
                );
                sleep(Duration::from_secs(wait_secs.min(MAX_BACKOFF_SECS))).await;
            } else {
                // Reset time passed but remaining is still low — add small delay
                debug!(
                    "[metadata:{}] {} requests remaining, adding proactive delay",
                    self.name, remaining
                );
                sleep(Duration::from_secs(PROACTIVE_SLOWDOWN_SECS)).await;
            }
        }

        // Governor enforces RPM
        self.governor.until_ready().await;
    }

    /// Update rate limiter state from response headers.
    pub fn update_from_headers(&self, headers: &HeaderMap) {
        // Parse X-RateLimit-Remaining
        if let Some(remaining) = headers.get("x-ratelimit-remaining") {
            if let Ok(val) = remaining.to_str() {
                if let Ok(n) = val.parse::<u32>() {
                    self.remaining.store(n, Ordering::Relaxed);
                    debug!(
                        "[metadata:{}] X-RateLimit-Remaining: {}",
                        self.name, n
                    );
                }
            }
        }

        // Parse X-RateLimit-Reset (unix timestamp)
        if let Some(reset) = headers.get("x-ratelimit-reset") {
            if let Ok(val) = reset.to_str() {
                if let Ok(ts) = val.parse::<i64>() {
                    self.reset_at.store(ts, Ordering::Relaxed);
                    debug!(
                        "[metadata:{}] X-RateLimit-Reset: {}",
                        self.name, ts
                    );
                }
            }
        }

        // Parse Retry-After (seconds)
        if let Some(retry) = headers.get("retry-after") {
            if let Ok(val) = retry.to_str() {
                if let Ok(secs) = val.parse::<u64>() {
                    self.retry_after.store(secs, Ordering::Relaxed);
                    debug!(
                        "[metadata:{}] Retry-After: {}s",
                        self.name, secs
                    );
                }
            }
        }
    }

    /// Handle a 429 response by setting a backoff.
    pub fn handle_rate_limited(&self, attempt: u32) {
        let backoff = INITIAL_BACKOFF_SECS
            .saturating_mul(2u64.saturating_pow(attempt))
            .min(MAX_BACKOFF_SECS);
        self.retry_after.store(backoff, Ordering::Relaxed);
        debug!(
            "[metadata:{}] Rate limited (attempt {}), backing off {}s",
            self.name, attempt, backoff
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_with_default_rpm() {
        let limiter = MetadataRateLimiter::default_rpm("test");
        assert_eq!(limiter.name, "test");
    }

    #[test]
    fn creates_with_custom_rpm() {
        let limiter = MetadataRateLimiter::new("test", 10);
        assert_eq!(limiter.name, "test");
    }

    #[test]
    fn updates_from_headers() {
        let limiter = MetadataRateLimiter::default_rpm("test");
        let mut headers = HeaderMap::new();
        headers.insert("x-ratelimit-remaining", "10".parse().unwrap());
        headers.insert("x-ratelimit-reset", "1700000000".parse().unwrap());
        headers.insert("retry-after", "5".parse().unwrap());

        limiter.update_from_headers(&headers);

        assert_eq!(limiter.remaining.load(Ordering::Relaxed), 10);
        assert_eq!(limiter.reset_at.load(Ordering::Relaxed), 1700000000);
        assert_eq!(limiter.retry_after.load(Ordering::Relaxed), 5);
    }

    #[test]
    fn handle_rate_limited_sets_backoff() {
        let limiter = MetadataRateLimiter::default_rpm("test");

        limiter.handle_rate_limited(0);
        assert_eq!(limiter.retry_after.load(Ordering::Relaxed), 2);

        limiter.handle_rate_limited(1);
        assert_eq!(limiter.retry_after.load(Ordering::Relaxed), 4);

        limiter.handle_rate_limited(5);
        assert_eq!(limiter.retry_after.load(Ordering::Relaxed), 64.min(MAX_BACKOFF_SECS));
    }
}