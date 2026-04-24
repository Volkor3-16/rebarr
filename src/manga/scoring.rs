use tracing::warn;

use crate::db::quality_rules::{QualityRule, RuleCondition};
use crate::manga::core::Chapter;

// ---------------------------------------------------------------------------
// Quality-rules scoring engine
// ---------------------------------------------------------------------------

/// Evaluate a single condition against a chapter.
fn evaluate_condition(cond: &RuleCondition, chapter: &Chapter) -> bool {
    let matched = match cond.field.as_str() {
        "scanlator_group" => {
            let val = chapter.scanlator_group.as_deref().unwrap_or("");
            match_string_op(cond, val)
        }
        "provider_name" => {
            let val = chapter.provider_name.as_deref().unwrap_or("");
            match_string_op(cond, val)
        }
        "language" => {
            let val = &chapter.language;
            match_string_op(cond, val)
        }
        "title" => {
            let present = chapter
                .title
                .as_deref()
                .map(|t| !t.is_empty())
                .unwrap_or(false);
            match_presence_op(cond, present)
        }
        "released_at" => {
            let present = chapter.released_at.is_some();
            match_presence_op(cond, present)
        }
        "chapter_variant" => {
            let val = chapter.chapter_variant.to_string();
            match_string_op(cond, &val)
        }
        "is_full_chapter" => {
            let present = chapter.chapter_variant == 0;
            match_presence_op(cond, present)
        }
        "is_split_chapter" => {
            let present = chapter.chapter_variant >= 1 && chapter.chapter_variant <= 4;
            match_presence_op(cond, present)
        }
        _ => {
            // Unknown field: condition does not match (forward-compat: new fields added later).
            false
        }
    };
    if cond.negate { !matched } else { matched }
}

fn match_string_op(cond: &RuleCondition, val: &str) -> bool {
    match cond.op.as_str() {
        "eq" => val.eq_ignore_ascii_case(cond.value.as_deref().unwrap_or("")),
        "contains" => val
            .to_lowercase()
            .contains(&cond.value.as_deref().unwrap_or("").to_lowercase()),
        "regex" => {
            if let Some(pattern) = &cond.value {
                regex::Regex::new(pattern)
                    .map(|re| re.is_match(val))
                    .unwrap_or(false)
            } else {
                false
            }
        }
        "present" => !val.is_empty(),
        "not_present" => val.is_empty(),
        _ => false,
    }
}

fn match_presence_op(cond: &RuleCondition, is_present: bool) -> bool {
    match cond.op.as_str() {
        "present" => is_present,
        "not_present" | "eq" => !is_present,
        _ => false,
    }
}

/// Compute a total quality score for a chapter by evaluating all rules.
/// Rules whose conditions ALL match contribute their score.
pub fn compute_score(chapter: &Chapter, rules: &[QualityRule]) -> i32 {
    compute_matched_rules(chapter, rules)
        .iter()
        .map(|(_, score)| *score)
        .sum()
}

/// Returns list of (rule name, score) for all rules that matched this chapter.
pub fn compute_matched_rules(chapter: &Chapter, rules: &[QualityRule]) -> Vec<(String, i32)> {
    rules
        .iter()
        .filter(|rule| {
            rule.conditions
                .iter()
                .all(|cond| evaluate_condition(cond, chapter))
        })
        .map(|rule| (rule.name.clone(), rule.score))
        .collect()
}

/// Returns entries sorted best-first using quality rules scoring.
/// Language filter applied first (falls back to all if no match).
pub fn rank_entries_scored(
    mut entries: Vec<Chapter>,
    language: Option<&str>,
    rules: &[QualityRule],
) -> Vec<Chapter> {
    if let Some(lang) = language {
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
    entries.sort_by_key(|b| std::cmp::Reverse(compute_score(b, rules)));
    entries
}

// ---------------------------------------------------------------------------
// Known condition fields (for the API / frontend rule builder)
// ---------------------------------------------------------------------------

pub struct FieldDef {
    pub field: &'static str,
    pub label: &'static str,
    pub ops: &'static [&'static str],
}

pub const KNOWN_FIELDS: &[FieldDef] = &[
    FieldDef {
        field: "scanlator_group",
        label: "Scanlator group",
        ops: &["eq", "contains", "regex", "present", "not_present"],
    },
    FieldDef {
        field: "provider_name",
        label: "Provider",
        ops: &["eq", "present", "not_present"],
    },
    FieldDef {
        field: "language",
        label: "Language",
        ops: &["eq", "present", "not_present"],
    },
    FieldDef {
        field: "title",
        label: "Has title",
        ops: &["present", "not_present"],
    },
    FieldDef {
        field: "released_at",
        label: "Has release date",
        ops: &["present", "not_present"],
    },
    FieldDef {
        field: "is_full_chapter",
        label: "Is full chapter",
        ops: &["present", "not_present"],
    },
    FieldDef {
        field: "is_split_chapter",
        label: "Is split chapter part",
        ops: &["present", "not_present"],
    },
    FieldDef {
        field: "chapter_variant",
        label: "Chapter variant number",
        ops: &["eq", "contains", "present", "not_present"],
    },
];

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
            tags: vec![],
        }
    }

    // --- rank_entries_scored ---

    #[test]
    fn rank_filters_by_language_exact() {
        let chapters = vec![
            make_chapter("EN", Some("GroupA")),
            make_chapter("FR", Some("GroupB")),
            make_chapter("EN", Some("official")),
        ];
        let ranked = rank_entries_scored(chapters, Some("EN"), &[]);
        assert!(ranked.iter().all(|c| c.language == "EN"));
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_falls_back_to_all_when_no_language_match() {
        let chapters = vec![
            make_chapter("FR", Some("GroupA")),
            make_chapter("DE", Some("GroupB")),
        ];
        let ranked = rank_entries_scored(chapters, Some("EN"), &[]);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_no_language_filter_returns_all() {
        let chapters = vec![
            make_chapter("EN", Some("GroupA")),
            make_chapter("FR", Some("GroupB")),
        ];
        let ranked = rank_entries_scored(chapters, None, &[]);
        assert_eq!(ranked.len(), 2);
    }

    #[test]
    fn rank_language_filter_case_insensitive() {
        let chapters = vec![
            make_chapter("en", Some("GroupA")),
            make_chapter("EN", Some("GroupB")),
            make_chapter("FR", Some("GroupC")),
        ];
        let ranked = rank_entries_scored(chapters, Some("EN"), &[]);
        assert_eq!(ranked.len(), 2);
    }
}
