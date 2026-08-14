//! The More tab and every settings page.

use egui::{Align, Color32, Layout, RichText, Ui, vec2};

use super::widgets;
use super::{App, Dialog, Route, SettingsPage};
use crate::backup::{self, Backup};
use crate::model::*;
use crate::prefs::{AppTheme, CardStyle, Density, NavStyle, ThemeMode};

// ---------------------------------------------------------------------------
// The "More" tab
// ---------------------------------------------------------------------------

pub fn show_more(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("more_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            ui.label(RichText::new("More").size(20.0).strong());
        });

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("more_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Two global switches, as in upstream's More screen.
                    super::theme::card(&palette).show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        let mut changed = false;
                        changed |= ui
                            .checkbox(&mut app.prefs.downloaded_only, "Downloaded only")
                            .on_hover_text("Hide anything that is not downloaded")
                            .changed();
                        changed |= ui
                            .checkbox(&mut app.prefs.incognito, "Incognito mode")
                            .on_hover_text("Stop recording history and reading progress")
                            .changed();
                        if changed {
                            app.prefs_changed();
                            app.library.dirty = true;
                        }
                    });

                    ui.add_space(10.0);
                    let queued = app.core.downloads.queue_len();
                    let entries = [
                        (
                            "📥",
                            "Download queue",
                            if queued > 0 {
                                format!("{queued} in the queue")
                            } else {
                                "Nothing queued".to_string()
                            },
                            Route::Downloads,
                        ),
                        (
                            "🏷",
                            "Categories",
                            format!("{} configured", app.core.db.categories().len()),
                            Route::Categories,
                        ),
                        (
                            "◎",
                            "Extensions",
                            format!("{} installed", app.extensions.installed.len()),
                            Route::Extensions,
                        ),
                        (
                            "▣",
                            "Statistics",
                            "Library at a glance".to_string(),
                            Route::Statistics,
                        ),
                        (
                            "⚙",
                            "Settings",
                            "Appearance, reader, downloads…".to_string(),
                            Route::Settings(SettingsPage::Root),
                        ),
                    ];

                    for (glyph, title, subtitle, route) in entries {
                        if nav_card(ui, &palette, glyph, title, &subtitle) {
                            app.push(route);
                        }
                    }
                });
        });
}

fn nav_card(
    ui: &mut Ui,
    palette: &super::theme::Palette,
    glyph: &str,
    title: &str,
    subtitle: &str,
) -> bool {
    super::theme::card(palette)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(glyph).size(17.0).color(palette.accent));
                ui.add_space(6.0);
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).strong());
                    ui.label(RichText::new(subtitle).size(11.5).color(palette.text_dim));
                });
                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                    ui.label(RichText::new("›").color(palette.text_dim));
                });
            });
        })
        .response
        .interact(egui::Sense::click())
        .clicked()
}

// ---------------------------------------------------------------------------
// Settings pages
// ---------------------------------------------------------------------------

pub fn show(app: &mut App, ui: &mut Ui, page: SettingsPage) {
    let palette = app.palette;

    egui::Panel::top("settings_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            widgets::screen_header(app, ui, page.title(), None);
        });

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt(("settings", page.title()))
                .auto_shrink([false, false])
                .show(ui, |ui| match page {
                    SettingsPage::Root => root_page(app, ui),
                    SettingsPage::Appearance => appearance_page(app, ui),
                    SettingsPage::Library => library_page(app, ui),
                    SettingsPage::Reader => reader_page(app, ui),
                    SettingsPage::Downloads => downloads_page(app, ui),
                    SettingsPage::Browse => browse_page(app, ui),
                    SettingsPage::DataStorage => data_page(app, ui),
                    SettingsPage::About => about_page(app, ui),
                });
            let _ = palette;
        });
}

fn root_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let pages = [
        (SettingsPage::Appearance, "Theme, accent colour, scaling"),
        (SettingsPage::Library, "Categories, global update, badges"),
        (
            SettingsPage::Reader,
            "Default reading mode and page handling",
        ),
        (
            SettingsPage::Downloads,
            "Location, concurrency, storage format",
        ),
        (
            SettingsPage::Browse,
            "Sources, languages, extension repositories",
        ),
        (SettingsPage::DataStorage, "Backup, restore, caches"),
        (SettingsPage::About, "Version and data locations"),
    ];
    for (page, subtitle) in pages {
        if nav_card(ui, &palette, "›", page.title(), subtitle) {
            app.push(Route::Settings(page));
        }
    }
}

fn section(ui: &mut Ui, palette: &super::theme::Palette, title: &str) {
    ui.add_space(10.0);
    ui.label(
        RichText::new(title.to_uppercase())
            .size(11.0)
            .color(palette.text_dim),
    );
    ui.add_space(4.0);
}

fn appearance_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let mut changed = false;

    // The preview is drawn from the *pending* preferences, so a slider being
    // dragged is visible before the frame that installs the new style.
    preview_card(ui, &app.prefs);

    section(ui, &palette, "Theme");
    ui.horizontal(|ui| {
        for mode in ThemeMode::ALL {
            if ui
                .selectable_label(app.prefs.theme_mode == mode, mode.label())
                .clicked()
            {
                app.prefs.theme_mode = mode;
                changed = true;
            }
        }
    });
    ui.add_space(6.0);
    changed |= ui
        .checkbox(&mut app.prefs.pure_black, "Pure black backgrounds")
        .on_hover_text("Saves power on OLED panels, and looks better in the dark")
        .changed();

    section(ui, &palette, "Accent");
    ui.horizontal_wrapped(|ui| {
        for theme in AppTheme::ALL {
            let ([r, g, b], _) = theme.accent();
            let selected = app.prefs.app_theme == theme && app.prefs.custom_accent.is_none();
            if swatch(ui, &palette, Color32::from_rgb(r, g, b), selected)
                .on_hover_text(theme.label())
                .clicked()
            {
                app.prefs.app_theme = theme;
                app.prefs.custom_accent = None;
                changed = true;
            }
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        let mut custom = app.prefs.custom_accent.unwrap_or_else(|| {
            let ([r, g, b], _) = app.prefs.app_theme.accent();
            [r, g, b]
        });
        if ui
            .color_edit_button_srgb(&mut custom)
            .on_hover_text("Pick any colour")
            .changed()
        {
            app.prefs.custom_accent = Some(custom);
            changed = true;
        }
        ui.label("Custom colour");
        if app.prefs.custom_accent.is_some() && ui.button("Use a preset").clicked() {
            app.prefs.custom_accent = None;
            changed = true;
        }
    });

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        ui.label("Accent bleed");
        changed |= ui
            .add(
                egui::Slider::new(&mut app.prefs.theme_tint, 0.0..=1.0)
                    .step_by(0.05)
                    .show_value(false),
            )
            .on_hover_text("How much of the accent colour tints backgrounds and cards")
            .changed();
    });

    section(ui, &palette, "Shape");
    ui.horizontal(|ui| {
        ui.label("Corners");
        let mut radius = app.prefs.corner_radius as f32;
        if ui
            .add(
                egui::Slider::new(&mut radius, 0.0..=18.0)
                    .step_by(1.0)
                    .show_value(false),
            )
            .on_hover_text("Square through to fully rounded")
            .changed()
        {
            app.prefs.corner_radius = radius as u8;
            changed = true;
        }
        ui.label(
            RichText::new(match app.prefs.corner_radius {
                0..=2 => "Square",
                3..=8 => "Soft",
                9..=13 => "Rounded",
                _ => "Pill",
            })
            .color(palette.text_dim),
        );
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Cards");
        let labels: Vec<&str> = CardStyle::ALL.iter().map(|s| s.label()).collect();
        let current = CardStyle::ALL
            .iter()
            .position(|s| *s == app.prefs.card_style)
            .unwrap_or(0);
        if let Some(index) = widgets::segmented(ui, &palette, &labels, current) {
            app.prefs.card_style = CardStyle::ALL[index];
            changed = true;
        }
    });

    section(ui, &palette, "Layout");
    ui.horizontal(|ui| {
        ui.label("Navigation");
        let labels: Vec<&str> = NavStyle::ALL.iter().map(|s| s.label()).collect();
        let current = NavStyle::ALL
            .iter()
            .position(|s| *s == app.prefs.nav_style)
            .unwrap_or(0);
        if let Some(index) = widgets::segmented(ui, &palette, &labels, current) {
            app.prefs.nav_style = NavStyle::ALL[index];
            changed = true;
        }
    });

    ui.add_space(6.0);
    ui.horizontal(|ui| {
        ui.label("Density");
        let labels: Vec<&str> = Density::ALL.iter().map(|d| d.label()).collect();
        let current = Density::ALL
            .iter()
            .position(|d| *d == app.prefs.density)
            .unwrap_or(1);
        if let Some(index) = widgets::segmented(ui, &palette, &labels, current) {
            app.prefs.density = Density::ALL[index];
            changed = true;
        }
    });

    section(ui, &palette, "Display");
    ui.horizontal(|ui| {
        ui.label("Interface scale");
        changed |= ui
            .add(egui::Slider::new(&mut app.prefs.ui_scale, 0.7..=2.0).step_by(0.05))
            .on_hover_text("Zooms the whole window, like a browser would")
            .changed();
    });
    ui.horizontal(|ui| {
        ui.label("Text size");
        changed |= ui
            .add(
                egui::Slider::new(&mut app.prefs.font_scale, 0.8..=1.5)
                    .step_by(0.05)
                    .fixed_decimals(2),
            )
            .on_hover_text("Only the text, leaving controls where they are")
            .changed();
    });
    changed |= ui
        .checkbox(&mut app.prefs.animations, "Animations")
        .on_hover_text("Sliding selections and fades")
        .changed();
    changed |= ui
        .checkbox(&mut app.prefs.relative_timestamps, "Relative timestamps")
        .on_hover_text("Show “2 h ago” instead of a date")
        .changed();

    ui.horizontal(|ui| {
        ui.label("Start on");
        egui::ComboBox::from_id_salt("start_tab")
            .selected_text(super::Tab::from_index(app.prefs.start_tab).label())
            .show_ui(ui, |ui| {
                for tab in super::Tab::ALL {
                    if ui
                        .selectable_label(app.prefs.start_tab == tab.index(), tab.label())
                        .clicked()
                    {
                        app.prefs.start_tab = tab.index();
                        changed = true;
                    }
                }
            });
    });

    ui.add_space(14.0);
    if ui.button("Reset appearance").clicked() {
        app.prefs.reset_appearance();
        changed = true;
    }
    ui.add_space(6.0);

    if changed {
        app.prefs_changed();
    }
}

/// A round colour chip used to pick a theme.
fn swatch(
    ui: &mut Ui,
    palette: &super::theme::Palette,
    colour: Color32,
    selected: bool,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(vec2(30.0, 30.0), egui::Sense::click());
    let round = egui::CornerRadius::same(15);
    let painter = ui.painter();
    painter.rect_filled(rect.shrink(if selected { 4.0 } else { 0.0 }), round, colour);
    if selected {
        painter.rect_stroke(
            rect,
            round,
            egui::Stroke::new(2.0, colour),
            egui::StrokeKind::Inside,
        );
    } else if response.hovered() {
        painter.rect_stroke(
            rect,
            round,
            egui::Stroke::new(2.0, palette.text.gamma_multiply(0.5)),
            egui::StrokeKind::Outside,
        );
    }
    response
}

fn shadow_shape(palette: &super::theme::Palette, rect: egui::Rect) -> egui::Shape {
    palette.shadow().as_shape(rect, palette.corner()).into()
}

/// A miniature of the app, painted with the preferences as they stand right
/// now rather than with the style installed at the start of the frame.
///
/// Without it, half of these settings — tint, density, card style — only show
/// their effect on screens you have to navigate away from this one to see.
fn preview_card(ui: &mut Ui, prefs: &crate::prefs::Preferences) {
    let system_is_dark = ui
        .ctx()
        .system_theme()
        .map(|theme| theme == egui::Theme::Dark);
    let preview = super::theme::Palette::build(prefs, prefs.theme_mode.is_dark(system_is_dark));

    let height = 132.0;
    let (rect, _) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, preview.corner_large(), preview.background);
    painter.rect_stroke(
        rect,
        preview.corner_large(),
        egui::Stroke::new(1.0, preview.outline),
        egui::StrokeKind::Inside,
    );

    let pad = preview.space(10.0);
    let rail_wide = prefs.nav_style == NavStyle::Rail;
    let rail_size = if rail_wide { 62.0 } else { 44.0 };

    // The rail (or bar) with three entries, the first one selected. It is flush
    // against two edges of the card, so only the corners it actually shares
    // with them are rounded — rounding all four makes it float.
    let outer = preview.corner_large().nw;
    let (rail, body, rail_corner) = if prefs.nav_style == NavStyle::Bottom {
        let split = rect.bottom() - 34.0;
        (
            egui::Rect::from_min_max(egui::pos2(rect.left(), split), rect.max),
            egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), split)),
            egui::CornerRadius {
                nw: 0,
                ne: 0,
                sw: outer,
                se: outer,
            },
        )
    } else {
        let split = rect.left() + rail_size;
        (
            egui::Rect::from_min_max(rect.min, egui::pos2(split, rect.bottom())),
            egui::Rect::from_min_max(egui::pos2(split, rect.top()), rect.max),
            egui::CornerRadius {
                nw: outer,
                sw: outer,
                ne: 0,
                se: 0,
            },
        )
    };
    painter.rect_filled(rail, rail_corner, preview.surface);

    let glyphs = ["📚", "🔄", "🕘"];
    for (index, glyph) in glyphs.iter().enumerate() {
        let slot = if prefs.nav_style == NavStyle::Bottom {
            let width = rail.width() / glyphs.len() as f32;
            egui::Rect::from_min_size(
                egui::pos2(rail.left() + width * index as f32, rail.top()),
                vec2(width, rail.height()),
            )
        } else {
            let step = 34.0;
            egui::Rect::from_min_size(
                egui::pos2(rail.left(), rail.top() + pad + step * index as f32),
                vec2(rail.width(), step),
            )
        };
        if index == 0 {
            painter.rect_filled(
                slot.shrink(3.0),
                preview.corner(),
                preview.accent.gamma_multiply(0.22),
            );
        }
        painter.text(
            slot.center(),
            egui::Align2::CENTER_CENTER,
            glyph,
            egui::FontId::proportional(13.0),
            if index == 0 {
                preview.accent
            } else {
                preview.text_dim
            },
        );
    }

    // A card holding a cover placeholder, two lines of text and a badge.
    let card = egui::Rect::from_min_max(
        egui::pos2(body.left() + pad, body.top() + pad),
        egui::pos2(body.right() - pad, body.top() + pad + 62.0),
    );
    match prefs.card_style {
        CardStyle::Outlined => {
            painter.rect_stroke(
                card,
                preview.corner(),
                egui::Stroke::new(1.0, preview.outline),
                egui::StrokeKind::Inside,
            );
        }
        CardStyle::Elevated => {
            painter.add(shadow_shape(&preview, card));
            painter.rect_filled(card, preview.corner(), preview.surface);
        }
        CardStyle::Flat => {
            painter.rect_filled(card, preview.corner(), preview.surface);
        }
    }

    let cover = egui::Rect::from_min_size(
        egui::pos2(card.left() + 8.0, card.top() + 8.0),
        vec2(30.0, 46.0),
    );
    painter.rect_filled(cover, preview.corner_small(), preview.surface_alt);
    painter.text(
        cover.center(),
        egui::Align2::CENTER_CENTER,
        "MD",
        egui::FontId::proportional(11.0 * prefs.font_scale),
        preview.text_dim,
    );

    let text_left = cover.right() + 8.0;
    painter.text(
        egui::pos2(text_left, card.top() + 12.0),
        egui::Align2::LEFT_CENTER,
        "Chapter 41",
        egui::FontId::proportional(13.0 * prefs.font_scale),
        preview.text,
    );
    painter.text(
        egui::pos2(text_left, card.top() + 28.0),
        egui::Align2::LEFT_CENTER,
        "2 h ago · MangaDex",
        egui::FontId::proportional(10.5 * prefs.font_scale),
        preview.text_dim,
    );

    // The accent, as a filled button and as a badge.
    let button = egui::Rect::from_min_size(
        egui::pos2(text_left, card.bottom() - 22.0),
        vec2(58.0, 17.0),
    );
    painter.rect_filled(button, preview.corner_small(), preview.accent);
    painter.text(
        button.center(),
        egui::Align2::CENTER_CENTER,
        "Read",
        egui::FontId::proportional(10.5 * prefs.font_scale),
        preview.on_accent,
    );
    let badge = egui::Rect::from_min_size(
        egui::pos2(card.right() - 34.0, card.top() + 8.0),
        vec2(26.0, 15.0),
    );
    painter.rect_filled(badge, preview.corner_small(), preview.accent);
    painter.text(
        badge.center(),
        egui::Align2::CENTER_CENTER,
        "12",
        egui::FontId::proportional(9.5 * prefs.font_scale),
        preview.on_accent,
    );

    // A strip of library tiles below, so grid spacing reacts to density too.
    // Capped at eight: beyond that they are too narrow to read as covers.
    let gap = preview.space(8.0);
    let tiles_top = card.bottom() + gap;
    let tile_height = (body.bottom() - pad - tiles_top).max(0.0);
    if tile_height > 12.0 {
        let tile_width = tile_height / widgets::COVER_ASPECT;
        let fit = ((card.width() + gap) / (tile_width + gap)).floor().max(0.0) as usize;
        for index in 0..fit.min(8) {
            let tile = egui::Rect::from_min_size(
                egui::pos2(card.left() + (tile_width + gap) * index as f32, tiles_top),
                vec2(tile_width, tile_height),
            );
            painter.rect_filled(tile, preview.corner(), preview.surface_alt);
        }
    }

    ui.add_space(4.0);
}

fn library_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let mut changed = false;

    section(ui, &palette, "Categories");
    if ui.button("Manage categories").clicked() {
        app.push(Route::Categories);
    }
    let categories = app.core.db.categories();
    ui.horizontal(|ui| {
        ui.label("Default category for new entries");
        let current = if app.prefs.library.default_category < 0 {
            "Always ask".to_string()
        } else {
            categories
                .iter()
                .find(|c| c.id == app.prefs.library.default_category)
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Default".into())
        };
        egui::ComboBox::from_id_salt("default_category")
            .selected_text(current)
            .show_ui(ui, |ui| {
                if ui.selectable_label(false, "Always ask").clicked() {
                    app.prefs.library.default_category = -1;
                    changed = true;
                }
                for category in &categories {
                    if ui.selectable_label(false, &category.name).clicked() {
                        app.prefs.library.default_category = category.id;
                        changed = true;
                    }
                }
            });
    });
    changed |= ui
        .checkbox(
            &mut app.prefs.library.prompt_for_category,
            "Ask for categories when adding an entry",
        )
        .changed();

    section(ui, &palette, "Global update");
    ui.horizontal(|ui| {
        ui.label("Automatic check every");
        let mut hours = app.prefs.library.update_interval_hours;
        egui::ComboBox::from_id_salt("update_interval")
            .selected_text(if hours == 0 {
                "Never".to_string()
            } else {
                format!("{hours} h")
            })
            .show_ui(ui, |ui| {
                for option in [0u32, 6, 12, 24, 48, 72, 168] {
                    let label = if option == 0 {
                        "Never".to_string()
                    } else {
                        format!("{option} h")
                    };
                    if ui.selectable_label(hours == option, label).clicked() {
                        hours = option;
                        changed = true;
                    }
                }
            });
        app.prefs.library.update_interval_hours = hours;
    });

    changed |= ui
        .checkbox(
            &mut app.prefs.library.skip_completed_entries,
            "Skip completed series",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.library.skip_entries_with_unread,
            "Skip entries with unread chapters",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.library.skip_unstarted_entries,
            "Skip entries that were never started",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.library.download_new_chapters,
            "Download new chapters automatically",
        )
        .changed();

    section(ui, &palette, "Categories included in updates");
    if categories.is_empty() {
        ui.label(RichText::new("No categories").color(palette.text_dim));
    }
    for category in &categories {
        let mut included = app.prefs.library.update_categories.is_empty()
            || app.prefs.library.update_categories.contains(&category.id);
        if ui.checkbox(&mut included, &category.name).changed() {
            let list = &mut app.prefs.library.update_categories;
            // An empty list means "everything", so materialise it on first change.
            if list.is_empty() {
                *list = categories.iter().map(|c| c.id).collect();
            }
            if included {
                if !list.contains(&category.id) {
                    list.push(category.id);
                }
            } else {
                list.retain(|id| *id != category.id);
            }
            changed = true;
        }
    }

    if changed {
        app.prefs_changed();
    }
}

fn reader_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let mut changed = false;

    section(ui, &palette, "Defaults for new series");
    ui.horizontal(|ui| {
        ui.label("Reading mode");
        egui::ComboBox::from_id_salt("default_mode")
            .selected_text(app.prefs.reader.default_reading_mode.label())
            .show_ui(ui, |ui| {
                for mode in ReadingMode::ALL {
                    if ui
                        .selectable_label(
                            app.prefs.reader.default_reading_mode == mode,
                            mode.label(),
                        )
                        .clicked()
                    {
                        app.prefs.reader.default_reading_mode = mode;
                        changed = true;
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Scale");
        egui::ComboBox::from_id_salt("default_scale")
            .selected_text(app.prefs.reader.scale_type.label())
            .show_ui(ui, |ui| {
                for scale in ImageScaleType::ALL {
                    if ui
                        .selectable_label(app.prefs.reader.scale_type == scale, scale.label())
                        .clicked()
                    {
                        app.prefs.reader.scale_type = scale;
                        changed = true;
                    }
                }
            });
    });
    ui.horizontal(|ui| {
        ui.label("Background");
        egui::ComboBox::from_id_salt("default_background")
            .selected_text(app.prefs.reader.background.label())
            .show_ui(ui, |ui| {
                for background in ReaderBackground::ALL {
                    if ui
                        .selectable_label(
                            app.prefs.reader.background == background,
                            background.label(),
                        )
                        .clicked()
                    {
                        app.prefs.reader.background = background;
                        changed = true;
                    }
                }
            });
    });

    section(ui, &palette, "Pages");
    changed |= ui
        .checkbox(&mut app.prefs.reader.crop_borders, "Crop borders")
        .changed();
    changed |= ui
        .checkbox(&mut app.prefs.reader.double_pages, "Double pages")
        .changed();
    changed |= ui
        .checkbox(&mut app.prefs.reader.show_page_number, "Show page number")
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.reader.keyboard_navigation,
            "Keyboard navigation",
        )
        .changed();
    ui.horizontal(|ui| {
        ui.label("Preload pages");
        changed |= ui
            .add(egui::Slider::new(
                &mut app.prefs.reader.preload_pages,
                0..=12,
            ))
            .changed();
    });

    section(ui, &palette, "Progress");
    changed |= ui
        .checkbox(
            &mut app.prefs.reader.mark_read_on_last_page,
            "Mark chapter read on the last page",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.reader.skip_read_chapters_on_finish,
            "Skip read chapters when moving forward",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.reader.remove_after_read,
            "Delete downloads after reading",
        )
        .changed();

    if changed {
        app.prefs_changed();
    }
}

fn downloads_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let mut changed = false;

    section(ui, &palette, "Location");
    let current = app.prefs.downloads_dir(&app.core.paths);
    ui.label(
        RichText::new(current.display().to_string())
            .size(12.0)
            .color(palette.text_dim),
    );
    ui.horizontal(|ui| {
        if ui.button("Change folder…").clicked()
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Choose the downloads folder")
                .pick_folder()
        {
            app.prefs.downloads.directory = Some(folder);
            changed = true;
            app.toast("restart the app to move the queue to the new folder");
        }
        if app.prefs.downloads.directory.is_some() && ui.button("Reset to default").clicked() {
            app.prefs.downloads.directory = None;
            changed = true;
        }
        if ui.button("Open folder").clicked()
            && let Err(err) = super::extensions::open_folder(&current)
        {
            app.toast_error(format!("could not open the folder: {err}"));
        }
    });

    section(ui, &palette, "Behaviour");
    ui.horizontal(|ui| {
        ui.label("Simultaneous downloads");
        if ui
            .add(egui::Slider::new(
                &mut app.prefs.downloads.concurrent_downloads,
                1..=8,
            ))
            .changed()
        {
            app.core
                .downloads
                .set_concurrency(app.prefs.downloads.concurrent_downloads as usize);
            changed = true;
        }
    });
    if ui
        .checkbox(
            &mut app.prefs.downloads.save_as_cbz,
            "Store chapters as CBZ archives",
        )
        .changed()
    {
        app.core
            .downloads
            .set_save_as_cbz(app.prefs.downloads.save_as_cbz);
        changed = true;
    }
    ui.horizontal(|ui| {
        ui.label("Download ahead while reading");
        changed |= ui
            .add(egui::Slider::new(
                &mut app.prefs.downloads.download_ahead,
                0..=10,
            ))
            .changed();
    });

    section(ui, &palette, "Storage");
    ui.label(
        RichText::new(format!(
            "{} used by downloads",
            widgets::format_bytes(app.core.downloads.storage_used())
        ))
        .color(palette.text_dim),
    );

    if changed {
        app.prefs_changed();
    }
}

fn browse_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let mut changed = false;

    section(ui, &palette, "Sources");
    changed |= ui
        .checkbox(
            &mut app.prefs.browse.show_nsfw_sources,
            "Show sources marked 18+",
        )
        .changed();
    changed |= ui
        .checkbox(
            &mut app.prefs.browse.search_pinned_only,
            "Global search only covers pinned sources",
        )
        .changed();

    section(ui, &palette, "Languages");
    let languages = app.core.sources.languages();
    ui.label(
        RichText::new("Leave everything unticked to show every language.")
            .size(11.5)
            .color(palette.text_dim),
    );
    ui.horizontal_wrapped(|ui| {
        for language in &languages {
            let mut enabled = app.prefs.browse.enabled_languages.contains(language);
            if ui.checkbox(&mut enabled, language.to_uppercase()).changed() {
                if enabled {
                    app.prefs.browse.enabled_languages.push(language.clone());
                } else {
                    app.prefs.browse.enabled_languages.retain(|l| l != language);
                }
                changed = true;
            }
        }
    });

    section(ui, &palette, "Enabled sources");
    let sources = app.core.sources.all();
    egui::ScrollArea::vertical()
        .id_salt("source_toggles")
        .max_height(260.0)
        .show(ui, |ui| {
            for source in &sources {
                let mut enabled = app.prefs.is_source_enabled(source.id());
                if ui
                    .checkbox(
                        &mut enabled,
                        format!("{} ({})", source.name(), source.lang()),
                    )
                    .changed()
                {
                    app.prefs.toggle_enabled(source.id());
                    changed = true;
                }
            }
        });

    section(ui, &palette, "Extension repositories");
    if ui.button("Manage extensions").clicked() {
        app.push(Route::Extensions);
    }
    if ui.button("Add repository").clicked() {
        app.dialog = Some(Dialog::AddRepo { url: String::new() });
    }

    section(ui, &palette, "Local source");
    ui.label(
        RichText::new(app.core.paths.local_source.display().to_string())
            .size(12.0)
            .color(palette.text_dim),
    );
    let extra: Vec<std::path::PathBuf> = app.prefs.local_source_dirs.clone();
    for dir in &extra {
        ui.horizontal(|ui| {
            ui.label(RichText::new(dir.display().to_string()).size(12.0));
            if ui.small_button("✖").clicked() {
                app.prefs.local_source_dirs.retain(|d| d != dir);
                changed = true;
            }
        });
    }
    ui.horizontal(|ui| {
        if ui.button("Add a folder…").clicked()
            && let Some(folder) = rfd::FileDialog::new()
                .set_title("Add a local library folder")
                .pick_folder()
            && !app.prefs.local_source_dirs.contains(&folder)
        {
            app.prefs.local_source_dirs.push(folder);
            changed = true;
        }
        if ui.button("Open local folder").clicked() {
            let path = app.core.paths.local_source.clone();
            if let Err(err) = super::extensions::open_folder(&path) {
                app.toast_error(format!("could not open the folder: {err}"));
            }
        }
    });

    if changed {
        app.core.refresh_local_source(&app.prefs);
        app.prefs_changed();
    }
}

fn data_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    section(ui, &palette, "Backup");
    ui.label(
        RichText::new("A backup holds your library, chapters, history and settings.")
            .size(12.0)
            .color(palette.text_dim),
    );
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.button("Create backup…").clicked() {
            create_backup(app);
        }
        if ui.button("Restore from file…").clicked() {
            restore_backup(app);
        }
    });

    section(ui, &palette, "Storage used");
    let cache = app.core.cache.size_bytes();
    let downloads = app.core.downloads.storage_used();
    ui.label(RichText::new(format!(
        "Image cache: {}",
        widgets::format_bytes(cache)
    )));
    ui.label(RichText::new(format!(
        "Downloads: {}",
        widgets::format_bytes(downloads)
    )));
    ui.label(RichText::new(format!(
        "Library: {} entries, {} in the queue",
        app.core.db.library_size(),
        app.core.downloads.queue_len()
    )));

    ui.add_space(8.0);
    ui.horizontal(|ui| {
        if ui.button("Clear image cache").clicked() {
            app.core.cache.clear(None);
            app.covers.clear();
            app.page_textures.clear();
            app.toast("image cache cleared");
        }
        if ui.button("Clear page cache only").clicked() {
            app.core.cache.clear(Some(crate::images::ImageKind::Page));
            app.page_textures.clear();
            app.toast("page cache cleared");
        }
    });

    section(ui, &palette, "Maintenance");
    if ui
        .button("Remove entries that are not in the library")
        .clicked()
    {
        let removed = purge_orphans(app);
        app.toast(format!(
            "{removed} entr{} removed",
            if removed == 1 { "y" } else { "ies" }
        ));
    }
}

fn about_page(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    section(ui, &palette, "Mihon Desktop");
    ui.label(
        RichText::new(format!("Version {}", env!("CARGO_PKG_VERSION"))).color(palette.text_dim),
    );
    ui.label(
        RichText::new(
            "A desktop reimplementation in Rust of the Mihon manga reader, \
             following its feature set and information architecture.",
        )
        .size(12.5)
        .color(palette.text_dim),
    );

    section(ui, &palette, "Locations");
    for (label, path) in [
        ("Data", app.core.paths.root.clone()),
        ("Downloads", app.prefs.downloads_dir(&app.core.paths)),
        ("Extensions", app.core.paths.extensions.clone()),
        ("Local source", app.core.paths.local_source.clone()),
    ] {
        ui.horizontal(|ui| {
            ui.label(RichText::new(format!("{label}:")).strong());
            ui.label(
                RichText::new(path.display().to_string())
                    .size(11.5)
                    .color(palette.text_dim),
            );
            if ui.small_button("Open").clicked() {
                let _ = super::extensions::open_folder(&path);
            }
        });
    }

    section(ui, &palette, "Upstream");
    ui.horizontal(|ui| {
        if ui.link("mihon.app").clicked() {
            let _ = super::manga::open_url("https://mihon.app/");
        }
        if ui.link("github.com/mihonapp/mihon").clicked() {
            let _ = super::manga::open_url("https://github.com/mihonapp/mihon");
        }
    });
}

// ---------------------------------------------------------------------------
// Statistics
// ---------------------------------------------------------------------------

pub fn show_statistics(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("stats_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            widgets::screen_header(app, ui, "Statistics", None);
        });

    let entries = &app.library.entries;
    let total = entries.len();
    let chapters: i64 = entries.iter().map(|e| e.total_chapters).sum();
    let unread: i64 = entries.iter().map(|e| e.unread_count).sum();
    let read = chapters - unread;
    let downloaded: i64 = entries.iter().map(|e| e.downloaded_count).sum();
    let completed = entries
        .iter()
        .filter(|e| e.manga.status == MangaStatus::Completed)
        .count();
    let time_read: i64 = app
        .core
        .db
        .recent_history(100_000, "")
        .iter()
        .map(|h| h.history.time_read)
        .sum();

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("stats_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.horizontal_wrapped(|ui| {
                        stat_tile(ui, &palette, "Entries", &total.to_string());
                        stat_tile(ui, &palette, "Chapters", &chapters.to_string());
                        stat_tile(ui, &palette, "Read", &read.max(0).to_string());
                        stat_tile(ui, &palette, "Unread", &unread.to_string());
                        stat_tile(ui, &palette, "Downloaded", &downloaded.to_string());
                        stat_tile(ui, &palette, "Completed", &completed.to_string());
                        stat_tile(
                            ui,
                            &palette,
                            "Time read",
                            &super::history::format_duration(time_read),
                        );
                    });

                    ui.add_space(14.0);
                    ui.label(
                        RichText::new("BY SOURCE")
                            .size(11.0)
                            .color(palette.text_dim),
                    );
                    ui.add_space(4.0);

                    let mut per_source: std::collections::BTreeMap<Id, usize> = Default::default();
                    for entry in entries {
                        *per_source.entry(entry.manga.source).or_default() += 1;
                    }
                    let max = per_source.values().copied().max().unwrap_or(1);
                    for (source_id, count) in per_source {
                        let name = app
                            .core
                            .sources
                            .get(source_id)
                            .map(|s| s.name().to_string())
                            .unwrap_or_else(|| "Not installed".into());
                        ui.horizontal(|ui| {
                            ui.add_sized(
                                vec2(160.0, 16.0),
                                egui::Label::new(RichText::new(name).size(12.0)),
                            );
                            ui.add(
                                egui::ProgressBar::new(count as f32 / max as f32)
                                    .desired_width(300.0)
                                    .desired_height(10.0)
                                    .text(count.to_string()),
                            );
                        });
                    }
                });
        });
}

fn stat_tile(ui: &mut Ui, palette: &super::theme::Palette, label: &str, value: &str) {
    super::theme::card(palette).show(ui, |ui| {
        ui.set_min_width(120.0);
        ui.vertical(|ui| {
            ui.label(
                RichText::new(value)
                    .size(22.0)
                    .strong()
                    .color(palette.accent),
            );
            ui.label(RichText::new(label).size(11.5).color(palette.text_dim));
        });
    });
}

// ---------------------------------------------------------------------------
// Backup helpers
// ---------------------------------------------------------------------------

fn create_backup(app: &mut App) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Save a backup")
        .set_file_name(backup::suggested_filename())
        .add_filter("Mihon Desktop backup", &["gz", "json"])
        .save_file()
    else {
        return;
    };

    let snapshot = Backup::create(&app.core.db, Some(&app.prefs));
    match snapshot.write_to(&path) {
        Ok(()) => app.toast(format!("backup saved — {}", snapshot.summary())),
        Err(err) => app.toast_error(format!("backup failed: {err}")),
    }
}

fn restore_backup(app: &mut App) {
    let Some(path) = rfd::FileDialog::new()
        .set_title("Restore a backup")
        .add_filter("Mihon Desktop backup", &["gz", "json"])
        .pick_file()
    else {
        return;
    };

    let backup = match Backup::read_from(&path) {
        Ok(backup) => backup,
        Err(err) => {
            app.toast_error(format!("could not read the backup: {err}"));
            return;
        }
    };

    let summary = backup.summary();
    if let Err(err) = app.core.db.import(backup.library.clone()) {
        app.toast_error(format!("restore failed: {err}"));
        return;
    }
    if let Some(prefs) = backup.preferences {
        app.prefs = prefs;
        app.prefs_changed();
    }

    app.library.categories = app.core.db.categories();
    app.invalidate_all();
    app.toast(format!("backup restored — {summary}"));
}

/// Drops stored entries that are neither favourited nor referenced by history.
fn purge_orphans(app: &mut App) -> usize {
    let keep: std::collections::HashSet<Id> = app
        .core
        .db
        .recent_history(100_000, "")
        .iter()
        .map(|h| h.manga_id)
        .collect();

    let orphans: Vec<Id> = app
        .core
        .db
        .all_manga()
        .into_iter()
        .filter(|m| !m.favorite && !keep.contains(&m.id))
        .map(|m| m.id)
        .collect();

    let mut removed = 0;
    for manga_id in orphans {
        if app.core.db.delete_manga(manga_id).is_ok() {
            removed += 1;
        }
    }
    app.invalidate_all();
    removed
}
