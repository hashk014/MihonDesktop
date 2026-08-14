//! The desktop UI.
//!
//! Layout follows Mihon's information architecture — Library, Updates, History,
//! Browse, More — but uses a left navigation rail instead of a bottom bar,
//! which suits a resizable desktop window better.

pub mod browse;
pub mod capture;
pub mod categories;
pub mod downloads;
pub mod extensions;
pub mod fonts;
pub mod history;
pub mod library;
pub mod manga;
pub mod reader;
pub mod settings;
pub mod source_browse;
pub mod theme;
pub mod updates;
pub mod widgets;

use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use std::time::Instant;

use egui::{Align, Color32, Layout, RichText, Vec2};

use crate::core::{BrowseMode, Core};
use crate::event::{AppEvent, EventBus, Toast, ToastKind};
use crate::images::{ImageKind, ImageSlot, TextureCache};
use crate::model::*;
use crate::prefs::{AppPaths, NavStyle, Preferences};
use crate::source::ext::{InstalledExtension, RepoEntry};
use crate::source::{FilterList, MangasPage, Page, SManga};
use theme::Palette;

// ---------------------------------------------------------------------------
// Navigation
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Library,
    Updates,
    History,
    Browse,
    More,
}

impl Tab {
    pub const ALL: [Tab; 5] = [
        Tab::Library,
        Tab::Updates,
        Tab::History,
        Tab::Browse,
        Tab::More,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Tab::Library => "Library",
            Tab::Updates => "Updates",
            Tab::History => "History",
            Tab::Browse => "Browse",
            Tab::More => "More",
        }
    }

    /// Only glyphs present in egui's bundled fonts are used here; anything else
    /// renders as an empty box.
    pub fn glyph(self) -> &'static str {
        match self {
            Tab::Library => "📚",
            Tab::Updates => "🔄",
            Tab::History => "🕘",
            Tab::Browse => "🌐",
            Tab::More => "☰",
        }
    }

    pub fn index(self) -> usize {
        Self::ALL.iter().position(|t| *t == self).unwrap_or(0)
    }

    pub fn from_index(index: usize) -> Self {
        *Self::ALL.get(index).unwrap_or(&Tab::Library)
    }
}

/// Screens pushed on top of the tabs.
#[derive(Debug, Clone, PartialEq)]
pub enum Route {
    SourceBrowse(Id),
    MangaDetails(Id),
    Reader { manga_id: Id, chapter_id: Id },
    Downloads,
    Categories,
    Extensions,
    Settings(SettingsPage),
    Statistics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettingsPage {
    Root,
    Appearance,
    Library,
    Reader,
    Downloads,
    Browse,
    DataStorage,
    About,
}

impl SettingsPage {
    pub fn title(self) -> &'static str {
        match self {
            Self::Root => "Settings",
            Self::Appearance => "Appearance",
            Self::Library => "Library",
            Self::Reader => "Reader",
            Self::Downloads => "Downloads",
            Self::Browse => "Browse",
            Self::DataStorage => "Data and storage",
            Self::About => "About",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseTab {
    Sources,
    Extensions,
    Migrate,
}

// ---------------------------------------------------------------------------
// Per-screen state
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct LibraryState {
    pub entries: Vec<LibraryEntry>,
    pub categories: Vec<Category>,
    pub category_index: usize,
    pub query: String,
    pub selection: HashSet<Id>,
    pub dirty: bool,
    /// Seed so the "random" sort stays stable between frames.
    pub random_seed: u64,
}

#[derive(Default)]
pub struct UpdatesState {
    pub entries: Vec<UpdatesEntry>,
    pub query: String,
    pub selection: HashSet<Id>,
    pub dirty: bool,
}

#[derive(Default)]
pub struct HistoryState {
    pub entries: Vec<HistoryEntry>,
    pub query: String,
    pub dirty: bool,
}

pub struct GlobalSearchResult {
    pub source_name: String,
    pub loading: bool,
    pub items: Vec<SManga>,
    pub error: Option<String>,
}

pub struct BrowseState {
    pub tab: BrowseTab,
    pub query: String,
    pub submitted_query: String,
    pub results: BTreeMap<Id, GlobalSearchResult>,
    pub migrate_source: Option<Id>,
}

impl Default for BrowseState {
    fn default() -> Self {
        Self {
            tab: BrowseTab::Sources,
            query: String::new(),
            submitted_query: String::new(),
            results: BTreeMap::new(),
            migrate_source: None,
        }
    }
}

pub struct SourceBrowseState {
    pub source_id: Id,
    pub mode: BrowseMode,
    pub query: String,
    pub filters: FilterList,
    pub items: Vec<SManga>,
    pub page: u32,
    pub has_next: bool,
    pub loading: bool,
    pub error: Option<String>,
    pub show_filters: bool,
}

impl Default for SourceBrowseState {
    fn default() -> Self {
        Self {
            source_id: 0,
            mode: BrowseMode::Popular,
            query: String::new(),
            filters: Vec::new(),
            items: Vec::new(),
            page: 1,
            has_next: false,
            loading: false,
            error: None,
            show_filters: false,
        }
    }
}

#[derive(Default)]
pub struct DetailsState {
    pub manga_id: Id,
    pub manga: Option<Manga>,
    pub chapters: Vec<Chapter>,
    pub selection: HashSet<Id>,
    pub loading: bool,
    pub error: Option<String>,
    pub description_expanded: bool,
    pub notes_buffer: String,
    pub notes_open: bool,
}

/// One chapter's worth of pages inside the reader's page list. Infinite scroll
/// keeps several of these back to back.
#[derive(Debug, Clone)]
pub struct ReaderSegment {
    pub chapter_id: Id,
    pub name: String,
    /// Index of this chapter's first page in [`ReaderState::pages`].
    pub start: usize,
    pub len: usize,
}

pub struct ReaderState {
    pub manga_id: Id,
    /// The chapter the reader is currently showing; in infinite scroll this
    /// follows whatever is under the viewport.
    pub chapter_id: Id,
    pub manga_title: String,
    pub chapter_name: String,
    /// Every chapter of the manga, ascending, for prev/next navigation.
    pub chapters: Vec<Chapter>,
    pub pages: Vec<Page>,
    /// Which chapter each page belongs to, parallel to `pages`.
    pub page_owner: Vec<Id>,
    /// Chapter boundaries, in reading order.
    pub segments: Vec<ReaderSegment>,
    /// Chapter whose pages are being fetched to append, if any.
    pub appending: Option<Id>,
    /// Set when there is nothing left to append.
    pub reached_end: bool,
    pub current: usize,
    pub loading: bool,
    pub error: Option<String>,
    pub menu_visible: bool,
    pub zoom: f32,
    pub pan: Vec2,
    pub mode: ReadingMode,
    pub opened_at: Instant,
    /// Set once, to restore the stored page after the list arrives.
    pub restore_page: Option<i64>,
    /// Width/height ratio per page, learned as images decode. Continuous mode
    /// needs a height estimate before the image is available.
    pub aspects: Vec<f32>,
    /// Scroll offset in continuous mode, kept so the menu can seek.
    pub scroll_to: Option<usize>,
}

impl Default for ReaderState {
    fn default() -> Self {
        Self {
            manga_id: 0,
            chapter_id: 0,
            manga_title: String::new(),
            chapter_name: String::new(),
            chapters: Vec::new(),
            pages: Vec::new(),
            page_owner: Vec::new(),
            segments: Vec::new(),
            appending: None,
            reached_end: false,
            current: 0,
            loading: false,
            error: None,
            menu_visible: true,
            zoom: 1.0,
            pan: Vec2::ZERO,
            mode: ReadingMode::RightToLeft,
            opened_at: Instant::now(),
            restore_page: None,
            aspects: Vec::new(),
            scroll_to: None,
        }
    }
}

#[derive(Default)]
pub struct ExtensionsState {
    pub installed: Vec<InstalledExtension>,
    pub available: BTreeMap<String, Vec<RepoEntry>>,
    pub loading: bool,
    pub query: String,
    pub dirty: bool,
}

/// Modal dialogs. Only one can be open at a time, like upstream's sheets.
pub enum Dialog {
    CategoryPicker {
        manga_ids: Vec<Id>,
        selected: HashSet<Id>,
    },
    ConfirmRemove {
        manga_ids: Vec<Id>,
        delete_downloads: bool,
    },
    NewCategory {
        name: String,
    },
    RenameCategory {
        id: Id,
        name: String,
    },
    LibrarySettings,
    ChapterSettings,
    ReaderSettings,
    AddRepo {
        url: String,
    },
    ConfirmClearHistory,
    MigrateTarget {
        manga_id: Id,
    },
}

/// Texture settings for artwork.
///
/// Manga pages and covers are almost always drawn smaller than they are stored
/// — a 1700px scan in a 900px column, a 512px cover in a 180px tile. Plain
/// bilinear minification samples four texels out of dozens, which is what makes
/// dense line art look soft and shimmery. Mipmaps fix that; the glow backend
/// this app uses supports them.
fn image_texture_options() -> egui::TextureOptions {
    egui::TextureOptions {
        magnification: egui::TextureFilter::Linear,
        minification: egui::TextureFilter::Linear,
        wrap_mode: egui::TextureWrapMode::ClampToEdge,
        mipmap_mode: Some(egui::TextureFilter::Linear),
    }
}

// ---------------------------------------------------------------------------
// The application
// ---------------------------------------------------------------------------

pub struct App {
    pub core: Core,
    pub bus: EventBus,
    pub prefs: Preferences,
    pub palette: Palette,
    prefs_dirty: Option<Instant>,

    pub covers: TextureCache,
    pub page_textures: TextureCache,

    pub toasts: Vec<Toast>,
    pub tab: Tab,
    pub stack: Vec<Route>,

    pub library: LibraryState,
    pub updates: UpdatesState,
    pub history: HistoryState,
    pub browse: BrowseState,
    pub source_browse: SourceBrowseState,
    pub details: DetailsState,
    pub reader: ReaderState,
    pub extensions: ExtensionsState,
    pub dialog: Option<Dialog>,

    pub update_progress: Option<(usize, usize, String)>,
    /// Present only when the screenshot harness is enabled via the environment.
    pub capture: Option<capture::Capture>,
    first_frame: bool,
}

impl App {
    pub fn new(paths: AppPaths, prefs: Preferences, bus: EventBus, core: Core) -> Self {
        // Replaced on the first frame, once egui can report the desktop theme.
        let palette = Palette::build(&prefs, prefs.theme_mode.is_dark(None));
        let _ = paths;
        let tab = Tab::from_index(prefs.start_tab);

        Self {
            core,
            bus,
            prefs,
            palette,
            prefs_dirty: None,
            // Covers are small and numerous; pages can be enormous, so they are
            // bounded by pixels rather than by count.
            covers: TextureCache::new(400, 64_000_000),
            page_textures: TextureCache::new(16, 80_000_000),
            toasts: Vec::new(),
            tab,
            stack: Vec::new(),
            library: LibraryState {
                dirty: true,
                random_seed: 0x5eed,
                ..Default::default()
            },
            updates: UpdatesState {
                dirty: true,
                ..Default::default()
            },
            history: HistoryState {
                dirty: true,
                ..Default::default()
            },
            browse: BrowseState::default(),
            source_browse: SourceBrowseState::default(),
            details: DetailsState::default(),
            reader: ReaderState::default(),
            extensions: ExtensionsState {
                dirty: true,
                ..Default::default()
            },
            dialog: None,
            update_progress: None,
            capture: capture::Capture::from_env(),
            first_frame: true,
        }
    }

    // -- preferences --------------------------------------------------------

    /// Marks preferences as changed; they are flushed shortly after.
    pub fn prefs_changed(&mut self) {
        self.prefs_dirty = Some(Instant::now());
    }

    fn flush_prefs(&mut self, force: bool) {
        let Some(since) = self.prefs_dirty else {
            return;
        };
        if force || since.elapsed().as_millis() > 700 {
            self.prefs.save(&self.core.paths.prefs);
            self.prefs_dirty = None;
        }
    }

    // -- navigation ---------------------------------------------------------

    pub fn push(&mut self, route: Route) {
        // Opening the same screen twice in a row is a no-op.
        if self.stack.last() == Some(&route) {
            return;
        }
        self.stack.push(route);
    }

    pub fn pop(&mut self) {
        self.stack.pop();
    }

    pub fn current_route(&self) -> Option<&Route> {
        self.stack.last()
    }

    pub fn open_manga(&mut self, manga_id: Id) {
        self.details = DetailsState {
            manga_id,
            loading: true,
            ..Default::default()
        };
        self.refresh_details();

        let manga = self.core.db.get_manga(manga_id);
        let needs_details = manga.as_ref().map(|m| !m.initialized).unwrap_or(true);
        let needs_chapters = self.details.chapters.is_empty();
        if needs_details || needs_chapters {
            self.core.refresh_manga(manga_id, needs_details);
        } else {
            self.details.loading = false;
        }
        self.push(Route::MangaDetails(manga_id));
    }

    pub fn open_source(&mut self, source_id: Id) {
        let filters = self
            .core
            .sources
            .get(source_id)
            .map(|s| s.filters())
            .unwrap_or_default();
        self.source_browse = SourceBrowseState {
            source_id,
            filters,
            loading: true,
            ..Default::default()
        };
        self.core
            .browse(source_id, BrowseMode::Popular, 1, String::new(), Vec::new());
        self.push(Route::SourceBrowse(source_id));
    }

    pub fn open_reader(&mut self, manga_id: Id, chapter_id: Id) {
        let manga = self.core.db.get_manga(manga_id);
        let chapter = self.core.db.get_chapter(chapter_id);

        let mut chapters = self.core.db.chapters_of(manga_id);
        chapters.sort_by(reading_order);

        let mode = manga
            .as_ref()
            .and_then(|m| m.reading_mode())
            .unwrap_or(self.prefs.reader.default_reading_mode);

        self.reader = ReaderState {
            manga_id,
            chapter_id,
            manga_title: manga.map(|m| m.title).unwrap_or_default(),
            chapter_name: chapter.as_ref().map(|c| c.name.clone()).unwrap_or_default(),
            restore_page: chapter.as_ref().map(|c| c.last_page_read),
            chapters,
            loading: true,
            mode,
            ..Default::default()
        };
        self.page_textures.clear();
        self.core.fetch_pages(manga_id, chapter_id);
        self.queue_read_ahead(manga_id, chapter_id);
        self.push(Route::Reader {
            manga_id,
            chapter_id,
        });
    }

    /// Appends a chapter's pages to the end of the reader, for infinite scroll.
    fn append_chapter(&mut self, chapter_id: Id, pages: Vec<Page>) {
        // Guard against a late duplicate reply for a chapter already appended.
        if self
            .reader
            .segments
            .iter()
            .any(|s| s.chapter_id == chapter_id)
        {
            return;
        }
        let name = self
            .reader
            .chapters
            .iter()
            .find(|c| c.id == chapter_id)
            .map(|c| c.name.clone())
            .unwrap_or_default();

        self.reader.segments.push(ReaderSegment {
            chapter_id,
            name,
            start: self.reader.pages.len(),
            len: pages.len(),
        });
        self.reader
            .aspects
            .extend(std::iter::repeat_n(0.7, pages.len()));
        self.reader
            .page_owner
            .extend(std::iter::repeat_n(chapter_id, pages.len()));
        self.reader.pages.extend(pages);
    }

    /// Queues the next few chapters for download, as upstream's "download ahead" does.
    fn queue_read_ahead(&mut self, manga_id: Id, chapter_id: Id) {
        let ahead = self.prefs.downloads.download_ahead as usize;
        if ahead == 0 || self.prefs.incognito {
            return;
        }
        let Some(position) = self.reader.chapters.iter().position(|c| c.id == chapter_id) else {
            return;
        };
        let upcoming: Vec<Id> = self
            .reader
            .chapters
            .iter()
            .skip(position + 1)
            .filter(|c| !c.read)
            .take(ahead)
            .filter(|c| !self.core.downloads.is_downloaded(manga_id, c.id))
            .map(|c| c.id)
            .collect();
        if !upcoming.is_empty() {
            self.core.queue_downloads(manga_id, &upcoming);
        }
    }

    /// Kicks off the scheduled library check when enough time has passed.
    fn maybe_auto_update(&mut self) {
        let hours = self.prefs.library.update_interval_hours;
        if hours == 0 || self.core.db.library_size() == 0 {
            return;
        }
        let elapsed = now_millis() - self.prefs.last_library_update;
        if self.prefs.last_library_update > 0 && elapsed < hours as i64 * 3_600_000 {
            return;
        }
        self.prefs.last_library_update = now_millis();
        self.prefs_changed();
        self.core.update_library(&self.prefs, None);
    }

    // -- data refresh -------------------------------------------------------

    pub fn refresh_library(&mut self) {
        self.library.entries = self.core.db.library_entries(&self.core.downloads.index());
        self.library.categories = self.core.db.categories();
        self.library.dirty = false;
    }

    pub fn refresh_updates(&mut self) {
        self.updates.entries = self
            .core
            .db
            .recent_updates(500, &self.core.downloads.index());
        self.updates.dirty = false;
    }

    pub fn refresh_history(&mut self) {
        self.history.entries = self.core.db.recent_history(300, &self.history.query);
        self.history.dirty = false;
    }

    pub fn refresh_details(&mut self) {
        let manga_id = self.details.manga_id;
        self.details.manga = self.core.db.get_manga(manga_id);
        self.details.chapters = self.core.db.chapters_of(manga_id);
        if let Some(manga) = &self.details.manga {
            self.details.notes_buffer = manga.notes.clone();
        }
    }

    pub fn refresh_extensions(&mut self) {
        self.extensions.installed = crate::source::ext::load_installed(&self.core.paths.extensions);
        self.extensions.dirty = false;
    }

    pub fn invalidate_all(&mut self) {
        self.library.dirty = true;
        self.updates.dirty = true;
        self.history.dirty = true;
    }

    // -- images -------------------------------------------------------------

    /// Returns the texture for `url`, kicking off a load the first time.
    pub fn texture(
        &mut self,
        kind: ImageKind,
        url: &str,
        headers: Vec<(String, String)>,
        max_width: u32,
    ) -> Option<Arc<crate::images::PageTexture>> {
        self.texture_of(kind, vec![url.to_string()], headers, max_width)
    }

    /// Same, but with fallback urls. The first is the cache key.
    pub fn texture_of(
        &mut self,
        kind: ImageKind,
        candidates: Vec<String>,
        headers: Vec<(String, String)>,
        max_width: u32,
    ) -> Option<Arc<crate::images::PageTexture>> {
        let key = candidates.first().filter(|url| !url.is_empty()).cloned()?;
        let crop = kind == ImageKind::Page && self.prefs.reader.crop_borders;

        let cache = match kind {
            ImageKind::Cover => &mut self.covers,
            ImageKind::Page => &mut self.page_textures,
        };

        match cache.get(&key) {
            Some(ImageSlot::Ready(texture)) => Some(texture),
            Some(_) => None,
            None => {
                cache.mark_loading(&key);
                self.core
                    .load_image(kind, candidates, headers, max_width, crop);
                None
            }
        }
    }

    pub fn image_failed(&mut self, kind: ImageKind, url: &str) -> bool {
        self.image_error(kind, url).is_some()
    }

    /// Why an image failed, for showing the reason instead of a bare message.
    pub fn image_error(&mut self, kind: ImageKind, url: &str) -> Option<String> {
        let cache = match kind {
            ImageKind::Cover => &mut self.covers,
            ImageKind::Page => &mut self.page_textures,
        };
        match cache.get(url) {
            Some(ImageSlot::Failed(reason)) => Some(reason),
            _ => None,
        }
    }

    /// Image headers required by the source that owns `manga`.
    pub fn source_headers(&self, source_id: Id) -> Vec<(String, String)> {
        self.core
            .sources
            .get(source_id)
            .map(|s| s.image_headers())
            .unwrap_or_default()
    }

    // -- toasts -------------------------------------------------------------

    pub fn toast(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::info(message));
    }

    pub fn toast_error(&mut self, message: impl Into<String>) {
        self.toasts.push(Toast::error(message));
    }

    // -- events -------------------------------------------------------------

    fn handle_events(&mut self, ctx: &egui::Context) {
        for event in self.bus.drain() {
            match event {
                AppEvent::Toast(toast) => self.toasts.push(toast),

                AppEvent::LibraryChanged => {
                    self.invalidate_all();
                    if self
                        .stack
                        .iter()
                        .any(|r| matches!(r, Route::MangaDetails(_)))
                    {
                        self.refresh_details();
                    }
                }
                AppEvent::ExtensionsChanged => self.extensions.dirty = true,

                AppEvent::BrowseLoaded {
                    source,
                    page,
                    result,
                } => self.on_browse_loaded(source, page, result),

                AppEvent::GlobalSearchLoaded { source, result } => {
                    if let Some(slot) = self.browse.results.get_mut(&source) {
                        slot.loading = false;
                        match result {
                            Ok(page) => slot.items = page.mangas,
                            Err(err) => slot.error = Some(err),
                        }
                    }
                }

                AppEvent::DetailsLoaded { manga_id, result } => {
                    if self.details.manga_id == manga_id {
                        self.details.loading = false;
                        if let Err(err) = result {
                            self.details.error = Some(err);
                        }
                        self.refresh_details();
                    }
                }

                AppEvent::ChaptersLoaded {
                    manga_id,
                    new_chapters,
                    result,
                } => {
                    if self.details.manga_id == manga_id {
                        self.details.loading = false;
                        match result {
                            Ok(()) => {
                                self.details.error = None;
                                if new_chapters > 0 {
                                    self.toast(format!("{new_chapters} new chapter(s)"));
                                }
                            }
                            Err(err) => self.details.error = Some(err),
                        }
                        self.refresh_details();
                    }
                    self.library.dirty = true;
                    self.updates.dirty = true;
                }

                AppEvent::PagesLoaded { chapter_id, result } => {
                    if self.reader.appending == Some(chapter_id) {
                        self.reader.appending = None;
                        match result {
                            Ok(pages) if !pages.is_empty() => {
                                self.append_chapter(chapter_id, pages)
                            }
                            // Nothing usable: stop pulling, rather than looping.
                            Ok(_) => self.reader.reached_end = true,
                            Err(err) => {
                                self.reader.reached_end = true;
                                self.toast_error(format!("could not continue: {err}"));
                            }
                        }
                    } else if self.reader.chapter_id == chapter_id {
                        self.reader.loading = false;
                        match result {
                            Ok(pages) => {
                                self.reader.aspects = vec![0.7; pages.len()];
                                self.reader.page_owner = vec![chapter_id; pages.len()];
                                self.reader.segments = vec![ReaderSegment {
                                    chapter_id,
                                    name: self.reader.chapter_name.clone(),
                                    start: 0,
                                    len: pages.len(),
                                }];
                                self.reader.pages = pages;
                                let last = self.reader.pages.len().saturating_sub(1);
                                let restore = self.reader.restore_page.take().unwrap_or(0);
                                self.reader.current = (restore as usize).min(last);
                                self.reader.error = None;
                            }
                            Err(err) => self.reader.error = Some(err),
                        }
                    }
                }

                AppEvent::ImageLoaded { kind, key, result } => {
                    let slot = match result {
                        Ok(image) => {
                            let size = egui::vec2(image.size[0] as f32, image.size[1] as f32);
                            let slices = image
                                .slices
                                .into_iter()
                                .enumerate()
                                .map(|(index, slice)| {
                                    ctx.load_texture(
                                        format!("{key}#{index}"),
                                        slice,
                                        image_texture_options(),
                                    )
                                })
                                .collect();
                            ImageSlot::Ready(Arc::new(crate::images::PageTexture { size, slices }))
                        }
                        Err(err) => ImageSlot::Failed(err),
                    };
                    let cache = match kind {
                        ImageKind::Cover => &mut self.covers,
                        ImageKind::Page => &mut self.page_textures,
                    };
                    // Only keep it if the slot is still wanted; an evicted entry
                    // means the user scrolled away while it was loading.
                    if cache.contains(&key) {
                        cache.insert(&key, slot);
                    }
                }

                AppEvent::DownloadProgress { .. } | AppEvent::DownloadQueueChanged => {
                    self.library.dirty = true;
                    self.updates.dirty = true;
                }

                AppEvent::LibraryUpdateProgress {
                    done,
                    total,
                    current,
                } => self.update_progress = Some((done, total, current)),

                AppEvent::LibraryUpdateFinished {
                    new_chapters,
                    failed,
                } => {
                    self.update_progress = None;
                    self.invalidate_all();
                    let mut message = if new_chapters > 0 {
                        format!("Library updated — {new_chapters} new chapter(s)")
                    } else {
                        "Library updated — nothing new".to_string()
                    };
                    if failed > 0 {
                        message.push_str(&format!(", {failed} failed"));
                    }
                    self.toast(message);
                }

                AppEvent::RepoLoaded { url, result } => {
                    self.extensions.loading = false;
                    match result {
                        Ok(entries) => {
                            self.extensions.available.insert(url, entries);
                        }
                        Err(err) => self.toasts.push(Toast::error(format!(
                            "could not read the repository: {err}"
                        ))),
                    }
                }
            }
        }
    }

    fn on_browse_loaded(&mut self, source: Id, page: u32, result: Result<MangasPage, String>) {
        if self.source_browse.source_id != source {
            return;
        }
        self.source_browse.loading = false;
        match result {
            Ok(mangas_page) => {
                if page <= 1 {
                    self.source_browse.items = mangas_page.mangas;
                } else {
                    self.source_browse.items.extend(mangas_page.mangas);
                }
                self.source_browse.page = page;
                self.source_browse.has_next = mangas_page.has_next_page;
                self.source_browse.error = None;
            }
            Err(err) => self.source_browse.error = Some(err),
        }
    }

    // -- frame --------------------------------------------------------------

    fn refresh_dirty(&mut self) {
        if self.library.dirty {
            self.refresh_library();
        }
        if self.updates.dirty {
            self.refresh_updates();
        }
        if self.history.dirty {
            self.refresh_history();
        }
        if self.extensions.dirty {
            self.refresh_extensions();
        }
    }

    fn global_shortcuts(&mut self, ctx: &egui::Context) {
        let in_reader = matches!(self.current_route(), Some(Route::Reader { .. }));
        ctx.input_mut(|input| {
            if input.consume_key(egui::Modifiers::NONE, egui::Key::Escape) {
                if self.dialog.is_some() {
                    self.dialog = None;
                } else if !self.stack.is_empty() {
                    self.stack.pop();
                }
            }
            if !in_reader {
                if input.consume_key(egui::Modifiers::COMMAND, egui::Key::L) {
                    self.tab = Tab::Library;
                    self.stack.clear();
                }
                if input.consume_key(egui::Modifiers::COMMAND, egui::Key::B) {
                    self.tab = Tab::Browse;
                    self.stack.clear();
                }
            }
        });
    }

    fn draw_toasts(&mut self, ui: &mut egui::Ui) {
        if self.toasts.is_empty() {
            return;
        }
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        for toast in &mut self.toasts {
            toast.remaining -= dt;
        }
        self.toasts.retain(|t| t.remaining > 0.0);
        if self.toasts.is_empty() {
            return;
        }
        // Keep the animation running while a toast is visible.
        ui.ctx().request_repaint();

        let palette = self.palette;
        let screen = ui.ctx().content_rect();
        let mut y = screen.bottom() - 16.0;

        for toast in self.toasts.iter().rev().take(4) {
            let (fill, text_colour) = match toast.kind {
                ToastKind::Info => (palette.elevated, palette.text),
                ToastKind::Error => (palette.error, Color32::WHITE),
            };
            let fade = toast.remaining.min(1.0);

            let area = egui::Area::new(egui::Id::new(("toast", y as i32)))
                .fixed_pos(egui::pos2(screen.left() + 96.0, y - 46.0))
                .order(egui::Order::Foreground);

            area.show(ui.ctx(), |ui| {
                egui::Frame::NONE
                    .fill(fill.gamma_multiply(fade))
                    .corner_radius(palette.corner())
                    .inner_margin(egui::Margin::symmetric(14, 10))
                    .show(ui, |ui| {
                        ui.set_max_width(520.0);
                        ui.label(
                            RichText::new(&toast.message).color(text_colour.gamma_multiply(fade)),
                        );
                    });
            });
            y -= 52.0;
        }
    }

    fn draw_navigation(&mut self, ui: &mut egui::Ui) {
        match self.prefs.nav_style {
            NavStyle::Rail => self.draw_nav_rail(ui, true),
            NavStyle::Compact => self.draw_nav_rail(ui, false),
            NavStyle::Bottom => self.draw_nav_bar(ui),
        }
    }

    fn draw_nav_rail(&mut self, ui: &mut egui::Ui, labelled: bool) {
        let palette = self.palette;
        let width = if labelled {
            palette.space(92.0)
        } else {
            palette.space(64.0)
        };
        let item_style = if labelled {
            widgets::NavItemStyle::Full
        } else {
            widgets::NavItemStyle::IconOnly
        };
        let item_height = palette.space(if labelled { 52.0 } else { 44.0 });

        egui::Panel::left("nav_rail")
            .exact_size(width)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(
                        palette.margin(6.0),
                        palette.margin(14.0),
                    )),
            )
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(2.0);
                    ui.label(
                        RichText::new(if labelled { "mihon" } else { "◆" })
                            .size(15.0)
                            .strong()
                            .color(palette.accent),
                    );
                    ui.add_space(palette.space(14.0));
                });

                // Reserved now, filled once the selected entry has a rectangle.
                let indicator = ui.painter().add(egui::Shape::Noop);
                let mut selected_rect = None;

                for tab in Tab::ALL {
                    let selected = self.tab == tab && self.stack.is_empty();
                    let response = widgets::nav_item(
                        ui,
                        &palette,
                        tab.glyph(),
                        tab.label(),
                        selected,
                        item_style,
                        egui::vec2(ui.available_width(), item_height),
                    );
                    if selected {
                        selected_rect = Some(response.rect.shrink2(egui::vec2(4.0, 2.0)));
                    }
                    if response.clicked() {
                        self.tab = tab;
                        self.stack.clear();
                    }
                }

                if let Some(rect) = selected_rect {
                    let shape =
                        widgets::nav_indicator_shape(ui, &palette, rect, self.prefs.animations);
                    ui.painter().set(indicator, shape);
                }

                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    ui.add_space(6.0);
                    let queued = self.core.downloads.queue_len();
                    let label = if queued > 0 {
                        format!("📥 {queued}")
                    } else {
                        "📥".to_string()
                    };
                    if widgets::nav_item(
                        ui,
                        &palette,
                        &label,
                        "Downloads",
                        false,
                        item_style,
                        egui::vec2(ui.available_width(), item_height),
                    )
                    .clicked()
                    {
                        self.push(Route::Downloads);
                    }
                    if let Some((done, total, _)) = &self.update_progress {
                        ui.add_space(4.0);
                        if labelled {
                            ui.label(
                                RichText::new(format!("{done}/{total}"))
                                    .size(10.5)
                                    .color(palette.text_dim),
                            );
                        }
                        ui.add(
                            egui::ProgressBar::new(*done as f32 / *total as f32)
                                .desired_height(4.0),
                        );
                    }
                });
            });
    }

    fn draw_nav_bar(&mut self, ui: &mut egui::Ui) {
        let palette = self.palette;
        let height = palette.space(58.0);
        egui::Panel::bottom("nav_bar")
            .exact_size(height)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::symmetric(
                        palette.margin(8.0),
                        palette.margin(5.0),
                    )),
            )
            .show(ui, |ui| {
                if let Some((done, total, _)) = &self.update_progress {
                    ui.add(
                        egui::ProgressBar::new(*done as f32 / *total as f32).desired_height(3.0),
                    );
                }

                let indicator = ui.painter().add(egui::Shape::Noop);
                let mut selected_rect = None;

                // Six equal slots: the five tabs plus the download queue. The
                // width is worked out here and handed to each entry, because an
                // entry sizing itself from `available_width` inside a horizontal
                // layout would swallow the whole row.
                let spacing = ui.spacing().item_spacing.x;
                let slot = ((ui.available_width() - spacing * 5.0) / 6.0).max(24.0);
                let size = egui::vec2(slot, ui.available_height());

                ui.horizontal(|ui| {
                    for tab in Tab::ALL {
                        let selected = self.tab == tab && self.stack.is_empty();
                        let response = widgets::nav_item(
                            ui,
                            &palette,
                            tab.glyph(),
                            tab.label(),
                            selected,
                            widgets::NavItemStyle::Horizontal,
                            size,
                        );
                        if selected {
                            selected_rect = Some(response.rect.shrink2(egui::vec2(2.0, 2.0)));
                        }
                        if response.clicked() {
                            self.tab = tab;
                            self.stack.clear();
                        }
                    }

                    let queued = self.core.downloads.queue_len();
                    let label = if queued > 0 {
                        format!("📥 {queued}")
                    } else {
                        "📥".to_string()
                    };
                    if widgets::nav_item(
                        ui,
                        &palette,
                        &label,
                        "Downloads",
                        false,
                        widgets::NavItemStyle::Horizontal,
                        size,
                    )
                    .clicked()
                    {
                        self.push(Route::Downloads);
                    }
                });

                if let Some(rect) = selected_rect {
                    let shape =
                        widgets::nav_indicator_shape(ui, &palette, rect, self.prefs.animations);
                    ui.painter().set(indicator, shape);
                }
            });
    }

    fn draw_body(&mut self, ui: &mut egui::Ui) {
        if self
            .capture
            .as_ref()
            .map(|c| c.is_glyph_probe())
            .unwrap_or(false)
        {
            capture::glyph_probe(self, ui);
            return;
        }
        match self.stack.last().cloned() {
            Some(Route::Reader { .. }) => reader::show(self, ui),
            Some(Route::MangaDetails(_)) => manga::show(self, ui),
            Some(Route::SourceBrowse(_)) => source_browse::show(self, ui),
            Some(Route::Downloads) => downloads::show(self, ui),
            Some(Route::Categories) => categories::show(self, ui),
            Some(Route::Extensions) => extensions::show(self, ui),
            Some(Route::Settings(page)) => settings::show(self, ui, page),
            Some(Route::Statistics) => settings::show_statistics(self, ui),
            None => match self.tab {
                Tab::Library => library::show(self, ui),
                Tab::Updates => updates::show(self, ui),
                Tab::History => history::show(self, ui),
                Tab::Browse => browse::show(self, ui),
                Tab::More => settings::show_more(self, ui),
            },
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        if self.first_frame {
            self.bus.sender.attach_context(&ctx);
            self.core.refresh_local_source(&self.prefs);
            self.refresh_library();
            self.maybe_auto_update();
            self.first_frame = false;
        }

        self.palette = theme::apply(&ctx, &self.prefs);
        self.handle_events(&ctx);
        self.refresh_dirty();
        self.global_shortcuts(&ctx);

        // The reader takes the whole window; everything else keeps the rail.
        let fullscreen = matches!(self.current_route(), Some(Route::Reader { .. }));
        if !fullscreen {
            self.draw_navigation(ui);
        }

        let palette = self.palette;
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(palette.background))
            .show(ui, |ui| {
                self.draw_body(ui);
            });

        widgets::dialogs(self, &ctx);
        self.draw_toasts(ui);
        capture::tick(self, &ctx);
        self.flush_prefs(false);
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.flush_prefs(true);
    }
}
