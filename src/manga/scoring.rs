// Tiers:
// 1 = Official Publisher releases
// 2 = Trusted scanlator groups (from the trusted-groups list)
// 3 = Scanlator groups not on the list
// 4 = No scanlator group listed

use tracing::warn;

use crate::manga::core::Chapter;

/// Criteria applied when selecting which provider entries to try for a download.
pub struct ChapterFilter {
    /// BCP 47 language code to prefer (e.g. "en"). `None` = accept any language.
    pub language: Option<String>,
}

/// Returns entries sorted best-first for download attempts, applying in order:
/// 1. Language filter (falls back to all if no entries match the language).
/// 2. Tier sort ascending (tier 1 = Official first, tier 4 = No group last).
pub fn rank_entries(
    mut entries: Vec<Chapter>,
    filter: &ChapterFilter,
    trusted_groups: &[String],
) -> Vec<Chapter> {
    if let Some(ref lang) = filter.language {
        let filtered: Vec<_> = entries
            .iter()
            .filter(|e| e.language.eq_ignore_ascii_case(lang))
            .cloned()
            .collect();
        if !filtered.is_empty() {
            entries = filtered;
        } else {
            warn!("[scoring] No entries match language '{lang}'; falling back to all languages.");
        }
    }

    entries.sort_by_key(|e| compute_tier(e.scanlator_group.as_deref(), trusted_groups));
    entries
}

/// Calculates the tier for a chapter based on the scanlator group name.
pub fn compute_tier(group: Option<&str>, trusted: &[String]) -> u8 {
    match group {
        None | Some("") => 4,
        Some(g) => {
            if g.trim().eq_ignore_ascii_case("official") {
                1
            } else if trusted.iter().any(|t| t.eq_ignore_ascii_case(g)) {
                2
            } else {
                3
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use super::*;
    use crate::manga::core::{Chapter, DownloadStatus};

    fn make_chapter(language: &str, group: Option<&str>) -> Chapter {
        Chapter {
            id: Uuid::new_v4(),
            manga_id: Uuid::new_v4(),
            chapter_base: 1,
            chapter_variant: 0,
            is_extra: false,
            title: None,
            language: language.to_owned(),
            scanlator_group: group.map(str::to_owned),
            provider_name: None,
            chapter_url: None,
            download_status: DownloadStatus::Missing,
            released_at: None,
            downloaded_at: None,
            scraped_at: None,
            file_size_bytes: None,
        }
    }

    fn trusted(groups: &[&str]) -> Vec<String> {
        groups.iter().map(|s| s.to_string()).collect()
    }

    // --- compute_tier ---

    #[test]
    fn tier_none_group_is_four() {
        assert_eq!(compute_tier(None, &[]), 4);
    }

    #[test]
    fn tier_empty_string_is_four() {
        assert_eq!(compute_tier(Some(""), &[]), 4);
    }

    #[test]
    fn tier_official_is_one() {
        assert_eq!(compute_tier(Some("Official"), &[]), 1);
        // Case-insensitive
        assert_eq!(compute_tier(Some("OFFICIAL"), &[]), 1);
        assert_eq!(compute_tier(Some("official"), &[]), 1);
    }

    #[test]
    fn tier_official_with_whitespace_is_one() {
        assert_eq!(compute_tier(Some("  official  "), &[]), 1);
    }

    #[test]
    fn tier_trusted_group_is_two() {
        let trusted = trusted(&["MangaStream", "CatScans"]);
        assert_eq!(compute_tier(Some("MangaStream"), &trusted), 2);
        // Case-insensitive
        assert_eq!(compute_tier(Some("MANGASTREAM"), &trusted), 2);
    }

    #[test]
    fn tier_unknown_group_is_three() {
        let trusted = trusted(&["MangaStream"]);
        assert_eq!(compute_tier(Some("SomeRandomGroup"), &trusted), 3);
    }

    #[test]
    fn tier_unknown_group_no_trusted_list_is_three() {
        assert_eq!(compute_tier(Some("AnyGroup"), &[]), 3);
    }

    // --- rank_entries ---

    #[test]
    fn rank_filters_by_language_exact() {
        let chapters = vec![
            make_chapter("EN", Some("GroupA")),
            make_chapter("FR", Some("GroupB")),
            make_chapter("EN", Some("official")),
        ];
        let filter = ChapterFilter { language: Some("EN".to_owned()) };
        let ranked = rank_entries(chapters, &filter, &[]);
        assert!(ranked.iter().all(|c| c.language == "EN"));
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_falls_back_to_all_when_no_language_match() {
        let chapters = vec![
            make_chapter("FR", Some("GroupA")),
            make_chapter("DE", Some("GroupB")),
        ];
        let filter = ChapterFilter { language: Some("EN".to_owned()) };
        let ranked = rank_entries(chapters, &filter, &[]);
        // Fallback: all languages returned
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_no_language_filter_returns_all() {
        let chapters = vec![
            make_chapter("EN", Some("GroupA")),
            make_chapter("FR", Some("GroupB")),
        ];
        let filter = ChapterFilter { language: None };
        let ranked = rank_entries(chapters, &filter, &[]);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_sorts_by_tier_ascending() {
        let trusted = trusted(&["TrustedGroup"]);
        let chapters = vec![
            make_chapter("EN", None),                        // tier 4
            make_chapter("EN", Some("UnknownGroup")),        // tier 3
            make_chapter("EN", Some("TrustedGroup")),        // tier 2
            make_chapter("EN", Some("official")),            // tier 1
        ];
        let filter = ChapterFilter { language: None };
        let ranked = rank_entries(chapters, &filter, &trusted);

        let tiers: Vec<u8> = ranked
            .iter()
            .map(|c| compute_tier(c.scanlator_group.as_deref(), &trusted))
            .collect();
        assert_eq!(tiers, vec![1, 2, 3, 4]);
    }

    #[test]
    fn rank_language_filter_case_insensitive() {
        let chapters = vec![
            make_chapter("en", Some("GroupA")),   // lowercase
            make_chapter("EN", Some("GroupB")),   // uppercase
            make_chapter("FR", Some("GroupC")),
        ];
        let filter = ChapterFilter { language: Some("EN".to_owned()) };
        let ranked = rank_entries(chapters, &filter, &[]);
        assert_eq!(ranked.len(), 2);
    }
}
