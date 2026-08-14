//! The reader: paged (LTR / RTL / vertical) and continuous (webtoon) modes.

use egui::{Color32, Rect, RichText, Sense, Ui, Vec2, pos2, vec2};

use super::widgets;
use super::{App, Dialog, ReaderSegment};
use crate::images::ImageKind;
use crate::model::*;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let background = palette.reader_background(app.prefs.reader.background);
    let foreground = palette.contrasting(background);

    // Painting the whole viewport ourselves keeps the page framed by the
    // reader background rather than the app background.
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0, background);

    handle_input(app, ui);

    if app.reader.loading && app.reader.pages.is_empty() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("Loading pages…").color(foreground));
            });
        });
        overlay(app, ui, full, foreground, background);
        return;
    }

    if let Some(error) = app.reader.error.clone() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| {
            ui.centered_and_justified(|ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(full.height() * 0.35);
                    ui.label(RichText::new("This chapter could not be opened").color(foreground));
                    ui.add_space(6.0);
                    ui.label(RichText::new(&error).size(12.0).color(palette.error));
                    ui.add_space(10.0);
                    if ui.button("Retry").clicked() {
                        let (manga_id, chapter_id) = (app.reader.manga_id, app.reader.chapter_id);
                        app.reader.loading = true;
                        app.reader.error = None;
                        app.core.fetch_pages(manga_id, chapter_id);
                    }
                });
            });
        });
        overlay(app, ui, full, foreground, background);
        return;
    }

    if app.reader.pages.is_empty() {
        ui.scope_builder(egui::UiBuilder::new().max_rect(full), |ui| {
            ui.centered_and_justified(|ui| {
                ui.label(RichText::new("No pages in this chapter").color(foreground));
            });
        });
        overlay(app, ui, full, foreground, background);
        return;
    }

    if app.reader.mode.is_continuous() {
        continuous_view(app, ui, full, foreground);
    } else {
        paged_view(app, ui, full, foreground);
    }

    preload(app);
    overlay(app, ui, full, foreground, background);
}

// ---------------------------------------------------------------------------
// Paged mode
// ---------------------------------------------------------------------------

fn paged_view(app: &mut App, ui: &mut Ui, area: Rect, foreground: Color32) {
    let current = app.reader.current;
    let double = app.prefs.reader.double_pages
        && app.reader.mode.is_horizontal()
        && area.width() > area.height();

    // In double-page mode a landscape page occupies a spread on its own.
    let second = if double && !is_spread(app, current) {
        let next = current + 1;
        (next < app.reader.pages.len() && !is_spread(app, next)).then_some(next)
    } else {
        None
    };

    match second {
        None => draw_page(app, ui, area, current, foreground),
        Some(next) => {
            let half = area.width() / 2.0;
            let (left_rect, right_rect) = (
                Rect::from_min_size(area.min, vec2(half, area.height())),
                Rect::from_min_size(
                    pos2(area.min.x + half, area.min.y),
                    vec2(half, area.height()),
                ),
            );
            // Right-to-left puts the earlier page on the right.
            if app.reader.mode.is_rtl() {
                draw_page(app, ui, right_rect, current, foreground);
                draw_page(app, ui, left_rect, next, foreground);
            } else {
                draw_page(app, ui, left_rect, current, foreground);
                draw_page(app, ui, right_rect, next, foreground);
            }
        }
    }
}

fn draw_page(app: &mut App, ui: &mut Ui, area: Rect, index: usize, foreground: Color32) {
    let Some(page) = app.reader.pages.get(index).cloned() else {
        return;
    };
    let texture = app.texture_of(
        ImageKind::Page,
        page.candidates(),
        page.headers.clone(),
        crate::images::MAX_IMAGE_WIDTH,
    );

    let Some(texture) = texture else {
        let error = app.image_error(ImageKind::Page, &page.image_url);
        let painter = ui.painter();
        painter.text(
            area.center(),
            egui::Align2::CENTER_CENTER,
            match &error {
                Some(_) => "This page could not be loaded",
                None => "Loading…",
            },
            egui::FontId::proportional(14.0),
            foreground.gamma_multiply(0.7),
        );
        if let Some(reason) = &error {
            // The reason matters: a 404 and a rate limit call for very
            // different reactions from the reader.
            painter.text(
                area.center() + egui::vec2(0.0, 22.0),
                egui::Align2::CENTER_CENTER,
                reason,
                egui::FontId::proportional(11.5),
                foreground.gamma_multiply(0.45),
            );
        } else {
            ui.ctx().request_repaint();
        }
        return;
    };

    let size = texture.size_vec2();
    if let Some(slot) = app.reader.aspects.get_mut(index) {
        *slot = size.x / size.y.max(1.0);
    }

    let scaled = fit_size(size, area.size(), app.prefs.reader.scale_type) * app.reader.zoom;
    let centre = area.center() + app.reader.pan;
    let rect = Rect::from_center_size(centre, scaled);

    paint_slices(ui, &texture, rect, area);
}

/// Draws an image's slices stacked into `rect`, clipped to `clip`.
///
/// A tall strip is uploaded as several textures; each one covers the share of
/// the target rectangle that matches its share of the image's height.
fn paint_slices(ui: &Ui, texture: &crate::images::PageTexture, rect: Rect, clip: Rect) {
    let painter = ui.painter().with_clip_rect(clip);
    let total_height = texture.size.y.max(1.0);
    let mut consumed = 0.0f32;

    for slice in &texture.slices {
        let slice_height = slice.size_vec2().y;
        let top = rect.top() + rect.height() * (consumed / total_height);
        let bottom = rect.top() + rect.height() * ((consumed + slice_height) / total_height);
        let slice_rect = Rect::from_min_max(pos2(rect.left(), top), pos2(rect.right(), bottom));
        consumed += slice_height;

        // Skip work for slices scrolled far out of view.
        if !slice_rect.intersects(clip) {
            continue;
        }

        let mut mesh = egui::Mesh::with_texture(slice.id());
        mesh.add_rect_with_uv(
            slice_rect,
            Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)),
            Color32::WHITE,
        );
        painter.add(egui::Shape::mesh(mesh));
    }
}

/// Computes the on-screen size of a page for the configured scale type.
pub fn fit_size(image: Vec2, area: Vec2, scale: ImageScaleType) -> Vec2 {
    if image.x <= 0.0 || image.y <= 0.0 {
        return area;
    }
    let scale_x = area.x / image.x;
    let scale_y = area.y / image.y;

    let factor = match scale {
        ImageScaleType::FitScreen => scale_x.min(scale_y),
        ImageScaleType::StretchToFit => return area,
        ImageScaleType::FitWidth => scale_x,
        ImageScaleType::FitHeight => scale_y,
        ImageScaleType::OriginalSize => 1.0,
        // Wide (spread) pages fit the width, tall pages fit the screen.
        ImageScaleType::SmartFit => {
            if image.x > image.y {
                scale_x
            } else {
                scale_x.min(scale_y)
            }
        }
    };
    image * factor
}

fn is_spread(app: &App, index: usize) -> bool {
    app.prefs.reader.double_page_split_spreads
        && app
            .reader
            .aspects
            .get(index)
            .map(|aspect| *aspect > 1.0)
            .unwrap_or(false)
}

// ---------------------------------------------------------------------------
// Continuous (webtoon) mode
// ---------------------------------------------------------------------------

fn continuous_view(app: &mut App, ui: &mut Ui, area: Rect, foreground: Color32) {
    let width_fraction = app.prefs.reader.webtoon_width_fraction.clamp(0.3, 1.0);
    let page_width = (area.width() * width_fraction * app.reader.zoom).min(area.width());
    let gap = app.prefs.reader.webtoon_page_gap;
    let count = app.reader.pages.len();
    let scroll_to = app.reader.scroll_to.take();

    let mut top_visible = app.reader.current;
    let infinite = app.reader.mode.is_infinite();
    // Chapter boundaries, so a header can be drawn where each one starts.
    let starts: Vec<(usize, String)> = app
        .reader
        .segments
        .iter()
        .skip(1)
        .map(|s| (s.start, s.name.clone()))
        .collect();

    ui.scope_builder(egui::UiBuilder::new().max_rect(area), |ui| {
        egui::ScrollArea::vertical()
            // In infinite mode the list keeps growing, so the scroll state must
            // not be keyed on the chapter or it would reset at every boundary.
            .id_salt(if infinite {
                ("webtoon", app.reader.manga_id)
            } else {
                ("webtoon", app.reader.chapter_id)
            })
            .auto_shrink([false, false])
            .show(ui, |ui| {
                ui.vertical_centered(|ui| {
                    for index in 0..count {
                        if let Some((_, name)) = starts.iter().find(|(start, _)| *start == index) {
                            chapter_divider(ui, page_width, name, foreground);
                        }

                        let aspect = app
                            .reader
                            .aspects
                            .get(index)
                            .copied()
                            .unwrap_or(0.7)
                            .max(0.05);
                        let height = page_width / aspect;
                        let (rect, _) =
                            ui.allocate_exact_size(vec2(page_width, height), Sense::hover());

                        // Seeking from the menu scrolls the requested page in.
                        if scroll_to == Some(index) {
                            ui.scroll_to_rect(rect, Some(egui::Align::TOP));
                        }

                        let clip = ui.clip_rect();
                        if rect.intersects(clip) {
                            draw_continuous_page(app, ui, rect, index, foreground);
                            // The page occupying the top of the viewport is the
                            // one the progress indicator should report.
                            if rect.top() <= clip.top() + 4.0 && rect.bottom() > clip.top() {
                                top_visible = index;
                            } else if index == 0 && rect.top() > clip.top() {
                                top_visible = 0;
                            }
                        }
                        if gap > 0.0 {
                            ui.add_space(gap);
                        }
                    }

                    ui.add_space(20.0);
                    if infinite {
                        infinite_tail(app, ui, foreground);
                    } else {
                        chapter_end_block(app, ui, foreground);
                    }
                    ui.add_space(40.0);
                });
            });
    });

    // A seek scrolls on this frame but is only measured on the next one, so the
    // stale measurement must not drag the position back.
    if scroll_to.is_none() && top_visible != app.reader.current {
        app.reader.current = top_visible;
        follow_visible_chapter(app);
        save_progress(app);
    }

    // Pull the next chapter in before the reader runs out of pages. The empty
    // check matters: with nothing loaded yet, "3 pages left" is trivially true
    // and the next chapter would be appended before the current one arrives.
    if infinite && count > 0 && !app.reader.loading {
        let remaining = count.saturating_sub(app.reader.current);
        if remaining <= 3 {
            request_next_chapter(app);
        }
    }
}

/// A labelled break between two chapters in the continuous view.
fn chapter_divider(ui: &mut Ui, width: f32, name: &str, foreground: Color32) {
    ui.add_space(18.0);
    let (rect, _) = ui.allocate_exact_size(vec2(width, 34.0), Sense::hover());
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        painter.rect_filled(
            rect,
            egui::CornerRadius::same(8),
            foreground.gamma_multiply(0.08),
        );
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            name,
            egui::FontId::proportional(13.0),
            foreground.gamma_multiply(0.75),
        );
    }
    ui.add_space(18.0);
}

/// Keeps `chapter_id` in step with whatever is under the viewport, and marks a
/// chapter read once it has been scrolled past.
fn follow_visible_chapter(app: &mut App) {
    let Some(&owner) = app.reader.page_owner.get(app.reader.current) else {
        return;
    };
    if owner == app.reader.chapter_id {
        return;
    }

    // Everything before the new chapter has been read through.
    let previous = app.reader.chapter_id;
    if app.prefs.reader.mark_read_on_last_page {
        mark_chapter_read(app, previous);
    }

    app.reader.chapter_id = owner;
    app.reader.chapter_name = app
        .reader
        .segments
        .iter()
        .find(|s| s.chapter_id == owner)
        .map(|s| s.name.clone())
        .unwrap_or_default();
    app.reader.opened_at = std::time::Instant::now();
}

/// Asks for the chapter following the last one loaded.
fn request_next_chapter(app: &mut App) {
    // Nothing to continue from until the first chapter has landed.
    if app.reader.appending.is_some() || app.reader.reached_end || app.reader.segments.is_empty() {
        return;
    }
    let Some(last) = app.reader.segments.last().map(|s| s.chapter_id) else {
        return;
    };
    let Some(position) = app.reader.chapters.iter().position(|c| c.id == last) else {
        return;
    };
    // Skipping read chapters here would leave holes in a continuous read.
    let Some(next) = app.reader.chapters.get(position + 1).cloned() else {
        app.reader.reached_end = true;
        return;
    };

    app.reader.appending = Some(next.id);
    let manga_id = app.reader.manga_id;
    app.core.fetch_pages(manga_id, next.id);
}

/// Footer shown at the very end of an infinite read.
fn infinite_tail(app: &mut App, ui: &mut Ui, foreground: Color32) {
    if app.reader.appending.is_some() {
        ui.label(RichText::new("Loading the next chapter…").color(foreground.gamma_multiply(0.8)));
        ui.ctx().request_repaint();
        return;
    }
    if app.reader.reached_end {
        ui.label(
            RichText::new("You have reached the end of the series")
                .color(foreground.gamma_multiply(0.8)),
        );
    }
}

fn draw_continuous_page(app: &mut App, ui: &mut Ui, rect: Rect, index: usize, foreground: Color32) {
    let Some(page) = app.reader.pages.get(index).cloned() else {
        return;
    };
    let texture = app.texture_of(
        ImageKind::Page,
        page.candidates(),
        page.headers.clone(),
        crate::images::MAX_IMAGE_WIDTH,
    );

    match texture {
        Some(texture) => {
            let size = texture.size_vec2();
            if let Some(slot) = app.reader.aspects.get_mut(index) {
                let aspect = size.x / size.y.max(1.0);
                if (*slot - aspect).abs() > 0.001 {
                    *slot = aspect;
                    // The row height changes once the real aspect is known.
                    ui.ctx().request_repaint();
                }
            }
            paint_slices(ui, &texture, rect, ui.clip_rect());
        }
        None => {
            let failed = app.image_failed(ImageKind::Page, &page.image_url);
            ui.painter().rect_filled(
                rect,
                egui::CornerRadius::same(4),
                Color32::from_black_alpha(40),
            );
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                if failed {
                    format!("Page {} failed", index + 1)
                } else {
                    format!("Page {}…", index + 1)
                },
                egui::FontId::proportional(13.0),
                foreground.gamma_multiply(0.6),
            );
            if !failed {
                ui.ctx().request_repaint();
            }
        }
    }
}

fn chapter_end_block(app: &mut App, ui: &mut Ui, foreground: Color32) {
    let has_next = neighbour_chapter(app, 1).is_some();
    ui.label(
        RichText::new(if has_next {
            "End of chapter"
        } else {
            "End of the last chapter"
        })
        .color(foreground.gamma_multiply(0.8)),
    );
    if has_next {
        ui.add_space(6.0);
        if ui.button("Next chapter").clicked() {
            go_to_chapter(app, 1);
        }
    }
}

// ---------------------------------------------------------------------------
// Input
// ---------------------------------------------------------------------------

fn handle_input(app: &mut App, ui: &mut Ui) {
    let area = ui.available_rect_before_wrap();
    let response = ui.interact(
        area,
        ui.id().with(("reader_surface", app.reader.chapter_id)),
        Sense::click_and_drag(),
    );

    // Zoom with ctrl + wheel, pan by dragging when zoomed in.
    let (scroll, zoom_delta, ctrl) = ui.input(|i| {
        (
            i.smooth_scroll_delta.y,
            i.zoom_delta(),
            i.modifiers.ctrl || i.modifiers.command,
        )
    });

    if ctrl && scroll != 0.0 {
        let step = if scroll > 0.0 { 1.1 } else { 1.0 / 1.1 };
        app.reader.zoom = (app.reader.zoom * step).clamp(0.25, 6.0);
    } else if (zoom_delta - 1.0).abs() > 0.001 {
        app.reader.zoom = (app.reader.zoom * zoom_delta).clamp(0.25, 6.0);
    }

    if response.dragged() && app.reader.zoom > 1.0 {
        app.reader.pan += response.drag_delta();
    }

    // In paged mode a plain wheel turns pages.
    if !app.reader.mode.is_continuous() && !ctrl && scroll.abs() > 0.5 {
        if scroll < 0.0 {
            advance(app, 1);
        } else {
            advance(app, -1);
        }
    }

    if response.clicked()
        && let Some(position) = response.interact_pointer_pos()
    {
        let relative = (position.x - area.left()) / area.width().max(1.0);
        if app.reader.mode.is_continuous() {
            app.reader.menu_visible = !app.reader.menu_visible;
        } else if relative < 0.33 {
            advance(app, if app.reader.mode.is_rtl() { 1 } else { -1 });
        } else if relative > 0.67 {
            advance(app, if app.reader.mode.is_rtl() { -1 } else { 1 });
        } else {
            app.reader.menu_visible = !app.reader.menu_visible;
        }
    }

    if !app.prefs.reader.keyboard_navigation {
        return;
    }

    let mut forward = 0i32;
    ui.input_mut(|input| {
        for (key, delta) in [
            (egui::Key::ArrowRight, 1),
            (egui::Key::ArrowLeft, -1),
            (egui::Key::ArrowDown, 1),
            (egui::Key::ArrowUp, -1),
            (egui::Key::Space, 1),
            (egui::Key::PageDown, 1),
            (egui::Key::PageUp, -1),
        ] {
            if input.consume_key(egui::Modifiers::NONE, key) {
                // Horizontal keys follow the reading direction.
                let horizontal = matches!(key, egui::Key::ArrowRight | egui::Key::ArrowLeft);
                forward += if horizontal && app.reader.mode.is_rtl() {
                    -delta
                } else {
                    delta
                };
            }
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::Home) {
            app.reader.current = 0;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::End) {
            app.reader.current = app.reader.pages.len().saturating_sub(1);
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::M) {
            app.reader.menu_visible = !app.reader.menu_visible;
        }
        if input.consume_key(egui::Modifiers::NONE, egui::Key::F) {
            app.reader.zoom = 1.0;
            app.reader.pan = Vec2::ZERO;
        }
    });

    if forward != 0 {
        if app.reader.mode.is_continuous() {
            // Continuous mode scrolls rather than jumping between pages.
            let target = (app.reader.current as i32 + forward).max(0) as usize;
            app.reader.scroll_to = Some(target.min(app.reader.pages.len().saturating_sub(1)));
        } else {
            advance(app, forward);
        }
    }
}

/// Turns pages the way a click or arrow key would. Used by the screenshot
/// harness to exercise the real navigation path, including progress saving.
pub(crate) fn turn_page(app: &mut App, delta: i32) {
    if app.reader.pages.is_empty() {
        return;
    }
    if app.reader.mode.is_continuous() {
        // Continuous modes scroll instead of paging, matching the key handler.
        let target = (app.reader.current as i32 + delta).max(0) as usize;
        app.reader.current = target.min(app.reader.pages.len().saturating_sub(1));
        app.reader.scroll_to = Some(app.reader.current);
        follow_visible_chapter(app);
        save_progress(app);
        if app.reader.mode.is_infinite()
            && app.reader.pages.len().saturating_sub(app.reader.current) <= 3
        {
            request_next_chapter(app);
        }
    } else {
        advance(app, delta);
    }
}

/// Moves by `delta` pages, rolling over into the neighbouring chapters.
fn advance(app: &mut App, delta: i32) {
    // Without pages, "past the last one" is trivially true and a stray click
    // would skip whole chapters while the first one is still loading.
    if app.reader.pages.is_empty() {
        return;
    }
    let step = if app.prefs.reader.double_pages && app.reader.mode.is_horizontal() {
        2
    } else {
        1
    };
    let delta = delta * step;
    let last = app.reader.pages.len().saturating_sub(1);
    let target = app.reader.current as i32 + delta;

    if target < 0 {
        if neighbour_chapter(app, -1).is_some() {
            go_to_chapter(app, -1);
        }
        return;
    }
    if target as usize > last {
        if app.prefs.reader.mark_read_on_last_page {
            mark_current_read(app);
        }
        if neighbour_chapter(app, 1).is_some() {
            go_to_chapter(app, 1);
        } else {
            app.toast("that was the last chapter");
        }
        return;
    }

    app.reader.current = target as usize;
    app.reader.pan = Vec2::ZERO;
    save_progress(app);

    if app.reader.current == last && app.prefs.reader.mark_read_on_last_page {
        mark_current_read(app);
    }
}

// ---------------------------------------------------------------------------
// Chapter navigation and progress
// ---------------------------------------------------------------------------

/// The chapter `offset` positions away in reading order.
fn neighbour_chapter(app: &App, offset: i32) -> Option<Chapter> {
    let index = app
        .reader
        .chapters
        .iter()
        .position(|c| c.id == app.reader.chapter_id)?;
    let target = index as i32 + offset;
    if target < 0 {
        return None;
    }
    let mut target = target as usize;

    // Skipping read chapters only applies when moving forward.
    if offset > 0 && app.prefs.reader.skip_read_chapters_on_finish {
        while target < app.reader.chapters.len() && app.reader.chapters[target].read {
            target += 1;
        }
    }
    app.reader.chapters.get(target).cloned()
}

fn go_to_chapter(app: &mut App, offset: i32) {
    let Some(chapter) = neighbour_chapter(app, offset) else {
        return;
    };
    save_progress(app);
    let manga_id = app.reader.manga_id;
    app.open_reader(manga_id, chapter.id);
}

/// The page number within the chapter currently on screen. In infinite scroll
/// `current` indexes the whole loaded run, so it has to be made chapter-local.
fn local_page(app: &App) -> (i64, i64) {
    local_index(
        &app.reader.segments,
        app.reader.chapter_id,
        app.reader.current,
        app.reader.pages.len(),
    )
}

/// Position and length of `chapter_id` within a run of segments.
fn local_index(
    segments: &[ReaderSegment],
    chapter_id: Id,
    current: usize,
    total: usize,
) -> (i64, i64) {
    match segments.iter().find(|s| s.chapter_id == chapter_id) {
        Some(segment) => (
            current.saturating_sub(segment.start) as i64,
            segment.len as i64,
        ),
        None => (current as i64, total as i64),
    }
}

fn save_progress(app: &mut App) {
    let (manga_id, chapter_id) = (app.reader.manga_id, app.reader.chapter_id);
    let (page, total) = local_page(app);
    if chapter_id == 0 {
        return;
    }
    // Incognito mode leaves no trace: neither progress nor history.
    if app.prefs.incognito {
        app.reader.opened_at = std::time::Instant::now();
        return;
    }
    let _ = app.core.db.update_chapter(chapter_id, |c| {
        c.last_page_read = page;
        if total > 0 {
            c.page_count = total;
        }
    });
    let elapsed = app.reader.opened_at.elapsed().as_millis() as i64;
    app.reader.opened_at = std::time::Instant::now();
    let _ = app.core.db.touch_history(manga_id, chapter_id, elapsed);
    app.history.dirty = true;
}

fn mark_current_read(app: &mut App) {
    let chapter_id = app.reader.chapter_id;
    mark_chapter_read(app, chapter_id);
}

fn mark_chapter_read(app: &mut App, chapter_id: Id) {
    if app.prefs.incognito || chapter_id == 0 {
        return;
    }
    let already = app
        .core
        .db
        .get_chapter(chapter_id)
        .map(|c| c.read)
        .unwrap_or(false);
    if already {
        return;
    }

    let _ = app.core.db.update_chapter(chapter_id, |c| {
        c.read = true;
        c.last_page_read = 0;
    });

    // Reflect it in the in-memory chapter list used for navigation.
    if let Some(chapter) = app.reader.chapters.iter_mut().find(|c| c.id == chapter_id) {
        chapter.read = true;
    }

    if app.prefs.reader.remove_after_read {
        let manga_id = app.reader.manga_id;
        app.core.downloads.delete_chapter(manga_id, chapter_id);
    }
    app.invalidate_all();
}

/// Warms the cache for the next few pages so page turns feel instant.
fn preload(app: &mut App) {
    let ahead = app.prefs.reader.preload_pages as usize;
    if ahead == 0 {
        return;
    }
    let start = app.reader.current + 1;
    let end = (start + ahead).min(app.reader.pages.len());
    for index in start..end {
        let Some(page) = app.reader.pages.get(index).cloned() else {
            break;
        };
        // `texture_of` schedules the fetch when the entry is missing.
        let _ = app.texture_of(
            ImageKind::Page,
            page.candidates(),
            page.headers,
            crate::images::MAX_IMAGE_WIDTH,
        );
    }
}

// ---------------------------------------------------------------------------
// Overlay menu
// ---------------------------------------------------------------------------

fn overlay(app: &mut App, ui: &mut Ui, area: Rect, foreground: Color32, background: Color32) {
    if !app.reader.menu_visible {
        // Even hidden, the page counter stays available like upstream.
        if app.prefs.reader.show_page_number && !app.reader.pages.is_empty() {
            let (page, total) = local_page(app);
            ui.painter().text(
                pos2(area.center().x, area.bottom() - 18.0),
                egui::Align2::CENTER_CENTER,
                format!("{} / {}", page + 1, total.max(1)),
                egui::FontId::proportional(12.5),
                foreground.gamma_multiply(0.55),
            );
        }
        return;
    }

    let palette = app.palette;
    let scrim = background.gamma_multiply(0.92);

    // -- top bar ------------------------------------------------------------
    let top_rect = Rect::from_min_size(area.min, vec2(area.width(), 56.0));
    ui.painter().rect_filled(top_rect, 0, scrim);
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(top_rect.shrink2(vec2(12.0, 8.0))),
        |ui| {
            ui.horizontal(|ui| {
                if ui.button("⬅").on_hover_text("Back (Esc)").clicked() {
                    save_progress(app);
                    app.pop();
                }
                ui.vertical(|ui| {
                    ui.label(
                        RichText::new(&app.reader.manga_title)
                            .size(14.0)
                            .strong()
                            .color(foreground),
                    );
                    ui.label(
                        RichText::new(&app.reader.chapter_name)
                            .size(11.5)
                            .color(foreground.gamma_multiply(0.7)),
                    );
                });
                widgets::toolbar_actions(ui, |ui| {
                    if ui.button("⚙").on_hover_text("Reader settings").clicked() {
                        app.dialog = Some(Dialog::ReaderSettings);
                    }
                    let bookmarked = app
                        .core
                        .db
                        .get_chapter(app.reader.chapter_id)
                        .map(|c| c.bookmark)
                        .unwrap_or(false);
                    if ui
                        .button(if bookmarked { "🔖" } else { "☆" })
                        .on_hover_text("Bookmark this chapter")
                        .clicked()
                    {
                        let chapter_id = app.reader.chapter_id;
                        let _ = app
                            .core
                            .db
                            .update_chapter(chapter_id, |c| c.bookmark = !bookmarked);
                        app.invalidate_all();
                    }
                });
            });
        },
    );

    // -- bottom bar ---------------------------------------------------------
    let bottom_rect = Rect::from_min_size(
        pos2(area.left(), area.bottom() - 72.0),
        vec2(area.width(), 72.0),
    );
    ui.painter().rect_filled(bottom_rect, 0, scrim);
    ui.scope_builder(
        egui::UiBuilder::new().max_rect(bottom_rect.shrink2(vec2(16.0, 10.0))),
        |ui| {
            ui.horizontal(|ui| {
                let has_prev = neighbour_chapter(app, -1).is_some();
                if ui
                    .add_enabled(has_prev, egui::Button::new("⏮"))
                    .on_hover_text("Previous chapter")
                    .clicked()
                {
                    go_to_chapter(app, -1);
                }

                let total = app.reader.pages.len();
                if total > 1 {
                    let mut page = app.reader.current;
                    let slider = egui::Slider::new(&mut page, 0..=total - 1)
                        .show_value(false)
                        .custom_formatter(|value, _| format!("{}", value as usize + 1));
                    let width = (ui.available_width() - 120.0).max(60.0);
                    let response = ui.add_sized(vec2(width, 20.0), slider);
                    // Compare the value rather than trusting `changed()`: the
                    // widget can report a change on its first frame, and a
                    // write per frame is both wasteful and surprising.
                    if response.changed() && page != app.reader.current {
                        app.reader.current = page;
                        app.reader.scroll_to = Some(page);
                        save_progress(app);
                    }
                }

                // In infinite scroll the slider spans everything loaded, but the
                // counter stays chapter-local, which is what a reader tracks.
                let (page, chapter_total) = local_page(app);
                ui.label(
                    RichText::new(format!("{} / {}", page + 1, chapter_total.max(1)))
                        .size(12.0)
                        .color(foreground),
                );

                let has_next = neighbour_chapter(app, 1).is_some();
                if ui
                    .add_enabled(has_next, egui::Button::new("⏭"))
                    .on_hover_text("Next chapter")
                    .clicked()
                {
                    go_to_chapter(app, 1);
                }
            });

            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new(app.reader.mode.label())
                        .size(11.0)
                        .color(palette.text_dim),
                );
                ui.label(
                    RichText::new(format!("· zoom {:.0}%", app.reader.zoom * 100.0))
                        .size(11.0)
                        .color(palette.text_dim),
                );
                ui.label(
                    RichText::new(if app.reader.mode.is_continuous() {
                        "· scroll to read, ctrl+wheel zooms, M toggles this bar"
                    } else {
                        "· click the edges or use ← → to turn pages, M toggles this bar"
                    })
                    .size(11.0)
                    .color(palette.text_dim),
                );
            });
        },
    );
}

// ---------------------------------------------------------------------------
// Settings sheet
// ---------------------------------------------------------------------------

pub fn settings_sheet(app: &mut App, ui: &mut Ui) -> bool {
    let palette = app.palette;
    let mut close = false;
    let mut changed = false;

    ui.label(RichText::new("Reader").size(17.0).strong());
    ui.add_space(8.0);

    egui::ScrollArea::vertical()
        .max_height(440.0)
        .id_salt("reader_settings")
        .show(ui, |ui| {
            ui.label(
                RichText::new("READING MODE")
                    .size(11.0)
                    .color(palette.text_dim),
            );
            for mode in ReadingMode::ALL {
                if ui.radio(app.reader.mode == mode, mode.label()).clicked() {
                    app.reader.mode = mode;
                    // Persist it on the manga, like upstream's per-series setting.
                    let manga_id = app.reader.manga_id;
                    let _ = app
                        .core
                        .db
                        .update_manga(manga_id, |m| m.set_reading_mode(Some(mode)));
                    app.refresh_details();
                }
            }

            ui.add_space(8.0);
            if ui.button("Use as the default for new series").clicked() {
                app.prefs.reader.default_reading_mode = app.reader.mode;
                changed = true;
                app.toast("default reading mode updated");
            }

            ui.add_space(12.0);
            ui.label(RichText::new("SCALE").size(11.0).color(palette.text_dim));
            for scale in ImageScaleType::ALL {
                if ui
                    .radio(app.prefs.reader.scale_type == scale, scale.label())
                    .clicked()
                {
                    app.prefs.reader.scale_type = scale;
                    changed = true;
                }
            }

            ui.add_space(12.0);
            ui.label(
                RichText::new("BACKGROUND")
                    .size(11.0)
                    .color(palette.text_dim),
            );
            ui.horizontal_wrapped(|ui| {
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

            ui.add_space(12.0);
            ui.label(RichText::new("PAGES").size(11.0).color(palette.text_dim));
            if ui
                .checkbox(&mut app.prefs.reader.crop_borders, "Crop borders")
                .changed()
            {
                // Cached textures were decoded with the old setting.
                app.page_textures.clear();
                changed = true;
            }
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.double_pages,
                    "Double pages (horizontal modes)",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.double_page_split_spreads,
                    "Give wide pages their own spread",
                )
                .changed();
            changed |= ui
                .checkbox(&mut app.prefs.reader.show_page_number, "Show page number")
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.mark_read_on_last_page,
                    "Mark chapter read on the last page",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.skip_read_chapters_on_finish,
                    "Skip already-read chapters when advancing",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.remove_after_read,
                    "Delete the download after reading",
                )
                .changed();
            changed |= ui
                .checkbox(
                    &mut app.prefs.reader.keyboard_navigation,
                    "Keyboard navigation",
                )
                .changed();

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.label("Preload");
                changed |= ui
                    .add(egui::Slider::new(
                        &mut app.prefs.reader.preload_pages,
                        0..=12,
                    ))
                    .changed();
            });

            if app.reader.mode.is_continuous() {
                ui.horizontal(|ui| {
                    ui.label("Page width");
                    changed |= ui
                        .add(
                            egui::Slider::new(
                                &mut app.prefs.reader.webtoon_width_fraction,
                                0.3..=1.0,
                            )
                            .show_value(false),
                        )
                        .changed();
                });
                ui.horizontal(|ui| {
                    ui.label("Page gap");
                    changed |= ui
                        .add(egui::Slider::new(
                            &mut app.prefs.reader.webtoon_page_gap,
                            0.0..=40.0,
                        ))
                        .changed();
                });
            }
        });

    ui.add_space(12.0);
    ui.horizontal(|ui| {
        if ui.button("Reset zoom").clicked() {
            app.reader.zoom = 1.0;
            app.reader.pan = Vec2::ZERO;
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

    fn segments() -> Vec<ReaderSegment> {
        // Two chapters back to back: 5 pages then 3.
        vec![
            ReaderSegment {
                chapter_id: 10,
                name: "Ch.1".into(),
                start: 0,
                len: 5,
            },
            ReaderSegment {
                chapter_id: 11,
                name: "Ch.2".into(),
                start: 5,
                len: 3,
            },
        ]
    }

    #[test]
    fn page_numbers_stay_chapter_local_across_a_boundary() {
        let segments = segments();
        // Third page of the first chapter.
        assert_eq!(local_index(&segments, 10, 2, 8), (2, 5));
        // First page of the second chapter, not "page 6 of 8".
        assert_eq!(local_index(&segments, 11, 5, 8), (0, 3));
        assert_eq!(local_index(&segments, 11, 7, 8), (2, 3));
    }

    #[test]
    fn an_unknown_chapter_falls_back_to_the_whole_run() {
        assert_eq!(local_index(&segments(), 99, 4, 8), (4, 8));
        assert_eq!(local_index(&[], 10, 3, 8), (3, 8));
    }

    #[test]
    fn infinite_is_continuous_but_the_others_are_not_infinite() {
        assert!(ReadingMode::Infinite.is_continuous());
        assert!(ReadingMode::Infinite.is_infinite());
        assert!(ReadingMode::Webtoon.is_continuous());
        assert!(!ReadingMode::Webtoon.is_infinite());
        assert!(!ReadingMode::RightToLeft.is_continuous());
        // The flag must round-trip so a per-manga override survives a restart.
        assert_eq!(
            ReadingMode::from_flag(ReadingMode::Infinite.flag()),
            Some(ReadingMode::Infinite)
        );
    }

    #[test]
    fn fit_screen_keeps_the_whole_page_visible() {
        let size = fit_size(
            vec2(1000.0, 1500.0),
            vec2(800.0, 600.0),
            ImageScaleType::FitScreen,
        );
        assert!(size.x <= 800.5 && size.y <= 600.5);
        // Aspect ratio preserved.
        assert!((size.x / size.y - 1000.0 / 1500.0).abs() < 0.001);
    }

    #[test]
    fn fit_width_fills_horizontally() {
        let size = fit_size(
            vec2(1000.0, 1500.0),
            vec2(800.0, 600.0),
            ImageScaleType::FitWidth,
        );
        assert!((size.x - 800.0).abs() < 0.001);
        assert!(size.y > 600.0, "a tall page should overflow vertically");
    }

    #[test]
    fn stretch_ignores_aspect_ratio() {
        let area = vec2(640.0, 480.0);
        assert_eq!(
            fit_size(vec2(100.0, 900.0), area, ImageScaleType::StretchToFit),
            area
        );
    }

    #[test]
    fn original_size_is_untouched() {
        let image = vec2(300.0, 400.0);
        assert_eq!(
            fit_size(image, vec2(1000.0, 1000.0), ImageScaleType::OriginalSize),
            image
        );
    }

    #[test]
    fn smart_fit_treats_spreads_as_width_first() {
        // A landscape page fits the width; a portrait one fits the screen.
        let wide = fit_size(
            vec2(2000.0, 1000.0),
            vec2(800.0, 600.0),
            ImageScaleType::SmartFit,
        );
        assert!((wide.x - 800.0).abs() < 0.001);
        let tall = fit_size(
            vec2(1000.0, 2000.0),
            vec2(800.0, 600.0),
            ImageScaleType::SmartFit,
        );
        assert!(tall.y <= 600.5);
    }

    #[test]
    fn degenerate_images_do_not_divide_by_zero() {
        let area = vec2(400.0, 300.0);
        assert_eq!(
            fit_size(vec2(0.0, 0.0), area, ImageScaleType::FitScreen),
            area
        );
    }
}
