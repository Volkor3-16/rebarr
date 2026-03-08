use sqlx::SqlitePool;

/// Fetch a settings value by key. Returns `default` if the key does not exist.
pub async fn get(pool: &SqlitePool, key: &str, default: &str) -> Result<String, sqlx::Error> {
    let val: Option<String> =
        sqlx::query_scalar("SELECT value FROM Settings WHERE key = ?")
            .bind(key)
            .fetch_optional(pool)
            .await?;
    Ok(val.unwrap_or_else(|| default.to_owned()))
}

/// Upsert a settings value.
pub async fn set(pool: &SqlitePool, key: &str, value: &str) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO Settings (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(pool)
        .await?;
    Ok(())
}
