// Tiers:
// 1 = Official Publisher releases
// 2 = Trusted scanlator groups (from the trusted-groups list)
// 3 = Scanlator groups not on the list
// 4 = No scanlator group listed

use log::warn;

use crate::manga::manga::Chapter;

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
