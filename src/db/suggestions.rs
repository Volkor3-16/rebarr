use std::collections::HashMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum SuggestionSourceKind {
    Recommendation,
    Relation,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub enum SuggestionRelationKind {
    Adaptation,
    Prequel,
    Sequel,
    Parent,
    SideStory,
    Character,
    Summary,
    Alternative,
    SpinOff,
    Other,
    Source,
    Compilation,
    Contains,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SuggestionSourceRecord {
    pub source_manga_id: Uuid,
    pub source_title: String,
    pub source_kind: SuggestionSourceKind,
    pub relation_type: Option<SuggestionRelationKind>,
    pub context: Option<String>,
    pub rating: Option<i32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LibrarySuggestion {
    pub anilist_id: u32,
    pub title: String,
    pub cover_url: Option<String>,
    pub synopsis: Option<String>,
    pub media_format: Option<String>,
    pub publishing_status: Option<String>,
    pub tags: Vec<String>,
    pub community_rating: Option<i32>,
    pub popularity: Option<i32>,
    pub favourites: Option<i32>,
    pub total_occurrences: i32,
    pub recommendation_occurrences: i32,
    pub relation_occurrences: i32,
    pub weighted_score: f64,
    pub refreshed_at: i64,
    pub sources: Vec<SuggestionSourceRecord>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LibrarySuggestionList {
    pub library_id: Uuid,
    pub refreshed_at: Option<i64>,
    pub suggestions: Vec<LibrarySuggestion>,
}

#[derive(Debug, Clone)]
pub struct UpsertSuggestionCandidate {
    pub anilist_id: u32,
    pub title: String,
    pub cover_url: Option<String>,
    pub synopsis: Option<String>,
    pub media_format: Option<String>,
    pub publishing_status: Option<String>,
    pub tags: Vec<String>,
    pub community_rating: Option<i32>,
    pub popularity: Option<i32>,
    pub favourites: Option<i32>,
    pub total_occurrences: i32,
    pub recommendation_occurrences: i32,
    pub relation_occurrences: i32,
    pub weighted_score: f64,
}

#[derive(Debug, Clone)]
pub struct UpsertSuggestionSource {
    pub source_manga_id: Uuid,
    pub target_anilist_id: u32,
    pub source_kind: SuggestionSourceKind,
    pub relation_type: Option<SuggestionRelationKind>,
    pub context: Option<String>,
    pub rating: Option<i32>,
}

#[derive(sqlx::FromRow)]
struct CandidateRow {
    target_anilist_id: i64,
    title: String,
    cover_url: Option<String>,
    synopsis: Option<String>,
    media_format: Option<String>,
    publishing_status: Option<String>,
    tags_json: Option<String>,
    community_rating: Option<i32>,
    popularity: Option<i32>,
    favourites: Option<i32>,
    total_occurrences: i32,
    recommendation_occurrences: i32,
    relation_occurrences: i32,
    weighted_score: f64,
    refreshed_at: i64,
}

#[derive(sqlx::FromRow)]
struct SourceRow {
    source_manga_id: String,
    source_title: String,
    target_anilist_id: i64,
    source_kind: String,
    relation_type: Option<String>,
    context: Option<String>,
    rating: Option<i32>,
}

fn parse_relation_kind(value: Option<String>) -> Option<SuggestionRelationKind> {
    match value.as_deref() {
        Some("Adaptation") => Some(SuggestionRelationKind::Adaptation),
        Some("Prequel") => Some(SuggestionRelationKind::Prequel),
        Some("Sequel") => Some(SuggestionRelationKind::Sequel),
        Some("Parent") => Some(SuggestionRelationKind::Parent),
        Some("SideStory") => Some(SuggestionRelationKind::SideStory),
        Some("Character") => Some(SuggestionRelationKind::Character),
        Some("Summary") => Some(SuggestionRelationKind::Summary),
        Some("Alternative") => Some(SuggestionRelationKind::Alternative),
        Some("SpinOff") => Some(SuggestionRelationKind::SpinOff),
        Some("Other") => Some(SuggestionRelationKind::Other),
        Some("Source") => Some(SuggestionRelationKind::Source),
        Some("Compilation") => Some(SuggestionRelationKind::Compilation),
        Some("Contains") => Some(SuggestionRelationKind::Contains),
        _ => None,
    }
}

fn source_kind_str(kind: SuggestionSourceKind) -> &'static str {
    match kind {
        SuggestionSourceKind::Recommendation => "Recommendation",
        SuggestionSourceKind::Relation => "Relation",
    }
}

fn relation_kind_str(kind: SuggestionRelationKind) -> &'static str {
    match kind {
        SuggestionRelationKind::Adaptation => "Adaptation",
        SuggestionRelationKind::Prequel => "Prequel",
        SuggestionRelationKind::Sequel => "Sequel",
        SuggestionRelationKind::Parent => "Parent",
        SuggestionRelationKind::SideStory => "SideStory",
        SuggestionRelationKind::Character => "Character",
        SuggestionRelationKind::Summary => "Summary",
        SuggestionRelationKind::Alternative => "Alternative",
        SuggestionRelationKind::SpinOff => "SpinOff",
        SuggestionRelationKind::Other => "Other",
        SuggestionRelationKind::Source => "Source",
        SuggestionRelationKind::Compilation => "Compilation",
        SuggestionRelationKind::Contains => "Contains",
    }
}

pub async fn replace_library_suggestions(
    pool: &SqlitePool,
    library_id: Uuid,
    candidates: &[UpsertSuggestionCandidate],
    sources: &[UpsertSuggestionSource],
) -> Result<(), sqlx::Error> {
    let library_id_str = library_id.to_string();
    let now = chrono::Utc::now().timestamp();
    let hidden_rows: Vec<(i64, i64, Option<i64>)> = sqlx::query_as(
        "SELECT target_anilist_id, hidden, hidden_at
         FROM LibrarySuggestionCandidate
         WHERE library_id = ?",
    )
    .bind(&library_id_str)
    .fetch_all(pool)
    .await?;

    let hidden_map: HashMap<i64, (i64, Option<i64>)> = hidden_rows
        .into_iter()
        .map(|(id, hidden, hidden_at)| (id, (hidden, hidden_at)))
        .collect();

    let mut tx = pool.begin().await?;

    sqlx::query("DELETE FROM LibrarySuggestionSource WHERE library_id = ?")
        .bind(&library_id_str)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM LibrarySuggestionCandidate WHERE library_id = ?")
        .bind(&library_id_str)
        .execute(&mut *tx)
        .await?;

    for candidate in candidates {
        let key = candidate.anilist_id as i64;
        let (hidden, hidden_at) = hidden_map.get(&key).copied().unwrap_or((0, None));
        let tags_json = if candidate.tags.is_empty() {
            None
        } else {
            Some(
                serde_json::to_string(&candidate.tags)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
            )
        };

        sqlx::query(
            "INSERT INTO LibrarySuggestionCandidate (
                library_id, target_anilist_id, title, cover_url, synopsis, media_format, publishing_status,
                tags_json, community_rating, popularity, favourites, total_occurrences,
                recommendation_occurrences, relation_occurrences, weighted_score, hidden, hidden_at, refreshed_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&library_id_str)
        .bind(key)
        .bind(&candidate.title)
        .bind(candidate.cover_url.as_deref())
        .bind(candidate.synopsis.as_deref())
        .bind(candidate.media_format.as_deref())
        .bind(candidate.publishing_status.as_deref())
        .bind(tags_json.as_deref())
        .bind(candidate.community_rating)
        .bind(candidate.popularity)
        .bind(candidate.favourites)
        .bind(candidate.total_occurrences)
        .bind(candidate.recommendation_occurrences)
        .bind(candidate.relation_occurrences)
        .bind(candidate.weighted_score)
        .bind(hidden)
        .bind(hidden_at)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    for source in sources {
        sqlx::query(
            "INSERT INTO LibrarySuggestionSource (
                library_id, source_manga_id, target_anilist_id, source_kind, relation_type, context, rating, created_at
             ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&library_id_str)
        .bind(source.source_manga_id.to_string())
        .bind(source.target_anilist_id as i64)
        .bind(source_kind_str(source.source_kind))
        .bind(source.relation_type.map(relation_kind_str))
        .bind(source.context.as_deref())
        .bind(source.rating)
        .bind(now)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

pub async fn get_for_library(
    pool: &SqlitePool,
    library_id: Uuid,
) -> Result<LibrarySuggestionList, sqlx::Error> {
    let library_id_str = library_id.to_string();
    let candidates: Vec<CandidateRow> = sqlx::query_as(
        "SELECT target_anilist_id, title, cover_url, synopsis, media_format, publishing_status, tags_json,
                community_rating, popularity, favourites, total_occurrences,
                recommendation_occurrences, relation_occurrences, weighted_score, refreshed_at
         FROM LibrarySuggestionCandidate
         WHERE library_id = ? AND hidden = 0
         ORDER BY weighted_score DESC, total_occurrences DESC, relation_occurrences DESC,
                  COALESCE(community_rating, 0) DESC, COALESCE(popularity, 0) DESC, title ASC",
    )
    .bind(&library_id_str)
    .fetch_all(pool)
    .await?;

    let sources: Vec<SourceRow> = sqlx::query_as(
        "SELECT s.source_manga_id, m.title AS source_title, s.target_anilist_id,
                s.source_kind, s.relation_type, s.context, s.rating
         FROM LibrarySuggestionSource s
         JOIN Manga m ON m.uuid = s.source_manga_id
         WHERE s.library_id = ?
         ORDER BY s.target_anilist_id ASC, m.title ASC, s.source_kind ASC",
    )
    .bind(&library_id_str)
    .fetch_all(pool)
    .await?;

    let mut sources_by_target: HashMap<i64, Vec<SuggestionSourceRecord>> = HashMap::new();
    for row in sources {
        let source_manga_id =
            Uuid::parse_str(&row.source_manga_id).map_err(|e| sqlx::Error::Decode(Box::new(e)))?;
        let source_kind = match row.source_kind.as_str() {
            "Recommendation" => SuggestionSourceKind::Recommendation,
            _ => SuggestionSourceKind::Relation,
        };
        sources_by_target
            .entry(row.target_anilist_id)
            .or_default()
            .push(SuggestionSourceRecord {
                source_manga_id,
                source_title: row.source_title,
                source_kind,
                relation_type: parse_relation_kind(row.relation_type),
                context: row.context,
                rating: row.rating,
            });
    }

    let existing_ids: Vec<i64> = sqlx::query_scalar(
        "SELECT anilist_id FROM Manga WHERE library_id = ? AND anilist_id IS NOT NULL",
    )
    .bind(&library_id_str)
    .fetch_all(pool)
    .await?;

    let existing_ids: std::collections::HashSet<i64> = existing_ids.into_iter().collect();

    let refreshed_at = candidates.iter().map(|c| c.refreshed_at).max();
    let suggestions = candidates
        .into_iter()
        .filter(|row| !existing_ids.contains(&row.target_anilist_id))
        .map(|row| LibrarySuggestion {
            anilist_id: row.target_anilist_id as u32,
            title: row.title,
            cover_url: row.cover_url,
            synopsis: row.synopsis,
            media_format: row.media_format,
            publishing_status: row.publishing_status,
            tags: row
                .tags_json
                .and_then(|json| serde_json::from_str::<Vec<String>>(&json).ok())
                .unwrap_or_default(),
            community_rating: row.community_rating,
            popularity: row.popularity,
            favourites: row.favourites,
            total_occurrences: row.total_occurrences,
            recommendation_occurrences: row.recommendation_occurrences,
            relation_occurrences: row.relation_occurrences,
            weighted_score: row.weighted_score,
            refreshed_at: row.refreshed_at,
            sources: sources_by_target
                .remove(&row.target_anilist_id)
                .unwrap_or_default(),
        })
        .collect();

    Ok(LibrarySuggestionList {
        library_id,
        refreshed_at,
        suggestions,
    })
}

pub async fn set_hidden(
    pool: &SqlitePool,
    library_id: Uuid,
    anilist_id: u32,
    hidden: bool,
) -> Result<bool, sqlx::Error> {
    let hidden_at = if hidden {
        Some(chrono::Utc::now().timestamp())
    } else {
        None
    };
    let result = sqlx::query(
        "UPDATE LibrarySuggestionCandidate
         SET hidden = ?, hidden_at = ?
         WHERE library_id = ? AND target_anilist_id = ?",
    )
    .bind(if hidden { 1 } else { 0 })
    .bind(hidden_at)
    .bind(library_id.to_string())
    .bind(anilist_id as i64)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db;
    use crate::manga::core::{
        Library, Manga, MangaMetadata, MangaSource, MangaType, PublishingStatus,
    };
    use std::path::PathBuf;

    async fn test_pool() -> SqlitePool {
        db::init("sqlite::memory:").await.expect("db init")
    }

    async fn insert_library(pool: &SqlitePool, root: &str) -> Library {
        let lib = Library {
            uuid: Uuid::new_v4(),
            r#type: MangaType::Manga,
            root_path: PathBuf::from(root),
        };
        db::library::insert(pool, &lib)
            .await
            .expect("insert library");
        lib
    }

    async fn insert_manga(
        pool: &SqlitePool,
        library_id: Uuid,
        title: &str,
        anilist_id: u32,
    ) -> Manga {
        let manga = Manga {
            id: Uuid::new_v4(),
            library_id,
            anilist_id: Some(anilist_id),
            mal_id: None,
            metadata: MangaMetadata {
                title: title.to_owned(),
                other_titles: None,
                synopsis: None,
                publishing_status: PublishingStatus::Ongoing,
                tags: vec![],
                start_year: None,
                start_month: None,
                start_day: None,
                end_year: None,
                writer: None,
                penciller: None,
                inker: None,
                colorist: None,
                letterer: None,
                editor: None,
                translator: None,
                genre: None,
                community_rating: None,
            },
            relative_path: PathBuf::from(title),
            downloaded_count: None,
            chapter_count: None,
            metadata_source: MangaSource::AniList,
            thumbnail_url: None,
            monitored: true,
            created_at: 0,
            metadata_updated_at: 0,
            last_checked_at: None,
            last_chapter_at: None,
        };
        db::manga::insert(pool, &manga).await.expect("insert manga");
        manga
    }

    #[tokio::test]
    async fn replace_preserves_hidden_per_library_only() {
        let pool = test_pool().await;
        let lib_a = insert_library(&pool, "/tmp/lib-a").await;
        let lib_b = insert_library(&pool, "/tmp/lib-b").await;
        let source_a = insert_manga(&pool, lib_a.uuid, "Source A", 1).await;
        let source_b = insert_manga(&pool, lib_b.uuid, "Source B", 2).await;

        let candidate = UpsertSuggestionCandidate {
            anilist_id: 77,
            title: "Target".to_owned(),
            cover_url: None,
            synopsis: Some("A target synopsis".to_owned()),
            media_format: Some("Manga".to_owned()),
            publishing_status: Some("Ongoing".to_owned()),
            tags: vec!["Action".to_owned()],
            community_rating: Some(80),
            popularity: Some(1000),
            favourites: Some(100),
            total_occurrences: 1,
            recommendation_occurrences: 1,
            relation_occurrences: 0,
            weighted_score: 1.0,
        };
        let source = UpsertSuggestionSource {
            source_manga_id: source_a.id,
            target_anilist_id: 77,
            source_kind: SuggestionSourceKind::Recommendation,
            relation_type: None,
            context: None,
            rating: Some(50),
        };
        replace_library_suggestions(
            &pool,
            lib_a.uuid,
            std::slice::from_ref(&candidate),
            std::slice::from_ref(&source),
        )
        .await
        .expect("replace a");
        replace_library_suggestions(
            &pool,
            lib_b.uuid,
            std::slice::from_ref(&candidate),
            &[UpsertSuggestionSource {
                source_manga_id: source_b.id,
                ..source.clone()
            }],
        )
        .await
        .expect("replace b");

        assert!(set_hidden(&pool, lib_a.uuid, 77, true).await.expect("hide"));

        replace_library_suggestions(&pool, lib_a.uuid, &[candidate.clone()], &[source.clone()])
            .await
            .expect("replace a again");
        replace_library_suggestions(
            &pool,
            lib_b.uuid,
            &[candidate],
            &[UpsertSuggestionSource {
                source_manga_id: source_b.id,
                ..source
            }],
        )
        .await
        .expect("replace b again");

        let listed_a = get_for_library(&pool, lib_a.uuid).await.expect("list a");
        let listed_b = get_for_library(&pool, lib_b.uuid).await.expect("list b");
        assert!(listed_a.suggestions.is_empty());
        assert_eq!(listed_b.suggestions.len(), 1);
    }
}
