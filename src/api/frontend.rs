use rocket::{get, response::content::RawHtml, routes};
use std::fs;

// This file handles serving the frontend.
// The frontend is served from the web/ directory as static files.
// This module provides route handlers that serve the index.html from the filesystem.

fn serve_index() -> RawHtml<String> {
    let html = fs::read_to_string("web/index.html")
        .expect("Failed to read web/index.html - make sure it exists");
    RawHtml(html)
}

#[get("/")]
pub fn index() -> RawHtml<String> {
    serve_index()
}

#[get("/library")]
pub fn library_page() -> RawHtml<String> {
    serve_index()
}

#[get("/series/<_id>")]
pub fn series_page(_id: &str) -> RawHtml<String> {
    serve_index()
}

#[get("/search")]
pub fn search_page() -> RawHtml<String> {
    serve_index()
}

#[get("/settings")]
pub fn settings_page() -> RawHtml<String> {
    serve_index()
}

#[get("/queue")]
pub fn queue_page() -> RawHtml<String> {
    serve_index()
}

#[get("/logs")]
pub fn logs_page() -> RawHtml<String> {
    serve_index()
}

// ---------------------------------------------------------------------------
// Route list
// ---------------------------------------------------------------------------

pub fn routes() -> Vec<rocket::Route> {
    routes![
        index,
        library_page,
        series_page,
        search_page,
        settings_page,
        queue_page,
        logs_page
    ]
}
