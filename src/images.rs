//! Image pipeline: fetch, disk-cache, decode, upload.
//!
//! Covers and reader pages both go through here. Network and decode work happen
//! off the UI thread; the finished [`egui::ColorImage`] is handed back through
//! the app event channel and turned into a texture during the next frame.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use anyhow::{Context, Result};
use sha2::{Digest, Sha256};

use crate::net::HttpClient;
use crate::source::local;

/// Which pool an image belongs to; the two have very different lifetimes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageKind {
    Cover,
    Page,
}

/// An uploaded image: one texture for ordinary pages and covers, several
/// stacked ones for a tall webtoon strip.
pub struct PageTexture {
    /// Total size of the assembled image.
    pub size: egui::Vec2,
    pub slices: Vec<egui::TextureHandle>,
}

impl PageTexture {
    pub fn size_vec2(&self) -> egui::Vec2 {
        self.size
    }

    /// Rough memory weight, used to bound the cache.
    pub fn pixels(&self) -> u64 {
        self.size.x as u64 * self.size.y as u64
    }
}

#[derive(Clone)]
pub enum ImageSlot {
    Loading,
    Ready(Arc<PageTexture>),
    Failed(String),
}

/// GPU-side texture cache with least-recently-used eviction.
pub struct TextureCache {
    slots: HashMap<String, ImageSlot>,
    /// Monotonic tick of the last access, used to pick eviction victims.
    touched: HashMap<String, u64>,
    clock: AtomicU64,
    capacity: usize,
    /// Total pixels the cache may hold. A count alone is not enough: one
    /// webtoon strip can be worth thirty ordinary pages.
    pixel_budget: u64,
}

impl TextureCache {
    pub fn new(capacity: usize, pixel_budget: u64) -> Self {
        Self {
            slots: HashMap::new(),
            touched: HashMap::new(),
            clock: AtomicU64::new(0),
            capacity,
            pixel_budget,
        }
    }

    fn used_pixels(&self) -> u64 {
        self.slots
            .values()
            .map(|slot| match slot {
                ImageSlot::Ready(texture) => texture.pixels(),
                _ => 0,
            })
            .sum()
    }

    pub fn get(&mut self, key: &str) -> Option<ImageSlot> {
        let slot = self.slots.get(key).cloned();
        if slot.is_some() {
            let tick = self.clock.fetch_add(1, Ordering::Relaxed);
            self.touched.insert(key.to_string(), tick);
        }
        slot
    }

    pub fn contains(&self, key: &str) -> bool {
        self.slots.contains_key(key)
    }

    pub fn mark_loading(&mut self, key: &str) {
        self.slots.insert(key.to_string(), ImageSlot::Loading);
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        self.touched.insert(key.to_string(), tick);
    }

    pub fn insert(&mut self, key: &str, slot: ImageSlot) {
        self.slots.insert(key.to_string(), slot);
        let tick = self.clock.fetch_add(1, Ordering::Relaxed);
        self.touched.insert(key.to_string(), tick);
        self.evict();
    }

    pub fn clear(&mut self) {
        self.slots.clear();
        self.touched.clear();
    }

    fn evict(&mut self) {
        while self.slots.len() > self.capacity || self.used_pixels() > self.pixel_budget {
            // Never evict something still in flight: its result would be dropped.
            let victim = self
                .touched
                .iter()
                .filter(|(key, _)| !matches!(self.slots.get(*key), Some(ImageSlot::Loading)))
                .min_by_key(|(_, tick)| **tick)
                .map(|(key, _)| key.clone());

            match victim {
                Some(key) => {
                    self.slots.remove(&key);
                    self.touched.remove(&key);
                }
                None => break,
            }
        }
    }
}

/// On-disk byte cache shared by covers and pages.
pub struct DiskCache {
    covers: PathBuf,
    pages: PathBuf,
}

impl DiskCache {
    pub fn new(covers: PathBuf, pages: PathBuf) -> Self {
        Self { covers, pages }
    }

    fn dir(&self, kind: ImageKind) -> &Path {
        match kind {
            ImageKind::Cover => &self.covers,
            ImageKind::Page => &self.pages,
        }
    }

    pub fn path_for(&self, kind: ImageKind, url: &str) -> PathBuf {
        let digest = Sha256::digest(url.as_bytes());
        let name: String = digest.iter().take(16).map(|b| format!("{b:02x}")).collect();
        self.dir(kind).join(name)
    }

    pub fn read(&self, kind: ImageKind, url: &str) -> Option<Vec<u8>> {
        std::fs::read(self.path_for(kind, url)).ok()
    }

    pub fn write(&self, kind: ImageKind, url: &str, bytes: &[u8]) {
        let path = self.path_for(kind, url);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(err) = std::fs::write(&path, bytes) {
            log::warn!("could not cache image at {}: {err}", path.display());
        }
    }

    /// Total size of both cache directories, for the settings screen.
    pub fn size_bytes(&self) -> u64 {
        dir_size(&self.covers) + dir_size(&self.pages)
    }

    pub fn clear(&self, kind: Option<ImageKind>) {
        let targets: Vec<&Path> = match kind {
            Some(kind) => vec![self.dir(kind)],
            None => vec![&self.covers, &self.pages],
        };
        for dir in targets {
            if let Ok(entries) = std::fs::read_dir(dir) {
                for entry in entries.flatten() {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
    }
}

fn dir_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => dir_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

/// Fetches image bytes, preferring the disk cache and falling back to the
/// network (or the local filesystem for local-source urls).
///
/// The first candidate that works wins, and the bytes are cached under the
/// first url so the key stays stable no matter which mirror served them.
pub async fn fetch_first(
    http: &HttpClient,
    cache: &DiskCache,
    kind: ImageKind,
    candidates: &[String],
    headers: &[(String, String)],
) -> Result<Vec<u8>> {
    let key = candidates
        .first()
        .ok_or_else(|| anyhow::anyhow!("no image url given"))?;

    if let Some(bytes) = cache.read(kind, key)
        && !bytes.is_empty()
    {
        return Ok(bytes);
    }

    let mut last: Option<anyhow::Error> = None;
    for url in candidates {
        let attempt = if local::is_local_url(url) {
            let url = url.to_string();
            tokio::task::spawn_blocking(move || local::read_local_page(&url))
                .await
                .context("local image task panicked")
                .and_then(|inner| inner)
        } else {
            http.get_bytes(url, headers).await
        };

        match attempt {
            Ok(bytes) if !bytes.is_empty() => {
                cache.write(kind, key, &bytes);
                return Ok(bytes);
            }
            Ok(_) => last = Some(anyhow::anyhow!("empty image response from {url}")),
            Err(err) => last = Some(err),
        }
    }

    Err(last.unwrap_or_else(|| anyhow::anyhow!("the image could not be fetched")))
}

/// A decoded page, cut into stackable slices.
///
/// Webtoon chapters arrive as a single strip — 900x15000 is typical — which is
/// far taller than any GPU will accept as one texture. Slicing keeps the full
/// horizontal resolution instead of shrinking the whole strip to fit.
pub struct DecodedImage {
    /// Total size in pixels, across all slices.
    pub size: [usize; 2],
    pub slices: Vec<egui::ColorImage>,
}

/// Tall enough to keep the slice count low, small enough for any GPU.
pub const MAX_SLICE_HEIGHT: u32 = 2048;
/// Beyond this the image is downscaled. It sits comfortably above any window
/// width, so pages keep their detail and zooming stays sharp.
pub const MAX_IMAGE_WIDTH: u32 = 2048;
/// A ceiling on total pixels, so one absurd file cannot exhaust memory.
const MAX_PIXELS: u64 = 40_000_000;

/// Decodes image bytes into egui's pixel format.
///
/// `max_width` bounds the *width* only. Bounding the larger edge instead would
/// destroy tall strips: a 900x15000 webtoon capped at 4096 comes out 246px
/// wide, which then has to be blown back up to fill the reader.
pub fn decode(bytes: &[u8], max_width: u32, crop_borders: bool) -> Result<DecodedImage> {
    let decoded = image::load_from_memory(bytes).context("unsupported or corrupt image")?;
    let mut rgba = decoded.to_rgba8();

    if crop_borders && let Some(cropped) = crop_uniform_borders(&rgba) {
        rgba = cropped;
    }

    let (width, height) = (rgba.width(), rgba.height());
    let (target_w, target_h) = fit_within(width, height, max_width, MAX_PIXELS);
    if (target_w, target_h) != (width, height) {
        // Lanczos keeps screentones and thin lines legible; this runs off the
        // UI thread, so the extra cost is not felt.
        rgba = image::imageops::resize(
            &rgba,
            target_w,
            target_h,
            image::imageops::FilterType::Lanczos3,
        );
    }

    let (width, height) = (rgba.width(), rgba.height());
    let mut slices = Vec::new();
    let mut y = 0;
    while y < height {
        let slice_height = MAX_SLICE_HEIGHT.min(height - y);
        let view = image::imageops::crop_imm(&rgba, 0, y, width, slice_height).to_image();
        slices.push(egui::ColorImage::from_rgba_unmultiplied(
            [width as usize, slice_height as usize],
            view.as_raw(),
        ));
        y += slice_height;
    }

    Ok(DecodedImage {
        size: [width as usize, height as usize],
        slices,
    })
}

/// Size an image should be decoded at: never wider than `max_width`, never more
/// than `max_pixels` in total, aspect ratio preserved.
///
/// Height is deliberately unbounded on its own — that is what lets a webtoon
/// strip keep its full width.
fn fit_within(width: u32, height: u32, max_width: u32, max_pixels: u64) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width.max(1), height.max(1));
    }

    let mut scale = 1.0f64;
    if max_width > 0 && width > max_width {
        scale = max_width as f64 / width as f64;
    }

    let pixels = (width as f64 * scale) * (height as f64 * scale);
    if pixels > max_pixels as f64 {
        scale *= (max_pixels as f64 / pixels).sqrt();
    }

    let mut target_w = ((width as f64 * scale).floor() as u32).max(1);
    let mut target_h = ((height as f64 * scale).floor() as u32).max(1);
    // Rounding can leave the result a hair over budget; nudge it back under.
    while target_w as u64 * target_h as u64 > max_pixels && target_w > 1 && target_h > 1 {
        target_w = (target_w as f64 * 0.995).floor().max(1.0) as u32;
        target_h = (target_h as f64 * 0.995).floor().max(1.0) as u32;
    }
    (target_w, target_h)
}

/// Trims uniform margins, the equivalent of Mihon's "crop borders" option.
fn crop_uniform_borders(image: &image::RgbaImage) -> Option<image::RgbaImage> {
    const TOLERANCE: i32 = 12;
    let (width, height) = (image.width(), image.height());
    if width < 8 || height < 8 {
        return None;
    }

    let corner = *image.get_pixel(0, 0);
    let matches = |p: &image::Rgba<u8>| {
        (0..3).all(|i| (p.0[i] as i32 - corner.0[i] as i32).abs() <= TOLERANCE)
    };

    let row_uniform = |y: u32| (0..width).all(|x| matches(image.get_pixel(x, y)));
    let col_uniform = |x: u32| (0..height).all(|y| matches(image.get_pixel(x, y)));

    let mut top = 0;
    while top + 1 < height && row_uniform(top) {
        top += 1;
    }
    let mut bottom = height - 1;
    while bottom > top + 1 && row_uniform(bottom) {
        bottom -= 1;
    }
    let mut left = 0;
    while left + 1 < width && col_uniform(left) {
        left += 1;
    }
    let mut right = width - 1;
    while right > left + 1 && col_uniform(right) {
        right -= 1;
    }

    // Nothing to do, or the whole image is one flat colour.
    if top == 0 && left == 0 && bottom == height - 1 && right == width - 1 {
        return None;
    }
    let (new_w, new_h) = (right - left + 1, bottom - top + 1);
    if new_w < 8 || new_h < 8 {
        return None;
    }
    Some(image::imageops::crop_imm(image, left, top, new_w, new_h).to_image())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn solid(width: u32, height: u32, colour: [u8; 4]) -> image::RgbaImage {
        image::RgbaImage::from_pixel(width, height, image::Rgba(colour))
    }

    fn encode(width: u32, height: u32) -> Vec<u8> {
        let mut buffer = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(solid(width, height, [10, 20, 30, 255]))
            .write_to(&mut buffer, image::ImageFormat::Png)
            .unwrap();
        buffer.into_inner()
    }

    #[test]
    fn decoding_produces_matching_dimensions() {
        let decoded = decode(&encode(40, 20), 0, false).unwrap();
        assert_eq!(decoded.size, [40, 20]);
        assert_eq!(decoded.slices.len(), 1);
    }

    #[test]
    fn overly_wide_images_are_downscaled() {
        let decoded = decode(&encode(400, 100), 200, false).unwrap();
        assert_eq!(decoded.size, [200, 50]);
    }

    /// Regression: a webtoon strip used to be scaled down until its *height*
    /// fit, which crushed a 900x15000 page to 246px wide and made the reader
    /// blow it back up. Height must not constrain the width.
    #[test]
    fn tall_strips_keep_their_width_and_are_sliced() {
        let decoded = decode(&encode(900, 9000), MAX_IMAGE_WIDTH, false).unwrap();

        assert_eq!(decoded.size[0], 900, "the width must survive intact");
        assert_eq!(decoded.size[1], 9000);

        // Sliced into GPU-sized pieces that add back up to the whole strip.
        assert_eq!(
            decoded.slices.len(),
            (9000_f32 / MAX_SLICE_HEIGHT as f32).ceil() as usize
        );
        let total: usize = decoded.slices.iter().map(|s| s.size[1]).sum();
        assert_eq!(total, 9000);
        assert!(decoded.slices.iter().all(|s| s.size[0] == 900));
        assert!(
            decoded
                .slices
                .iter()
                .all(|s| s.size[1] <= MAX_SLICE_HEIGHT as usize),
            "no slice may exceed the texture limit"
        );
    }

    #[test]
    fn sizing_keeps_the_width_and_bounds_the_total() {
        // A typical webtoon strip is left completely alone.
        assert_eq!(
            fit_within(900, 15000, MAX_IMAGE_WIDTH, MAX_PIXELS),
            (900, 15000)
        );
        // Ordinary pages too.
        assert_eq!(
            fit_within(1700, 2400, MAX_IMAGE_WIDTH, MAX_PIXELS),
            (1700, 2400)
        );

        // Too wide: scaled down on width, aspect preserved.
        let (w, h) = fit_within(4000, 2000, 2048, MAX_PIXELS);
        assert_eq!(w, 2048);
        assert!((h as i64 - 1024).abs() <= 1, "aspect drifted: {w}x{h}");

        // Absurdly large: bounded by the pixel budget, and strictly so.
        let (w, h) = fit_within(4000, 60_000, MAX_IMAGE_WIDTH, MAX_PIXELS);
        assert!(
            w as u64 * h as u64 <= MAX_PIXELS,
            "{w}x{h} exceeds the budget"
        );
        assert!(w <= MAX_IMAGE_WIDTH);

        // Degenerate input must not divide by zero.
        assert_eq!(fit_within(0, 0, 2048, MAX_PIXELS), (1, 1));
    }

    #[test]
    fn uniform_borders_are_trimmed() {
        // A white canvas with a black block inset by 10px on every side.
        let mut img = solid(60, 60, [255, 255, 255, 255]);
        for y in 10..50 {
            for x in 10..50 {
                img.put_pixel(x, y, image::Rgba([0, 0, 0, 255]));
            }
        }
        let cropped = crop_uniform_borders(&img).expect("borders should be detected");
        assert_eq!((cropped.width(), cropped.height()), (40, 40));
    }

    #[test]
    fn flat_images_are_left_alone() {
        let img = solid(30, 30, [7, 7, 7, 255]);
        assert!(crop_uniform_borders(&img).is_none());
    }

    #[test]
    fn cache_evicts_least_recently_used() {
        let mut cache = TextureCache::new(2, u64::MAX);
        cache.insert("a", ImageSlot::Failed("x".into()));
        cache.insert("b", ImageSlot::Failed("x".into()));
        let _ = cache.get("a"); // "a" becomes the most recent
        cache.insert("c", ImageSlot::Failed("x".into()));

        assert!(cache.contains("a"));
        assert!(cache.contains("c"));
        assert!(!cache.contains("b"));
    }

    #[test]
    fn in_flight_entries_survive_eviction() {
        let mut cache = TextureCache::new(1, u64::MAX);
        cache.mark_loading("pending");
        cache.insert("done", ImageSlot::Failed("x".into()));
        assert!(cache.contains("pending"));
    }
}
