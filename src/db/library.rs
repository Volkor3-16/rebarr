use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::manga::{Library, MangaType};

// ---------------------------------------------------------------------------
// Deterministic UUID
// ---------------------------------------------------------------------------

/// Fixed namespace for library UUID v5 derivation.
const LIBRARY_NAMESPACE: Uuid = Uuid::from_bytes([
    0x3b, 0xe1, 0x7f, 0x22, 0x84, 0xc0, 0x4d, 0xb9,
    0xa2, 0x55, 0x00, 0x1f, 0x6e, 0x3c, 0x91, 0x05,
]);

/// Compute the deterministic UUID v5 for a library.
///
/// Key: `"{library_type}:{root_path}"` — unique per machine layout.
pub fn library_uuid(library_type: &str, root_path: &str) -> Uuid {
    let key = format!("{library_type}:{root_path}");
    Uuid::new_v5(&LIBRARY_NAMESPACE, key.as_bytes())
}

#[derive(sqlx::FromRow)]
struct LibraryRow {
    uuid: String,
    library_type: String,
    root_path: String,
}

fn from_row(row: LibraryRow) -> Result<Library, sqlx::Error> {
    let uuid = Uuid::parse_str(&row.uuid).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;

    let r#type = match row.library_type.as_str() {
        "Comics" => MangaType::Comics,
        "Manga" => MangaType::Manga,
        other => {
            return Err(sqlx::Error::Decode(
                format!("unknown library_type: {other}").into(),
            ));
        }
    };

    Ok(Library {
        uuid,
        r#type,
        root_path: PathBuf::from(row.root_path),
    })
}

pub async fn insert(pool: &SqlitePool, lib: &Library) -> Result<(), sqlx::Error> {
    let library_type = match lib.r#type {
        MangaType::Comics => "Comics",
        MangaType::Manga => "Manga",
    };

    sqlx::query("INSERT INTO Library (uuid, library_type, root_path) VALUES (?, ?, ?)")
        .bind(lib.uuid.to_string())
        .bind(library_type)
        .bind(lib.root_path.to_string_lossy().as_ref())
        .execute(pool)
        .await?;

    Ok(())
}

pub async fn get_by_id(pool: &SqlitePool, id: Uuid) -> Result<Option<Library>, sqlx::Error> {
    let row = sqlx::query_as::<_, LibraryRow>(
        "SELECT uuid, library_type, root_path FROM Library WHERE uuid = ?",
    )
    .bind(id.to_string())
    .fetch_optional(pool)
    .await?;

    row.map(from_row).transpose()
}

pub async fn get_all(pool: &SqlitePool) -> Result<Vec<Library>, sqlx::Error> {
    let rows = sqlx::query_as::<_, LibraryRow>(
        "SELECT uuid, library_type, root_path FROM Library ORDER BY rowid ASC",
    )
    .fetch_all(pool)
    .await?;

    rows.into_iter().map(from_row).collect()
}

pub async fn delete(pool: &SqlitePool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("DELETE FROM Library WHERE uuid = ?")
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn update_root_path(
    pool: &SqlitePool,
    id: Uuid,
    new_path: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE Library SET root_path = ? WHERE uuid = ?")
        .bind(new_path)
        .bind(id.to_string())
        .execute(pool)
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Data migration
// ---------------------------------------------------------------------------

#[derive(sqlx::FromRow)]
struct LibraryKeyRow {
    uuid: String,
    library_type: String,
    root_path: String,
}

/// One-time startup migration: recompute all library UUIDs deterministically.
///
/// Idempotent — skips if `DataMigrations` already records
/// `deterministic_library_uuids_v1`. Updates `Manga.library_id` and
/// `Task.library_id` references.
pub async fn backfill_deterministic_uuids(pool: &SqlitePool) -> Result<bool, sqlx::Error> {
    let already_ran: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM DataMigrations WHERE name = 'deterministic_library_uuids_v1'",
    )
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    if already_ran > 0 {
        return Ok(false); // false = nothing changed
    }

    let rows: Vec<LibraryKeyRow> = sqlx::query_as::<_, LibraryKeyRow>(
        "SELECT uuid, library_type, root_path FROM Library",
    )
    .fetch_all(pool)
    .await?;

    let mut any_changed = false;
    let mut tx = pool.begin().await?;

    // Temporarily disable FK enforcement so we can update PK/FK columns freely.
    sqlx::query("PRAGMA foreign_keys = OFF").execute(&mut *tx).await?;

    for row in &rows {
        let new_id = library_uuid(&row.library_type, &row.root_path);
        let new_id_str = new_id.to_string();

        if new_id_str == row.uuid {
            continue;
        }

        let collision: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM Library WHERE uuid = ?")
                .bind(&new_id_str)
                .fetch_one(&mut *tx)
                .await
                .unwrap_or(0);

        if collision > 0 {
            log::warn!("[backfill] Library UUID collision {} → {} — skipping.", row.uuid, new_id_str);
            continue;
        }

        sqlx::query("UPDATE Library SET uuid = ? WHERE uuid = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE Manga SET library_id = ? WHERE library_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;
        sqlx::query("UPDATE Task SET library_id = ? WHERE library_id = ?")
            .bind(&new_id_str).bind(&row.uuid).execute(&mut *tx).await?;

        any_changed = true;
    }

    sqlx::query("PRAGMA foreign_keys = ON").execute(&mut *tx).await?;

    sqlx::query(
        "INSERT OR IGNORE INTO DataMigrations (name, ran_at) VALUES ('deterministic_library_uuids_v1', unixepoch())",
    )
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;

    if any_changed {
        log::info!("[backfill] Backfilled deterministic UUIDs for {} library(s).", rows.len());
    }

    Ok(any_changed)
}
