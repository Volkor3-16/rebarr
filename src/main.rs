use dotenvy::dotenv;

mod metadata;
use crate::metadata::myanimelist

#[tokio::main]
async fn main() {
    // Init Logging
    env_logger::init();

    // Load .env variables
    dotenv().ok();

    // Create the MAL service
    let mal_service = MalService::new();
    
    // Test Query
    mal_service.search_manga("Frieren").await;
}