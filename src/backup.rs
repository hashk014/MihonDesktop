//! Backup and restore.
//!
//! Upstream writes a protobuf `.tachibk`; here it is gzipped JSON, which keeps
//! the file inspectable and avoids a schema compiler. The payload covers the
//! whole library plus the preferences.

use std::io::{Read, Write};
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

use crate::db::{BackupData, Db};
use crate::prefs::Preferences;

const MAGIC: &str = "mihon-desktop-backup";
const FORMAT_VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
pub struct Backup {
    pub magic: String,
    pub version: u32,
    pub created_at: i64,
    pub library: BackupData,
    /// Optional so a library-only backup can be restored onto existing settings.
    #[serde(default)]
    pub preferences: Option<Preferences>,
}

impl Backup {
    pub fn create(db: &Db, prefs: Option<&Preferences>) -> Self {
        Self {
            magic: MAGIC.to_string(),
            version: FORMAT_VERSION,
            created_at: crate::model::now_millis(),
            library: db.export(),
            preferences: prefs.cloned(),
        }
    }

    pub fn encode(&self) -> Result<Vec<u8>> {
        let json = serde_json::to_vec(self).context("encoding the backup")?;
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(&json)?;
        encoder.finish().context("compressing the backup")
    }

    pub fn decode(bytes: &[u8]) -> Result<Self> {
        // Accept both compressed and plain JSON, so a hand-edited file still works.
        let json = if bytes.starts_with(&[0x1f, 0x8b]) {
            let mut decoder = flate2::read::GzDecoder::new(bytes);
            let mut buffer = Vec::new();
            decoder
                .read_to_end(&mut buffer)
                .context("decompressing the backup")?;
            buffer
        } else {
            bytes.to_vec()
        };

        let backup: Self =
            serde_json::from_slice(&json).context("this file is not a readable backup")?;
        if backup.magic != MAGIC {
            bail!("this file was not produced by Mihon Desktop");
        }
        if backup.version > FORMAT_VERSION {
            bail!(
                "the backup was written by a newer version (format {})",
                backup.version
            );
        }
        Ok(backup)
    }

    pub fn write_to(&self, path: &Path) -> Result<()> {
        let bytes = self.encode()?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, bytes).with_context(|| format!("writing {}", path.display()))
    }

    pub fn read_from(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        Self::decode(&bytes)
    }

    pub fn summary(&self) -> String {
        format!(
            "{} entries, {} chapters, {} categories",
            self.library.manga.len(),
            self.library.chapters.len(),
            self.library.categories.len()
        )
    }
}

/// Default file name for a new backup, stamped with the date.
pub fn suggested_filename() -> String {
    let now = chrono::Local::now();
    format!("mihon-desktop-{}.json.gz", now.format("%Y-%m-%d-%H%M"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Category, Manga};

    fn sample() -> Backup {
        Backup {
            magic: MAGIC.into(),
            version: FORMAT_VERSION,
            created_at: 1,
            library: BackupData {
                manga: vec![Manga::new(7, "/m/1".into(), "Title".into())],
                chapters: Vec::new(),
                categories: vec![Category {
                    id: 0,
                    name: "Default".into(),
                    order: 0,
                    flags: 0,
                    hidden: false,
                }],
                manga_categories: Vec::new(),
                history: Vec::new(),
                tracks: Vec::new(),
            },
            preferences: Some(Preferences::default()),
        }
    }

    #[test]
    fn round_trips_through_gzip() {
        let backup = sample();
        let bytes = backup.encode().unwrap();
        assert_eq!(&bytes[..2], &[0x1f, 0x8b], "should be gzip framed");

        let restored = Backup::decode(&bytes).unwrap();
        assert_eq!(restored.library.manga.len(), 1);
        assert_eq!(restored.library.manga[0].title, "Title");
        assert!(restored.preferences.is_some());
    }

    #[test]
    fn plain_json_is_accepted() {
        let json = serde_json::to_vec(&sample()).unwrap();
        assert!(Backup::decode(&json).is_ok());
    }

    #[test]
    fn foreign_files_are_rejected() {
        let json = br#"{"magic":"something-else","version":1,"created_at":0,
                        "library":{"manga":[],"chapters":[],"categories":[],
                        "manga_categories":[],"history":[],"tracks":[]}}"#;
        let err = Backup::decode(json).unwrap_err().to_string();
        assert!(err.contains("not produced by"), "unexpected error: {err}");
    }

    #[test]
    fn newer_formats_are_refused() {
        let mut backup = sample();
        backup.version = FORMAT_VERSION + 1;
        let bytes = backup.encode().unwrap();
        assert!(Backup::decode(&bytes).is_err());
    }
}
