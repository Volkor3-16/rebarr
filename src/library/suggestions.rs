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
    http::metadata::{
        AniListMetadata,
        anilist::{AniListSuggestionBundle, SuggestionMedia},
    },
};

#[derive(Debug, Clone)]
struct AggregatedCandidate {
    meta: SuggestionMedia,
    total_occurrences: i32,
    recommendation_occurrences: i32,
    relation_occurrences: i32,
    weighted_score: f64,
}

async fn enrich_candidate_details(
    al: &AniListMetadata,
    entry: &mut AggregatedCandidate,
) -> Result<(), String> {
    let details = al
        .fetch_suggestion_details(entry.meta.anilist_id as i32)
        .await
        .map_err(|e| {
            format!(
                "AniList detail fetch failed for {}: {e}",
                entry.meta.anilist_id
            )
        })?;

    if entry.meta.synopsis.is_none() {
        entry.meta.synopsis = details.synopsis;
    }
    if entry.meta.community_rating.is_none() {
        entry.meta.community_rating = details.community_rating;
    }
    if entry.meta.popularity.is_none() {
        entry.meta.popularity = details.popularity;
    }
    if entry.meta.favourites.is_none() {
        entry.meta.favourites = details.favourites;
    }

    Ok(())
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
    refresh_library_suggestions_with_seed(pool, al, library_id, None).await
}

pub async fn refresh_library_suggestions_with_seed(
    pool: &SqlitePool,
    al: &AniListMetadata,
    library_id: Uuid,
    seeded_bundle: Option<(Uuid, AniListSuggestionBundle)>,
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

    let mut aggregate: HashMap<u32, AggregatedCandidate> = HashMap::new();
    let mut source_rows: Vec<UpsertSuggestionSource> = Vec::new();
    let seeded_by_manga = seeded_bundle
        .as_ref()
        .map(|(manga_id, bundle)| (*manga_id, bundle));

    for manga in manga_list.into_iter().filter(|m| m.anilist_id.is_some()) {
        let source_anilist_id = manga.anilist_id.expect("filtered");
        let bundle = if let Some((seeded_manga_id, seeded)) = seeded_by_manga {
            if seeded_manga_id == manga.id {
                seeded.clone()
            } else {
                al.fetch_suggestions(source_anilist_id as i32)
                    .await
                    .map_err(|e| {
                        format!(
                            "AniList suggestion fetch failed for '{}': {e}",
                            manga.metadata.title
                        )
                    })?
            }
        } else {
            al.fetch_suggestions(source_anilist_id as i32)
                .await
                .map_err(|e| {
                    format!(
                        "AniList suggestion fetch failed for '{}': {e}",
                        manga.metadata.title
                    )
                })?
        };

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
                        meta: rec.target.clone(),
                        total_occurrences: 0,
                        recommendation_occurrences: 0,
                        relation_occurrences: 0,
                        weighted_score: 0.0,
                    });
            entry.total_occurrences += 1;
            entry.recommendation_occurrences += 1;
            entry.weighted_score += 1.0 + (f64::from(rec.rating.unwrap_or(0)) / 1000.0);
            source_rows.push(UpsertSuggestionSource {
                source_manga_id: manga.id,
                target_anilist_id: rec.target.anilist_id,
                source_kind: SuggestionSourceKind::Recommendation,
                relation_type: None,
                context: Some(format!("Recommended from {}", manga.metadata.title)),
                rating: rec.rating,
            });
        }

        for relation in bundle.relations {
            if relation.target.anilist_id == source_anilist_id
                || existing_ids.contains(&relation.target.anilist_id)
            {
                continue;
            }
            let entry = aggregate
                .entry(relation.target.anilist_id)
                .or_insert_with(|| AggregatedCandidate {
                    meta: relation.target.clone(),
                    total_occurrences: 0,
                    recommendation_occurrences: 0,
                    relation_occurrences: 0,
                    weighted_score: 0.0,
                });
            let relation_kind = relation_kind(relation.relation_type);
            entry.total_occurrences += 1;
            entry.relation_occurrences += 1;
            entry.weighted_score += 1.35;
            source_rows.push(UpsertSuggestionSource {
                source_manga_id: manga.id,
                target_anilist_id: relation.target.anilist_id,
                source_kind: SuggestionSourceKind::Relation,
                relation_type: Some(relation_kind),
                context: Some(format!(
                    "{} to {}",
                    relation_label(relation_kind),
                    manga.metadata.title
                )),
                rating: None,
            });
        }
    }

    let mut enriched_entries = Vec::new();
    for mut entry in aggregate.into_values() {
        enrich_candidate_details(al, &mut entry).await?;
        enriched_entries.push(entry);
    }

    let mut candidates: Vec<UpsertSuggestionCandidate> = enriched_entries
        .into_iter()
        .map(|entry| UpsertSuggestionCandidate {
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
