//! The `project.*`, `binder.*` and `snapshot.*` commands. Each names the project folder in
//! `"project"` (except `project.new`, which takes `"path"`), reads `project.toml`, and writes it
//! back when it changes the binder. None of them edits the open document, so none is an undo
//! step; `binder.openItem` opens a section as File › Open does.

use std::path::Path;

use serde_json::{Value, json};
use wordcraft_engine::{CmdError, CmdResult, CommandSpec, Session, p};

use crate::binder;
use crate::cache::Cache;
use crate::project::{Item, Kind, MAX_TEXT, Project, TRASH, valid_id};
use crate::snapshot;

const LOC: &str = "Project";

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("project.new", "New Project", LOC, new)
            .params(r#"{"path": folder to make (must not exist, or be empty), "title"?: string, "author"?: string, "template"?: "blank" (default: one empty section) | "thesis" (front matter, six chapters, references, an appendix) | "book" (front matter, three chapters, back matter)} → project.info's answer and "first": the section to open: the folder, project.toml, sections/, snapshots/, research/ and the template's sections, each starting from the project's base document (heading styles numbered 1 / 1.1 / 1.1.1, or "Chapter 1" for a book)"#)
            .pure(),
        CommandSpec::new("project.info", "Project Info", LOC, |_, v| info(&open(v)?))
            .params(r#"{"project": folder} → {"title", "author", "targets", "recovered": why project.toml couldn't be used (the binder then lists every section file) | null, "words": included sections' total, "items": [{"id", "kind": "folder|section", "title", "synopsis", "label", "status", "role", "target", "include", "words" (sections), "children" (folders)}]}"#)
            .pure(),
        CommandSpec::new("project.outline", "Project Outline", LOC, outline)
            .params(r#"{"project": folder, "skip"?: section id (left out: the one open in the editor, whose outline is the live document's)} → {"sections": [{"id", "title", "role", "outline": structure.outline's answer for the section's file | null (skipped, or unreadable)}]}: the headings, appendices, figures, tables and equations of every included section, in binder order. Each file is read once and again only when it changes"#)
            .pure(),
        CommandSpec::new("project.setMetadata", "Project Properties", LOC, set_metadata)
            .params(r#"{"project": folder, "title"?: string, "author"?: string, "projectWords"?: n | null, "sessionWords"?: n | null, "deadline"?: "YYYY-MM-DD" | null}"#)
            .pure(),
        CommandSpec::new("binder.add", "New Binder Item", LOC, add)
            .params(r#"{"project": folder, "kind"?: "section|folder" (default section), "title"?: string, "parent"?: folder id (default the top level), "index"?: n (default last), "role"?: string (contents, figures, tables, equations, bibliography or index: compile puts that generated list there)} → {"id"}: a section gets an empty document"#)
            .pure(),
        CommandSpec::new("binder.import", "Import Document", LOC, import)
            .params(r#"{"project": folder, "path": a document (any format WordCraft opens), "split"?: "heading1" (default) | "heading2" | "none", "parent"?: folder id} → {"ids": the new sections, "converted": typed headings turned into real ones}: one section per Heading 1 (or Heading 1 and 2), titled by its heading, the text before the first heading a section of its own. A document with no heading styles has its typed numbered headings ("1 Introduction", "2.1 Methods") made real first"#)
            .pure(),
        CommandSpec::new("binder.move", "Move Binder Item", LOC, move_item)
            .params(r#"{"project": folder, "id", "parent"?: folder id | null (the top level), "index"?: n (default last)}"#)
            .pure(),
        CommandSpec::new("binder.setMeta", "Binder Item Properties", LOC, set_meta)
            .params(r#"{"project": folder, "id", "title"?, "synopsis"?, "label"?, "status"?, "role"?: string, "target"?: words | null, "include"?: bool, "numbering"?: {"zone"?: "continue|front|main|appendix|none", "show"?: "all|not-first|none", "start"?: n (the number its first page takes: "start at 5 from here") | null, "counted"?: bool (false: its pages show no number and aren't counted; the pages after number as if they weren't there), "startOn"?: "next|odd|even"} | null (how its pages are numbered when compiled; what a folder holds inherits it)}"#)
            .pure(),
        CommandSpec::new("binder.rename", "Rename Binder Item", LOC, set_meta).params(r#"{"project": folder, "id", "title": string}"#).pure(),
        CommandSpec::new("binder.delete", "Delete Binder Item", LOC, delete)
            .params(r#"{"project": folder, "id"}: into the Trash folder; nothing is removed until binder.emptyTrash"#)
            .pure(),
        CommandSpec::new("binder.emptyTrash", "Empty Trash", LOC, empty_trash)
            .params(r#"{"project": folder} → {"removed": [section ids]}: the Trash's items go, and their section files (their snapshots stay)"#)
            .pure(),
        CommandSpec::new("binder.openItem", "Open Binder Item", LOC, open_item)
            .params(r#"{"project": folder, "id": section id}: opens the section's document, as File › Open does"#),
        CommandSpec::new("compile.presets", "Compile Presets", LOC, |_, v| {
            let pr = open(v)?;
            let list = crate::compile::presets(&pr).map_err(fail)?;
            Ok(json!({"presets": list.iter().map(|p| json!({"name": p.name, "output": p.output, "template": p.template})).collect::<Vec<_>>()}))
        })
        .params(r#"{"project": folder} → {"presets": [{"name", "output", "template"}]} (a Default preset when the project has none)"#)
        .pure(),
        CommandSpec::new("compile.run", "Compile", LOC, compile_run)
            .params(r#"{"project": folder, "path": file to write (its extension picks the format: pdf, docx, odt, rtf, html, md, tex, txt; none: the preset's output), "preset"?: name (default the first)} → {"path", "sections", "words", "warnings"}: the included sections in binder order, joined with the preset's template and separators, fields, tables of contents and heading numbers updated. Section files are not changed"#)
            .pure(),
        CommandSpec::new("compile.check", "Check Before Submitting", LOC, |_, v| {
            let pr = open(v)?;
            let preset = crate::compile::preset(&pr, p::str(v, "preset")).map_err(fail)?;
            let c = crate::compile::compile(&pr, &preset).map_err(fail)?;
            Ok(crate::compile::check(&c, &preset))
        })
        .params(r#"{"project": folder, "preset"?: name} → {"ok": false when the preset is final and comments, tracked changes or markers (TODO, [CITE]…) are left, "problems": [{"kind", "message", "error"}], "sections", "words"}: compiles in memory and writes nothing"#)
        .pure(),
        CommandSpec::new("compile.preview", "Compile Preview", LOC, compile_preview)
            .params(r#"{"project": folder, "preset"?: name} → {"document": id, "sections", "words", "warnings"}: compiles into a new unsaved document beside the open ones and shows it"#),
        CommandSpec::new("snapshot.take", "Take Snapshot", LOC, |_, v| {
            let pr = open(v)?;
            let id = section_id(&pr, v)?;
            let label: String = p::str(v, "label").unwrap_or("").chars().take(MAX_TEXT).collect();
            let s = snapshot::take(&pr, &id, &label).map_err(CmdError::Failed)?;
            Ok(json!({"at": s.at, "label": s.meta.label, "words": s.meta.words}))
        })
        .params(r#"{"project": folder, "id": section id, "label"?: string} → {"at": the snapshot's name, "label", "words"}"#)
        .pure(),
        CommandSpec::new("snapshot.list", "Snapshots", LOC, |_, v| {
            let pr = open(v)?;
            let id = section_id(&pr, v)?;
            let list = snapshot::list(&pr, &id).map_err(CmdError::Failed)?;
            Ok(json!({"snapshots": list.iter().map(|s| json!({"at": s.at, "label": s.meta.label, "words": s.meta.words})).collect::<Vec<_>>()}))
        })
        .params(r#"{"project": folder, "id": section id} → {"snapshots": [{"at", "label", "words"}]} (oldest first)"#)
        .pure(),
        CommandSpec::new("snapshot.restore", "Restore Snapshot", LOC, |_, v| {
            let pr = open(v)?;
            let id = section_id(&pr, v)?;
            let at = p::req_str(v, "at")?;
            let before = snapshot::restore(&pr, &id, at).map_err(CmdError::Failed)?;
            Ok(json!({"restored": at, "before": before.at}))
        })
        .params(r#"{"project": folder, "id": section id, "at": snapshot name} → {"restored", "before": the snapshot taken of the section first}"#)
        .pure(),
    ]
}

fn fail(e: String) -> CmdError {
    CmdError::Failed(e)
}

/// The project named by `"project"`.
fn open(v: &Value) -> Result<Project, CmdError> {
    let path = p::req_str(v, "project")?;
    if path.trim().is_empty() {
        return Err(CmdError::Params("`project` is empty".into()));
    }
    Project::open(Path::new(path)).map_err(fail)
}

/// `"id"`: an item of the binder.
fn item_id(pr: &Project, v: &Value) -> Result<String, CmdError> {
    let id = p::req_str(v, "id")?;
    if !valid_id(id) || binder::find(&pr.file.items, id).is_none() {
        return Err(CmdError::Params(format!("no item {:?}", id.chars().take(40).collect::<String>())));
    }
    Ok(id.to_string())
}

/// `"id"`: a section of the binder.
fn section_id(pr: &Project, v: &Value) -> Result<String, CmdError> {
    let id = item_id(pr, v)?;
    match binder::find(&pr.file.items, &id) {
        Some(i) if i.kind == Kind::Section => Ok(id),
        _ => Err(CmdError::Params(format!("{id:?} is a folder, not a section"))),
    }
}

fn text(v: &Value, k: &str) -> Result<Option<String>, CmdError> {
    match v.get(k) {
        None => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.chars().take(MAX_TEXT).collect())),
        Some(_) => Err(CmdError::Params(format!("`{k}` is text"))),
    }
}

/// A count that may be cleared: absent (unchanged), null (cleared) or a whole number.
fn count(v: &Value, k: &str) -> Result<Option<Option<u64>>, CmdError> {
    match v.get(k) {
        None => Ok(None),
        Some(Value::Null) => Ok(Some(None)),
        Some(n) => n.as_u64().map(|n| Some(Some(n))).ok_or_else(|| CmdError::Params(format!("`{k}` is a whole number or null"))),
    }
}

/// `"parent"`: a folder id, or the top level when absent or null.
fn parent(pr: &Project, v: &Value) -> Result<Option<String>, CmdError> {
    match v.get("parent") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(id)) => match binder::find(&pr.file.items, id) {
            Some(i) if i.kind == Kind::Folder => Ok(Some(id.clone())),
            Some(_) => Err(CmdError::Params(format!("{id:?} is a section; items go in folders"))),
            None => Err(CmdError::Params(format!("no folder {:?}", id.chars().take(40).collect::<String>()))),
        },
        Some(_) => Err(CmdError::Params("`parent` is a folder id or null".into())),
    }
}

fn index(v: &Value) -> Result<Option<usize>, CmdError> {
    match v.get("index") {
        None | Some(Value::Null) => Ok(None),
        Some(n) => {
            n.as_u64().map(|n| Some(usize::try_from(n).unwrap_or(usize::MAX))).ok_or_else(|| CmdError::Params("`index` is a whole number".into()))
        }
    }
}

/// An empty section document at `id`'s path.
fn new_section_file(pr: &Project, id: &str) -> Result<(), CmdError> {
    let path = pr.section_path(id).map_err(fail)?;
    // The project's base document: its heading styles carry the heading list, so a heading typed
    // with its number is numbered like the other sections'.
    let doc = crate::template::base_document(crate::template::chapter_word(pr)).map_err(fail)?;
    wordcraft_engine::io::save_path(&path, &doc).map_err(fail)
}

fn new(_: &mut Session, v: &Value) -> CmdResult {
    let path = p::req_str(v, "path")?;
    if path.trim().is_empty() {
        return Err(CmdError::Params("`path` is empty".into()));
    }
    let title: String = p::str(v, "title").unwrap_or("Untitled project").chars().take(MAX_TEXT).collect();
    let template = p::str(v, "template").unwrap_or("blank");
    if !crate::template::TEMPLATES.contains(&template) {
        return Err(CmdError::Params(format!("`template` is blank, thesis or book, not {:?}", template.chars().take(20).collect::<String>())));
    }
    let mut pr = Project::create(Path::new(path), &title).map_err(fail)?;
    if let Some(author) = text(v, "author")? {
        pr.file.author = author;
    }
    let first = if template == "blank" {
        let id = pr.file.new_id().map_err(fail)?;
        let mut item = Item::new(id.clone(), Kind::Section, "Untitled".into());
        item.file = Some(crate::project::file_name_for("Untitled", &pr.taken_files(None)));
        pr.file.items.push(item);
        new_section_file(&pr, &id)?;
        Some(id)
    } else {
        crate::template::fill(&mut pr, template).map_err(fail)?
    };
    pr.save().map_err(fail)?;
    let mut r = info(&pr)?;
    r["first"] = json!(first);
    Ok(r)
}

fn item_json(i: &Item, cache: &mut Cache, pr: &Project, changed: &mut bool, depth: usize) -> Value {
    let mut o = json!({
        "id": i.id,
        "kind": match i.kind { Kind::Folder => "folder", Kind::Section => "section" },
        "title": i.title,
        "synopsis": i.synopsis,
        "label": i.label,
        "status": i.status,
        "role": i.role,
        "target": i.target_words,
        "include": i.include,
        "numbering": i.numbering.as_ref().map(|n| json!({"zone": n.zone, "show": n.show, "startOn": n.start_on, "start": n.start, "counted": n.counted})),
    });
    match i.kind {
        Kind::Section => {
            o["words"] = json!(cache.words(pr, &i.id, changed));
            o["file"] = json!(pr.section_file(&i.id));
        }
        Kind::Folder if depth < crate::project::MAX_DEPTH => {
            o["children"] = Value::Array(i.children.iter().map(|c| item_json(c, cache, pr, changed, depth + 1)).collect());
        }
        Kind::Folder => {}
    }
    o
}

fn info(pr: &Project) -> CmdResult {
    let mut cache = Cache::load(pr);
    let mut changed = false;
    let items: Vec<Value> = pr.file.items.iter().map(|i| item_json(i, &mut cache, pr, &mut changed, 1)).collect();
    let words: u64 = binder::sections(&pr.file.items)
        .into_iter()
        .filter(|(_, on)| *on)
        .filter_map(|(i, _)| cache.words(pr, &i.id, &mut changed))
        .fold(0u64, u64::saturating_add);
    if changed {
        cache.save(pr);
    }
    let t = &pr.file.targets;
    Ok(json!({
        "project": pr.root.to_string_lossy(),
        "title": pr.file.title,
        "author": pr.file.author,
        "targets": {"projectWords": t.project_words, "sessionWords": t.session_words, "deadline": t.deadline},
        "recovered": pr.recovered,
        "words": words,
        "items": items,
    }))
}

fn set_metadata(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    if let Some(t) = text(v, "title")? {
        pr.file.title = t;
    }
    if let Some(a) = text(v, "author")? {
        pr.file.author = a;
    }
    if let Some(n) = count(v, "projectWords")? {
        pr.file.targets.project_words = n;
    }
    if let Some(n) = count(v, "sessionWords")? {
        pr.file.targets.session_words = n;
    }
    match v.get("deadline") {
        None => {}
        Some(Value::Null) => pr.file.targets.deadline = None,
        Some(Value::String(d)) => {
            chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").map_err(|_| CmdError::Params("`deadline` is a date, YYYY-MM-DD".into()))?;
            pr.file.targets.deadline = Some(d.clone());
        }
        Some(_) => return Err(CmdError::Params("`deadline` is a date, YYYY-MM-DD, or null".into())),
    }
    pr.save().map_err(fail)?;
    info(&pr)
}

fn add(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    let kind = match p::str(v, "kind").unwrap_or("section") {
        "section" => Kind::Section,
        "folder" => Kind::Folder,
        k => return Err(CmdError::Params(format!("`kind` is section or folder, not {:?}", k.chars().take(20).collect::<String>()))),
    };
    let title = text(v, "title")?.unwrap_or_else(|| if kind == Kind::Folder { "New folder".into() } else { "Untitled".into() });
    let parent = parent(&pr, v)?;
    if parent.as_deref() == Some(TRASH) {
        return Err(CmdError::Params("new items don't go in the Trash".into()));
    }
    let index = index(v)?;
    if binder::sections(&pr.file.items).len() >= crate::project::MAX_ITEMS {
        return Err(CmdError::Failed("the binder is full".into()));
    }
    let id = pr.file.new_id().map_err(fail)?;
    let mut item = Item::new(id.clone(), kind, title.clone());
    if let Some(role) = text(v, "role")? {
        item.role = role;
    }
    if kind == Kind::Section {
        item.file = Some(crate::project::file_name_for(&title, &pr.taken_files(None)));
    }
    binder::insert(&mut pr.file.items, parent.as_deref(), index, item).map_err(fail)?;
    // The file first: a binder entry never points at a file that wasn't made.
    if kind == Kind::Section {
        new_section_file(&pr, &id)?;
    }
    if let Err(e) = crate::project::validate(&pr.file) {
        return Err(CmdError::Failed(e));
    }
    pr.save().map_err(fail)?;
    Ok(json!({"id": id}))
}

fn move_item(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    let id = item_id(&pr, v)?;
    let parent = parent(&pr, v)?;
    let index = index(v)?;
    binder::move_item(&mut pr.file.items, &id, parent.as_deref(), index).map_err(CmdError::Params)?;
    pr.save().map_err(fail)?;
    Ok(json!({"id": id}))
}

fn set_meta(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    let id = item_id(&pr, v)?;
    let (title, synopsis, label, status, role) = (text(v, "title")?, text(v, "synopsis")?, text(v, "label")?, text(v, "status")?, text(v, "role")?);
    let target = count(v, "target")?;
    let include = match v.get("include") {
        None => None,
        Some(b) => Some(b.as_bool().ok_or_else(|| CmdError::Params("`include` is true or false".into()))?),
    };
    let numbering = numbering_rule(v)?;
    // A section's file follows its title: renamed on disk first, so the binder never points at a
    // file that isn't there.
    let new_file = match (&title, binder::find(&pr.file.items, &id)) {
        (Some(t), Some(i)) if i.kind == Kind::Section && &i.title != t => {
            let name = crate::project::file_name_for(t, &pr.taken_files(Some(&id)));
            let (from, to) = (pr.section_path(&id).map_err(fail)?, pr.root.join(crate::project::SECTIONS).join(&name));
            if from != to {
                wordcraft_engine::file_access::check_write(&to.to_string_lossy()).map_err(fail)?;
                if to.exists() && !name.eq_ignore_ascii_case(&pr.section_file(&id)) {
                    return Err(CmdError::Failed(format!("{name} is already in the sections folder")));
                }
                if from.exists() {
                    std::fs::rename(&from, &to).map_err(|e| CmdError::Failed(format!("renaming the section's file: {e}")))?;
                }
            }
            Some(name)
        }
        _ => None,
    };
    let item = binder::find_mut(&mut pr.file.items, &id).ok_or_else(|| CmdError::Params(format!("no item {id:?}")))?;
    if let Some(n) = numbering {
        item.numbering = n;
    }
    if let Some(f) = new_file {
        item.file = Some(f);
    }
    for (field, value) in
        [(&mut item.title, title), (&mut item.synopsis, synopsis), (&mut item.label, label), (&mut item.status, status), (&mut item.role, role)]
    {
        if let Some(x) = value {
            *field = x;
        }
    }
    if let Some(t) = target {
        item.target_words = t;
    }
    if let Some(b) = include {
        item.include = b;
    }
    pr.save().map_err(fail)?;
    Ok(json!({"id": id}))
}

/// `"numbering"`: absent (unchanged), null (cleared) or `{"zone"?, "show"?, "startOn"?}`, each
/// checked as compile reads it.
fn numbering_rule(v: &Value) -> Result<Option<Option<crate::project::NumberingRule>>, CmdError> {
    let n = match v.get("numbering") {
        None => return Ok(None),
        Some(Value::Null) => return Ok(Some(None)),
        Some(Value::Object(o)) => o,
        Some(_) => return Err(CmdError::Params("`numbering` is {\"zone\", \"show\", \"startOn\"} or null".into())),
    };
    let get = |k: &str| -> Result<Option<String>, CmdError> {
        match n.get(k) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) => Ok(Some(s.chars().take(40).collect())),
            Some(_) => Err(CmdError::Params(format!("`numbering.{k}` is text"))),
        }
    };
    let start = match n.get("start") {
        None | Some(Value::Null) => None,
        Some(x) => Some(
            x.as_u64()
                .and_then(|x| u32::try_from(x).ok())
                .filter(|x| *x <= crate::compile::MAX_PAGE_START)
                .ok_or_else(|| CmdError::Params(format!("`numbering.start` is a whole number up to {}", crate::compile::MAX_PAGE_START)))?,
        ),
    };
    let counted = match n.get("counted") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(_) => return Err(CmdError::Params("`numbering.counted` is true or false".into())),
    };
    let rule = crate::project::NumberingRule {
        zone: get("zone")?,
        show: get("show")?,
        start_on: get("startOn")?,
        start,
        counted,
        extra: toml::Table::new(),
    };
    crate::compile::Rule::default().with(&rule).map_err(CmdError::Params)?;
    Ok(Some(Some(rule)))
}

fn delete(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    let id = item_id(&pr, v)?;
    binder::trash(&mut pr.file.items, &id).map_err(CmdError::Params)?;
    pr.save().map_err(fail)?;
    Ok(json!({"id": id, "trash": TRASH}))
}

fn empty_trash(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    // The files, found while the items are still in the binder.
    let paths: Vec<std::path::PathBuf> = binder::find(&pr.file.items, TRASH)
        .map(|t| binder::sections(&t.children).into_iter().filter_map(|(i, _)| pr.section_path(&i.id).ok()).collect())
        .unwrap_or_default();
    let removed = binder::empty_trash(&mut pr.file.items);
    // The binder first: a failed save leaves every file where it was.
    pr.save().map_err(fail)?;
    for path in paths {
        if wordcraft_engine::file_access::check_write(&path.to_string_lossy()).is_ok() {
            let _ = std::fs::remove_file(path);
        }
    }
    Ok(json!({"removed": removed}))
}

fn open_item(s: &mut Session, v: &Value) -> CmdResult {
    let pr = open(v)?;
    let id = section_id(&pr, v)?;
    let path = pr.section_path(&id).map_err(fail)?;
    s.run("file.open", &json!({"path": path.to_string_lossy()}))
}

fn compile_run(_: &mut Session, v: &Value) -> CmdResult {
    let pr = open(v)?;
    let preset = crate::compile::preset(&pr, p::str(v, "preset")).map_err(fail)?;
    let path = p::req_str(v, "path")?;
    if path.trim().is_empty() {
        return Err(CmdError::Params("`path` is empty".into()));
    }
    let path = if Path::new(path).extension().is_some() { path.to_string() } else { format!("{path}.{}", preset.output) };
    let c = crate::compile::compile(&pr, &preset).map_err(fail)?;
    wordcraft_engine::io::save_path(Path::new(&path), &c.doc).map_err(fail)?;
    Ok(crate::compile::summary(&c, json!({"path": path, "preset": preset.name})))
}

fn compile_preview(s: &mut Session, v: &Value) -> CmdResult {
    let pr = open(v)?;
    let preset = crate::compile::preset(&pr, p::str(v, "preset")).map_err(fail)?;
    let c = crate::compile::compile(&pr, &preset).map_err(fail)?;
    let summary = crate::compile::summary(&c, json!({"preset": preset.name}));
    let id = s.add_document(c.doc);
    s.activate(id)?;
    let mut summary = summary;
    summary["document"] = json!(id);
    Ok(summary)
}

/// The heading level (0 = Heading 1) of a body paragraph, if it is a heading.
fn heading_level(doc: &wordcraft_doc::Document, b: &wordcraft_doc::Block) -> Option<u8> {
    let wordcraft_doc::Block::Para(p) = b else { return None };
    doc.styles.resolve_para(&p.props).outline_level.filter(|l| *l < 9)
}

/// A section file's outline as last read: the file's modification time and size, and the answer.
type Outlined = (std::time::SystemTime, u64, Value);

/// The outlines read so far, by file: a project's files are read once, and again only when they
/// change. Bounded: when it is full it is emptied.
static OUTLINES: std::sync::Mutex<Option<std::collections::HashMap<std::path::PathBuf, Outlined>>> = std::sync::Mutex::new(None);
const MAX_OUTLINES: usize = 4000;

/// `structure.outline` for the section file at `path`.
fn file_outline(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    let key = (meta.modified().ok()?, meta.len());
    if let Ok(guard) = OUTLINES.lock()
        && let Some((m, l, v)) = guard.as_ref().and_then(|c| c.get(path))
        && (*m, *l) == key
    {
        return Some(v.clone());
    }
    wordcraft_engine::file_access::check_read(&path.to_string_lossy()).ok()?;
    let doc = wordcraft_engine::io::open_path(path).ok()?;
    let v = Session::new(doc).run("structure.outline", &json!({})).ok()?;
    if let Ok(mut guard) = OUTLINES.lock() {
        let cache = guard.get_or_insert_with(Default::default);
        if cache.len() >= MAX_OUTLINES {
            cache.clear();
        }
        cache.insert(path.to_path_buf(), (key.0, key.1, v.clone()));
    }
    Some(v)
}

fn outline(_: &mut Session, v: &Value) -> CmdResult {
    let pr = open(v)?;
    let skip = p::str(v, "skip");
    let sections: Vec<Value> = binder::sections(&pr.file.items)
        .into_iter()
        .filter(|(_, on)| *on)
        .map(|(i, _)| {
            let generated = crate::compile::GENERATED.contains(&i.role.as_str());
            let outline = if generated || skip == Some(i.id.as_str()) { None } else { pr.section_path(&i.id).ok().and_then(|p| file_outline(&p)) };
            json!({"id": i.id, "title": i.title, "role": i.role, "outline": outline})
        })
        .collect();
    Ok(json!({"sections": sections}))
}

/// Most sections one import makes.
const MAX_IMPORT_SECTIONS: usize = 2000;

fn import(_: &mut Session, v: &Value) -> CmdResult {
    let mut pr = open(v)?;
    let path = p::req_str(v, "path")?;
    let deepest: u8 = match p::str(v, "split").unwrap_or("heading1") {
        "heading1" => 0,
        "heading2" => 1,
        "none" => u8::MAX,
        x => return Err(CmdError::Params(format!("`split` is heading1, heading2 or none, not {:?}", x.chars().take(20).collect::<String>()))),
    };
    let parent = parent(&pr, v)?;
    if parent.as_deref() == Some(TRASH) {
        return Err(CmdError::Params("imported sections don't go in the Trash".into()));
    }
    wordcraft_engine::file_access::check_read(path).map_err(fail)?;
    let doc = wordcraft_engine::io::open_path(Path::new(path)).map_err(fail)?;
    let stem = Path::new(path).file_stem().and_then(|s| s.to_str()).unwrap_or("Imported").to_string();
    // No heading at the split level: typed numbered headings become real ones first. Then what
    // else was typed as structure (headings with their numbers, appendices, "Figure 2.1: …"
    // captions) becomes the real thing, and floating pictures are set in line, where no later
    // edit can move them.
    let mut converted = 0usize;
    let mut s = Session::new(doc);
    if deepest != u8::MAX
        && !s.doc.body.iter().any(|b| heading_level(&s.doc, b).is_some_and(|l| l <= deepest))
        && let Ok(r) = s.run("numbering.convert", &json!({}))
    {
        converted = r["converted"].as_array().map_or(0, Vec::len);
    }
    if let Ok(r) = s.run("structure.convert", &json!({})) {
        converted = converted.saturating_add(r["converted"].as_u64().and_then(|n| usize::try_from(n).ok()).unwrap_or(0));
    }
    let settled = s.run("structure.settleFigures", &json!({})).ok().and_then(|r| r["settled"].as_u64()).unwrap_or(0);
    let doc = s.doc.clone();
    // The chunks: (title, blocks), split before each heading at the level or above.
    let mut chunks: Vec<(String, wordcraft_doc::Blocks)> = Vec::new();
    for b in &doc.body {
        let starts = deepest != u8::MAX && heading_level(&doc, b).is_some_and(|l| l <= deepest);
        if starts || chunks.is_empty() {
            let title = if starts {
                b.as_para().map(|p| p.plain_text().trim().chars().take(200).collect::<String>()).unwrap_or_default()
            } else {
                stem.clone()
            };
            chunks.push((if title.is_empty() { "Untitled".into() } else { title }, Vec::new()));
        }
        if let Some((_, blocks)) = chunks.last_mut() {
            blocks.push(b.clone());
        }
    }
    // Text before the first heading that is only empty paragraphs isn't a section.
    chunks.retain(|(_, blocks)| blocks.iter().any(|b| b.as_para().is_none_or(|p| !p.plain_text().trim().is_empty() || !p.objects.is_empty())));
    if chunks.len() > MAX_IMPORT_SECTIONS {
        return Err(CmdError::Failed(format!(
            "the document would make {} sections; at most {MAX_IMPORT_SECTIONS} are imported at once",
            chunks.len()
        )));
    }
    if binder::sections(&pr.file.items).len().saturating_add(chunks.len()) > crate::project::MAX_ITEMS {
        return Err(CmdError::Failed("the binder would be too full".into()));
    }
    let mut ids = Vec::new();
    for (title, blocks) in chunks {
        let id = pr.file.new_id().map_err(fail)?;
        let mut item = Item::new(id.clone(), Kind::Section, title.clone());
        item.file = Some(crate::project::file_name_for(&title, &pr.taken_files(None)));
        binder::insert(&mut pr.file.items, parent.as_deref(), None, item).map_err(fail)?;
        // The whole document's styles, lists, notes and pictures, with this part's text.
        let mut part = doc.clone();
        part.body = blocks;
        wordcraft_engine::io::save_path(&pr.section_path(&id).map_err(fail)?, &part).map_err(fail)?;
        ids.push(id);
    }
    pr.save().map_err(fail)?;
    Ok(json!({"ids": ids, "converted": converted, "settled": settled}))
}
