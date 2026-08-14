//! The Browse tab: source list, global search, extensions and migration.

use std::collections::BTreeMap;

use egui::{Color32, RichText, Ui, vec2};

use super::widgets::{self, COVER_ASPECT};
use super::{App, BrowseTab, GlobalSearchResult};
use crate::model::{Category, Id};
use crate::source::{SManga, local};

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("browse_top")
        .frame(super::theme::plain(14))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Browse").size(20.0).strong());
                ui.add_space(12.0);

                let labels = ["Sources", "Extensions", "Migrate"];
                let selected = match app.browse.tab {
                    BrowseTab::Sources => 0,
                    BrowseTab::Extensions => 1,
                    BrowseTab::Migrate => 2,
                };
                if let Some(index) = widgets::segmented(ui, &palette, &labels, selected) {
                    app.browse.tab = match index {
                        0 => BrowseTab::Sources,
                        1 => BrowseTab::Extensions,
                        _ => BrowseTab::Migrate,
                    };
                }
            });

            if app.browse.tab == BrowseTab::Sources {
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let response = widgets::search_field(
                        ui,
                        &palette,
                        "Search every source",
                        &mut app.browse.query,
                    );
                    let submitted =
                        response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                    if submitted {
                        start_global_search(app);
                    }
                    if ui.button("Search").clicked() {
                        start_global_search(app);
                    }
                    if !app.browse.results.is_empty() && ui.button("Clear").clicked() {
                        app.browse.results.clear();
                        app.browse.submitted_query.clear();
                    }
                });
            }
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 6)))
        .show(ui, |ui| match app.browse.tab {
            BrowseTab::Sources => {
                if app.browse.results.is_empty() {
                    sources_list(app, ui);
                } else {
                    global_search_results(app, ui);
                }
            }
            BrowseTab::Extensions => super::extensions::show_inline(app, ui),
            BrowseTab::Migrate => migrate_list(app, ui),
        });
}

// ---------------------------------------------------------------------------
// Source list
// ---------------------------------------------------------------------------

fn sources_list(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let sources = app.core.sources.visible(&app.prefs);

    if sources.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            "🌐",
            "No sources enabled",
            "Install an extension, or enable one in Settings → Browse.",
        );
        return;
    }

    // Pinned first, then grouped by language, like upstream's source list.
    let mut pinned: Vec<(Id, String, String)> = Vec::new();
    let mut by_language: BTreeMap<String, Vec<(Id, String, String)>> = BTreeMap::new();

    for source in &sources {
        let row = (
            source.id(),
            source.name().to_string(),
            source.lang().to_string(),
        );
        if app.prefs.is_source_pinned(source.id()) {
            pinned.push(row.clone());
        }
        let group = if source.lang() == local::LOCAL_LANG {
            "Local".to_string()
        } else {
            source.lang().to_uppercase()
        };
        by_language.entry(group).or_default().push(row);
    }

    egui::ScrollArea::vertical()
        .id_salt("sources_list")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !pinned.is_empty() {
                section_label(ui, &palette, "PINNED");
                for (id, name, lang) in &pinned {
                    source_row(app, ui, *id, name, lang);
                }
                ui.add_space(10.0);
            }

            for (language, mut rows) in by_language {
                rows.sort_by_key(|a| a.1.to_lowercase());
                section_label(ui, &palette, &language);
                for (id, name, lang) in &rows {
                    source_row(app, ui, *id, name, lang);
                }
                ui.add_space(10.0);
            }
        });
}

fn section_label(ui: &mut Ui, palette: &super::theme::Palette, text: &str) {
    ui.add_space(4.0);
    ui.label(RichText::new(text).size(11.0).color(palette.text_dim));
    ui.add_space(2.0);
}

fn source_row(app: &mut App, ui: &mut Ui, source_id: Id, name: &str, lang: &str) {
    let palette = app.palette;
    let pinned = app.prefs.is_source_pinned(source_id);

    let response = widgets::clickable_row(ui, |ui| {
        super::theme::card(&palette).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                // A coloured initial stands in for the extension icon.
                let (rect, _) = ui.allocate_exact_size(vec2(34.0, 34.0), egui::Sense::hover());
                let hue = (source_id.unsigned_abs() % 360) as f32;
                let colour = egui::ecolor::Hsva::new(hue / 360.0, 0.45, 0.65, 1.0);
                ui.painter().rect_filled(
                    rect,
                    egui::CornerRadius::same(super::theme::RADIUS_SMALL),
                    Color32::from(colour),
                );
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    name.chars()
                        .next()
                        .unwrap_or('?')
                        .to_uppercase()
                        .to_string(),
                    egui::FontId::proportional(16.0),
                    Color32::WHITE,
                );

                ui.add_space(8.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(name).strong());
                    ui.label(
                        RichText::new(if lang == local::LOCAL_LANG {
                            "on this computer".to_string()
                        } else {
                            lang.to_uppercase()
                        })
                        .size(11.5)
                        .color(palette.text_dim),
                    );
                });

                widgets::toolbar_actions(ui, |ui| {
                    let pin_label = if pinned { "★" } else { "☆" };
                    if ui
                        .button(pin_label)
                        .on_hover_text(if pinned { "Unpin" } else { "Pin" })
                        .clicked()
                    {
                        app.prefs.toggle_pinned(source_id);
                        app.prefs_changed();
                    }
                    if ui.button("Latest").clicked() {
                        app.open_source(source_id);
                        app.source_browse.mode = crate::core::BrowseMode::Latest;
                        app.core.browse(
                            source_id,
                            crate::core::BrowseMode::Latest,
                            1,
                            String::new(),
                            Vec::new(),
                        );
                    }
                });
            });
        });
    })
    .response;

    if response.clicked() {
        app.open_source(source_id);
    }
}

// ---------------------------------------------------------------------------
// Global search
// ---------------------------------------------------------------------------

fn start_global_search(app: &mut App) {
    let query = app.browse.query.trim().to_string();
    if query.is_empty() {
        app.browse.results.clear();
        return;
    }

    let sources = app.core.sources.visible(&app.prefs);
    let targets: Vec<_> = if app.prefs.browse.search_pinned_only
        && sources.iter().any(|s| app.prefs.is_source_pinned(s.id()))
    {
        sources
            .into_iter()
            .filter(|s| app.prefs.is_source_pinned(s.id()))
            .collect()
    } else {
        sources
    };

    if targets.is_empty() {
        app.toast("no sources to search");
        return;
    }

    app.browse.submitted_query = query.clone();
    app.browse.results = targets
        .iter()
        .map(|source| {
            (
                source.id(),
                GlobalSearchResult {
                    source_name: source.name().to_string(),
                    loading: true,
                    items: Vec::new(),
                    error: None,
                },
            )
        })
        .collect();

    let ids: Vec<Id> = targets.iter().map(|s| s.id()).collect();
    app.core.global_search(ids, query);
}

fn global_search_results(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let query = app.browse.submitted_query.clone();
    let order: Vec<Id> = app.browse.results.keys().copied().collect();
    let mut open: Option<(Id, SManga)> = None;

    egui::ScrollArea::vertical()
        .id_salt("global_results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.label(
                RichText::new(format!("Results for “{query}”"))
                    .size(12.5)
                    .color(palette.text_dim),
            );
            ui.add_space(6.0);

            for source_id in order {
                let (name, loading, error, items) = {
                    let Some(result) = app.browse.results.get(&source_id) else {
                        continue;
                    };
                    (
                        result.source_name.clone(),
                        result.loading,
                        result.error.clone(),
                        result.items.clone(),
                    )
                };

                ui.horizontal(|ui| {
                    ui.label(RichText::new(&name).strong());
                    if !items.is_empty() {
                        ui.label(
                            RichText::new(format!("{} result(s)", items.len()))
                                .size(11.5)
                                .color(palette.text_dim),
                        );
                    }
                    widgets::toolbar_actions(ui, |ui| {
                        if ui.small_button("Open source").clicked() {
                            app.open_source(source_id);
                            app.source_browse.query = query.clone();
                            app.source_browse.mode = crate::core::BrowseMode::Search;
                            app.core.browse(
                                source_id,
                                crate::core::BrowseMode::Search,
                                1,
                                query.clone(),
                                Vec::new(),
                            );
                        }
                    });
                });

                if loading {
                    widgets::spinner_row(ui, &palette, "Searching…");
                } else if let Some(error) = error {
                    ui.label(
                        RichText::new(error)
                            .size(11.5)
                            .color(palette.error.gamma_multiply(0.9)),
                    );
                } else if items.is_empty() {
                    ui.label(
                        RichText::new("No results")
                            .size(11.5)
                            .color(palette.text_dim),
                    );
                } else {
                    // One horizontal strip per source, like upstream.
                    egui::ScrollArea::horizontal()
                        .id_salt(("strip", source_id))
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                for item in items.iter().take(20) {
                                    let response = widgets::cover_tile(
                                        app,
                                        ui,
                                        108.0,
                                        item.thumbnail_url.as_deref(),
                                        source_id,
                                        &item.title,
                                        true,
                                        false,
                                        &[],
                                        false,
                                    );
                                    if response.clicked() {
                                        open = Some((source_id, item.clone()));
                                    }
                                }
                            });
                        });
                }
                ui.add_space(14.0);
            }
        });

    if let Some((source_id, item)) = open {
        super::source_browse::open_result(app, source_id, &item);
    }
}

// ---------------------------------------------------------------------------
// Migration
// ---------------------------------------------------------------------------

fn migrate_list(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    // Group the library by source: migration always starts from "this source".
    let mut counts: BTreeMap<Id, usize> = BTreeMap::new();
    for entry in &app.library.entries {
        *counts.entry(entry.manga.source).or_default() += 1;
    }

    if counts.is_empty() {
        widgets::empty_state(
            ui,
            &palette,
            "🔀",
            "Nothing to migrate",
            "Migration moves library entries from one source to another.",
        );
        return;
    }

    egui::ScrollArea::vertical()
        .id_salt("migrate_list")
        .auto_shrink([false, false])
        .show(ui, |ui| match app.browse.migrate_source {
            None => {
                ui.label(
                    RichText::new("Pick the source to migrate away from")
                        .size(12.5)
                        .color(palette.text_dim),
                );
                ui.add_space(6.0);
                for (source_id, count) in counts {
                    let name = app
                        .core
                        .sources
                        .get(source_id)
                        .map(|s| s.name().to_string())
                        .unwrap_or_else(|| format!("Source {source_id} (not installed)"));

                    let response = super::theme::card(&palette)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(name).strong());
                                widgets::toolbar_actions(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!(
                                            "{count} entr{}",
                                            if count == 1 { "y" } else { "ies" }
                                        ))
                                        .size(11.5)
                                        .color(palette.text_dim),
                                    );
                                });
                            });
                        })
                        .response
                        .interact(egui::Sense::click());
                    if response.clicked() {
                        app.browse.migrate_source = Some(source_id);
                    }
                }
            }
            Some(source_id) => {
                ui.horizontal(|ui| {
                    if ui.button("⬅ Sources").clicked() {
                        app.browse.migrate_source = None;
                    }
                    let name = app
                        .core
                        .sources
                        .get(source_id)
                        .map(|s| s.name().to_string())
                        .unwrap_or_else(|| "Unknown source".into());
                    ui.label(RichText::new(name).strong());
                });
                ui.add_space(8.0);

                let entries: Vec<(Id, String, Option<String>)> = app
                    .library
                    .entries
                    .iter()
                    .filter(|e| e.manga.source == source_id)
                    .map(|e| {
                        (
                            e.manga.id,
                            e.manga.title.clone(),
                            e.manga.thumbnail_url.clone(),
                        )
                    })
                    .collect();

                for (manga_id, title, cover) in entries {
                    let response = super::theme::card(&palette)
                        .show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                let (rect, _) = ui.allocate_exact_size(
                                    vec2(28.0, 28.0 * COVER_ASPECT),
                                    egui::Sense::hover(),
                                );
                                widgets::paint_cover(
                                    app,
                                    ui,
                                    rect,
                                    cover.as_deref(),
                                    source_id,
                                    &title,
                                );
                                ui.add_space(6.0);
                                ui.label(&title);
                                widgets::toolbar_actions(ui, |ui| {
                                    if ui.button("Migrate").clicked() {
                                        app.dialog =
                                            Some(super::Dialog::MigrateTarget { manga_id });
                                    }
                                });
                            });
                        })
                        .response;
                    let _ = response;
                }
            }
        });
}

/// The migration dialog: pick a target source, then a matching entry.
/// Returns true when it should close.
pub fn migrate_sheet(app: &mut App, ui: &mut Ui, manga_id: Id) -> bool {
    let palette = app.palette;
    let mut close = false;

    let Some(manga) = app.core.db.get_manga(manga_id) else {
        return true;
    };

    ui.label(RichText::new("Migrate entry").size(17.0).strong());
    ui.add_space(4.0);
    ui.label(
        RichText::new(&manga.title)
            .size(12.5)
            .color(palette.text_dim),
    );
    ui.add_space(10.0);

    // Reuse the source-browse state as the search scratchpad for the target.
    let target = app.source_browse.source_id;
    let picked_target = target != 0 && target != manga.source;

    if !picked_target {
        ui.label(
            RichText::new("Choose the source to migrate to")
                .size(12.0)
                .color(palette.text_dim),
        );
        ui.add_space(6.0);
        if let Some(source_id) = super::source_browse::source_picker(app, ui, manga.source) {
            app.source_browse = super::SourceBrowseState {
                source_id,
                mode: crate::core::BrowseMode::Search,
                query: manga.title.clone(),
                loading: true,
                filters: app
                    .core
                    .sources
                    .get(source_id)
                    .map(|s| s.filters())
                    .unwrap_or_default(),
                ..Default::default()
            };
            app.core.browse(
                source_id,
                crate::core::BrowseMode::Search,
                1,
                manga.title.clone(),
                Vec::new(),
            );
        }
    } else {
        let source_name = app
            .core
            .sources
            .get(target)
            .map(|s| s.name().to_string())
            .unwrap_or_default();
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("Target: {source_name}")).strong());
            if ui.small_button("Change").clicked() {
                app.source_browse.source_id = 0;
            }
        });
        ui.add_space(6.0);

        if app.source_browse.loading {
            widgets::spinner_row(ui, &palette, "Searching the target source…");
        } else if let Some(error) = app.source_browse.error.clone() {
            widgets::error_box(ui, &palette, &error);
        } else if app.source_browse.items.is_empty() {
            ui.label(RichText::new("No match found").color(palette.text_dim));
        } else {
            let items = app.source_browse.items.clone();
            let mut chosen: Option<SManga> = None;
            egui::ScrollArea::vertical()
                .max_height(300.0)
                .id_salt("migrate_candidates")
                .show(ui, |ui| {
                    for item in items.iter().take(30) {
                        let button = egui::Button::new(&item.title)
                            .fill(Color32::TRANSPARENT)
                            .min_size(vec2(ui.available_width(), 26.0));
                        if ui.add(button).clicked() {
                            chosen = Some(item.clone());
                        }
                    }
                });
            if let Some(item) = chosen {
                match perform_migration(app, manga_id, target, &item) {
                    Ok(new_id) => {
                        app.toast("entry migrated");
                        app.source_browse.source_id = 0;
                        app.invalidate_all();
                        app.open_manga(new_id);
                        close = true;
                    }
                    Err(err) => app.toast_error(format!("migration failed: {err}")),
                }
            }
        }
    }

    ui.add_space(10.0);
    if ui.button("Cancel").clicked() {
        app.source_browse.source_id = 0;
        close = true;
    }
    close
}

/// Moves favourite status, categories and read progress onto the new entry.
fn perform_migration(
    app: &mut App,
    manga_id: Id,
    target_source: Id,
    target: &SManga,
) -> anyhow::Result<Id> {
    let old = app
        .core
        .db
        .get_manga(manga_id)
        .ok_or_else(|| anyhow::anyhow!("the original entry disappeared"))?;

    let new_manga = app.core.intern_manga(target_source, target)?;

    let categories = app.core.db.categories_of(manga_id);
    let categories = if categories.is_empty() {
        vec![Category::DEFAULT_ID]
    } else {
        categories
    };
    app.core.db.set_categories(new_manga.id, categories)?;

    // Carry read progress across by chapter number, which is the only thing
    // two different sources reliably agree on.
    let old_chapters = app.core.db.chapters_of(manga_id);
    let highest_read = old_chapters
        .iter()
        .filter(|c| c.read)
        .map(|c| c.chapter_number)
        .fold(f64::MIN, f64::max);

    app.core.db.update_manga(new_manga.id, |m| {
        m.favorite = true;
        m.date_added = old.date_added.max(1);
    })?;
    app.core.db.update_manga(manga_id, |m| m.favorite = false)?;

    if highest_read > f64::MIN {
        let ids: Vec<Id> = app
            .core
            .db
            .chapters_of(new_manga.id)
            .into_iter()
            .filter(|c| c.chapter_number >= 0.0 && c.chapter_number <= highest_read)
            .map(|c| c.id)
            .collect();
        app.core.db.update_chapters(&ids, |c| c.read = true)?;
    }

    // Pull the new entry's details and chapter list in the background.
    app.core.refresh_manga(new_manga.id, true);
    Ok(new_manga.id)
}
