//! Scripted screenshots, for verifying the UI without a human at the keyboard.
//!
//! Enabled only when `MIHON_SCREENSHOT` points at a `.png`. The harness drives
//! the app to a named screen, waits for the asynchronous data to land, grabs the
//! frame and exits. Also handy for producing release artwork.
//!
//! ```text
//! MIHON_SCREENSHOT=out.png MIHON_SCREENSHOT_VIEW=source MIHON_SCREENSHOT_DELAY=8 mihon-desktop
//! ```

use std::path::PathBuf;
use std::time::{Duration, Instant};

use super::{App, Route, SettingsPage, Tab};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Stage {
    Navigate,
    WaitForData,
    Requested,
    Done,
}

pub struct Capture {
    path: PathBuf,
    view: String,
    delay: Duration,
    started: Instant,
    stage: Stage,
    /// Which browse result to try next; some titles are licensed and expose no
    /// readable chapters, so the harness walks past them.
    attempt: usize,
    attempt_since: Option<Instant>,
}

impl Capture {
    pub fn is_glyph_probe(&self) -> bool {
        self.view == "glyphs"
    }

    pub fn from_env() -> Option<Self> {
        let path = std::env::var("MIHON_SCREENSHOT").ok()?;
        let delay = std::env::var("MIHON_SCREENSHOT_DELAY")
            .ok()
            .and_then(|value| value.parse::<f32>().ok())
            .unwrap_or(6.0);
        Some(Self {
            path: PathBuf::from(path),
            view: std::env::var("MIHON_SCREENSHOT_VIEW").unwrap_or_else(|_| "library".into()),
            delay: Duration::from_secs_f32(delay),
            started: Instant::now(),
            stage: Stage::Navigate,
            attempt: 0,
            attempt_since: None,
        })
    }
}

/// Advances the capture state machine. Called once per frame.
pub fn tick(app: &mut App, ctx: &egui::Context) {
    let Some(capture) = app.capture.as_ref() else {
        return;
    };
    if capture.stage == Stage::Done {
        return;
    }
    // Keep frames coming even when nothing is interacting with the window.
    ctx.request_repaint_after(Duration::from_millis(120));

    let (view, stage, elapsed, delay) = (
        capture.view.clone(),
        capture.stage,
        capture.started.elapsed(),
        capture.delay,
    );

    match stage {
        Stage::Navigate => {
            navigate(app, &view);
            if let Some(capture) = app.capture.as_mut() {
                capture.stage = Stage::WaitForData;
            }
        }
        Stage::WaitForData => {
            follow_up(app, &view);
            if elapsed >= delay {
                log::info!(
                    "capture: requesting screenshot (route={:?}, tab={:?})",
                    app.current_route(),
                    app.tab
                );
                ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
                if let Some(capture) = app.capture.as_mut() {
                    capture.stage = Stage::Requested;
                }
            }
        }
        Stage::Requested => {
            let shot = ctx.input(|input| {
                input.events.iter().find_map(|event| match event {
                    egui::Event::Screenshot { image, .. } => Some(image.clone()),
                    _ => None,
                })
            });
            if let Some(image) = shot {
                let path = app.capture.as_ref().map(|c| c.path.clone()).unwrap();
                match save(&image, &path) {
                    Ok(()) => log::info!("screenshot written to {}", path.display()),
                    Err(err) => log::error!("could not write the screenshot: {err}"),
                }
                if let Some(capture) = app.capture.as_mut() {
                    capture.stage = Stage::Done;
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            }
        }
        Stage::Done => {}
    }
}

/// Applies `MIHON_SCREENSHOT_THEME=light|dark`, so both palettes can be
/// checked without touching the user's saved preferences.
fn apply_theme_override(app: &mut App) {
    use crate::prefs::{AppTheme, CardStyle, Density, NavStyle};

    match std::env::var("MIHON_SCREENSHOT_THEME").as_deref() {
        Ok("light") => app.prefs.theme_mode = crate::prefs::ThemeMode::Light,
        Ok("dark") => app.prefs.theme_mode = crate::prefs::ThemeMode::Dark,
        Ok("system") => app.prefs.theme_mode = crate::prefs::ThemeMode::System,
        _ => {}
    }

    // Every appearance knob is overridable, so each combination can be looked
    // at without editing the saved preferences of whoever is running this.
    if let Ok(value) = std::env::var("MIHON_SCREENSHOT_NAV") {
        app.prefs.nav_style = match value.as_str() {
            "compact" => NavStyle::Compact,
            "bottom" => NavStyle::Bottom,
            _ => NavStyle::Rail,
        };
    }
    if let Ok(value) = std::env::var("MIHON_SCREENSHOT_DENSITY") {
        app.prefs.density = match value.as_str() {
            "compact" => Density::Compact,
            "comfortable" => Density::Comfortable,
            _ => Density::Cozy,
        };
    }
    if let Ok(value) = std::env::var("MIHON_SCREENSHOT_CARD") {
        app.prefs.card_style = match value.as_str() {
            "outlined" => CardStyle::Outlined,
            "elevated" => CardStyle::Elevated,
            _ => CardStyle::Flat,
        };
    }
    if let Ok(value) = std::env::var("MIHON_SCREENSHOT_ACCENT") {
        // Matched on letters alone, so "Teal & Turquoise", "teal-turquoise"
        // and "Tealturquoise" all name the same theme.
        let normalise = |s: &str| {
            s.chars()
                .filter(|c| c.is_ascii_alphanumeric())
                .map(|c| c.to_ascii_lowercase())
                .collect::<String>()
        };
        let wanted = normalise(&value);
        match AppTheme::ALL
            .iter()
            .find(|t| normalise(t.label()) == wanted)
        {
            Some(theme) => app.prefs.app_theme = *theme,
            None => log::warn!("capture: no theme called {value:?}"),
        }
    }
    if let Ok(Ok(value)) = std::env::var("MIHON_SCREENSHOT_TINT").map(|v| v.parse()) {
        app.prefs.theme_tint = value;
    }
    if let Ok(Ok(value)) = std::env::var("MIHON_SCREENSHOT_RADIUS").map(|v| v.parse()) {
        app.prefs.corner_radius = value;
    }
    if std::env::var("MIHON_SCREENSHOT_PURE_BLACK").is_ok() {
        app.prefs.pure_black = true;
    }

    // `MIHON_SCREENSHOT_MODE` forces a reading mode, so both the paged and the
    // continuous reader can be captured.
    use crate::model::ReadingMode;
    let mode = match std::env::var("MIHON_SCREENSHOT_MODE").as_deref() {
        Ok("ltr") => Some(ReadingMode::LeftToRight),
        Ok("rtl") => Some(ReadingMode::RightToLeft),
        Ok("vertical") => Some(ReadingMode::Vertical),
        Ok("webtoon") => Some(ReadingMode::Webtoon),
        Ok("continuous") => Some(ReadingMode::ContinuousVertical),
        Ok("infinite") => Some(ReadingMode::Infinite),
        _ => None,
    };
    if let Some(mode) = mode {
        app.prefs.reader.default_reading_mode = mode;
        app.reader.mode = mode;
    }
}

/// Opens the requested screen.
fn navigate(app: &mut App, view: &str) {
    apply_theme_override(app);
    match view {
        "library" => app.tab = Tab::Library,
        "updates" => app.tab = Tab::Updates,
        "history" => app.tab = Tab::History,
        "browse" => app.tab = Tab::Browse,
        "more" => app.tab = Tab::More,
        "extensions" => {
            app.tab = Tab::Browse;
            app.browse.tab = super::BrowseTab::Extensions;
        }
        "downloads" => app.push(Route::Downloads),
        // Opens a library entry by title, for reproducing a report about a
        // specific series: MIHON_SCREENSHOT_TITLE=Kumo
        "libitem" => {
            app.tab = Tab::Library;
            app.refresh_library();
            let needle = std::env::var("MIHON_SCREENSHOT_TITLE")
                .unwrap_or_default()
                .to_lowercase();
            let target = app
                .library
                .entries
                .iter()
                .find(|e| e.manga.title.to_lowercase().contains(&needle))
                .map(|e| (e.manga.id, e.manga.title.clone()));
            match target {
                Some((manga_id, title)) => {
                    log::info!("capture: opening {title:?} (id {manga_id})");
                    app.open_manga(manga_id);
                }
                None => log::warn!("capture: no library entry matches {needle:?}"),
            }
        }
        // The library tile's play button jumps straight into the reader; that
        // path is easy to hit by accident, so it gets its own scenario.
        "libread" => {
            app.tab = Tab::Library;
            app.refresh_library();
            let needle = std::env::var("MIHON_SCREENSHOT_TITLE")
                .unwrap_or_default()
                .to_lowercase();
            let target = app
                .library
                .entries
                .iter()
                .find(|e| e.manga.title.to_lowercase().contains(&needle))
                .map(|e| e.manga.id);
            match target.and_then(|id| app.core.db.next_unread_chapter(id).map(|c| (id, c))) {
                Some((manga_id, chapter)) => {
                    log::info!(
                        "capture: reading {:?} (chapter {})",
                        chapter.name,
                        chapter.id
                    );
                    app.open_reader(manga_id, chapter.id);
                }
                None => log::warn!("capture: nothing to read for {needle:?}"),
            }
        }
        // Not a real screen: renders candidate glyphs so missing ones can be
        // spotted before they end up in the interface.
        "glyphs" => app.tab = Tab::Library,
        "settings" => app.push(Route::Settings(SettingsPage::Appearance)),
        "statistics" => app.push(Route::Statistics),
        // Searches the source for MIHON_SCREENSHOT_TITLE and reads the first
        // hit — the only way to reach a specific series in a fresh profile.
        "search" => {
            if let Some(source_id) = first_remote_source(app) {
                let query = std::env::var("MIHON_SCREENSHOT_TITLE").unwrap_or_default();
                app.open_source(source_id);
                app.source_browse.query = query.clone();
                app.source_browse.mode = crate::core::BrowseMode::Search;
                app.source_browse.loading = true;
                app.core.browse(
                    source_id,
                    crate::core::BrowseMode::Search,
                    1,
                    query,
                    Vec::new(),
                );
            }
        }
        // Anything source-driven starts by browsing MangaDex.
        "source" | "manga" | "reader" | "seed" => {
            if let Some(source) = first_remote_source(app) {
                app.open_source(source);
            }
        }
        other => log::warn!("unknown screenshot view {other:?}"),
    }
}

/// Steps deeper once the previous screen's data has arrived.
fn follow_up(app: &mut App, view: &str) {
    match view {
        // Search hit -> details -> reader, one step per frame.
        "search" => {
            if matches!(app.current_route(), Some(Route::SourceBrowse(_)))
                && !app.source_browse.items.is_empty()
            {
                let source_id = app.source_browse.source_id;
                let item = app.source_browse.items[0].clone();
                log::info!("capture: opening search hit {:?}", item.title);
                super::source_browse::open_result(app, source_id, &item);
            } else if matches!(app.current_route(), Some(Route::MangaDetails(_)))
                && !app.details.chapters.is_empty()
            {
                let manga_id = app.details.manga_id;
                if let Some(chapter) = app.core.db.next_unread_chapter(manga_id) {
                    log::info!("capture: reading {:?}", chapter.name);
                    app.open_reader(manga_id, chapter.id);
                }
            }
        }
        // Favourites a handful of browse results, then shows the library. Also
        // exercises the "add to library" path end to end.
        "seed" => {
            if app.source_browse.items.is_empty() {
                return;
            }
            // Seed only once, but always land on the library afterwards.
            if app.core.db.library_size() == 0 {
                let source_id = app.source_browse.source_id;
                let items = app.source_browse.items.clone();
                for item in items.iter().take(14) {
                    if let Ok(manga) = app.core.intern_manga(source_id, item) {
                        let _ = app.core.db.update_manga(manga.id, |m| {
                            m.favorite = true;
                            m.date_added = crate::model::now_millis();
                        });
                        let _ = app
                            .core
                            .db
                            .set_categories(manga.id, vec![crate::model::Category::DEFAULT_ID]);
                    }
                }
            }
            if !app.stack.is_empty() {
                app.stack.clear();
                app.tab = Tab::Library;
                app.refresh_library();
            }
        }
        "manga" | "reader" => {
            let on_source = matches!(app.current_route(), Some(Route::SourceBrowse(_)));
            if on_source && !app.source_browse.items.is_empty() {
                let attempt = app.capture.as_ref().map(|c| c.attempt).unwrap_or(0);
                let source_id = app.source_browse.source_id;
                let Some(item) = app.source_browse.items.get(attempt).cloned() else {
                    return;
                };
                super::source_browse::open_result(app, source_id, &item);
                if let Some(capture) = app.capture.as_mut() {
                    capture.attempt_since = Some(Instant::now());
                }
            }

            let on_details = matches!(app.current_route(), Some(Route::MangaDetails(_)));
            if on_details && app.details.chapters.is_empty() && !app.details.loading {
                // Give the fetch a moment, then try the next candidate.
                let stale = app
                    .capture
                    .as_ref()
                    .and_then(|c| c.attempt_since)
                    .map(|since| since.elapsed() > Duration::from_secs(4))
                    .unwrap_or(false);
                if stale {
                    if let Some(capture) = app.capture.as_mut() {
                        capture.attempt += 1;
                        capture.attempt_since = None;
                    }
                    app.pop();
                }
                return;
            }

            if view == "reader" && on_details && !app.details.chapters.is_empty() {
                let manga_id = app.details.manga_id;
                // The oldest unread chapter is the natural starting point.
                if let Some(chapter) = app.core.db.next_unread_chapter(manga_id) {
                    app.open_reader(manga_id, chapter.id);
                }
            }
        }
        // Turns pages once they have loaded, so the save-progress path (which
        // once deadlocked on the first page of an unread chapter) is covered.
        "libread" => {
            let turns: i32 = std::env::var("MIHON_SCREENSHOT_TURNS")
                .ok()
                .and_then(|v| v.parse().ok())
                .unwrap_or(0);
            if app.reader.pages.is_empty() {
                return;
            }
            // A negative count means "jump to the end", which is how the
            // chapter hand-off in infinite scroll gets exercised.
            if turns < 0 {
                let done = app.capture.as_ref().map(|c| c.attempt).unwrap_or(0);
                if done == 0 {
                    let remaining = app.reader.pages.len() as i32;
                    log::info!("capture: jumping to the end of the chapter");
                    super::reader::turn_page(app, remaining);
                    if let Some(capture) = app.capture.as_mut() {
                        capture.attempt += 1;
                    }
                }
                return;
            }
            if turns == 0 {
                return;
            }
            let done = app.capture.as_ref().map(|c| c.attempt).unwrap_or(0) as i32;
            // Pace the turns: firing one per frame would race ahead of the
            // images and exercise nothing.
            let ready = app
                .capture
                .as_ref()
                .and_then(|c| c.attempt_since)
                .map(|since| since.elapsed() > Duration::from_millis(350))
                .unwrap_or(true);
            if done < turns && ready {
                log::info!("capture: turning page ({}/{turns})", done + 1);
                super::reader::turn_page(app, 1);
                if let Some(capture) = app.capture.as_mut() {
                    capture.attempt += 1;
                    capture.attempt_since = Some(Instant::now());
                }
            }
        }
        _ => {}
    }
}

/// Candidate glyphs, rendered side by side with their code point so that any
/// character the bundled font lacks shows up as a box next to its name.
pub const CANDIDATES: &[(&str, &str)] = &[
    // Directional
    ("◀", "tri-left"),
    ("▶", "tri-right"),
    ("▲", "tri-up"),
    ("▼", "tri-down"),
    ("‹", "chev-left"),
    ("›", "chev-right"),
    ("«", "guill-left"),
    ("»", "guill-right"),
    ("⬅", "emoji-left"),
    ("➡", "emoji-right"),
    ("⬆", "emoji-up"),
    ("⬇", "emoji-down"),
    ("⏪", "rewind"),
    ("⏩", "fast-fwd"),
    // Containers / states
    ("▢", "square-round"),
    ("▣", "square-fill"),
    ("◻", "square-white"),
    ("◼", "square-black"),
    ("▪", "sq-small-black"),
    ("▫", "sq-small-white"),
    ("◇", "diamond"),
    ("◆", "diamond-fill"),
    ("⊞", "boxplus"),
    ("⬚", "dashed-box"),
    // Objects
    ("📚", "books"),
    ("📖", "book-open"),
    ("📕", "book"),
    ("🗂", "cards"),
    ("📁", "folder"),
    ("📥", "inbox"),
    ("📤", "outbox"),
    ("🗑", "trash"),
    ("💾", "save"),
    ("🔔", "bell"),
    ("🏷", "tag"),
    ("⚑", "flag"),
    ("➕", "plus-emoji"),
    ("➖", "minus-emoji"),
    ("🔄", "arrows-round"),
    ("🔀", "shuffle"),
    ("🎲", "dice"),
    ("⏹", "stop"),
    ("⏯", "play-pause"),
    ("🖼", "picture"),
    ("🌐", "globe"),
    ("🧩", "puzzle"),
];

/// Draws the probe sheet.
pub fn glyph_probe(app: &mut App, ui: &mut egui::Ui) {
    let palette = app.palette;
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::same(16)))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("Glyph availability probe")
                    .size(18.0)
                    .strong(),
            );
            ui.label(
                egui::RichText::new("A box means the bundled font has no such glyph.")
                    .size(12.0)
                    .color(palette.text_dim),
            );
            ui.add_space(8.0);

            for (script, sample) in super::fonts::SCRIPT_SAMPLES {
                ui.horizontal(|ui| {
                    ui.add_sized(
                        egui::vec2(80.0, 18.0),
                        egui::Label::new(
                            egui::RichText::new(*script)
                                .size(11.0)
                                .color(palette.text_dim),
                        ),
                    );
                    ui.label(egui::RichText::new(*sample).size(16.0));
                });
            }
            ui.add_space(10.0);

            egui::ScrollArea::vertical().show(ui, |ui| {
                egui::Grid::new("glyph_grid")
                    .num_columns(8)
                    .spacing(egui::vec2(10.0, 10.0))
                    .show(ui, |ui| {
                        for (index, (glyph, name)) in CANDIDATES.iter().enumerate() {
                            ui.scope_builder(
                                egui::UiBuilder::new()
                                    .max_rect(egui::Rect::from_min_size(
                                        ui.cursor().min,
                                        egui::vec2(120.0, 54.0),
                                    ))
                                    .layout(egui::Layout::top_down(egui::Align::Center)),
                                |ui| {
                                    ui.label(egui::RichText::new(*glyph).size(22.0));
                                    ui.label(
                                        egui::RichText::new(*name)
                                            .size(10.0)
                                            .color(palette.text_dim),
                                    );
                                },
                            );
                            if (index + 1) % 8 == 0 {
                                ui.end_row();
                            }
                        }
                    });
            });
        });
}

fn first_remote_source(app: &App) -> Option<crate::model::Id> {
    // `MIHON_SCREENSHOT_SOURCE` picks a source by name; otherwise the first
    // English one is used.
    let wanted = std::env::var("MIHON_SCREENSHOT_SOURCE")
        .unwrap_or_default()
        .to_lowercase();
    app.core
        .sources
        .visible(&app.prefs)
        .into_iter()
        .filter(|s| s.id() != crate::source::local::LOCAL_ID)
        .find(|s| {
            if wanted.is_empty() {
                s.lang() == "en"
            } else {
                s.name().to_lowercase().contains(&wanted)
            }
        })
        .map(|s| s.id())
}

fn save(image: &egui::ColorImage, path: &std::path::Path) -> anyhow::Result<()> {
    let [width, height] = image.size;
    let bytes: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|pixel| pixel.to_array())
        .collect();

    let buffer = image::RgbaImage::from_raw(width as u32, height as u32, bytes)
        .ok_or_else(|| anyhow::anyhow!("screenshot buffer has the wrong size"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    buffer.save(path)?;
    Ok(())
}
