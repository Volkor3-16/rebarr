use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

pub(crate) mod chapter;
pub(crate) mod library;
pub(crate) mod manga;
pub(crate) mod provider;
pub(crate) mod provider_scores;
pub(crate) mod settings;
pub(crate) mod task;

/// Initialise the database pool.
///
/// - Creates the SQLite file if it does not exist.
/// - Enables foreign key enforcement and journal_mode on the connections.
/// - Runs any pending migrations from the `migrations/` directory.
pub async fn init(db_url: &str) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::from_str(db_url)?
        .pragma("foreign_keys", "ON")
        .pragma("journal_mode", "WAL")
        .create_if_missing(true);

    let pool = SqlitePool::connect_with(opts).await?;
    sqlx::migrate!("./migrations").run(&pool).await?;
    Ok(pool)
}
