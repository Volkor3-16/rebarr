// Library crate root — exposes modules needed by test binaries in src/bin/.
// The main binary (src/main.rs) uses its own private module tree.
pub mod api;
pub mod db;
pub mod http;
pub mod importer;
pub mod library;
pub mod manga;
pub mod scheduler;
pub mod scraper;
