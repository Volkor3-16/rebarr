// This module handles all the API endpoints that rocket uses.

// Error types and helpers
pub(crate) mod errors;

// API Endpoints
pub(crate) mod chapters;
pub(crate) mod events;
pub(crate) mod import;
pub(crate) mod libraries;
pub(crate) mod manga;
pub(crate) mod metadata_rules;
pub(crate) mod provider_settings;
pub(crate) mod quality_rules;
pub(crate) mod settings;
pub(crate) mod system;
pub(crate) mod tasks;
pub(crate) mod webhooks;

// Frontend HTML
pub(crate) mod frontend;

// re-export the route functions
pub use chapters::routes as chapter_routes;
pub use events::routes as event_routes;
pub use frontend::routes as frontend_routes;
pub use import::routes as import_routes;
pub use libraries::routes as library_routes;
pub use manga::routes as manga_routes;
pub use metadata_rules::routes as metadata_rule_routes;
pub use provider_settings::routes as provider_setting_routes;
pub use quality_rules::routes as quality_rule_routes;
pub use settings::routes as settings_routes;
pub use system::routes as system_routes;
pub use tasks::routes as task_routes;
pub use webhooks::routes as webhook_routes;

/// Routes that can't be included in the OpenAPI spec (e.g. raw file responses)
pub fn extra_routes() -> Vec<rocket::Route> {
    rocket::routes![manga::serve_cover, events::events]
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
    routes.extend(provider_setting_routes());
    routes.extend(metadata_rule_routes());
    routes.extend(quality_rule_routes());
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
            chapters::clear_canonical_override_api,
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
            libraries::list_library_suggestions,
            libraries::refresh_library_suggestions,
            libraries::set_suggestion_visibility,
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
            // Provider Settings
            provider_settings::get_global_settings,
            provider_settings::set_global_settings,
            provider_settings::get_series_settings,
            provider_settings::set_series_settings,
            provider_settings::delete_series_settings,
            // Settings
            settings::get_settings,
            settings::update_settings,
            // System
            system::system_info,
            system::desktop_health,
            system::version_info,
            system::changelog,
            system::purge_orphan_cbz_api,
            // Tasks
            tasks::list_tasks,
            tasks::list_queue_tasks,
            tasks::list_tasks_grouped,
            tasks::cancel_task,
            // Metadata Rules
            metadata_rules::list_rules,
            metadata_rules::create_rule,
            metadata_rules::update_rule,
            metadata_rules::delete_rule,
            // Quality Rules
            quality_rules::list_rules,
            quality_rules::list_fields,
            quality_rules::create_rule,
            quality_rules::update_rule,
            quality_rules::delete_rule,
            quality_rules::reorder_rules,
            // Webhooks
            webhooks::list_webhooks,
            webhooks::create_webhook,
            webhooks::update_webhook,
            webhooks::delete_webhook
    ]
}
