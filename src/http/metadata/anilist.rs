use anilist_moe::{AniListClient, AniListError, enums::media::MediaFormat};
use reqwest::header::HeaderMap;
use tracing::{debug, warn};

use crate::http::metadata::rate_limiter::MetadataRateLimiter;
use crate::manga::core::Manga;

/// Maximum retry attempts on rate limit.
const MAX_RETRIES: u32 = 3;

/// Rate-limited AniList client.
///
/// Wraps `anilist_moe::AniListClient` with header-aware rate limiting
/// and automatic retry on 429 responses.
pub struct AniListMetadata {
    client: AniListClient,
    limiter: MetadataRateLimiter,
}

impl AniListMetadata {
    /// Create a new AniList metadata client with the default 25 RPM rate limit.
    pub fn new() -> Self {
        Self {
            client: AniListClient::new(),
            limiter: MetadataRateLimiter::default_rpm("anilist"),
        }
    }

    /// Create with a custom RPM.
    pub fn with_rpm(rpm: u32) -> Self {
        Self {
            client: AniListClient::new(),
            limiter: MetadataRateLimiter::new("anilist", rpm),
        }
    }

    /// Search manga by title with rate limiting and retry.
    pub async fn search_manga(&self, title: &str) -> Result<Vec<Manga>, AniListError> {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            self.limiter.wait_for_permit().await;

            match self.client.manga().search_manga(title, Some(1), Some(10)).await {
                Ok(page) => {
                    let results: Vec<Manga> = page
                        .data
                        .into_iter()
                        .filter(|media| {
                            matches!(
                                media.format,
                                Some(MediaFormat::Manga) | Some(MediaFormat::OneShot)
                            )
                        })
                        .map(|media| media.into())
                        .collect();
                    debug!(
                        "[anilist] Found {} manga results for '{}'",
                        results.len(),
                        title
                    );
                    return Ok(results);
                }
                Err(e) => {
                    if self.is_rate_limit_error(&e) && attempt + 1 < MAX_RETRIES {
                        self.limiter.handle_rate_limited(attempt);
                        warn!(
                            "[anilist] Rate limited searching '{}', attempt {}/{}",
                            title,
                            attempt + 1,
                            MAX_RETRIES
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or(AniListError::RateLimitSimple))
    }

    /// Fetch manga by AniList ID with rate limiting and retry.
    pub async fn grab_manga(&self, id: i32) -> Result<Manga, AniListError> {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            self.limiter.wait_for_permit().await;

            match self.client.manga().get_anime_by_id(id).await {
                Ok(media) => {
                    debug!(
                        "[anilist] Found manga '{:?}' with ID {}",
                        media.title.as_ref().and_then(|t| t.english.as_ref()),
                        id
                    );
                    return Ok(media.into());
                }
                Err(e) => {
                    if self.is_rate_limit_error(&e) && attempt + 1 < MAX_RETRIES {
                        self.limiter.handle_rate_limited(attempt);
                        warn!(
                            "[anilist] Rate limited fetching ID {}, attempt {}/{}",
                            id,
                            attempt + 1,
                            MAX_RETRIES
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or(AniListError::RateLimitSimple))
    }

    /// Fetch popular manga with rate limiting.
    pub async fn popular_manga(&self) -> Result<Vec<Manga>, AniListError> {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            self.limiter.wait_for_permit().await;

            match self.client.manga().get_popular_manga(Some(1), Some(25)).await {
                Ok(page) => {
                    let results: Vec<Manga> =
                        page.data.into_iter().map(|media| media.into()).collect();
                    debug!("[anilist] Fetched {} popular manga", results.len());
                    return Ok(results);
                }
                Err(e) => {
                    if self.is_rate_limit_error(&e) && attempt + 1 < MAX_RETRIES {
                        self.limiter.handle_rate_limited(attempt);
                        warn!(
                            "[anilist] Rate limited fetching popular, attempt {}/{}",
                            attempt + 1,
                            MAX_RETRIES
                        );
                        last_err = Some(e);
                        continue;
                    }
                    return Err(e);
                }
            }
        }
        Err(last_err.unwrap_or(AniListError::RateLimitSimple))
    }

    /// Update the rate limiter from HTTP response headers.
    /// Call this if you have access to raw response headers.
    pub fn update_from_headers(&self, headers: &HeaderMap) {
        self.limiter.update_from_headers(headers);
    }

    /// Check if an error is a rate limit error.
    fn is_rate_limit_error(&self, error: &AniListError) -> bool {
        let error_str = format!("{error:?}");
        error_str.contains("429")
            || error_str.contains("rate")
            || error_str.contains("Rate")
            || error_str.contains("limit")
            || error_str.contains("Limit")
    }
}

impl Default for AniListMetadata {
    fn default() -> Self {
        Self::new()
    }
}