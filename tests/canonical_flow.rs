/// End-to-end test of the canonical chapter selection + auto-download queueing system.
///
/// Scenario — two providers with different quality tiers:
///   ProviderA: scanlator group "TrustedGroup"  → tier 2 (trusted)
///   ProviderB: scanlator group "RandomGroup"   → tier 3 (unknown)
///
/// Scan 1 — initial full scan (both providers first-time):
///   ProviderA has chapters 1-5.  ProviderB has chapters 1-3.
///   Because it is the first sync for both providers, new_ids is empty → nothing queued.
///
/// Scan 2 — incremental check (both providers previously synced):
///   ProviderA adds ch6 (new).
///   ProviderB adds ch4 and ch5 (new to B, but ch4/ch5 already exist from ProviderA → ProviderA wins).
///   → ProviderA's ch6 is canonical + new            → 1 DownloadChapter task queued.
///   → ProviderB's ch4/ch5 are new but NOT canonical → no tasks.
///
/// Scan 3 — incremental check:
///   ProviderA: no new chapters.
///   ProviderB adds ch6 (new to B, but ProviderA's ch6 is already canonical).
///   → ProviderB's ch6 is new but NOT canonical → no new tasks.
mod helpers;

use std::sync::Arc;

use helpers::{
    test_db, test_ctx,
    static_provider::{StaticProvider, ch_group},
};
use rebarr::{
    db::{chapter as db_chapter, provider as db_provider},
    manga::{core::MangaType, merge},
    scraper::ProviderRegistry,
};
use sqlx::SqlitePool;
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Build a ProviderRegistry containing exactly the two providers for this scenario.
fn make_registry(
    a_chapters: Vec<helpers::static_provider::StaticChapter>,
    b_chapters: Vec<helpers::static_provider::StaticChapter>,
) -> ProviderRegistry {
    let provider_a = StaticProvider::new("ProviderA")
        .with_series("TestManga", "static://testmanga-a", a_chapters);
    let provider_b = StaticProvider::new("ProviderB")
        .with_series("TestManga", "static://testmanga-b", b_chapters);

    ProviderRegistry::from_providers_for_tests(vec![
        Arc::new(provider_a),
        Arc::new(provider_b),
    ])
}

/// Insert a monitored manga into the test DB and return it.
async fn setup_manga(pool: &SqlitePool) -> rebarr::manga::core::Manga {
    use std::path::PathBuf;
    use rebarr::manga::core::{Manga, MangaMetadata, MangaSource, PublishingStatus};

    let lib = rebarr::manga::core::Library {
        uuid: Uuid::new_v4(),
        r#type: MangaType::Manga,
        root_path: PathBuf::from("/tmp/rebarr-canonical-test"),
    };
    rebarr::db::library::insert(pool, &lib).await.unwrap();

    let manga = Manga {
        id: Uuid::new_v4(),
        library_id: lib.uuid,
        anilist_id: None,
        mal_id: None,
        metadata: MangaMetadata {
            title: "TestManga".to_owned(),
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
        relative_path: PathBuf::from("testmanga"),
        downloaded_count: Some(0),
        chapter_count: None,
        metadata_source: MangaSource::Local,
        thumbnail_url: None,
        monitored: true,      // ← MUST be true for auto-download to fire
        created_at: chrono::Utc::now().timestamp(),
        metadata_updated_at: chrono::Utc::now().timestamp(),
        last_checked_at: None,
    };
    rebarr::db::manga::insert(pool, &manga).await.unwrap();
    manga
}

/// Count Pending DownloadChapter tasks for a manga in the task queue.
async fn count_pending_downloads(pool: &SqlitePool, manga_id: Uuid) -> i64 {
    sqlx::query_scalar(
        "SELECT COUNT(*) FROM Task
         WHERE manga_id = ? AND task_type = 'DownloadChapter' AND status = 'Pending'",
    )
    .bind(manga_id.to_string())
    .fetch_one(pool)
    .await
    .unwrap()
}

/// Return the chapter_base values of chapters whose download task is Pending.
async fn pending_download_chapter_bases(pool: &SqlitePool, manga_id: Uuid) -> Vec<i32> {
    let chapter_ids: Vec<String> = sqlx::query_scalar(
        "SELECT chapter_id FROM Task
         WHERE manga_id = ? AND task_type = 'DownloadChapter' AND status = 'Pending'",
    )
    .bind(manga_id.to_string())
    .fetch_all(pool)
    .await
    .unwrap();

    let mut bases = Vec::new();
    for id_str in chapter_ids {
        let id = Uuid::parse_str(&id_str).unwrap();
        if let Ok(Some(ch)) = db_chapter::get_by_id(pool, id).await {
            bases.push(ch.chapter_base);
        }
    }
    bases.sort();
    bases
}

// ---------------------------------------------------------------------------
// The scenario test
// ---------------------------------------------------------------------------

#[tokio::test]
async fn canonical_chapter_download_queueing_three_scans() {
    let pool = test_db().await;
    let manga = setup_manga(&pool).await;

    // Register "TrustedGroup" so ProviderA's chapters get tier 2
    db_provider::add_trusted_group(&pool, "TrustedGroup")
        .await
        .unwrap();

    // -----------------------------------------------------------------------
    // SCAN 1 — initial full scan, both providers first-time
    // ProviderA: ch1..5 (TrustedGroup), ProviderB: ch1..3 (RandomGroup)
    // Expected: chapters inserted, NO download tasks (first-time sync)
    // -----------------------------------------------------------------------

    let a_scan1 = vec![
        ch_group("1", "TrustedGroup"),
        ch_group("2", "TrustedGroup"),
        ch_group("3", "TrustedGroup"),
        ch_group("4", "TrustedGroup"),
        ch_group("5", "TrustedGroup"),
    ];
    let b_scan1 = vec![
        ch_group("1", "RandomGroup"),
        ch_group("2", "RandomGroup"),
        ch_group("3", "RandomGroup"),
    ];

    let registry = make_registry(a_scan1, b_scan1);
    let ctx = test_ctx(&registry);
    let result = merge::scan_manga(&pool, &registry, &ctx, &manga, Uuid::new_v4())
        .await
        .expect("scan 1 failed");

    assert_eq!(result.providers_found, 2, "scan 1: both providers should be found");

    // ch1, ch2, ch3 from both providers = 6 rows; ch4, ch5 from A only = 2 more = 8 total
    let all_chapters = db_chapter::get_all_for_manga(&pool, manga.id).await.unwrap();
    assert_eq!(all_chapters.len(), 8, "scan 1: 8 chapter rows expected");

    let tasks_after_scan1 = count_pending_downloads(&pool, manga.id).await;
    assert_eq!(
        tasks_after_scan1, 0,
        "scan 1: first-time sync must not queue any downloads"
    );

    // Canonical should be all 5 chapter numbers, each won by ProviderA (TrustedGroup tier 2)
    let canonical = db_chapter::get_canonical_for_manga(&pool, manga.id).await.unwrap();
    assert_eq!(canonical.len(), 5, "scan 1: 5 canonical chapters");
    assert!(
        canonical.iter().all(|c| c.scanlator_group.as_deref() == Some("TrustedGroup")),
        "scan 1: TrustedGroup should win canonical for all chapters"
    );

    // -----------------------------------------------------------------------
    // SCAN 2 — incremental check, both previously synced
    // ProviderA adds ch6 (new canonical).
    // ProviderB adds ch4 + ch5 (new to B, but ProviderA's are already canonical).
    // Expected: 1 download task for ch6 (canonical from ProviderA)
    //           ProviderB's ch4/ch5 = new but NOT canonical → no tasks for them
    // -----------------------------------------------------------------------

    let a_scan2 = vec![
        ch_group("1", "TrustedGroup"),
        ch_group("2", "TrustedGroup"),
        ch_group("3", "TrustedGroup"),
        ch_group("4", "TrustedGroup"),
        ch_group("5", "TrustedGroup"),
        ch_group("6", "TrustedGroup"),  // ← NEW
    ];
    let b_scan2 = vec![
        ch_group("1", "RandomGroup"),
        ch_group("2", "RandomGroup"),
        ch_group("3", "RandomGroup"),
        ch_group("4", "RandomGroup"),   // ← new to B, but A already owns canonical ch4
        ch_group("5", "RandomGroup"),   // ← new to B, but A already owns canonical ch5
    ];

    let registry2 = make_registry(a_scan2.clone(), b_scan2);
    let ctx2 = test_ctx(&registry2);
    merge::check_new_chapters(&pool, &registry2, &ctx2, &manga, Uuid::new_v4())
        .await
        .expect("scan 2 failed");

    let tasks_after_scan2 = count_pending_downloads(&pool, manga.id).await;
    assert_eq!(
        tasks_after_scan2, 1,
        "scan 2: exactly 1 download task expected (canonical ch6 from ProviderA)"
    );

    let queued_bases = pending_download_chapter_bases(&pool, manga.id).await;
    assert_eq!(queued_bases, vec![6], "scan 2: the queued chapter should be ch6");

    // Verify the queued task points at ProviderA's ch6, not ProviderB's
    let canonical2 = db_chapter::get_canonical_for_manga(&pool, manga.id).await.unwrap();
    let canonical_ch6 = canonical2.iter().find(|c| c.chapter_base == 6).unwrap();
    assert_eq!(
        canonical_ch6.scanlator_group.as_deref(),
        Some("TrustedGroup"),
        "scan 2: canonical ch6 must belong to TrustedGroup, not RandomGroup"
    );

    // ProviderB's ch4 and ch5 should exist in DB but NOT be canonical
    let all_ch4 = db_chapter::get_all_for_chapter(&pool, manga.id, 4, 0).await.unwrap();
    assert_eq!(all_ch4.len(), 2, "scan 2: ch4 should have 2 rows (both providers)");
    let canonical_ch4 = canonical2.iter().find(|c| c.chapter_base == 4).unwrap();
    assert_eq!(
        canonical_ch4.scanlator_group.as_deref(),
        Some("TrustedGroup"),
        "scan 2: ProviderA should remain canonical for ch4"
    );

    // -----------------------------------------------------------------------
    // SCAN 3 — incremental check
    // ProviderA: no new chapters.
    // ProviderB adds ch6 (new to B — but ProviderA's ch6 is already canonical).
    // Expected: 0 new download tasks (ProviderB's ch6 is new but not canonical)
    // -----------------------------------------------------------------------

    let a_scan3 = a_scan2.clone(); // No change from ProviderA

    let b_scan3 = vec![
        ch_group("1", "RandomGroup"),
        ch_group("2", "RandomGroup"),
        ch_group("3", "RandomGroup"),
        ch_group("4", "RandomGroup"),
        ch_group("5", "RandomGroup"),
        ch_group("6", "RandomGroup"),  // ← new to B, but NOT canonical (A's ch6 is)
    ];

    let registry3 = make_registry(a_scan3, b_scan3);
    let ctx3 = test_ctx(&registry3);
    merge::check_new_chapters(&pool, &registry3, &ctx3, &manga, Uuid::new_v4())
        .await
        .expect("scan 3 failed");

    let tasks_after_scan3 = count_pending_downloads(&pool, manga.id).await;
    assert_eq!(
        tasks_after_scan3, 1,
        "scan 3: task count must stay at 1 — ProviderB's ch6 is not canonical, must not be queued"
    );

    // ProviderB's ch6 should exist in DB but still NOT be canonical
    let all_ch6 = db_chapter::get_all_for_chapter(&pool, manga.id, 6, 0).await.unwrap();
    assert_eq!(all_ch6.len(), 2, "scan 3: ch6 now has 2 rows (A + B)");

    let canonical3 = db_chapter::get_canonical_for_manga(&pool, manga.id).await.unwrap();
    let canonical_ch6_final = canonical3.iter().find(|c| c.chapter_base == 6).unwrap();
    assert_eq!(
        canonical_ch6_final.scanlator_group.as_deref(),
        Some("TrustedGroup"),
        "scan 3: TrustedGroup must remain canonical for ch6"
    );
}
