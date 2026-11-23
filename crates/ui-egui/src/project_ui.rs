//! Thesis mode: a manuscript project (see `docs/manuscript-design.md` and the `manuscript`
//! crate) open in the window. The Project tab makes and opens projects and adds sections; the
//! Binder pane on the left shows the project's folders and sections with their word counts,
//! opens a section when it is clicked, and renames, moves, deletes and snapshots items. Each
//! section is an ordinary document: it opens beside the others, and the one being left is saved
//! first, so nothing typed is lost moving around the binder.
//!
//! The UI commands (`ui.project.*`) pass the project's folder to the manuscript commands
//! (`project.*`, `binder.*`, `snapshot.*`), which do the work.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use egui::Ui;
use serde_json::{Value, json};

use crate::WordApp;
use crate::file_dialogs::{AfterPick, Asked, FileDialogRequest};
use crate::theme::{regular, semibold};

/// Seconds between refreshes of the binder's word counts while a project is open.
const REFRESH_SECS: f64 = 3.0;

/// The project open in the window.
pub struct ProjectUi {
    /// The project folder.
    pub root: String,
    /// `project.info`'s last answer.
    pub info: Value,
    /// The item picked in the binder (what Rename, Delete, Move and New act on).
    pub selected: Option<String>,
    /// Whether the Binder pane is shown.
    pub binder: bool,
    /// When `info` was read (egui time).
    refreshed: f64,
    /// An item being renamed, and the new title typed so far.
    pub(crate) renaming: Option<(String, String)>,
    /// Folders shown closed.
    collapsed: HashSet<String>,
    /// The last thing that went wrong, shown in the pane.
    pub error: Option<String>,
    /// `project.outline`'s last answer: every section's headings, figures and tables, but the
    /// open section's (that one is read from the document as it is being written).
    pub outline: Value,
    /// What the pane lists: "binder", "outline", "figures", "tables" or "appendices".
    pub view: String,
}

impl ProjectUi {
    fn new(root: String, info: Value) -> Self {
        ProjectUi {
            root,
            info,
            selected: None,
            binder: true,
            refreshed: f64::NEG_INFINITY,
            renaming: None,
            collapsed: HashSet::new(),
            error: None,
            outline: Value::Null,
            view: "binder".into(),
        }
    }
}

/// Run manuscript command `id` on the open project (its folder added as `"project"`), then read
/// the binder again.
pub(crate) fn project_run(app: &mut WordApp, id: &str, params: Value) -> Result<Value, String> {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else { return Err(tl!("No project is open.").into()) };
    let mut params = if params.is_object() { params } else { json!({}) };
    params["project"] = json!(root);
    let r = app.session.run(id, &params).map_err(|e| e.to_string());
    refresh(app);
    if let (Err(e), Some(p)) = (&r, app.project.as_mut()) {
        p.error = Some(e.clone());
    }
    r
}

/// Read the binder again (`project.info`).
pub(crate) fn refresh(app: &mut WordApp) {
    let Some(root) = app.project.as_ref().map(|p| p.root.clone()) else { return };
    let now = app.ctx.as_ref().map_or(0.0, |c| c.input(|i| i.time));
    // The structure lists are read only while one of them is shown.
    let listing = app.project.as_ref().is_some_and(|p| p.view != "binder");
    if listing {
        let skip = current_section(app);
        if let Ok(o) = app.session.run("project.outline", &json!({"project": root, "skip": skip}))
            && let Some(p) = app.project.as_mut()
        {
            p.outline = o;
        }
    }
    match app.session.run("project.info", &json!({"project": root})) {
        Ok(info) => {
            if let Some(p) = app.project.as_mut() {
                p.info = info;
                p.refreshed = now;
            }
        }
        Err(e) => {
            if let Some(p) = app.project.as_mut() {
                p.error = Some(e.to_string());
                p.refreshed = now;
            }
        }
    }
}

/// The project folder for a picked path: the folder itself, or the folder of a picked
/// `project.toml`.
fn project_root(picked: &str) -> PathBuf {
    let p = Path::new(picked);
    if p.file_name().is_some_and(|n| n == wordcraft_manuscript::project::PROJECT_FILE) {
        p.parent().map(Path::to_path_buf).unwrap_or_default()
    } else {
        p.to_path_buf()
    }
}

/// Whether `path` names a manuscript project: a folder holding `project.toml`, or that file.
pub fn is_project_path(path: &str) -> bool {
    let root = project_root(path);
    root.join(wordcraft_manuscript::project::PROJECT_FILE).is_file()
}

/// Open the project at `path` (its folder or its `project.toml`) and its first section.
pub(crate) fn open_project(app: &mut WordApp, path: &str) -> Result<Value, String> {
    let root = project_root(path).to_string_lossy().to_string();
    let info = app.session.run("project.info", &json!({"project": root})).map_err(|e| e.to_string())?;
    app.project = Some(ProjectUi::new(root, info.clone()));
    if let Some(p) = app.project.as_mut()
        && let Some(why) = info["recovered"].as_str()
    {
        p.error = Some(crate::i18n::fmt(tl!("The project file couldn't be read ({why}): every section is listed instead."), &[("why", why)]));
    }
    if let Some(first) = first_section(&info["items"]) {
        open_section(app, &first)?;
    }
    app.ui.backstage = false;
    // Thesis mode starts on its own tab.
    app.ui.tab = "Project".into();
    Ok(info)
}

/// Open the project that was open when WordCraft last closed, if it is still there. Returns
/// whether one opened.
pub fn reopen_last_project(app: &mut WordApp) -> bool {
    let Some(path) = app.ui.last_project.clone() else { return false };
    is_project_path(&path) && open_project(app, &path).is_ok()
}

/// Document extensions a Save dialog may have added to a new project's name.
const DOCUMENT_EXTENSIONS: [&str; 8] = ["docx", "doc", "dotx", "odt", "rtf", "txt", "pdf", "md"];

/// Make a project at `path` (`.wcproj` added when missing) from `template` (blank, thesis or
/// book), open it and the section writing starts in.
pub(crate) fn new_project(app: &mut WordApp, path: &str, template: &str) -> Result<Value, String> {
    // A Save dialog adds its own extension ("My Thesis.wcproj.docx"): that goes first.
    let mut path = path.to_string();
    while let Some(ext) = Path::new(&path).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase) {
        if !DOCUMENT_EXTENSIONS.contains(&ext.as_str()) {
            break;
        }
        path.truncate(path.len() - ext.len() - 1);
    }
    let path = if path.ends_with(".wcproj") { path } else { format!("{path}.wcproj") };
    let title = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled project").to_string();
    let made = app.session.run("project.new", &json!({"path": path, "title": title, "template": template})).map_err(|e| e.to_string())?;
    let r = open_project(app, &path)?;
    // A template's project starts in its first chapter, not on its title page.
    if template != "blank"
        && let Some(first) = made["first"].as_str()
    {
        open_section(app, first)?;
    }
    Ok(r)
}

/// The first section of a binder listing, depth first (the Trash's aren't).
fn first_section(items: &Value) -> Option<String> {
    for i in items.as_array().into_iter().flatten() {
        if i["id"] == wordcraft_manuscript::project::TRASH {
            continue;
        }
        if i["kind"] == "section" {
            return i["id"].as_str().map(str::to_string);
        }
        if let Some(id) = first_section(&i["children"]) {
            return Some(id);
        }
    }
    None
}

/// Every section of a binder listing with its file name, depth first.
fn section_files(items: &Value, out: &mut Vec<(String, String)>, depth: usize) {
    if depth > wordcraft_manuscript::project::MAX_DEPTH {
        return;
    }
    for i in items.as_array().into_iter().flatten() {
        if i["kind"] == "section"
            && let Some(id) = i["id"].as_str()
        {
            let file = i["file"].as_str().map_or_else(|| format!("{id}.docx"), str::to_string);
            out.push((id.to_string(), file));
        }
        section_files(&i["children"], out, depth + 1);
    }
}

/// The file of section `id` of the open project (named by its title; `<id>.docx` in older
/// projects).
fn section_path(app: &WordApp, id: &str) -> Option<PathBuf> {
    let p = app.project.as_ref()?;
    let mut all = Vec::new();
    section_files(&p.info["items"], &mut all, 0);
    let (_, file) = all.into_iter().find(|(i, _)| i == id)?;
    wordcraft_manuscript::project::valid_file_name(&file).then(|| Path::new(&p.root).join(wordcraft_manuscript::project::SECTIONS).join(file))
}

/// The section of the open project a document's file is, if it is one.
fn section_of(app: &WordApp, path: &Path) -> Option<String> {
    let p = app.project.as_ref()?;
    let dir = Path::new(&p.root).join(wordcraft_manuscript::project::SECTIONS);
    if path.parent()? != dir {
        return None;
    }
    let name = path.file_name()?.to_str()?;
    let mut all = Vec::new();
    section_files(&p.info["items"], &mut all, 0);
    all.into_iter().find(|(_, f)| f.eq_ignore_ascii_case(name)).map(|(id, _)| id)
}

/// The section of the open project shown in the window, if it is one.
pub(crate) fn current_section(app: &WordApp) -> Option<String> {
    section_of(app, app.session.path.as_ref()?)
}

/// Close the open documents that are other sections of the project, saved: thesis mode edits
/// one section at a time in the window, as moving through a binder does.
fn close_other_sections(app: &mut WordApp) {
    let others: Vec<u64> = app
        .session
        .documents()
        .into_iter()
        .filter(|d| !d.active && !d.dirty && d.path.as_deref().is_some_and(|path| section_of(app, path).is_some()))
        .map(|d| d.id)
        .collect();
    for id in others {
        if let Err(e) = app.session.close_document(id) {
            log::warn!("closing section document {id}: {e}");
        }
    }
    app.sync_document();
}

/// Show section `id`: the section being left is saved first when it has changes, then the
/// section opens (beside the others; one already open is shown).
pub(crate) fn open_section(app: &mut WordApp, id: &str) -> Result<Value, String> {
    let path = section_path(app, id).ok_or_else(|| tl!("That isn't a section of the project.").to_string())?;
    if current_section(app).is_some() && app.document_unsaved() {
        app.run("file.save", json!({}))?;
    }
    // Opening a document goes to Home; thesis mode keeps the tab the writer is on.
    let tab = app.ui.tab.clone();
    let r = app.open_file(&path.to_string_lossy(), None);
    if r.is_ok() {
        close_other_sections(app);
        app.ui.tab = tab;
    }
    if let Some(p) = app.project.as_mut() {
        p.selected = Some(id.to_string());
    }
    r
}

/// The Project tab's and the Binder pane's commands (`ui.project.*`); None for any other id.
pub(crate) fn command(app: &mut WordApp, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if !id.starts_with("ui.project.") {
        return None;
    }
    let path = params.get("path").and_then(Value::as_str).map(str::to_string);
    Some(match id {
        "ui.project.new" => {
            let template = params.get("template").and_then(Value::as_str).filter(|t| matches!(*t, "thesis" | "book")).unwrap_or("blank");
            match path {
                Some(p) => new_project(app, &p, template),
                None => {
                    let name = if template == "book" { tl!("My Book") } else { tl!("My Thesis") };
                    asked(
                        app.ask_file(
                            FileDialogRequest::Save { name: name.to_string() + ".wcproj", dir: None },
                            AfterPick::NewProject(template.into()),
                        ),
                    )
                }
            }
        }
        // Show a place in the project: the section opens when it isn't the one shown.
        "ui.project.goto" => {
            let block = params.get("block").and_then(Value::as_u64).unwrap_or(0);
            match params.get("id").and_then(Value::as_str) {
                Some(sid) => goto(app, sid, block),
                None => Err("`id` is required".into()),
            }
        }
        "ui.project.view" => match app.project.as_mut() {
            Some(p) => {
                let view = params.get("view").and_then(Value::as_str).unwrap_or("binder");
                p.view = if VIEWS.iter().any(|(v, _)| *v == view) { view.into() } else { "binder".into() };
                p.refreshed = f64::NEG_INFINITY;
                Ok(json!({"view": p.view}))
            }
            None => Err(tl!("No project is open.").into()),
        },
        "ui.project.open" => match path {
            Some(p) => open_project(app, &p),
            None => asked(app.ask_file(FileDialogRequest::Open { purpose: "project".into() }, AfterPick::OpenProject)),
        },
        "ui.project.import" => match path {
            Some(p) => import_document(app, &p),
            None => asked(app.ask_file(FileDialogRequest::Open { purpose: "document".into() }, AfterPick::ImportDocument)),
        },
        "ui.project.close" => {
            app.project = None;
            Ok(json!({"closed": true}))
        }
        "ui.project.binder" => match app.project.as_mut() {
            Some(p) => {
                p.binder = params.get("value").and_then(Value::as_bool).unwrap_or(!p.binder);
                Ok(json!({"binder": p.binder}))
            }
            None => Err(tl!("No project is open.").into()),
        },
        "ui.project.add" => add(app, params.get("kind").and_then(Value::as_str).unwrap_or("section")),
        "ui.project.snapshot" => {
            let target = current_section(app).or_else(|| app.project.as_ref().and_then(|p| p.selected.clone()));
            match target {
                Some(sid) => {
                    // The section as it is now: unsaved changes are saved first.
                    if current_section(app).as_deref() == Some(sid.as_str()) && app.document_unsaved() {
                        let _ = app.run("file.save", json!({}));
                    }
                    let label = params.get("label").and_then(Value::as_str).unwrap_or("");
                    let r = project_run(app, "snapshot.take", json!({"id": sid, "label": label}));
                    if r.is_ok() {
                        app.status(tl!("Snapshot taken.").to_string());
                    }
                    r
                }
                None => Err(tl!("Pick a section to take a snapshot of.").into()),
            }
        }
        "ui.project.check" => {
            save_open_section(app);
            let r = project_run(app, "compile.check", json!({"preset": params.get("preset")}));
            if let Ok(v) = &r {
                let problems: Vec<String> = v["problems"]
                    .as_array()
                    .map(|a| {
                        a.iter().map(|p| format!("{}{}", if p["error"] == true { "✗ " } else { "• " }, p["message"].as_str().unwrap_or(""))).collect()
                    })
                    .unwrap_or_default();
                let msg = if problems.is_empty() {
                    tl!("Ready to submit: nothing to fix.").to_string()
                } else if v["ok"] == true {
                    crate::i18n::fmt(tl!("{n} things to look at before submitting"), &[("n", &problems.len().to_string())])
                } else {
                    crate::i18n::fmt(tl!("Not ready: {n} problems to fix"), &[("n", &problems.len().to_string())])
                };
                app.status(msg);
                if let Some(p) = app.project.as_mut() {
                    p.error = (!problems.is_empty()).then(|| problems.join("\n"));
                }
            }
            r
        }
        "ui.project.preview" => {
            save_open_section(app);
            let r = project_run(app, "compile.preview", json!({"preset": params.get("preset")}));
            app.sync_document();
            report(app, r)
        }
        "ui.project.compile" => {
            let format = params
                .get("format")
                .and_then(Value::as_str)
                .filter(|f| f.chars().all(|c| c.is_ascii_alphanumeric()) && !f.is_empty())
                .unwrap_or("pdf")
                .to_string();
            match path {
                Some(p) => compile_to(app, &p, params.get("preset").cloned()),
                None => {
                    let title = app
                        .project
                        .as_ref()
                        .and_then(|p| p.info["title"].as_str().map(str::to_string))
                        .unwrap_or_else(|| tl!("Manuscript").to_string());
                    asked(app.ask_file(FileDialogRequest::Save { name: format!("{title}.{format}"), dir: None }, AfterPick::CompileProject))
                }
            }
        }
        "ui.project.openItem" => match params.get("id").and_then(Value::as_str) {
            Some(sid) => open_section(app, sid),
            None => Err("`id` is required".into()),
        },
        _ => Err(format!("unknown command {id}")),
    })
}

/// The lists the Binder pane shows.
const VIEWS: [(&str, &str); 5] =
    [("binder", "Binder"), ("outline", "Outline"), ("figures", "Figures"), ("tables", "Tables"), ("appendices", "Appendices")];

/// Put the caret at the start of body block `block` of section `id`, opening the section when
/// another is shown, and bring it into view.
fn goto(app: &mut WordApp, id: &str, block: u64) -> Result<Value, String> {
    if current_section(app).as_deref() != Some(id) {
        open_section(app, id)?;
    }
    let last = app.session.doc.body.len().saturating_sub(1) as u64;
    let pos = wordcraft_doc::Pos::body(usize::try_from(block.min(last)).unwrap_or(0), 0);
    let r = app.run("caret.set", json!({"pos": pos}));
    app.canvas.nav_to_caret = true;
    app.canvas.scroll_to_caret = false;
    app.canvas.want_focus = true;
    r
}

/// One line of a structure list: the section it is in, its block there, how deep it is indented
/// and what it says.
struct Entry {
    section: String,
    block: u64,
    level: u8,
    text: String,
    /// Typed as structure but still plain text: Detect Structure makes it real.
    typed: bool,
}

/// The lines of structure list `view` for the whole project, in binder order: the open section's
/// from the document being written, the others' from their files.
fn entries(app: &WordApp, view: &str) -> Vec<Entry> {
    let Some(p) = app.project.as_ref() else { return Vec::new() };
    let current = current_section(app);
    let live = current.as_ref().map(|_| wordcraft_engine::cmd::structure::outline(&app.session));
    let mut out = Vec::new();
    for sec in p.outline["sections"].as_array().into_iter().flatten() {
        let Some(id) = sec["id"].as_str() else { continue };
        let o = if current.as_deref() == Some(id) { live.as_ref() } else { Some(&sec["outline"]) };
        let Some(o) = o.filter(|o| o.is_object()) else { continue };
        let block = |v: &Value| v["path"].get(0).and_then(Value::as_u64).unwrap_or(0);
        match view {
            "figures" | "tables" => {
                for c in o[view].as_array().into_iter().flatten() {
                    let label = c["label"].as_str().unwrap_or("");
                    let text = c["title"].as_str().or(c["text"].as_str()).unwrap_or("");
                    let text = if text.is_empty() { label.to_string() } else { format!("{label}  {text}") };
                    out.push(Entry { section: id.into(), block: block(c), level: 0, text, typed: false });
                }
                let want = if view == "figures" { "Fig" } else { "Tab" };
                for t in o["typed"].as_array().into_iter().flatten().filter(|t| t["kind"] == "caption") {
                    let text = t["text"].as_str().unwrap_or("");
                    if text.starts_with(want) {
                        out.push(Entry { section: id.into(), block: block(t), level: 0, text: text.into(), typed: true });
                    }
                }
            }
            _ => {
                let mut in_appendix = false;
                for h in o["headings"].as_array().into_iter().flatten() {
                    let level = u8::try_from(h["level"].as_u64().unwrap_or(1)).unwrap_or(1).clamp(1, 9);
                    let appendix = h["appendix"] == true;
                    if level == 1 {
                        in_appendix = appendix;
                    }
                    if view == "appendices" && !(appendix || in_appendix) {
                        continue;
                    }
                    let text = h["text"].as_str().unwrap_or("");
                    let text = match h["number"].as_str() {
                        Some(n) => format!("{n}  {text}"),
                        None => text.to_string(),
                    };
                    out.push(Entry { section: id.into(), block: block(h), level: level - 1, text, typed: false });
                }
                if view == "outline" {
                    for t in o["typed"].as_array().into_iter().flatten().filter(|t| t["kind"] != "caption") {
                        out.push(Entry { section: id.into(), block: block(t), level: 0, text: t["text"].as_str().unwrap_or("").into(), typed: true });
                    }
                }
            }
        }
    }
    out
}

/// A structure list in the Binder pane: every heading, figure, table or appendix of the project;
/// a click goes there.
fn structure_list(app: &mut WordApp, ui: &mut Ui, view: &str) {
    let list = entries(app, view);
    let current = current_section(app);
    let caret = app.session.sel.focus.path.0.first().copied().unwrap_or(0) as u64;
    let mut go: Option<(String, u64)> = None;
    let mut detect = false;
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            let hint = match view {
                "figures" => tl!("No figures yet.\n\nType a caption such as “Figure 2.1: Cell counts” on a line of its own and press Enter: it is numbered, listed here and kept with its picture."),
                "tables" => tl!("No tables yet.\n\nType a caption such as “Table 3.1: Reagents” on a line of its own and press Enter: it is numbered and listed here."),
                "appendices" => tl!("No appendices yet.\n\nType “Appendix A” on a line of its own and press Enter."),
                _ => tl!("No headings yet.\n\nType a heading with its number, such as “2.1 Methods”, and press Enter: it becomes a numbered heading and is listed here."),
            };
            ui.label(egui::RichText::new(hint).weak());
        }
        // The line the caret is under: the last one at or before it in the open section.
        let here = list.iter().rposition(|e| current.as_deref() == Some(e.section.as_str()) && e.block <= caret);
        for (i, e) in list.iter().enumerate() {
            crate::widgets::row(ui, |ui| {
                ui.add_space(f32::from(e.level) * 12.0);
                let mut text = egui::RichText::new(e.text.as_str()).font(if e.level == 0 && view != "figures" && view != "tables" { semibold(12.5) } else { regular(12.5) });
                if e.typed {
                    text = text.italics().weak();
                }
                let width = ui.available_width();
                let r = ui.add(egui::Button::selectable(here == Some(i), (text, egui::Atom::grow())).wrap().min_size(egui::vec2(width, ui.spacing().interact_size.y)));
                let r = if e.typed { r.on_hover_text(tl!("Typed as text. Detect Structure makes it a real heading or caption.")) } else { r };
                if r.clicked() {
                    go = Some((e.section.clone(), e.block));
                }
            });
        }
        if list.iter().any(|e| e.typed) {
            ui.add_space(6.0);
            detect = ui.button(tl!("Detect Structure")).on_hover_text(tl!("Make the lines typed as headings and captions real ones in this section")).clicked();
        }
    });
    if let Some((id, block)) = go {
        let r = goto(app, &id, block);
        if let (Err(e), Some(p)) = (r, app.project.as_mut()) {
            p.error = Some(e);
        }
    }
    if detect {
        let _ = app.run("structure.convert", json!({}));
    }
}

/// Import the document at `path` into the project, split at its headings (into the picked
/// folder, else at the end), and show its first new section.
pub(crate) fn import_document(app: &mut WordApp, path: &str) -> Result<Value, String> {
    let parent = app.project.as_ref().and_then(|p| {
        let id = p.selected.as_deref()?;
        locate(&p.info["items"], id, None)
            .filter(|(i, _, _)| i["kind"] == "folder" && i["id"] != wordcraft_manuscript::project::TRASH)
            .map(|_| id.to_string())
    });
    let r = project_run(app, "binder.import", json!({"path": path, "parent": parent}))?;
    let n = r["ids"].as_array().map_or(0, Vec::len);
    app.status(crate::i18n::fmt(tl!("Imported {n} sections"), &[("n", &n.to_string())]));
    if let Some(first) = r["ids"].get(0).and_then(Value::as_str) {
        open_section(app, first)?;
    }
    Ok(r)
}

/// Save the section open in the window when it has changes, so compiling reads what's there.
fn save_open_section(app: &mut WordApp) {
    if current_section(app).is_some() && app.document_unsaved() {
        let _ = app.run("file.save", json!({}));
    }
}

/// Compile the open project to `path` (its extension picks the format).
pub(crate) fn compile_to(app: &mut WordApp, path: &str, preset: Option<Value>) -> Result<Value, String> {
    save_open_section(app);
    let r = project_run(app, "compile.run", json!({"path": path, "preset": preset}));
    report(app, r)
}

/// Say how a compile went: on the status line, and its warnings in the Binder.
fn report(app: &mut WordApp, r: Result<Value, String>) -> Result<Value, String> {
    if let Ok(v) = &r {
        let n = v["sections"].as_array().map_or(0, Vec::len);
        let warnings: Vec<String> =
            v["warnings"].as_array().map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default();
        let msg = match v["path"].as_str() {
            Some(p) => crate::i18n::fmt(tl!("Compiled {n} sections to {path}"), &[("n", &n.to_string()), ("path", p)]),
            None => crate::i18n::fmt(tl!("Compiled {n} sections"), &[("n", &n.to_string())]),
        };
        app.status(msg);
        if let Some(p) = app.project.as_mut() {
            p.error = (!warnings.is_empty()).then(|| warnings.join("\n"));
        }
    }
    r
}

fn asked(a: Asked) -> Result<Value, String> {
    match a {
        Asked::Busy => Err(tl!("A file dialog is already open.").into()),
        Asked::Pending => Ok(json!({"pending": "fileDialog"})),
        Asked::Done(r) => r,
    }
}

/// New Section / New Folder: inside the picked folder, or after the picked item at its level,
/// else at the end. A new section opens.
fn add(app: &mut WordApp, kind: &str) -> Result<Value, String> {
    let Some(p) = app.project.as_ref() else { return Err(tl!("No project is open.").into()) };
    let picked = p.selected.clone();
    let (parent, index) = match picked.as_deref().and_then(|id| locate(&p.info["items"], id, None)) {
        Some((item, _, _)) if item["kind"] == "folder" && item["id"] != wordcraft_manuscript::project::TRASH => {
            (item["id"].as_str().map(str::to_string), None)
        }
        Some((item, parent, at)) if !in_trash(&p.info["items"], item["id"].as_str().unwrap_or("")) => (parent, Some(at + 1)),
        _ => (None, None),
    };
    let title = if kind == "folder" { tl!("New folder").to_string() } else { tl!("Untitled").to_string() };
    let r = project_run(app, "binder.add", json!({"kind": kind, "title": title, "parent": parent, "index": index}))?;
    let id = r["id"].as_str().unwrap_or("").to_string();
    if kind == "section" {
        open_section(app, &id)?;
    }
    if let Some(p) = app.project.as_mut() {
        p.selected = Some(id.clone());
        p.renaming = Some((id, title));
    }
    Ok(r)
}

/// Item `id` in a binder listing: the item, its folder's id (None at the top level) and its
/// index there.
fn locate<'a>(items: &'a Value, id: &str, parent: Option<&str>) -> Option<(&'a Value, Option<String>, usize)> {
    let list = items.as_array()?;
    for (k, i) in list.iter().enumerate() {
        if i["id"] == id {
            return Some((i, parent.map(str::to_string), k));
        }
        if let Some(found) = locate(&i["children"], id, i["id"].as_str()) {
            return Some(found);
        }
    }
    None
}

fn in_trash(items: &Value, id: &str) -> bool {
    locate(items, wordcraft_manuscript::project::TRASH, None).is_some_and(|(t, _, _)| locate(&t["children"], id, None).is_some())
}

/// What a click in the binder asks for.
enum Do {
    Open(String),
    Pick(String),
    Toggle(String),
    Rename(String, String),
    StartRename(String),
    Move(String, Option<String>, usize),
    Delete(String),
    EmptyTrash,
    Add(&'static str),
    Snapshot,
    /// Set an item's page numbering (`binder.setMeta`'s `numbering`; null clears it).
    Numbering(String, Value),
}

/// The Binder pane.
pub fn binder(app: &mut WordApp, ui: &mut Ui) {
    // Word counts change as sections are saved: read the binder again now and then.
    let now = ui.input(|i| i.time);
    if app.project.as_ref().is_some_and(|p| now - p.refreshed > REFRESH_SECS) {
        refresh(app);
    }
    let current = current_section(app);
    let Some(p) = app.project.as_ref() else {
        ui.label(egui::RichText::new(tl!("No project is open.")).weak());
        return;
    };
    let info = p.info.clone();
    let selected = p.selected.clone();
    let renaming = p.renaming.clone();
    let collapsed = p.collapsed.clone();
    let error = p.error.clone();
    let mut todo: Option<Do> = None;
    ui.label(egui::RichText::new(info["title"].as_str().unwrap_or("")).font(semibold(14.0)));
    let words = info["words"].as_u64().unwrap_or(0);
    let total = match info["targets"]["projectWords"].as_u64() {
        Some(t) => crate::i18n::fmt(tl!("{n} of {t} words"), &[("n", &words.to_string()), ("t", &t.to_string())]),
        None => crate::i18n::fmt(tl!("{n} words"), &[("n", &words.to_string())]),
    };
    ui.label(egui::RichText::new(total).small().weak());
    if let Some(e) = &error {
        ui.colored_label(ui.visuals().warn_fg_color, egui::RichText::new(e.as_str()).small());
    }
    ui.add_space(4.0);
    crate::widgets::row(ui, |ui| {
        if ui.small_button(tl!("+ Section")).on_hover_text(tl!("New section")).clicked() {
            todo = Some(Do::Add("section"));
        }
        if ui.small_button(tl!("+ Folder")).on_hover_text(tl!("New folder")).clicked() {
            todo = Some(Do::Add("folder"));
        }
        if ui.small_button(tl!("Snapshot")).on_hover_text(tl!("Save a copy of this section as it is now")).clicked() {
            todo = Some(Do::Snapshot);
        }
    });
    // What the pane lists: the binder, or the project's structure.
    let view = p.view.clone();
    let mut pick: Option<&str> = None;
    ui.horizontal_wrapped(|ui| {
        for (k, label) in VIEWS {
            if ui.selectable_label(view == k, tl!(label)).clicked() {
                pick = Some(k);
            }
        }
    });
    ui.separator();
    if let Some(k) = pick {
        let _ = app.run("ui.project.view", json!({"view": k}));
        return;
    }
    if view != "binder" {
        structure_list(app, ui, &view);
        return;
    }
    egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
        tree(
            ui,
            &info["items"],
            None,
            0,
            &Ctx { current: current.as_deref(), selected: selected.as_deref(), renaming: renaming.as_ref(), collapsed: &collapsed },
            &mut todo,
        );
    });
    let Some(todo) = todo else { return };
    let r = match todo {
        Do::Open(id) => open_section(app, &id),
        Do::Pick(id) => {
            if let Some(p) = app.project.as_mut() {
                p.selected = Some(id);
            }
            Ok(Value::Null)
        }
        Do::Toggle(id) => {
            if let Some(p) = app.project.as_mut()
                && !p.collapsed.remove(&id)
            {
                p.collapsed.insert(id);
            }
            Ok(Value::Null)
        }
        Do::StartRename(id) => {
            let title = locate(&info["items"], &id, None).and_then(|(i, _, _)| i["title"].as_str().map(str::to_string)).unwrap_or_default();
            if let Some(p) = app.project.as_mut() {
                p.renaming = Some((id, title));
            }
            Ok(Value::Null)
        }
        Do::Rename(id, title) => {
            if let Some(p) = app.project.as_mut() {
                p.renaming = None;
            }
            project_run(app, "binder.rename", json!({"id": id, "title": title}))
        }
        Do::Move(id, parent, index) => project_run(app, "binder.move", json!({"id": id, "parent": parent, "index": index})),
        Do::Delete(id) => project_run(app, "binder.delete", json!({"id": id})),
        Do::EmptyTrash => project_run(app, "binder.emptyTrash", json!({})),
        Do::Add(kind) => add(app, kind),
        Do::Snapshot => app.run("ui.project.snapshot", json!({})),
        Do::Numbering(id, rule) => project_run(app, "binder.setMeta", json!({"id": id, "numbering": rule})),
    };
    match r {
        Ok(_) => {
            if let Some(p) = app.project.as_mut() {
                p.error = None;
            }
        }
        Err(e) => {
            if let Some(p) = app.project.as_mut() {
                p.error = Some(e);
            }
        }
    }
}

/// `rule` (an item's `numbering`, or null) with `key` set to `value`, or taken out when `value`
/// is null; null when nothing is left set.
fn with_rule(rule: &Value, key: &str, value: Value) -> Value {
    let mut o = rule.as_object().cloned().unwrap_or_default();
    o.retain(|_, v| !v.is_null());
    if value.is_null() {
        o.remove(key);
    } else {
        o.insert(key.to_string(), value);
    }
    if o.is_empty() { Value::Null } else { Value::Object(o) }
}

/// A few words on an item's own page numbering, shown beside it in the binder.
fn numbering_note(rule: &Value) -> Option<String> {
    if !rule.is_object() {
        return None;
    }
    let mut parts: Vec<String> = Vec::new();
    match rule["zone"].as_str() {
        Some("front") => parts.push("i, ii".into()),
        Some("main") => parts.push("1, 2".into()),
        Some("none") => parts.push(tl!("no numbers").into()),
        _ => {}
    }
    if let Some(n) = rule["start"].as_u64() {
        parts.push(crate::i18n::fmt(tl!("from {n}"), &[("n", &n.to_string())]));
    }
    if rule["counted"] == false {
        parts.push(tl!("not counted").into());
    } else if rule["show"] == "none" {
        parts.push(tl!("number hidden").into());
    } else if rule["show"] == "not-first" {
        parts.push(tl!("hidden on first page").into());
    }
    if rule["startOn"] == "odd" {
        parts.push(tl!("right-hand page").into());
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

/// Page Numbers for a binder item: how its pages are numbered when the project is compiled. A
/// folder's choices apply to what it holds, but "Start at", which goes to its first section.
fn page_numbers_menu(ui: &mut Ui, id: &str, rule: &Value, todo: &mut Option<Do>) {
    let mut set = |ui: &mut Ui, key: &str, value: Value| {
        *todo = Some(Do::Numbering(id.to_string(), with_rule(rule, key, value)));
        ui.close();
    };
    let zone = rule["zone"].as_str().unwrap_or("");
    if ui.radio(zone == "front", tl!("Roman numerals (i, ii, iii)")).clicked() {
        set(ui, "zone", json!("front"));
    }
    if ui.radio(zone == "main", tl!("Numbers (1, 2, 3), from 1")).clicked() {
        set(ui, "zone", json!("main"));
    }
    if ui.radio(zone.is_empty() || zone == "continue", tl!("Carry on from the pages before")).clicked() {
        set(ui, "zone", Value::Null);
    }
    ui.separator();
    // Start at N from here.
    let key = egui::Id::unique(("binder_page_start", id));
    let mut n = ui.data(|d| d.get_temp::<u32>(key)).unwrap_or_else(|| rule["start"].as_u64().and_then(|n| u32::try_from(n).ok()).unwrap_or(1));
    crate::widgets::row(ui, |ui| {
        ui.label(tl!("Start at"));
        if ui.add(egui::DragValue::new(&mut n).range(0..=wordcraft_manuscript::compile::MAX_PAGE_START).speed(0.2)).changed() {
            ui.data_mut(|d| d.insert_temp(key, n));
        }
        if ui.button(tl!("Set")).on_hover_text(tl!("The first page of this item takes that number, and the pages after count on from it")).clicked() {
            ui.data_mut(|d| d.remove::<u32>(key));
            set(ui, "start", json!(n));
        }
    });
    if rule["start"].is_u64() && ui.button(tl!("Don't restart here")).clicked() {
        set(ui, "start", Value::Null);
    }
    ui.separator();
    let counted = rule["counted"] != false;
    let shown = rule["show"] != "none";
    let mut c = counted;
    if ui
        .checkbox(&mut c, tl!("Count these pages"))
        .on_hover_text(tl!("Off: these pages show no number, and the pages after them number as if they weren't there"))
        .clicked()
    {
        set(ui, "counted", if c { Value::Null } else { json!(false) });
    }
    let mut sh = shown && counted;
    if ui.add_enabled(counted, egui::Checkbox::new(&mut sh, tl!("Show the page number"))).clicked() {
        set(ui, "show", if sh { Value::Null } else { json!("none") });
    }
    let mut first = rule["show"] == "not-first";
    if ui.add_enabled(counted && shown, egui::Checkbox::new(&mut first, tl!("Hide it on the first page"))).clicked() {
        set(ui, "show", if first { json!("not-first") } else { Value::Null });
    }
    let mut odd = rule["startOn"] == "odd";
    if ui.checkbox(&mut odd, tl!("Start on a right-hand page")).clicked() {
        set(ui, "startOn", if odd { json!("odd") } else { Value::Null });
    }
    ui.separator();
    if ui.add_enabled(rule.is_object(), egui::Button::new(tl!("Reset"))).on_hover_text(tl!("Number these pages as the folder above says")).clicked() {
        *todo = Some(Do::Numbering(id.to_string(), Value::Null));
        ui.close();
    }
}

struct Ctx<'a> {
    current: Option<&'a str>,
    selected: Option<&'a str>,
    renaming: Option<&'a (String, String)>,
    collapsed: &'a HashSet<String>,
}

/// One level of the binder, indented by `depth`.
fn tree(ui: &mut Ui, items: &Value, parent: Option<&str>, depth: usize, cx: &Ctx<'_>, todo: &mut Option<Do>) {
    let Some(list) = items.as_array() else { return };
    if depth > wordcraft_manuscript::project::MAX_DEPTH {
        return;
    }
    let count = list.len();
    for (k, item) in list.iter().enumerate() {
        let Some(id) = item["id"].as_str() else { continue };
        let folder = item["kind"] == "folder";
        let trash = id == wordcraft_manuscript::project::TRASH;
        ui.push_id(("binder", id), |ui| {
            crate::widgets::row(ui, |ui| {
                ui.add_space(depth as f32 * 14.0);
                if folder {
                    let open = !cx.collapsed.contains(id);
                    if ui.small_button(if open { "▾" } else { "▸" }).clicked() {
                        *todo = Some(Do::Toggle(id.to_string()));
                    }
                }
                if let Some((rid, title)) = cx.renaming
                    && rid == id
                {
                    let mut t = title.clone();
                    let key = egui::Id::unique(("binder_rename", id));
                    let mut draft = ui.data(|d| d.get_temp::<String>(key)).unwrap_or(t.clone());
                    let r = ui.add(egui::TextEdit::singleline(&mut draft).hint_text(tl!("Title")).desired_width(160.0));
                    if r.changed() {
                        ui.data_mut(|d| d.insert_temp(key, draft.clone()));
                    }
                    // Focus once, when renaming starts: the writer can click elsewhere to finish.
                    let asked = egui::Id::unique(("binder_rename_focus", id));
                    if !ui.data(|d| d.get_temp::<bool>(asked).unwrap_or(false)) {
                        r.request_focus();
                        ui.data_mut(|d| d.insert_temp(asked, true));
                    }
                    if r.lost_focus() {
                        t = draft.trim().to_string();
                        ui.data_mut(|d| d.remove::<String>(key));
                        ui.data_mut(|d| d.remove::<bool>(egui::Id::unique(("binder_rename_focus", id))));
                        *todo = Some(Do::Rename(id.to_string(), if t.is_empty() { title.clone() } else { t }));
                    }
                    return;
                }
                let title = item["title"].as_str().unwrap_or("");
                let title = if trash {
                    tl!("Trash").to_string()
                } else if title.is_empty() {
                    tl!("Untitled").to_string()
                } else {
                    title.to_string()
                };
                let shown = cx.current == Some(id);
                let picked = cx.selected == Some(id);
                let mut text = egui::RichText::new(title).font(regular(13.0));
                if shown {
                    text = text.strong();
                }
                if item["include"] == false && !trash {
                    text = text.weak().italics();
                }
                let r = ui.add(egui::Button::selectable(picked || shown, text).frame_when_inactive(false));
                if r.clicked() {
                    *todo = Some(if folder { Do::Pick(id.to_string()) } else { Do::Open(id.to_string()) });
                }
                if r.double_clicked() && !trash {
                    *todo = Some(Do::StartRename(id.to_string()));
                }
                if let Some(note) = numbering_note(&item["numbering"]) {
                    ui.label(egui::RichText::new(note).small().weak())
                        .on_hover_text(tl!("Page numbering set here. Right-click the item, then Page Numbers, to change it."));
                }
                if let Some(w) = item["words"].as_u64() {
                    let words = match item["target"].as_u64() {
                        Some(t) => format!("{w}/{t}"),
                        None => w.to_string(),
                    };
                    ui.label(egui::RichText::new(words).small().weak());
                }
                r.context_menu(|ui| {
                    if trash {
                        if ui.button(tl!("Empty Trash")).clicked() {
                            *todo = Some(Do::EmptyTrash);
                            ui.close();
                        }
                        return;
                    }
                    if ui.button(tl!("Rename")).clicked() {
                        *todo = Some(Do::StartRename(id.to_string()));
                        ui.close();
                    }
                    if ui.add_enabled(k > 0, egui::Button::new(tl!("Move Up"))).clicked() {
                        *todo = Some(Do::Move(id.to_string(), parent.map(str::to_string), k.saturating_sub(1)));
                        ui.close();
                    }
                    if ui.add_enabled(k + 1 < count, egui::Button::new(tl!("Move Down"))).clicked() {
                        *todo = Some(Do::Move(id.to_string(), parent.map(str::to_string), k + 1));
                        ui.close();
                    }
                    ui.menu_button(tl!("Page Numbers"), |ui| page_numbers_menu(ui, id, &item["numbering"], todo));
                    ui.separator();
                    if ui.button(tl!("Move to Trash")).clicked() {
                        *todo = Some(Do::Delete(id.to_string()));
                        ui.close();
                    }
                });
            });
            if folder && !cx.collapsed.contains(id) {
                tree(ui, &item["children"], Some(id), depth + 1, cx, todo);
            }
        });
    }
}
