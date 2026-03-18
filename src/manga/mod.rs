// This module handles all the core manga stuff

/// Core Manga Stuffs
pub(crate) mod manga;
/// Creation of ComicInfo.xml handler
pub(crate) mod comicinfo;
/// Download handler for cover images
pub(crate) mod covers;
/// Merging provider-supplied chapter lists into a merged list for viewing
pub(crate) mod merge;
/// Scoring provider chapters
pub(crate) mod scoring;