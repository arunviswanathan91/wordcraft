use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use serde_json::{Value, json};
use wordcraft_engine::Session;

use super::*;
use crate::project::{MAX_DEPTH, PROJECT_FILE, Project};

/// Section `id`'s file in the project at `root`.
pub(crate) fn section(root: &Path, id: &str) -> PathBuf {
    Project::open(root).unwrap().section_path(id).unwrap()
}

/// A fresh, empty folder for one test.
pub(crate) fn scratch() -> PathBuf {
    static N: AtomicU64 = AtomicU64::new(0);
    let dir = std::env::temp_dir().join(format!(
        "wc-manuscript-{}-{}-{}",
        std::process::id(),
        wordcraft_engine::cmd::now_unix(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

pub(crate) fn session() -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    install(&mut s);
    s
}

/// A new project in a scratch folder.
fn project(s: &mut Session) -> (PathBuf, String) {
    let root = scratch().join("thesis.wcproj");
    s.run("project.new", &json!({"path": root.to_string_lossy(), "title": "Ink and Iron"})).unwrap();
    let p = root.to_string_lossy().to_string();
    (root, p)
}

fn info(s: &mut Session, p: &str) -> Value {
    s.run("project.info", &json!({"project": p})).unwrap()
}

/// (id, title) of the items, nested folders as "folder/…".
fn tree(v: &Value) -> Vec<String> {
    fn walk(items: &Value, prefix: &str, out: &mut Vec<String>) {
        for i in items.as_array().into_iter().flatten() {
            let name = format!("{prefix}{}", i["title"].as_str().unwrap_or(""));
            out.push(name.clone());
            walk(&i["children"], &format!("{name}/"), out);
        }
    }
    let mut out = Vec::new();
    walk(&v["items"], "", &mut out);
    out
}

#[test]
fn a_new_project_has_its_folders_and_one_empty_section() {
    let mut s = session();
    let (root, p) = project(&mut s);
    for dir in ["sections", "snapshots", "research"] {
        assert!(root.join(dir).is_dir(), "{dir}");
    }
    assert!(section(&root, "s0001").is_file());
    let v = info(&mut s, &p);
    assert_eq!((v["title"].as_str(), v["recovered"].clone()), (Some("Ink and Iron"), Value::Null));
    assert_eq!(tree(&v), ["Untitled"]);
    assert_eq!(v["items"][0]["words"], 0);
    // Not over an existing project.
    assert!(s.run("project.new", &json!({"path": p})).is_err());
}

#[test]
fn binder_items_are_added_moved_changed_and_trashed() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let folder = s.run("binder.add", &json!({"project": p, "kind": "folder", "title": "Front matter"})).unwrap()["id"].as_str().unwrap().to_string();
    let abs = s.run("binder.add", &json!({"project": p, "title": "Abstract", "parent": folder})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(section(&root, &abs).is_file(), "a section gets its document");
    s.run("binder.move", &json!({"project": p, "id": "s0001", "parent": folder, "index": 0})).unwrap();
    assert_eq!(tree(&info(&mut s, &p)), ["Front matter", "Front matter/Untitled", "Front matter/Abstract"]);
    s.run("binder.rename", &json!({"project": p, "id": "s0001", "title": "Title page"})).unwrap();
    s.run(
        "binder.setMeta",
        &json!({"project": p, "id": abs, "synopsis": "300 words.", "status": "draft", "target": 300, "include": false, "role": "abstract"}),
    )
    .unwrap();
    let v = info(&mut s, &p);
    let a = &v["items"][0]["children"][1];
    assert_eq!(
        (a["synopsis"].as_str(), a["status"].as_str(), a["target"].as_u64(), a["include"].as_bool(), a["role"].as_str()),
        (Some("300 words."), Some("draft"), Some(300), Some(false), Some("abstract"))
    );
    s.run("binder.setMeta", &json!({"project": p, "id": abs, "target": null})).unwrap();
    assert_eq!(info(&mut s, &p)["items"][0]["children"][1]["target"], Value::Null);
    // Delete: into the Trash, file kept; Empty Trash removes it.
    s.run("binder.delete", &json!({"project": p, "id": abs})).unwrap();
    assert_eq!(tree(&info(&mut s, &p)), ["Front matter", "Front matter/Title page", "Trash", "Trash/Abstract"]);
    assert!(section(&root, &abs).is_file());
    let r = s.run("binder.emptyTrash", &json!({"project": p})).unwrap();
    assert_eq!(r["removed"], json!([abs]));
    assert!(!section(&root, &abs).exists());
    // Ids aren't given out again.
    let next = s.run("binder.add", &json!({"project": p})).unwrap()["id"].as_str().unwrap().to_string();
    assert_ne!(next, abs);
}

#[test]
fn binder_refuses_impossible_moves() {
    let mut s = session();
    let (_, p) = project(&mut s);
    let a = s.run("binder.add", &json!({"project": p, "kind": "folder", "title": "A"})).unwrap()["id"].as_str().unwrap().to_string();
    let b = s.run("binder.add", &json!({"project": p, "kind": "folder", "title": "B", "parent": a})).unwrap()["id"].as_str().unwrap().to_string();
    let before = std::fs::read_to_string(Path::new(&p).join(PROJECT_FILE)).unwrap();
    for bad in [
        json!({"project": p, "id": a, "parent": b}),
        json!({"project": p, "id": a, "parent": a}),
        json!({"project": p, "id": b, "parent": "s0001"}),
        json!({"project": p, "id": "nope", "parent": a}),
        json!({"project": p, "id": "../../etc", "parent": null}),
        json!({"project": p, "id": a, "parent": 7}),
    ] {
        assert!(s.run("binder.move", &bad).is_err(), "{bad}");
    }
    assert_eq!(std::fs::read_to_string(Path::new(&p).join(PROJECT_FILE)).unwrap(), before, "refused moves change nothing");
    assert!(s.run("binder.add", &json!({"project": p, "kind": "chapter"})).is_err());
    assert!(s.run("binder.add", &json!({"project": p, "parent": "s0001"})).is_err(), "a section holds no items");
    assert!(s.run("project.setMetadata", &json!({"project": p, "deadline": "31/03/2027"})).is_err());
    s.run("project.setMetadata", &json!({"project": p, "deadline": "2027-03-31", "projectWords": 80000})).unwrap();
    assert_eq!(info(&mut s, &p)["targets"]["deadline"], "2027-03-31");
}

/// Keys WordCraft doesn't know survive a save, on the project, its targets, items and presets.
#[test]
fn unknown_keys_are_kept() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let text = r#"format = 1
title = "Kept"
next_id = 5
colour_scheme = "plum"

[targets]
project_words = 1000
pages = 120

[[item]]
id = "s0001"
kind = "section"
title = "One"
mood = "calm"

[[preset]]
name = "University submission"
output = "pdf"
"#;
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    s.run("binder.rename", &json!({"project": p, "id": "s0001", "title": "One, renamed"})).unwrap();
    let saved = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    for kept in ["colour_scheme = \"plum\"", "pages = 120", "mood = \"calm\"", "name = \"University submission\"", "One, renamed"] {
        assert!(saved.contains(kept), "{kept} in:\n{saved}");
    }
}

/// A project.toml that can't be used opens in recovery mode, listing every section file; the
/// next save keeps the broken file beside the new one.
#[test]
fn a_broken_project_file_opens_in_recovery_mode() {
    let deep_headers =
        (1..=MAX_DEPTH + 5).map(|n| format!("[[item{}]]\nid = \"f{n}\"\nkind = \"folder\"\n", ".children".repeat(n - 1))).collect::<String>();
    let deep_inline = format!("item = [{}{}]", "{ id = \"a\", kind = \"folder\", children = [".repeat(200), "]}".repeat(200));
    let cases = [
        "this is not toml [[[".to_string(),
        "[[item]]\nid = \"../../etc/passwd\"\nkind = \"section\"\n".to_string(),
        "[[item]]\nid = \"a\"\nkind = \"section\"\n[[item]]\nid = \"a\"\nkind = \"section\"\n".to_string(),
        "[[item]]\nid = \"a\"\nkind = \"section\"\n[[item.children]]\nid = \"b\"\nkind = \"section\"\n".to_string(),
        deep_headers,
        deep_inline,
        "x".repeat(5 * 1024 * 1024),
    ];
    for (n, broken) in cases.iter().enumerate() {
        let mut s = session();
        let (root, p) = project(&mut s);
        s.run("binder.add", &json!({"project": p, "title": "Second"})).unwrap();
        std::fs::write(root.join(PROJECT_FILE), broken).unwrap();
        let v = info(&mut s, &p);
        assert!(v["recovered"].is_string(), "case {n}: {v}");
        let ids: Vec<&str> = v["items"].as_array().unwrap().iter().filter_map(|i| i["id"].as_str()).collect();
        assert_eq!(ids, ["s0001", "s0002"], "case {n}");
        // Changing the binder writes a good file and keeps the broken one.
        s.run("binder.rename", &json!({"project": p, "id": "s0002", "title": "Methods"})).unwrap();
        assert!(Project::open(&root).unwrap().recovered.is_none(), "case {n}");
        let kept = std::fs::read_dir(&root)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with("project.toml.broken-"))
            .count();
        assert_eq!(kept, 1, "case {n}");
        // A new id doesn't take a section file's.
        let id = s.run("binder.add", &json!({"project": p})).unwrap()["id"].as_str().unwrap().to_string();
        assert_eq!(id, "s0003", "case {n}");
        if let Some(dir) = root.parent() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }
}

#[test]
fn a_newer_project_format_is_refused() {
    let mut s = session();
    let (root, p) = project(&mut s);
    std::fs::write(root.join(PROJECT_FILE), "format = 99\ntitle = \"Future\"\n").unwrap();
    let e = s.run("project.info", &json!({"project": p})).unwrap_err().to_string();
    assert!(e.contains("newer"), "{e}");
}

#[test]
fn snapshots_are_taken_listed_and_restored() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let path = section(&root, "s0001");
    wordcraft_engine::io::save_path(&path, &wordcraft_doc::Document::from_text("The first draft of the chapter.")).unwrap();
    let first = s.run("snapshot.take", &json!({"project": p, "id": "s0001", "label": "Sent to supervisor"})).unwrap();
    assert_eq!(first["words"], 6);
    wordcraft_engine::io::save_path(&path, &wordcraft_doc::Document::from_text("Rewritten.")).unwrap();
    let at = first["at"].as_str().unwrap().to_string();
    let r = s.run("snapshot.restore", &json!({"project": p, "id": "s0001", "at": at})).unwrap();
    let back = wordcraft_engine::io::open_path(&path).unwrap();
    assert_eq!(back.plain_text(wordcraft_doc::StoryRef::Body).trim(), "The first draft of the chapter.");
    let list = s.run("snapshot.list", &json!({"project": p, "id": "s0001"})).unwrap();
    let labels: Vec<&str> = list["snapshots"].as_array().unwrap().iter().filter_map(|x| x["label"].as_str()).collect();
    assert_eq!(labels.len(), 2, "{list}");
    assert_eq!(labels[0], "Sent to supervisor");
    assert!(labels[1].starts_with("Before restoring"), "{labels:?}");
    assert_ne!(r["before"], json!(at), "two snapshots in one second get different names");
    // Bad names and folders are refused.
    for bad in ["../../x", "", "20261107T143000Z.docx", "nope"] {
        assert!(s.run("snapshot.restore", &json!({"project": p, "id": "s0001", "at": bad})).is_err(), "{bad}");
    }
    let f = s.run("binder.add", &json!({"project": p, "kind": "folder"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(s.run("snapshot.take", &json!({"project": p, "id": f})).is_err(), "a folder has no document");
}

/// Word counts come from the sections' documents (and the cache while they're unchanged); the
/// total counts only included sections.
#[test]
fn word_counts_follow_the_sections() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let two = s.run("binder.add", &json!({"project": p})).unwrap()["id"].as_str().unwrap().to_string();
    wordcraft_engine::io::save_path(&section(&root, "s0001"), &wordcraft_doc::Document::from_text("one two three")).unwrap();
    wordcraft_engine::io::save_path(&section(&root, &two), &wordcraft_doc::Document::from_text("four five")).unwrap();
    assert_eq!(info(&mut s, &p)["words"], 5);
    assert!(root.join(".wcproj-cache.toml").is_file());
    s.run("binder.setMeta", &json!({"project": p, "id": two, "include": false})).unwrap();
    assert_eq!(info(&mut s, &p)["words"], 3);
    // A damaged cache is ignored.
    std::fs::write(root.join(".wcproj-cache.toml"), "[[[ not toml").unwrap();
    assert_eq!(info(&mut s, &p)["words"], 3);
}

#[test]
fn opening_a_section_opens_its_document() {
    let mut s = session();
    let (root, p) = project(&mut s);
    wordcraft_engine::io::save_path(&section(&root, "s0001"), &wordcraft_doc::Document::from_text("Chapter one text")).unwrap();
    s.run("binder.openItem", &json!({"project": p, "id": "s0001"})).unwrap();
    assert_eq!(s.doc.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Chapter one text");
}

/// Every manuscript command with junk parameters returns, never panics (called directly, past
/// the session's panic guard).
#[test]
fn hostile_params_never_panic() {
    let mut s = session();
    let (_, p) = project(&mut s);
    let missing = scratch().join("not-a-project").to_string_lossy().to_string();
    let junk = [
        json!(null),
        json!(5),
        json!("x"),
        json!({}),
        json!({"project": 5}),
        json!({"project": ""}),
        json!({"project": missing}),
        json!({"project": p, "id": "../../x", "parent": "../..", "index": -1, "at": "../.."}),
        json!({"project": p, "id": "s0001", "parent": [1, 2], "index": 1e300, "title": {"a": 1}, "target": -5, "include": "yes", "deadline": 3}),
        json!({"project": p, "id": "s0001", "title": "x".repeat(100_000), "kind": "folder", "parent": null, "index": u64::MAX}),
        json!({"project": p, "id": "trash"}),
    ];
    for spec in specs() {
        if spec.id == "project.new" {
            continue;
        }
        for v in &junk {
            let _ = (spec.run)(&mut s, v);
        }
    }
    // project.new with junk makes nothing outside a scratch folder.
    for v in [json!(null), json!({"path": 5}), json!({"path": ""}), json!({"path": p})] {
        assert!(new_rejects(&mut s, &v), "{v}");
    }
}

fn new_rejects(s: &mut Session, v: &Value) -> bool {
    specs().iter().find(|x| x.id == "project.new").is_some_and(|spec| (spec.run)(s, v).is_err())
}

#[test]
fn the_registry_has_the_engines_commands_and_these() {
    let r = registry();
    assert!(r.get("text.insert").is_some() && r.get("project.new").is_some() && r.get("snapshot.restore").is_some());
    let mut ids: Vec<&str> = r.all().iter().map(|s| s.id).collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n, "no command id twice");
}

/// Sections are saved under their titles, and renaming one renames its file; names Windows
/// can't take are made safe, and two sections never share a file.
#[test]
fn section_files_are_named_by_their_titles() {
    let mut s = session();
    let (root, p) = project(&mut s);
    assert!(root.join("sections/Untitled.docx").is_file());
    let lit = s.run("binder.add", &json!({"project": p, "title": "Review of Literature"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(root.join("sections/Review of Literature.docx").is_file());
    let twin = s.run("binder.add", &json!({"project": p, "title": "Review of Literature"})).unwrap()["id"].as_str().unwrap().to_string();
    assert!(root.join("sections/Review of Literature 2.docx").is_file(), "two sections never share a file");
    wordcraft_engine::io::save_path(&section(&root, &lit), &wordcraft_doc::Document::from_text("Kept text")).unwrap();
    s.run("binder.rename", &json!({"project": p, "id": lit, "title": "Literature: a review?"})).unwrap();
    let renamed = root.join("sections/Literature- a review-.docx");
    assert!(renamed.is_file() && !root.join("sections/Review of Literature.docx").exists(), "the file follows the title");
    assert_eq!(wordcraft_engine::io::open_path(&renamed).unwrap().plain_text(wordcraft_doc::StoryRef::Body).trim(), "Kept text");
    assert_eq!(info(&mut s, &p)["items"][1]["file"], "Literature- a review-.docx");
    // Emptying the Trash removes a title-named file too.
    s.run("binder.delete", &json!({"project": p, "id": twin})).unwrap();
    s.run("binder.emptyTrash", &json!({"project": p})).unwrap();
    assert!(!root.join("sections/Review of Literature 2.docx").exists());
    // A project file naming a file outside sections/ is refused (recovery mode).
    let text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap().replace("file = \"Untitled.docx\"", "file = \"../../outside.docx\"");
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    assert!(info(&mut s, &p)["recovered"].is_string());
}

#[test]
fn file_names_are_safe() {
    use crate::project::{file_name_for, valid_file_name};
    let none = std::collections::HashSet::new();
    assert_eq!(file_name_for("Chapter 1: Methods", &none), "Chapter 1- Methods.docx");
    assert_eq!(file_name_for("   ", &none), "Untitled.docx");
    assert_eq!(file_name_for("CON", &none), "CON section.docx");
    assert_eq!(file_name_for("ends with dots...", &none), "ends with dots.docx");
    assert_eq!(file_name_for(&"x".repeat(500), &none).chars().count(), 85);
    for bad in ["../x.docx", "a/b.docx", "x.doc", ".docx", "NUL.docx", "a\u{0}.docx", ".hidden.docx"] {
        assert!(!valid_file_name(bad), "{bad:?}");
    }
    assert!(valid_file_name("Review of Literature.docx"));
}

/// Recovery mode lists title-named files as sections too, with fresh ids.
#[test]
fn recovery_lists_title_named_files() {
    let mut s = session();
    let (root, p) = project(&mut s);
    s.run("binder.add", &json!({"project": p, "title": "Methods"})).unwrap();
    std::fs::write(root.join(PROJECT_FILE), "not toml [[[").unwrap();
    let v = info(&mut s, &p);
    let titles: Vec<&str> = v["items"].as_array().unwrap().iter().filter_map(|i| i["title"].as_str()).collect();
    assert_eq!(titles, ["Methods", "Untitled"]);
    s.run("binder.rename", &json!({"project": p, "id": v["items"][0]["id"], "title": "Method"})).unwrap();
    assert!(root.join("sections/Method.docx").is_file());
}

/// Import splits a document at its Heading 1s, one section each titled by its heading; text
/// before the first heading is a section of its own.
#[test]
fn import_splits_a_document_at_its_headings() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let mut d = Session::new(wordcraft_doc::Document::from_text("Preface text\nIntroduction\nIntro text\nMethods\nMethods text"));
    for h in ["Introduction", "Methods"] {
        d.run("select.text", &json!({"text": h})).unwrap();
        d.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    }
    let src = root.parent().unwrap().join("thesis draft.docx");
    wordcraft_engine::io::save_path(&src, &d.doc).unwrap();
    let r = s.run("binder.import", &json!({"project": p, "path": src.to_string_lossy()})).unwrap();
    assert_eq!(r["ids"].as_array().unwrap().len(), 3, "{r}");
    let v = info(&mut s, &p);
    let titles: Vec<&str> = v["items"].as_array().unwrap().iter().filter_map(|i| i["title"].as_str()).collect();
    assert_eq!(titles, ["Untitled", "thesis draft", "Introduction", "Methods"]);
    let methods = wordcraft_engine::io::open_path(&root.join("sections/Methods.docx")).unwrap();
    assert_eq!(methods.plain_text(wordcraft_doc::StoryRef::Body).trim(), "Methods\nMethods text");
    assert!(s.run("binder.import", &json!({"project": p, "path": src.to_string_lossy(), "split": "chapters"})).is_err());
}

/// A document whose headings were typed ("1 Introduction") is split at them too: they become
/// real headings first.
#[test]
fn import_finds_typed_headings() {
    let mut s = session();
    let (root, p) = project(&mut s);
    let text =
        "1 Introduction\nWe study kilns here, at length, in a paragraph that ends with a full stop.\n1.1 Aims\nAims text.\n2 Methods\nMethods text.";
    let src = root.parent().unwrap().join("typed.docx");
    wordcraft_engine::io::save_path(&src, &wordcraft_doc::Document::from_text(text)).unwrap();
    let r = s.run("binder.import", &json!({"project": p, "path": src.to_string_lossy()})).unwrap();
    assert!(r["converted"].as_u64().unwrap() >= 3, "{r}");
    let titles: Vec<String> = info(&mut s, &p)["items"].as_array().unwrap().iter().filter_map(|i| i["title"].as_str().map(str::to_string)).collect();
    assert_eq!(titles, ["Untitled", "Introduction", "Methods"], "split at the level-1 headings, numbers gone from the titles");
}
