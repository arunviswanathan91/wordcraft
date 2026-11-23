//! What a new project starts with (`project.new`'s `template`): a thesis, a book, or one empty
//! section. The text is WordCraft's own: section titles and nothing else, no university's or
//! publisher's wording.
//!
//! Every section of a project starts from the same base document, whose heading styles carry
//! the heading list (1 / 1.1 / 1.1.1, or "Chapter 1" for a book). Sections therefore share one
//! numbering scheme, a heading typed as `2.1 Methods` in any of them is numbered by it, and
//! compiling joins them without a second list. A chapter's file shows its own number because the
//! heading was typed with it (see `wordcraft_engine::cmd::numbering::typed`); compiling
//! renumbers by position.

use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::{Path, StoryRef};
use wordcraft_engine::Session;

use crate::project::{Item, Kind, NumberingRule, Project};

/// The templates `project.new` knows.
pub const TEMPLATES: [&str; 3] = ["blank", "thesis", "book"];

/// What a section's file starts with.
enum Body {
    /// Nothing: a generated list's place, or a page the writer fills.
    Empty,
    /// The project's title, in the Title style.
    Title,
    /// A heading without a number ("Abstract").
    Heading,
    /// A numbered chapter: its number and title.
    Chapter(u32),
    /// A heading typed as it is ("Appendix A").
    Typed,
}

struct Part {
    title: &'static str,
    role: &'static str,
    body: Body,
    /// `show` of its page numbering ("none": the title page shows no number).
    show: Option<&'static str>,
}

const fn part(title: &'static str, role: &'static str, body: Body) -> Part {
    Part { title, role, body, show: None }
}

struct Group {
    title: &'static str,
    zone: &'static str,
    parts: &'static [Part],
}

const THESIS: [Group; 3] = [
    Group {
        title: "Front matter",
        zone: "front",
        parts: &[
            Part { title: "Title page", role: "title", body: Body::Title, show: Some("none") },
            part("Declaration", "declaration", Body::Heading),
            part("Abstract", "abstract", Body::Heading),
            part("Acknowledgements", "acknowledgements", Body::Heading),
            part("Contents", "contents", Body::Empty),
            part("List of Figures", "figures", Body::Empty),
            part("List of Tables", "tables", Body::Empty),
            part("Abbreviations", "", Body::Heading),
        ],
    },
    Group {
        title: "Chapters",
        zone: "main",
        parts: &[
            part("Introduction", "introduction", Body::Chapter(1)),
            part("Literature Review", "chapter", Body::Chapter(2)),
            part("Methods", "chapter", Body::Chapter(3)),
            part("Results", "chapter", Body::Chapter(4)),
            part("Discussion", "chapter", Body::Chapter(5)),
            part("Conclusion", "conclusion", Body::Chapter(6)),
        ],
    },
    Group {
        title: "Back matter",
        zone: "continue",
        parts: &[part("References", "bibliography", Body::Empty), part("Appendix A", "appendix", Body::Typed)],
    },
];

const BOOK: [Group; 3] = [
    Group {
        title: "Front matter",
        zone: "front",
        parts: &[
            Part { title: "Title page", role: "title", body: Body::Title, show: Some("none") },
            part("Dedication", "dedication", Body::Empty),
            part("Contents", "contents", Body::Empty),
            part("Preface", "", Body::Heading),
        ],
    },
    Group {
        title: "Manuscript",
        zone: "main",
        parts: &[
            part("Chapter One", "chapter", Body::Chapter(1)),
            part("Chapter Two", "chapter", Body::Chapter(2)),
            part("Chapter Three", "chapter", Body::Chapter(3)),
        ],
    },
    Group {
        title: "Back matter",
        zone: "continue",
        parts: &[part("Acknowledgements", "acknowledgements", Body::Heading), part("About the Author", "", Body::Heading)],
    },
];

/// The document every section of a project starts from: empty, its heading styles numbered by
/// the heading list, whose first level reads "Chapter 1" when `chapter_word`.
pub fn base_document(chapter_word: bool) -> Result<Document, String> {
    let mut s = Session::new(Document::new());
    s.run("list.multilevelPreset", &json!({"preset": "headings"})).map_err(|e| e.to_string())?;
    // The preset made the caret's paragraph a heading: it goes back to body text.
    let para = s.doc.para_mut(StoryRef::Body, &Path::top(0)).map_err(|e| e.to_string())?;
    para.props.style = None;
    para.props.numbering = None;
    para.touch();
    if chapter_word {
        let h1 = wordcraft_doc::props::ParaProps { style: Some("Heading1".into()), ..Default::default() };
        let abs = s.doc.styles.resolve_para(&h1).numbering.and_then(|n| s.doc.numbering.abstract_of(n.num).map(|a| a.id));
        if let Some(abs) = abs
            && let Some(a) = std::sync::Arc::make_mut(&mut s.doc.numbering).abstracts.iter_mut().find(|a| a.id == abs)
            && let Some(l) = a.levels.first_mut()
        {
            l.text = "Chapter %1".into();
        }
    }
    Ok(s.doc.clone())
}

/// Whether the project's sections number their chapters "Chapter 1" (a book).
pub fn chapter_word(project: &Project) -> bool {
    project.file.extra.get("template").and_then(toml::Value::as_str) == Some("book")
}

/// A section's first document: `base` with what `body` says typed into it.
fn section_document(base: &Document, body: &Body, title: &str, project_title: &str) -> Result<Document, String> {
    let mut s = Session::new(base.clone());
    let typed =
        |s: &mut Session, text: String| s.run("text.insert", &json!({"text": text, "paragraphs": true})).map(|_| ()).map_err(|e| e.to_string());
    match body {
        Body::Empty => {}
        Body::Title => {
            typed(&mut s, format!("{project_title}\n"))?;
            let para = s.doc.para_mut(StoryRef::Body, &Path::top(0)).map_err(|e| e.to_string())?;
            para.props.style = Some("Title".into());
            para.touch();
        }
        Body::Heading => {
            typed(&mut s, format!("{title}\n"))?;
            let para = s.doc.para_mut(StoryRef::Body, &Path::top(0)).map_err(|e| e.to_string())?;
            para.props.style = Some("Heading1".into());
            // `num: 0`: the heading styles' numbering doesn't apply to it.
            para.props.numbering = Some(NumRef { num: 0, level: 0 });
            para.touch();
        }
        // Typed as a person would type it: Enter makes it the numbered heading.
        Body::Chapter(n) => typed(&mut s, format!("{n} {title}\n"))?,
        Body::Typed => typed(&mut s, format!("{title}\n"))?,
    }
    Ok(s.doc.clone())
}

/// The compile preset a template's project starts with.
fn preset(name: &str, per_chapter: bool) -> toml::Table {
    let mut t = toml::Table::new();
    t.insert("name".into(), name.into());
    t.insert("output".into(), "pdf".into());
    t.insert("toc".into(), false.into());
    t.insert("caption_numbering".into(), if per_chapter { "per-chapter" } else { "continuous" }.into());
    t.insert("caption_separator".into(), ".".into());
    // Pictures that could move are set in line when compiled.
    t.insert("normalize_figures".into(), "inline".into());
    t
}

/// Fill the new, empty `project` from `template`: its binder, its sections' files and its
/// compile preset. Returns the first section a writer would open.
pub fn fill(project: &mut Project, template: &str) -> Result<Option<String>, String> {
    let groups: &[Group] = match template {
        "thesis" => &THESIS,
        "book" => &BOOK,
        _ => return Err(format!("no template {:?}; use blank, thesis or book", template.chars().take(20).collect::<String>())),
    };
    let book = template == "book";
    project.file.extra.insert("template".into(), template.into());
    project.file.presets.push(preset(if book { "Book" } else { "Thesis" }, !book));
    let base = base_document(book)?;
    let project_title = project.file.title.clone();
    let mut first = None;
    for g in groups {
        let folder_id = project.file.new_id()?;
        let mut folder = Item::new(folder_id.clone(), Kind::Folder, g.title.into());
        if g.zone != "continue" {
            folder.numbering = Some(NumberingRule { zone: Some(g.zone.into()), ..Default::default() });
        }
        project.file.items.push(folder);
        for p in g.parts {
            let id = project.file.new_id()?;
            let mut item = Item::new(id.clone(), Kind::Section, p.title.into());
            item.role = p.role.into();
            item.file = Some(crate::project::file_name_for(p.title, &project.taken_files(None)));
            if p.role == "appendix" {
                item.numbering = Some(NumberingRule { zone: Some("appendix".into()), ..Default::default() });
            }
            if let Some(show) = p.show {
                item.numbering = Some(NumberingRule { show: Some(show.into()), ..Default::default() });
            }
            crate::binder::insert(&mut project.file.items, Some(&folder_id), None, item)?;
            let doc = section_document(&base, &p.body, p.title, &project_title)?;
            wordcraft_engine::io::save_path(&project.section_path(&id)?, &doc)?;
            if first.is_none() && matches!(p.body, Body::Chapter(_)) {
                first = Some(id);
            }
        }
    }
    Ok(first)
}
