//! `.wcproj-cache.toml`: each section's word count, kept by its file's modification time and
//! size, so the binder's totals don't open every section. Never needed for correctness: an entry
//! whose file changed is counted again, and a missing or damaged cache is rebuilt.

use std::collections::BTreeMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::project::{MAX_FILE_BYTES, MAX_ITEMS, Project};

pub const CACHE_FILE: &str = ".wcproj-cache.toml";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Entry {
    pub modified: u64,
    pub size: u64,
    pub words: u64,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct Cache {
    #[serde(default)]
    pub sections: BTreeMap<String, Entry>,
}

/// A file's modification time (seconds) and size, if it can be read.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok()?;
    let modified = meta.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs();
    Some((modified, meta.len()))
}

impl Cache {
    pub fn load(project: &Project) -> Cache {
        let path = project.root.join(CACHE_FILE);
        let ok = wordcraft_engine::file_access::check_read(&path.to_string_lossy()).is_ok()
            && std::fs::metadata(&path).is_ok_and(|m| m.len() <= MAX_FILE_BYTES);
        let text = if ok { std::fs::read_to_string(&path).unwrap_or_default() } else { String::new() };
        let mut cache: Cache = toml::from_str(&text).unwrap_or_default();
        if cache.sections.len() > MAX_ITEMS {
            cache.sections.clear();
        }
        cache
    }

    /// Section `id`'s word count: from the cache while its file is unchanged, else counted.
    /// None when the file can't be opened. `changed` is set when the cache gained an entry.
    pub fn words(&mut self, project: &Project, id: &str, changed: &mut bool) -> Option<u64> {
        let path = project.section_path(id).ok()?;
        let (modified, size) = stamp(&path)?;
        if let Some(e) = self.sections.get(id)
            && e.modified == modified
            && e.size == size
        {
            return Some(e.words);
        }
        wordcraft_engine::file_access::check_read(&path.to_string_lossy()).ok()?;
        let doc = wordcraft_engine::io::open_path(&path).ok()?;
        let words = u64::try_from(doc.word_count()).unwrap_or(u64::MAX);
        if self.sections.len() < MAX_ITEMS {
            self.sections.insert(id.to_string(), Entry { modified, size, words });
            *changed = true;
        }
        Some(words)
    }

    /// Write the cache; failing to is not an error (it's only a cache).
    pub fn save(&self, project: &Project) {
        if let Ok(text) = toml::to_string(self) {
            let _ = wordcraft_engine::io::write_file(&project.root.join(CACHE_FILE).to_string_lossy(), text.as_bytes());
        }
    }
}
