use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use wordcraft_doc::numbering::Counters;
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::{Document, StoryRef};
use wordcraft_engine::Session;

use crate::project::PROJECT_FILE;
use crate::tests::{scratch, section, session};

/// Write section `id` of the project at `root` from a session the closure builds.
fn write_section(root: &Path, id: &str, build: impl FnOnce(&mut Session)) {
    let mut s = Session::new(Document::new());
    build(&mut s);
    wordcraft_engine::io::save_path(&section(root, id), &s.doc).unwrap();
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
    std::fs::remove_file(section(&root, "s0002")).unwrap();
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

/// The section properties of the compiled document's sections, in order (the breaks', then
/// the last section's).
fn sections_of(doc: &Document) -> Vec<wordcraft_doc::section::SectionProps> {
    let mut v: Vec<_> = doc.body.iter().filter_map(|b| b.as_para()).filter_map(|p| p.section.as_deref().cloned()).collect();
    v.push(doc.last_section.clone());
    v
}

/// Page numbering zones: a title page with no number, roman front matter, then chapters in
/// arabic from 1, each on a right-hand page.
#[test]
fn numbering_zones_become_section_numbering() {
    use wordcraft_doc::section::{NumFormat, SectionStart};
    let mut s = session();
    let root = scratch().join("z.wcproj");
    let p = root.to_string_lossy().to_string();
    s.run("project.new", &json!({"path": p})).unwrap();
    let add = |s: &mut Session, v: Value| s.run("binder.add", &v).unwrap()["id"].as_str().unwrap().to_string();
    let abs = add(&mut s, json!({"project": p, "title": "Abstract"}));
    let ch = add(&mut s, json!({"project": p, "kind": "folder", "title": "Chapters"}));
    let one = add(&mut s, json!({"project": p, "title": "One", "parent": ch}));
    let two = add(&mut s, json!({"project": p, "title": "Two", "parent": ch}));
    s.run("binder.setMeta", &json!({"project": p, "id": "s0001", "numbering": {"zone": "front", "show": "none"}})).unwrap();
    s.run("binder.setMeta", &json!({"project": p, "id": abs, "numbering": {"zone": "front"}})).unwrap();
    s.run("binder.setMeta", &json!({"project": p, "id": ch, "numbering": {"zone": "main", "startOn": "odd"}})).unwrap();
    for (id, t) in [("s0001", "Title page"), (abs.as_str(), "Abstract text"), (one.as_str(), "One text"), (two.as_str(), "Two text")] {
        write_section(&root, id, |s| {
            s.run("text.insert", &json!({"text": t})).unwrap();
        });
    }
    let out = root.parent().unwrap().join("z.docx");
    s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let secs = sections_of(&doc);
    let got: Vec<(NumFormat, Option<u32>, SectionStart)> = secs.iter().map(|x| (x.page_num_format, x.page_num_start, x.start)).collect();
    assert_eq!(
        got,
        [
            (NumFormat::None, Some(1), SectionStart::NextPage),
            (NumFormat::LowerRoman, None, SectionStart::NextPage),
            (NumFormat::Decimal, Some(1), SectionStart::OddPage),
        ],
        "title (front, no number), abstract (front, carrying on), chapters (main from 1, odd pages); the two chapters share a section, apart by a page break"
    );
    assert!(s.run("binder.setMeta", &json!({"project": p, "id": abs, "numbering": {"zone": "sideways"}})).is_err());
    assert!(s.run("binder.setMeta", &json!({"project": p, "id": abs, "numbering": 7})).is_err());
}

/// Items whose role is a generated list hold it in their place: the contents and the list of
/// figures after the first section, in binder order. Without captions, a list of figures is
/// empty: a warning, its heading kept.
#[test]
fn roles_place_generated_lists() {
    let (mut s, root, p) = thesis();
    let add = |s: &mut Session, v: Value| s.run("binder.add", &v).unwrap()["id"].as_str().unwrap().to_string();
    let toc = add(&mut s, json!({"project": p, "title": "Contents", "role": "contents", "index": 1}));
    let lof = add(&mut s, json!({"project": p, "title": "List of Figures", "role": "figures", "index": 2}));
    let empty = s.run("compile.run", &json!({"project": p, "path": root.parent().unwrap().join("e.docx").to_string_lossy()})).unwrap();
    assert!(empty["warnings"].to_string().contains("no Figure captions"), "{empty}");
    write_section(&root, "s0002", |s| {
        heading(s, "Methods", "Heading 1");
        s.run("references.caption", &json!({"text": "Kiln plan"})).unwrap();
    });
    let out = root.parent().unwrap().join("r.docx");
    let r = s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    assert!(!r["warnings"].to_string().contains("no Figure captions"), "{r}");
    let ids = r["sections"].to_string();
    assert!(ids.contains(&toc) && ids.contains(&lof), "{r}");
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let text = doc.plain_text(StoryRef::Body);
    let at = |t: &str| text.find(t).unwrap_or(usize::MAX);
    assert!(at("Introduction text.") < at("Contents") && at("Contents") < at("List of Figures") && at("List of Figures") < usize::MAX, "{text}");
    // A field spanning paragraphs comes back from the file as a FieldStart.
    let fields: Vec<String> = doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .filter_map(|o| match o {
            InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } => Some(instr.clone()),
            _ => None,
        })
        .collect();
    assert!(fields.iter().any(|f| f.starts_with("TOC") && !f.contains("\\c")), "a table of contents: {fields:?}");
    assert!(fields.iter().any(|f| f.starts_with("TOC") && f.contains("\\c") && f.contains("Figure")), "a list of figures: {fields:?}");
}

/// Captions numbered per chapter: Figure 1.1 and Figure 2.1 rather than 1 and 2.
#[test]
fn captions_can_be_numbered_per_chapter() {
    let (mut s, root, p) = thesis();
    for (id, title) in [("s0001", "Introduction"), ("s0002", "Methods")] {
        write_section(&root, id, |s| {
            heading(s, title, "Heading 1");
            s.run("select.text", &json!({"text": title})).unwrap();
            s.run("list.multilevelPreset", &json!({"preset": "headings"})).unwrap();
            s.run("caret.docEnd", &json!({})).unwrap();
            s.run("references.caption", &json!({"text": format!("{title} kiln")})).unwrap();
        });
    }
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text.push_str("\n[[preset]]\nname = \"Chapters\"\ncaption_numbering = \"per-chapter\"\n");
    std::fs::write(root.join(PROJECT_FILE), text).unwrap();
    let out = root.parent().unwrap().join("c.docx");
    s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let captions: Vec<String> =
        doc.body.iter().filter_map(|b| b.as_para()).filter(|p| p.props.style.as_deref() == Some("Caption")).map(|p| p.plain_text()).collect();
    assert_eq!(captions, ["Figure 1.1: Introduction kiln", "Figure 2.1: Methods kiln"]);
    let mut bad = text_with_preset(&root, "Bad", "caption_numbering = \"sometimes\"");
    bad.push(' ');
    assert!(s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy(), "preset": "Bad"})).is_err());
}

/// Add a preset to the project file; returns the file's new text.
fn text_with_preset(root: &Path, name: &str, body: &str) -> String {
    let mut text = std::fs::read_to_string(root.join(PROJECT_FILE)).unwrap();
    text.push_str(&format!("\n[[preset]]\nname = \"{name}\"\n{body}\n"));
    std::fs::write(root.join(PROJECT_FILE), &text).unwrap();
    text
}

/// normalize_figures = "inline": a Square-wrapped picture becomes In Line with Text, and the
/// floating warning goes.
#[test]
fn floating_figures_can_be_made_inline() {
    let (mut s, root, p) = thesis();
    write_section(&root, "s0001", |s| {
        s.run("text.insert", &json!({"text": "Text beside a shape. "})).unwrap();
        s.run("insert.shape", &json!({"kind": "rectangle"})).unwrap();
        s.run("arrange.wrap", &json!({"wrap": "square"})).unwrap();
    });
    let out = root.parent().unwrap().join("f.docx");
    let r = s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy()})).unwrap();
    assert!(r["warnings"].to_string().contains("floating"), "{r}");
    text_with_preset(&root, "Inline", "normalize_figures = \"inline\"");
    let r = s.run("compile.run", &json!({"project": p, "path": out.to_string_lossy(), "preset": "Inline"})).unwrap();
    assert!(!r["warnings"].to_string().contains("floating"), "{r}");
    let doc = wordcraft_engine::io::open_path(&out).unwrap();
    let floating = doc.body.iter().filter_map(|b| b.as_para()).flat_map(|p| p.objects.iter()).filter(|o| o.is_floating()).count();
    assert_eq!(floating, 0);
}

/// compile.check: comments and TODO markers are warnings, or errors with a final preset.
#[test]
fn check_finds_what_isnt_finished() {
    let (mut s, root, p) = thesis();
    write_section(&root, "s0001", |s| {
        s.run("text.insert", &json!({"text": "Results TODO here."})).unwrap();
        s.run("select.text", &json!({"text": "Results"})).unwrap();
        s.run("review.newComment", &json!({"text": "Check this"})).unwrap();
    });
    let r = s.run("compile.check", &json!({"project": p})).unwrap();
    assert_eq!(r["ok"], true, "warnings only: {r}");
    let kinds: Vec<&str> = r["problems"].as_array().unwrap().iter().filter_map(|x| x["kind"].as_str()).collect();
    assert!(kinds.contains(&"comments") && kinds.contains(&"marker"), "{r}");
    text_with_preset(&root, "Final", "final = true");
    let r = s.run("compile.check", &json!({"project": p, "preset": "Final"})).unwrap();
    assert_eq!(r["ok"], false, "{r}");
    assert!(!root.parent().unwrap().join("thesis.pdf").exists(), "a check writes nothing");
}
