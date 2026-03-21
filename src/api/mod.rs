// This module handles all the API endpoints that rocket uses.

// Error types and helpers
pub(crate) mod errors;

// API Endpoints
pub(crate) mod chapters;
pub(crate) mod import;
pub(crate) mod libraries;
pub(crate) mod manga;
pub(crate) mod provider_scores;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod tasks;
pub(crate) mod trusted_groups;

// Frontend HTML
pub(crate) mod frontend;

// re-export the route functions
pub use chapters::routes as chapter_routes;
pub use frontend::routes as frontend_routes;
pub use import::routes as import_routes;
pub use libraries::routes as library_routes;
pub use manga::routes as manga_routes;
pub use provider_scores::routes as provider_score_routes;
pub use settings::routes as settings_routes;
pub use system::routes as system_routes;
pub use tasks::routes as task_routes;
pub use trusted_groups::routes as trusted_group_routes;

/// All API routes combined
pub fn api_routes() -> Vec<rocket::Route> {
    let mut routes = Vec::new();
    routes.extend(library_routes());
    routes.extend(manga_routes());
    routes.extend(chapter_routes());
    routes.extend(import_routes());
    routes.extend(task_routes());
    routes.extend(settings_routes());
    routes.extend(trusted_group_routes());
    routes.extend(provider_score_routes());
    routes.extend(system_routes());
    routes
}
