//! Extension management: installed manifests, repositories, installation.

use egui::{Color32, RichText, Ui};

use super::widgets;
use super::{App, Dialog};
use crate::source::ext::{self, RepoEntry};

pub fn show(app: &mut App, ui: &mut Ui) {
    egui::Panel::top("extensions_top")
        .frame(super::theme::header_frame(&app.palette))
        .show(ui, |ui| {
            widgets::screen_header(
                app,
                ui,
                "Extensions",
                Some("Sources loaded from JSON manifests"),
            );
        });

    egui::CentralPanel::default()
        .frame(super::theme::body_frame(&app.palette))
        .show(ui, |ui| show_inline(app, ui));
}

pub fn show_inline(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;

    ui.horizontal(|ui| {
        widgets::search_field(ui, &palette, "Filter extensions", &mut app.extensions.query);
        widgets::toolbar_actions(ui, |ui| {
            if ui.button("Add repository").clicked() {
                app.dialog = Some(Dialog::AddRepo { url: String::new() });
            }
            if ui.button("Install from file…").clicked() {
                install_from_file(app);
            }
            if ui.button("Refresh").clicked() {
                app.core.reload_extensions();
                app.extensions.dirty = true;
                for repo in app.prefs.browse.extension_repos.clone() {
                    app.extensions.loading = true;
                    app.core.fetch_repo(repo);
                }
            }
        });
    });
    ui.add_space(8.0);

    let needle = app.extensions.query.trim().to_lowercase();

    egui::ScrollArea::vertical()
        .id_salt("extensions_scroll")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            installed_section(app, ui, &needle);
            ui.add_space(12.0);
            repositories_section(app, ui);
            ui.add_space(12.0);
            available_section(app, ui, &needle);
            ui.add_space(12.0);
            format_help(app, ui);
        });
}

fn installed_section(app: &mut App, ui: &mut Ui, needle: &str) {
    let palette = app.palette;
    let installed = app.extensions.installed.clone();

    ui.label(
        RichText::new(format!("INSTALLED ({})", installed.len()))
            .size(11.0)
            .color(palette.text_dim),
    );
    ui.add_space(4.0);

    if installed.is_empty() {
        ui.label(
            RichText::new("No extensions installed. MangaDex and the local source are built in.")
                .size(12.0)
                .color(palette.text_dim),
        );
        return;
    }

    let mut to_remove = None;
    for extension in &installed {
        let manifest = &extension.manifest;
        if !needle.is_empty() && !manifest.name.to_lowercase().contains(needle) {
            continue;
        }

        super::theme::card(&palette).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&manifest.name).strong());
                        if manifest.nsfw {
                            widgets::badge(ui, "18+", palette.error, Color32::WHITE);
                        }
                    });
                    let mut meta = format!(
                        "{} · v{} · {}",
                        manifest.lang.to_uppercase(),
                        manifest.version,
                        manifest.base_url
                    );
                    if let Some(author) = &manifest.author {
                        meta.push_str(&format!(" · by {author}"));
                    }
                    ui.label(RichText::new(meta).size(11.5).color(palette.text_dim));
                    if let Some(description) = &manifest.description {
                        ui.label(
                            RichText::new(description)
                                .size(11.5)
                                .color(palette.text_dim),
                        );
                    }
                });

                widgets::toolbar_actions(ui, |ui| {
                    if ui.button("Uninstall").clicked() {
                        to_remove = Some(extension.path.clone());
                    }
                    let loaded = app.core.sources.get(extension.source_id).is_some();
                    if !loaded {
                        ui.label(RichText::new("not loaded").size(11.0).color(palette.error));
                    }
                });
            });
        });
    }

    if let Some(path) = to_remove {
        match ext::uninstall(&path) {
            Ok(()) => {
                app.core.reload_extensions();
                app.extensions.dirty = true;
                app.toast("extension uninstalled");
            }
            Err(err) => app.toast_error(format!("could not uninstall: {err}")),
        }
    }
}

fn repositories_section(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    let repos = app.prefs.browse.extension_repos.clone();

    ui.label(
        RichText::new("REPOSITORIES")
            .size(11.0)
            .color(palette.text_dim),
    );
    ui.add_space(4.0);

    if repos.is_empty() {
        ui.label(
            RichText::new("No repository configured. Add one to browse installable extensions.")
                .size(12.0)
                .color(palette.text_dim),
        );
        return;
    }

    let mut to_remove = None;
    for repo in &repos {
        ui.horizontal(|ui| {
            ui.label(RichText::new(repo).size(12.0));
            widgets::toolbar_actions(ui, |ui| {
                if ui.small_button("✖").on_hover_text("Remove").clicked() {
                    to_remove = Some(repo.clone());
                }
                if ui.small_button("🔄").on_hover_text("Refresh").clicked() {
                    app.extensions.loading = true;
                    app.core.fetch_repo(repo.clone());
                }
            });
        });
    }

    if let Some(repo) = to_remove {
        app.prefs.browse.extension_repos.retain(|r| *r != repo);
        app.extensions.available.remove(&repo);
        app.prefs_changed();
    }
}

fn available_section(app: &mut App, ui: &mut Ui, needle: &str) {
    let palette = app.palette;

    if app.extensions.loading {
        widgets::spinner_row(ui, &palette, "Loading repositories…");
        return;
    }
    if app.extensions.available.is_empty() {
        return;
    }

    let installed_ids: Vec<String> = app
        .extensions
        .installed
        .iter()
        .map(|e| e.manifest.id.clone())
        .collect();

    let available = app.extensions.available.clone();
    let mut to_install: Option<RepoEntry> = None;

    for (repo, entries) in &available {
        ui.label(
            RichText::new(format!("AVAILABLE — {repo}"))
                .size(11.0)
                .color(palette.text_dim),
        );
        ui.add_space(4.0);

        for entry in entries {
            if !needle.is_empty() && !entry.name.to_lowercase().contains(needle) {
                continue;
            }
            let already = installed_ids.contains(&entry.id);

            super::theme::card(&palette).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(RichText::new(&entry.name).strong());
                            if entry.nsfw {
                                widgets::badge(ui, "18+", palette.error, Color32::WHITE);
                            }
                        });
                        let description = entry
                            .description
                            .clone()
                            .unwrap_or_else(|| entry.url.clone());
                        ui.label(
                            RichText::new(format!(
                                "{} · v{} · {description}",
                                entry.lang.to_uppercase(),
                                entry.version
                            ))
                            .size(11.5)
                            .color(palette.text_dim),
                        );
                    });

                    widgets::toolbar_actions(ui, |ui| {
                        if already {
                            ui.label(RichText::new("installed").size(11.5).color(palette.success));
                        } else if ui.button("Install").clicked() {
                            to_install = Some(entry.clone());
                        }
                    });
                });
            });
        }
        ui.add_space(8.0);
    }

    if let Some(entry) = to_install {
        app.core.install_extension(entry);
    }
}

fn install_from_file(app: &mut App) {
    let Some(path) = rfd::FileDialog::new()
        .add_filter("Extension manifest", &["json"])
        .set_title("Pick an extension manifest")
        .pick_file()
    else {
        return;
    };

    match ext::install_from_file(&app.core.paths.extensions, &path) {
        Ok(_) => {
            app.core.reload_extensions();
            app.extensions.dirty = true;
            app.toast("extension installed");
        }
        Err(err) => app.toast_error(format!("could not install: {err}")),
    }
}

/// A short reference so a user can write their own extension without docs.
fn format_help(app: &mut App, ui: &mut Ui) {
    let palette = app.palette;
    egui::CollapsingHeader::new("Writing an extension")
        .id_salt("ext_help")
        .show(ui, |ui| {
            ui.label(
                RichText::new(
                    "An extension is a JSON file describing where each value lives in a \
                     site's HTML or JSON responses. Drop it in the extensions folder, or \
                     use “Install from file”.",
                )
                .size(12.0)
                .color(palette.text_dim),
            );
            ui.add_space(6.0);
            if ui.button("Open extensions folder").clicked() {
                let path = app.core.paths.extensions.clone();
                if let Err(err) = open_folder(&path) {
                    app.toast_error(format!("could not open the folder: {err}"));
                }
            }
            ui.add_space(6.0);
            ui.add(
                egui::TextEdit::multiline(&mut EXAMPLE.to_string())
                    .code_editor()
                    .desired_width(f32::INFINITY)
                    .desired_rows(18)
                    .interactive(false),
            );
        });
}

pub fn open_folder(path: &std::path::Path) -> std::io::Result<()> {
    // `explorer` returns a non-zero exit code even on success, so the status is
    // deliberately ignored here.
    std::process::Command::new("explorer")
        .arg(path)
        .spawn()
        .map(|_| ())
}

const EXAMPLE: &str = r#"{
  "id": "example-en",
  "name": "Example",
  "lang": "en",
  "baseUrl": "https://example.com",
  "version": 1,
  "rateLimit": { "permits": 2, "periodMs": 1000 },
  "popular": {
    "url": "{base}/popular?page={page}",
    "list": {
      "item": "div.manga-card",
      "title": "h3 a",
      "url": { "selector": "h3 a", "attr": "href" },
      "thumbnail": { "selector": "img", "attr": "data-src" }
    },
    "nextPage": "a.next-page"
  },
  "search": { "url": "{base}/search?q={query}&page={page}{filters}", "list": { ... } },
  "details": {
    "url": "{base}{url}",
    "fields": {
      "title": "h1.title",
      "author": "span.author",
      "description": "div.summary",
      "genre": { "selector": "a.genre", "all": true },
      "status": { "selector": "span.status" },
      "thumbnail": { "selector": "img.cover", "attr": "src" }
    }
  },
  "chapters": {
    "url": "{base}{url}",
    "list": {
      "item": "li.chapter",
      "name": "a",
      "url": { "selector": "a", "attr": "href" },
      "date": { "selector": "span.date", "dateFormat": "%b %d, %Y" }
    }
  },
  "pages": {
    "url": "{base}{url}",
    "regex": { "pattern": "\"page_url\":\"(.*?)\"" }
  }
}"#;
