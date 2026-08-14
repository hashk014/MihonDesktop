//! Content sources.
//!
//! Mirrors Mihon's `CatalogueSource` contract: a source can list popular and
//! latest entries, run a search with a filter list it defines itself, and
//! resolve details, chapters and pages. Built-in sources are compiled in;
//! scripted ones are loaded at runtime from `extensions/` (see [`ext`]).

pub mod animesama;
pub mod ext;
pub mod local;
pub mod mangadex;

use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use sha2::{Digest, Sha256};

use crate::model::{MangaStatus, TriState, UpdateStrategy};
use crate::net::HttpClient;

// ---------------------------------------------------------------------------
// Wire types
// ---------------------------------------------------------------------------

/// A manga as returned by a source, before it is stored locally.
#[derive(Debug, Clone, Default)]
pub struct SManga {
    pub url: String,
    pub title: String,
    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    pub genre: Option<Vec<String>>,
    pub status: MangaStatus,
    pub thumbnail_url: Option<String>,
    pub update_strategy: UpdateStrategy,
    /// Set once `details` has filled the record in.
    pub initialized: bool,
}

impl SManga {
    pub fn new(url: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            title: title.into(),
            ..Default::default()
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct SChapter {
    pub url: String,
    pub name: String,
    pub date_upload: i64,
    pub chapter_number: f64,
    pub scanlator: Option<String>,
}

/// One page of a chapter. `image_url` is what actually gets downloaded.
#[derive(Debug, Clone)]
pub struct Page {
    pub index: usize,
    pub image_url: String,
    /// Alternative urls tried when `image_url` fails. Mirrors and reduced-size
    /// variants live here; individual files do go missing on CDN nodes.
    pub fallbacks: Vec<String>,
    /// Headers required to fetch this image (hotlink protection, mostly).
    pub headers: Vec<(String, String)>,
}

impl Page {
    /// Every url to try, in order.
    pub fn candidates(&self) -> Vec<String> {
        let mut all = Vec::with_capacity(1 + self.fallbacks.len());
        all.push(self.image_url.clone());
        all.extend(self.fallbacks.iter().cloned());
        all
    }
}

#[derive(Debug, Clone, Default)]
pub struct MangasPage {
    pub mangas: Vec<SManga>,
    pub has_next_page: bool,
}

// ---------------------------------------------------------------------------
// Filters
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub enum FilterKind {
    Header,
    Separator,
    Text {
        value: String,
    },
    Select {
        /// Labels shown to the user.
        options: Vec<String>,
        /// Values handed back to the source, parallel to `options`.
        values: Vec<String>,
        index: usize,
    },
    CheckBox {
        checked: bool,
        value: String,
    },
    Tri {
        state: TriState,
        value: String,
    },
    Sort {
        options: Vec<String>,
        values: Vec<String>,
        index: usize,
        ascending: bool,
    },
    Group {
        children: Vec<Filter>,
    },
}

#[derive(Debug, Clone)]
pub struct Filter {
    pub name: String,
    pub kind: FilterKind,
}

impl Filter {
    pub fn header(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::Header,
        }
    }

    pub fn separator() -> Self {
        Self {
            name: String::new(),
            kind: FilterKind::Separator,
        }
    }

    pub fn text(name: &str) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::Text {
                value: String::new(),
            },
        }
    }

    pub fn sort(name: &str, options: &[(&str, &str)], index: usize, ascending: bool) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::Sort {
                options: options.iter().map(|(l, _)| l.to_string()).collect(),
                values: options.iter().map(|(_, v)| v.to_string()).collect(),
                index,
                ascending,
            },
        }
    }

    pub fn checkbox(name: &str, value: &str) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::CheckBox {
                checked: false,
                value: value.into(),
            },
        }
    }

    pub fn tri(name: &str, value: &str) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::Tri {
                state: TriState::Disabled,
                value: value.into(),
            },
        }
    }

    pub fn group(name: &str, children: Vec<Filter>) -> Self {
        Self {
            name: name.into(),
            kind: FilterKind::Group { children },
        }
    }
}

/// The filter list a source hands to the UI and receives back on search.
pub type FilterList = Vec<Filter>;

/// Walks a filter list (including groups) applying `f` to every leaf.
pub fn for_each_filter(filters: &FilterList, f: &mut impl FnMut(&Filter)) {
    for filter in filters {
        if let FilterKind::Group { children } = &filter.kind {
            for_each_filter(children, f);
        }
        f(filter);
    }
}

// ---------------------------------------------------------------------------
// The trait
// ---------------------------------------------------------------------------

#[async_trait]
pub trait Source: Send + Sync {
    fn id(&self) -> i64;
    fn name(&self) -> &str;
    /// ISO language code, or "all"/"localsourcelang" for the special sources.
    fn lang(&self) -> &str;
    fn base_url(&self) -> &str;

    fn is_nsfw(&self) -> bool {
        false
    }

    fn supports_latest(&self) -> bool {
        true
    }

    /// True for sources loaded from an extension manifest rather than compiled in.
    fn is_scripted(&self) -> bool {
        false
    }

    /// The default filter list shown on the browse screen.
    fn filters(&self) -> FilterList {
        Vec::new()
    }

    /// Extra headers needed to fetch cover and page images.
    fn image_headers(&self) -> Vec<(String, String)> {
        Vec::new()
    }

    async fn popular(&self, page: u32) -> Result<MangasPage>;

    async fn latest(&self, page: u32) -> Result<MangasPage>;

    async fn search(&self, page: u32, query: &str, filters: &FilterList) -> Result<MangasPage>;

    async fn details(&self, manga: &SManga) -> Result<SManga>;

    async fn chapters(&self, manga: &SManga) -> Result<Vec<SChapter>>;

    async fn pages(&self, manga: &SManga, chapter: &SChapter) -> Result<Vec<Page>>;

    /// Absolute URL for the "open in browser" action.
    fn web_url(&self, manga: &SManga) -> String {
        crate::net::absolute_url(self.base_url(), &manga.url)
    }
}

/// Stable source id derived from identity, the way upstream hashes
/// `name/lang/versionId`. Kept positive so it can be stored as a rowid.
pub fn generate_id(name: &str, lang: &str, version: u32) -> i64 {
    let key = format!("{}/{}/{}", name.to_lowercase(), lang, version);
    let digest = Sha256::digest(key.as_bytes());
    let mut value: u64 = 0;
    for byte in digest.iter().take(8) {
        value = (value << 8) | *byte as u64;
    }
    (value & (i64::MAX as u64)) as i64
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

/// Holds every source instance available to the app.
pub struct SourceManager {
    sources: parking_lot::RwLock<Vec<Arc<dyn Source>>>,
}

impl SourceManager {
    pub fn new() -> Self {
        Self {
            sources: parking_lot::RwLock::new(Vec::new()),
        }
    }

    /// Registers the compiled-in sources.
    pub fn with_builtins(http: Arc<HttpClient>, paths: &crate::prefs::AppPaths) -> Self {
        let manager = Self::new();
        manager.register(Arc::new(local::LocalSource::new(
            paths.local_source.clone(),
        )));
        for lang in mangadex::SUPPORTED_LANGS {
            manager.register(Arc::new(mangadex::MangaDex::new(http.clone(), lang)));
        }
        manager.register(Arc::new(animesama::AnimeSama::new(http.clone())));
        manager
    }

    pub fn register(&self, source: Arc<dyn Source>) {
        let mut sources = self.sources.write();
        let id = source.id();
        match sources.iter_mut().find(|s| s.id() == id) {
            Some(slot) => *slot = source,
            None => sources.push(source),
        }
    }

    /// Drops every source that came from an extension manifest.
    pub fn remove_scripted(&self) {
        self.sources.write().retain(|s| !s.is_scripted());
    }

    pub fn get(&self, id: i64) -> Option<Arc<dyn Source>> {
        self.sources.read().iter().find(|s| s.id() == id).cloned()
    }

    pub fn all(&self) -> Vec<Arc<dyn Source>> {
        self.sources.read().clone()
    }

    /// Sources visible on the Browse screen, honouring NSFW and language prefs.
    pub fn visible(&self, prefs: &crate::prefs::Preferences) -> Vec<Arc<dyn Source>> {
        let langs = &prefs.browse.enabled_languages;
        self.sources
            .read()
            .iter()
            .filter(|s| prefs.browse.show_nsfw_sources || !s.is_nsfw())
            .filter(|s| {
                langs.is_empty()
                    || s.lang() == local::LOCAL_LANG
                    || langs.iter().any(|l| l == s.lang())
            })
            .filter(|s| prefs.is_source_enabled(s.id()))
            .cloned()
            .collect()
    }

    pub fn languages(&self) -> Vec<String> {
        let mut langs: Vec<String> = self
            .sources
            .read()
            .iter()
            .map(|s| s.lang().to_string())
            .filter(|l| l != local::LOCAL_LANG)
            .collect();
        langs.sort();
        langs.dedup();
        langs
    }
}

impl Default for SourceManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Helpers shared by source implementations
// ---------------------------------------------------------------------------

/// Best-effort extraction of a chapter number out of a chapter title, matching
/// upstream's `ChapterRecognition` closely enough for sorting to behave.
pub fn recognise_chapter_number(manga_title: &str, chapter_name: &str) -> f64 {
    let name = chapter_name.to_lowercase();
    // Drop the series title if the chapter repeats it, so "Bleach 12" does not
    // pick up a number that belongs to the series name.
    let name = name.replace(&manga_title.to_lowercase(), " ");
    // Remove common volume prefixes; they would otherwise win the match.
    let name = regex_replace(&name, r"vol\.?\s*\d+(\.\d+)?", " ");

    let patterns = [
        r"(?:ch(?:apter|\.)?|episode|ep\.?|#)\s*(\d+(?:\.\d+)?)",
        r"\b(\d+(?:\.\d+)?)\s*(?:$|[^\d])",
    ];
    for pattern in patterns {
        if let Ok(re) = regex::Regex::new(pattern)
            && let Some(caps) = re.captures(&name)
            && let Some(m) = caps.get(1)
            && let Ok(value) = m.as_str().parse::<f64>()
        {
            return value;
        }
    }
    -1.0
}

fn regex_replace(haystack: &str, pattern: &str, replacement: &str) -> String {
    match regex::Regex::new(pattern) {
        Ok(re) => re.replace_all(haystack, replacement).into_owned(),
        Err(_) => haystack.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chapter_numbers_are_recognised() {
        assert_eq!(recognise_chapter_number("Bleach", "Chapter 12"), 12.0);
        assert_eq!(recognise_chapter_number("Bleach", "Ch.5.5 - Extra"), 5.5);
        assert_eq!(recognise_chapter_number("Naruto", "Vol.2 Ch.7"), 7.0);
        assert_eq!(recognise_chapter_number("One Piece", "Prologue"), -1.0);
    }

    #[test]
    fn source_ids_are_stable_and_positive() {
        let a = generate_id("MangaDex", "en", 1);
        let b = generate_id("MangaDex", "en", 1);
        assert_eq!(a, b);
        assert!(a > 0);
        assert_ne!(a, generate_id("MangaDex", "fr", 1));
    }
}
