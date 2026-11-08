use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wordcraft_doc::numbering::Counters;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;

use crate::project::PROJECT_FILE;
use crate::tests::{scratch, session};

/// Write section `id` of the project at `root` from a session the closure builds.
fn write_section(root: &Path, id: &str, build: impl FnOnce(&mut Session)) {
    let mut s = Session::new(Document::new());
    build(&mut s);
    wordcraft_engine::io::save_path(&root.join(format!("sections/{id}.docx")), &s.doc).unwrap();
}

fn heading(s: &mut Session, text: &str, style: &str) {
    s.run("text.insert", &json!({"text": text})).unwrap();
    s.run("styles.apply", &json!({"style": style})).unwrap();
    s.run("text.newline", &json!({})).ok();
}

/// A project: two top-level chapters, a folder holding two more sections, an excluded section
/// and one in the Trash.
fn thesis() -> (Session, PathBuf, String) {
    let mut s = session();
    let root = scratch().join("thesis.wcproj");
    let p = root.to_string_lossy().to_string();
    s.run("project.new", &json!({"path": p})).unwrap();
    let add = |s: &mut Session, v: Value| s.run("binder.add", &v).unwrap()["id"].as_str().unwrap().to_string();
    let methods = add(&mut s, json!({"project": p, "title": "Methods"}));
    let appendix = add(&mut s, json!({"project": p, "kind": "folder", "title": "Appendices"}));
    let a1 = add(&mut s, json!({"project": p, "title": "A1", "parent": appendix}));
    let a2 = add(&mut s, json!({"project": p, "title": "A2", "parent": appendix}));
    let notes = add(&mut s, json!({"project": p, "title": "Notes"}));
    let gone = add(&mut s, json!({"project": p, "title": "Gone"}));
    s.run("binder.setMeta", &json!({"project": p, "id": notes, "include": false})).unwrap();
    s.run("binder.delete", &json!({"project": p, "id": gone})).unwrap();
    for (id, text) in [
        ("s0001", "Introduction text."),
        (methods.as_str(), "Methods text."),
        (a1.as_str(), "First appendix."),
        (a2.as_str(), "Second appendix."),
        (notes.as_str(), "Private notes."),
        (gone.as_str(), "Deleted text."),
    ] {
        write_section(&root, id, |s| {
            s.run("text.insert", &json!({"text": text})).unwrap();
        });
    }
    (s, root, p)
}

fn body_texts(doc: &Document) -> Vec<String> {
    doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).filter(|t| !t.is_empty()).collect()
}

#[test]
fn compiling_joins_the_included_sections_in_binder_order() {
    let (mut s, root, p) = thesis();
    let before: Vec<Vec<u8>> = std::fs::read_dir(root.join("sections")).unwrap().map(|e| std::fs::read(e.unwrap().path()).unwrap()).collect();
    let out = root.parent().unwrap().join("thesis.docx");
    let r = s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    assert_eq!(r["sections"].as_array().unwrap().len(), 4, "{r}");
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    assert_eq!(
        body_texts(&doc),
        ["Introduction text.", "Methods text.", "First appendix.", "Second appendix."],
        "excluded and trashed sections are left out"
    );
    // Chapters start new sections; the appendices inside one folder are split by a page break.
    let paras: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).collect();
    let breaks = paras.iter().filter(|p| p.section.is_some()).count();
    assert_eq!(breaks, 2, "Introduction | Methods | Appendices");
    let second = paras.iter().find(|p| p.plain_text() == "Second appendix.").unwrap();
    assert_eq!(second.props.page_break_before, Some(true));
    let after: Vec<Vec<u8>> = std::fs::read_dir(root.join("sections")).unwrap().map(|e| std::fs::read(e.unwrap().path()).unwrap()).collect();
    assert_eq!(before, after, "section files are not changed");
}

/// Two sections that each have a bookmark `_Ref1` and a cross-reference to it: both survive,
/// each reference pointing at its own section's bookmark.
#[test]
fn bookmarks_with_the_same_name_are_kept_apart() {
    let (mut s, root, p) = thesis();
    for id in ["s0001", "s0002"] {
        write_section(&root, id, |s| {
            s.run("text.insert", &json!({"text": format!("Target {id}. ")})).unwrap();
            s.run("select.text", &json!({"text": format!("Target {id}")})).unwrap();
            s.run("insert.bookmark", &json!({"name": "_Ref1"})).unwrap();
            s.run("caret.docEnd", &json!({})).unwrap();
            s.run("insert.field", &json!({"instr": "REF _Ref1 \\h", "result": "x"})).unwrap();
        });
    }
    let r = s.run("compile.run", &json!({"project": p, "path": root.parent().unwrap().join("b.docx").to_string_lossy()})).unwrap();
    assert!(!r["warnings"].to_string().contains("cross-reference"), "{r}");
    let doc = wordcraft_engine::io::open_path(&root.parent().unwrap().join("b.docx")).unwrap();
    let mut marks = Vec::new();
    let mut refs = Vec::new();
    for p in doc.body.iter().filter_map(|b| b.as_para()) {
        for o in &p.objects {
            match o {
                InlineObject::BookmarkStart { name } => marks.push(name.clone()),
                InlineObject::Field { instr, result, .. } if instr.starts_with("REF") => refs.push((instr.clone(), result.clone())),
                _ => {}
            }
        }
    }
    assert_eq!(marks.len(), 2, "{marks:?}");
    assert_ne!(marks[0], marks[1]);
    assert!(refs[0].0.contains(&marks[0]) && refs[1].0.contains(&marks[1]), "{refs:?} {marks:?}");
    assert_eq!((refs[0].1.as_str(), refs[1].1.as_str()), ("Target s0001", "Target s0002"), "each shows its own target");
}

/// Each chapter file numbers its Heading 1 with a list of its own (1 in each file); compiled,
/// they count 1, 2.
#[test]
fn chapters_number_on_across_files() {
    let (mut s, root, p) = thesis();
    for (id, title) in [("s0001", "Introduction"), ("s0002", "Methods")] {
        write_section(&root, id, |s| {
            heading(s, title, "Heading 1");
            s.run("select.text", &json!({"text": title})).unwrap();
            s.run("list.multilevelPreset", &json!({"preset": "headings"})).unwrap();
        });
    }
    let out = root.parent().unwrap().join("n.docx");
    s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let mut c = Counters::default();
    let labels: Vec<String> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref() == Some("Heading1"))
        .map(|p| {
            let n = p.props.numbering.or_else(|| doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0).unwrap();
            c.next_label(&doc.numbering, n.num, n.level).unwrap().0
        })
        .collect();
    assert_eq!(labels, ["1", "2"]);
}

/// A preset with a table of contents and the sections' titles as headings.
#[test]
fn presets_add_titles_and_a_table_of_contents() {
    let (mut s, root, p) = thesis();
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text.push_str(
        "\n[[preset]]\nname = \"Thesis\"\ntoc = true\nheading_from_title = true\nseparator = \"page-break\"\nsection_separator = \"none\"\n",
    );
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    let names = s.run("compile.presets", &json!({"project": p})).unwrap();
    assert_eq!(names["presets"][0]["name"], "Thesis");
    let out = root.parent().unwrap().join("t.docx");
    s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy(), "preset": "Thesis"})).unwrap();
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let has_toc = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .any(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().starts_with("TOC"))));
    assert!(has_toc, "a table of contents at the start");
    let heads: Vec<(Option<String>, String)> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .filter(|p| p.props.style.as_deref().is_some_and(|s| s.starts_with("Heading")))
        .map(|p| (p.props.style.clone(), p.plain_text()))
        .collect();
    assert_eq!(
        heads,
        [
            (Some("Heading1".into()), "Untitled".into()),
            (Some("Heading1".into()), "Methods".into()),
            (Some("Heading2".into()), "A1".into()),
            (Some("Heading2".into()), "A2".into())
        ]
    );
    assert_eq!(doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.section.is_some()).count(), 0, "page breaks, no section breaks");
    assert!(s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy(), "preset": "Nope"})).is_err());
}

/// A missing section file is a warning, not a failure; a template outside the project is
/// refused; preview opens the result beside the open document.
#[test]
fn problems_are_warnings_and_templates_stay_inside() {
    let (mut s, root, p) = thesis();
    std::fs::remove_file(root.join("sections/s0002.docx")).unwrap();
    let r = s.run("compile.run", &json!({"project": p, "path": root.parent().unwrap().join("w.docx").to_string_lossy()})).unwrap();
    assert_eq!(r["sections"].as_array().unwrap().len(), 3);
    assert!(r["warnings"].to_string().contains("left out"), "{r}");
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text.push_str("\n[[preset]]\nname = \"Bad\"\ntemplate = \"../../outside.dotx\"\n");
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    assert!(s.run("compile.run", &json!({"project": p, "path": root.parent().unwrap().join("x.docx").to_string_lossy()})).is_err());
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text = text.replace("template = \"../../outside.dotx\"", "separator = \"sideways\"");
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    assert!(s.run("compile.presets", &json!({"project": p})).is_err(), "a bad separator is named");
}

#[test]
fn preview_opens_the_compiled_document() {
    let (mut s, _, p) = thesis();
    let home = s.document_id();
    let r = s.run("compile.preview", &json!({"project": p})).unwrap();
    assert_ne!(s.document_id(), home);
    assert_eq!(r["document"], s.document_id());
    assert!(s.doc.plain_text(StoryRef::Body).contains("Second appendix."));
}

/// The template's styles and page setup carry over; its body text doesn't.
#[test]
fn a_template_gives_its_page_setup() {
    let (mut s, root, p) = thesis();
    let mut t = Session::new(Document::from_text("Template body text"));
    t.doc.last_section.margin_left = 123.0;
    wordcraft_engine::io::save_path(&root.join("templates.docx"), &t.doc).unwrap();
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text.push_str("\n[[preset]]\nname = \"T\"\ntemplate = \"templates.docx\"\n");
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    let out = root.parent().unwrap().join("tt.docx");
    s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    assert!((doc.last_section.margin_left - 123.0).abs() < 0.5);
    assert!(!doc.plain_text(StoryRef::Body).contains("Template body text"));
}

/// Without an extension the preset's output format is used: PDF by default.
#[test]
fn a_path_without_extension_gets_the_presets_format() {
    let (mut s, root, p) = thesis();
    let r = s.run("compile.run", &json!({"project": p, "path": root.parent().unwrap().join("thesis").to_string_lossy()})).unwrap();
    let path = r["path"].as_str().unwrap();
    assert!(path.ends_with("thesis.pdf"), "{path}");
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.starts_with(b"%PDF"), "a PDF file");
}
