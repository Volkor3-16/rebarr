use std::collections::{HashMap, HashSet};

use anilist_moe::enums::media::MediaRelation;
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    db,
    db::suggestions::{
        SuggestionRelationKind, SuggestionSourceKind, UpsertSuggestionCandidate,
        UpsertSuggestionSource,
    },
    http::metadata::{AniListMetadata, anilist::suggestion_bundle_from_media},
};

#[derive(Debug, Clone)]
struct AggregatedCandidate {
    meta: AggregatedMeta,
    total_occurrences: i32,
    recommendation_occurrences: i32,
    relation_occurrences: i32,
    weighted_score: f64,
}

/// Minimal metadata extracted from the batch Page response for a suggestion candidate.
#[derive(Debug, Clone)]
struct AggregatedMeta {
    anilist_id: u32,
    title: String,
    cover_url: Option<String>,
    synopsis: Option<String>,
    media_format: Option<String>,
    publishing_status: Option<String>,
    tags: Vec<String>,
    community_rating: Option<i32>,
    popularity: Option<i32>,
    favourites: Option<i32>,
}

fn relation_kind(relation: MediaRelation) -> SuggestionRelationKind {
    match relation {
        MediaRelation::Adaptation => SuggestionRelationKind::Adaptation,
        MediaRelation::Prequel => SuggestionRelationKind::Prequel,
        MediaRelation::Sequel => SuggestionRelationKind::Sequel,
        MediaRelation::Parent => SuggestionRelationKind::Parent,
        MediaRelation::SideStory => SuggestionRelationKind::SideStory,
        MediaRelation::Character => SuggestionRelationKind::Character,
        MediaRelation::Summary => SuggestionRelationKind::Summary,
        MediaRelation::Alternative => SuggestionRelationKind::Alternative,
        MediaRelation::SpinOff => SuggestionRelationKind::SpinOff,
        MediaRelation::Other => SuggestionRelationKind::Other,
        MediaRelation::Source => SuggestionRelationKind::Source,
        MediaRelation::Compilation => SuggestionRelationKind::Compilation,
        MediaRelation::Contains => SuggestionRelationKind::Contains,
    }
}

fn relation_label(kind: SuggestionRelationKind) -> &'static str {
    match kind {
        SuggestionRelationKind::Adaptation => "Adaptation",
        SuggestionRelationKind::Prequel => "Prequel",
        SuggestionRelationKind::Sequel => "Sequel",
        SuggestionRelationKind::Parent => "Parent story",
        SuggestionRelationKind::SideStory => "Side story",
        SuggestionRelationKind::Character => "Character link",
        SuggestionRelationKind::Summary => "Summary",
        SuggestionRelationKind::Alternative => "Alternative version",
        SuggestionRelationKind::SpinOff => "Spin-off",
        SuggestionRelationKind::Other => "Related work",
        SuggestionRelationKind::Source => "Source material",
        SuggestionRelationKind::Compilation => "Compilation",
        SuggestionRelationKind::Contains => "Contains",
    }
}

pub async fn refresh_library_suggestions(
    pool: &SqlitePool,
    al: &AniListMetadata,
    library_id: Uuid,
) -> Result<(), String> {
    let library = db::library::get_by_id(pool, library_id)
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("library {library_id} not found"))?;

    if !matches!(library.r#type, crate::manga::core::MangaType::Manga) {
        db::suggestions::replace_library_suggestions(pool, library_id, &[], &[])
            .await
            .map_err(|e| e.to_string())?;
        return Ok(());
    }

    let manga_list = db::manga::get_all_for_library(pool, library_id)
        .await
        .map_err(|e| e.to_string())?;
    let existing_ids: HashSet<u32> = manga_list.iter().filter_map(|m| m.anilist_id).collect();

    // Load cached enrichment details from the previous refresh to minimise API calls.
    let enrichment_cache = db::suggestions::get_enrichment_cache(pool, library_id)
        .await
        .map_err(|e| e.to_string())?;

    // Build a map from anilist_id → manga so we can match batch results back to source rows.
    let manga_by_anilist: HashMap<u32, &crate::manga::core::Manga> = manga_list
        .iter()
        .filter_map(|m| m.anilist_id.map(|id| (id, m)))
        .collect();

    let all_ids: Vec<i32> = manga_by_anilist.keys().map(|id| *id as i32).collect();

    let mut aggregate: HashMap<u32, AggregatedCandidate> = HashMap::new();
    let mut source_rows: Vec<UpsertSuggestionSource> = Vec::new();

    if !all_ids.is_empty() {
        let batch_media = al
            .batch_fetch_with_suggestions(&all_ids)
            .await
            .map_err(|e| format!("AniList batch suggestion fetch failed: {e}"))?;

        for media in batch_media {
            let source_anilist_id = match media.id {
                Some(id) => id as u32,
                None => continue,
            };
            let source_manga = match manga_by_anilist.get(&source_anilist_id) {
                Some(m) => *m,
                None => continue,
            };

            let bundle = suggestion_bundle_from_media(&media);

            for rec in bundle.recommendations {
                if rec.target.anilist_id == source_anilist_id
                    || existing_ids.contains(&rec.target.anilist_id)
                {
                    continue;
                }
                let entry =
                    aggregate
                        .entry(rec.target.anilist_id)
                        .or_insert_with(|| AggregatedCandidate {
                            meta: AggregatedMeta {
                                anilist_id: rec.target.anilist_id,
                                title: rec.target.title.clone(),
                                cover_url: rec.target.cover_url.clone(),
                                synopsis: rec.target.synopsis.clone(),
                                media_format: rec.target.media_format.clone(),
                                publishing_status: rec.target.publishing_status.clone(),
                                tags: rec.target.tags.clone(),
                                community_rating: rec.target.community_rating,
                                popularity: rec.target.popularity,
                                favourites: rec.target.favourites,
                            },
                            total_occurrences: 0,
                            recommendation_occurrences: 0,
                            relation_occurrences: 0,
                            weighted_score: 0.0,
                        });
                entry.total_occurrences += 1;
                entry.recommendation_occurrences += 1;
                entry.weighted_score += 1.0 + (f64::from(rec.rating.unwrap_or(0)) / 1000.0);
                source_rows.push(UpsertSuggestionSource {
                    source_manga_id: source_manga.id,
                    target_anilist_id: rec.target.anilist_id,
                    source_kind: SuggestionSourceKind::Recommendation,
                    relation_type: None,
                    context: Some(format!("Recommended from {}", source_manga.metadata.title)),
                    rating: rec.rating,
                });
            }

            for relation in bundle.relations {
                if relation.target.anilist_id == source_anilist_id
                    || existing_ids.contains(&relation.target.anilist_id)
                {
                    continue;
                }
                let rk = relation_kind(relation.relation_type);
                let entry = aggregate
                    .entry(relation.target.anilist_id)
                    .or_insert_with(|| AggregatedCandidate {
                        meta: AggregatedMeta {
                            anilist_id: relation.target.anilist_id,
                            title: relation.target.title.clone(),
                            cover_url: relation.target.cover_url.clone(),
                            synopsis: relation.target.synopsis.clone(),
                            media_format: relation.target.media_format.clone(),
                            publishing_status: relation.target.publishing_status.clone(),
                            tags: relation.target.tags.clone(),
                            community_rating: relation.target.community_rating,
                            popularity: relation.target.popularity,
                            favourites: relation.target.favourites,
                        },
                        total_occurrences: 0,
                        recommendation_occurrences: 0,
                        relation_occurrences: 0,
                        weighted_score: 0.0,
                    });
                entry.total_occurrences += 1;
                entry.relation_occurrences += 1;
                entry.weighted_score += 1.35;
                source_rows.push(UpsertSuggestionSource {
                    source_manga_id: source_manga.id,
                    target_anilist_id: relation.target.anilist_id,
                    source_kind: SuggestionSourceKind::Relation,
                    relation_type: Some(rk),
                    context: Some(format!(
                        "{} to {}",
                        relation_label(rk),
                        source_manga.metadata.title
                    )),
                    rating: None,
                });
            }
        }
    }

    // Determine which candidates need fresh enrichment (not already cached with useful data).
    let uncached_ids: Vec<i32> = aggregate
        .keys()
        .filter(|id| !enrichment_cache.contains_key(id))
        .map(|id| *id as i32)
        .collect();

    let batch_details = if uncached_ids.is_empty() {
        HashMap::new()
    } else {
        al.batch_fetch_media_details(&uncached_ids)
            .await
            .map_err(|e| format!("AniList batch detail fetch failed: {e}"))?
    };

    // Apply enrichment from batch fetch or DB cache, whichever is available.
    let mut candidates: Vec<UpsertSuggestionCandidate> = aggregate
        .into_values()
        .map(|mut entry| {
            let id = entry.meta.anilist_id;
            if let Some(d) = batch_details.get(&id) {
                if entry.meta.synopsis.is_none() {
                    entry.meta.synopsis = d.synopsis.clone();
                }
                if entry.meta.community_rating.is_none() {
                    entry.meta.community_rating = d.community_rating;
                }
                if entry.meta.popularity.is_none() {
                    entry.meta.popularity = d.popularity;
                }
                if entry.meta.favourites.is_none() {
                    entry.meta.favourites = d.favourites;
                }
            } else if let Some(c) = enrichment_cache.get(&id) {
                if entry.meta.synopsis.is_none() {
                    entry.meta.synopsis = c.synopsis.clone();
                }
                if entry.meta.community_rating.is_none() {
                    entry.meta.community_rating = c.community_rating;
                }
                if entry.meta.popularity.is_none() {
                    entry.meta.popularity = c.popularity;
                }
                if entry.meta.favourites.is_none() {
                    entry.meta.favourites = c.favourites;
                }
            }

            UpsertSuggestionCandidate {
                anilist_id: entry.meta.anilist_id,
                title: entry.meta.title,
                cover_url: entry.meta.cover_url,
                synopsis: entry.meta.synopsis,
                media_format: entry.meta.media_format,
                publishing_status: entry.meta.publishing_status,
                tags: entry.meta.tags,
                community_rating: entry.meta.community_rating,
                popularity: entry.meta.popularity,
                favourites: entry.meta.favourites,
                total_occurrences: entry.total_occurrences,
                recommendation_occurrences: entry.recommendation_occurrences,
                relation_occurrences: entry.relation_occurrences,
                weighted_score: entry.weighted_score
                    + (entry.relation_occurrences as f64 * 0.15)
                    + (entry.meta.community_rating.unwrap_or(0) as f64 / 1000.0)
                    + (entry.meta.popularity.unwrap_or(0) as f64 / 1_000_000.0),
            }
        })
        .collect();

    candidates.sort_by(|a, b| {
        b.weighted_score
            .partial_cmp(&a.weighted_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.total_occurrences.cmp(&a.total_occurrences))
            .then_with(|| b.relation_occurrences.cmp(&a.relation_occurrences))
            .then_with(|| {
                b.community_rating
                    .unwrap_or(0)
                    .cmp(&a.community_rating.unwrap_or(0))
            })
            .then_with(|| b.popularity.unwrap_or(0).cmp(&a.popularity.unwrap_or(0)))
            .then_with(|| a.title.cmp(&b.title))
    });

    db::suggestions::replace_library_suggestions(pool, library_id, &candidates, &source_rows)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relation_labels_are_collection_friendly() {
        assert_eq!(relation_label(SuggestionRelationKind::Sequel), "Sequel");
        assert_eq!(
            relation_label(SuggestionRelationKind::SideStory),
            "Side story"
        );
    }
}
