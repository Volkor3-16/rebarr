use std::path::Path;

use log::warn;
use uuid::Uuid;

/// Downloads the cover image at `url` and saves it as `cover.<ext>` inside
/// `series_dir` (the manga's folder on disk, e.g. `{lib_root}/{relative_path}`).
/// Returns the API URL `/api/manga/<manga_id>/cover`, or `None` on failure.
pub async fn download_cover(
    client: &reqwest::Client,
    url: &str,
    manga_id: Uuid,
    series_dir: &Path,
) -> Option<String> {
    let ext = Path::new(url)
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.split('?').next().unwrap_or(e)) // strip query params
        .filter(|e| matches!(*e, "jpg" | "jpeg" | "png" | "webp" | "avif"))
        .unwrap_or("jpg");

    if let Err(e) = tokio::fs::create_dir_all(series_dir).await {
        warn!("Failed to create series dir {}: {e}", series_dir.display());
        return None;
    }

    let dest = series_dir.join(format!("cover.{ext}"));

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
        warn!("Failed to write cover to {}: {e}", dest.display());
        return None;
    }

    Some(format!("/api/manga/{manga_id}/cover"))
}
