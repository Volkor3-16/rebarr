use dotenvy::dotenv;

mod db;
mod manga;
mod metadata;
mod web;

use crate::metadata::anilist::ALClient;

#[rocket::main]
async fn main() -> Result<(), rocket::Error> {
    dotenv().ok();
    env_logger::init();

    let pool = db::init("sqlite:rebarr.db").await.expect("DB init failed");
    let al_client = ALClient::new();

    rocket::build()
        .manage(pool)
        .manage(al_client)
        .mount("/", web::routes())
        .launch()
        .await?;

    Ok(())
}
