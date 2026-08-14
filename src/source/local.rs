//! The local source: manga read straight off disk.
//!
//! Layout, matching Mihon's local source closely enough that an existing folder
//! tree works unchanged:
//!
//! ```text
//! local/
//!   Series Name/
//!     cover.jpg          (optional)
//!     details.json       (optional metadata)
//!     Chapter 1.cbz      (or .zip, or a folder of images)
//!     Chapter 2/
//!       01.jpg
//! ```

use std::io::Read;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use serde::Deserialize;

use super::{Filter, FilterList, MangasPage, Page, SChapter, SManga, Source};
use crate::model::{MangaStatus, UpdateStrategy};

/// Upstream reserves source id 0 for the local source.
pub const LOCAL_ID: i64 = 0;
pub const LOCAL_LANG: &str = "localsourcelang";

/// Marks a page that lives inside a CBZ/ZIP: `cbz://<archive>|<entry>`.
pub const CBZ_SCHEME: &str = "cbz://";
/// Marks a page that is a plain file on disk: `file://<path>`.
pub const FILE_SCHEME: &str = "file://";

const IMAGE_EXTENSIONS: &[&str] = &["jpg", "jpeg", "png", "webp", "gif", "bmp", "avif", "jxl"];
const ARCHIVE_EXTENSIONS: &[&str] = &["cbz", "zip"];

pub struct LocalSource {
    root: PathBuf,
    extra_roots: Vec<PathBuf>,
}

impl LocalSource {
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            extra_roots: Vec::new(),
        }
    }

    pub fn with_extra_roots(root: PathBuf, extra_roots: Vec<PathBuf>) -> Self {
        Self { root, extra_roots }
    }

    fn roots(&self) -> Vec<PathBuf> {
        let mut roots = vec![self.root.clone()];
        roots.extend(self.extra_roots.iter().cloned());
        roots
    }

    /// Resolves a stored url (the series folder name) back to a directory.
    fn resolve(&self, url: &str) -> Option<PathBuf> {
        let name = url.trim_start_matches('/');
        self.roots()
            .into_iter()
            .map(|root| root.join(name))
            .find(|path| path.is_dir())
    }

    fn scan(&self) -> Vec<SManga> {
        let mut entries = Vec::new();
        for root in self.roots() {
            let Ok(dir) = std::fs::read_dir(&root) else {
                continue;
            };
            for entry in dir.flatten() {
                let path = entry.path();
                if !path.is_dir() {
                    continue;
                }
                let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
                    continue;
                };
                entries.push(read_series(&path, name));
            }
        }
        entries.sort_by_key(|a| a.title.to_lowercase());
        entries
    }
}

#[async_trait]
impl Source for LocalSource {
    fn id(&self) -> i64 {
        LOCAL_ID
    }

    fn name(&self) -> &str {
        "Local source"
    }

    fn lang(&self) -> &str {
        LOCAL_LANG
    }

    fn base_url(&self) -> &str {
        ""
    }

    fn supports_latest(&self) -> bool {
        false
    }

    fn filters(&self) -> FilterList {
        vec![Filter::sort(
            "Sort",
            &[("Title", "title"), ("Date added", "date")],
            0,
            true,
        )]
    }

    async fn popular(&self, _page: u32) -> Result<MangasPage> {
        let all = self.scan();
        Ok(MangasPage {
            mangas: all,
            has_next_page: false,
        })
    }

    async fn latest(&self, page: u32) -> Result<MangasPage> {
        self.popular(page).await
    }

    async fn search(&self, _page: u32, query: &str, _filters: &FilterList) -> Result<MangasPage> {
        let needle = query.to_lowercase();
        let mangas = self
            .scan()
            .into_iter()
            .filter(|m| needle.is_empty() || m.title.to_lowercase().contains(&needle))
            .collect();
        Ok(MangasPage {
            mangas,
            has_next_page: false,
        })
    }

    async fn details(&self, manga: &SManga) -> Result<SManga> {
        let path = self
            .resolve(&manga.url)
            .with_context(|| format!("local series {:?} no longer exists", manga.url))?;
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or(&manga.title);
        let mut parsed = read_series(&path, name);
        parsed.initialized = true;
        Ok(parsed)
    }

    async fn chapters(&self, manga: &SManga) -> Result<Vec<SChapter>> {
        let path = self
            .resolve(&manga.url)
            .with_context(|| format!("local series {:?} no longer exists", manga.url))?;

        let mut chapters = Vec::new();
        for entry in std::fs::read_dir(&path)?.flatten() {
            let child = entry.path();
            let Some(name) = child.file_name().and_then(|n| n.to_str()) else {
                continue;
            };

            let is_archive = has_extension(&child, ARCHIVE_EXTENSIONS);
            let is_chapter_dir = child.is_dir() && contains_images(&child);
            if !is_archive && !is_chapter_dir {
                continue;
            }

            let display = if is_archive {
                child
                    .file_stem()
                    .and_then(|n| n.to_str())
                    .unwrap_or(name)
                    .to_string()
            } else {
                name.to_string()
            };

            let modified = entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);

            chapters.push(SChapter {
                url: format!("{}/{}", manga.url.trim_start_matches('/'), name),
                chapter_number: super::recognise_chapter_number(&manga.title, &display),
                name: display,
                date_upload: modified,
                scanlator: None,
            });
        }

        // Newest first, using the recognised number when there is one.
        chapters.sort_by(|a, b| {
            b.chapter_number
                .total_cmp(&a.chapter_number)
                .then_with(|| natural_cmp(&b.name, &a.name))
        });
        Ok(chapters)
    }

    async fn pages(&self, manga: &SManga, chapter: &SChapter) -> Result<Vec<Page>> {
        let series = self
            .resolve(&manga.url)
            .with_context(|| format!("local series {:?} no longer exists", manga.url))?;
        let file_name = chapter
            .url
            .rsplit('/')
            .next()
            .context("malformed local chapter url")?;
        let target = series.join(file_name);

        if target.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&target)?
                .flatten()
                .map(|e| e.path())
                .filter(|p| has_extension(p, IMAGE_EXTENSIONS))
                .collect();
            files.sort_by(|a, b| natural_cmp(&file_label(a), &file_label(b)));
            if files.is_empty() {
                bail!("no images in {}", target.display());
            }
            return Ok(files
                .into_iter()
                .enumerate()
                .map(|(index, path)| Page {
                    index,
                    image_url: format!("{FILE_SCHEME}{}", path.display()),
                    fallbacks: Vec::new(),
                    headers: Vec::new(),
                })
                .collect());
        }

        if has_extension(&target, ARCHIVE_EXTENSIONS) {
            let names = archive_entries(&target)?;
            if names.is_empty() {
                bail!("no images in {}", target.display());
            }
            return Ok(names
                .into_iter()
                .enumerate()
                .map(|(index, entry)| Page {
                    index,
                    image_url: format!("{CBZ_SCHEME}{}|{entry}", target.display()),
                    fallbacks: Vec::new(),
                    headers: Vec::new(),
                })
                .collect());
        }

        bail!("{} is neither a folder nor an archive", target.display())
    }

    fn web_url(&self, _manga: &SManga) -> String {
        String::new()
    }
}

// ---------------------------------------------------------------------------
// Disk helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize, Default)]
struct DetailsJson {
    title: Option<String>,
    author: Option<String>,
    artist: Option<String>,
    description: Option<String>,
    genre: Option<Vec<String>>,
    status: Option<String>,
}

fn read_series(path: &Path, folder_name: &str) -> SManga {
    let details: DetailsJson = std::fs::read_to_string(path.join("details.json"))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();

    let status = match details.status.as_deref().map(str::to_lowercase).as_deref() {
        Some("ongoing") => MangaStatus::Ongoing,
        Some("completed") => MangaStatus::Completed,
        Some("licensed") => MangaStatus::Licensed,
        Some("cancelled") => MangaStatus::Cancelled,
        Some("hiatus" | "on hiatus") => MangaStatus::OnHiatus,
        _ => MangaStatus::Unknown,
    };

    SManga {
        url: folder_name.to_string(),
        title: details.title.unwrap_or_else(|| folder_name.to_string()),
        artist: details.artist,
        author: details.author,
        description: details.description,
        genre: details.genre,
        status,
        thumbnail_url: find_cover(path).map(|p| format!("{FILE_SCHEME}{}", p.display())),
        // Local content never changes behind our back on a schedule.
        update_strategy: UpdateStrategy::AlwaysUpdate,
        initialized: false,
    }
}

/// Uses `cover.*` when present, otherwise the first image of the first chapter.
fn find_cover(series: &Path) -> Option<PathBuf> {
    for ext in IMAGE_EXTENSIONS {
        let candidate = series.join(format!("cover.{ext}"));
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let mut children: Vec<PathBuf> = std::fs::read_dir(series)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    children.sort_by(|a, b| natural_cmp(&file_label(a), &file_label(b)));

    for child in children {
        if child.is_dir() {
            let mut images: Vec<PathBuf> = std::fs::read_dir(&child)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| has_extension(p, IMAGE_EXTENSIONS))
                .collect();
            images.sort_by(|a, b| natural_cmp(&file_label(a), &file_label(b)));
            if let Some(first) = images.into_iter().next() {
                return Some(first);
            }
        }
    }
    None
}

fn contains_images(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .flatten()
                .any(|e| has_extension(&e.path(), IMAGE_EXTENSIONS))
        })
        .unwrap_or(false)
}

fn has_extension(path: &Path, extensions: &[&str]) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_lowercase())
        .map(|e| extensions.contains(&e.as_str()))
        .unwrap_or(false)
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Sorted list of image entries inside a CBZ/ZIP.
pub fn archive_entries(archive: &Path) -> Result<Vec<String>> {
    let file =
        std::fs::File::open(archive).with_context(|| format!("opening {}", archive.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).with_context(|| format!("reading {}", archive.display()))?;

    let mut names = Vec::new();
    for index in 0..zip.len() {
        let entry = zip.by_index(index)?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if IMAGE_EXTENSIONS
            .iter()
            .any(|ext| name.to_lowercase().ends_with(&format!(".{ext}")))
        {
            names.push(name);
        }
    }
    names.sort_by(|a, b| natural_cmp(a, b));
    Ok(names)
}

/// Reads one page referenced by a `file://` or `cbz://` url.
pub fn read_local_page(url: &str) -> Result<Vec<u8>> {
    if let Some(rest) = url.strip_prefix(CBZ_SCHEME) {
        let (archive, entry) = rest
            .rsplit_once('|')
            .context("malformed cbz page reference")?;
        let file = std::fs::File::open(archive).with_context(|| format!("opening {archive}"))?;
        let mut zip = zip::ZipArchive::new(file)?;
        let mut item = zip
            .by_name(entry)
            .with_context(|| format!("{entry} is missing from {archive}"))?;
        let mut buffer = Vec::with_capacity(item.size() as usize);
        item.read_to_end(&mut buffer)?;
        return Ok(buffer);
    }

    if let Some(path) = url.strip_prefix(FILE_SCHEME) {
        return std::fs::read(path).with_context(|| format!("reading {path}"));
    }

    bail!("{url} is not a local page reference")
}

pub fn is_local_url(url: &str) -> bool {
    url.starts_with(CBZ_SCHEME) || url.starts_with(FILE_SCHEME)
}

/// Compares strings so that "2" sorts before "10".
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let mut left = a.chars().peekable();
    let mut right = b.chars().peekable();

    loop {
        match (left.peek().copied(), right.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, Some(_)) => return std::cmp::Ordering::Less,
            (Some(_), None) => return std::cmp::Ordering::Greater,
            (Some(lc), Some(rc)) => {
                if lc.is_ascii_digit() && rc.is_ascii_digit() {
                    let lnum = take_number(&mut left);
                    let rnum = take_number(&mut right);
                    match lnum.cmp(&rnum) {
                        std::cmp::Ordering::Equal => {}
                        other => return other,
                    }
                } else {
                    let lk = lc.to_ascii_lowercase();
                    let rk = rc.to_ascii_lowercase();
                    match lk.cmp(&rk) {
                        std::cmp::Ordering::Equal => {
                            left.next();
                            right.next();
                        }
                        other => return other,
                    }
                }
            }
        }
    }
}

fn take_number(chars: &mut std::iter::Peekable<std::str::Chars>) -> u128 {
    let mut value: u128 = 0;
    while let Some(c) = chars.peek() {
        if let Some(digit) = c.to_digit(10) {
            // Saturate rather than overflow on absurdly long digit runs.
            value = value.saturating_mul(10).saturating_add(digit as u128);
            chars.next();
        } else {
            break;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cmp::Ordering;

    #[test]
    fn natural_order_sorts_numbers_numerically() {
        assert_eq!(natural_cmp("page2.jpg", "page10.jpg"), Ordering::Less);
        assert_eq!(natural_cmp("Chapter 10", "Chapter 9"), Ordering::Greater);
        assert_eq!(natural_cmp("a", "A"), Ordering::Equal);
    }

    /// Builds a small library on disk and walks it the way the app does:
    /// scan -> details -> chapters -> pages -> decode the bytes.
    #[test]
    fn local_library_round_trip() {
        let root = std::env::temp_dir().join(format!("mihon-local-{}", std::process::id()));
        let series = root.join("Test Series");
        let folder_chapter = series.join("Chapter 2");
        std::fs::create_dir_all(&folder_chapter).unwrap();

        // A one-pixel PNG, reused for every page.
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image::RgbaImage::from_pixel(
            2,
            3,
            image::Rgba([9, 9, 9, 255]),
        ))
        .write_to(&mut png, image::ImageFormat::Png)
        .unwrap();
        let png = png.into_inner();

        std::fs::write(folder_chapter.join("02.png"), &png).unwrap();
        std::fs::write(folder_chapter.join("10.png"), &png).unwrap();
        std::fs::write(
            series.join("details.json"),
            r#"{"author":"Someone","status":"completed"}"#,
        )
        .unwrap();

        // And one chapter stored as a CBZ.
        {
            let file = std::fs::File::create(series.join("Chapter 1.cbz")).unwrap();
            let mut zip = zip::ZipWriter::new(file);
            let options = zip::write::SimpleFileOptions::default()
                .compression_method(zip::CompressionMethod::Stored);
            for name in ["001.png", "002.png"] {
                zip.start_file(name, options).unwrap();
                std::io::Write::write_all(&mut zip, &png).unwrap();
            }
            zip.finish().unwrap();
        }

        let source = LocalSource::new(root.clone());
        let runtime = tokio::runtime::Runtime::new().unwrap();

        runtime.block_on(async {
            let listing = source.popular(1).await.unwrap();
            assert_eq!(listing.mangas.len(), 1);
            let manga = &listing.mangas[0];
            assert_eq!(manga.title, "Test Series");

            let details = source.details(manga).await.unwrap();
            assert_eq!(details.author.as_deref(), Some("Someone"));
            assert_eq!(details.status, MangaStatus::Completed);
            assert!(
                details.thumbnail_url.is_some(),
                "a cover should be inferred"
            );

            let chapters = source.chapters(manga).await.unwrap();
            assert_eq!(chapters.len(), 2, "one folder and one archive");
            // Newest first: chapter 2 leads.
            assert_eq!(chapters[0].chapter_number, 2.0);
            assert_eq!(chapters[1].chapter_number, 1.0);

            // The folder chapter, in natural page order.
            let pages = source.pages(manga, &chapters[0]).await.unwrap();
            assert_eq!(pages.len(), 2);
            assert!(
                pages[0].image_url.ends_with("02.png"),
                "got {}",
                pages[0].image_url
            );
            assert!(pages[1].image_url.ends_with("10.png"));

            // The archived chapter, read back through the cbz:// scheme.
            let pages = source.pages(manga, &chapters[1]).await.unwrap();
            assert_eq!(pages.len(), 2);
            let bytes = read_local_page(&pages[0].image_url).unwrap();
            let decoded = image::load_from_memory(&bytes).unwrap();
            assert_eq!((decoded.width(), decoded.height()), (2, 3));
        });

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn local_urls_are_recognised() {
        assert!(is_local_url("file://C:/x/y.jpg"));
        assert!(is_local_url("cbz://C:/x/a.cbz|001.jpg"));
        assert!(!is_local_url("https://example.com/a.jpg"));
    }
}
