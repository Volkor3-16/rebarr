// This module handles all the API endpoints that rocket uses.

// Error types and helpers
pub(crate) mod errors;

// API Endpoints
pub(crate) mod chapters;
pub(crate) mod events;
pub(crate) mod import;
pub(crate) mod libraries;
pub(crate) mod manga;
pub(crate) mod provider_scores;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod tasks;
pub(crate) mod trusted_groups;
pub(crate) mod webhooks;

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
pub use events::routes as event_routes;
pub use system::routes as system_routes;
pub use tasks::routes as task_routes;
pub use trusted_groups::routes as trusted_group_routes;
pub use webhooks::routes as webhook_routes;

/// Routes that can't be included in the OpenAPI spec (e.g. raw file responses)
pub fn extra_routes() -> Vec<rocket::Route> {
    rocket::routes![manga::serve_cover]
}

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
    routes.extend(webhook_routes());
    routes.extend(event_routes());
    routes
}

/// Generate OpenAPI routes (including the /openapi.json spec route)
/// This function must be defined here because the openapi_get_routes! macro
/// needs access to the route functions which are defined in private modules
pub fn openapi_routes() -> Vec<rocket::Route> {
    let settings = rocket_okapi::settings::OpenApiSettings::default();
    rocket_okapi::openapi_get_routes![
        settings:
            // Chapters
            chapters::list_chapters,
            chapters::download_chapter_api,
            chapters::delete_chapter_api,
            chapters::mark_chapter_downloaded,
            chapters::reset_chapter_api,
            chapters::toggle_extra_api,
            chapters::optimise_chapter_api,
            chapters::set_canonical_api,
            // Import
            import::scan_api,
            import::execute_api,
            import::series_scan_api,
            import::series_execute_api,
            // Libraries
            libraries::list_libraries,
            libraries::create_library,
            libraries::get_library,
            libraries::update_library,
            libraries::delete_library,
            libraries::list_library_manga,
            // Manga
            manga::search_manga,
            manga::add_manga,
            manga::add_manga_manual,
            manga::get_manga,
            manga::delete_manga,
            manga::patch_manga,
            manga::list_providers,
            manga::scan_manga_api,
            manga::check_new_chapters_api,
            manga::list_manga_providers,
            manga::refresh_manga_api,
            manga::scan_disk_api,
            manga::update_synonyms,
            manga::provider_candidates,
            manga::set_provider_url,
            manga::upload_cover_url,
            manga::upload_cover_file,
            // Provider Scores
            provider_scores::get_global_score,
            provider_scores::set_global_score,
            provider_scores::delete_global_score,
            provider_scores::get_series_score,
            provider_scores::set_series_score,
            provider_scores::delete_series_score,
            // Settings
            settings::get_settings,
            settings::update_settings,
            // System
            system::system_info,
            system::desktop_health,
            system::version_info,
            system::changelog,
            // Tasks
            tasks::list_tasks,
            tasks::list_tasks_grouped,
            tasks::cancel_task,
            // Trusted Groups
            trusted_groups::list_trusted_groups,
            trusted_groups::add_trusted_group,
            trusted_groups::remove_trusted_group,
            // Webhooks
            webhooks::list_webhooks,
            webhooks::create_webhook,
            webhooks::update_webhook,
            webhooks::delete_webhook
    ]
}
