//! Snapshots: saved copies of a section, as `snapshots/<id>/<UTC time>.docx` with a small
//! `.toml` beside it (label, word count). Restoring takes a snapshot of the section as it is
//! first, so a restore can be undone by restoring that one.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::project::{MAX_ITEMS, MAX_TEXT, Project, SNAPSHOTS};

/// Largest section file copied into a snapshot (bytes).
pub const MAX_SECTION_BYTES: u64 = 512 * 1024 * 1024;

/// A snapshot's details, from its `.toml`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub words: u64,
}

/// A snapshot: its name (the time it was taken, `20261107T143000Z`, maybe with `-2` after
/// it) and details.
#[derive(Clone, Debug, PartialEq)]
pub struct Snapshot {
    pub at: String,
    pub meta: Meta,
}

/// Whether `at` can name a snapshot: digits, `T`, `Z` and `-`, at most 40 characters.
pub fn valid_name(at: &str) -> bool {
    (1..=40).contains(&at.len()) && at.bytes().all(|b| b.is_ascii_digit() || matches!(b, b'T' | b'Z' | b'-'))
}

fn dir(project: &Project, id: &str) -> Result<PathBuf, String> {
    // The section path checks the id.
    project.section_path(id)?;
    Ok(project.root.join(SNAPSHOTS).join(id))
}

/// Now, as a snapshot name (UTC, to the second).
fn now_name() -> String {
    let secs = i64::try_from(wordcraft_engine::cmd::now_unix()).unwrap_or(0);
    chrono::DateTime::from_timestamp(secs, 0).map_or_else(|| "19700101T000000Z".to_string(), |t| t.format("%Y%m%dT%H%M%SZ").to_string())
}

/// The bytes of section `id`'s file.
fn section_bytes(project: &Project, id: &str) -> Result<Vec<u8>, String> {
    let path = project.section_path(id)?;
    wordcraft_engine::file_access::check_read(&path.to_string_lossy())?;
    let meta = std::fs::metadata(&path).map_err(|e| format!("section {id}: {e}"))?;
    if meta.len() > MAX_SECTION_BYTES {
        return Err(format!("section {id} is too large to snapshot"));
    }
    std::fs::read(&path).map_err(|e| format!("section {id}: {e}"))
}

/// Take a snapshot of section `id` now.
pub fn take(project: &Project, id: &str, label: &str) -> Result<Snapshot, String> {
    let bytes = section_bytes(project, id)?;
    let dir = dir(project, id)?;
    wordcraft_engine::file_access::check_write(&dir.join("x.docx").to_string_lossy())?;
    std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let base = now_name();
    // Two in the same second get -2, -3…
    let at = (1..=MAX_ITEMS)
        .map(|n| if n == 1 { base.clone() } else { format!("{base}-{n}") })
        .find(|at| !dir.join(format!("{at}.docx")).exists())
        .ok_or("too many snapshots this second")?;
    let words = wordcraft_engine::io::open_path(&project.section_path(id)?).map(|d| u64::try_from(d.word_count()).unwrap_or(u64::MAX)).unwrap_or(0);
    let meta = Meta { label: label.chars().take(MAX_TEXT).collect(), words };
    wordcraft_engine::io::write_file(&dir.join(format!("{at}.docx")).to_string_lossy(), &bytes)?;
    let text = toml::to_string(&meta).map_err(|e| e.to_string())?;
    wordcraft_engine::io::write_file(&dir.join(format!("{at}.toml")).to_string_lossy(), text.as_bytes())?;
    Ok(Snapshot { at, meta })
}

/// Section `id`'s snapshots, oldest first (at most [`MAX_ITEMS`]).
pub fn list(project: &Project, id: &str) -> Result<Vec<Snapshot>, String> {
    let dir = dir(project, id)?;
    wordcraft_engine::file_access::check_read(&dir.to_string_lossy())?;
    let Ok(rd) = std::fs::read_dir(&dir) else { return Ok(Vec::new()) };
    let mut names: Vec<String> = rd
        .filter_map(Result::ok)
        .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".docx")).map(str::to_string))
        .filter(|n| valid_name(n))
        .take(MAX_ITEMS)
        .collect();
    names.sort();
    Ok(names
        .into_iter()
        .map(|at| {
            let path = dir.join(format!("{at}.toml"));
            let small = std::fs::metadata(&path).is_ok_and(|m| m.len() <= 64 * 1024);
            let meta =
                if small { std::fs::read_to_string(&path).ok().and_then(|t| toml::from_str(&t).ok()).unwrap_or_default() } else { Meta::default() };
            Snapshot { at, meta }
        })
        .collect())
}

/// Put snapshot `at` back as section `id`, after taking a snapshot of the section as it is.
/// Returns that new snapshot.
pub fn restore(project: &Project, id: &str, at: &str) -> Result<Snapshot, String> {
    if !valid_name(at) {
        return Err(format!("{at:?} isn't a snapshot"));
    }
    let src = dir(project, id)?.join(format!("{at}.docx"));
    wordcraft_engine::file_access::check_read(&src.to_string_lossy())?;
    let len = std::fs::metadata(&src).map_err(|_| format!("section {id} has no snapshot {at}"))?.len();
    if len > MAX_SECTION_BYTES {
        return Err(format!("snapshot {at} is too large"));
    }
    let bytes = std::fs::read(&src).map_err(|e| format!("snapshot {at}: {e}"))?;
    let before = take(project, id, &format!("Before restoring {at}"))?;
    wordcraft_engine::io::write_file(&project.section_path(id)?.to_string_lossy(), &bytes)?;
    Ok(before)
}
