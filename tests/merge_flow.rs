/// Integration tests for the scan_manga / check_new_chapters merge flow.
///
/// Uses StaticProvider so no network or browser is involved.
mod helpers;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use helpers::{
    insert_library, insert_manga,
    static_provider::{StaticChapter, StaticProvider, ch, ch_group},
    test_ctx, test_db,
};
use rebarr::{
    db::{chapter as db_chapter, provider as db_provider},
    manga::merge,
    scraper::{
        PageUrl, Provider, ProviderChapterInfo, ProviderRegistry, ProviderSearchResult,
        ProviderVariables, ScraperCtx, error::ScraperError,
    },
};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helper: build a registry + ctx backed by a single StaticProvider
// ---------------------------------------------------------------------------

fn provider_registry(p: StaticProvider) -> ProviderRegistry {
    ProviderRegistry::from_providers_for_tests(vec![Arc::new(p)])
}

struct VariableBackedProvider {
    chapter_calls: Arc<AtomicUsize>,
}

#[async_trait]
impl Provider for VariableBackedProvider {
    fn name(&self) -> &str {
        "vars"
    }

    fn needs_browser(&self) -> bool {
        false
    }

    async fn search(
        &self,
        _ctx: &ScraperCtx,
        title: &str,
    ) -> Result<Vec<ProviderSearchResult>, ScraperError> {
        Ok(vec![ProviderSearchResult {
            title: title.to_owned(),
            url: "static://shared-series-url".to_owned(),
            cover_url: None,
            variables: HashMap::from([("manga_id".to_owned(), "series-123".to_owned())]),
        }])
    }

    async fn chapters(
        &self,
        _ctx: &ScraperCtx,
        _manga_url: &str,
        variables: &ProviderVariables,
    ) -> Result<Vec<ProviderChapterInfo>, ScraperError> {
        if variables.get("manga_id").map(String::as_str) != Some("series-123") {
            return Ok(vec![]);
        }

        let call = self.chapter_calls.fetch_add(1, Ordering::SeqCst);
        let raws: &[&str] = if call == 0 {
            &["1", "2"]
        } else {
            &["1", "2", "3"]
        };

        Ok(raws
            .iter()
            .map(|raw| {
                let number = raw.parse::<f32>().unwrap();
                ProviderChapterInfo {
                    raw_number: (*raw).to_owned(),
                    number,
                    chapter_base: number.floor(),
                    chapter_variant: 0,
                    is_extra: false,
                    title: None,
                    url: Some(format!("static://shared-series-url/{raw}")),
                    volume: None,
                    scanlator_group: None,
                    language: Some("en".to_owned()),
                    date_released: None,
                }
            })
            .collect())
    }

    async fn pages(
        &self,
        _ctx: &ScraperCtx,
        _chapter_url: &str,
    ) -> Result<Vec<PageUrl>, ScraperError> {
        Ok(vec![])
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// A full scan from scratch finds chapters and inserts them.
#[tokio::test]
async fn scan_inserts_chapters_for_clean_numbering() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "Berserk").await;

    let provider = StaticProvider::new("static").with_series(
        "Berserk",
        "static://berserk",
        vec![ch("1"), ch("2"), ch("3"), ch("10")],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    let result = merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    assert_eq!(result.providers_found, 1, "expected 1 provider found");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 4, "expected 4 chapters inserted");

    let bases: Vec<i32> = {
        let mut v: Vec<i32> = chapters.iter().map(|c| c.chapter_base).collect();
        v.sort();
        v.dedup();
        v
    };
    assert_eq!(bases, vec![1, 2, 3, 10]);
}

/// Chapters with decimal numbers (.5) are parsed and stored correctly.
#[tokio::test]
async fn scan_handles_half_chapters() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "HalfSeries").await;

    let provider = StaticProvider::new("static").with_series(
        "HalfSeries",
        "static://half",
        vec![ch("12"), ch("12.5"), ch("13")],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 3);

    // Find chapter 12.5: base=12, variant=5
    let half = chapters.iter().find(|c| c.chapter_variant == 5);
    assert!(half.is_some(), "chapter 12.5 (variant=5) not found");
    assert_eq!(half.unwrap().chapter_base, 12);
}

/// Numbered chapters with "Side Story" in the title should stay normal chapters.
#[tokio::test]
async fn scan_does_not_mark_side_story_titles_as_extra() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "SideStorySeries").await;

    let provider = StaticProvider::new("static").with_series(
        "SideStorySeries",
        "static://side-story",
        vec![
            StaticChapter::new("200").with_title("Side Story 20"),
            ch("201"),
        ],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");
    let side_story = chapters
        .iter()
        .find(|c| c.chapter_base == 200 && c.chapter_variant == 0)
        .expect("chapter 200 should exist");

    assert!(
        !side_story.is_extra,
        "side story titles should not be auto-marked as extras"
    );
}

/// Variant extras should still be classified as extras.
#[tokio::test]
async fn scan_keeps_true_extra_variants_marked_as_extra() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "ExtraVariantSeries").await;

    let provider = StaticProvider::new("static").with_series(
        "ExtraVariantSeries",
        "static://extra-variant",
        vec![
            StaticChapter::new("12.5").with_title("Extra"),
            ch("13"),
        ],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");
    let extra = chapters
        .iter()
        .find(|c| c.chapter_base == 12 && c.chapter_variant == 5)
        .expect("chapter 12.5 should exist");

    assert!(extra.is_extra, "real extra variants should stay marked extra");
}

/// Two scanlator groups providing the same chapter number both get stored.
#[tokio::test]
async fn scan_stores_multiple_groups_for_same_chapter() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "MultiGroup").await;

    let provider = StaticProvider::new("static").with_series(
        "MultiGroup",
        "static://multigroup",
        vec![
            ch_group("1", "GroupA"),
            ch_group("1", "GroupB"), // same chapter, different group
            ch("2"),
        ],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");

    // Three rows: GroupA ch1, GroupB ch1, ch2
    assert_eq!(chapters.len(), 3);

    let ch1s: Vec<_> = chapters.iter().filter(|c| c.chapter_base == 1).collect();
    assert_eq!(
        ch1s.len(),
        2,
        "both scanlator groups for ch1 should be stored"
    );
}

/// Letter-suffixed chapters ("6a", "6b") parse into correct variants.
#[tokio::test]
async fn scan_handles_letter_variant_chapters() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "LetterSeries").await;

    let provider = StaticProvider::new("static").with_series(
        "LetterSeries",
        "static://letters",
        vec![ch("6a"), ch("6b"), ch("7")],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    let chapters = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters");
    assert_eq!(chapters.len(), 3);

    let ch6a = chapters
        .iter()
        .find(|c| c.chapter_base == 6 && c.chapter_variant == 1);
    let ch6b = chapters
        .iter()
        .find(|c| c.chapter_base == 6 && c.chapter_variant == 2);
    assert!(ch6a.is_some(), "6a → variant 1 not found");
    assert!(ch6b.is_some(), "6b → variant 2 not found");
}

/// A second scan does not duplicate chapters already in the DB.
#[tokio::test]
async fn scan_is_idempotent() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "Idempotent").await;

    let provider = StaticProvider::new("static").with_series(
        "Idempotent",
        "static://idempotent",
        vec![ch("1"), ch("2"), ch("3")],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);
    let task_id = Uuid::new_v4();

    merge::scan_manga(&pool, &registry, &ctx, &manga, task_id)
        .await
        .expect("first scan failed");
    let first_count = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters")
        .len();

    // Second scan — provider has already been searched, goes through check_new_chapters path
    merge::check_new_chapters(&pool, &registry, &ctx, &manga, task_id)
        .await
        .expect("second scan failed");
    let second_count = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters")
        .len();

    assert_eq!(
        first_count, second_count,
        "second scan should not add duplicates"
    );
}

/// Search-time provider variables are persisted and available during later chapter sync jobs.
#[tokio::test]
async fn check_new_chapters_uses_persisted_provider_variables() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "VariableSeries").await;

    let provider: Arc<dyn Provider> = Arc::new(VariableBackedProvider {
        chapter_calls: Arc::new(AtomicUsize::new(0)),
    });
    let registry = ProviderRegistry::from_providers_for_tests(vec![provider]);
    let ctx = test_ctx(&registry);
    let task_id = Uuid::new_v4();

    merge::scan_manga(&pool, &registry, &ctx, &manga, task_id)
        .await
        .expect("first scan failed");

    let entries = db_provider::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get providers");
    assert_eq!(entries.len(), 1);
    assert_eq!(
        entries[0].provider_data.get("manga_id").map(String::as_str),
        Some("series-123")
    );

    let first_count = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters")
        .len();
    assert_eq!(first_count, 2, "first sync should use persisted variable");

    merge::check_new_chapters(&pool, &registry, &ctx, &manga, task_id)
        .await
        .expect("second scan failed");

    let second_count = db_chapter::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get chapters")
        .len();
    assert_eq!(
        second_count, 3,
        "background sync should reuse persisted variable"
    );
}

/// A no-browser provider with no matching series stores a not-found marker.
#[tokio::test]
async fn scan_records_not_found_when_no_match() {
    let pool = test_db().await;
    let lib = insert_library(&pool).await;
    let manga = insert_manga(&pool, lib.uuid, "UnknownTitle12345").await;

    let provider = StaticProvider::new("static").with_series(
        "CompleteDifferent",
        "static://other",
        vec![ch("1")],
    );
    let registry = provider_registry(provider);
    let ctx = test_ctx(&registry);

    let result = merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan_manga failed");

    assert_eq!(result.providers_found, 0);
    assert_eq!(result.new_chapters, 0);

    // Provider should be recorded as "searched, not found"
    let entries = db_provider::get_all_for_manga(&pool, manga.id)
        .await
        .expect("get providers");
    assert_eq!(entries.len(), 1);
    assert!(
        entries[0].provider_url.is_none(),
        "not-found entry should have no URL"
    );
}
