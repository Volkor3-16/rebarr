use log::{debug, info};
use mal_api::oauth::MalClientId;
use mal_api::prelude::*;
use std::env;

/// Service for interacting with the MyAnimeList API
pub struct MalService {
    api_client: MangaApiClient,
}

impl MalService {
    /// Creates a new instance of MalService
    pub fn new() -> Self {
        // Construct the API Client
        let client_id = MalClientId::try_from_env().expect("MAL_CLIENT_ID environment variable not found. Please set it in your .env file");
        debug!("My Anime List Client ID: {client_id:?}");
        let api_client = MangaApiClient::from(&client_id);
        
        MalService { api_client }
    }

    /// Performs a manga search query
    pub async fn search_manga(&self, title: &str) -> Result<(), Box<dyn std::error::Error>> {
        let common_fields = mal_api::manga::all_common_fields();
        let detail_fields = mal_api::manga::all_detail_fields();

        // Test Query
        let query = GetMangaList::builder(title)
            .fields(&common_fields)
            .limit(3)
            .build()
            .unwrap();
        let response = self.api_client.get_manga_list(&query).await;
        if let Ok(response) = response {
            debug!("Response: {response}");
        }
        
        Ok(())
    }
}