// This module handles all the core manga stuff

/// Creation of ComicInfo.xml handler
pub mod comicinfo;
/// Core Manga Stuffs
pub mod core;
/// Download handler for cover images
pub mod covers;
pub mod files;
/// Merging provider-supplied chapter lists into a merged list for viewing
pub mod merge;
/// Metadata filtering rules
pub mod metadata_rules;
/// Scoring provider chapters
pub mod scoring;
