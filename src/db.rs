use sqlx::{SqlitePool, sqlite::SqliteConnectOptions};
use std::str::FromStr;

pub mod library;
pub mod manga;

/// Initialise the database pool.
///
/// - Creates the SQLite file if it does not exist.
/// - Enables foreign key enforcement on every pooled connection via
///   `SqliteConnectOptions::pragma` (a per-connection setting that would be
///   lost if set on a single connection after pool creation).
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
