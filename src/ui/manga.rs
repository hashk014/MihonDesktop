//! Manga details: header, description, actions and the chapter list.

use egui::{Align, Color32, Layout, RichText, Ui, vec2};

use super::widgets::{self, COVER_ASPECT};
use super::{App, Dialog};
use crate::download::DownloadState;
use crate::model::*;
use crate::source::local;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let Some(manga) = app.details.manga.clone() else {
        egui::Panel::top("manga_top")
            .frame(super::theme::header_frame(&app.palette))
            .show(ui, |ui| {
                widgets::screen_header(app, ui, "Loading…", None);
            });
        ui.centered_and_justified(|ui| {
            widgets::spinner_row(ui, &palette, "Fetching details…");
        });
        return;
    };

    egui::Panel::top("manga_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            widgets::screen_header(app, ui, "Details", None);
        });

    // -- selection toolbar --------------------------------------------------
    if !app.details.selection.is_empty() {
        egui::Panel::top("chapter_selection")
            .frame(
                egui::Frame::NONE
                    .fill(palette.accent.gamma_multiply(0.18))
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| chapter_selection_bar(app, ui));
    }

    let chapters = visible_chapters(app, &manga);

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 0)))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("manga_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    header_block(app, ui, &manga);
                    ui.add_space(10.0);
                    chapter_toolbar(app, ui, &manga, chapters.len());
                    ui.add_space(4.0);

                    if let Some(error) = app.details.error.clone() {
                        widgets::error_box(ui, &palette, &error);
                        ui.add_space(8.0);
                    }

                    if chapters.is_empty() {
                        if app.details.loading {
                            widgets::spinner_row(ui, &palette, "Loading chapters…");
                        } else {
                            ui.add_space(20.0);
                            ui.vertical_centered(|ui| {
                                ui.label(
                                    RichText::new("No chapters to show").color(palette.text_dim),
                                );
                            });
                        }
                    } else {
                        for chapter in &chapters {
                            chapter_row(app, ui, &manga, chapter);
                        }
                    }
                    ui.add_space(24.0);
                });
        });
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

fn header_block(app: &mut App, ui: &mut Ui, manga: &Manga) {
    let palette = app.palette;

    ui.horizontal_top(|ui| {
        let cover_width = 150.0;
        let (rect, _) = ui.allocate_exact_size(
            vec2(cover_width, cover_width * COVER_ASPECT),
            egui::Sense::hover(),
        );
        widgets::paint_cover(
            app,
            ui,
            rect,
            manga.thumbnail_url.as_deref(),
            manga.source,
            &manga.title,
        );

        ui.add_space(14.0);
        ui.vertical(|ui| {
            ui.label(RichText::new(&manga.title).size(22.0).strong());
            ui.add_space(2.0);

            if let Some(author) = manga.author_artist() {
                ui.label(RichText::new(author).size(13.0).color(palette.text_dim));
            }

            let source_name = app
                .core
                .sources
                .get(manga.source)
                .map(|s| s.name().to_string())
                .unwrap_or_else(|| "Unknown source".into());
            ui.label(
                RichText::new(format!("{} · {}", manga.status.label(), source_name))
                    .size(12.5)
                    .color(palette.text_dim),
            );

            ui.add_space(10.0);
            action_buttons(app, ui, manga);
            ui.add_space(10.0);

            description_block(app, ui, manga);
        });
    });

    genre_chips(app, ui, manga);
}

fn action_buttons(app: &mut App, ui: &mut Ui, manga: &Manga) {
    let palette = app.palette;
    let manga_id = manga.id;

    ui.horizontal_wrapped(|ui| {
        // Add / remove from library
        let (label, fill, text_colour) = if manga.favorite {
            ("♥  In library", palette.accent, palette.on_accent)
        } else {
            ("♡  Add to library", palette.surface_alt, palette.text)
        };
        if ui
            .add(egui::Button::new(RichText::new(label).color(text_colour)).fill(fill))
            .clicked()
        {
            toggle_favorite(app, manga);
        }

        if ui.button("🔄  Refresh").clicked() {
            app.details.loading = true;
            app.core.refresh_manga(manga_id, true);
        }

        if ui.button("📥  Download").clicked() {
            let unread: Vec<Id> = app
                .core
                .db
                .chapters_of(manga_id)
                .into_iter()
                .filter(|c| !c.read && !app.core.downloads.is_downloaded(manga_id, c.id))
                .map(|c| c.id)
                .collect();
            if unread.is_empty() {
                app.toast("nothing left to download");
            } else {
                app.core.queue_downloads(manga_id, &unread);
            }
        }

        if manga.favorite && ui.button("Categories").clicked() {
            let current = app.core.db.categories_of(manga_id);
            app.dialog = Some(Dialog::CategoryPicker {
                manga_ids: vec![manga_id],
                selected: current.into_iter().collect(),
            });
        }

        if ui.button("Notes").clicked() {
            app.details.notes_open = !app.details.notes_open;
        }

        if manga.source != local::LOCAL_ID && ui.button("Open in browser").clicked() {
            let url = app
                .core
                .sources
                .get(manga.source)
                .map(|s| s.web_url(&crate::download::to_smanga(manga)))
                .unwrap_or_default();
            if url.is_empty() {
                app.toast("this source has no web page");
            } else if let Err(err) = open_url(&url) {
                app.toast_error(format!("could not open the browser: {err}"));
            }
        }

        if manga.favorite && ui.button("Migrate").clicked() {
            app.source_browse.source_id = 0;
            app.dialog = Some(Dialog::MigrateTarget { manga_id });
        }
    });

    // Continue / start reading
    let next = app.core.db.next_unread_chapter(manga_id);
    if let Some(chapter) = next {
        let any_read = app.details.chapters.iter().any(|c| c.read);
        let label = if any_read {
            format!("▶  Continue — {}", chapter.name)
        } else {
            format!("▶  Start reading — {}", chapter.name)
        };
        ui.add_space(6.0);
        if ui
            .add(
                egui::Button::new(RichText::new(label).color(palette.on_accent))
                    .fill(palette.accent),
            )
            .clicked()
        {
            app.open_reader(manga_id, chapter.id);
        }
    }

    if app.details.notes_open {
        ui.add_space(8.0);
        ui.label(RichText::new("Notes").size(12.0).color(palette.text_dim));
        let changed = ui
            .add(
                egui::TextEdit::multiline(&mut app.details.notes_buffer)
                    .desired_width(f32::INFINITY)
                    .desired_rows(3)
                    .hint_text("Private notes about this entry"),
            )
            .changed();
        if changed {
            let notes = app.details.notes_buffer.clone();
            let _ = app.core.db.update_manga(manga_id, |m| m.notes = notes);
        }
    }
}

fn description_block(app: &mut App, ui: &mut Ui, manga: &Manga) {
    let palette = app.palette;
    let Some(description) = manga.description.as_ref().filter(|d| !d.trim().is_empty()) else {
        return;
    };

    let expanded = app.details.description_expanded;
    let text = if expanded {
        description.clone()
    } else {
        // Collapsed: roughly three lines' worth.
        let mut short: String = description.chars().take(280).collect();
        if description.chars().count() > 280 {
            short.push('…');
        }
        short
    };

    ui.label(RichText::new(text).size(13.0).color(palette.text));
    if description.chars().count() > 280 {
        let label = if expanded { "Show less" } else { "Show more" };
        if ui
            .add(egui::Button::new(RichText::new(label).color(palette.accent)).frame(false))
            .clicked()
        {
            app.details.description_expanded = !expanded;
        }
    }
}

fn genre_chips(app: &mut App, ui: &mut Ui, manga: &Manga) {
    let palette = app.palette;
    let Some(genres) = manga.genre.clone() else {
        return;
    };
    if genres.is_empty() {
        return;
    }

    ui.add_space(10.0);
    ui.horizontal_wrapped(|ui| {
        for genre in genres {
            if widgets::chip(ui, &palette, &genre)
                .interact(egui::Sense::click())
                .on_hover_text("Search this source for the genre")
                .clicked()
            {
                let source_id = manga.source;
                app.open_source(source_id);
                app.source_browse.query = genre.clone();
                app.source_browse.mode = crate::core::BrowseMode::Search;
                app.core.browse(
                    source_id,
                    crate::core::BrowseMode::Search,
                    1,
                    genre.clone(),
                    Vec::new(),
                );
            }
        }
    });
}

fn toggle_favorite(app: &mut App, manga: &Manga) {
    let becoming_favorite = !manga.favorite;
    let manga_id = manga.id;

    let outcome = app.core.db.update_manga(manga_id, |m| {
        m.favorite = becoming_favorite;
        if becoming_favorite && m.date_added == 0 {
            m.date_added = now_millis();
        }
    });

    match outcome {
        Ok(_) => {
            if becoming_favorite {
                // File it into the default category, or ask when configured to.
                let default = app.prefs.library.default_category;
                if app.prefs.library.prompt_for_category {
                    app.dialog = Some(Dialog::CategoryPicker {
                        manga_ids: vec![manga_id],
                        selected: Default::default(),
                    });
                } else {
                    let category = if default >= 0 {
                        default
                    } else {
                        Category::DEFAULT_ID
                    };
                    let _ = app.core.db.set_categories(manga_id, vec![category]);
                }
                app.toast("added to library");
                // A freshly added entry usually has no chapters yet.
                if app.details.chapters.is_empty() {
                    app.details.loading = true;
                    app.core.refresh_manga(manga_id, true);
                }
            } else {
                app.toast("removed from library");
            }
            app.invalidate_all();
            app.refresh_details();
        }
        Err(err) => app.toast_error(format!("could not update the library: {err}")),
    }
}

// ---------------------------------------------------------------------------
// Chapter list
// ---------------------------------------------------------------------------

fn chapter_toolbar(app: &mut App, ui: &mut Ui, manga: &Manga, shown: usize) {
    let palette = app.palette;
    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{shown} chapter(s)"))
                .size(13.0)
                .strong(),
        );
        let filtered = manga.unread_filter().is_enabled()
            || manga.downloaded_filter().is_enabled()
            || manga.bookmarked_filter().is_enabled();
        if filtered {
            ui.label(RichText::new("filtered").size(11.0).color(palette.accent));
        }
        widgets::toolbar_actions(ui, |ui| {
            if ui
                .button("⚙")
                .on_hover_text("Sort and filter chapters")
                .clicked()
            {
                app.dialog = Some(Dialog::ChapterSettings);
            }
        });
    });
}

fn chapter_row(app: &mut App, ui: &mut Ui, manga: &Manga, chapter: &Chapter) {
    let palette = app.palette;
    let manga_id = manga.id;
    let selected = app.details.selection.contains(&chapter.id);
    let downloaded = app.core.downloads.is_downloaded(manga_id, chapter.id);
    let download_state = app.core.downloads.state_of(chapter.id);

    let title_colour = if chapter.read {
        palette.text_dim
    } else {
        palette.text
    };

    // A button inside the row also lands inside the row's own click area, so it
    // has to claim the click or the reader would open on top of the action.
    let mut consumed = false;

    let response = widgets::clickable_row(ui, |ui| {
        super::theme::card(&palette)
            .fill(if selected {
                palette.accent.gamma_multiply(0.2)
            } else {
                palette.surface
            })
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    if chapter.bookmark {
                        ui.label(RichText::new("🔖").size(12.0).color(palette.accent));
                    }

                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(chapter.display_name(manga.display_chapter_number()))
                                .color(title_colour),
                        );

                        let mut meta = Vec::new();
                        if chapter.date_upload > 0 {
                            meta.push(widgets::format_timestamp(
                                chapter.date_upload,
                                app.prefs.relative_timestamps,
                            ));
                        }
                        if let Some(scanlator) = &chapter.scanlator {
                            meta.push(scanlator.clone());
                        }
                        if !chapter.read && chapter.last_page_read > 0 {
                            meta.push(format!("page {}", chapter.last_page_read + 1));
                        }
                        if !meta.is_empty() {
                            ui.label(
                                RichText::new(meta.join(" · "))
                                    .size(11.0)
                                    .color(palette.text_dim),
                            );
                        }
                    });

                    ui.with_layout(
                        Layout::right_to_left(Align::Center),
                        |ui| match &download_state {
                            Some(DownloadState::Running { done, total }) => {
                                ui.add(
                                    egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                                        .desired_width(60.0)
                                        .desired_height(6.0),
                                );
                            }
                            Some(DownloadState::Queued) => {
                                ui.label(
                                    RichText::new("queued").size(11.0).color(palette.text_dim),
                                );
                            }
                            Some(DownloadState::Failed(_)) => {
                                ui.label(RichText::new("failed").size(11.0).color(palette.error));
                            }
                            _ => {
                                if downloaded {
                                    if ui
                                        .button("🗑")
                                        .on_hover_text("Downloaded — click to delete")
                                        .clicked()
                                    {
                                        consumed = true;
                                        app.core.downloads.delete_chapter(manga_id, chapter.id);
                                        app.invalidate_all();
                                    }
                                } else if ui
                                    .button("📥")
                                    .on_hover_text("Download this chapter")
                                    .clicked()
                                {
                                    consumed = true;
                                    app.core.queue_downloads(manga_id, &[chapter.id]);
                                }
                            }
                        },
                    );
                });
            });
    })
    .response;

    if response.clicked() && !consumed {
        if app.details.selection.is_empty() {
            app.open_reader(manga_id, chapter.id);
        } else {
            toggle_chapter_selection(app, chapter.id);
        }
    }
    if response.secondary_clicked() {
        toggle_chapter_selection(app, chapter.id);
    }
}

fn toggle_chapter_selection(app: &mut App, chapter_id: Id) {
    if !app.details.selection.remove(&chapter_id) {
        app.details.selection.insert(chapter_id);
    }
}

fn chapter_selection_bar(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let selected: Vec<Id> = app.details.selection.iter().copied().collect();
    let manga_id = app.details.manga_id;

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} selected", selected.len()))
                .strong()
                .color(palette.text),
        );
        ui.add_space(10.0);

        if ui.button("Mark read").clicked() {
            apply_to_chapters(app, &selected, |c| {
                c.read = true;
                c.last_page_read = 0;
            });
        }
        if ui.button("Mark unread").clicked() {
            apply_to_chapters(app, &selected, |c| {
                c.read = false;
                c.last_page_read = 0;
            });
        }
        if ui.button("Mark previous read").clicked() {
            mark_previous_read(app, &selected);
        }
        if ui.button("Bookmark").clicked() {
            apply_to_chapters(app, &selected, |c| c.bookmark = true);
        }
        if ui.button("Remove bookmark").clicked() {
            apply_to_chapters(app, &selected, |c| c.bookmark = false);
        }
        if ui.button("Download").clicked() {
            app.core.queue_downloads(manga_id, &selected);
        }
        if ui.button("Delete download").clicked() {
            for chapter_id in &selected {
                app.core.downloads.delete_chapter(manga_id, *chapter_id);
            }
            app.invalidate_all();
        }

        widgets::toolbar_actions(ui, |ui| {
            if ui.button("✖").on_hover_text("Clear selection").clicked() {
                app.details.selection.clear();
            }
            if ui.button("Select all").clicked() {
                let all: Vec<Id> = app.details.chapters.iter().map(|c| c.id).collect();
                app.details.selection.extend(all);
            }
        });
    });
}

fn apply_to_chapters(app: &mut App, ids: &[Id], mut f: impl FnMut(&mut Chapter)) {
    match app.core.db.update_chapters(ids, &mut f) {
        Ok(_) => {
            app.refresh_details();
            app.invalidate_all();
        }
        Err(err) => app.toast_error(format!("could not update chapters: {err}")),
    }
}

/// Marks everything ordered before the lowest selected chapter as read.
fn mark_previous_read(app: &mut App, selected: &[Id]) {
    let chapters = app.details.chapters.clone();
    let threshold = chapters
        .iter()
        .filter(|c| selected.contains(&c.id))
        .map(|c| c.chapter_number)
        .fold(f64::MAX, f64::min);

    if threshold == f64::MAX {
        return;
    }
    let ids: Vec<Id> = chapters
        .iter()
        .filter(|c| c.chapter_number >= 0.0 && c.chapter_number < threshold)
        .map(|c| c.id)
        .collect();
    apply_to_chapters(app, &ids, |c| {
        c.read = true;
        c.last_page_read = 0;
    });
}

/// Applies the manga's stored filter and sort flags to the chapter list.
pub fn visible_chapters(app: &App, manga: &Manga) -> Vec<Chapter> {
    let mut chapters: Vec<Chapter> = app
        .details
        .chapters
        .iter()
        .filter(|c| manga.unread_filter().matches(!c.read))
        .filter(|c| {
            manga
                .downloaded_filter()
                .matches(app.core.downloads.is_downloaded(manga.id, c.id))
        })
        .filter(|c| manga.bookmarked_filter().matches(c.bookmark))
        .filter(|c| !app.prefs.downloaded_only || app.core.downloads.is_downloaded(manga.id, c.id))
        .cloned()
        .collect();

    order_chapters(
        &mut chapters,
        manga.chapter_sort_mode(),
        manga.sort_descending(),
    );
    chapters
}

/// Orders a chapter list in place.
///
/// Every mode is first put in "oldest first" order so the direction flag means
/// the same thing everywhere; sources list newest first, hence the reversed
/// source order. Descending is the stored default, putting the newest chapter
/// at the top like upstream.
pub fn order_chapters(chapters: &mut [Chapter], mode: ChapterSortMode, descending: bool) {
    match mode {
        ChapterSortMode::SourceOrder => chapters.sort_by_key(|c| std::cmp::Reverse(c.source_order)),
        ChapterSortMode::ChapterNumber => {
            chapters.sort_by(|a, b| a.chapter_number.total_cmp(&b.chapter_number))
        }
        ChapterSortMode::UploadDate => chapters.sort_by_key(|c| c.date_upload),
        ChapterSortMode::Alphabet => chapters.sort_by(|a, b| local::natural_cmp(&a.name, &b.name)),
    }
    if descending {
        chapters.reverse();
    }
}

// ---------------------------------------------------------------------------
// Chapter sort / filter sheet
// ---------------------------------------------------------------------------

pub fn chapter_settings_sheet(app: &mut App, ui: &mut Ui) -> bool {
    let palette = app.palette;
    let mut close = false;
    let Some(mut manga) = app.details.manga.clone() else {
        return true;
    };
    let mut changed = false;

    ui.label(RichText::new("Chapters").size(17.0).strong());
    ui.add_space(8.0);

    ui.label(RichText::new("FILTER").size(11.0).color(palette.text_dim));
    let mut unread = manga.unread_filter();
    if widgets::tri_state_row(ui, &palette, "Unread", &mut unread) {
        manga.set_unread_filter(unread);
        changed = true;
    }
    let mut downloaded = manga.downloaded_filter();
    if widgets::tri_state_row(ui, &palette, "Downloaded", &mut downloaded) {
        manga.set_downloaded_filter(downloaded);
        changed = true;
    }
    let mut bookmarked = manga.bookmarked_filter();
    if widgets::tri_state_row(ui, &palette, "Bookmarked", &mut bookmarked) {
        manga.set_bookmarked_filter(bookmarked);
        changed = true;
    }

    ui.add_space(10.0);
    ui.label(RichText::new("SORT").size(11.0).color(palette.text_dim));
    for mode in ChapterSortMode::ALL {
        let selected = manga.chapter_sort_mode() == mode;
        let arrow = if !selected {
            "     "
        } else if manga.sort_descending() {
            " ⬇ "
        } else {
            " ⬆ "
        };
        let button = egui::Button::new(RichText::new(format!("{arrow}{}", mode.label())).color(
            if selected {
                palette.accent
            } else {
                palette.text
            },
        ))
        .fill(Color32::TRANSPARENT)
        .min_size(vec2(ui.available_width(), 26.0));

        if ui.add(button).clicked() {
            if selected {
                let descending = manga.sort_descending();
                manga.set_sort_descending(!descending);
            } else {
                manga.set_chapter_sort_mode(mode);
            }
            changed = true;
        }
    }

    ui.add_space(10.0);
    ui.label(RichText::new("DISPLAY").size(11.0).color(palette.text_dim));
    let by_number = manga.display_chapter_number();
    if ui.radio(!by_number, "Source title").clicked() && by_number {
        manga.set_display_chapter_number(false);
        changed = true;
    }
    if ui.radio(by_number, "Chapter number").clicked() && !by_number {
        manga.set_display_chapter_number(true);
        changed = true;
    }

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Done").clicked() {
            close = true;
        }
    });

    if changed {
        let chapter_flags = manga.chapter_flags;
        match app
            .core
            .db
            .update_manga(manga.id, |m| m.chapter_flags = chapter_flags)
        {
            Ok(_) => app.refresh_details(),
            Err(err) => app.toast_error(format!("could not save chapter settings: {err}")),
        }
    }
    close
}

/// Opens a URL in the system browser.
pub fn open_url(url: &str) -> std::io::Result<()> {
    std::process::Command::new("cmd")
        .args(["/C", "start", "", url])
        .spawn()
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manga_with(flags: i64) -> Manga {
        let mut manga = Manga::new(1, "/m/1".into(), "Test".into());
        manga.id = 1;
        manga.chapter_flags = flags;
        manga
    }

    #[test]
    fn sort_flags_round_trip() {
        let mut manga = manga_with(0);
        manga.set_chapter_sort_mode(ChapterSortMode::UploadDate);
        assert_eq!(manga.chapter_sort_mode(), ChapterSortMode::UploadDate);

        manga.set_sort_descending(false);
        assert!(!manga.sort_descending());
        manga.set_sort_descending(true);
        assert!(manga.sort_descending());

        // Changing the direction must not disturb the sort mode.
        assert_eq!(manga.chapter_sort_mode(), ChapterSortMode::UploadDate);
    }

    #[test]
    fn filter_flags_are_independent() {
        let mut manga = manga_with(0);
        manga.set_unread_filter(TriState::EnabledIs);
        manga.set_bookmarked_filter(TriState::EnabledNot);

        assert_eq!(manga.unread_filter(), TriState::EnabledIs);
        assert_eq!(manga.bookmarked_filter(), TriState::EnabledNot);
        assert_eq!(manga.downloaded_filter(), TriState::Disabled);

        manga.set_unread_filter(TriState::Disabled);
        assert_eq!(manga.unread_filter(), TriState::Disabled);
        assert_eq!(manga.bookmarked_filter(), TriState::EnabledNot);
    }

    fn chapters() -> Vec<Chapter> {
        // As a source returns them: newest first, so source_order 0 is newest.
        [(3.0, 0, 300), (2.0, 1, 200), (1.0, 2, 100)]
            .into_iter()
            .map(|(number, order, date)| {
                let mut chapter = Chapter::new(1, format!("/c/{number}"), format!("Ch.{number}"));
                chapter.id = order + 1;
                chapter.chapter_number = number;
                chapter.source_order = order;
                chapter.date_upload = date;
                chapter
            })
            .collect()
    }

    #[test]
    fn default_ordering_puts_the_newest_chapter_first() {
        let mut list = chapters();
        // Default flags: source order, descending.
        order_chapters(&mut list, ChapterSortMode::SourceOrder, true);
        assert_eq!(list[0].chapter_number, 3.0);
        assert_eq!(list[2].chapter_number, 1.0);
    }

    #[test]
    fn ascending_reverses_every_mode_consistently() {
        for mode in ChapterSortMode::ALL {
            let mut descending = chapters();
            order_chapters(&mut descending, mode, true);
            let mut ascending = chapters();
            order_chapters(&mut ascending, mode, false);

            let descending: Vec<f64> = descending.iter().map(|c| c.chapter_number).collect();
            let mut reversed = ascending
                .iter()
                .map(|c| c.chapter_number)
                .collect::<Vec<_>>();
            reversed.reverse();
            assert_eq!(descending, reversed, "mode {mode:?} is inconsistent");
            assert_eq!(
                descending[0], 3.0,
                "mode {mode:?} should lead with the newest"
            );
        }
    }

    #[test]
    fn display_flag_does_not_clobber_sorting() {
        let mut manga = manga_with(0);
        manga.set_chapter_sort_mode(ChapterSortMode::Alphabet);
        manga.set_display_chapter_number(true);
        assert!(manga.display_chapter_number());
        assert_eq!(manga.chapter_sort_mode(), ChapterSortMode::Alphabet);
    }
}
