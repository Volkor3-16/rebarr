use anilist_moe::{AniListClient, AniListError, enums::media::MediaFormat, objects::media::Media};
use tracing::debug;

use crate::manga::core::Manga;

/// Service for interacting with anilist API
pub struct ALClient {
    api_client: AniListClient,
}

impl Default for ALClient {
    fn default() -> Self {
        Self::new()
    }
}

impl ALClient {
    /// Creates a new instance of ALClient
    pub fn new() -> Self {
        // Construct ALClient
        ALClient {
            api_client: AniListClient::new(),
        }
    }

    /// Performs a manga search for a given title
    /// Returns the raw Page<Vec<Media>>
    pub async fn search_manga(
        &self,
        title: &str,
    ) -> Result<
        anilist_moe::objects::responses::Page<Vec<anilist_moe::objects::media::Media>>,
        AniListError,
    > {
        let response = self
            .api_client
            .manga()
            .search_manga(title, Some(1), Some(10))
            .await?;
        debug!(
            "Found {} manga results for '{}'",
            response.data.len(),
            title
        );
        Ok(response)
    }

    /// Converts search results Media type into Manga Struct objects
    pub async fn search_manga_as_manga(&self, title: &str) -> Result<Vec<Manga>, AniListError> {
        let page = self.search_manga(title).await?;
        Ok(page
            .data
            .into_iter()
            .filter(|media| {
                matches!(
                    media.format,
                    Some(MediaFormat::Manga) | Some(MediaFormat::OneShot)
                )
            })
            .map(|media| media.into())
            .collect())
    }

    /// Grabs the metadata for a specific AniList ID and converts to internal Manga struct
    pub async fn grab_manga(&self, id: i32) -> Result<Manga, AniListError> {
        let response = self.api_client.manga().get_anime_by_id(id).await?;
        debug!(
            "Found manga '{:?}' with ID '{:?}'",
            response.title.as_ref().and_then(|t| t.english.as_ref()),
            response.id
        );
        Ok(response.into())
    }

    /// Grabs the raw AniList Media payload for inspection/debugging.
    pub async fn grab_raw_media(&self, id: i32) -> Result<Media, AniListError> {
        self.api_client.manga().get_anime_by_id(id).await
    }

    /// Grabs popular manga for new instance onboarding
    pub async fn popular_manga(&self) -> Result<Vec<Manga>, AniListError> {
        let page = self
            .api_client
            .manga()
            .get_popular_manga(Some(1), Some(25))
            .await?;
        Ok(page.data.into_iter().map(|media| media.into()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Live debug helper for verifying AniList's raw fetch payload.
    // Run with:
    //   cargo test live_anilist_fetch_contains_relations_and_recommendations --lib -- --ignored --nocapture
    #[tokio::test]
    #[ignore = "live AniList network test for debugging raw metadata payloads"]
    async fn live_anilist_fetch_contains_relations_and_recommendations() {
        let client = ALClient::new();
        let media = client
            .grab_raw_media(101468)
            .await
            .expect("fetch AniList media");

        let relation_count = media
            .relations
            .as_ref()
            .and_then(|rels| rels.edges.as_ref())
            .map(|edges| edges.len())
            .unwrap_or(0);
        let recommendation_count = media
            .recommendations
            .as_ref()
            .and_then(|recs| recs.nodes.as_ref())
            .map(|nodes| nodes.len())
            .unwrap_or(0);

        println!(
            "AniList raw media summary: id={:?} title={:?} relations={} recommendations={}",
            media.id,
            media.title.as_ref().and_then(|t| t
                .english
                .as_ref()
                .or(t.romaji.as_ref())
                .or(t.user_preferred.as_ref())),
            relation_count,
            recommendation_count
        );
        println!(
            "{}",
            serde_json::to_string_pretty(&media).expect("serialize raw AniList media")
        );

        assert!(
            relation_count > 0,
            "expected at least one relation in raw AniList payload"
        );
        assert!(
            recommendation_count > 0,
            "expected at least one recommendation in raw AniList payload"
        );
    }
}
