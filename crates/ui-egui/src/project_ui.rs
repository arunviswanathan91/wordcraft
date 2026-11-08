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
    renaming: Option<(String, String)>,
    /// Folders shown closed.
    collapsed: HashSet<String>,
    /// The last thing that went wrong, shown in the pane.
    pub error: Option<String>,
}

impl ProjectUi {
    fn new(root: String, info: Value) -> Self {
        ProjectUi { root, info, selected: None, binder: true, refreshed: f64::NEG_INFINITY, renaming: None, collapsed: HashSet::new(), error: None }
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
    Ok(info)
}

/// Make a project at `path` (`.wcproj` added when missing), open it and its first section.
pub(crate) fn new_project(app: &mut WordApp, path: &str) -> Result<Value, String> {
    let path = if path.ends_with(".wcproj") { path.to_string() } else { format!("{path}.wcproj") };
    let title = Path::new(&path).file_stem().and_then(|s| s.to_str()).unwrap_or("Untitled project").to_string();
    app.session.run("project.new", &json!({"path": path, "title": title})).map_err(|e| e.to_string())?;
    open_project(app, &path)
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

/// The file of section `id` of the open project.
fn section_path(app: &WordApp, id: &str) -> Option<PathBuf> {
    let root = app.project.as_ref()?.root.clone();
    wordcraft_manuscript::project::valid_id(id).then(|| Path::new(&root).join(wordcraft_manuscript::project::SECTIONS).join(format!("{id}.docx")))
}

/// The section of the open project shown in the window, if it is one.
pub(crate) fn current_section(app: &WordApp) -> Option<String> {
    let root = PathBuf::from(&app.project.as_ref()?.root);
    let path = app.session.path.as_ref()?;
    let id = path.file_stem()?.to_str()?;
    (path.parent()? == root.join(wordcraft_manuscript::project::SECTIONS)).then(|| id.to_string())
}

/// Show section `id`: the section being left is saved first when it has changes, then the
/// section opens (beside the others; one already open is shown).
pub(crate) fn open_section(app: &mut WordApp, id: &str) -> Result<Value, String> {
    let path = section_path(app, id).ok_or_else(|| tl!("That isn't a section of the project.").to_string())?;
    if current_section(app).is_some() && app.document_unsaved() {
        app.run("file.save", json!({}))?;
    }
    let r = app.open_file(&path.to_string_lossy(), None);
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
        "ui.project.new" => match path {
            Some(p) => new_project(app, &p),
            None => asked(app.ask_file(FileDialogRequest::Save { name: tl!("My Thesis").to_string() + ".wcproj", dir: None }, AfterPick::NewProject)),
        },
        "ui.project.open" => match path {
            Some(p) => open_project(app, &p),
            None => asked(app.ask_file(FileDialogRequest::Open { purpose: "project".into() }, AfterPick::OpenProject)),
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
    ui.separator();
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
                    if !r.has_focus() && !r.lost_focus() {
                        r.request_focus();
                    }
                    if r.lost_focus() {
                        t = draft.trim().to_string();
                        ui.data_mut(|d| d.remove::<String>(key));
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
