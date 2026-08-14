//! Persistence layer.
//!
//! Mihon uses SQLite through SQLDelight. Here the store is `redb` (a pure-Rust
//! embedded B-tree) holding JSON-encoded records, fronted by an in-memory cache
//! that every read goes through. A manga library is small enough to keep fully
//! resident, which keeps the UI thread free of I/O and makes the aggregate
//! queries the library screen needs (unread counts, latest chapter, ...) cheap.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::Arc;

use anyhow::{Context, Result};
use parking_lot::RwLock;
use redb::{Database, ReadableDatabase, ReadableTable, TableDefinition};

use crate::model::*;

const T_MANGA: TableDefinition<i64, String> = TableDefinition::new("manga");
const T_CHAPTER: TableDefinition<i64, String> = TableDefinition::new("chapter");
const T_CATEGORY: TableDefinition<i64, String> = TableDefinition::new("category");
const T_MANGA_CATS: TableDefinition<i64, String> = TableDefinition::new("manga_categories");
const T_HISTORY: TableDefinition<i64, String> = TableDefinition::new("history");
const T_TRACK: TableDefinition<i64, String> = TableDefinition::new("track");
const T_META: TableDefinition<&str, String> = TableDefinition::new("meta");

/// Which (manga, chapter) pairs exist on disk. Maintained by the download
/// manager and consulted when building library/updates rows.
#[derive(Debug, Default, Clone)]
pub struct DownloadIndex {
    inner: Arc<RwLock<HashSet<(Id, Id)>>>,
}

impl DownloadIndex {
    pub fn contains(&self, manga_id: Id, chapter_id: Id) -> bool {
        self.inner.read().contains(&(manga_id, chapter_id))
    }

    pub fn insert(&self, manga_id: Id, chapter_id: Id) {
        self.inner.write().insert((manga_id, chapter_id));
    }

    pub fn remove(&self, manga_id: Id, chapter_id: Id) {
        self.inner.write().remove(&(manga_id, chapter_id));
    }

    pub fn count_for_manga(&self, manga_id: Id) -> i64 {
        self.inner
            .read()
            .iter()
            .filter(|(m, _)| *m == manga_id)
            .count() as i64
    }

    pub fn chapters_of(&self, manga_id: Id) -> Vec<Id> {
        self.inner
            .read()
            .iter()
            .filter(|(m, _)| *m == manga_id)
            .map(|(_, c)| *c)
            .collect()
    }

    pub fn total(&self) -> usize {
        self.inner.read().len()
    }

    pub fn replace_all(&self, entries: HashSet<(Id, Id)>) {
        *self.inner.write() = entries;
    }
}

#[derive(Default)]
struct Cache {
    manga: HashMap<Id, Manga>,
    /// manga id -> chapters, kept in source order.
    chapters: HashMap<Id, Vec<Chapter>>,
    /// chapter id -> manga id, so a chapter can be found without a scan.
    chapter_owner: HashMap<Id, Id>,
    categories: Vec<Category>,
    manga_cats: HashMap<Id, Vec<Id>>,
    /// chapter id -> history row.
    history: HashMap<Id, History>,
    tracks: HashMap<Id, Vec<Track>>,
    next_id: i64,
}

pub struct Db {
    db: Database,
    cache: RwLock<Cache>,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        let db = Database::create(path)
            .with_context(|| format!("opening database at {}", path.display()))?;

        // Make sure every table exists so that later read transactions do not fail.
        {
            let tx = db.begin_write()?;
            tx.open_table(T_MANGA)?;
            tx.open_table(T_CHAPTER)?;
            tx.open_table(T_CATEGORY)?;
            tx.open_table(T_MANGA_CATS)?;
            tx.open_table(T_HISTORY)?;
            tx.open_table(T_TRACK)?;
            tx.open_table(T_META)?;
            tx.commit()?;
        }

        let this = Self {
            db,
            cache: RwLock::new(Cache::default()),
        };
        this.load()?;
        this.ensure_default_category()?;
        Ok(this)
    }

    fn load(&self) -> Result<()> {
        let tx = self.db.begin_read()?;
        let mut cache = Cache::default();

        for row in tx.open_table(T_MANGA)?.iter()? {
            let (_, v) = row?;
            if let Ok(m) = serde_json::from_str::<Manga>(&v.value()) {
                cache.manga.insert(m.id, m);
            }
        }

        for row in tx.open_table(T_CHAPTER)?.iter()? {
            let (_, v) = row?;
            if let Ok(c) = serde_json::from_str::<Chapter>(&v.value()) {
                cache.chapter_owner.insert(c.id, c.manga_id);
                cache.chapters.entry(c.manga_id).or_default().push(c);
            }
        }
        for list in cache.chapters.values_mut() {
            list.sort_by_key(|c| c.source_order);
        }

        for row in tx.open_table(T_CATEGORY)?.iter()? {
            let (_, v) = row?;
            if let Ok(c) = serde_json::from_str::<Category>(&v.value()) {
                cache.categories.push(c);
            }
        }
        cache.categories.sort_by_key(|c| c.order);

        for row in tx.open_table(T_MANGA_CATS)?.iter()? {
            let (k, v) = row?;
            if let Ok(ids) = serde_json::from_str::<Vec<Id>>(&v.value()) {
                cache.manga_cats.insert(k.value(), ids);
            }
        }

        for row in tx.open_table(T_HISTORY)?.iter()? {
            let (_, v) = row?;
            if let Ok(h) = serde_json::from_str::<History>(&v.value()) {
                cache.history.insert(h.chapter_id, h);
            }
        }

        for row in tx.open_table(T_TRACK)?.iter()? {
            let (_, v) = row?;
            if let Ok(t) = serde_json::from_str::<Track>(&v.value()) {
                cache.tracks.entry(t.manga_id).or_default().push(t);
            }
        }

        let meta = tx.open_table(T_META)?;
        cache.next_id = meta
            .get("next_id")?
            .and_then(|v| v.value().parse::<i64>().ok())
            .unwrap_or(1);

        // Guard against a truncated write leaving the counter behind the data.
        let highest = cache
            .manga
            .keys()
            .chain(cache.chapter_owner.keys())
            .chain(cache.history.values().map(|h| &h.id))
            .copied()
            .max()
            .unwrap_or(0);
        cache.next_id = cache.next_id.max(highest + 1);

        *self.cache.write() = cache;
        Ok(())
    }

    fn ensure_default_category(&self) -> Result<()> {
        if self.cache.read().categories.iter().any(|c| c.is_system()) {
            return Ok(());
        }
        let default = Category {
            id: Category::DEFAULT_ID,
            name: "Default".into(),
            order: 0,
            flags: 0,
            hidden: false,
        };
        self.put(T_CATEGORY, default.id, &default)?;
        let mut cache = self.cache.write();
        cache.categories.insert(0, default);
        Ok(())
    }

    // -- low level ----------------------------------------------------------

    fn put<V: serde::Serialize>(
        &self,
        table: TableDefinition<i64, String>,
        key: i64,
        value: &V,
    ) -> Result<()> {
        let encoded = serde_json::to_string(value)?;
        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(table)?;
            t.insert(key, encoded)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn put_many<V: serde::Serialize>(
        &self,
        table: TableDefinition<i64, String>,
        values: &[(i64, V)],
    ) -> Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(table)?;
            for (key, value) in values {
                t.insert(*key, serde_json::to_string(value)?)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn delete(&self, table: TableDefinition<i64, String>, key: i64) -> Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(table)?;
            t.remove(key)?;
        }
        tx.commit()?;
        Ok(())
    }

    fn delete_many(&self, table: TableDefinition<i64, String>, keys: &[i64]) -> Result<()> {
        let tx = self.db.begin_write()?;
        {
            let mut t = tx.open_table(table)?;
            for key in keys {
                t.remove(*key)?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    fn next_id(&self) -> i64 {
        let mut cache = self.cache.write();
        let id = cache.next_id;
        cache.next_id += 1;
        drop(cache);
        // Persisting the counter is best-effort; `load` repairs it if it lags.
        if let Ok(tx) = self.db.begin_write() {
            if let Ok(mut t) = tx.open_table(T_META) {
                let _ = t.insert("next_id", (id + 1).to_string());
            }
            let _ = tx.commit();
        }
        id
    }

    // -- manga --------------------------------------------------------------

    pub fn get_manga(&self, id: Id) -> Option<Manga> {
        self.cache.read().manga.get(&id).cloned()
    }

    pub fn find_manga(&self, source: Id, url: &str) -> Option<Manga> {
        self.cache
            .read()
            .manga
            .values()
            .find(|m| m.source == source && m.url == url)
            .cloned()
    }

    /// Inserts the manga if `(source, url)` is unknown, otherwise returns the
    /// existing row. Mirrors `NetworkToLocalManga` upstream.
    pub fn get_or_insert_manga(&self, manga: &Manga) -> Result<Manga> {
        if let Some(existing) = self.find_manga(manga.source, &manga.url) {
            return Ok(existing);
        }
        let mut stored = manga.clone();
        stored.id = self.next_id();
        stored.last_modified_at = now_millis();
        self.put(T_MANGA, stored.id, &stored)?;
        self.cache.write().manga.insert(stored.id, stored.clone());
        Ok(stored)
    }

    pub fn upsert_manga(&self, manga: &Manga) -> Result<()> {
        let mut stored = manga.clone();
        if stored.id == 0 {
            stored.id = self.next_id();
        }
        stored.last_modified_at = now_millis();
        self.put(T_MANGA, stored.id, &stored)?;
        self.cache.write().manga.insert(stored.id, stored);
        Ok(())
    }

    /// Applies a mutation to a stored manga and persists it.
    pub fn update_manga(&self, id: Id, f: impl FnOnce(&mut Manga)) -> Result<Option<Manga>> {
        let updated = {
            let mut cache = self.cache.write();
            match cache.manga.get_mut(&id) {
                Some(m) => {
                    f(m);
                    m.last_modified_at = now_millis();
                    m.version += 1;
                    Some(m.clone())
                }
                None => None,
            }
        };
        if let Some(m) = &updated {
            self.put(T_MANGA, m.id, m)?;
        }
        Ok(updated)
    }

    pub fn all_manga(&self) -> Vec<Manga> {
        self.cache.read().manga.values().cloned().collect()
    }

    pub fn favorites(&self) -> Vec<Manga> {
        self.cache
            .read()
            .manga
            .values()
            .filter(|m| m.favorite)
            .cloned()
            .collect()
    }

    /// Removes a manga together with its chapters, history, tracks and category links.
    pub fn delete_manga(&self, id: Id) -> Result<()> {
        let chapter_ids: Vec<Id> = self
            .cache
            .read()
            .chapters
            .get(&id)
            .map(|list| list.iter().map(|c| c.id).collect())
            .unwrap_or_default();
        let history_ids: Vec<Id> = self
            .cache
            .read()
            .history
            .values()
            .filter(|h| h.manga_id == id)
            .map(|h| h.id)
            .collect();
        let track_ids: Vec<Id> = self
            .cache
            .read()
            .tracks
            .get(&id)
            .map(|list| list.iter().map(|t| t.id).collect())
            .unwrap_or_default();

        self.delete(T_MANGA, id)?;
        self.delete_many(T_CHAPTER, &chapter_ids)?;
        self.delete_many(T_HISTORY, &history_ids)?;
        self.delete_many(T_TRACK, &track_ids)?;
        self.delete(T_MANGA_CATS, id)?;

        let mut cache = self.cache.write();
        cache.manga.remove(&id);
        cache.chapters.remove(&id);
        cache.chapter_owner.retain(|_, m| *m != id);
        cache.history.retain(|_, h| h.manga_id != id);
        cache.tracks.remove(&id);
        cache.manga_cats.remove(&id);
        Ok(())
    }

    // -- chapters -----------------------------------------------------------

    pub fn chapters_of(&self, manga_id: Id) -> Vec<Chapter> {
        self.cache
            .read()
            .chapters
            .get(&manga_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn get_chapter(&self, chapter_id: Id) -> Option<Chapter> {
        let cache = self.cache.read();
        let manga_id = cache.chapter_owner.get(&chapter_id)?;
        cache
            .chapters
            .get(manga_id)?
            .iter()
            .find(|c| c.id == chapter_id)
            .cloned()
    }

    pub fn update_chapter(&self, id: Id, f: impl FnOnce(&mut Chapter)) -> Result<Option<Chapter>> {
        let updated = {
            let mut cache = self.cache.write();
            let manga_id = match cache.chapter_owner.get(&id) {
                Some(m) => *m,
                None => return Ok(None),
            };
            let list = match cache.chapters.get_mut(&manga_id) {
                Some(l) => l,
                None => return Ok(None),
            };
            match list.iter_mut().find(|c| c.id == id) {
                Some(c) => {
                    f(c);
                    c.last_modified_at = now_millis();
                    c.version += 1;
                    Some(c.clone())
                }
                None => None,
            }
        };
        if let Some(c) = &updated {
            self.put(T_CHAPTER, c.id, c)?;
        }
        Ok(updated)
    }

    pub fn update_chapters<F>(&self, ids: &[Id], mut f: F) -> Result<Vec<Chapter>>
    where
        F: FnMut(&mut Chapter),
    {
        let mut changed = Vec::new();
        {
            let mut cache = self.cache.write();
            for id in ids {
                let Some(manga_id) = cache.chapter_owner.get(id).copied() else {
                    continue;
                };
                let Some(list) = cache.chapters.get_mut(&manga_id) else {
                    continue;
                };
                if let Some(c) = list.iter_mut().find(|c| c.id == *id) {
                    f(c);
                    c.last_modified_at = now_millis();
                    c.version += 1;
                    changed.push(c.clone());
                }
            }
        }
        let rows: Vec<(i64, Chapter)> = changed.iter().map(|c| (c.id, c.clone())).collect();
        self.put_many(T_CHAPTER, &rows)?;
        Ok(changed)
    }

    /// Reconciles the chapter list coming from a source with what is stored.
    ///
    /// Matching is done on the chapter URL, like upstream's
    /// `SyncChaptersWithSource`: known chapters keep their read progress and
    /// bookmark, chapters missing from the source are dropped, and the rest are
    /// inserted. Returns the newly added chapters.
    pub fn sync_chapters(&self, manga_id: Id, incoming: Vec<Chapter>) -> Result<Vec<Chapter>> {
        let existing = self.chapters_of(manga_id);
        let by_url: HashMap<String, Chapter> = existing
            .iter()
            .map(|c| (c.url.clone(), c.clone()))
            .collect();
        let incoming_urls: HashSet<&str> = incoming.iter().map(|c| c.url.as_str()).collect();

        let removed: Vec<Id> = existing
            .iter()
            .filter(|c| !incoming_urls.contains(c.url.as_str()))
            .map(|c| c.id)
            .collect();

        let mut merged = Vec::with_capacity(incoming.len());
        let mut added = Vec::new();
        for mut chapter in incoming {
            chapter.manga_id = manga_id;
            match by_url.get(&chapter.url) {
                Some(old) => {
                    // Keep local progress, take fresh metadata from the source.
                    chapter.id = old.id;
                    chapter.read = old.read;
                    chapter.bookmark = old.bookmark;
                    chapter.last_page_read = old.last_page_read;
                    chapter.page_count = old.page_count;
                    chapter.date_fetch = old.date_fetch;
                    chapter.version = old.version + 1;
                }
                None => {
                    chapter.id = self.next_id();
                    chapter.date_fetch = now_millis();
                    added.push(chapter.clone());
                }
            }
            chapter.last_modified_at = now_millis();
            merged.push(chapter);
        }

        let rows: Vec<(i64, Chapter)> = merged.iter().map(|c| (c.id, c.clone())).collect();
        self.put_many(T_CHAPTER, &rows)?;
        if !removed.is_empty() {
            self.delete_many(T_CHAPTER, &removed)?;
        }

        {
            let mut cache = self.cache.write();
            for id in &removed {
                cache.chapter_owner.remove(id);
                cache.history.retain(|cid, _| cid != id);
            }
            for c in &merged {
                cache.chapter_owner.insert(c.id, manga_id);
            }
            cache.chapters.insert(manga_id, merged);
        }

        if !removed.is_empty() {
            let history_ids: Vec<Id> = removed.clone();
            let _ = self.delete_many(T_HISTORY, &history_ids);
        }

        Ok(added)
    }

    // -- categories ---------------------------------------------------------

    pub fn categories(&self) -> Vec<Category> {
        self.cache.read().categories.clone()
    }

    pub fn create_category(&self, name: &str) -> Result<Category> {
        let order = self
            .cache
            .read()
            .categories
            .iter()
            .map(|c| c.order)
            .max()
            .unwrap_or(0)
            + 1;
        let category = Category {
            id: self.next_id(),
            name: name.to_string(),
            order,
            flags: 0,
            hidden: false,
        };
        self.put(T_CATEGORY, category.id, &category)?;
        self.cache.write().categories.push(category.clone());
        Ok(category)
    }

    pub fn update_category(&self, category: &Category) -> Result<()> {
        self.put(T_CATEGORY, category.id, category)?;
        let mut cache = self.cache.write();
        if let Some(slot) = cache.categories.iter_mut().find(|c| c.id == category.id) {
            *slot = category.clone();
        }
        cache.categories.sort_by_key(|c| c.order);
        Ok(())
    }

    pub fn delete_category(&self, id: Id) -> Result<()> {
        if id == Category::DEFAULT_ID {
            anyhow::bail!("the default category cannot be deleted");
        }
        self.delete(T_CATEGORY, id)?;

        let reassigned: Vec<(Id, Vec<Id>)> = {
            let mut cache = self.cache.write();
            cache.categories.retain(|c| c.id != id);
            cache
                .manga_cats
                .iter_mut()
                .filter_map(|(manga_id, cats)| {
                    if cats.contains(&id) {
                        cats.retain(|c| *c != id);
                        Some((*manga_id, cats.clone()))
                    } else {
                        None
                    }
                })
                .collect()
        };
        for (manga_id, cats) in reassigned {
            self.put(T_MANGA_CATS, manga_id, &cats)?;
        }
        Ok(())
    }

    pub fn reorder_categories(&self, ordered_ids: &[Id]) -> Result<()> {
        let updated: Vec<Category> = {
            let mut cache = self.cache.write();
            for (index, id) in ordered_ids.iter().enumerate() {
                if let Some(c) = cache.categories.iter_mut().find(|c| c.id == *id) {
                    c.order = index as i64;
                }
            }
            cache.categories.sort_by_key(|c| c.order);
            cache.categories.clone()
        };
        let rows: Vec<(i64, Category)> = updated.iter().map(|c| (c.id, c.clone())).collect();
        self.put_many(T_CATEGORY, &rows)
    }

    pub fn categories_of(&self, manga_id: Id) -> Vec<Id> {
        self.cache
            .read()
            .manga_cats
            .get(&manga_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn set_categories(&self, manga_id: Id, category_ids: Vec<Id>) -> Result<()> {
        self.put(T_MANGA_CATS, manga_id, &category_ids)?;
        self.cache.write().manga_cats.insert(manga_id, category_ids);
        Ok(())
    }

    // -- history ------------------------------------------------------------

    /// Records (or refreshes) the "last read" entry for a chapter.
    pub fn touch_history(&self, manga_id: Id, chapter_id: Id, extra_time: i64) -> Result<()> {
        // `next_id` takes the write lock, so the read guard must be gone first.
        // Binding the boolean ends the temporary here; putting the `read()`
        // straight into a `match` or `if` scrutinee would hold it across the
        // arms and deadlock, since parking_lot locks are not reentrant.
        let is_new = !self.cache.read().history.contains_key(&chapter_id);
        let fresh_id = if is_new { Some(self.next_id()) } else { None };

        let entry = {
            let mut cache = self.cache.write();
            let entry = cache.history.entry(chapter_id).or_insert_with(|| History {
                id: fresh_id.unwrap_or_default(),
                chapter_id,
                manga_id,
                read_at: 0,
                time_read: 0,
            });
            entry.read_at = now_millis();
            entry.time_read += extra_time.max(0);
            entry.clone()
        };
        self.put(T_HISTORY, entry.id, &entry)
    }

    pub fn history_for(&self, chapter_id: Id) -> Option<History> {
        self.cache.read().history.get(&chapter_id).cloned()
    }

    pub fn remove_history(&self, chapter_id: Id) -> Result<()> {
        let id = self.cache.write().history.remove(&chapter_id).map(|h| h.id);
        if let Some(id) = id {
            self.delete(T_HISTORY, id)?;
        }
        Ok(())
    }

    pub fn remove_history_for_manga(&self, manga_id: Id) -> Result<()> {
        let ids: Vec<(Id, Id)> = self
            .cache
            .read()
            .history
            .values()
            .filter(|h| h.manga_id == manga_id)
            .map(|h| (h.chapter_id, h.id))
            .collect();
        {
            let mut cache = self.cache.write();
            for (chapter_id, _) in &ids {
                cache.history.remove(chapter_id);
            }
        }
        let row_ids: Vec<Id> = ids.iter().map(|(_, id)| *id).collect();
        self.delete_many(T_HISTORY, &row_ids)
    }

    pub fn clear_history(&self) -> Result<()> {
        let ids: Vec<Id> = self.cache.read().history.values().map(|h| h.id).collect();
        self.cache.write().history.clear();
        self.delete_many(T_HISTORY, &ids)
    }

    /// Most recently read chapters, newest first, one row per chapter.
    pub fn recent_history(&self, limit: usize, query: &str) -> Vec<HistoryEntry> {
        let cache = self.cache.read();
        let needle = query.to_lowercase();
        let mut rows: Vec<HistoryEntry> = cache
            .history
            .values()
            .filter_map(|h| {
                let manga = cache.manga.get(&h.manga_id)?;
                if !needle.is_empty() && !manga.title.to_lowercase().contains(&needle) {
                    return None;
                }
                let chapter = cache
                    .chapters
                    .get(&h.manga_id)?
                    .iter()
                    .find(|c| c.id == h.chapter_id)?;
                Some(HistoryEntry {
                    history: h.clone(),
                    chapter: chapter.clone(),
                    manga_id: manga.id,
                    manga_title: manga.title.clone(),
                    cover_url: manga.thumbnail_url.clone(),
                    source: manga.source,
                })
            })
            .collect();
        rows.sort_by_key(|row| std::cmp::Reverse(row.history.read_at));
        rows.truncate(limit);
        rows
    }

    // -- tracking -----------------------------------------------------------

    pub fn tracks_of(&self, manga_id: Id) -> Vec<Track> {
        self.cache
            .read()
            .tracks
            .get(&manga_id)
            .cloned()
            .unwrap_or_default()
    }

    pub fn upsert_track(&self, track: &Track) -> Result<Track> {
        let mut stored = track.clone();
        if stored.id == 0 {
            stored.id = self.next_id();
        }
        self.put(T_TRACK, stored.id, &stored)?;
        let mut cache = self.cache.write();
        let list = cache.tracks.entry(stored.manga_id).or_default();
        match list.iter_mut().find(|t| t.id == stored.id) {
            Some(slot) => *slot = stored.clone(),
            None => list.push(stored.clone()),
        }
        Ok(stored)
    }

    pub fn delete_track(&self, manga_id: Id, track_id: Id) -> Result<()> {
        self.delete(T_TRACK, track_id)?;
        if let Some(list) = self.cache.write().tracks.get_mut(&manga_id) {
            list.retain(|t| t.id != track_id);
        }
        Ok(())
    }

    // -- aggregates ---------------------------------------------------------

    /// Builds the rows shown by the library screen.
    pub fn library_entries(&self, downloads: &DownloadIndex) -> Vec<LibraryEntry> {
        let cache = self.cache.read();
        cache
            .manga
            .values()
            .filter(|m| m.favorite)
            .map(|manga| {
                let chapters = cache.chapters.get(&manga.id);
                let empty = Vec::new();
                let chapters = chapters.unwrap_or(&empty);

                let total_chapters = chapters.len() as i64;
                let unread_count = chapters.iter().filter(|c| !c.read).count() as i64;
                let bookmark_count = chapters.iter().filter(|c| c.bookmark).count() as i64;
                let downloaded_count = chapters
                    .iter()
                    .filter(|c| downloads.contains(manga.id, c.id))
                    .count() as i64;
                let has_started = chapters.iter().any(|c| c.read || c.last_page_read > 0);
                let latest_upload = chapters.iter().map(|c| c.date_upload).max().unwrap_or(0);
                let chapter_fetch_date = chapters.iter().map(|c| c.date_fetch).max().unwrap_or(0);
                let last_read = chapters
                    .iter()
                    .filter_map(|c| cache.history.get(&c.id).map(|h| h.read_at))
                    .max()
                    .unwrap_or(0);

                LibraryEntry {
                    manga: manga.clone(),
                    category_ids: cache
                        .manga_cats
                        .get(&manga.id)
                        .cloned()
                        .unwrap_or_else(|| vec![Category::DEFAULT_ID]),
                    total_chapters,
                    unread_count,
                    downloaded_count,
                    has_started,
                    bookmark_count,
                    latest_upload,
                    last_read,
                    chapter_fetch_date,
                    is_tracked: cache
                        .tracks
                        .get(&manga.id)
                        .map(|t| !t.is_empty())
                        .unwrap_or(false),
                }
            })
            .collect()
    }

    /// Recently fetched chapters of favourited manga, newest first.
    pub fn recent_updates(&self, limit: usize, downloads: &DownloadIndex) -> Vec<UpdatesEntry> {
        let cache = self.cache.read();
        let mut rows: Vec<UpdatesEntry> = cache
            .manga
            .values()
            .filter(|m| m.favorite)
            .flat_map(|manga| {
                cache
                    .chapters
                    .get(&manga.id)
                    .into_iter()
                    .flatten()
                    .map(move |chapter| UpdatesEntry {
                        chapter: chapter.clone(),
                        manga_id: manga.id,
                        manga_title: manga.title.clone(),
                        cover_url: manga.thumbnail_url.clone(),
                        source: manga.source,
                        downloaded: downloads.contains(manga.id, chapter.id),
                    })
            })
            .collect();
        rows.sort_by(|a, b| {
            b.chapter.date_fetch.cmp(&a.chapter.date_fetch).then(
                b.chapter
                    .chapter_number
                    .total_cmp(&a.chapter.chapter_number),
            )
        });
        rows.truncate(limit);
        rows
    }

    /// The next chapter to read: the first unread one in reading order.
    pub fn next_unread_chapter(&self, manga_id: Id) -> Option<Chapter> {
        let cache = self.cache.read();
        let chapters = cache.chapters.get(&manga_id)?;
        let mut sorted: Vec<&Chapter> = chapters.iter().collect();
        sorted.sort_by(|a, b| reading_order(a, b));
        sorted
            .iter()
            .find(|c| !c.read)
            .map(|c| (*c).clone())
            .or_else(|| sorted.first().map(|c| (*c).clone()))
    }

    pub fn library_size(&self) -> usize {
        self.cache
            .read()
            .manga
            .values()
            .filter(|m| m.favorite)
            .count()
    }

    /// Snapshot used by the backup writer.
    pub fn export(&self) -> BackupData {
        let cache = self.cache.read();
        BackupData {
            manga: cache.manga.values().cloned().collect(),
            chapters: cache.chapters.values().flatten().cloned().collect(),
            categories: cache.categories.clone(),
            manga_categories: cache
                .manga_cats
                .iter()
                .map(|(k, v)| (*k, v.clone()))
                .collect(),
            history: cache.history.values().cloned().collect(),
            tracks: cache.tracks.values().flatten().cloned().collect(),
        }
    }

    /// Replaces the whole database with a backup's contents.
    pub fn import(&self, data: BackupData) -> Result<()> {
        {
            let tx = self.db.begin_write()?;
            for table in [
                T_MANGA,
                T_CHAPTER,
                T_CATEGORY,
                T_MANGA_CATS,
                T_HISTORY,
                T_TRACK,
            ] {
                let mut t = tx.open_table(table)?;
                t.retain(|_, _| false)?;
            }
            tx.commit()?;
        }

        let manga_rows: Vec<(i64, Manga)> = data.manga.iter().map(|m| (m.id, m.clone())).collect();
        let chapter_rows: Vec<(i64, Chapter)> =
            data.chapters.iter().map(|c| (c.id, c.clone())).collect();
        let category_rows: Vec<(i64, Category)> =
            data.categories.iter().map(|c| (c.id, c.clone())).collect();
        let cat_link_rows: Vec<(i64, Vec<Id>)> = data.manga_categories.clone();
        let history_rows: Vec<(i64, History)> =
            data.history.iter().map(|h| (h.id, h.clone())).collect();
        let track_rows: Vec<(i64, Track)> = data.tracks.iter().map(|t| (t.id, t.clone())).collect();

        self.put_many(T_MANGA, &manga_rows)?;
        self.put_many(T_CHAPTER, &chapter_rows)?;
        self.put_many(T_CATEGORY, &category_rows)?;
        self.put_many(T_MANGA_CATS, &cat_link_rows)?;
        self.put_many(T_HISTORY, &history_rows)?;
        self.put_many(T_TRACK, &track_rows)?;

        self.load()?;
        self.ensure_default_category()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    fn temp_db(tag: &str) -> (Db, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("mihon-db-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let db = Db::open(&dir.join("library.redb")).unwrap();
        (db, dir)
    }

    fn seed(db: &Db) -> (Manga, Vec<Chapter>) {
        let manga = db
            .get_or_insert_manga(&Manga::new(7, "/m/1".into(), "Test".into()))
            .unwrap();
        let incoming: Vec<Chapter> = (1..=3)
            .map(|n| {
                let mut c = Chapter::new(manga.id, format!("/c/{n}"), format!("Ch.{n}"));
                c.chapter_number = n as f64;
                c.source_order = 3 - n;
                c
            })
            .collect();
        db.sync_chapters(manga.id, incoming).unwrap();
        let chapters = db.chapters_of(manga.id);
        (manga, chapters)
    }

    /// Regression: recording history for the first time used to hold a read
    /// guard across an id allocation that wanted the write guard, freezing the
    /// whole app on the first page turn of an unread chapter.
    #[test]
    fn touch_history_does_not_deadlock() {
        let (db, dir) = temp_db("history");
        let (manga, chapters) = seed(&db);

        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            // First call takes the "new entry" path, second the update path.
            db.touch_history(manga.id, chapters[0].id, 1_000).unwrap();
            db.touch_history(manga.id, chapters[0].id, 2_000).unwrap();
            let entry = db.history_for(chapters[0].id).unwrap();
            tx.send(entry).unwrap();
        });

        let entry = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("touch_history deadlocked");
        assert_eq!(entry.time_read, 3_000, "reading time should accumulate");
        assert!(entry.read_at > 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Inspects a real profile: `MIHON_PROBE_DB=<path to library.redb>`.
    #[test]
    #[ignore]
    fn probe_existing_library() {
        let Ok(path) = std::env::var("MIHON_PROBE_DB") else {
            println!("set MIHON_PROBE_DB to run this");
            return;
        };
        let db = Db::open(std::path::Path::new(&path)).expect("could not open the database");
        for manga in db.all_manga() {
            let chapters = db.chapters_of(manga.id);
            let read = chapters.iter().filter(|c| c.read).count();
            println!(
                "[{}] {:?} favourite={} initialized={} chapters={} read={} flags={:#x}",
                manga.id,
                manga.title,
                manga.favorite,
                manga.initialized,
                chapters.len(),
                read,
                manga.chapter_flags
            );
            let mut names: Vec<&str> = chapters.iter().map(|c| c.name.as_str()).collect();
            names.sort();
            if !names.is_empty() {
                println!("   first={:?} last={:?}", names.first(), names.last());
            }
        }
    }

    #[test]
    fn chapters_survive_a_resync_with_their_progress() {
        let (db, dir) = temp_db("resync");
        let (manga, chapters) = seed(&db);

        db.update_chapter(chapters[0].id, |c| {
            c.read = true;
            c.last_page_read = 12;
        })
        .unwrap();

        // The source drops one chapter and adds another.
        let incoming: Vec<Chapter> = [1, 2, 4]
            .iter()
            .map(|n| {
                let mut c = Chapter::new(manga.id, format!("/c/{n}"), format!("Ch.{n}"));
                c.chapter_number = *n as f64;
                c
            })
            .collect();
        let added = db.sync_chapters(manga.id, incoming).unwrap();

        assert_eq!(added.len(), 1, "only chapter 4 is new");
        let stored = db.chapters_of(manga.id);
        assert_eq!(stored.len(), 3);
        let first = stored.iter().find(|c| c.url == "/c/1").unwrap();
        assert!(first.read, "read state must survive a resync");
        assert_eq!(first.last_page_read, 12);
        assert!(
            stored.iter().all(|c| c.url != "/c/3"),
            "removed chapter should be gone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn library_aggregates_are_computed() {
        let (db, dir) = temp_db("aggregate");
        let (manga, chapters) = seed(&db);
        db.update_manga(manga.id, |m| m.favorite = true).unwrap();
        db.update_chapter(chapters[0].id, |c| c.read = true)
            .unwrap();
        db.update_chapter(chapters[1].id, |c| c.bookmark = true)
            .unwrap();

        let downloads = DownloadIndex::default();
        downloads.insert(manga.id, chapters[2].id);

        let entries = db.library_entries(&downloads);
        assert_eq!(entries.len(), 1);
        let entry = &entries[0];
        assert_eq!(entry.total_chapters, 3);
        assert_eq!(entry.unread_count, 2);
        assert_eq!(entry.bookmark_count, 1);
        assert_eq!(entry.downloaded_count, 1);
        assert!(entry.has_started);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Regression: "Bonus" and other unnumbered chapters carry -1, which used
    /// to sort them ahead of chapter 0, so "continue reading" kept offering the
    /// bonus chapter no matter how far along you were.
    #[test]
    fn unnumbered_chapters_do_not_hijack_continue_reading() {
        let (db, dir) = temp_db("bonus");
        let manga = db
            .get_or_insert_manga(&Manga::new(7, "/m/1".into(), "Test".into()))
            .unwrap();

        // As AnimeSama returns them: newest first, with a "Bonus" at the end.
        let incoming: Vec<Chapter> = [
            ("Chapitre 2", 2.0),
            ("Chapitre 1", 1.0),
            ("Chapitre Bonus", -1.0),
        ]
        .into_iter()
        .enumerate()
        .map(|(index, (name, number))| {
            let mut c = Chapter::new(manga.id, format!("/c/{name}"), name.to_string());
            c.chapter_number = number;
            c.source_order = index as i64;
            c
        })
        .collect();
        db.sync_chapters(manga.id, incoming).unwrap();

        assert_eq!(db.next_unread_chapter(manga.id).unwrap().name, "Chapitre 1");

        // After reading 1, the next is 2 — still not the bonus.
        let first = db
            .chapters_of(manga.id)
            .into_iter()
            .find(|c| c.chapter_number == 1.0)
            .unwrap();
        db.update_chapter(first.id, |c| c.read = true).unwrap();
        assert_eq!(db.next_unread_chapter(manga.id).unwrap().name, "Chapitre 2");

        // Only once everything numbered is read does the bonus come up.
        let second = db
            .chapters_of(manga.id)
            .into_iter()
            .find(|c| c.chapter_number == 2.0)
            .unwrap();
        db.update_chapter(second.id, |c| c.read = true).unwrap();
        assert_eq!(
            db.next_unread_chapter(manga.id).unwrap().name,
            "Chapitre Bonus"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn next_unread_walks_in_reading_order() {
        let (db, dir) = temp_db("next");
        let (manga, chapters) = seed(&db);

        assert_eq!(
            db.next_unread_chapter(manga.id).unwrap().chapter_number,
            1.0
        );
        db.update_chapter(chapters[0].id, |c| c.read = true)
            .unwrap();
        // chapters[0] is Ch.1 because sync keeps source order; find it by number.
        let first = db
            .chapters_of(manga.id)
            .into_iter()
            .find(|c| c.chapter_number == 1.0)
            .unwrap();
        db.update_chapter(first.id, |c| c.read = true).unwrap();
        assert_eq!(
            db.next_unread_chapter(manga.id).unwrap().chapter_number,
            2.0
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn deleting_a_manga_takes_its_rows_with_it() {
        let (db, dir) = temp_db("delete");
        let (manga, chapters) = seed(&db);
        db.touch_history(manga.id, chapters[0].id, 0).unwrap();
        db.set_categories(manga.id, vec![Category::DEFAULT_ID])
            .unwrap();

        db.delete_manga(manga.id).unwrap();
        assert!(db.get_manga(manga.id).is_none());
        assert!(db.chapters_of(manga.id).is_empty());
        assert!(db.history_for(chapters[0].id).is_none());
        assert!(db.categories_of(manga.id).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn data_survives_a_reopen() {
        let dir = std::env::temp_dir().join(format!("mihon-db-reopen-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("library.redb");

        let manga_id = {
            let db = Db::open(&path).unwrap();
            let (manga, chapters) = seed(&db);
            db.update_manga(manga.id, |m| m.favorite = true).unwrap();
            db.touch_history(manga.id, chapters[0].id, 500).unwrap();
            manga.id
        };

        let db = Db::open(&path).unwrap();
        let manga = db.get_manga(manga_id).expect("manga should be persisted");
        assert!(manga.favorite);
        assert_eq!(db.chapters_of(manga_id).len(), 3);
        assert_eq!(db.recent_history(10, "").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackupData {
    pub manga: Vec<Manga>,
    pub chapters: Vec<Chapter>,
    pub categories: Vec<Category>,
    pub manga_categories: Vec<(Id, Vec<Id>)>,
    pub history: Vec<History>,
    pub tracks: Vec<Track>,
}
