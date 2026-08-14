//! The History tab: recently read chapters, with a resume shortcut.

use egui::{Align, Layout, RichText, Ui, vec2};

use super::widgets::{self, COVER_ASPECT};
use super::{App, Dialog};
use crate::model::*;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("history_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("History").size(20.0).strong());
                    ui.label(
                        RichText::new(format!(
                            "{} entr{}",
                            app.history.entries.len(),
                            if app.history.entries.len() == 1 {
                                "y"
                            } else {
                                "ies"
                            }
                        ))
                        .size(12.0)
                        .color(palette.text_dim),
                    );
                });
                ui.add_space(12.0);
                let response =
                    widgets::search_field(ui, &palette, "Search history", &mut app.history.query);
                if response.changed() {
                    app.history.dirty = true;
                }

                widgets::toolbar_actions(ui, |ui| {
                    if ui.button("Clear all").clicked() {
                        app.dialog = Some(Dialog::ConfirmClearHistory);
                    }
                });
            });
        });

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            if app.history.entries.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    "🕘",
                    "Nothing read yet",
                    "Chapters you open will show up here.",
                );
                return;
            }

            let entries = app.history.entries.clone();
            let mut last_bucket = String::new();

            egui::ScrollArea::vertical()
                .id_salt("history_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &entries {
                        let bucket = super::updates::day_bucket(entry.history.read_at);
                        if bucket != last_bucket {
                            ui.add_space(8.0);
                            ui.label(RichText::new(&bucket).size(11.5).color(palette.text_dim));
                            ui.add_space(2.0);
                            last_bucket = bucket;
                        }
                        row(app, ui, entry);
                    }
                    ui.add_space(20.0);
                });
        });
}

fn row(app: &mut App, ui: &mut Ui, entry: &HistoryEntry) {
    let palette = app.palette;
    let mut remove = false;
    let mut resume = false;
    let mut open_details = false;

    let response = widgets::clickable_row(ui, |ui| {
        super::theme::card(&palette)
            .inner_margin(egui::Margin::symmetric(10, 7))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    let (rect, cover_response) = ui
                        .allocate_exact_size(vec2(32.0, 32.0 * COVER_ASPECT), egui::Sense::click());
                    widgets::paint_cover(
                        app,
                        ui,
                        rect,
                        entry.cover_url.as_deref(),
                        entry.source,
                        &entry.manga_title,
                    );
                    if cover_response.clicked() {
                        open_details = true;
                    }

                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.manga_title).strong());
                        ui.label(
                            RichText::new(&entry.chapter.name)
                                .size(11.5)
                                .color(palette.text_dim),
                        );
                        let mut meta = vec![widgets::format_timestamp(
                            entry.history.read_at,
                            app.prefs.relative_timestamps,
                        )];
                        if entry.history.time_read > 0 {
                            meta.push(format_duration(entry.history.time_read));
                        }
                        ui.label(
                            RichText::new(meta.join(" · "))
                                .size(11.0)
                                .color(palette.text_dim),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .button("✖")
                            .on_hover_text("Remove from history")
                            .clicked()
                        {
                            remove = true;
                        }
                        if ui.button("▶ Resume").clicked() {
                            resume = true;
                        }
                    });
                });
            });
    })
    .response;

    // `remove` and `open_details` are buttons inside the row, so they take
    // precedence over the row's own click.
    if remove {
        match app.core.db.remove_history(entry.chapter.id) {
            Ok(()) => app.history.dirty = true,
            Err(err) => app.toast_error(format!("could not remove the entry: {err}")),
        }
    } else if resume || (response.clicked() && !open_details) {
        app.open_reader(entry.manga_id, entry.chapter.id);
    } else if open_details {
        app.open_manga(entry.manga_id);
    }
}

/// Renders a reading duration compactly ("12 min", "1 h 05").
pub fn format_duration(millis: i64) -> String {
    let seconds = millis / 1000;
    if seconds < 60 {
        return format!("{seconds} s");
    }
    let minutes = seconds / 60;
    if minutes < 60 {
        return format!("{minutes} min");
    }
    format!("{} h {:02}", minutes / 60, minutes % 60)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn durations_scale_by_unit() {
        assert_eq!(format_duration(5_000), "5 s");
        assert_eq!(format_duration(120_000), "2 min");
        assert_eq!(format_duration(3_900_000), "1 h 05");
    }
}
