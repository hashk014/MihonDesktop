//! Browsing one source: popular / latest / search, with the source's own filters.

use egui::{Color32, RichText, Ui, vec2};

use super::App;
use super::widgets::{self, COVER_ASPECT};
use crate::core::BrowseMode;
use crate::model::Id;
use crate::source::ext::display_name;
use crate::source::{FilterKind, FilterList, SManga};

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let source_id = app.source_browse.source_id;
    let Some(source) = app.core.sources.get(source_id) else {
        widgets::screen_header(app, ui, "Source unavailable", None);
        widgets::empty_state(
            ui,
            &palette,
            "⚑",
            "This source is not installed",
            "It may have been removed from your extensions folder.",
        );
        return;
    };
    let source_name = source.name().to_string();
    let source_lang = source.lang().to_string();
    let supports_latest = source.supports_latest();
    let has_filters = !app.source_browse.filters.is_empty();

    // -- toolbar ------------------------------------------------------------
    egui::Panel::top("source_top")
        .frame(super::theme::plain(14))
        .show(ui, |ui| {
            widgets::screen_header(app, ui, &source_name, Some(&source_lang));
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let labels: Vec<&str> = if supports_latest {
                    vec!["Popular", "Latest", "Search"]
                } else {
                    vec!["Popular", "Search"]
                };
                let selected = match app.source_browse.mode {
                    BrowseMode::Popular => 0,
                    BrowseMode::Latest => 1,
                    BrowseMode::Search => labels.len() - 1,
                };
                if let Some(index) = widgets::segmented(ui, &palette, &labels, selected) {
                    let mode = match (index, supports_latest) {
                        (0, _) => BrowseMode::Popular,
                        (1, true) => BrowseMode::Latest,
                        _ => BrowseMode::Search,
                    };
                    switch_mode(app, mode);
                }

                ui.add_space(8.0);
                let response = widgets::search_field(
                    ui,
                    &palette,
                    "Search this source",
                    &mut app.source_browse.query,
                );
                if response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    switch_mode(app, BrowseMode::Search);
                }

                widgets::toolbar_actions(ui, |ui| {
                    if has_filters {
                        let label = if app.source_browse.show_filters {
                            "Hide filters"
                        } else {
                            "Filters"
                        };
                        if ui.button(label).clicked() {
                            app.source_browse.show_filters = !app.source_browse.show_filters;
                        }
                    }
                });
            });
        });

    // -- filter side panel --------------------------------------------------
    if app.source_browse.show_filters && has_filters {
        egui::Panel::right("source_filters")
            .exact_size(280.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(palette.surface)
                    .inner_margin(egui::Margin::same(12)),
            )
            .show(ui, |ui| {
                ui.label(RichText::new("Filters").size(15.0).strong());
                ui.add_space(6.0);

                egui::ScrollArea::vertical()
                    .id_salt("filters_scroll")
                    .auto_shrink([false, false])
                    .max_height(ui.available_height() - 46.0)
                    .show(ui, |ui| {
                        let mut filters = std::mem::take(&mut app.source_browse.filters);
                        filter_list_ui(ui, &palette, &mut filters);
                        app.source_browse.filters = filters;
                    });

                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    if ui.button("Reset").clicked() {
                        app.source_browse.filters = app
                            .core
                            .sources
                            .get(source_id)
                            .map(|s| s.filters())
                            .unwrap_or_default();
                    }
                    if ui.button("Apply").clicked() {
                        switch_mode(app, BrowseMode::Search);
                    }
                });
            });
    }

    // -- results ------------------------------------------------------------
    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 6)))
        .show(ui, |ui| {
            if let Some(error) = app.source_browse.error.clone() {
                widgets::error_box(ui, &palette, &error);
                ui.add_space(8.0);
                if ui.button("Retry").clicked() {
                    let page = app.source_browse.page.max(1);
                    request_page(app, page);
                }
                ui.add_space(8.0);
            }

            if app.source_browse.items.is_empty() {
                if app.source_browse.loading {
                    ui.add_space(30.0);
                    ui.vertical_centered(|ui| {
                        widgets::spinner_row(ui, &palette, "Loading…");
                    });
                } else if app.source_browse.error.is_none() {
                    widgets::empty_state(
                        ui,
                        &palette,
                        "🔍",
                        "No results",
                        "Try a different search or adjust the filters.",
                    );
                }
                return;
            }

            results_grid(app, ui);
        });
}

fn results_grid(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let spacing = 10.0;
    let (columns, tile_width) =
        widgets::grid_columns(ui.available_width(), app.prefs.library.columns, spacing);
    let row_height = tile_width * COVER_ASPECT + 34.0 + spacing;

    let items: Vec<SManga> = app.source_browse.items.clone();
    let source_id = app.source_browse.source_id;
    let total_rows = items.len().div_ceil(columns);
    let mut open: Option<SManga> = None;

    egui::ScrollArea::vertical()
        .id_salt("source_results")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            for row_index in 0..total_rows {
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = spacing;
                    for column in 0..columns {
                        let Some(item) = items.get(row_index * columns + column) else {
                            break;
                        };
                        let in_library = app
                            .core
                            .db
                            .find_manga(source_id, &item.url)
                            .map(|m| m.favorite)
                            .unwrap_or(false);

                        let response = widgets::cover_tile(
                            app,
                            ui,
                            tile_width,
                            item.thumbnail_url.as_deref(),
                            source_id,
                            &item.title,
                            true,
                            false,
                            &[],
                            in_library,
                        );
                        if response.clicked() {
                            open = Some(item.clone());
                        }
                    }
                });
                ui.add_space(spacing);
            }

            // Paging: a button rather than blind infinite scroll, so a source
            // is never hammered by an accidental fling.
            ui.add_space(6.0);
            ui.vertical_centered(|ui| {
                if app.source_browse.loading {
                    widgets::spinner_row(ui, &palette, "Loading more…");
                } else if app.source_browse.has_next {
                    if ui.button("Load more").clicked() {
                        let next = app.source_browse.page + 1;
                        request_page(app, next);
                    }
                } else {
                    ui.label(
                        RichText::new("End of results")
                            .size(12.0)
                            .color(palette.text_dim),
                    );
                }
            });
            ui.add_space(12.0);
        });

    let _ = row_height;

    if let Some(item) = open {
        open_result(app, source_id, &item);
    }
}

/// Stores the entry locally (if new) and opens its details screen.
pub fn open_result(app: &mut App, source_id: Id, item: &SManga) {
    match app.core.intern_manga(source_id, item) {
        Ok(manga) => app.open_manga(manga.id),
        Err(err) => app.toast_error(format!("could not open this entry: {err}")),
    }
}

fn switch_mode(app: &mut App, mode: BrowseMode) {
    app.source_browse.mode = mode;
    app.source_browse.items.clear();
    app.source_browse.error = None;
    request_page(app, 1);
}

fn request_page(app: &mut App, page: u32) {
    app.source_browse.loading = true;
    app.source_browse.error = None;
    let source_id = app.source_browse.source_id;
    let mode = app.source_browse.mode;
    let query = app.source_browse.query.clone();
    let filters = app.source_browse.filters.clone();
    app.core.browse(source_id, mode, page, query, filters);
}

// ---------------------------------------------------------------------------
// Filter rendering
// ---------------------------------------------------------------------------

pub fn filter_list_ui(ui: &mut Ui, palette: &super::theme::Palette, filters: &mut FilterList) {
    for filter in filters.iter_mut() {
        let label = display_name(&filter.name).to_string();
        match &mut filter.kind {
            FilterKind::Header => {
                ui.add_space(4.0);
                ui.label(
                    RichText::new(label.to_uppercase())
                        .size(11.0)
                        .color(palette.text_dim),
                );
            }
            FilterKind::Separator => {
                ui.add_space(4.0);
                ui.separator();
            }
            FilterKind::Text { value } => {
                ui.label(RichText::new(&label).size(12.0).color(palette.text_dim));
                ui.add(
                    egui::TextEdit::singleline(value)
                        .desired_width(f32::INFINITY)
                        .hint_text(&label),
                );
                ui.add_space(4.0);
            }
            FilterKind::CheckBox { checked, .. } => {
                ui.checkbox(checked, &label);
            }
            FilterKind::Tri { state, .. } => {
                widgets::tri_state_row(ui, palette, &label, state);
            }
            FilterKind::Select { options, index, .. } => {
                ui.label(RichText::new(&label).size(12.0).color(palette.text_dim));
                let current = options
                    .get(*index)
                    .cloned()
                    .unwrap_or_else(|| "—".to_string());
                egui::ComboBox::from_id_salt(format!("select_{label}"))
                    .selected_text(current)
                    .width(ui.available_width())
                    .show_ui(ui, |ui| {
                        for (option_index, option) in options.iter().enumerate() {
                            ui.selectable_value(index, option_index, option);
                        }
                    });
                ui.add_space(4.0);
            }
            FilterKind::Sort {
                options,
                index,
                ascending,
                ..
            } => {
                ui.label(RichText::new(&label).size(12.0).color(palette.text_dim));
                let current = options
                    .get(*index)
                    .cloned()
                    .unwrap_or_else(|| "—".to_string());
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt(format!("sort_{label}"))
                        .selected_text(current)
                        .width(ui.available_width() - 44.0)
                        .show_ui(ui, |ui| {
                            for (option_index, option) in options.iter().enumerate() {
                                ui.selectable_value(index, option_index, option);
                            }
                        });
                    let arrow = if *ascending { "⬆" } else { "⬇" };
                    if ui
                        .button(arrow)
                        .on_hover_text(if *ascending {
                            "Ascending"
                        } else {
                            "Descending"
                        })
                        .clicked()
                    {
                        *ascending = !*ascending;
                    }
                });
                ui.add_space(4.0);
            }
            FilterKind::Group { children } => {
                egui::CollapsingHeader::new(&label)
                    .id_salt(format!("group_{label}"))
                    .show(ui, |ui| {
                        filter_list_ui(ui, palette, children);
                    });
            }
        }
    }
}

/// Convenience used by the migrate flow: a compact source picker.
pub fn source_picker(app: &App, ui: &mut Ui, exclude: Id) -> Option<Id> {
    let palette = app.palette;
    let mut chosen = None;
    egui::ScrollArea::vertical()
        .max_height(320.0)
        .id_salt("source_picker")
        .show(ui, |ui| {
            for source in app.core.sources.visible(&app.prefs) {
                if source.id() == exclude {
                    continue;
                }
                let button = egui::Button::new(format!("{} ({})", source.name(), source.lang()))
                    .fill(Color32::TRANSPARENT)
                    .min_size(vec2(ui.available_width(), 26.0));
                if ui.add(button).clicked() {
                    chosen = Some(source.id());
                }
            }
            let _ = palette;
        });
    chosen
}
