//! `project.toml`: the binder and the project's settings, read and written safely.
//!
//! The file is hostile input: it is refused past [`MAX_FILE_BYTES`], [`MAX_ITEMS`] items or
//! [`MAX_DEPTH`] levels of folders, and every id must be [`valid_id`] (it names a file). Keys
//! WordCraft doesn't know are kept and written back, so a newer version's settings survive an
//! older one. A file that can't be used opens in recovery mode instead: every `sections/*.docx`
//! in a flat binder ([`Project::recovered`] says why), and the next save keeps the broken file
//! as `project.toml.broken-<seconds>` before writing.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// The project file's name in the project folder.
pub const PROJECT_FILE: &str = "project.toml";
/// The folder sections live in.
pub const SECTIONS: &str = "sections";
/// The folder snapshots live in.
pub const SNAPSHOTS: &str = "snapshots";
/// The folder research files live in.
pub const RESEARCH: &str = "research";
/// The project format this version writes.
pub const FORMAT: u32 = 1;

/// Largest `project.toml` read (bytes).
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;
/// Most items in a binder.
pub const MAX_ITEMS: usize = 10_000;
/// Deepest nesting of folders.
pub const MAX_DEPTH: usize = 32;
/// Longest title, synopsis, label or status kept (characters).
pub const MAX_TEXT: usize = 10_000;
/// The id of the binder's Trash folder (deleted items wait there).
pub const TRASH: &str = "trash";

/// What an item is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Folder,
    Section,
}

/// A binder item: a folder (it groups items) or a section (one `.docx`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub kind: Kind,
    #[serde(default)]
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub synopsis: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub label: String,
    /// Free text; the UI offers todo, draft, revised and final.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub status: String,
    /// What the item is in the manuscript (cover, abstract, chapter, appendix…), for compiling.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub role: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_words: Option<u64>,
    /// How its pages are numbered when compiled (inherited by what a folder holds).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub numbering: Option<NumberingRule>,
    /// Whether compiling includes it.
    #[serde(default = "yes")]
    pub include: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<Item>,
    /// Keys this version doesn't know, kept as they were.
    #[serde(flatten)]
    pub extra: toml::Table,
}

fn yes() -> bool {
    true
}

impl Item {
    pub fn new(id: String, kind: Kind, title: String) -> Self {
        Item {
            id,
            kind,
            title,
            synopsis: String::new(),
            label: String::new(),
            status: String::new(),
            role: String::new(),
            target_words: None,
            numbering: None,
            include: true,
            children: Vec::new(),
            extra: toml::Table::new(),
        }
    }
}

/// How an item's pages are numbered in the compiled document (`[item.numbering]`). Unset
/// fields come from the folder above.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct NumberingRule {
    /// `continue` (as before), `front` (i, ii, iii from i), `main` (1, 2, 3 from 1), `appendix`
    /// (numbers carry on), `none` (no numbers shown).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub zone: Option<String>,
    /// `all`, `not-first` (none on its first page) or `none`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub show: Option<String>,
    /// `next` (a new page), `odd` (a right-hand page) or `even`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start_on: Option<String>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

/// Word targets for the whole project.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Targets {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_words: Option<u64>,
    /// A date, `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub deadline: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_words: Option<u64>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

impl Targets {
    fn is_empty(&self) -> bool {
        self.project_words.is_none() && self.deadline.is_none() && self.session_words.is_none() && self.extra.is_empty()
    }
}

/// The contents of `project.toml`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct ProjectFile {
    #[serde(default = "format")]
    pub format: u32,
    #[serde(default)]
    pub title: String,
    /// Left blank unless the writer sets it: never guessed from the system or git.
    #[serde(default)]
    pub author: String,
    /// The number the next new item's id is made from.
    #[serde(default = "one")]
    pub next_id: u64,
    #[serde(default, skip_serializing_if = "Targets::is_empty")]
    pub targets: Targets,
    #[serde(default, rename = "item", skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<Item>,
    /// Compile presets (milestone 2), kept as they are until then.
    #[serde(default, rename = "preset", skip_serializing_if = "Vec::is_empty")]
    pub presets: Vec<toml::Table>,
    #[serde(flatten)]
    pub extra: toml::Table,
}

fn format() -> u32 {
    FORMAT
}

fn one() -> u64 {
    1
}

impl ProjectFile {
    pub fn new(title: &str) -> Self {
        ProjectFile {
            format: FORMAT,
            title: title.to_string(),
            author: String::new(),
            next_id: 1,
            targets: Targets::default(),
            items: Vec::new(),
            presets: Vec::new(),
            extra: toml::Table::new(),
        }
    }

    /// A new item id (`s0001`, `s0002`…) not used in the binder.
    pub fn new_id(&mut self) -> Result<String, String> {
        for _ in 0..=MAX_ITEMS {
            let id = format!("s{:04}", self.next_id);
            self.next_id = self.next_id.checked_add(1).ok_or("no item ids left")?;
            if crate::binder::find(&self.items, &id).is_none() && id != TRASH {
                return Ok(id);
            }
        }
        Err("no free item id".into())
    }
}

/// An open project: its folder and binder.
#[derive(Clone, Debug)]
pub struct Project {
    pub root: PathBuf,
    pub file: ProjectFile,
    /// Why the project opened in recovery mode (its `project.toml` couldn't be used), if it did.
    pub recovered: Option<String>,
}

/// Whether `id` can name an item (and its file): 1–32 lowercase ASCII letters and digits.
pub fn valid_id(id: &str) -> bool {
    (1..=32).contains(&id.len()) && id.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
}

/// Check a parsed binder: ids valid and unique, the item count and folder depth within the
/// caps, sections without children, texts within [`MAX_TEXT`].
pub fn validate(file: &ProjectFile) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    let mut stack: Vec<(&Item, usize)> = file.items.iter().map(|i| (i, 1)).collect();
    while let Some((item, depth)) = stack.pop() {
        if depth > MAX_DEPTH {
            return Err(format!("folders nest more than {MAX_DEPTH} deep"));
        }
        if !valid_id(&item.id) {
            return Err(format!("item id {:?} isn't 1-32 lowercase letters and digits", truncate(&item.id, 40)));
        }
        if !seen.insert(item.id.as_str()) {
            return Err(format!("item id {:?} is used twice", item.id));
        }
        if seen.len() > MAX_ITEMS {
            return Err(format!("more than {MAX_ITEMS} items"));
        }
        if item.kind == Kind::Section && !item.children.is_empty() {
            return Err(format!("section {:?} has items inside it", item.id));
        }
        for t in [&item.title, &item.synopsis, &item.label, &item.status, &item.role] {
            if t.chars().count() > MAX_TEXT {
                return Err(format!("a text of item {:?} is longer than {MAX_TEXT} characters", item.id));
            }
        }
        stack.extend(item.children.iter().map(|c| (c, depth + 1)));
    }
    Ok(())
}

fn truncate(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The text of a `project.toml`, refused past [`MAX_FILE_BYTES`].
fn read_capped(path: &Path) -> Result<String, String> {
    wordcraft_engine::file_access::check_read(&path.to_string_lossy())?;
    let meta = std::fs::metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if meta.len() > MAX_FILE_BYTES {
        return Err(format!("{} is larger than {} MiB", PROJECT_FILE, MAX_FILE_BYTES / (1024 * 1024)));
    }
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    String::from_utf8(bytes).map_err(|_| format!("{PROJECT_FILE} isn't UTF-8 text"))
}

/// Parse and check a `project.toml`.
pub fn parse(text: &str) -> Result<ProjectFile, String> {
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(format!("{PROJECT_FILE} is too large"));
    }
    // Nesting is checked before parsing too, so a hostile file can't make the parser recurse
    // deep: brackets and braces outside strings, and a table header's dotted path.
    if nesting(text) > MAX_DEPTH * 2 + 4 || text.lines().any(|l| l.trim_start().starts_with('[') && l.matches('.').count() > MAX_DEPTH * 2) {
        return Err(format!("folders nest more than {MAX_DEPTH} deep"));
    }
    let file: ProjectFile = toml::from_str(text).map_err(|e| format!("{PROJECT_FILE}: {}", truncate(&e.to_string(), 400)))?;
    validate(&file)?;
    Ok(file)
}

/// The deepest nesting of `[` and `{` in TOML text, outside strings and comments.
fn nesting(text: &str) -> usize {
    let b = text.as_bytes();
    let (mut depth, mut deepest, mut i) = (0usize, 0usize, 0usize);
    while let Some(&c) = b.get(i) {
        match c {
            b'#' => {
                while b.get(i).is_some_and(|&c| c != b'\n') {
                    i += 1;
                }
            }
            b'"' | b'\'' => {
                // Strings: one quote, or three for a multi-line string; `\` escapes in "…".
                let triple = b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c);
                let open = if triple { 3 } else { 1 };
                i += open;
                loop {
                    match b.get(i) {
                        None => break,
                        Some(b'\\') if c == b'"' => i += 2,
                        Some(&q) if q == c && (!triple || (b.get(i + 1) == Some(&c) && b.get(i + 2) == Some(&c))) => {
                            i += open - 1;
                            break;
                        }
                        Some(b'\n') if !triple => break,
                        Some(_) => i += 1,
                    }
                }
            }
            b'[' | b'{' => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            b']' | b'}' => depth = depth.saturating_sub(1),
            _ => {}
        }
        i += 1;
    }
    deepest
}

/// The section ids that have a file in `sections/`, sorted (for recovery mode).
fn section_files(root: &Path) -> Vec<String> {
    let mut ids: Vec<String> = std::fs::read_dir(root.join(SECTIONS))
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix(".docx")).map(str::to_string))
                .filter(|id| valid_id(id))
                .take(MAX_ITEMS)
                .collect()
        })
        .unwrap_or_default();
    ids.sort();
    ids
}

impl Project {
    /// Open the project in folder `root`. A missing folder or `project.toml` is an error; a
    /// `project.toml` that can't be used opens in recovery mode.
    pub fn open(root: &Path) -> Result<Project, String> {
        let path = root.join(PROJECT_FILE);
        if !root.is_dir() {
            return Err(format!("{} isn't a project folder", root.display()));
        }
        if !path.is_file() {
            return Err(format!("{} has no {PROJECT_FILE}", root.display()));
        }
        match read_capped(&path).and_then(|t| parse(&t)) {
            Ok(file) if file.format > FORMAT => {
                Err(format!("this project was saved by a newer WordCraft (format {}); it can't be changed here", file.format))
            }
            Ok(file) => Ok(Project { root: root.to_path_buf(), file, recovered: None }),
            Err(why) => {
                // Recovery mode: every section with a file, in id order.
                let mut file = ProjectFile::new(root.file_stem().and_then(|s| s.to_str()).unwrap_or("Recovered project"));
                for id in section_files(root) {
                    file.items.push(Item::new(id.clone(), Kind::Section, id));
                }
                let highest = file.items.iter().filter_map(|i| i.id.strip_prefix('s')).filter_map(|n| n.parse::<u64>().ok()).max().unwrap_or(0);
                file.next_id = highest.saturating_add(1);
                Ok(Project { root: root.to_path_buf(), file, recovered: Some(why) })
            }
        }
    }

    /// Make a new project in folder `root`, which must not exist yet or be empty.
    pub fn create(root: &Path, title: &str) -> Result<Project, String> {
        if root.exists() && std::fs::read_dir(root).map(|mut rd| rd.next().is_some()).unwrap_or(true) {
            return Err(format!("{} already exists and isn't empty", root.display()));
        }
        wordcraft_engine::file_access::check_write(&root.join(PROJECT_FILE).to_string_lossy())?;
        for dir in [SECTIONS, SNAPSHOTS, RESEARCH] {
            std::fs::create_dir_all(root.join(dir)).map_err(|e| format!("{}: {e}", root.join(dir).display()))?;
        }
        let mut project = Project { root: root.to_path_buf(), file: ProjectFile::new(title), recovered: None };
        project.save()?;
        Ok(project)
    }

    /// Write `project.toml`. A file there that can't be read as a project is kept first, as
    /// `project.toml.broken-<seconds>`, so recovery never loses it.
    pub fn save(&mut self) -> Result<(), String> {
        validate(&self.file)?;
        let path = self.root.join(PROJECT_FILE);
        if path.is_file() && read_capped(&path).and_then(|t| parse(&t)).is_err() {
            let kept = self.root.join(format!("{PROJECT_FILE}.broken-{}", wordcraft_engine::cmd::now_unix()));
            wordcraft_engine::file_access::check_write(&kept.to_string_lossy())?;
            std::fs::rename(&path, &kept).map_err(|e| format!("keeping the broken {PROJECT_FILE}: {e}"))?;
        }
        let text = toml::to_string(&self.file).map_err(|e| format!("{PROJECT_FILE}: {e}"))?;
        wordcraft_engine::io::write_file(&path.to_string_lossy(), text.as_bytes())?;
        self.recovered = None;
        Ok(())
    }

    /// The file of section `id`.
    pub fn section_path(&self, id: &str) -> Result<PathBuf, String> {
        if !valid_id(id) {
            return Err(format!("{id:?} isn't an item id"));
        }
        Ok(self.root.join(SECTIONS).join(format!("{id}.docx")))
    }
}
