// Library crate root — exposes modules needed by test binaries in src/bin/.
// The main binary (src/main.rs) uses its own private module tree.
pub mod manga;
pub mod scraper;
pub mod db;
pub mod library;
pub mod api;
pub mod http;
pub mod scheduler;