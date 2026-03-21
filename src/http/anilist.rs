use anilist_moe::{AniListClient, AniListError, enums::media::MediaFormat};
use log::{debug, info};

use crate::manga::manga::Manga;

/// Service for interacting with anilist API
pub struct ALClient {
    api_client: AniListClient,
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
