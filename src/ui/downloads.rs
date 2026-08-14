//! The download queue screen.

use egui::{Align, Layout, RichText, Ui};

use super::App;
use super::widgets;
use crate::download::DownloadState;

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let queue = app.core.downloads.queue_snapshot();
    let paused = app.core.downloads.is_paused();

    egui::Panel::top("downloads_top")
        .frame(super::theme::plain(14))
        .show(ui, |ui| {
            widgets::screen_header(
                app,
                ui,
                "Download queue",
                Some(&format!(
                    "{} queued · {} on disk",
                    queue.len(),
                    widgets::format_bytes(app.core.downloads.storage_used())
                )),
            );
            ui.add_space(8.0);

            ui.horizontal(|ui| {
                let label = if paused { "▶ Resume" } else { "⏸ Pause" };
                if ui.button(label).clicked() {
                    app.core.downloads.set_paused(!paused);
                }
                if ui
                    .add_enabled(!queue.is_empty(), egui::Button::new("Clear queue"))
                    .clicked()
                {
                    app.core.downloads.clear_queue();
                }
                if ui.button("Open folder").clicked() {
                    let path = app.prefs.downloads_dir(&app.core.paths);
                    if let Err(err) = super::extensions::open_folder(&path) {
                        app.toast_error(format!("could not open the folder: {err}"));
                    }
                }
                if paused {
                    ui.label(
                        RichText::new("downloads paused")
                            .size(11.5)
                            .color(palette.accent),
                    );
                }
            });
        });

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 6)))
        .show(ui, |ui| {
            if queue.is_empty() {
                widgets::empty_state(
                    ui,
                    &palette,
                    "📥",
                    "The queue is empty",
                    "Download chapters from a manga's page or from Updates.",
                );
                return;
            }

            egui::ScrollArea::vertical()
                .id_salt("download_queue")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for (job, state) in &queue {
                        super::theme::card(&palette).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(RichText::new(&job.manga_title).strong());
                                    ui.label(
                                        RichText::new(&job.chapter_name)
                                            .size(11.5)
                                            .color(palette.text_dim),
                                    );
                                    match state {
                                        DownloadState::Running { done, total } => {
                                            ui.add(
                                                egui::ProgressBar::new(
                                                    *done as f32 / (*total).max(1) as f32,
                                                )
                                                .desired_width(260.0)
                                                .desired_height(6.0)
                                                .text(format!("{done}/{total} pages")),
                                            );
                                        }
                                        DownloadState::Queued => {
                                            ui.label(
                                                RichText::new("waiting")
                                                    .size(11.0)
                                                    .color(palette.text_dim),
                                            );
                                        }
                                        DownloadState::Finished => {
                                            ui.label(
                                                RichText::new("done")
                                                    .size(11.0)
                                                    .color(palette.success),
                                            );
                                        }
                                        DownloadState::Failed(error) => {
                                            ui.label(
                                                RichText::new(format!("failed — {error}"))
                                                    .size(11.0)
                                                    .color(palette.error),
                                            );
                                        }
                                    }
                                });

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if ui.button("✖").on_hover_text("Cancel").clicked() {
                                        app.core.downloads.cancel(job.chapter_id);
                                    }
                                    if ui.button("⬇").on_hover_text("Move to bottom").clicked() {
                                        app.core.downloads.move_to_bottom(job.chapter_id);
                                    }
                                    if ui.button("⬆").on_hover_text("Move to top").clicked() {
                                        app.core.downloads.move_to_top(job.chapter_id);
                                    }
                                });
                            });
                        });
                    }
                    ui.add_space(20.0);
                });
        });

    // Progress arrives from a worker thread; keep the bars moving.
    if queue
        .iter()
        .any(|(_, state)| matches!(state, DownloadState::Running { .. } | DownloadState::Queued))
    {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(300));
    }
}
