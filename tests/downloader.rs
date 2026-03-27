/// Tests for the download utility helpers that don't require a browser.
mod helpers;

use rebarr::scraper::downloader::image_ext;

// ---------------------------------------------------------------------------
// image_ext (public helper for magic-byte detection)
// ---------------------------------------------------------------------------

#[test]
fn image_ext_detects_jpeg() {
    let jpeg = [0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10];
    assert_eq!(image_ext(&jpeg), "jpg");
}

#[test]
fn image_ext_detects_png() {
    let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];
    assert_eq!(image_ext(&png), "png");
}

#[test]
fn image_ext_detects_gif() {
    let gif = b"GIF89a\x01\x00\x01\x00\x00\x00\x00";
    assert_eq!(image_ext(gif), "gif");
}

#[test]
fn image_ext_detects_webp() {
    let mut webp = b"RIFF\x00\x00\x00\x00WEBP".to_vec();
    webp.extend_from_slice(&[0u8; 4]);
    assert_eq!(image_ext(&webp), "webp");
}

#[test]
fn image_ext_falls_back_to_jpg() {
    assert_eq!(image_ext(&[0x00, 0x01, 0x02]), "jpg");
}
