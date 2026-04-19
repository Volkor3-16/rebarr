use anilist_moe::{
    AniListClient, AniListError,
    enums::media::{MediaFormat, MediaRelation, MediaStatus, MediaType},
    objects::media::{Media, MediaCoverImage, MediaTitle},
};
use reqwest::header::HeaderMap;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tracing::{debug, warn};

use crate::http::metadata::rate_limiter::MetadataRateLimiter;
use crate::manga::core::Manga;

/// Maximum retry attempts on rate limit.
const MAX_RETRIES: u32 = 3;

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SuggestionMedia {
    pub anilist_id: u32,
    pub title: String,
    pub cover_url: Option<String>,
    pub synopsis: Option<String>,
    pub media_format: Option<String>,
    pub publishing_status: Option<String>,
    pub tags: Vec<String>,
    pub community_rating: Option<i32>,
    pub popularity: Option<i32>,
    pub favourites: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct RecommendationSuggestion {
    pub target: SuggestionMedia,
    pub rating: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct RelationSuggestion {
    pub target: SuggestionMedia,
    pub relation_type: MediaRelation,
}

#[derive(Debug, Clone)]
pub struct AniListSuggestionBundle {
    pub recommendations: Vec<RecommendationSuggestion>,
    pub relations: Vec<RelationSuggestion>,
}

#[derive(Debug, Clone)]
pub struct SuggestionMediaDetails {
    pub synopsis: Option<String>,
    pub community_rating: Option<i32>,
    pub popularity: Option<i32>,
    pub favourites: Option<i32>,
}

fn is_supported_media_type(media_type: Option<MediaType>, format: Option<MediaFormat>) -> bool {
    matches!(media_type, Some(MediaType::Manga))
        && matches!(format, Some(MediaFormat::Manga) | Some(MediaFormat::OneShot))
}

fn format_to_string(format: MediaFormat) -> String {
    match format {
        MediaFormat::Tv => "Tv",
        MediaFormat::TvShort => "TvShort",
        MediaFormat::Movie => "Movie",
        MediaFormat::Special => "Special",
        MediaFormat::Ova => "Ova",
        MediaFormat::Ona => "Ona",
        MediaFormat::Music => "Music",
        MediaFormat::Manga => "Manga",
        MediaFormat::Novel => "Novel",
        MediaFormat::OneShot => "OneShot",
    }
    .to_owned()
}

fn status_to_string(status: MediaStatus) -> String {
    match status {
        MediaStatus::Finished => "Completed",
        MediaStatus::Releasing => "Ongoing",
        MediaStatus::NotYetReleased => "NotYetReleased",
        MediaStatus::Cancelled => "Cancelled",
        MediaStatus::Hiatus => "Hiatus",
    }
    .to_owned()
}

fn title_from_media_title(title: &MediaTitle) -> Option<String> {
    title
        .english
        .clone()
        .or_else(|| title.user_preferred.clone())
        .or_else(|| title.romaji.clone())
        .or_else(|| title.native.clone())
}

fn cover_from_media_cover(cover: Option<&MediaCoverImage>) -> Option<String> {
    cover.and_then(|c| c.large.clone().or(c.medium.clone()).or(c.extra_large.clone()))
}

fn suggestion_media_from_media(node: &Media) -> Option<SuggestionMedia> {
    if !is_supported_media_type(node.media_type, node.format) {
        return None;
    }
    let anilist_id = node.id? as u32;
    let title = title_from_media_title(node.title.as_ref()?)?;
    let mut tags = node.genres.clone().unwrap_or_default();
    if let Some(extra_tags) = &node.tags {
        for tag in extra_tags.iter().filter_map(|t| t.name.clone()) {
            if !tags.iter().any(|existing| existing.eq_ignore_ascii_case(&tag)) {
                tags.push(tag);
            }
        }
    }
    if tags.len() > 8 {
        tags.truncate(8);
    }
    Some(SuggestionMedia {
        anilist_id,
        title,
        cover_url: cover_from_media_cover(node.cover_image.as_ref()),
        synopsis: node.description.as_deref().map(crate::manga::core::strip_html),
        media_format: node.format.map(format_to_string),
        publishing_status: node.status.map(status_to_string),
        tags,
        community_rating: node.average_score,
        popularity: node.popularity,
        favourites: node.favourites,
    })
}

pub fn suggestion_bundle_from_media(media: &Media) -> AniListSuggestionBundle {
    let recommendations = media
        .recommendations
        .as_ref()
        .and_then(|r| r.nodes.as_ref())
        .map(|nodes| {
            nodes.iter()
                .filter_map(|rec| {
                    let target = suggestion_media_from_media(rec.media_recommendation.as_ref()?)?;
                    Some(RecommendationSuggestion {
                        target,
                        rating: rec.rating,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    let relations = media
        .relations
        .as_ref()
        .and_then(|r| r.edges.as_ref())
        .map(|edges| {
            edges.iter()
                .filter_map(|edge| {
                    let relation_type = edge.relation_type?;
                    let target = suggestion_media_from_media(edge.node.as_ref()?)?;
                    Some(RelationSuggestion {
                        target,
                        relation_type,
                    })
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    AniListSuggestionBundle {
        recommendations,
        relations,
    }
}

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

            match self
                .client
                .manga()
                .search_manga(title, Some(1), Some(10))
                .await
            {
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

    /// Fetch the raw AniList Media payload with rate limiting.
    pub async fn grab_raw_media(&self, id: i32) -> Result<Media, AniListError> {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            self.limiter.wait_for_permit().await;

            match self.client.manga().get_anime_by_id(id).await {
                Ok(media) => return Ok(media),
                Err(e) => {
                    if self.is_rate_limit_error(&e) && attempt + 1 < MAX_RETRIES {
                        self.limiter.handle_rate_limited(attempt);
                        warn!(
                            "[anilist] Rate limited fetching raw media ID {}, attempt {}/{}",
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

    /// Fetch manga plus extracted suggestion bundle from the same AniList payload.
    pub async fn grab_manga_with_suggestions(
        &self,
        id: i32,
    ) -> Result<(Manga, AniListSuggestionBundle), AniListError> {
        let mut last_err = None;
        for attempt in 0..MAX_RETRIES {
            self.limiter.wait_for_permit().await;

            match self.client.manga().get_anime_by_id(id).await {
                Ok(media) => {
                    debug!(
                        "[anilist] Found manga '{:?}' with ID {} (with suggestions)",
                        media.title.as_ref().and_then(|t| t.english.as_ref()),
                        id
                    );
                    let suggestions = suggestion_bundle_from_media(&media);
                    return Ok((media.into(), suggestions));
                }
                Err(e) => {
                    if self.is_rate_limit_error(&e) && attempt + 1 < MAX_RETRIES {
                        self.limiter.handle_rate_limited(attempt);
                        warn!(
                            "[anilist] Rate limited fetching ID {} with suggestions, attempt {}/{}",
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

            match self
                .client
                .manga()
                .get_popular_manga(Some(1), Some(25))
                .await
            {
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

    /// Fetch relation and recommendation candidates for a manga.
    pub async fn fetch_suggestions(&self, id: i32) -> Result<AniListSuggestionBundle, AniListError> {
        let media = self.grab_raw_media(id).await?;
        Ok(suggestion_bundle_from_media(&media))
    }

    pub async fn fetch_suggestion_details(
        &self,
        id: i32,
    ) -> Result<SuggestionMediaDetails, AniListError> {
        let media = self.grab_raw_media(id).await?;
        Ok(SuggestionMediaDetails {
            synopsis: media.description.as_deref().map(crate::manga::core::strip_html),
            community_rating: media.average_score,
            popularity: media.popularity,
            favourites: media.favourites,
        })
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
