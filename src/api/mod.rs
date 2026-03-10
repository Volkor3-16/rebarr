// This module handles all the API endpoints that rocket uses.

// API Endpoints
pub(crate) mod api;

// Frontend HTML
pub(crate) mod frontend;

// re-export the route functions
pub use api::routes as api_routes;
pub use frontend::routes as frontend_routes;