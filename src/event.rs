//! Messages flowing from background work back to the UI thread.
//!
//! egui redraws on the main thread only, so every task communicates through
//! this channel and asks for a repaint. Nothing else crosses the boundary.

use std::sync::Arc;
use std::sync::mpsc::{Receiver, Sender, channel};

use crate::download::DownloadState;
use crate::images::ImageKind;
use crate::model::Id;
use crate::source::MangasPage;
use crate::source::ext::RepoEntry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToastKind {
    Info,
    Error,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub message: String,
    pub kind: ToastKind,
    /// Frame time (seconds) remaining before it fades out.
    pub remaining: f32,
}

impl Toast {
    pub fn info(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Info,
            remaining: 4.0,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            kind: ToastKind::Error,
            remaining: 7.0,
        }
    }
}

pub enum AppEvent {
    Toast(Toast),
    /// Library rows need rebuilding (favourite toggled, chapters synced, ...).
    LibraryChanged,
    ExtensionsChanged,

    BrowseLoaded {
        source: Id,
        page: u32,
        result: Result<MangasPage, String>,
    },
    GlobalSearchLoaded {
        source: Id,
        result: Result<MangasPage, String>,
    },
    DetailsLoaded {
        manga_id: Id,
        result: Result<(), String>,
    },
    ChaptersLoaded {
        manga_id: Id,
        new_chapters: usize,
        result: Result<(), String>,
    },
    PagesLoaded {
        chapter_id: Id,
        result: Result<Vec<crate::source::Page>, String>,
    },

    ImageLoaded {
        kind: ImageKind,
        key: String,
        result: Result<crate::images::DecodedImage, String>,
    },

    /// A chapter's download advanced. The payload lets a screen react without
    /// polling the manager, and marks the library rows stale.
    DownloadProgress {
        #[allow(dead_code)]
        chapter_id: Id,
        #[allow(dead_code)]
        state: DownloadState,
    },
    DownloadQueueChanged,

    LibraryUpdateProgress {
        done: usize,
        total: usize,
        current: String,
    },
    LibraryUpdateFinished {
        new_chapters: usize,
        failed: usize,
    },

    RepoLoaded {
        url: String,
        result: Result<Vec<RepoEntry>, String>,
    },
}

/// Cloneable handle used by background tasks to talk to the UI.
#[derive(Clone)]
pub struct EventSender {
    tx: Sender<AppEvent>,
    /// Set once the first frame has run, so tasks can wake a sleeping window.
    ctx: Arc<parking_lot::Mutex<Option<egui::Context>>>,
}

impl EventSender {
    pub fn send(&self, event: AppEvent) {
        // A closed channel only happens while the app is tearing down.
        if self.tx.send(event).is_err() {
            return;
        }
        if let Some(ctx) = self.ctx.lock().as_ref() {
            ctx.request_repaint();
        }
    }

    pub fn toast(&self, message: impl Into<String>) {
        self.send(AppEvent::Toast(Toast::info(message)));
    }

    pub fn error(&self, message: impl Into<String>) {
        self.send(AppEvent::Toast(Toast::error(message)));
    }

    /// Reports a failed operation with its full error chain.
    pub fn report(&self, context: &str, error: &anyhow::Error) {
        log::warn!("{context}: {error:#}");
        self.error(format!("{context}: {error}"));
    }

    pub fn attach_context(&self, ctx: &egui::Context) {
        *self.ctx.lock() = Some(ctx.clone());
    }
}

pub struct EventBus {
    pub sender: EventSender,
    pub receiver: Receiver<AppEvent>,
}

impl EventBus {
    pub fn new() -> Self {
        let (tx, rx) = channel();
        Self {
            sender: EventSender {
                tx,
                ctx: Arc::new(parking_lot::Mutex::new(None)),
            },
            receiver: rx,
        }
    }

    /// Drains everything queued since the previous frame.
    pub fn drain(&self) -> Vec<AppEvent> {
        self.receiver.try_iter().collect()
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}
