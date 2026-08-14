//! The Updates tab: chapters that recently appeared in library entries.

use egui::{Align, Layout, RichText, Ui, vec2};

use super::App;
use super::widgets::{self, COVER_ASPECT};
use crate::download::DownloadState;
use crate::model::*;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("updates_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new("Updates").size(20.0).strong());
                    ui.label(
                        RichText::new(format!("{} recent chapter(s)", app.updates.entries.len()))
                            .size(12.0)
                            .color(palette.text_dim),
                    );
                });
                ui.add_space(12.0);
                widgets::search_field(ui, &palette, "Filter by title", &mut app.updates.query);

                widgets::toolbar_actions(ui, |ui| {
                    let updating = app.core.is_updating();
                    if ui
                        .add_enabled(!updating, egui::Button::new("🔄 Update library"))
                        .clicked()
                    {
                        app.core.update_library(&app.prefs, None);
                    }
                });
            });

            if let Some((done, total, current)) = app.update_progress.clone() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    ui.add(
                        egui::ProgressBar::new(done as f32 / total.max(1) as f32)
                            .desired_width(220.0)
                            .desired_height(6.0),
                    );
                    ui.label(
                        RichText::new(format!("{done}/{total} · {current}"))
                            .size(11.5)
                            .color(palette.text_dim),
                    );
                });
            }
        });

    if !app.updates.selection.is_empty() {
        egui::Panel::top("updates_selection")
            .frame(
                egui::Frame::NONE
                    .fill(palette.accent.gamma_multiply(0.18))
                    .inner_margin(egui::Margin::symmetric(14, 8)),
            )
            .show(ui, |ui| selection_bar(app, ui));
    }

    let rows = filtered(app);

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| {
            if rows.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    "🔄",
                    "No updates yet",
                    "Update your library to check sources for new chapters.",
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("updates_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let mut last_bucket = String::new();
                    for entry in &rows {
                        let bucket = day_bucket(entry.chapter.date_fetch);
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

fn row(app: &mut App, ui: &mut Ui, entry: &UpdatesEntry) {
    let palette = app.palette;
    let selected = app.updates.selection.contains(&entry.chapter.id);
    let download_state = app.core.downloads.state_of(entry.chapter.id);
    // Buttons sit inside the row's click area and must claim their click.
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
                    let (rect, _) = ui
                        .allocate_exact_size(vec2(32.0, 32.0 * COVER_ASPECT), egui::Sense::hover());
                    widgets::paint_cover(
                        app,
                        ui,
                        rect,
                        entry.cover_url.as_deref(),
                        entry.source,
                        &entry.manga_title,
                    );

                    ui.add_space(8.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new(&entry.manga_title).strong().color(
                            if entry.chapter.read {
                                palette.text_dim
                            } else {
                                palette.text
                            },
                        ));
                        ui.label(
                            RichText::new(&entry.chapter.name)
                                .size(11.5)
                                .color(palette.text_dim),
                        );
                    });

                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        match &download_state {
                            Some(DownloadState::Running { done, total }) => {
                                ui.add(
                                    egui::ProgressBar::new(*done as f32 / (*total).max(1) as f32)
                                        .desired_width(52.0)
                                        .desired_height(6.0),
                                );
                            }
                            Some(DownloadState::Queued) => {
                                ui.label(
                                    RichText::new("queued").size(11.0).color(palette.text_dim),
                                );
                            }
                            _ => {
                                if entry.downloaded {
                                    if ui.button("🗑").on_hover_text("Delete download").clicked()
                                    {
                                        consumed = true;
                                        app.core
                                            .downloads
                                            .delete_chapter(entry.manga_id, entry.chapter.id);
                                        app.invalidate_all();
                                    }
                                } else if ui.button("📥").on_hover_text("Download").clicked() {
                                    consumed = true;
                                    app.core
                                        .queue_downloads(entry.manga_id, &[entry.chapter.id]);
                                }
                            }
                        }
                        if ui
                            .button(if entry.chapter.read { "✔" } else { "○" })
                            .on_hover_text(if entry.chapter.read {
                                "Mark unread"
                            } else {
                                "Mark read"
                            })
                            .clicked()
                        {
                            consumed = true;
                            let read = entry.chapter.read;
                            let _ = app.core.db.update_chapter(entry.chapter.id, |c| {
                                c.read = !read;
                                c.last_page_read = 0;
                            });
                            app.invalidate_all();
                        }
                    });
                });
            });
    })
    .response;

    if response.clicked() && !consumed {
        if app.updates.selection.is_empty() {
            app.open_reader(entry.manga_id, entry.chapter.id);
        } else {
            toggle(app, entry.chapter.id);
        }
    }
    if response.secondary_clicked() {
        toggle(app, entry.chapter.id);
    }
}

fn toggle(app: &mut App, chapter_id: Id) {
    if !app.updates.selection.remove(&chapter_id) {
        app.updates.selection.insert(chapter_id);
    }
}

fn selection_bar(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let selected: Vec<Id> = app.updates.selection.iter().copied().collect();

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("{} selected", selected.len()))
                .strong()
                .color(palette.text),
        );
        ui.add_space(10.0);

        if ui.button("Mark read").clicked() {
            let _ = app.core.db.update_chapters(&selected, |c| {
                c.read = true;
                c.last_page_read = 0;
            });
            app.invalidate_all();
        }
        if ui.button("Mark unread").clicked() {
            let _ = app.core.db.update_chapters(&selected, |c| {
                c.read = false;
                c.last_page_read = 0;
            });
            app.invalidate_all();
        }
        if ui.button("Download").clicked() {
            // Chapters can span several manga, so queue them per entry.
            let jobs: Vec<(Id, Id)> = app
                .updates
                .entries
                .iter()
                .filter(|e| selected.contains(&e.chapter.id))
                .map(|e| (e.manga_id, e.chapter.id))
                .collect();
            for (manga_id, chapter_id) in jobs {
                app.core.queue_downloads(manga_id, &[chapter_id]);
            }
        }

        widgets::toolbar_actions(ui, |ui| {
            if ui.button("✖").on_hover_text("Clear selection").clicked() {
                app.updates.selection.clear();
            }
        });
    });
}

fn filtered(app: &App) -> Vec<UpdatesEntry> {
    let needle = app.updates.query.trim().to_lowercase();
    app.updates
        .entries
        .iter()
        .filter(|e| needle.is_empty() || e.manga_title.to_lowercase().contains(&needle))
        .cloned()
        .collect()
}

/// Groups rows into Today / Yesterday / an absolute date.
pub fn day_bucket(millis: i64) -> String {
    let Some(datetime) = chrono::DateTime::from_timestamp_millis(millis) else {
        return "Unknown date".into();
    };
    let date = datetime.with_timezone(&chrono::Local).date_naive();
    let today = chrono::Local::now().date_naive();

    match (today - date).num_days() {
        0 => "Today".to_string(),
        1 => "Yesterday".to_string(),
        days if days < 7 => format!("{days} days ago"),
        _ => date.format("%d %B %Y").to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets_name_recent_days() {
        let now = chrono::Local::now();
        assert_eq!(day_bucket(now.timestamp_millis()), "Today");

        let yesterday = now - chrono::Duration::days(1);
        assert_eq!(day_bucket(yesterday.timestamp_millis()), "Yesterday");

        let old = now - chrono::Duration::days(40);
        let label = day_bucket(old.timestamp_millis());
        assert!(label.contains(&old.format("%Y").to_string()), "got {label}");
    }

    #[test]
    fn invalid_timestamps_do_not_panic() {
        assert_eq!(day_bucket(i64::MAX), "Unknown date");
    }
}
