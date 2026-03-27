use std::path::{Path, PathBuf};

use crate::manga::manga::{Chapter, Manga};

pub fn sanitize_chapter_filename(name: &str) -> String {
    name.chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') {
                '_'
            } else {
                c
            }
        })
        .collect()
}

pub fn chapter_base_name(chapter: &Chapter) -> String {
    let mut name = format!("Chapter {}", chapter.number_sort());
    if let Some(title) = chapter.title.as_deref().filter(|s| !s.is_empty()) {
        name.push_str(&format!(" - {title}"));
    }
    if let Some(group) = chapter.scanlator_group.as_deref().filter(|s| !s.is_empty()) {
        name.push_str(&format!(" [{group}]"));
    }
    sanitize_chapter_filename(&name)
}

pub fn chapter_cbz_path(series_dir: &Path, chapter: &Chapter) -> PathBuf {
    series_dir.join(format!("{}.cbz", chapter_base_name(chapter)))
}

pub fn series_dir(lib_root: &Path, manga: &Manga) -> PathBuf {
    lib_root.join(&manga.relative_path)
}
