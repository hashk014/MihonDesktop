//! Chapter downloads.
//!
//! Mirrors Mihon's download manager: a persistent queue, a bounded number of
//! concurrent workers, pause/resume, and chapters stored as CBZ archives so
//! they stay readable outside the app.

use std::collections::{HashMap, VecDeque};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result, bail};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::db::{Db, DownloadIndex};
use crate::event::{AppEvent, EventSender};
use crate::images::{DiskCache, ImageKind, fetch_first};
use crate::model::{Chapter, Id, Manga};
use crate::net::HttpClient;
use crate::source::{SChapter, SManga, SourceManager, local};

#[derive(Debug, Clone, PartialEq)]
pub enum DownloadState {
    Queued,
    Running { done: usize, total: usize },
    Finished,
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct QueuedDownload {
    pub manga_id: Id,
    pub chapter_id: Id,
    pub manga_title: String,
    pub chapter_name: String,
}

/// Persisted map of what has been downloaded and where it lives.
#[derive(Debug, Default, Serialize, Deserialize)]
struct StoredIndex {
    entries: Vec<StoredEntry>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredEntry {
    manga_id: Id,
    chapter_id: Id,
    path: PathBuf,
}

struct Inner {
    /// Jobs waiting for a worker slot.
    queue: VecDeque<QueuedDownload>,
    /// Jobs a worker has taken. Kept separate so the same chapter can never be
    /// picked up twice, while still showing up in the queue screen.
    active: Vec<QueuedDownload>,
    states: HashMap<Id, DownloadState>,
    /// chapter id -> (manga id, location on disk)
    paths: HashMap<Id, (Id, PathBuf)>,
}

pub struct DownloadManager {
    inner: Mutex<Inner>,
    notify: Notify,
    paused: AtomicBool,
    index: DownloadIndex,
    root: PathBuf,
    index_path: PathBuf,
    /// Kept as a field so the setting can change without restarting workers.
    concurrency: Mutex<usize>,
    save_as_cbz: AtomicBool,
}

impl DownloadManager {
    pub fn new(root: PathBuf, concurrency: usize, save_as_cbz: bool) -> Self {
        let index_path = root.join("index.json");
        let manager = Self {
            inner: Mutex::new(Inner {
                queue: VecDeque::new(),
                active: Vec::new(),
                states: HashMap::new(),
                paths: HashMap::new(),
            }),
            notify: Notify::new(),
            paused: AtomicBool::new(false),
            index: DownloadIndex::default(),
            root,
            index_path,
            concurrency: Mutex::new(concurrency.clamp(1, 8)),
            save_as_cbz: AtomicBool::new(save_as_cbz),
        };
        manager.load_index();
        manager
    }

    pub fn index(&self) -> DownloadIndex {
        self.index.clone()
    }

    pub fn set_concurrency(&self, value: usize) {
        *self.concurrency.lock() = value.clamp(1, 8);
        self.notify.notify_waiters();
    }

    pub fn set_save_as_cbz(&self, value: bool) {
        self.save_as_cbz.store(value, Ordering::Relaxed);
    }

    // -- persisted index ----------------------------------------------------

    fn load_index(&self) {
        let stored: StoredIndex = std::fs::read_to_string(&self.index_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();

        let mut paths = HashMap::new();
        let mut live = std::collections::HashSet::new();
        for entry in stored.entries {
            // Drop rows whose files were deleted behind our back.
            if entry.path.exists() {
                live.insert((entry.manga_id, entry.chapter_id));
                paths.insert(entry.chapter_id, (entry.manga_id, entry.path));
            }
        }
        self.index.replace_all(live);
        self.inner.lock().paths = paths;
    }

    fn save_index(&self) {
        let entries: Vec<StoredEntry> = {
            let inner = self.inner.lock();
            inner
                .paths
                .iter()
                .map(|(chapter_id, (manga_id, path))| StoredEntry {
                    manga_id: *manga_id,
                    chapter_id: *chapter_id,
                    path: path.clone(),
                })
                .collect()
        };

        let stored = StoredIndex { entries };
        if let Some(parent) = self.index_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        match serde_json::to_string_pretty(&stored) {
            Ok(text) => {
                if let Err(err) = std::fs::write(&self.index_path, text) {
                    log::warn!("could not persist the download index: {err}");
                }
            }
            Err(err) => log::warn!("could not encode the download index: {err}"),
        }
    }

    // -- queue --------------------------------------------------------------

    pub fn enqueue(&self, jobs: Vec<QueuedDownload>) -> usize {
        let mut added = 0;
        {
            let mut inner = self.inner.lock();
            for job in jobs {
                let already_queued = inner.queue.iter().any(|q| q.chapter_id == job.chapter_id)
                    || inner.active.iter().any(|q| q.chapter_id == job.chapter_id);
                let already_done = inner.paths.contains_key(&job.chapter_id);
                if already_queued || already_done {
                    continue;
                }
                inner.states.insert(job.chapter_id, DownloadState::Queued);
                inner.queue.push_back(job);
                added += 1;
            }
        }
        if added > 0 {
            self.notify.notify_waiters();
        }
        added
    }

    pub fn cancel(&self, chapter_id: Id) {
        let mut inner = self.inner.lock();
        inner.queue.retain(|job| job.chapter_id != chapter_id);
        inner.states.remove(&chapter_id);
    }

    pub fn clear_queue(&self) {
        let mut inner = self.inner.lock();
        for job in inner.queue.drain(..).collect::<Vec<_>>() {
            inner.states.remove(&job.chapter_id);
        }
    }

    pub fn move_to_top(&self, chapter_id: Id) {
        let mut inner = self.inner.lock();
        if let Some(pos) = inner.queue.iter().position(|j| j.chapter_id == chapter_id)
            && let Some(job) = inner.queue.remove(pos)
        {
            inner.queue.push_front(job);
        }
    }

    pub fn move_to_bottom(&self, chapter_id: Id) {
        let mut inner = self.inner.lock();
        if let Some(pos) = inner.queue.iter().position(|j| j.chapter_id == chapter_id)
            && let Some(job) = inner.queue.remove(pos)
        {
            inner.queue.push_back(job);
        }
    }

    /// Moves the next pending job into the active set, if a slot is free.
    ///
    /// Taking the job out of the queue is what stops two workers from picking
    /// up the same chapter and racing over the same output file.
    fn take_job(&self, limit: usize) -> Option<QueuedDownload> {
        let mut inner = self.inner.lock();
        if inner.active.len() >= limit {
            return None;
        }
        let job = inner.queue.pop_front()?;
        inner.active.push(job.clone());
        Some(job)
    }

    /// Running jobs first, then the pending ones in order.
    pub fn queue_snapshot(&self) -> Vec<(QueuedDownload, DownloadState)> {
        let inner = self.inner.lock();
        inner
            .active
            .iter()
            .chain(inner.queue.iter())
            .map(|job| {
                let state = inner
                    .states
                    .get(&job.chapter_id)
                    .cloned()
                    .unwrap_or(DownloadState::Queued);
                (job.clone(), state)
            })
            .collect()
    }

    pub fn queue_len(&self) -> usize {
        let inner = self.inner.lock();
        inner.queue.len() + inner.active.len()
    }

    pub fn state_of(&self, chapter_id: Id) -> Option<DownloadState> {
        self.inner.lock().states.get(&chapter_id).cloned()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn set_paused(&self, paused: bool) {
        self.paused.store(paused, Ordering::Relaxed);
        if !paused {
            self.notify.notify_waiters();
        }
    }

    // -- stored chapters ----------------------------------------------------

    pub fn path_of(&self, chapter_id: Id) -> Option<PathBuf> {
        self.inner
            .lock()
            .paths
            .get(&chapter_id)
            .map(|(_, path)| path.clone())
    }

    pub fn is_downloaded(&self, manga_id: Id, chapter_id: Id) -> bool {
        self.index.contains(manga_id, chapter_id)
    }

    /// Builds the page list of a downloaded chapter, without touching the network.
    pub fn local_pages(&self, chapter_id: Id) -> Option<Vec<crate::source::Page>> {
        let path = self.path_of(chapter_id)?;
        if path.is_dir() {
            let mut files: Vec<PathBuf> = std::fs::read_dir(&path)
                .ok()?
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension()
                        .and_then(|e| e.to_str())
                        .map(|e| {
                            matches!(
                                e.to_lowercase().as_str(),
                                "jpg" | "jpeg" | "png" | "webp" | "gif" | "bmp"
                            )
                        })
                        .unwrap_or(false)
                })
                .collect();
            files.sort_by(|a, b| {
                local::natural_cmp(
                    a.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
                    b.file_name().and_then(|n| n.to_str()).unwrap_or_default(),
                )
            });
            return Some(
                files
                    .into_iter()
                    .enumerate()
                    .map(|(index, file)| crate::source::Page {
                        index,
                        image_url: format!("{}{}", local::FILE_SCHEME, file.display()),
                        fallbacks: Vec::new(),
                        headers: Vec::new(),
                    })
                    .collect(),
            );
        }

        let entries = local::archive_entries(&path).ok()?;
        Some(
            entries
                .into_iter()
                .enumerate()
                .map(|(index, entry)| crate::source::Page {
                    index,
                    image_url: format!("{}{}|{entry}", local::CBZ_SCHEME, path.display()),
                    fallbacks: Vec::new(),
                    headers: Vec::new(),
                })
                .collect(),
        )
    }

    pub fn delete_chapter(&self, manga_id: Id, chapter_id: Id) {
        let removed = self.inner.lock().paths.remove(&chapter_id);
        if let Some((_, path)) = removed {
            let result = if path.is_dir() {
                std::fs::remove_dir_all(&path)
            } else {
                std::fs::remove_file(&path)
            };
            if let Err(err) = result {
                log::warn!("could not delete {}: {err}", path.display());
            }
        }
        self.index.remove(manga_id, chapter_id);
        self.inner.lock().states.remove(&chapter_id);
        self.save_index();
    }

    pub fn delete_manga(&self, manga_id: Id) {
        for chapter_id in self.index.chapters_of(manga_id) {
            self.delete_chapter(manga_id, chapter_id);
        }
    }

    fn record(&self, manga_id: Id, chapter_id: Id, path: PathBuf) {
        self.inner.lock().paths.insert(chapter_id, (manga_id, path));
        self.index.insert(manga_id, chapter_id);
        self.save_index();
    }

    /// Total bytes occupied by downloaded chapters.
    pub fn storage_used(&self) -> u64 {
        self.inner
            .lock()
            .paths
            .values()
            .map(|(_, path)| match std::fs::metadata(path) {
                Ok(meta) if meta.is_file() => meta.len(),
                Ok(_) => directory_size(path),
                Err(_) => 0,
            })
            .sum()
    }
}

fn directory_size(dir: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => directory_size(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

// ---------------------------------------------------------------------------
// Worker
// ---------------------------------------------------------------------------

pub struct DownloadDeps {
    pub http: Arc<HttpClient>,
    pub sources: Arc<SourceManager>,
    pub db: Arc<Db>,
    pub cache: Arc<DiskCache>,
    pub events: EventSender,
}

/// Long-lived task pulling jobs off the queue. One instance is spawned at
/// startup and lives for the whole session.
pub async fn run_worker(manager: Arc<DownloadManager>, deps: Arc<DownloadDeps>) {
    loop {
        if manager.is_paused() {
            manager.notify.notified().await;
            continue;
        }

        let limit = *manager.concurrency.lock();
        let job = manager.take_job(limit);

        let Some(job) = job else {
            // Nothing to do (or at capacity): sleep until something changes.
            tokio::select! {
                _ = manager.notify.notified() => {}
                _ = tokio::time::sleep(std::time::Duration::from_millis(400)) => {}
            }
            continue;
        };

        let manager2 = manager.clone();
        let deps2 = deps.clone();
        tokio::spawn(async move {
            let chapter_id = job.chapter_id;
            let manga_id = job.manga_id;

            let result = download_chapter(&manager2, &deps2, &job).await;

            manager2
                .inner
                .lock()
                .active
                .retain(|q| q.chapter_id != chapter_id);

            match result {
                Ok(path) => {
                    manager2.record(manga_id, chapter_id, path);
                    manager2
                        .inner
                        .lock()
                        .states
                        .insert(chapter_id, DownloadState::Finished);
                    deps2.events.send(AppEvent::DownloadProgress {
                        chapter_id,
                        state: DownloadState::Finished,
                    });
                }
                Err(err) => {
                    let message = format!("{err:#}");
                    log::warn!("download of chapter {chapter_id} failed: {message}");
                    manager2
                        .inner
                        .lock()
                        .states
                        .insert(chapter_id, DownloadState::Failed(message.clone()));
                    deps2.events.send(AppEvent::DownloadProgress {
                        chapter_id,
                        state: DownloadState::Failed(message),
                    });
                }
            }

            deps2.events.send(AppEvent::DownloadQueueChanged);
            manager2.notify.notify_waiters();
        });
    }
}

async fn download_chapter(
    manager: &DownloadManager,
    deps: &DownloadDeps,
    job: &QueuedDownload,
) -> Result<PathBuf> {
    let manga = deps
        .db
        .get_manga(job.manga_id)
        .context("the manga is no longer in the database")?;
    let chapter = deps
        .db
        .get_chapter(job.chapter_id)
        .context("the chapter is no longer in the database")?;
    let source = deps
        .sources
        .get(manga.source)
        .with_context(|| format!("source {} is not installed", manga.source))?;

    let smanga = to_smanga(&manga);
    let schapter = to_schapter(&chapter);

    let mut pages = source
        .pages(&smanga, &schapter)
        .await
        .context("fetching the page list")?;
    if pages.is_empty() {
        bail!("the source returned no pages");
    }

    let total = pages.len();
    manager
        .inner
        .lock()
        .states
        .insert(job.chapter_id, DownloadState::Running { done: 0, total });
    deps.events.send(AppEvent::DownloadProgress {
        chapter_id: job.chapter_id,
        state: DownloadState::Running { done: 0, total },
    });

    let dir = manager
        .root
        .join(sanitise(source.name()))
        .join(sanitise(&manga.title));
    std::fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;

    let mut downloaded: Vec<(String, Vec<u8>)> = Vec::with_capacity(total);
    let mut refreshed = false;

    for index in 0..total {
        let bytes = match fetch_with_retry(deps, &pages[index], 3).await {
            Ok(bytes) => bytes,
            Err(err) if !refreshed => {
                // Image hosts hand out short-lived urls (MangaDex's at-home
                // nodes rotate), so ask the source for a fresh list once
                // before giving up on the chapter.
                refreshed = true;
                log::info!(
                    "page {} of {total} failed ({err:#}); refreshing the page list",
                    index + 1
                );
                let fresh = source
                    .pages(&smanga, &schapter)
                    .await
                    .context("refreshing the page list")?;
                if fresh.len() != total {
                    bail!("the page list changed while downloading");
                }
                pages = fresh;
                fetch_with_retry(deps, &pages[index], 3)
                    .await
                    .with_context(|| format!("downloading page {} of {total}", index + 1))?
            }
            Err(err) => {
                return Err(err)
                    .with_context(|| format!("downloading page {} of {total}", index + 1));
            }
        };
        let extension = image::guess_format(&bytes)
            .ok()
            .and_then(|format| format.extensions_str().first().copied())
            .unwrap_or("jpg");
        // Name files from the page's own index so the archive stays in reading
        // order even if a source ever returns pages out of sequence.
        downloaded.push((format!("{:03}.{extension}", pages[index].index + 1), bytes));

        let done = index + 1;
        manager
            .inner
            .lock()
            .states
            .insert(job.chapter_id, DownloadState::Running { done, total });
        deps.events.send(AppEvent::DownloadProgress {
            chapter_id: job.chapter_id,
            state: DownloadState::Running { done, total },
        });
    }

    let base_name = sanitise(&chapter.name);
    let as_cbz = manager.save_as_cbz.load(Ordering::Relaxed);

    // Writing is blocking I/O; keep it off the async runtime's threads.
    let target = tokio::task::spawn_blocking(move || -> Result<PathBuf> {
        if as_cbz {
            let path = dir.join(format!("{base_name}.cbz"));
            write_cbz(&path, &downloaded)?;
            Ok(path)
        } else {
            let path = dir.join(&base_name);
            std::fs::create_dir_all(&path)?;
            for (name, bytes) in &downloaded {
                std::fs::write(path.join(name), bytes)?;
            }
            Ok(path)
        }
    })
    .await
    .context("the archive writer panicked")??;

    Ok(target)
}

async fn fetch_with_retry(
    deps: &DownloadDeps,
    page: &crate::source::Page,
    attempts: usize,
) -> Result<Vec<u8>> {
    let candidates = page.candidates();
    let mut last: Option<anyhow::Error> = None;
    for attempt in 0..attempts {
        match fetch_first(
            &deps.http,
            &deps.cache,
            ImageKind::Page,
            &candidates,
            &page.headers,
        )
        .await
        {
            Ok(bytes) => return Ok(bytes),
            Err(err) => {
                last = Some(err);
                // Back off a little before trying again.
                tokio::time::sleep(std::time::Duration::from_millis(400 * (attempt as u64 + 1)))
                    .await;
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("the image could not be fetched")))
}

fn write_cbz(path: &Path, entries: &[(String, Vec<u8>)]) -> Result<()> {
    // Write to a temporary file first so an interrupted download never leaves a
    // half-written archive that would look like a complete one.
    let temp = path.with_extension("cbz.part");
    {
        let file =
            std::fs::File::create(&temp).with_context(|| format!("creating {}", temp.display()))?;
        let mut zip = zip::ZipWriter::new(file);
        // Images are already compressed; storing them keeps writes fast.
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        for (name, bytes) in entries {
            zip.start_file(name.as_str(), options)?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
    }

    if !temp.exists() {
        bail!(
            "the temporary archive {} vanished before it could be moved into place",
            temp.display()
        );
    }
    std::fs::rename(&temp, path).with_context(|| {
        format!(
            "moving {} to {} (temp exists: {}, parent exists: {})",
            temp.display(),
            path.display(),
            temp.exists(),
            path.parent().map(|p| p.exists()).unwrap_or(false)
        )
    })?;
    Ok(())
}

/// Strips characters Windows refuses in file names.
pub fn sanitise(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            c if (c as u32) < 0x20 => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim().trim_end_matches('.').trim();
    if trimmed.is_empty() {
        "untitled".to_string()
    } else {
        // Leave room for the extension and the parent path.
        trimmed.chars().take(120).collect()
    }
}

pub fn to_smanga(manga: &Manga) -> SManga {
    SManga {
        url: manga.url.clone(),
        title: manga.title.clone(),
        artist: manga.artist.clone(),
        author: manga.author.clone(),
        description: manga.description.clone(),
        genre: manga.genre.clone(),
        status: manga.status,
        thumbnail_url: manga.thumbnail_url.clone(),
        update_strategy: manga.update_strategy,
        initialized: manga.initialized,
    }
}

pub fn to_schapter(chapter: &Chapter) -> SChapter {
    SChapter {
        url: chapter.url.clone(),
        name: chapter.name.clone(),
        date_upload: chapter.date_upload,
        chapter_number: chapter.chapter_number,
        scanlator: chapter.scanlator.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_made_safe() {
        assert_eq!(sanitise("Ch. 1: A/B?"), "Ch. 1_ A_B_");
        assert_eq!(sanitise("   "), "untitled");
        assert_eq!(sanitise("trailing."), "trailing");
    }

    #[test]
    fn cbz_handles_dotted_chapter_names() {
        // "Vol.1 Ch.1" has dots in the stem, which trips naive extension logic.
        let dir = std::env::temp_dir().join(format!("mihon-dots-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("Vol.1 Ch.1.cbz");

        write_cbz(&path, &[("001.jpg".to_string(), b"x".to_vec())]).unwrap();
        assert!(path.exists(), "{} was not created", path.display());
        assert_eq!(local::archive_entries(&path).unwrap(), vec!["001.jpg"]);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cbz_round_trips() {
        let dir = std::env::temp_dir().join(format!("mihon-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("chapter.cbz");

        let entries = vec![
            ("001.jpg".to_string(), b"first".to_vec()),
            ("002.jpg".to_string(), b"second".to_vec()),
        ];
        write_cbz(&path, &entries).unwrap();

        let names = local::archive_entries(&path).unwrap();
        assert_eq!(names, vec!["001.jpg", "002.jpg"]);

        let page =
            local::read_local_page(&format!("{}{}|002.jpg", local::CBZ_SCHEME, path.display()))
                .unwrap();
        assert_eq!(page, b"second");

        // The temporary file must not survive a successful write.
        assert!(!path.with_extension("cbz.part").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Downloads a real chapter end to end: source -> pages -> CBZ -> index,
    /// then reads the archive back the way the reader would.
    #[test]
    #[ignore]
    fn live_download_round_trip() {
        use crate::event::EventBus;
        use crate::model::Manga;
        use crate::source::Source;
        use crate::source::mangadex::MangaDex;

        let root = std::env::temp_dir().join(format!("mihon-dl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();

        let runtime = tokio::runtime::Runtime::new().unwrap();
        let http = Arc::new(HttpClient::new().unwrap());
        let sources = Arc::new(SourceManager::new());
        let source = Arc::new(MangaDex::new(http.clone(), "en"));
        let source_id = source.id();
        sources.register(source.clone());

        let db = Arc::new(Db::open(&root.join("library.redb")).unwrap());
        let cache = Arc::new(DiskCache::new(root.join("covers"), root.join("pages")));
        let bus = EventBus::new();

        // Find a title that actually has readable chapters.
        let (manga_row, chapter_row) = runtime.block_on(async {
            let popular = source.popular(1).await.expect("popular failed");
            for candidate in popular.mangas.iter().take(6) {
                let chapters = source.chapters(candidate).await.unwrap_or_default();
                let Some(last) = chapters.last() else {
                    continue;
                };

                let mut manga =
                    Manga::new(source_id, candidate.url.clone(), candidate.title.clone());
                manga.favorite = true;
                let manga = db.get_or_insert_manga(&manga).unwrap();
                let stored = crate::core::to_chapters(manga.id, &manga.title, vec![last.clone()]);
                db.sync_chapters(manga.id, stored).unwrap();
                let chapter = db.chapters_of(manga.id).first().cloned().unwrap();
                return (manga, chapter);
            }
            panic!("no downloadable chapter found");
        });

        let manager = Arc::new(DownloadManager::new(root.join("downloads"), 2, true));
        let deps = Arc::new(DownloadDeps {
            http,
            sources,
            db: db.clone(),
            cache,
            events: bus.sender.clone(),
        });

        let worker = manager.clone();
        let worker_deps = deps.clone();
        runtime.spawn(async move { run_worker(worker, worker_deps).await });

        assert_eq!(
            manager.enqueue(vec![QueuedDownload {
                manga_id: manga_row.id,
                chapter_id: chapter_row.id,
                manga_title: manga_row.title.clone(),
                chapter_name: chapter_row.name.clone(),
            }]),
            1
        );

        // Wait for the worker, with a ceiling so a hang fails loudly.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(180);
        let outcome = loop {
            match manager.state_of(chapter_row.id) {
                Some(DownloadState::Finished) => break DownloadState::Finished,
                Some(DownloadState::Failed(err)) => break DownloadState::Failed(err),
                _ if std::time::Instant::now() > deadline => panic!("download timed out"),
                _ => std::thread::sleep(std::time::Duration::from_millis(250)),
            }
        };
        assert_eq!(outcome, DownloadState::Finished, "download did not succeed");

        // The archive must exist, be indexed, and be readable page by page.
        let path = manager.path_of(chapter_row.id).expect("no path recorded");
        assert!(path.exists(), "{} is missing", path.display());
        assert_eq!(path.extension().and_then(|e| e.to_str()), Some("cbz"));
        assert!(manager.is_downloaded(manga_row.id, chapter_row.id));

        // The Downloads screen reports this figure; it must not stay at zero.
        let used = manager.storage_used();
        println!("storage_used = {used} bytes");
        assert!(used > 0, "a finished download must count towards storage");

        let pages = manager.local_pages(chapter_row.id).expect("no local pages");
        assert!(!pages.is_empty());
        let bytes = local::read_local_page(&pages[0].image_url).unwrap();
        let decoded = image::load_from_memory(&bytes).expect("stored page is not an image");
        println!(
            "downloaded {} pages to {} ({}x{})",
            pages.len(),
            path.display(),
            decoded.width(),
            decoded.height()
        );

        // Deleting must remove both the file and the index entry.
        manager.delete_chapter(manga_row.id, chapter_row.id);
        assert!(!path.exists());
        assert!(!manager.is_downloaded(manga_row.id, chapter_row.id));

        let _ = std::fs::remove_dir_all(&root);
    }

    fn job(chapter_id: Id) -> QueuedDownload {
        QueuedDownload {
            manga_id: 1,
            chapter_id,
            manga_title: "M".into(),
            chapter_name: format!("Ch.{chapter_id}"),
        }
    }

    /// Regression: two workers must never receive the same job, or they race
    /// over the same output file and one of them loses its archive.
    #[test]
    fn a_job_is_only_handed_out_once() {
        let dir = std::env::temp_dir().join(format!("mihon-take-{}", std::process::id()));
        let manager = DownloadManager::new(dir.clone(), 4, true);
        manager.enqueue(vec![job(1), job(2)]);

        let first = manager.take_job(4).expect("first job");
        let second = manager.take_job(4).expect("second job");
        assert_ne!(first.chapter_id, second.chapter_id);
        // Both are in flight, nothing left to hand out.
        assert!(manager.take_job(4).is_none());
        // ...but they are still visible to the user.
        assert_eq!(manager.queue_len(), 2);
        assert_eq!(manager.queue_snapshot().len(), 2);

        // A chapter already in flight must not be re-queued.
        assert_eq!(manager.enqueue(vec![job(1)]), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn concurrency_limit_is_respected() {
        let dir = std::env::temp_dir().join(format!("mihon-limit-{}", std::process::id()));
        let manager = DownloadManager::new(dir.clone(), 1, true);
        manager.enqueue(vec![job(1), job(2), job(3)]);

        assert!(manager.take_job(1).is_some());
        assert!(manager.take_job(1).is_none(), "the limit must hold");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Probe against a real profile: `MIHON_PROBE_DOWNLOADS=<downloads dir>`.
    #[test]
    #[ignore]
    fn probe_existing_downloads() {
        let Ok(dir) = std::env::var("MIHON_PROBE_DOWNLOADS") else {
            println!("set MIHON_PROBE_DOWNLOADS to run this");
            return;
        };
        let root = PathBuf::from(dir);
        let manager = DownloadManager::new(root.clone(), 3, true);
        println!("root: {}", root.display());
        println!("index entries loaded: {}", manager.inner.lock().paths.len());
        println!("download index size: {}", manager.index().total());
        println!("storage_used: {} bytes", manager.storage_used());
        for (chapter_id, (manga_id, path)) in manager.inner.lock().paths.iter() {
            println!(
                "  manga={manga_id} chapter={chapter_id} exists={} {}",
                path.exists(),
                path.display()
            );
        }
    }

    #[test]
    fn queue_rejects_duplicates() {
        let dir = std::env::temp_dir().join(format!("mihon-queue-{}", std::process::id()));
        let manager = DownloadManager::new(dir.clone(), 2, true);
        let job = QueuedDownload {
            manga_id: 1,
            chapter_id: 2,
            manga_title: "M".into(),
            chapter_name: "C".into(),
        };
        assert_eq!(manager.enqueue(vec![job.clone()]), 1);
        assert_eq!(manager.enqueue(vec![job]), 0);
        assert_eq!(manager.queue_len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
