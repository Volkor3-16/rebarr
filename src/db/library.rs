use std::path::PathBuf;

use sqlx::SqlitePool;
use uuid::Uuid;

use crate::manga::manga::{Library, MangaType};

// ---------------------------------------------------------------------------
// Deterministic UUID
// ---------------------------------------------------------------------------

/// Fixed namespace for library UUID v5 derivation.
const LIBRARY_NAMESPACE: Uuid = Uuid::from_bytes([
    0x3b, 0xe1, 0x7f, 0x22, 0x84, 0xc0, 0x4d, 0xb9, 0xa2, 0x55, 0x00, 0x1f, 0x6e, 0x3c, 0x91, 0x05,
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
