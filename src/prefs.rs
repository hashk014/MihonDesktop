//! User preferences, mirroring Mihon's preference screens.
//!
//! Everything lives in a single JSON document next to the database; it is loaded
//! once at startup and written back (debounced) whenever a setting changes.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::model::{
    ImageScaleType, LibraryDisplayMode, LibraryFilters, LibrarySort, ReaderBackground, ReadingMode,
};

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
pub struct AppPaths {
    pub root: PathBuf,
    pub database: PathBuf,
    pub prefs: PathBuf,
    pub covers: PathBuf,
    pub pages: PathBuf,
    pub downloads: PathBuf,
    pub extensions: PathBuf,
    pub local_source: PathBuf,
    pub backups: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        // An explicit override enables a portable install (and lets a second
        // instance run against a copy, since the database takes a lock).
        if let Ok(dir) = std::env::var("MIHON_DATA_DIR")
            && !dir.trim().is_empty()
        {
            return Self::from_root(PathBuf::from(dir));
        }
        let root = directories::ProjectDirs::from("app", "Mihon", "MihonDesktop")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("./mihon-data"));
        Self::from_root(root)
    }

    pub fn from_root(root: PathBuf) -> Self {
        Self {
            database: root.join("library.redb"),
            prefs: root.join("preferences.json"),
            covers: root.join("cache/covers"),
            pages: root.join("cache/pages"),
            downloads: root.join("downloads"),
            extensions: root.join("extensions"),
            local_source: root.join("local"),
            backups: root.join("backups"),
            root,
        }
    }

    /// Creates every directory the app writes into. Called once at startup.
    pub fn ensure(&self) -> std::io::Result<()> {
        for dir in [
            &self.root,
            &self.covers,
            &self.pages,
            &self.downloads,
            &self.extensions,
            &self.local_source,
            &self.backups,
        ] {
            std::fs::create_dir_all(dir)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Appearance
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ThemeMode {
    Light,
    #[default]
    Dark,
    /// Follow whatever the desktop reports.
    System,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::Light, Self::Dark, Self::System];

    pub fn label(self) -> &'static str {
        match self {
            Self::Light => "Light",
            Self::Dark => "Dark",
            Self::System => "System",
        }
    }

    /// Resolves to "is dark", given what the desktop reports (if anything).
    pub fn is_dark(self, system_is_dark: Option<bool>) -> bool {
        match self {
            Self::Light => false,
            Self::Dark => true,
            // A desktop that does not answer is far more likely to be one this
            // app is read on at night than a bright one.
            Self::System => system_is_dark.unwrap_or(true),
        }
    }
}

/// The named palettes Mihon ships with, plus a few of our own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AppTheme {
    #[default]
    Default,
    Midnight,
    Green,
    Strawberry,
    Tako,
    Tealturquoise,
    Yotsuba,
    Lavender,
    Nord,
    TidalWave,
    Monochrome,
}

impl AppTheme {
    pub const ALL: [Self; 11] = [
        Self::Default,
        Self::Midnight,
        Self::Green,
        Self::Strawberry,
        Self::Tako,
        Self::Tealturquoise,
        Self::Yotsuba,
        Self::Lavender,
        Self::Nord,
        Self::TidalWave,
        Self::Monochrome,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Default => "Default",
            Self::Midnight => "Midnight Dusk",
            Self::Green => "Green Apple",
            Self::Strawberry => "Strawberry Daiquiri",
            Self::Tako => "Tako",
            Self::Tealturquoise => "Teal & Turquoise",
            Self::Yotsuba => "Yotsuba",
            Self::Lavender => "Lavender",
            Self::Nord => "Nord",
            Self::TidalWave => "Tidal Wave",
            Self::Monochrome => "Monochrome",
        }
    }

    /// (accent, accent_variant) as sRGB bytes.
    pub fn accent(self) -> ([u8; 3], [u8; 3]) {
        match self {
            Self::Default => ([0x8b, 0x7a, 0xff], [0x6c, 0x5b, 0xd6]),
            Self::Midnight => ([0xf0, 0x69, 0x92], [0xc2, 0x4d, 0x72]),
            Self::Green => ([0x4c, 0xaf, 0x50], [0x38, 0x8e, 0x3c]),
            Self::Strawberry => ([0xed, 0x50, 0x6a], [0xc0, 0x3a, 0x52]),
            Self::Tako => ([0xd8, 0xac, 0x59], [0xb0, 0x8b, 0x42]),
            Self::Tealturquoise => ([0x00, 0xa8, 0xa8], [0x00, 0x86, 0x86]),
            Self::Yotsuba => ([0xf5, 0x8f, 0x3c], [0xc9, 0x71, 0x2c]),
            Self::Lavender => ([0xc4, 0x9b, 0xf0], [0x9c, 0x74, 0xc9]),
            Self::Nord => ([0x88, 0xc0, 0xd0], [0x5e, 0x81, 0xac]),
            Self::TidalWave => ([0x3d, 0x9b, 0xe9], [0x2c, 0x74, 0xb3]),
            Self::Monochrome => ([0xb4, 0xb8, 0xc2], [0x74, 0x79, 0x85]),
        }
    }
}

/// How tightly the interface is packed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum Density {
    Compact,
    #[default]
    Cozy,
    Comfortable,
}

impl Density {
    pub const ALL: [Self; 3] = [Self::Compact, Self::Cozy, Self::Comfortable];

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "Compact",
            Self::Cozy => "Cozy",
            Self::Comfortable => "Comfortable",
        }
    }

    /// Multiplies every gap, padding and margin. One value keeps the three
    /// densities proportional instead of drifting apart control by control.
    pub fn scale(self) -> f32 {
        match self {
            Self::Compact => 0.7,
            Self::Cozy => 1.0,
            Self::Comfortable => 1.35,
        }
    }
}

/// Where the primary navigation lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum NavStyle {
    /// Icons with labels down the left edge.
    #[default]
    Rail,
    /// Icons only, half the width.
    Compact,
    /// A phone-style bar along the bottom.
    Bottom,
}

impl NavStyle {
    pub const ALL: [Self; 3] = [Self::Rail, Self::Compact, Self::Bottom];

    pub fn label(self) -> &'static str {
        match self {
            Self::Rail => "Rail",
            Self::Compact => "Compact",
            Self::Bottom => "Bottom bar",
        }
    }
}

/// How cards and panels separate themselves from the background.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CardStyle {
    /// Fill only.
    #[default]
    Flat,
    /// A hairline border, no fill difference.
    Outlined,
    /// Fill plus a soft drop shadow.
    Elevated,
}

impl CardStyle {
    pub const ALL: [Self; 3] = [Self::Flat, Self::Outlined, Self::Elevated];

    pub fn label(self) -> &'static str {
        match self {
            Self::Flat => "Flat",
            Self::Outlined => "Outlined",
            Self::Elevated => "Elevated",
        }
    }
}

// ---------------------------------------------------------------------------
// Library
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LibraryPrefs {
    #[serde(default)]
    pub display_mode: LibraryDisplayMode,
    #[serde(default = "default_columns")]
    pub columns: u32,
    #[serde(default)]
    pub sort: LibrarySort,
    #[serde(default = "yes")]
    pub sort_ascending: bool,
    #[serde(default)]
    pub filters: LibraryFilters,

    #[serde(default = "yes")]
    pub badge_unread: bool,
    #[serde(default = "yes")]
    pub badge_downloaded: bool,
    #[serde(default)]
    pub badge_local: bool,
    #[serde(default)]
    pub badge_language: bool,
    #[serde(default = "yes")]
    pub show_continue_button: bool,

    /// Hours between automatic library updates; 0 disables them.
    #[serde(default = "default_update_interval")]
    pub update_interval_hours: u32,
    /// Skip entries that already have unread chapters waiting.
    #[serde(default)]
    pub skip_entries_with_unread: bool,
    /// Skip entries whose chapters have not been started.
    #[serde(default)]
    pub skip_unstarted_entries: bool,
    /// Skip series that can no longer receive chapters.
    #[serde(default = "yes")]
    pub skip_completed_entries: bool,
    /// Category ids to include in automatic updates; empty means "all".
    #[serde(default)]
    pub update_categories: Vec<i64>,
    #[serde(default)]
    pub default_category: i64,
    /// Ask which categories to file a manga into when adding it.
    #[serde(default)]
    pub prompt_for_category: bool,
    /// Download new chapters as soon as an update finds them.
    #[serde(default)]
    pub download_new_chapters: bool,
}

fn default_columns() -> u32 {
    6
}
fn default_update_interval() -> u32 {
    24
}
fn yes() -> bool {
    true
}

impl Default for LibraryPrefs {
    fn default() -> Self {
        Self {
            display_mode: LibraryDisplayMode::default(),
            columns: default_columns(),
            sort: LibrarySort::default(),
            sort_ascending: true,
            filters: LibraryFilters::default(),
            badge_unread: true,
            badge_downloaded: true,
            badge_local: false,
            badge_language: false,
            show_continue_button: true,
            update_interval_hours: default_update_interval(),
            skip_entries_with_unread: false,
            skip_unstarted_entries: false,
            skip_completed_entries: true,
            update_categories: Vec::new(),
            default_category: -1,
            prompt_for_category: false,
            download_new_chapters: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Reader
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReaderPrefs {
    #[serde(default)]
    pub default_reading_mode: ReadingMode,
    #[serde(default)]
    pub scale_type: ImageScaleType,
    #[serde(default)]
    pub background: ReaderBackground,

    /// Trim uniform borders off the page before displaying it.
    #[serde(default)]
    pub crop_borders: bool,
    /// Show two pages side by side in horizontal modes.
    #[serde(default)]
    pub double_pages: bool,
    /// Give wide (spread) pages a slot of their own when double paging.
    #[serde(default = "yes")]
    pub double_page_split_spreads: bool,
    #[serde(default = "default_preload")]
    pub preload_pages: u32,
    /// Gap between pages in continuous modes, in points.
    #[serde(default)]
    pub webtoon_page_gap: f32,
    /// Cap the rendered width in webtoon mode, as a fraction of the viewport.
    #[serde(default = "default_side_padding")]
    pub webtoon_width_fraction: f32,

    #[serde(default = "yes")]
    pub show_page_number: bool,
    #[serde(default = "yes")]
    pub show_progress_bar: bool,
    #[serde(default = "yes")]
    pub skip_read_chapters_on_finish: bool,
    /// Fraction of the last page that must be reached to mark a chapter read.
    #[serde(default = "yes")]
    pub mark_read_on_last_page: bool,
    /// Delete a chapter's downloaded files once it has been read.
    #[serde(default)]
    pub remove_after_read: bool,
    /// How many read chapters to keep before deleting older ones (0 = keep all).
    #[serde(default)]
    pub remove_after_read_slots: u32,
    #[serde(default = "yes")]
    pub keyboard_navigation: bool,
    /// Invert the tap/scroll direction for the "next page" action.
    #[serde(default)]
    pub invert_navigation: bool,
    #[serde(default = "default_zoom_step")]
    pub zoom_step: f32,
}

fn default_preload() -> u32 {
    4
}
fn default_side_padding() -> f32 {
    0.7
}
fn default_zoom_step() -> f32 {
    1.25
}

impl Default for ReaderPrefs {
    fn default() -> Self {
        Self {
            default_reading_mode: ReadingMode::default(),
            scale_type: ImageScaleType::default(),
            background: ReaderBackground::default(),
            crop_borders: false,
            double_pages: false,
            double_page_split_spreads: true,
            preload_pages: default_preload(),
            webtoon_page_gap: 0.0,
            webtoon_width_fraction: default_side_padding(),
            show_page_number: true,
            show_progress_bar: true,
            skip_read_chapters_on_finish: true,
            mark_read_on_last_page: true,
            remove_after_read: false,
            remove_after_read_slots: 0,
            keyboard_navigation: true,
            invert_navigation: false,
            zoom_step: default_zoom_step(),
        }
    }
}

// ---------------------------------------------------------------------------
// Downloads
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadPrefs {
    /// Overrides the default downloads directory when set.
    #[serde(default)]
    pub directory: Option<PathBuf>,
    #[serde(default = "default_workers")]
    pub concurrent_downloads: u32,
    /// Automatically queue the next N chapters while reading.
    #[serde(default)]
    pub download_ahead: u32,
    /// Store each chapter as a single CBZ instead of a folder of images.
    #[serde(default = "yes")]
    pub save_as_cbz: bool,
    /// Category ids excluded from automatic downloads.
    #[serde(default)]
    pub excluded_categories: Vec<i64>,
}

fn default_workers() -> u32 {
    3
}

impl Default for DownloadPrefs {
    fn default() -> Self {
        Self {
            directory: None,
            concurrent_downloads: default_workers(),
            download_ahead: 0,
            save_as_cbz: true,
            excluded_categories: Vec::new(),
        }
    }
}

// ---------------------------------------------------------------------------
// Browse
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BrowsePrefs {
    #[serde(default)]
    pub pinned_sources: Vec<i64>,
    #[serde(default)]
    pub disabled_sources: Vec<i64>,
    /// Empty means "no language filter".
    #[serde(default)]
    pub enabled_languages: Vec<String>,
    #[serde(default)]
    pub show_nsfw_sources: bool,
    /// URLs of extension repositories to fetch index files from.
    #[serde(default)]
    pub extension_repos: Vec<String>,
    #[serde(default = "yes")]
    pub search_pinned_only: bool,
}

impl Default for BrowsePrefs {
    fn default() -> Self {
        Self {
            pinned_sources: Vec::new(),
            disabled_sources: Vec::new(),
            enabled_languages: Vec::new(),
            show_nsfw_sources: false,
            extension_repos: Vec::new(),
            search_pinned_only: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Root
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Preferences {
    #[serde(default)]
    pub theme_mode: ThemeMode,
    #[serde(default)]
    pub app_theme: AppTheme,
    /// Overrides the accent of `app_theme` when set.
    #[serde(default)]
    pub custom_accent: Option<[u8; 3]>,
    /// How much of the accent hue bleeds into backgrounds and surfaces.
    #[serde(default = "default_tint")]
    pub theme_tint: f32,
    /// True black backgrounds, for OLED panels.
    #[serde(default)]
    pub pure_black: bool,
    /// Corner rounding in points, from square to pill.
    #[serde(default = "default_corner_radius")]
    pub corner_radius: u8,
    #[serde(default)]
    pub density: Density,
    #[serde(default)]
    pub nav_style: NavStyle,
    #[serde(default)]
    pub card_style: CardStyle,
    /// Text size relative to the rest of the interface.
    #[serde(default = "default_ui_scale")]
    pub font_scale: f32,
    #[serde(default = "yes")]
    pub animations: bool,
    #[serde(default = "yes")]
    pub relative_timestamps: bool,
    #[serde(default = "default_ui_scale")]
    pub ui_scale: f32,
    /// Tab shown when the app starts (index into the bottom navigation).
    #[serde(default)]
    pub start_tab: usize,
    /// Ask for confirmation before leaving the app from the library tab.
    #[serde(default)]
    pub confirm_exit: bool,

    #[serde(default)]
    pub library: LibraryPrefs,
    #[serde(default)]
    pub reader: ReaderPrefs,
    #[serde(default)]
    pub downloads: DownloadPrefs,
    #[serde(default)]
    pub browse: BrowsePrefs,

    /// Hide anything that is not downloaded, across library and browsing.
    #[serde(default)]
    pub downloaded_only: bool,
    /// Stop recording reading history and progress.
    #[serde(default)]
    pub incognito: bool,

    /// Timestamp of the last automatic library check, in milliseconds.
    #[serde(default)]
    pub last_library_update: i64,

    /// Set once the onboarding flow has been completed.
    #[serde(default)]
    pub onboarding_complete: bool,
    /// Extra root folders scanned by the local source.
    #[serde(default)]
    pub local_source_dirs: Vec<PathBuf>,
}

fn default_ui_scale() -> f32 {
    1.0
}
fn default_tint() -> f32 {
    0.35
}
fn default_corner_radius() -> u8 {
    10
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            theme_mode: ThemeMode::default(),
            app_theme: AppTheme::default(),
            custom_accent: None,
            theme_tint: default_tint(),
            pure_black: false,
            corner_radius: default_corner_radius(),
            density: Density::default(),
            nav_style: NavStyle::default(),
            card_style: CardStyle::default(),
            font_scale: 1.0,
            animations: true,
            relative_timestamps: true,
            ui_scale: 1.0,
            start_tab: 0,
            confirm_exit: false,
            library: LibraryPrefs::default(),
            reader: ReaderPrefs::default(),
            downloads: DownloadPrefs::default(),
            browse: BrowsePrefs::default(),
            downloaded_only: false,
            incognito: false,
            last_library_update: 0,
            onboarding_complete: false,
            local_source_dirs: Vec::new(),
        }
    }
}

impl Preferences {
    /// (accent, accent_variant) for the current theme, honouring a custom hue.
    pub fn accent(&self) -> ([u8; 3], [u8; 3]) {
        match self.custom_accent {
            // A single picked colour has to yield the darker companion the
            // palettes normally ship with, or light mode loses its contrast.
            Some([r, g, b]) => {
                let dim = |c: u8| (c as f32 * 0.78) as u8;
                ([r, g, b], [dim(r), dim(g), dim(b)])
            }
            None => self.app_theme.accent(),
        }
    }

    /// Restores everything under Appearance, leaving the rest untouched.
    pub fn reset_appearance(&mut self) {
        let fresh = Self::default();
        self.theme_mode = fresh.theme_mode;
        self.app_theme = fresh.app_theme;
        self.custom_accent = fresh.custom_accent;
        self.theme_tint = fresh.theme_tint;
        self.pure_black = fresh.pure_black;
        self.corner_radius = fresh.corner_radius;
        self.density = fresh.density;
        self.nav_style = fresh.nav_style;
        self.card_style = fresh.card_style;
        self.font_scale = fresh.font_scale;
        self.animations = fresh.animations;
        self.ui_scale = fresh.ui_scale;
    }

    pub fn load(path: &Path) -> Self {
        let Ok(text) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        // Editors and PowerShell happily prepend a UTF-8 BOM, which serde_json
        // rejects; that must not silently wipe every setting.
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

        match serde_json::from_str(text) {
            Ok(prefs) => prefs,
            Err(err) => {
                log::warn!("preferences.json is unreadable ({err}); starting from defaults");
                // Keep the unreadable file around instead of overwriting it, so
                // a hand-edited mistake can still be recovered.
                let backup = path.with_extension("json.invalid");
                if let Err(err) = std::fs::rename(path, &backup) {
                    log::warn!("could not set the broken preferences aside: {err}");
                } else {
                    log::warn!("the previous file was kept at {}", backup.display());
                }
                Self::default()
            }
        }
    }

    pub fn save(&self, path: &Path) {
        match serde_json::to_string_pretty(self) {
            Ok(text) => {
                if let Err(err) = std::fs::write(path, text) {
                    log::error!("could not write preferences: {err}");
                }
            }
            Err(err) => log::error!("could not serialise preferences: {err}"),
        }
    }

    pub fn is_source_pinned(&self, id: i64) -> bool {
        self.browse.pinned_sources.contains(&id)
    }

    pub fn is_source_enabled(&self, id: i64) -> bool {
        !self.browse.disabled_sources.contains(&id)
    }

    pub fn toggle_pinned(&mut self, id: i64) {
        if let Some(pos) = self.browse.pinned_sources.iter().position(|&s| s == id) {
            self.browse.pinned_sources.remove(pos);
        } else {
            self.browse.pinned_sources.push(id);
        }
    }

    pub fn toggle_enabled(&mut self, id: i64) {
        if let Some(pos) = self.browse.disabled_sources.iter().position(|&s| s == id) {
            self.browse.disabled_sources.remove(pos);
        } else {
            self.browse.disabled_sources.push(id);
        }
    }

    pub fn downloads_dir(&self, paths: &AppPaths) -> PathBuf {
        self.downloads
            .directory
            .clone()
            .unwrap_or_else(|| paths.downloads.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("mihon-prefs-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn round_trips_through_disk() {
        let dir = temp_dir("round");
        let path = dir.join("preferences.json");

        let mut prefs = Preferences {
            theme_mode: ThemeMode::Light,
            app_theme: AppTheme::Tako,
            ..Default::default()
        };
        prefs.library.columns = 9;
        prefs.save(&path);

        let loaded = Preferences::load(&path);
        assert_eq!(loaded.theme_mode, ThemeMode::Light);
        assert_eq!(loaded.app_theme, AppTheme::Tako);
        assert_eq!(loaded.library.columns, 9);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A BOM used to make the whole file unreadable, silently resetting
    /// every preference.
    #[test]
    fn tolerates_a_utf8_bom() {
        let dir = temp_dir("bom");
        let path = dir.join("preferences.json");

        let prefs = Preferences {
            theme_mode: ThemeMode::Light,
            ..Default::default()
        };
        let json = serde_json::to_string(&prefs).unwrap();
        std::fs::write(&path, format!("\u{feff}{json}")).unwrap();

        assert_eq!(Preferences::load(&path).theme_mode, ThemeMode::Light);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_keys_fall_back_to_defaults() {
        let dir = temp_dir("partial");
        let path = dir.join("preferences.json");
        std::fs::write(&path, r#"{"ui_scale": 1.5}"#).unwrap();

        let loaded = Preferences::load(&path);
        assert_eq!(loaded.ui_scale, 1.5);
        assert_eq!(loaded.library.columns, default_columns());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file written before the appearance settings existed must keep working
    /// and land on the same look as a fresh install, not on zeroed-out values.
    #[test]
    fn a_file_without_the_appearance_keys_keeps_the_defaults() {
        let dir = temp_dir("appearance");
        let path = dir.join("preferences.json");
        std::fs::write(&path, r#"{"theme_mode": "Dark", "app_theme": "Tako"}"#).unwrap();

        let loaded = Preferences::load(&path);
        assert_eq!(loaded.app_theme, AppTheme::Tako);
        assert_eq!(loaded.corner_radius, default_corner_radius());
        assert_eq!(loaded.theme_tint, default_tint());
        assert_eq!(loaded.density, Density::Cozy);
        assert_eq!(loaded.nav_style, NavStyle::Rail);
        assert_eq!(loaded.card_style, CardStyle::Flat);
        assert_eq!(loaded.font_scale, 1.0);
        assert!(loaded.animations);
        assert!(loaded.custom_accent.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resetting_the_appearance_leaves_everything_else_alone() {
        let mut prefs = Preferences {
            app_theme: AppTheme::Nord,
            custom_accent: Some([1, 2, 3]),
            density: Density::Compact,
            nav_style: NavStyle::Bottom,
            corner_radius: 0,
            incognito: true,
            start_tab: 3,
            ..Default::default()
        };
        prefs.library.columns = 9;

        prefs.reset_appearance();

        assert_eq!(prefs.app_theme, AppTheme::default());
        assert!(prefs.custom_accent.is_none());
        assert_eq!(prefs.density, Density::Cozy);
        assert_eq!(prefs.nav_style, NavStyle::Rail);
        assert_eq!(prefs.corner_radius, default_corner_radius());
        // Untouched: these are not appearance.
        assert!(prefs.incognito);
        assert_eq!(prefs.start_tab, 3);
        assert_eq!(prefs.library.columns, 9);
    }

    #[test]
    fn a_custom_accent_takes_priority_and_yields_a_darker_companion() {
        let prefs = Preferences {
            app_theme: AppTheme::Green,
            custom_accent: Some([200, 100, 50]),
            ..Default::default()
        };
        let (accent, dim) = prefs.accent();
        assert_eq!(accent, [200, 100, 50]);
        assert!(dim.iter().zip(accent.iter()).all(|(d, a)| d < a));
    }

    #[test]
    fn a_broken_file_is_kept_aside() {
        let dir = temp_dir("broken");
        let path = dir.join("preferences.json");
        std::fs::write(&path, "not json at all").unwrap();

        let _ = Preferences::load(&path);
        assert!(
            path.with_extension("json.invalid").exists(),
            "the unreadable file should have been preserved"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
