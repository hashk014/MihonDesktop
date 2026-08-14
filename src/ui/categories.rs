//! Category management: create, rename, reorder, delete.

use egui::{Align, Color32, Layout, RichText, Ui};

use super::widgets;
use super::{App, Dialog};
use crate::model::{Category, Id};

pub fn show(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    egui::Panel::top("categories_top")
        .frame(super::theme::plain(14))
        .show(ui, |ui| {
            widgets::screen_header(
                app,
                ui,
                "Categories",
                Some("Group library entries into tabs"),
            );
            ui.add_space(8.0);
            if ui.button("➕ New category").clicked() {
                app.dialog = Some(Dialog::NewCategory {
                    name: String::new(),
                });
            }
        });

    let categories = app.core.db.categories();

    egui::CentralPanel::default()
        .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(14, 6)))
        .show(ui, |ui| {
            if categories.iter().all(|c| c.is_system()) {
                widgets::empty_state(
                    ui,
                    &palette,
                    "🏷",
                    "No categories yet",
                    "Everything lives in Default until you add one.",
                );
                return;
            }

            let mut move_up: Option<Id> = None;
            let mut move_down: Option<Id> = None;
            let mut delete: Option<Id> = None;
            let mut rename: Option<Category> = None;

            egui::ScrollArea::vertical()
                .id_salt("categories_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let last = categories.len().saturating_sub(1);
                    for (index, category) in categories.iter().enumerate() {
                        super::theme::card(&palette).show(ui, |ui| {
                            ui.set_width(ui.available_width());
                            ui.horizontal(|ui| {
                                ui.label(RichText::new(&category.name).strong());
                                if category.is_system() {
                                    ui.label(
                                        RichText::new("built-in")
                                            .size(11.0)
                                            .color(palette.text_dim),
                                    );
                                }
                                let count = app
                                    .library
                                    .entries
                                    .iter()
                                    .filter(|e| {
                                        if e.category_ids.is_empty() {
                                            category.id == Category::DEFAULT_ID
                                        } else {
                                            e.category_ids.contains(&category.id)
                                        }
                                    })
                                    .count();

                                ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                    if !category.is_system() {
                                        let remove = egui::Button::new(
                                            RichText::new("Delete").color(Color32::WHITE),
                                        )
                                        .fill(palette.error);
                                        if ui.add(remove).clicked() {
                                            delete = Some(category.id);
                                        }
                                        if ui.button("Rename").clicked() {
                                            rename = Some(category.clone());
                                        }
                                    }
                                    if ui
                                        .add_enabled(index < last, egui::Button::new("⬇"))
                                        .clicked()
                                    {
                                        move_down = Some(category.id);
                                    }
                                    if ui.add_enabled(index > 0, egui::Button::new("⬆")).clicked()
                                    {
                                        move_up = Some(category.id);
                                    }
                                    ui.label(
                                        RichText::new(format!("{count}"))
                                            .size(11.5)
                                            .color(palette.text_dim),
                                    );
                                });
                            });
                        });
                    }
                });

            if let Some(id) = move_up {
                reorder(app, &categories, id, -1);
            }
            if let Some(id) = move_down {
                reorder(app, &categories, id, 1);
            }
            if let Some(category) = rename {
                app.dialog = Some(Dialog::RenameCategory {
                    id: category.id,
                    name: category.name,
                });
            }
            if let Some(id) = delete {
                match app.core.db.delete_category(id) {
                    Ok(()) => {
                        app.library.categories = app.core.db.categories();
                        app.library.category_index = 0;
                        app.library.dirty = true;
                        app.toast("category deleted");
                    }
                    Err(err) => app.toast_error(format!("could not delete: {err}")),
                }
            }
        });
}

fn reorder(app: &mut App, categories: &[Category], id: Id, delta: i32) {
    let Some(index) = categories.iter().position(|c| c.id == id) else {
        return;
    };
    let target = index as i32 + delta;
    if target < 0 || target as usize >= categories.len() {
        return;
    }

    let mut ids: Vec<Id> = categories.iter().map(|c| c.id).collect();
    ids.swap(index, target as usize);

    match app.core.db.reorder_categories(&ids) {
        Ok(()) => {
            app.library.categories = app.core.db.categories();
            app.library.dirty = true;
        }
        Err(err) => app.toast_error(format!("could not reorder: {err}")),
    }
}
