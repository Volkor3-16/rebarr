use dotenvy::dotenv;
use log::debug;

mod metadata;
mod manga;
use crate::{manga::Manga, metadata::anilist::{self, ALClient}};

#[tokio::main]
async fn main() {
    // Init Logging
    env_logger::init();

    // Load .env variables
    dotenv().ok();

    // Create Anilist service
    let alclient = ALClient::new();
    
    // Test Searching (and grab the ID from the first result)
    let search = alclient.search_manga("Frieren").await.unwrap();

    let frieren = alclient.grab_manga(search.data.first().unwrap().id.unwrap()).await;

    debug!("Frieren Struct: {:#?}", frieren);
    // Test conversion to internal manga type
    //let converted_frieren = Manga::from(frieren.unwrap());
}