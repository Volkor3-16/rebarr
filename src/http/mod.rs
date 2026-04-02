// This module manages:
// - All http requests
//  - Metadata requests
//  - Any other web request thats not a provider scrape.

pub(crate) mod anilist;
pub(crate) mod metadata;
pub(crate) mod webhook;

pub use anilist::ALClient;
pub use metadata::AniListMetadata;
pub use webhook::WebhookDispatcher;
