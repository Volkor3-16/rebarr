use dotenvy::dotenv;

mod api;
mod covers;
mod db;
mod manga;
mod metadata;
mod web;

use crate::metadata::anilist::ALClient;

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();
    env_logger::init();

    tokio::fs::create_dir_all("./thumbnails")
        .await
        .expect("Failed to create thumbnails directory");

    let pool = db::init("sqlite:rebarr.db").await.expect("DB init failed");
    let al_client = ALClient::new();
    let http_client = reqwest::Client::new();

    rocket::build()
        .manage(pool)
        .manage(al_client)
        .manage(http_client)
        .mount("/", web::routes())
        .mount("/", api::routes())
        .mount("/thumbnails", rocket::fs::FileServer::from("./thumbnails"))
        .launch()
        .await?;

    Ok(())
}
