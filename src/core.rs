//! Application core: owns the services and drives every background task.
//!
//! The UI never awaits anything. It calls a method here, which spawns work on
//! the Tokio runtime and reports back through the event channel.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::{Context, Result};

use crate::db::Db;
use crate::download::{DownloadDeps, DownloadManager, QueuedDownload, to_schapter, to_smanga};
use crate::event::{AppEvent, EventBus, EventSender};
use crate::images::{DiskCache, ImageKind, decode, fetch_first};
use crate::model::*;
use crate::net::HttpClient;
use crate::prefs::{AppPaths, Preferences};
use crate::source::{FilterList, SourceManager, ext, local};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseMode {
    Popular,
    Latest,
    Search,
}

pub struct Core {
    pub paths: AppPaths,
    pub db: Arc<Db>,
    pub http: Arc<HttpClient>,
    pub sources: Arc<SourceManager>,
    pub downloads: Arc<DownloadManager>,
    pub cache: Arc<DiskCache>,
    pub events: EventSender,
    /// Guards against two library updates running at once.
    updating: Arc<AtomicBool>,
    rt: tokio::runtime::Runtime,
}

impl Core {
    pub fn new(paths: AppPaths, prefs: &Preferences, bus: &EventBus) -> Result<Self> {
        paths.ensure().context("creating the data directories")?;

        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(4)
            .enable_all()
            .build()
            .context("starting the async runtime")?;

        let db = Arc::new(Db::open(&paths.database)?);
        let http = Arc::new(HttpClient::new()?);
        let sources = Arc::new(SourceManager::with_builtins(http.clone(), &paths));
        let cache = Arc::new(DiskCache::new(paths.covers.clone(), paths.pages.clone()));

        let downloads = Arc::new(DownloadManager::new(
            prefs.downloads_dir(&paths),
            prefs.downloads.concurrent_downloads as usize,
            prefs.downloads.save_as_cbz,
        ));

        let core = Self {
            paths,
            db,
            http,
            sources,
            downloads,
            cache,
            events: bus.sender.clone(),
            updating: Arc::new(AtomicBool::new(false)),
            rt,
        };

        core.reload_extensions();
        core.start_download_worker();
        Ok(core)
    }

    fn start_download_worker(&self) {
        let deps = Arc::new(DownloadDeps {
            http: self.http.clone(),
            sources: self.sources.clone(),
            db: self.db.clone(),
            cache: self.cache.clone(),
            events: self.events.clone(),
        });
        let manager = self.downloads.clone();
        self.rt
            .spawn(async move { crate::download::run_worker(manager, deps).await });
    }

    /// Rebuilds the scripted sources from `extensions/`.
    pub fn reload_extensions(&self) {
        // Drop every scripted source first: a manifest that was deleted from
        // disk can no longer identify itself, so it has to go this way.
        self.sources.remove_scripted();

        let mut loaded = 0;
        for extension in ext::load_installed(&self.paths.extensions) {
            self.sources.register(Arc::new(ext::ScriptedSource::new(
                extension.manifest,
                self.http.clone(),
            )));
            loaded += 1;
        }
        log::info!("{loaded} scripted extension(s) loaded");
        self.events.send(AppEvent::ExtensionsChanged);
    }

    /// Rebuilds the local source so newly added folders are picked up.
    pub fn refresh_local_source(&self, prefs: &Preferences) {
        self.sources
            .register(Arc::new(local::LocalSource::with_extra_roots(
                self.paths.local_source.clone(),
                prefs.local_source_dirs.clone(),
            )));
    }

    // -- browsing -----------------------------------------------------------

    pub fn browse(
        &self,
        source_id: Id,
        mode: BrowseMode,
        page: u32,
        query: String,
        filters: FilterList,
    ) {
        let Some(source) = self.sources.get(source_id) else {
            self.events.error("that source is not installed");
            return;
        };
        let events = self.events.clone();

        self.rt.spawn(async move {
            let result = match mode {
                BrowseMode::Popular => source.popular(page).await,
                BrowseMode::Latest => source.latest(page).await,
                BrowseMode::Search => source.search(page, &query, &filters).await,
            };
            events.send(AppEvent::BrowseLoaded {
                source: source_id,
                page,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
    }

    /// Runs the same query across several sources at once.
    pub fn global_search(&self, source_ids: Vec<Id>, query: String) {
        for source_id in source_ids {
            let Some(source) = self.sources.get(source_id) else {
                continue;
            };
            let events = self.events.clone();
            let query = query.clone();
            let filters = source.filters();

            self.rt.spawn(async move {
                let result = source.search(1, &query, &filters).await;
                events.send(AppEvent::GlobalSearchLoaded {
                    source: source_id,
                    result: result.map_err(|err| format!("{err:#}")),
                });
            });
        }
    }

    // -- manga --------------------------------------------------------------

    /// Stores a source result locally (without favouriting it) and returns its row.
    pub fn intern_manga(&self, source_id: Id, smanga: &crate::source::SManga) -> Result<Manga> {
        let mut manga = Manga::new(source_id, smanga.url.clone(), smanga.title.clone());
        manga.thumbnail_url = smanga.thumbnail_url.clone();
        manga.author = smanga.author.clone();
        manga.artist = smanga.artist.clone();
        manga.description = smanga.description.clone();
        manga.genre = smanga.genre.clone();
        manga.status = smanga.status;
        self.db.get_or_insert_manga(&manga)
    }

    /// Fetches details and the chapter list, then persists both.
    pub fn refresh_manga(&self, manga_id: Id, fetch_details: bool) {
        let Some(manga) = self.db.get_manga(manga_id) else {
            return;
        };
        let Some(source) = self.sources.get(manga.source) else {
            self.events
                .error(format!("source {} is not installed", manga.source));
            self.events.send(AppEvent::ChaptersLoaded {
                manga_id,
                new_chapters: 0,
                result: Err("source not installed".into()),
            });
            return;
        };

        let db = self.db.clone();
        let events = self.events.clone();

        self.rt.spawn(async move {
            let smanga = to_smanga(&manga);

            if fetch_details {
                match source.details(&smanga).await {
                    Ok(details) => {
                        let outcome = db.update_manga(manga_id, |m| {
                            m.title = details.title.clone();
                            m.author = details.author.clone();
                            m.artist = details.artist.clone();
                            m.description = details.description.clone();
                            m.genre = details.genre.clone();
                            m.status = details.status;
                            if details.thumbnail_url.is_some() {
                                m.thumbnail_url = details.thumbnail_url.clone();
                            }
                            m.update_strategy = details.update_strategy;
                            m.initialized = true;
                        });
                        events.send(AppEvent::DetailsLoaded {
                            manga_id,
                            result: outcome.map(|_| ()).map_err(|err| format!("{err:#}")),
                        });
                    }
                    Err(err) => events.send(AppEvent::DetailsLoaded {
                        manga_id,
                        result: Err(format!("{err:#}")),
                    }),
                }
            }

            match source.chapters(&smanga).await {
                Ok(list) => {
                    let chapters = to_chapters(manga_id, &manga.title, list);
                    match db.sync_chapters(manga_id, chapters) {
                        Ok(added) => {
                            let _ = db.update_manga(manga_id, |m| {
                                m.last_update = now_millis();
                            });
                            events.send(AppEvent::ChaptersLoaded {
                                manga_id,
                                new_chapters: added.len(),
                                result: Ok(()),
                            });
                            events.send(AppEvent::LibraryChanged);
                        }
                        Err(err) => events.send(AppEvent::ChaptersLoaded {
                            manga_id,
                            new_chapters: 0,
                            result: Err(format!("{err:#}")),
                        }),
                    }
                }
                Err(err) => events.send(AppEvent::ChaptersLoaded {
                    manga_id,
                    new_chapters: 0,
                    result: Err(format!("{err:#}")),
                }),
            }
        });
    }

    /// Resolves the page list of a chapter, preferring a downloaded copy.
    pub fn fetch_pages(&self, manga_id: Id, chapter_id: Id) {
        if let Some(pages) = self.downloads.local_pages(chapter_id) {
            self.events.send(AppEvent::PagesLoaded {
                chapter_id,
                result: Ok(pages),
            });
            return;
        }

        let (Some(manga), Some(chapter)) =
            (self.db.get_manga(manga_id), self.db.get_chapter(chapter_id))
        else {
            self.events.send(AppEvent::PagesLoaded {
                chapter_id,
                result: Err("the chapter is no longer available".into()),
            });
            return;
        };
        let Some(source) = self.sources.get(manga.source) else {
            self.events.send(AppEvent::PagesLoaded {
                chapter_id,
                result: Err(format!("source {} is not installed", manga.source)),
            });
            return;
        };

        let events = self.events.clone();
        let db = self.db.clone();

        self.rt.spawn(async move {
            let result = source
                .pages(&to_smanga(&manga), &to_schapter(&chapter))
                .await;
            if let Ok(pages) = &result {
                // Cache the count so the library can show progress without opening.
                let count = pages.len() as i64;
                let _ = db.update_chapter(chapter_id, |c| c.page_count = count);
            }
            events.send(AppEvent::PagesLoaded {
                chapter_id,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
    }

    // -- images -------------------------------------------------------------

    /// Loads an image. `candidates` are tried in order; the first doubles as
    /// the cache key, so mirrors never fragment the cache.
    pub fn load_image(
        &self,
        kind: ImageKind,
        candidates: Vec<String>,
        headers: Vec<(String, String)>,
        max_width: u32,
        crop_borders: bool,
    ) {
        let http = self.http.clone();
        let cache = self.cache.clone();
        let events = self.events.clone();
        let Some(key) = candidates.first().cloned() else {
            return;
        };

        self.rt.spawn(async move {
            let bytes = fetch_first(&http, &cache, kind, &candidates, &headers).await;

            let result = match bytes {
                Ok(bytes) => {
                    // Decoding is CPU-bound; keep it off the async workers.
                    tokio::task::spawn_blocking(move || decode(&bytes, max_width, crop_borders))
                        .await
                        .map_err(|err| anyhow::anyhow!("decode task failed: {err}"))
                        .and_then(|inner| inner)
                }
                Err(err) => Err(err),
            };

            events.send(AppEvent::ImageLoaded {
                kind,
                key,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
    }

    // -- downloads ----------------------------------------------------------

    pub fn queue_downloads(&self, manga_id: Id, chapter_ids: &[Id]) {
        let Some(manga) = self.db.get_manga(manga_id) else {
            return;
        };

        let mut chapters: Vec<Chapter> = chapter_ids
            .iter()
            .filter_map(|id| self.db.get_chapter(*id))
            .collect();
        // Download in reading order: the oldest unread chapter is the one you
        // want on disk first, whatever order the source lists them in.
        chapters.sort_by(reading_order);

        let jobs: Vec<QueuedDownload> = chapters
            .into_iter()
            .map(|chapter| QueuedDownload {
                manga_id,
                chapter_id: chapter.id,
                manga_title: manga.title.clone(),
                chapter_name: chapter.name,
            })
            .collect();

        let added = self.downloads.enqueue(jobs);
        if added > 0 {
            self.events
                .toast(format!("{added} chapter(s) added to the download queue"));
        }
        self.events.send(AppEvent::DownloadQueueChanged);
    }

    // -- library update -----------------------------------------------------

    /// Refreshes every eligible favourite, the way "Update library" does upstream.
    pub fn update_library(&self, prefs: &Preferences, only_category: Option<Id>) {
        if self.updating.swap(true, Ordering::SeqCst) {
            self.events.toast("a library update is already running");
            return;
        }

        let entries = self.db.library_entries(&self.downloads.index());
        let library_prefs = prefs.library.clone();
        let auto_download = prefs.library.download_new_chapters;

        let targets: Vec<Manga> = entries
            .into_iter()
            .filter(|entry| match only_category {
                Some(category) => entry.category_ids.contains(&category),
                None => {
                    library_prefs.update_categories.is_empty()
                        || entry
                            .category_ids
                            .iter()
                            .any(|c| library_prefs.update_categories.contains(c))
                }
            })
            .filter(|entry| {
                // Mirrors the "global update restrictions" preference block.
                if library_prefs.skip_completed_entries && entry.manga.status.is_finished() {
                    return false;
                }
                if library_prefs.skip_entries_with_unread && entry.unread_count > 0 {
                    return false;
                }
                if library_prefs.skip_unstarted_entries && !entry.has_started {
                    return false;
                }
                entry.manga.update_strategy == UpdateStrategy::AlwaysUpdate
            })
            .map(|entry| entry.manga)
            .collect();

        let total = targets.len();
        if total == 0 {
            self.updating.store(false, Ordering::SeqCst);
            self.events.send(AppEvent::LibraryUpdateFinished {
                new_chapters: 0,
                failed: 0,
            });
            self.events.toast("nothing to update");
            return;
        }

        let db = self.db.clone();
        let sources = self.sources.clone();
        let events = self.events.clone();
        let downloads = self.downloads.clone();
        let updating = self.updating.clone();

        self.rt.spawn(async move {
            // Three at a time keeps sources happy without dragging the update out.
            let semaphore = Arc::new(tokio::sync::Semaphore::new(3));
            let done = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let new_total = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let failed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

            let mut handles = Vec::with_capacity(total);
            for manga in targets {
                let semaphore = semaphore.clone();
                let db = db.clone();
                let sources = sources.clone();
                let events = events.clone();
                let downloads = downloads.clone();
                let done = done.clone();
                let new_total = new_total.clone();
                let failed = failed.clone();

                handles.push(tokio::spawn(async move {
                    let _permit = semaphore.acquire().await;

                    let outcome: Result<usize> = async {
                        let source = sources.get(manga.source).context("source not installed")?;
                        let list = source.chapters(&to_smanga(&manga)).await?;
                        let chapters = to_chapters(manga.id, &manga.title, list);
                        let added = db.sync_chapters(manga.id, chapters)?;
                        let _ = db.update_manga(manga.id, |m| m.last_update = now_millis());

                        if auto_download && !added.is_empty() {
                            let jobs = added
                                .iter()
                                .map(|chapter| QueuedDownload {
                                    manga_id: manga.id,
                                    chapter_id: chapter.id,
                                    manga_title: manga.title.clone(),
                                    chapter_name: chapter.name.clone(),
                                })
                                .collect();
                            downloads.enqueue(jobs);
                        }
                        Ok(added.len())
                    }
                    .await;

                    match outcome {
                        Ok(added) => {
                            new_total.fetch_add(added, Ordering::Relaxed);
                        }
                        Err(err) => {
                            log::warn!("update of {:?} failed: {err:#}", manga.title);
                            failed.fetch_add(1, Ordering::Relaxed);
                        }
                    }

                    let completed = done.fetch_add(1, Ordering::Relaxed) + 1;
                    events.send(AppEvent::LibraryUpdateProgress {
                        done: completed,
                        total,
                        current: manga.title.clone(),
                    });
                }));
            }

            for handle in handles {
                let _ = handle.await;
            }

            updating.store(false, Ordering::SeqCst);
            events.send(AppEvent::LibraryChanged);
            events.send(AppEvent::LibraryUpdateFinished {
                new_chapters: new_total.load(Ordering::Relaxed),
                failed: failed.load(Ordering::Relaxed),
            });
        });
    }

    pub fn is_updating(&self) -> bool {
        self.updating.load(Ordering::SeqCst)
    }

    // -- extensions ---------------------------------------------------------

    pub fn fetch_repo(&self, url: String) {
        let http = self.http.clone();
        let events = self.events.clone();
        self.rt.spawn(async move {
            let result = ext::fetch_repo(&http, &url).await;
            events.send(AppEvent::RepoLoaded {
                url,
                result: result.map_err(|err| format!("{err:#}")),
            });
        });
    }

    pub fn install_extension(&self, entry: ext::RepoEntry) {
        let http = self.http.clone();
        let events = self.events.clone();
        let dir = self.paths.extensions.clone();
        let name = entry.name.clone();

        self.rt.spawn(async move {
            match ext::install(&http, &dir, &entry).await {
                Ok(_) => {
                    events.toast(format!("{name} installed"));
                    events.send(AppEvent::ExtensionsChanged);
                }
                Err(err) => events.report(&format!("could not install {name}"), &err),
            }
        });
    }

    // -- misc ---------------------------------------------------------------
}

/// Converts source chapters into storable rows, filling in missing numbers.
pub fn to_chapters(
    manga_id: Id,
    manga_title: &str,
    list: Vec<crate::source::SChapter>,
) -> Vec<Chapter> {
    list.into_iter()
        .enumerate()
        .map(|(index, item)| {
            let mut chapter = Chapter::new(manga_id, item.url, item.name);
            chapter.date_upload = item.date_upload;
            chapter.scanlator = item.scanlator;
            chapter.source_order = index as i64;
            chapter.chapter_number = if item.chapter_number >= 0.0 {
                item.chapter_number
            } else {
                crate::source::recognise_chapter_number(manga_title, &chapter.name)
            };
            chapter
        })
        .collect()
}
