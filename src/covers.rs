use std::path::Path;

use log::warn;
use uuid::Uuid;

/// Downloads the cover image at `url` and saves it to `./thumbnails/<manga_id>.<ext>`.
/// Returns the local URL path `/thumbnails/<manga_id>.<ext>`, or `None` on failure.
pub async fn download_cover(client: &reqwest::Client, url: &str, manga_id: Uuid) -> Option<String> {
    let ext = Path::new(url)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.split('?').next().unwrap_or(e)) // strip query params
        .filter(|e| matches!(*e, "jpg" | "jpeg" | "png" | "webp" | "avif"))
        .unwrap_or("jpg");

    let filename = format!("{manga_id}.{ext}");
    let dest = format!("./thumbnails/{filename}");

    if let Err(e) = tokio::fs::create_dir_all("./thumbnails").await {
        warn!("Failed to create thumbnails dir: {e}");
        return None;
    }

    let response = match client.get(url).send().await {
        Ok(r) => r,
        Err(e) => {
            warn!("Cover download request failed for {url}: {e}");
            return None;
        }
    };

    if !response.status().is_success() {
        warn!("Cover download got HTTP {} for {url}", response.status());
        return None;
    }

    let bytes = match response.bytes().await {
        Ok(b) => b,
        Err(e) => {
            warn!("Failed to read cover response body for {url}: {e}");
            return None;
        }
    };

    if let Err(e) = tokio::fs::write(&dest, &bytes).await {
        warn!("Failed to write cover to {dest}: {e}");
        return None;
    }

    Some(format!("/thumbnails/{filename}"))
}
