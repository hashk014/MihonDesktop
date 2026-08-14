//! Mihon Desktop — a manga reader for the desktop, rebuilt in Rust.
//!
//! Feature set and information architecture follow the Mihon Android app:
//! a categorised library, chapter updates, reading history, source browsing
//! with extensions, a paged/webtoon reader, downloads, and backups.

// The window should not drag a console along on Windows release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backup;
mod core;
mod db;
mod download;
mod event;
mod images;
mod model;
mod net;
mod prefs;
mod source;
mod ui;

use anyhow::{Context, Result};

use crate::core::Core;
use crate::event::EventBus;
use crate::prefs::{AppPaths, Preferences};

fn main() -> eframe::Result {
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,eframe=warn,egui_glow=warn"),
    )
    .init();

    let paths = AppPaths::resolve();
    // A release build has no console, so an unhandled panic would otherwise
    // make the window vanish with nothing to go on.
    install_panic_handler(paths.root.join("crash.log"));

    if let Err(err) = paths.ensure() {
        fatal(&format!(
            "Mihon Desktop could not create its data directory at {}:\n\n{err}",
            paths.root.display()
        ));
        return Ok(());
    }
    let prefs = Preferences::load(&paths.prefs);
    log::info!("data directory: {}", paths.root.display());

    let bus = EventBus::new();
    let core = match Core::new(paths.clone(), &prefs, &bus) {
        Ok(core) => core,
        Err(err) => {
            fatal(&format!("Mihon Desktop failed to start:\n\n{err:#}"));
            return Ok(());
        }
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("Mihon Desktop")
            .with_inner_size([1360.0, 880.0])
            .with_min_inner_size([900.0, 600.0])
            .with_app_id("mihon-desktop"),
        ..Default::default()
    };

    eframe::run_native(
        "Mihon Desktop",
        options,
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            // Must happen before the first frame: manga titles are mostly CJK.
            ui::fonts::install(&cc.egui_ctx);
            Ok(Box::new(ui::App::new(paths, prefs, bus, core)))
        }),
    )
}

/// Writes any panic to a log file and tells the user where to find it.
fn install_panic_handler(log_path: std::path::PathBuf) {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        default_hook(info);

        let location = info
            .location()
            .map(|l| format!("{}:{}:{}", l.file(), l.line(), l.column()))
            .unwrap_or_else(|| "unknown location".into());
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| (*s).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".into());
        let backtrace = std::backtrace::Backtrace::force_capture();

        let report = format!(
            "----- {} -----\nversion: {}\nthread: {}\nlocation: {location}\nmessage: {message}\n\n{backtrace}\n",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            env!("CARGO_PKG_VERSION"),
            std::thread::current().name().unwrap_or("unnamed"),
        );

        if let Some(parent) = log_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let appended = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)
            .and_then(|mut file| std::io::Write::write_all(&mut file, report.as_bytes()));

        // Only the first panic gets a dialog; a cascade would bury the screen.
        static REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if REPORTED.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }

        let where_to_look = if appended.is_ok() {
            format!("\n\nDetails were written to:\n{}", log_path.display())
        } else {
            String::new()
        };
        rfd::MessageDialog::new()
            .set_title("Mihon Desktop crashed")
            .set_description(format!("{message}\n\nat {location}{where_to_look}"))
            .set_level(rfd::MessageLevel::Error)
            .show();
    }));
}

/// Startup failures happen before there is a window to report them in.
fn fatal(message: &str) {
    log::error!("{message}");
    rfd::MessageDialog::new()
        .set_title("Mihon Desktop")
        .set_description(message)
        .set_level(rfd::MessageLevel::Error)
        .show();
}

/// Loads a backup file from disk, used by the settings screen.
pub fn restore_backup(db: &db::Db, path: &std::path::Path) -> Result<backup::Backup> {
    let backup = backup::Backup::read_from(path)?;
    db.import(backup.library.clone())
        .context("writing the restored library")?;
    Ok(backup)
}
