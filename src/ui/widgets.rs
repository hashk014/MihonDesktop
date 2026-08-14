//! Reusable pieces of the interface: covers, badges, headers, dialogs.

use egui::{
    Align, Color32, Layout, Rect, Response, RichText, Sense, Stroke, TextStyle, Ui, pos2, vec2,
};

use super::theme::{self, Palette};
use super::{App, Dialog};
use crate::images::ImageKind;
use crate::model::{Id, TriState};

/// Cover art is drawn at the usual manga aspect ratio.
pub const COVER_ASPECT: f32 = 1.5;

// ---------------------------------------------------------------------------
// Navigation rail
// ---------------------------------------------------------------------------

/// One entry of the navigation, in whichever shape the user picked.
///
/// The selected entry is *not* filled here: the moving pill behind it is
/// painted once by the rail, so it can slide between entries instead of
/// blinking from one to the next.
/// `size` is passed in rather than taken from `ui.available_width()`: inside a
/// horizontal bar that would give the first entry the entire row.
pub fn nav_item(
    ui: &mut Ui,
    palette: &Palette,
    glyph: &str,
    label: &str,
    selected: bool,
    style: NavItemStyle,
    size: egui::Vec2,
) -> Response {
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();

        if hovered && !selected {
            painter.rect_filled(
                rect.shrink2(vec2(4.0, 2.0)),
                palette.corner(),
                palette.surface_alt,
            );
        }

        let colour = if selected {
            palette.accent
        } else if hovered {
            palette.text
        } else {
            palette.text_dim
        };

        match style {
            NavItemStyle::Full => {
                painter.text(
                    pos2(rect.center().x, rect.top() + rect.height() * 0.32),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(17.0),
                    colour,
                );
                painter.text(
                    pos2(rect.center().x, rect.bottom() - rect.height() * 0.23),
                    egui::Align2::CENTER_CENTER,
                    label,
                    egui::FontId::proportional(10.5),
                    colour,
                );
            }
            NavItemStyle::IconOnly => {
                painter.text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(18.0),
                    colour,
                );
            }
            NavItemStyle::Horizontal => {
                // The label sits beside the glyph so the bar stays shallow.
                let galley = painter.layout_no_wrap(
                    label.to_string(),
                    egui::FontId::proportional(11.5),
                    colour,
                );
                let glyph_width = 20.0;
                let total = glyph_width + 4.0 + galley.size().x;
                let left = rect.center().x - total / 2.0;
                painter.text(
                    pos2(left + glyph_width / 2.0, rect.center().y),
                    egui::Align2::CENTER_CENTER,
                    glyph,
                    egui::FontId::proportional(15.0),
                    colour,
                );
                painter.galley(
                    pos2(
                        left + glyph_width + 4.0,
                        rect.center().y - galley.size().y / 2.0,
                    ),
                    galley,
                    colour,
                );
            }
        }
    }

    response.on_hover_text(label)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavItemStyle {
    /// Glyph above a label.
    Full,
    /// Glyph alone.
    IconOnly,
    /// Glyph beside a label.
    Horizontal,
}

/// The accent pill that marks the selected navigation entry.
///
/// Animating one rectangle rather than fading two of them in and out is what
/// makes the movement read as a single indicator travelling; it costs two
/// interpolated values per axis and egui asks for the repaints itself.
///
/// Returns a shape instead of painting, because the pill has to sit *behind*
/// entries whose rectangles are only known once they have been laid out. The
/// caller reserves a slot with `Shape::Noop` first and fills it afterwards.
pub fn nav_indicator_shape(ui: &Ui, palette: &Palette, target: Rect, animate: bool) -> egui::Shape {
    let id = ui.id().with("nav_indicator");
    let rect = if animate {
        let time = ui.style().animation_time.max(0.12);
        let value = |suffix: &str, target: f32| {
            ui.ctx()
                .animate_value_with_time(id.with(suffix), target, time)
        };
        Rect::from_min_max(
            pos2(value("x0", target.left()), value("y0", target.top())),
            pos2(value("x1", target.right()), value("y1", target.bottom())),
        )
    } else {
        target
    };

    egui::Shape::rect_filled(rect, palette.corner(), palette.accent.gamma_multiply(0.22))
}

// ---------------------------------------------------------------------------
// Headers and common controls
// ---------------------------------------------------------------------------

/// Screen title with an optional back button. Returns true when back is pressed.
pub fn header(
    ui: &mut Ui,
    palette: &Palette,
    title: &str,
    subtitle: Option<&str>,
    back: bool,
) -> bool {
    let mut went_back = false;
    ui.horizontal(|ui| {
        if back {
            if ui
                .add(
                    egui::Button::new(RichText::new("⬅").size(15.0))
                        .fill(palette.surface)
                        .min_size(vec2(36.0, 32.0)),
                )
                .on_hover_text("Back (Esc)")
                .clicked()
            {
                went_back = true;
            }
            ui.add_space(4.0);
        }
        ui.vertical(|ui| {
            ui.label(RichText::new(title).size(20.0).strong().color(palette.text));
            if let Some(subtitle) = subtitle {
                ui.label(RichText::new(subtitle).size(12.0).color(palette.text_dim));
            }
        });
    });
    went_back
}

pub fn search_field(ui: &mut Ui, palette: &Palette, hint: &str, text: &mut String) -> Response {
    let width = ui.available_width().min(340.0);
    egui::Frame::NONE
        .fill(palette.surface_alt)
        .corner_radius(palette.corner())
        .inner_margin(egui::Margin::symmetric(10, 4))
        .show(ui, |ui| {
            ui.set_width(width);
            ui.horizontal(|ui| {
                ui.label(RichText::new("🔍").color(palette.text_dim));
                let response = ui.add(
                    egui::TextEdit::singleline(text)
                        .hint_text(hint)
                        .desired_width(f32::INFINITY)
                        .frame(egui::Frame::NONE),
                );
                if !text.is_empty() && ui.small_button("✖").clicked() {
                    text.clear();
                }
                response
            })
            .inner
        })
        .inner
}

/// Small coloured label used for unread / downloaded counters.
///
/// Reads its rounding from the live style rather than a palette: badges are
/// painted from a dozen call sites that have no reason to carry one around.
pub fn badge(ui: &mut Ui, text: &str, fill: Color32, text_colour: Color32) {
    let corner = theme::small_corner(ui);
    egui::Frame::NONE
        .fill(fill)
        .corner_radius(corner)
        .inner_margin(egui::Margin::symmetric(5, 1))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(11.0).color(text_colour).strong());
        });
}

pub fn chip(ui: &mut Ui, palette: &Palette, text: &str) -> Response {
    let response = egui::Frame::NONE
        .fill(palette.surface_alt)
        .corner_radius(palette.corner_small())
        .inner_margin(egui::Margin::symmetric(8, 3))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(palette.text_dim));
        })
        .response;
    response.on_hover_cursor(egui::CursorIcon::Default)
}

/// A pill-shaped tab selector. Returns the newly selected index when it changes.
pub fn segmented(
    ui: &mut Ui,
    palette: &Palette,
    labels: &[&str],
    selected: usize,
) -> Option<usize> {
    let mut clicked = None;
    egui::Frame::NONE
        .fill(palette.surface)
        .corner_radius(palette.corner())
        .inner_margin(egui::Margin::same(3))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                for (index, label) in labels.iter().enumerate() {
                    let is_selected = index == selected;
                    let button =
                        egui::Button::new(RichText::new(*label).size(13.0).color(if is_selected {
                            palette.on_accent
                        } else {
                            palette.text_dim
                        }))
                        .fill(if is_selected {
                            palette.accent
                        } else {
                            Color32::TRANSPARENT
                        })
                        .corner_radius(palette.corner_small());

                    if ui.add(button).clicked() && !is_selected {
                        clicked = Some(index);
                    }
                }
            });
        });
    clicked
}

/// Tri-state row used by the library and chapter filter sheets.
pub fn tri_state_row(ui: &mut Ui, palette: &Palette, label: &str, state: &mut TriState) -> bool {
    let (glyph, colour) = match state {
        TriState::Disabled => ("○", palette.text_dim),
        TriState::EnabledIs => ("☑", palette.accent),
        TriState::EnabledNot => ("✖", palette.error),
    };

    let response = ui
        .add(
            egui::Button::new(RichText::new(format!("{glyph}  {label}")).color(
                if *state == TriState::Disabled {
                    palette.text
                } else {
                    colour
                },
            ))
            .fill(Color32::TRANSPARENT)
            .min_size(vec2(ui.available_width(), 26.0)),
        )
        .on_hover_text("Click to cycle: off → include → exclude");

    if response.clicked() {
        *state = state.next();
        true
    } else {
        false
    }
}

/// A row that is clickable as a whole while the buttons inside it keep working.
///
/// egui resolves overlapping click areas in favour of the *last* registered
/// widget, so wrapping a row in `Response::interact` (registered after its
/// contents) silently swallows every button inside it. A `Ui` created with a
/// sense registers itself *before* its children — egui does this on purpose,
/// "to ensure we are behind all widgets we contain" — which gives the buttons
/// priority and leaves the row to handle clicks on empty space.
pub fn clickable_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> egui::InnerResponse<R> {
    ui.scope_builder(egui::UiBuilder::new().sense(Sense::click()), add)
}

pub fn empty_state(ui: &mut Ui, palette: &Palette, glyph: &str, title: &str, hint: &str) {
    ui.vertical_centered(|ui| {
        ui.add_space(ui.available_height() * 0.28);
        ui.label(RichText::new(glyph).size(44.0).color(palette.text_dim));
        ui.add_space(6.0);
        ui.label(RichText::new(title).size(16.0).strong().color(palette.text));
        ui.add_space(2.0);
        ui.label(RichText::new(hint).size(12.5).color(palette.text_dim));
    });
}

pub fn spinner_row(ui: &mut Ui, palette: &Palette, text: &str) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(15.0).color(palette.accent));
        ui.label(RichText::new(text).color(palette.text_dim));
    });
}

pub fn error_box(ui: &mut Ui, palette: &Palette, message: &str) {
    egui::Frame::NONE
        .fill(palette.error.gamma_multiply(0.15))
        .stroke(Stroke::new(1.0, palette.error.gamma_multiply(0.5)))
        .corner_radius(palette.corner())
        .inner_margin(egui::Margin::same(10))
        .show(ui, |ui| {
            ui.label(RichText::new(message).color(palette.error).size(12.5));
        });
}

// ---------------------------------------------------------------------------
// Covers
// ---------------------------------------------------------------------------

/// Draws cover art into `rect`, cropping to fill rather than stretching.
pub fn paint_cover(
    app: &mut App,
    ui: &mut Ui,
    rect: Rect,
    url: Option<&str>,
    source_id: Id,
    title: &str,
) {
    let palette = app.palette;
    let corner = palette.corner();

    let texture = url.and_then(|url| {
        let headers = app.source_headers(source_id);
        app.texture(ImageKind::Cover, url, headers, 900)
    });

    if !ui.is_rect_visible(rect) {
        return;
    }

    match texture
        .as_ref()
        .and_then(|t| t.slices.first().map(|s| (t, s)))
    {
        Some((texture, first_slice)) => {
            let size = texture.size_vec2();
            // Cover-fit: keep the aspect ratio and crop the overflow.
            let target_ratio = rect.width() / rect.height().max(0.001);
            let source_ratio = size.x / size.y.max(0.001);
            let uv = if source_ratio > target_ratio {
                let visible = target_ratio / source_ratio;
                let margin = (1.0 - visible) / 2.0;
                Rect::from_min_max(pos2(margin, 0.0), pos2(1.0 - margin, 1.0))
            } else {
                let visible = source_ratio / target_ratio;
                let margin = (1.0 - visible) / 2.0;
                Rect::from_min_max(pos2(0.0, margin), pos2(1.0, 1.0 - margin))
            };

            // Covers are never tall enough to be sliced, so the first slice is
            // the whole image.
            let mut mesh = egui::Mesh::with_texture(first_slice.id());
            mesh.add_rect_with_uv(rect, uv, Color32::WHITE);
            ui.painter()
                .with_clip_rect(rect)
                .add(egui::Shape::mesh(mesh));
            // Rounded corners: paint the surface colour back into them.
            ui.painter().rect_stroke(
                rect,
                corner,
                Stroke::new(1.0, palette.outline.gamma_multiply(0.6)),
                egui::StrokeKind::Inside,
            );
        }
        None => {
            let failed = url
                .map(|url| app.image_failed(ImageKind::Cover, url))
                .unwrap_or(true);
            ui.painter().rect_filled(rect, corner, palette.surface_alt);

            let initials: String = title
                .split_whitespace()
                .filter_map(|word| word.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase();

            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if failed { initials } else { "…".into() },
                egui::FontId::proportional(rect.width() * 0.28),
                palette.text_dim.gamma_multiply(0.7),
            );
        }
    }
}

/// A library / browse grid tile. Returns the click response.
#[allow(clippy::too_many_arguments)]
pub fn cover_tile(
    app: &mut App,
    ui: &mut Ui,
    width: f32,
    url: Option<&str>,
    source_id: Id,
    title: &str,
    show_title: bool,
    selected: bool,
    badges: &[(String, Color32)],
    in_library: bool,
) -> Response {
    let cover_height = width * COVER_ASPECT;
    let title_height = if show_title { 34.0 } else { 0.0 };
    let (rect, response) =
        ui.allocate_exact_size(vec2(width, cover_height + title_height), Sense::click());

    let palette = app.palette;
    let cover_rect = Rect::from_min_size(rect.min, vec2(width, cover_height));

    paint_cover(app, ui, cover_rect, url, source_id, title);

    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter().clone();

    // Dim entries already in the library when browsing a source.
    if in_library {
        painter.rect_filled(cover_rect, palette.corner(), Color32::from_black_alpha(120));
        painter.text(
            cover_rect.center(),
            egui::Align2::CENTER_CENTER,
            "In library",
            egui::FontId::proportional(12.0),
            Color32::WHITE,
        );
    }

    if show_title {
        // A gradient-ish scrim keeps the title readable over busy art.
        let text_rect = Rect::from_min_size(
            pos2(rect.left(), cover_rect.bottom() + 4.0),
            vec2(width, title_height - 4.0),
        );
        let galley = painter.layout(
            title.to_string(),
            TextStyle::Small.resolve(ui.style()),
            palette.text,
            width - 2.0,
        );
        let clipped = painter.with_clip_rect(text_rect);
        clipped.galley(text_rect.min, galley, palette.text);
    }

    // Badges sit in the top-left corner, like upstream.
    let mut x = cover_rect.left() + 4.0;
    for (text, colour) in badges {
        let font = egui::FontId::proportional(10.5);
        let galley = painter.layout_no_wrap(text.clone(), font.clone(), Color32::WHITE);
        let badge_rect = Rect::from_min_size(
            pos2(x, cover_rect.top() + 4.0),
            vec2(galley.size().x + 10.0, 16.0),
        );
        painter.rect_filled(badge_rect, palette.corner_small(), *colour);
        painter.galley(
            pos2(badge_rect.left() + 5.0, badge_rect.top() + 2.0),
            galley,
            Color32::WHITE,
        );
        x += badge_rect.width() + 3.0;
    }

    if selected {
        painter.rect_stroke(
            cover_rect,
            palette.corner(),
            Stroke::new(3.0, palette.accent),
            egui::StrokeKind::Inside,
        );
        painter.rect_filled(
            cover_rect,
            palette.corner(),
            palette.accent.gamma_multiply(0.25),
        );
    } else if response.hovered() {
        painter.rect_stroke(
            cover_rect,
            palette.corner(),
            Stroke::new(2.0, palette.accent.gamma_multiply(0.8)),
            egui::StrokeKind::Inside,
        );
    }

    response
}

/// Number of grid columns that fit, honouring the user's preferred count.
pub fn grid_columns(available_width: f32, preferred: u32, spacing: f32) -> (usize, f32) {
    let preferred = preferred.max(1) as f32;
    // Treat the preference as a target width rather than a hard column count so
    // the grid still adapts when the window is resized.
    let target = ((available_width + spacing) / preferred - spacing).clamp(96.0, 260.0);
    let columns = ((available_width + spacing) / (target + spacing))
        .floor()
        .max(1.0);
    let width = (available_width - spacing * (columns - 1.0)) / columns;
    (columns as usize, width.max(64.0))
}

// ---------------------------------------------------------------------------
// Dialogs
// ---------------------------------------------------------------------------

pub fn dialogs(app: &mut App, ctx: &egui::Context) {
    let Some(mut dialog) = app.dialog.take() else {
        return;
    };
    let palette = app.palette;
    // The dialog is owned locally for the frame, so its arms can freely borrow
    // `app` without aliasing `app.dialog`.
    let mut close = false;

    let frame = egui::Frame::NONE
        .fill(palette.elevated)
        .corner_radius(palette.corner_large())
        .inner_margin(egui::Margin::same(18))
        .stroke(Stroke::new(1.0, palette.outline));

    let response = egui::Modal::new(egui::Id::new("modal"))
        .frame(frame)
        .show(ctx, |ui| {
            ui.set_max_width(460.0);
            match &mut dialog {
                Dialog::CategoryPicker {
                    manga_ids,
                    selected,
                } => {
                    ui.label(RichText::new("Set categories").size(17.0).strong());
                    ui.add_space(8.0);
                    let categories = app.core.db.categories();
                    if categories.iter().all(|c| c.is_system()) {
                        ui.label(
                            RichText::new("No categories yet. Create some from More → Categories.")
                                .color(palette.text_dim),
                        );
                    }
                    egui::ScrollArea::vertical()
                        .max_height(280.0)
                        .show(ui, |ui| {
                            for category in &categories {
                                let mut checked = selected.contains(&category.id);
                                if ui.checkbox(&mut checked, &category.name).changed() {
                                    if checked {
                                        selected.insert(category.id);
                                    } else {
                                        selected.remove(&category.id);
                                    }
                                }
                            }
                        });
                    ui.add_space(10.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.button("Save").clicked() {
                            let ids: Vec<Id> = selected.iter().copied().collect();
                            for manga_id in manga_ids.iter() {
                                if let Err(err) = app.core.db.set_categories(*manga_id, ids.clone())
                                {
                                    app.toasts.push(crate::event::Toast::error(format!(
                                        "could not save categories: {err}"
                                    )));
                                }
                            }
                            app.library.dirty = true;
                            close = true;
                        }
                    });
                }

                Dialog::ConfirmRemove {
                    manga_ids,
                    delete_downloads,
                } => {
                    ui.label(RichText::new("Remove from library").size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new(format!(
                            "{} entr{} will be removed from your library.",
                            manga_ids.len(),
                            if manga_ids.len() == 1 { "y" } else { "ies" }
                        ))
                        .color(palette.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.checkbox(delete_downloads, "Also delete downloaded chapters");
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        let remove =
                            egui::Button::new(RichText::new("Remove").color(Color32::WHITE))
                                .fill(palette.error);
                        if ui.add(remove).clicked() {
                            let purge = *delete_downloads;
                            for manga_id in manga_ids.iter() {
                                if purge {
                                    app.core.downloads.delete_manga(*manga_id);
                                }
                                let _ = app.core.db.update_manga(*manga_id, |m| {
                                    m.favorite = false;
                                });
                            }
                            app.library.selection.clear();
                            app.invalidate_all();
                            close = true;
                        }
                    });
                }

                Dialog::NewCategory { name } => {
                    ui.label(RichText::new("New category").size(17.0).strong());
                    ui.add_space(8.0);
                    let response = ui.add(
                        egui::TextEdit::singleline(name)
                            .hint_text("Name")
                            .desired_width(f32::INFINITY),
                    );
                    response.request_focus();
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        let submit = ui.button("Create").clicked()
                            || ui.input(|i| i.key_pressed(egui::Key::Enter));
                        if submit && !name.trim().is_empty() {
                            match app.core.db.create_category(name.trim()) {
                                Ok(_) => app.library.categories = app.core.db.categories(),
                                Err(err) => app.toasts.push(crate::event::Toast::error(format!(
                                    "could not create the category: {err}"
                                ))),
                            }
                            close = true;
                        }
                    });
                }

                Dialog::RenameCategory { id, name } => {
                    ui.label(RichText::new("Rename category").size(17.0).strong());
                    ui.add_space(8.0);
                    ui.add(egui::TextEdit::singleline(name).desired_width(f32::INFINITY));
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.button("Rename").clicked() && !name.trim().is_empty() {
                            if let Some(mut category) =
                                app.core.db.categories().into_iter().find(|c| c.id == *id)
                            {
                                category.name = name.trim().to_string();
                                let _ = app.core.db.update_category(&category);
                                app.library.categories = app.core.db.categories();
                            }
                            close = true;
                        }
                    });
                }

                Dialog::ConfirmClearHistory => {
                    ui.label(RichText::new("Clear history").size(17.0).strong());
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("Every reading history entry will be deleted.")
                            .color(palette.text_dim),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        let clear = egui::Button::new(RichText::new("Clear").color(Color32::WHITE))
                            .fill(palette.error);
                        if ui.add(clear).clicked() {
                            let _ = app.core.db.clear_history();
                            app.history.dirty = true;
                            close = true;
                        }
                    });
                }

                Dialog::AddRepo { url } => {
                    ui.label(
                        RichText::new("Add extension repository")
                            .size(17.0)
                            .strong(),
                    );
                    ui.add_space(6.0);
                    ui.label(
                        RichText::new("URL of a JSON index listing extension manifests.")
                            .size(12.0)
                            .color(palette.text_dim),
                    );
                    ui.add_space(8.0);
                    ui.add(
                        egui::TextEdit::singleline(url)
                            .hint_text("https://example.com/index.json")
                            .desired_width(f32::INFINITY),
                    );
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                        if ui.button("Add").clicked() && !url.trim().is_empty() {
                            let url = url.trim().to_string();
                            if !app.prefs.browse.extension_repos.contains(&url) {
                                app.prefs.browse.extension_repos.push(url.clone());
                                app.prefs_changed();
                            }
                            app.extensions.loading = true;
                            app.core.fetch_repo(url);
                            close = true;
                        }
                    });
                }

                Dialog::LibrarySettings => {
                    if super::library::settings_sheet(app, ui) {
                        close = true;
                    }
                }
                Dialog::ChapterSettings => {
                    if super::manga::chapter_settings_sheet(app, ui) {
                        close = true;
                    }
                }
                Dialog::ReaderSettings => {
                    if super::reader::settings_sheet(app, ui) {
                        close = true;
                    }
                }
                Dialog::MigrateTarget { manga_id } => {
                    if super::browse::migrate_sheet(app, ui, *manga_id) {
                        close = true;
                    }
                }
            }
        });

    if response.should_close() {
        close = true;
    }
    if !close {
        app.dialog = Some(dialog);
    }
}

/// Formats a timestamp the way the history and updates lists want it.
pub fn format_timestamp(millis: i64, relative: bool) -> String {
    if millis <= 0 {
        return "Unknown".into();
    }
    let Some(datetime) = chrono::DateTime::from_timestamp_millis(millis) else {
        return "Unknown".into();
    };
    let local = datetime.with_timezone(&chrono::Local);

    if relative {
        let delta = chrono::Local::now().signed_duration_since(local);
        let minutes = delta.num_minutes();
        if minutes < 1 {
            return "Just now".into();
        }
        if minutes < 60 {
            return format!("{minutes} min ago");
        }
        let hours = delta.num_hours();
        if hours < 24 {
            return format!("{hours} h ago");
        }
        let days = delta.num_days();
        if days < 7 {
            return format!("{days} d ago");
        }
        if days < 365 {
            return local.format("%d %b").to_string();
        }
    }
    local.format("%d %b %Y").to_string()
}

pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{} {}", bytes, UNITS[0])
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

/// Right-aligned row of actions in a toolbar.
pub fn toolbar_actions<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    ui.with_layout(Layout::right_to_left(Align::Center), add)
        .inner
}

/// Convenience for screens that need a back-aware header row.
pub fn screen_header(app: &mut App, ui: &mut Ui, title: &str, subtitle: Option<&str>) {
    let palette = app.palette;
    let has_back = !app.stack.is_empty();
    if header(ui, &palette, title, subtitle, has_back) {
        app.pop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs one headless egui frame and reports what got clicked.
    fn test_context() -> egui::Context {
        let ctx = egui::Context::default();
        // Mirror the app's style: selectable labels would otherwise sense the
        // clicks meant for the row underneath them.
        ctx.all_styles_mut(|style| style.interaction.selectable_labels = false);
        ctx
    }

    fn run_row_frame(ctx: &egui::Context, click_at: Option<egui::Pos2>) -> (bool, bool, Rect) {
        let mut row_clicked = false;
        let mut button_clicked = false;
        let mut button_rect = Rect::NOTHING;

        let mut events = Vec::new();
        if let Some(pos) = click_at {
            events.push(egui::Event::PointerMoved(pos));
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
        }

        let input = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(600.0, 400.0))),
            events,
            ..Default::default()
        };

        let mut output = ctx.run_ui(input, |ui| {
            let response = clickable_row(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("a chapter name");
                    let button = ui.button("delete");
                    button_rect = button.rect;
                    button_clicked = button.clicked();
                });
            })
            .response;
            row_clicked = response.clicked();
        });
        // There is no renderer here to consume the font atlas upload.
        output.textures_delta.clear();

        (row_clicked, button_clicked, button_rect)
    }

    /// Regression: the trash button inside a chapter row did nothing, because
    /// the row's own click area was registered after it and swallowed every
    /// click. The button must win inside its own rect.
    #[test]
    fn a_button_inside_a_clickable_row_receives_the_click() {
        let ctx = test_context();
        // First pass lays things out; interaction uses the previous pass.
        let (_, _, button_rect) = run_row_frame(&ctx, None);
        assert!(button_rect.is_positive(), "the button should have a rect");
        let _ = run_row_frame(&ctx, None);

        let (row_clicked, button_clicked, _) = run_row_frame(&ctx, Some(button_rect.center()));
        assert!(button_clicked, "the button must receive the click");
        assert!(
            !row_clicked,
            "the row must not also fire and open the reader"
        );
    }

    /// The row still has to be clickable everywhere else, which is what opens
    /// the chapter.
    #[test]
    fn clicking_beside_the_button_still_hits_the_row() {
        let ctx = test_context();
        let (_, _, button_rect) = run_row_frame(&ctx, None);
        let _ = run_row_frame(&ctx, None);

        // Well to the left of the button, over the label area.
        let empty_spot = pos2(20.0, button_rect.center().y);
        let (row_clicked, button_clicked, _) = run_row_frame(&ctx, Some(empty_spot));
        assert!(row_clicked, "empty space must still open the row");
        assert!(!button_clicked);
    }

    #[test]
    fn grid_columns_fit_the_available_width() {
        let (columns, width) = grid_columns(1000.0, 6, 10.0);
        assert!(columns >= 1);
        // Columns plus the gaps between them must not exceed the width.
        let used = width * columns as f32 + 10.0 * (columns as f32 - 1.0);
        assert!(used <= 1000.5, "grid overflows: {used}");
    }

    #[test]
    fn grid_never_collapses_to_zero() {
        let (columns, width) = grid_columns(50.0, 12, 8.0);
        assert_eq!(columns, 1);
        assert!(width > 0.0);
    }

    #[test]
    fn byte_formatting_is_readable() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn timestamps_degrade_gracefully() {
        assert_eq!(format_timestamp(0, true), "Unknown");
        assert_eq!(format_timestamp(-5, false), "Unknown");
        assert!(!format_timestamp(1_600_000_000_000, false).is_empty());
    }
}
