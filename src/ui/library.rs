//! The Library tab: categorised grid of favourited manga.

use egui::{Align, Color32, Layout, RichText, Ui, vec2};

use super::widgets::{self, COVER_ASPECT};
use super::{App, Dialog};
use crate::model::*;
use crate::source::local;

/// Row data pulled out of a [`LibraryEntry`] just before drawing it, so the
/// immutable borrow of the library ends before `App` is borrowed mutably.
struct Row {
    manga_id: Id,
    title: String,
    cover: Option<String>,
    source: Id,
    unread: i64,
    downloaded: i64,
    total: i64,
    is_local: bool,
    lang: Option<String>,
}

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    // -- toolbar ------------------------------------------------------------
    egui::Panel::top("library_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let count = app.library.entries.len();
                ui.vertical(|ui| {
                    ui.label(RichText::new("Library").size(20.0).strong());
                    ui.label(
                        RichText::new(format!(
                            "{count} entr{}",
                            if count == 1 { "y" } else { "ies" }
                        ))
                        .size(12.0)
                        .color(palette.text_dim),
                    );
                });
                ui.add_space(12.0);
                widgets::search_field(ui, &palette, "Search your library", &mut app.library.query);

                widgets::toolbar_actions(ui, |ui| {
                    if ui
                        .button("⚙")
                        .on_hover_text("Filter, sort and display")
                        .clicked()
                    {
                        app.dialog = Some(Dialog::LibrarySettings);
                    }
                    let updating = app.core.is_updating();
                    let update = ui.add_enabled(!updating, egui::Button::new("🔄"));
                    if update.on_hover_text("Update library").clicked() {
                        let category = current_category_id(app);
                        app.core.update_library(&app.prefs, category);
                    }
                    if ui
                        .button("🎲")
                        .on_hover_text("Open a random entry")
                        .clicked()
                    {
                        open_random(app);
                    }
                });
            });
        });

    // -- selection toolbar --------------------------------------------------
    if !app.library.selection.is_empty() {
        egui::Panel::top("library_selection")
            .frame(
                egui::Frame::NONE
                    .fill(palette.accent.gamma_multiply(0.18))
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| selection_bar(app, ui));
    }

    // -- category tabs ------------------------------------------------------
    let categories = visible_categories(app);
    if categories.len() > 1 {
        egui::Panel::top("library_tabs")
            .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 0)))
            .show(ui, |ui| {
                egui::ScrollArea::horizontal()
                    .id_salt("category_tabs")
                    .show(ui, |ui| {
                        ui.horizontal(|ui| {
                            for (index, category) in categories.iter().enumerate() {
                                let selected = app.library.category_index == index;
                                let count = count_in_category(app, category.id);
                                let label = format!("{} ({count})", category.name);
                                let button =
                                    egui::Button::new(RichText::new(label).color(if selected {
                                        palette.accent
                                    } else {
                                        palette.text_dim
                                    }))
                                    .fill(if selected {
                                        palette.accent.gamma_multiply(0.18)
                                    } else {
                                        Color32::TRANSPARENT
                                    });
                                if ui.add(button).clicked() {
                                    app.library.category_index = index;
                                }
                            }
                        });
                    });
                ui.add_space(6.0);
            });
    }

    // -- content ------------------------------------------------------------
    let rows = collect_rows(app);

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            if app.library.entries.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    "📚",
                    "Your library is empty",
                    "Find something in Browse and add it to your library.",
                );
                return;
            }
            if rows.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    "🔍",
                    "Nothing matches",
                    "No entry matches the current search or filters.",
                );
                return;
            }

            if app.prefs.library.display_mode == LibraryDisplayMode::List {
                list_view(app, ui, &rows);
            } else {
                grid_view(app, ui, &rows);
            }
        });
}

// ---------------------------------------------------------------------------
// Content views
// ---------------------------------------------------------------------------

fn grid_view(app: &mut App, ui: &mut Ui, rows: &[Row]) {
    let spacing = 10.0;
    let (columns, tile_width) =
        widgets::grid_columns(ui.available_width(), app.prefs.library.columns, spacing);

    let show_title = app.prefs.library.display_mode != LibraryDisplayMode::CoverOnlyGrid;
    let comfortable = app.prefs.library.display_mode == LibraryDisplayMode::ComfortableGrid;
    let title_height = if show_title {
        if comfortable { 38.0 } else { 34.0 }
    } else {
        0.0
    };
    let row_height = tile_width * COVER_ASPECT + title_height + spacing;
    let total_rows = rows.len().div_ceil(columns);

    egui::ScrollArea::vertical()
        .id_salt("library_grid")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, total_rows, |ui, range| {
            for row_index in range {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    for column in 0..columns {
                        let Some(row) = rows.get(row_index * columns + column) else {
                            break;
                        };
                        draw_tile(app, ui, row, tile_width, show_title);
                    }
                });
                ui.add_space(spacing);
            }
        });
}

fn draw_tile(app: &mut App, ui: &mut Ui, row: &Row, width: f32, show_title: bool) {
    let palette = app.palette;
    let selected = app.library.selection.contains(&row.manga_id);
    let badges = badges_for(app, row);

    let response = widgets::cover_tile(
        app,
        ui,
        width,
        row.cover.as_deref(),
        row.source,
        &row.title,
        show_title,
        selected,
        &badges,
        false,
    );

    // The play button overlaps the tile, so it is resolved first and the tile's
    // own click is skipped when it fires.
    let mut opened_reader = false;
    if app.prefs.library.show_continue_button && response.hovered() && row.unread > 0 {
        let rect = response.rect;
        let button_rect = egui::Rect::from_min_size(
            egui::pos2(
                rect.right() - 34.0,
                rect.top() + width * COVER_ASPECT - 34.0,
            ),
            vec2(28.0, 28.0),
        );
        let painter = ui.painter();
        painter.circle_filled(button_rect.center(), 14.0, palette.accent);
        painter.text(
            button_rect.center(),
            egui::Align2::CENTER_CENTER,
            "▶",
            egui::FontId::proportional(13.0),
            palette.on_accent,
        );
        let button_response = ui.interact(
            button_rect,
            ui.id().with(("continue", row.manga_id)),
            egui::Sense::click(),
        );
        if button_response.clicked() {
            opened_reader = true;
            continue_reading(app, row.manga_id);
        }
    }

    if response.clicked() && !opened_reader {
        if app.library.selection.is_empty() {
            app.open_manga(row.manga_id);
        } else {
            toggle_selection(app, row.manga_id);
        }
    }
    if response.secondary_clicked() {
        toggle_selection(app, row.manga_id);
    }

    response.on_hover_text(format!(
        "{}\n{} chapter(s), {} unread",
        row.title, row.total, row.unread
    ));
}

fn list_view(app: &mut App, ui: &mut Ui, rows: &[Row]) {
    let palette = app.palette;
    let row_height = 74.0;

    egui::ScrollArea::vertical()
        .id_salt("library_list")
        .auto_shrink([false, false])
        .show_rows(ui, row_height, rows.len(), |ui, range| {
            for index in range {
                let Some(row) = rows.get(index) else { break };
                let selected = app.library.selection.contains(&row.manga_id);

                let response = widgets::clickable_row(ui, |ui| {
                    super::theme::card(&palette)
                        .fill(if selected {
                            palette.accent.gamma_multiply(0.2)
                        } else {
                            palette.surface
                        })
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let cover_size = vec2(34.0, 34.0 * COVER_ASPECT);
                                let (rect, _) =
                                    ui.allocate_exact_size(cover_size, egui::Sense::hover());
                                widgets::paint_cover(
                                    app,
                                    ui,
                                    rect,
                                    row.cover.as_deref(),
                                    row.source,
                                    &row.title,
                                );
                                ui.add_space(6.0);
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&row.title).strong());
                                    ui.label(
                                        RichText::new(format!(
                                            "{} chapter(s) · {} unread",
                                            row.total, row.unread
                                        ))
                                        .size(11.5)
                                        .color(palette.text_dim),
                                    );
                                });
                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    for (text, colour) in badges_for(app, row) {
                                        widgets::badge(ui, &text, colour, Color32::WHITE);
                                    }
                                });
                            });
                        });
                })
                .response;

                if response.clicked() {
                    if app.library.selection.is_empty() {
                        app.open_manga(row.manga_id);
                    } else {
                        toggle_selection(app, row.manga_id);
                    }
                }
                if response.secondary_clicked() {
                    toggle_selection(app, row.manga_id);
                }
            }
        });
}

// ---------------------------------------------------------------------------
// Selection actions
// ---------------------------------------------------------------------------

fn selection_bar(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let selected: Vec<Id> = app.library.selection.iter().copied().collect();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} selected", selected.len()))
                .strong()
                .color(palette.text),
        );
        ui.add_space(10.0);

        if ui.button("Select all").clicked() {
            let all: Vec<Id> = collect_rows(app).iter().map(|r| r.manga_id).collect();
            app.library.selection.extend(all);
        }
        if ui.button("Mark read").clicked() {
            set_read_state(app, &selected, true);
        }
        if ui.button("Mark unread").clicked() {
            set_read_state(app, &selected, false);
        }
        if ui.button("Download unread").clicked() {
            let mut queued = 0;
            for manga_id in &selected {
                let unread: Vec<Id> = app
                    .core
                    .db
                    .chapters_of(*manga_id)
                    .into_iter()
                    .filter(|c| !c.read)
                    .map(|c| c.id)
                    .collect();
                queued += unread.len();
                app.core.queue_downloads(*manga_id, &unread);
            }
            if queued == 0 {
                app.toast("nothing left to download");
            }
        }
        if ui.button("Categories").clicked() {
            let current = selected
                .first()
                .map(|id| app.core.db.categories_of(*id))
                .unwrap_or_default();
            app.dialog = Some(Dialog::CategoryPicker {
                manga_ids: selected.clone(),
                selected: current.into_iter().collect(),
            });
        }
        let remove =
            egui::Button::new(RichText::new("Remove").color(Color32::WHITE)).fill(palette.error);
        if ui.add(remove).clicked() {
            app.dialog = Some(Dialog::ConfirmRemove {
                manga_ids: selected.clone(),
                delete_downloads: false,
            });
        }

        widgets::toolbar_actions(ui, |ui| {
            if ui.button("✖").on_hover_text("Clear selection").clicked() {
                app.library.selection.clear();
            }
        });
    });
}

fn toggle_selection(app: &mut App, manga_id: Id) {
    if !app.library.selection.remove(&manga_id) {
        app.library.selection.insert(manga_id);
    }
}

fn set_read_state(app: &mut App, manga_ids: &[Id], read: bool) {
    for manga_id in manga_ids {
        let ids: Vec<Id> = app
            .core
            .db
            .chapters_of(*manga_id)
            .into_iter()
            .map(|c| c.id)
            .collect();
        if let Err(err) = app.core.db.update_chapters(&ids, |chapter| {
            chapter.read = read;
            if read {
                chapter.last_page_read = 0;
            }
        }) {
            app.toast_error(format!("could not update chapters: {err}"));
            return;
        }
    }
    app.invalidate_all();
}

fn continue_reading(app: &mut App, manga_id: Id) {
    match app.core.db.next_unread_chapter(manga_id) {
        Some(chapter) => app.open_reader(manga_id, chapter.id),
        None => app.toast("no chapters to read yet"),
    }
}

fn open_random(app: &mut App) {
    let rows = collect_rows(app);
    if rows.is_empty() {
        app.toast("your library is empty");
        return;
    }
    // A cheap deterministic pick that still varies between clicks.
    let seed = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.subsec_nanos() as usize)
        .unwrap_or(0);
    let manga_id = rows[seed % rows.len()].manga_id;
    app.open_manga(manga_id);
}

// ---------------------------------------------------------------------------
// Filtering, sorting, helpers
// ---------------------------------------------------------------------------

fn visible_categories(app: &App) -> Vec<Category> {
    let mut categories: Vec<Category> = app
        .library
        .categories
        .iter()
        .filter(|c| !c.hidden)
        .cloned()
        .collect();
    categories.sort_by_key(|c| c.order);
    categories
}

fn current_category_id(app: &App) -> Option<Id> {
    let categories = visible_categories(app);
    if categories.len() <= 1 {
        return None;
    }
    categories.get(app.library.category_index).map(|c| c.id)
}

fn count_in_category(app: &App, category_id: Id) -> usize {
    app.library
        .entries
        .iter()
        .filter(|entry| entry_in_category(entry, category_id))
        .count()
}

fn entry_in_category(entry: &LibraryEntry, category_id: Id) -> bool {
    if entry.category_ids.is_empty() {
        category_id == Category::DEFAULT_ID
    } else {
        entry.category_ids.contains(&category_id)
    }
}

/// Applies the category, search and filter state, then sorts.
fn collect_rows(app: &App) -> Vec<Row> {
    let prefs = &app.prefs.library;
    let needle = app.library.query.trim().to_lowercase();
    let category = current_category_id(app);

    let mut selected: Vec<&LibraryEntry> = app
        .library
        .entries
        .iter()
        .filter(|entry| match category {
            Some(id) => entry_in_category(entry, id),
            None => true,
        })
        .filter(|entry| {
            needle.is_empty()
                || entry.manga.title.to_lowercase().contains(&needle)
                || entry
                    .manga
                    .author
                    .as_deref()
                    .map(|a| a.to_lowercase().contains(&needle))
                    .unwrap_or(false)
        })
        .filter(|entry| prefs.filters.matches(entry))
        // "Downloaded only" is a global switch that overrides the filter sheet.
        .filter(|entry| !app.prefs.downloaded_only || entry.downloaded_count > 0)
        .collect();

    sort_entries(
        &mut selected,
        prefs.sort,
        prefs.sort_ascending,
        app.library.random_seed,
    );

    selected
        .into_iter()
        .map(|entry| Row {
            manga_id: entry.manga.id,
            title: entry.manga.title.clone(),
            cover: entry.manga.thumbnail_url.clone(),
            source: entry.manga.source,
            unread: entry.unread_count,
            downloaded: entry.downloaded_count,
            total: entry.total_chapters,
            is_local: entry.manga.source == local::LOCAL_ID,
            lang: source_lang(app, entry.manga.source),
        })
        .collect()
}

fn source_lang(app: &App, source_id: Id) -> Option<String> {
    app.core
        .sources
        .get(source_id)
        .map(|s| s.lang().to_string())
        .filter(|lang| lang != local::LOCAL_LANG)
}

fn sort_entries(entries: &mut [&LibraryEntry], sort: LibrarySort, ascending: bool, seed: u64) {
    entries.sort_by(|a, b| {
        let ordering = match sort {
            LibrarySort::Alphabetical => a
                .manga
                .title
                .to_lowercase()
                .cmp(&b.manga.title.to_lowercase()),
            LibrarySort::LastRead => a.last_read.cmp(&b.last_read),
            LibrarySort::LastUpdate => a.manga.last_update.cmp(&b.manga.last_update),
            LibrarySort::UnreadCount => a.unread_count.cmp(&b.unread_count),
            LibrarySort::TotalChapters => a.total_chapters.cmp(&b.total_chapters),
            LibrarySort::LatestChapter => a.latest_upload.cmp(&b.latest_upload),
            LibrarySort::ChapterFetchDate => a.chapter_fetch_date.cmp(&b.chapter_fetch_date),
            LibrarySort::DateAdded => a.manga.date_added.cmp(&b.manga.date_added),
            LibrarySort::Random => {
                shuffle_key(a.manga.id, seed).cmp(&shuffle_key(b.manga.id, seed))
            }
        };
        // Ties fall back to the title so the order never jitters between frames.
        let ordering = ordering.then_with(|| {
            a.manga
                .title
                .to_lowercase()
                .cmp(&b.manga.title.to_lowercase())
        });
        if ascending {
            ordering
        } else {
            ordering.reverse()
        }
    });
}

/// Deterministic per-entry key for the "random" sort.
fn shuffle_key(id: Id, seed: u64) -> u64 {
    let mut value = (id as u64) ^ seed.wrapping_mul(0x9e37_79b9_7f4a_7c15);
    value ^= value >> 33;
    value = value.wrapping_mul(0xff51_afd7_ed55_8ccd);
    value ^= value >> 29;
    value
}

fn badges_for(app: &App, row: &Row) -> Vec<(String, Color32)> {
    let palette = app.palette;
    let prefs = &app.prefs.library;
    let mut badges = Vec::new();

    if prefs.badge_unread && row.unread > 0 {
        badges.push((row.unread.to_string(), palette.accent));
    }
    if prefs.badge_downloaded && row.downloaded > 0 {
        badges.push((row.downloaded.to_string(), palette.success));
    }
    if prefs.badge_local && row.is_local {
        badges.push(("local".to_string(), palette.text_dim));
    }
    if prefs.badge_language
        && let Some(lang) = &row.lang
    {
        badges.push((lang.to_uppercase(), palette.text_dim));
    }
    badges
}

// ---------------------------------------------------------------------------
// Filter / sort / display sheet
// ---------------------------------------------------------------------------

/// Returns true when the sheet asks to be closed.
pub fn settings_sheet(app: &mut App, ui: &mut Ui) -> bool {
    let palette = app.palette;
    let mut close = false;
    let mut changed = false;

    ui.label(RichText::new("Library").size(17.0).strong());
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .max_height(440.0)
        .show(ui, |ui| {
            ui.label(RichText::new("FILTER").size(11.0).color(palette.text_dim));
            ui.add_space(4.0);
            let filters = &mut app.prefs.library.filters;
            changed |= widgets::tri_state_row(ui, &palette, "Downloaded", &mut filters.downloaded);
            changed |= widgets::tri_state_row(ui, &palette, "Unread", &mut filters.unread);
            changed |= widgets::tri_state_row(ui, &palette, "Started", &mut filters.started);
            changed |= widgets::tri_state_row(ui, &palette, "Bookmarked", &mut filters.bookmarked);
            changed |= widgets::tri_state_row(ui, &palette, "Completed", &mut filters.completed);
            changed |= widgets::tri_state_row(ui, &palette, "Tracked", &mut filters.tracked);

            ui.add_space(12.0);
            ui.label(RichText::new("SORT").size(11.0).color(palette.text_dim));
            ui.add_space(4.0);
            for sort in LibrarySort::ALL {
                let selected = app.prefs.library.sort == sort;
                let arrow = if !selected {
                    "     "
                } else if app.prefs.library.sort_ascending {
                    " ⬆ "
                } else {
                    " ⬇ "
                };
                let button = egui::Button::new(
                    RichText::new(format!("{arrow}{}", sort.label())).color(if selected {
                        palette.accent
                    } else {
                        palette.text
                    }),
                )
                .fill(Color32::TRANSPARENT)
                .min_size(vec2(ui.available_width(), 26.0));

                if ui.add(button).clicked() {
                    if selected {
                        app.prefs.library.sort_ascending = !app.prefs.library.sort_ascending;
                    } else {
                        app.prefs.library.sort = sort;
                    }
                    changed = true;
                }
            }

            ui.add_space(12.0);
            ui.label(RichText::new("DISPLAY").size(11.0).color(palette.text_dim));
            ui.add_space(4.0);
            for mode in LibraryDisplayMode::ALL {
                let selected = app.prefs.library.display_mode == mode;
                if ui.radio(selected, mode.label()).clicked() {
                    app.prefs.library.display_mode = mode;
                    changed = true;
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Grid density");
                if ui
                    .add(
                        egui::Slider::new(&mut app.prefs.library.columns, 3..=12).show_value(false),
                    )
                    .changed()
                {
                    changed = true;
                }
                ui.label(
                    RichText::new(app.prefs.library.columns.to_string()).color(palette.text_dim),
                );
            });

            ui.add_space(10.0);
            ui.label(RichText::new("BADGES").size(11.0).color(palette.text_dim));
            changed |= ui
                .checkbox(&mut app.prefs.library.badge_unread, "Unread count")
                .changed();
            changed |= ui
                .checkbox(&mut app.prefs.library.badge_downloaded, "Downloaded count")
                .changed();
            changed |= ui
                .checkbox(&mut app.prefs.library.badge_local, "Local source")
                .changed();
            changed |= ui
                .checkbox(&mut app.prefs.library.badge_language, "Language")
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.library.show_continue_button,
                    "Continue reading button",
                )
                .changed();
        });

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Reset filters").clicked() {
            app.prefs.library.filters = LibraryFilters::default();
            changed = true;
        }
        widgets::toolbar_actions(ui, |ui| {
            if ui.button("Done").clicked() {
                close = true;
            }
        });
    });

    if changed {
        app.prefs_changed();
    }
    close
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: Id, title: &str, unread: i64, added: i64) -> LibraryEntry {
        let mut manga = Manga::new(1, format!("/m/{id}"), title.into());
        manga.id = id;
        manga.favorite = true;
        manga.date_added = added;
        LibraryEntry {
            manga,
            category_ids: vec![Category::DEFAULT_ID],
            total_chapters: 10,
            unread_count: unread,
            downloaded_count: 0,
            has_started: false,
            bookmark_count: 0,
            latest_upload: 0,
            last_read: 0,
            chapter_fetch_date: 0,
            is_tracked: false,
        }
    }

    #[test]
    fn alphabetical_sort_respects_direction() {
        let a = entry(1, "Bleach", 0, 0);
        let b = entry(2, "Akira", 0, 0);
        let mut refs = vec![&a, &b];

        sort_entries(&mut refs, LibrarySort::Alphabetical, true, 0);
        assert_eq!(refs[0].manga.title, "Akira");

        sort_entries(&mut refs, LibrarySort::Alphabetical, false, 0);
        assert_eq!(refs[0].manga.title, "Bleach");
    }

    #[test]
    fn ties_fall_back_to_title_for_stability() {
        // Same unread count: order must be deterministic, not frame-dependent.
        let a = entry(1, "Zeta", 3, 0);
        let b = entry(2, "Alpha", 3, 0);
        let mut refs = vec![&a, &b];
        sort_entries(&mut refs, LibrarySort::UnreadCount, true, 0);
        assert_eq!(refs[0].manga.title, "Alpha");
    }

    #[test]
    fn random_sort_is_stable_for_a_given_seed() {
        let a = entry(1, "A", 0, 0);
        let b = entry(2, "B", 0, 0);
        let c = entry(3, "C", 0, 0);

        let order = |seed| {
            let mut refs = vec![&a, &b, &c];
            sort_entries(&mut refs, LibrarySort::Random, true, seed);
            refs.iter().map(|e| e.manga.id).collect::<Vec<_>>()
        };
        assert_eq!(order(42), order(42));
    }

    #[test]
    fn entries_without_categories_fall_into_default() {
        let mut e = entry(1, "X", 0, 0);
        e.category_ids.clear();
        assert!(entry_in_category(&e, Category::DEFAULT_ID));
        assert!(!entry_in_category(&e, 5));
    }

    #[test]
    fn filters_narrow_the_list() {
        let mut filters = LibraryFilters::default();
        let unread = entry(1, "A", 4, 0);
        let read = entry(2, "B", 0, 0);

        filters.unread = TriState::EnabledIs;
        assert!(filters.matches(&unread));
        assert!(!filters.matches(&read));

        filters.unread = TriState::EnabledNot;
        assert!(!filters.matches(&unread));
        assert!(filters.matches(&read));
    }
}
