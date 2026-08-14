//! Domain model, transposed from Mihon's `tachiyomi.domain.*` Kotlin data classes.
//!
//! Field names and the bit-flag layouts for `chapter_flags` / `viewer_flags` are kept
//! identical to upstream so that behaviour (and backups) stay recognisable.

use serde::{Deserialize, Serialize};

pub type Id = i64;

/// Milliseconds since the Unix epoch, like upstream.
pub fn now_millis() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

// ---------------------------------------------------------------------------
// Manga
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[derive(Default)]
pub enum MangaStatus {
    #[default]
    Unknown,
    Ongoing,
    Completed,
    Licensed,
    PublishingFinished,
    Cancelled,
    OnHiatus,
}

impl MangaStatus {
    pub fn from_code(code: i64) -> Self {
        match code {
            1 => Self::Ongoing,
            2 => Self::Completed,
            3 => Self::Licensed,
            4 => Self::PublishingFinished,
            5 => Self::Cancelled,
            6 => Self::OnHiatus,
            _ => Self::Unknown,
        }
    }

    pub fn code(self) -> i64 {
        match self {
            Self::Unknown => 0,
            Self::Ongoing => 1,
            Self::Completed => 2,
            Self::Licensed => 3,
            Self::PublishingFinished => 4,
            Self::Cancelled => 5,
            Self::OnHiatus => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "Unknown",
            Self::Ongoing => "Ongoing",
            Self::Completed => "Completed",
            Self::Licensed => "Licensed",
            Self::PublishingFinished => "Publishing finished",
            Self::Cancelled => "Cancelled",
            Self::OnHiatus => "On hiatus",
        }
    }

    /// Upstream stops fetching new chapters for series that can no longer receive any.
    pub fn is_finished(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Licensed)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[derive(Default)]
pub enum UpdateStrategy {
    #[default]
    AlwaysUpdate,
    OnlyFetchOnce,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manga {
    pub id: Id,
    pub source: Id,
    pub url: String,
    pub title: String,

    pub artist: Option<String>,
    pub author: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub genre: Option<Vec<String>>,
    #[serde(default)]
    pub status: MangaStatus,
    pub thumbnail_url: Option<String>,

    #[serde(default)]
    pub favorite: bool,
    /// Timestamp of the latest chapter known to us.
    #[serde(default)]
    pub last_update: i64,
    #[serde(default)]
    pub next_update: i64,
    /// Days between update checks; negative means "computed automatically".
    #[serde(default)]
    pub fetch_interval: i32,
    #[serde(default)]
    pub date_added: i64,

    #[serde(default)]
    pub viewer_flags: i64,
    #[serde(default)]
    pub chapter_flags: i64,
    #[serde(default)]
    pub cover_last_modified: i64,

    #[serde(default)]
    pub update_strategy: UpdateStrategy,
    /// False until the details have been fetched from the source at least once.
    #[serde(default)]
    pub initialized: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub last_modified_at: i64,
    #[serde(default)]
    pub version: i64,
}

impl Manga {
    pub fn new(source: Id, url: String, title: String) -> Self {
        Self {
            id: 0,
            source,
            url,
            title,
            artist: None,
            author: None,
            description: None,
            genre: None,
            status: MangaStatus::Unknown,
            thumbnail_url: None,
            favorite: false,
            last_update: 0,
            next_update: 0,
            fetch_interval: 0,
            date_added: 0,
            viewer_flags: 0,
            chapter_flags: 0,
            cover_last_modified: 0,
            update_strategy: UpdateStrategy::AlwaysUpdate,
            initialized: false,
            notes: String::new(),
            last_modified_at: now_millis(),
            version: 0,
        }
    }

    pub fn author_artist(&self) -> Option<String> {
        match (&self.author, &self.artist) {
            (Some(a), Some(b)) if a != b => Some(format!("{a}, {b}")),
            (Some(a), _) => Some(a.clone()),
            (None, Some(b)) => Some(b.clone()),
            (None, None) => None,
        }
    }

    // -- chapter_flags accessors, mirroring Manga.kt --------------------------

    pub fn chapter_sort_mode(&self) -> ChapterSortMode {
        ChapterSortMode::from_flag(self.chapter_flags & flags::CHAPTER_SORTING_MASK)
    }

    pub fn set_chapter_sort_mode(&mut self, mode: ChapterSortMode) {
        self.chapter_flags = (self.chapter_flags & !flags::CHAPTER_SORTING_MASK) | mode.flag();
    }

    /// True when chapters are listed oldest-first.
    pub fn sort_descending(&self) -> bool {
        self.chapter_flags & flags::CHAPTER_SORT_DIR_MASK == flags::CHAPTER_SORT_DESC
    }

    pub fn set_sort_descending(&mut self, descending: bool) {
        let bit = if descending {
            flags::CHAPTER_SORT_DESC
        } else {
            flags::CHAPTER_SORT_ASC
        };
        self.chapter_flags = (self.chapter_flags & !flags::CHAPTER_SORT_DIR_MASK) | bit;
    }

    pub fn unread_filter(&self) -> TriState {
        TriState::from_bits(
            self.chapter_flags,
            flags::CHAPTER_SHOW_UNREAD,
            flags::CHAPTER_SHOW_READ,
        )
    }

    pub fn set_unread_filter(&mut self, state: TriState) {
        self.chapter_flags = state.apply(
            self.chapter_flags,
            flags::CHAPTER_SHOW_UNREAD,
            flags::CHAPTER_SHOW_READ,
        );
    }

    pub fn downloaded_filter(&self) -> TriState {
        TriState::from_bits(
            self.chapter_flags,
            flags::CHAPTER_SHOW_DOWNLOADED,
            flags::CHAPTER_SHOW_NOT_DOWNLOADED,
        )
    }

    pub fn set_downloaded_filter(&mut self, state: TriState) {
        self.chapter_flags = state.apply(
            self.chapter_flags,
            flags::CHAPTER_SHOW_DOWNLOADED,
            flags::CHAPTER_SHOW_NOT_DOWNLOADED,
        );
    }

    pub fn bookmarked_filter(&self) -> TriState {
        TriState::from_bits(
            self.chapter_flags,
            flags::CHAPTER_SHOW_BOOKMARKED,
            flags::CHAPTER_SHOW_NOT_BOOKMARKED,
        )
    }

    pub fn set_bookmarked_filter(&mut self, state: TriState) {
        self.chapter_flags = state.apply(
            self.chapter_flags,
            flags::CHAPTER_SHOW_BOOKMARKED,
            flags::CHAPTER_SHOW_NOT_BOOKMARKED,
        );
    }

    pub fn display_chapter_number(&self) -> bool {
        self.chapter_flags & flags::CHAPTER_DISPLAY_MASK == flags::CHAPTER_DISPLAY_NUMBER
    }

    pub fn set_display_chapter_number(&mut self, by_number: bool) {
        let bit = if by_number {
            flags::CHAPTER_DISPLAY_NUMBER
        } else {
            flags::CHAPTER_DISPLAY_NAME
        };
        self.chapter_flags = (self.chapter_flags & !flags::CHAPTER_DISPLAY_MASK) | bit;
    }

    /// Per-manga reading mode override; `None` means "follow the global default".
    pub fn reading_mode(&self) -> Option<ReadingMode> {
        let raw = self.viewer_flags & flags::READING_MODE_MASK;
        if raw == 0 {
            None
        } else {
            ReadingMode::from_flag(raw)
        }
    }

    pub fn set_reading_mode(&mut self, mode: Option<ReadingMode>) {
        let bits = mode.map(|m| m.flag()).unwrap_or(0);
        self.viewer_flags = (self.viewer_flags & !flags::READING_MODE_MASK) | bits;
    }
}

pub mod flags {
    // Chapter sorting
    pub const CHAPTER_SORTING_MASK: i64 = 0x0000_0300;
    pub const CHAPTER_SORTING_SOURCE: i64 = 0x0000_0000;
    pub const CHAPTER_SORTING_NUMBER: i64 = 0x0000_0100;
    pub const CHAPTER_SORTING_UPLOAD_DATE: i64 = 0x0000_0200;
    pub const CHAPTER_SORTING_ALPHABET: i64 = 0x0000_0300;

    // Sort direction
    pub const CHAPTER_SORT_DIR_MASK: i64 = 0x0000_0001;
    pub const CHAPTER_SORT_DESC: i64 = 0x0000_0000;
    pub const CHAPTER_SORT_ASC: i64 = 0x0000_0001;

    // Tri-state chapter filters
    pub const CHAPTER_SHOW_UNREAD: i64 = 0x0000_0002;
    pub const CHAPTER_SHOW_READ: i64 = 0x0000_0004;
    pub const CHAPTER_SHOW_DOWNLOADED: i64 = 0x0000_0008;
    pub const CHAPTER_SHOW_NOT_DOWNLOADED: i64 = 0x0000_0010;
    pub const CHAPTER_SHOW_BOOKMARKED: i64 = 0x0000_0020;
    pub const CHAPTER_SHOW_NOT_BOOKMARKED: i64 = 0x0000_0040;

    // Chapter list display
    pub const CHAPTER_DISPLAY_MASK: i64 = 0x0010_0000;
    pub const CHAPTER_DISPLAY_NAME: i64 = 0x0000_0000;
    pub const CHAPTER_DISPLAY_NUMBER: i64 = 0x0010_0000;

    // Reader
    pub const READING_MODE_MASK: i64 = 0x0000_0007;
}

/// A tri-state toggle: off, "include only", "exclude". Mihon's `TriState`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum TriState {
    #[default]
    Disabled,
    EnabledIs,
    EnabledNot,
}

impl TriState {
    pub fn next(self) -> Self {
        match self {
            Self::Disabled => Self::EnabledIs,
            Self::EnabledIs => Self::EnabledNot,
            Self::EnabledNot => Self::Disabled,
        }
    }

    pub fn is_enabled(self) -> bool {
        !matches!(self, Self::Disabled)
    }

    fn from_bits(flags: i64, is_bit: i64, not_bit: i64) -> Self {
        if flags & is_bit != 0 {
            Self::EnabledIs
        } else if flags & not_bit != 0 {
            Self::EnabledNot
        } else {
            Self::Disabled
        }
    }

    fn apply(self, flags: i64, is_bit: i64, not_bit: i64) -> i64 {
        let cleared = flags & !(is_bit | not_bit);
        match self {
            Self::Disabled => cleared,
            Self::EnabledIs => cleared | is_bit,
            Self::EnabledNot => cleared | not_bit,
        }
    }

    /// Applies the filter to a boolean property. Returns whether the item survives.
    pub fn matches(self, value: bool) -> bool {
        match self {
            Self::Disabled => true,
            Self::EnabledIs => value,
            Self::EnabledNot => !value,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChapterSortMode {
    SourceOrder,
    ChapterNumber,
    UploadDate,
    Alphabet,
}

impl ChapterSortMode {
    pub fn from_flag(flag: i64) -> Self {
        match flag {
            flags::CHAPTER_SORTING_NUMBER => Self::ChapterNumber,
            flags::CHAPTER_SORTING_UPLOAD_DATE => Self::UploadDate,
            flags::CHAPTER_SORTING_ALPHABET => Self::Alphabet,
            _ => Self::SourceOrder,
        }
    }

    pub fn flag(self) -> i64 {
        match self {
            Self::SourceOrder => flags::CHAPTER_SORTING_SOURCE,
            Self::ChapterNumber => flags::CHAPTER_SORTING_NUMBER,
            Self::UploadDate => flags::CHAPTER_SORTING_UPLOAD_DATE,
            Self::Alphabet => flags::CHAPTER_SORTING_ALPHABET,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::SourceOrder => "By source",
            Self::ChapterNumber => "By chapter number",
            Self::UploadDate => "By upload date",
            Self::Alphabet => "Alphabetically",
        }
    }

    pub const ALL: [Self; 4] = [
        Self::SourceOrder,
        Self::ChapterNumber,
        Self::UploadDate,
        Self::Alphabet,
    ];
}

// ---------------------------------------------------------------------------
// Chapter
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Chapter {
    pub id: Id,
    pub manga_id: Id,
    pub url: String,
    pub name: String,

    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub bookmark: bool,
    #[serde(default)]
    pub last_page_read: i64,
    /// Total page count, cached once the chapter has been opened.
    #[serde(default)]
    pub page_count: i64,

    #[serde(default)]
    pub date_fetch: i64,
    #[serde(default)]
    pub date_upload: i64,
    /// Position in the source's own ordering.
    #[serde(default)]
    pub source_order: i64,
    /// `-1.0` when the number could not be parsed out of the name.
    #[serde(default = "minus_one")]
    pub chapter_number: f64,
    pub scanlator: Option<String>,

    #[serde(default)]
    pub last_modified_at: i64,
    #[serde(default)]
    pub version: i64,
}

fn minus_one() -> f64 {
    -1.0
}

impl Chapter {
    pub fn new(manga_id: Id, url: String, name: String) -> Self {
        Self {
            id: 0,
            manga_id,
            url,
            name,
            read: false,
            bookmark: false,
            last_page_read: 0,
            page_count: 0,
            date_fetch: now_millis(),
            date_upload: 0,
            source_order: 0,
            chapter_number: -1.0,
            scanlator: None,
            last_modified_at: now_millis(),
            version: 0,
        }
    }

    pub fn is_recognized_number(&self) -> bool {
        self.chapter_number >= 0.0
    }

    /// The label shown in chapter lists, honouring the manga's display flag.
    pub fn display_name(&self, by_number: bool) -> String {
        if by_number && self.is_recognized_number() {
            format!("Chapter {}", format_chapter_number(self.chapter_number))
        } else {
            self.name.clone()
        }
    }
}

/// Ascending reading order.
///
/// Chapters whose number could not be parsed out of the title ("Bonus",
/// "Oneshot", ...) carry `-1`, which would otherwise sort them *before*
/// chapter 0 and make "continue reading" offer them forever. They go last
/// instead, ordered by their position in the source listing (newest first, so
/// the comparison is reversed).
pub fn reading_order(a: &Chapter, b: &Chapter) -> std::cmp::Ordering {
    match (a.is_recognized_number(), b.is_recognized_number()) {
        (true, true) => a
            .chapter_number
            .total_cmp(&b.chapter_number)
            .then(b.source_order.cmp(&a.source_order)),
        (true, false) => std::cmp::Ordering::Less,
        (false, true) => std::cmp::Ordering::Greater,
        (false, false) => b.source_order.cmp(&a.source_order),
    }
}

pub fn format_chapter_number(n: f64) -> String {
    if n < 0.0 {
        return "-".into();
    }
    if (n - n.round()).abs() < f64::EPSILON {
        format!("{}", n.round() as i64)
    } else {
        // Trim trailing zeros: 12.50 -> 12.5
        let s = format!("{n:.2}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

// ---------------------------------------------------------------------------
// Category
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: Id,
    pub name: String,
    pub order: i64,
    #[serde(default)]
    pub flags: i64,
    #[serde(default)]
    pub hidden: bool,
}

impl Category {
    /// Upstream reserves id 0 for the implicit "Default" category.
    pub const DEFAULT_ID: Id = 0;

    pub fn is_system(&self) -> bool {
        self.id == Self::DEFAULT_ID
    }
}

// ---------------------------------------------------------------------------
// History
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct History {
    pub id: Id,
    pub chapter_id: Id,
    pub manga_id: Id,
    pub read_at: i64,
    /// Accumulated reading time in milliseconds.
    #[serde(default)]
    pub time_read: i64,
}

// ---------------------------------------------------------------------------
// Tracking
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: Id,
    pub manga_id: Id,
    pub tracker_id: i64,
    pub remote_id: i64,
    pub library_id: Option<i64>,
    pub title: String,
    #[serde(default)]
    pub last_chapter_read: f64,
    #[serde(default)]
    pub total_chapters: i64,
    #[serde(default)]
    pub status: i64,
    #[serde(default)]
    pub score: f64,
    #[serde(default)]
    pub remote_url: String,
    #[serde(default)]
    pub start_date: i64,
    #[serde(default)]
    pub finish_date: i64,
}

// ---------------------------------------------------------------------------
// Library view models
// ---------------------------------------------------------------------------

/// A library row: the manga plus the aggregates the library screen needs.
#[derive(Debug, Clone)]
pub struct LibraryEntry {
    pub manga: Manga,
    pub category_ids: Vec<Id>,
    pub total_chapters: i64,
    pub unread_count: i64,
    pub downloaded_count: i64,
    pub has_started: bool,
    pub bookmark_count: i64,
    pub latest_upload: i64,
    pub last_read: i64,
    pub chapter_fetch_date: i64,
    pub is_tracked: bool,
}

impl LibraryEntry {
    pub fn read_count(&self) -> i64 {
        self.total_chapters - self.unread_count
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LibrarySort {
    #[default]
    Alphabetical,
    LastRead,
    LastUpdate,
    UnreadCount,
    TotalChapters,
    LatestChapter,
    ChapterFetchDate,
    DateAdded,
    Random,
}

impl LibrarySort {
    pub const ALL: [Self; 9] = [
        Self::Alphabetical,
        Self::LastRead,
        Self::LastUpdate,
        Self::UnreadCount,
        Self::TotalChapters,
        Self::LatestChapter,
        Self::ChapterFetchDate,
        Self::DateAdded,
        Self::Random,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Alphabetical => "Alphabetically",
            Self::LastRead => "Last read",
            Self::LastUpdate => "Last checked",
            Self::UnreadCount => "Unread count",
            Self::TotalChapters => "Total chapters",
            Self::LatestChapter => "Latest chapter",
            Self::ChapterFetchDate => "Chapter fetch date",
            Self::DateAdded => "Date added",
            Self::Random => "Random",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum LibraryDisplayMode {
    #[default]
    CompactGrid,
    ComfortableGrid,
    CoverOnlyGrid,
    List,
}

impl LibraryDisplayMode {
    pub const ALL: [Self; 4] = [
        Self::CompactGrid,
        Self::ComfortableGrid,
        Self::CoverOnlyGrid,
        Self::List,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::CompactGrid => "Compact grid",
            Self::ComfortableGrid => "Comfortable grid",
            Self::CoverOnlyGrid => "Cover-only grid",
            Self::List => "List",
        }
    }

    pub fn is_grid(self) -> bool {
        !matches!(self, Self::List)
    }
}

/// The set of tri-state toggles in the library filter sheet.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
pub struct LibraryFilters {
    #[serde(default)]
    pub downloaded: TriState,
    #[serde(default)]
    pub unread: TriState,
    #[serde(default)]
    pub started: TriState,
    #[serde(default)]
    pub bookmarked: TriState,
    #[serde(default)]
    pub completed: TriState,
    #[serde(default)]
    pub tracked: TriState,
}

impl LibraryFilters {
    pub fn any_enabled(&self) -> bool {
        [
            self.downloaded,
            self.unread,
            self.started,
            self.bookmarked,
            self.completed,
            self.tracked,
        ]
        .iter()
        .any(|s| s.is_enabled())
    }

    pub fn matches(&self, entry: &LibraryEntry) -> bool {
        self.downloaded.matches(entry.downloaded_count > 0)
            && self.unread.matches(entry.unread_count > 0)
            && self.started.matches(entry.has_started)
            && self.bookmarked.matches(entry.bookmark_count > 0)
            && self
                .completed
                .matches(entry.manga.status == MangaStatus::Completed)
            && self.tracked.matches(entry.is_tracked)
    }
}

// ---------------------------------------------------------------------------
// Reader settings
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReadingMode {
    LeftToRight,
    #[default]
    RightToLeft,
    Vertical,
    Webtoon,
    ContinuousVertical,
    /// Like webtoon, but the next chapter is appended as you reach the end, so
    /// a series can be read in one uninterrupted scroll.
    Infinite,
}

impl ReadingMode {
    pub const ALL: [Self; 6] = [
        Self::LeftToRight,
        Self::RightToLeft,
        Self::Vertical,
        Self::Webtoon,
        Self::ContinuousVertical,
        Self::Infinite,
    ];

    pub fn from_flag(flag: i64) -> Option<Self> {
        match flag {
            1 => Some(Self::LeftToRight),
            2 => Some(Self::RightToLeft),
            3 => Some(Self::Vertical),
            4 => Some(Self::Webtoon),
            5 => Some(Self::ContinuousVertical),
            6 => Some(Self::Infinite),
            _ => None,
        }
    }

    pub fn flag(self) -> i64 {
        match self {
            Self::LeftToRight => 1,
            Self::RightToLeft => 2,
            Self::Vertical => 3,
            Self::Webtoon => 4,
            Self::ContinuousVertical => 5,
            Self::Infinite => 6,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::LeftToRight => "Left to right",
            Self::RightToLeft => "Right to left",
            Self::Vertical => "Vertical",
            Self::Webtoon => "Webtoon",
            Self::ContinuousVertical => "Continuous vertical",
            Self::Infinite => "Infinite scroll (all chapters)",
        }
    }

    /// Webtoon-style modes scroll continuously; pager modes show discrete pages.
    pub fn is_continuous(self) -> bool {
        matches!(
            self,
            Self::Webtoon | Self::ContinuousVertical | Self::Infinite
        )
    }

    /// Whether reaching the end should pull the following chapter in.
    pub fn is_infinite(self) -> bool {
        matches!(self, Self::Infinite)
    }

    pub fn is_horizontal(self) -> bool {
        matches!(self, Self::LeftToRight | Self::RightToLeft)
    }

    /// Right-to-left inverts what "next page" means.
    pub fn is_rtl(self) -> bool {
        matches!(self, Self::RightToLeft)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReaderBackground {
    White,
    Gray,
    #[default]
    Black,
    FollowTheme,
}

impl ReaderBackground {
    pub const ALL: [Self; 4] = [Self::White, Self::Gray, Self::Black, Self::FollowTheme];

    pub fn label(self) -> &'static str {
        match self {
            Self::White => "White",
            Self::Gray => "Gray",
            Self::Black => "Black",
            Self::FollowTheme => "Follow theme",
        }
    }
}

/// How a page is fitted into the viewport.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImageScaleType {
    #[default]
    FitScreen,
    StretchToFit,
    FitWidth,
    FitHeight,
    OriginalSize,
    SmartFit,
}

impl ImageScaleType {
    pub const ALL: [Self; 6] = [
        Self::FitScreen,
        Self::StretchToFit,
        Self::FitWidth,
        Self::FitHeight,
        Self::OriginalSize,
        Self::SmartFit,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::FitScreen => "Fit screen",
            Self::StretchToFit => "Stretch",
            Self::FitWidth => "Fit width",
            Self::FitHeight => "Fit height",
            Self::OriginalSize => "Original size",
            Self::SmartFit => "Smart fit",
        }
    }
}

// ---------------------------------------------------------------------------
// Misc
// ---------------------------------------------------------------------------

/// One item in the Updates tab: a chapter plus enough of its manga to render it.
#[derive(Debug, Clone)]
pub struct UpdatesEntry {
    pub chapter: Chapter,
    pub manga_id: Id,
    pub manga_title: String,
    pub cover_url: Option<String>,
    pub source: Id,
    pub downloaded: bool,
}

/// One item in the History tab.
#[derive(Debug, Clone)]
pub struct HistoryEntry {
    pub history: History,
    pub chapter: Chapter,
    pub manga_id: Id,
    pub manga_title: String,
    pub cover_url: Option<String>,
    pub source: Id,
}
