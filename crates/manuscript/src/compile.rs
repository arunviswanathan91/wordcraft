//! Compile: the project's included sections, in binder order, joined into one document with a
//! preset's template and separators, its generated content updated, then exported (any format
//! WordCraft writes, by the file's extension) or opened for a look. Section files are never
//! changed: everything happens on a new document in memory.
//!
//! Joining many documents needs more than pasting one into another (`Document::adopt_blocks`
//! does the media, lists, styles, notes and comments):
//! - bookmarks are renamed when another section used the name already (Word makes `_Ref…`
//!   names for cross-references in every file), with the section's own REF, PAGEREF and NOTEREF
//!   fields following; project-wide `_wcp_` names are left alone;
//! - bibliography sources are merged by tag (the first one wins, a different one is a warning);
//! - the sections' own headers and footers give way to the template's, as Insert › Text from
//!   File does;
//! - headings numbered directly with a list of their own go back onto the heading list
//!   (`numbering.repair`), so chapters count 1, 2, 3 across files.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::para::InlineObject;
use wordcraft_doc::section::SectionStart;
use wordcraft_doc::{Block, Blocks, Document, Paragraph};
use wordcraft_engine::Session;

use crate::project::{Item, Kind, MAX_DEPTH, Project};

/// What goes between two sections.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Separator {
    /// A section break, starting a new page: the next section can have its own page setup.
    SectionBreak,
    /// A page break.
    PageBreak,
    /// Nothing: the next section carries straight on.
    None,
}

impl Separator {
    fn parse(v: Option<&str>, default: Separator) -> Result<Separator, String> {
        match v {
            None => Ok(default),
            Some("section-break") => Ok(Separator::SectionBreak),
            Some("page-break") => Ok(Separator::PageBreak),
            Some("none") => Ok(Separator::None),
            Some(x) => Err(format!("separator {:?} isn't section-break, page-break or none", x.chars().take(30).collect::<String>())),
        }
    }
}

/// A compile preset (`[[preset]]` in `project.toml`).
#[derive(Clone, Debug, PartialEq)]
pub struct Preset {
    pub name: String,
    /// A template (`.dotx`/`.docx`) whose styles, page setup, headers and footers the result
    /// takes: relative to the project folder, or absolute.
    pub template: Option<String>,
    /// The format when the output path has no extension: `pdf`, `docx`, `odt`, `tex`, `html`, `md`…
    pub output: String,
    /// Between sections under different top-level items (chapters).
    pub separator: Separator,
    /// Between sections in the same top-level folder.
    pub section_separator: Separator,
    /// Each section starts with its title as a heading (level by its depth in the binder).
    pub heading_from_title: bool,
    /// A table of contents at the start, unless the document already has one.
    pub toc: bool,
    /// Only items with these labels (empty: every included item).
    pub include_labels: Vec<String>,
}

impl Default for Preset {
    fn default() -> Self {
        Preset {
            name: "Default".into(),
            template: None,
            output: "pdf".into(),
            separator: Separator::SectionBreak,
            section_separator: Separator::PageBreak,
            heading_from_title: false,
            toc: false,
            include_labels: Vec::new(),
        }
    }
}

impl Preset {
    /// A preset from its table (unknown keys are ignored; a wrong value is an error naming it).
    pub fn from_table(t: &toml::Table) -> Result<Preset, String> {
        let d = Preset::default();
        let s = |k: &str| t.get(k).and_then(toml::Value::as_str);
        let b = |k: &str, def: bool| t.get(k).and_then(toml::Value::as_bool).unwrap_or(def);
        let labels = t
            .get("include_labels")
            .and_then(toml::Value::as_array)
            .map(|a| a.iter().filter_map(toml::Value::as_str).take(1000).map(str::to_string).collect())
            .unwrap_or_default();
        Ok(Preset {
            name: s("name").unwrap_or("Untitled preset").chars().take(200).collect(),
            template: s("template").map(str::to_string),
            output: s("output").unwrap_or(&d.output).chars().take(10).collect(),
            separator: Separator::parse(s("separator"), d.separator)?,
            section_separator: Separator::parse(s("section_separator"), d.section_separator)?,
            heading_from_title: b("heading_from_title", d.heading_from_title),
            toc: b("toc", d.toc),
            include_labels: labels,
        })
    }
}

/// The project's presets, or the default one when it has none.
pub fn presets(project: &Project) -> Result<Vec<Preset>, String> {
    if project.file.presets.is_empty() {
        return Ok(vec![Preset::default()]);
    }
    project.file.presets.iter().map(Preset::from_table).collect()
}

/// The preset called `name` (default: the first).
pub fn preset(project: &Project, name: Option<&str>) -> Result<Preset, String> {
    let all = presets(project)?;
    match name {
        None => all.into_iter().next().ok_or_else(|| "no preset".to_string()),
        Some(n) => all.into_iter().find(|p| p.name == n).ok_or_else(|| format!("no preset {:?}", n.chars().take(60).collect::<String>())),
    }
}

/// A compiled manuscript, before it is written anywhere.
pub struct Compiled {
    pub doc: Document,
    /// The sections joined, in order.
    pub sections: Vec<String>,
    /// What the writer should know about (a missing file, a reference with no target…).
    pub warnings: Vec<String>,
}

/// The template's path: relative to the project folder (it must stay inside it), or absolute.
fn template_path(project: &Project, t: &str) -> Result<PathBuf, String> {
    let p = Path::new(t);
    if p.is_absolute() {
        return Ok(p.to_path_buf());
    }
    if p.components().any(|c| !matches!(c, Component::Normal(_) | Component::CurDir)) {
        return Err(format!("template {t:?} must be inside the project folder, or an absolute path"));
    }
    Ok(project.root.join(p))
}

/// The base document, with an empty body: the template's styles, numbering, page setup,
/// headers and footers; without a template, the first included section's, so the manuscript
/// keeps the styles its sections were written with (a heading list in their heading styles
/// numbers on through every chapter); a new document when there's neither.
fn base(project: &Project, preset: &Preset) -> Result<Document, String> {
    let Some(t) = &preset.template else {
        let first = included(&project.file.items, preset).into_iter().find_map(|(i, _, _)| {
            let path = project.section_path(&i.id).ok()?;
            wordcraft_engine::file_access::check_read(&path.to_string_lossy()).ok()?;
            wordcraft_engine::io::open_path(&path).ok()
        });
        let mut doc = first.unwrap_or_else(Document::new);
        // Only the setup: its text, notes, comments, sources and revisions come with the section.
        doc.body = vec![Arc::new(Block::Para(Paragraph::new()))];
        doc.parts.retain(|_, part| matches!(part.kind, wordcraft_doc::PartKind::Header | wordcraft_doc::PartKind::Footer));
        doc.comments.clear();
        doc.sources.clear();
        doc.revisions.clear();
        return Ok(doc);
    };
    let path = template_path(project, t)?;
    wordcraft_engine::file_access::check_read(&path.to_string_lossy())?;
    let mut doc = wordcraft_engine::io::open_path(&path).map_err(|e| format!("template {t}: {e}"))?;
    doc.body = vec![Arc::new(Block::Para(Paragraph::new()))];
    Ok(doc)
}

/// Every paragraph in `blocks`, tables included, to change (`depth` bounds nested tables).
fn each_para(blocks: &mut Blocks, depth: usize, f: &mut dyn FnMut(&mut Paragraph)) {
    if depth > MAX_DEPTH * 4 {
        return;
    }
    for b in blocks.iter_mut() {
        match Arc::make_mut(b) {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                for row in &mut t.rows {
                    for cell in &mut row.cells {
                        each_para(&mut cell.blocks, depth + 1, f);
                    }
                }
            }
        }
    }
}

/// Every paragraph in `blocks`, to read.
fn each_para_ref(blocks: &Blocks, depth: usize, f: &mut dyn FnMut(&Paragraph)) {
    if depth > MAX_DEPTH * 4 {
        return;
    }
    for b in blocks {
        match &**b {
            Block::Para(p) => f(p),
            Block::Table(t) => {
                for row in &t.rows {
                    for cell in &row.cells {
                        each_para_ref(&cell.blocks, depth + 1, f);
                    }
                }
            }
        }
    }
}

/// The bookmark a REF, PAGEREF or NOTEREF field points at.
fn ref_target(instr: &str) -> Option<(&str, &str)> {
    let mut words = instr.split_whitespace();
    let name = words.next()?;
    let target = words.next()?;
    ["REF", "PAGEREF", "NOTEREF"].iter().any(|n| name.eq_ignore_ascii_case(n)).then_some((name, target))
}

/// Rename `doc`'s body bookmarks that `taken` has (not the project-wide `_wcp_` ones) to
/// `<section>_<name>`, and its REF/PAGEREF/NOTEREF fields with them; then add its names to
/// `taken`.
fn rename_bookmarks(doc: &mut Document, section: &str, taken: &mut HashSet<String>) {
    let mut names: Vec<String> = Vec::new();
    each_para_ref(&doc.body, 0, &mut |p| {
        for o in &p.objects {
            if let InlineObject::BookmarkStart { name } = o {
                names.push(name.clone());
            }
        }
    });
    let mut renamed: HashMap<String, String> = HashMap::new();
    for n in &names {
        if n.starts_with("_wcp_") || !taken.contains(n) || renamed.contains_key(n) {
            continue;
        }
        let mut fresh = format!("{section}_{n}");
        let mut k = 2;
        while taken.contains(&fresh) || names.contains(&fresh) {
            fresh = format!("{section}_{n}_{k}");
            k += 1;
            if k > 10_000 {
                break;
            }
        }
        renamed.insert(n.clone(), fresh);
    }
    if !renamed.is_empty() {
        each_para(&mut doc.body, 0, &mut |p| {
            let mut changed = false;
            for o in p.objects.iter_mut() {
                match o {
                    InlineObject::BookmarkStart { name } | InlineObject::BookmarkEnd { name } => {
                        if let Some(n) = renamed.get(name.as_str()) {
                            *name = n.clone();
                            changed = true;
                        }
                    }
                    InlineObject::Field { instr, .. } => {
                        let new = ref_target(instr)
                            .and_then(|(kind, target)| renamed.get(target).map(|n| (kind.to_string(), target.to_string(), n.clone())));
                        if let Some((kind, target, n)) = new {
                            *instr = instr.replacen(&format!("{kind} {target}"), &format!("{kind} {n}"), 1);
                            changed = true;
                        }
                    }
                    _ => {}
                }
            }
            if changed {
                p.touch();
            }
        });
    }
    taken.extend(names.into_iter().map(|n| renamed.get(&n).cloned().unwrap_or(n)));
}

/// Headers, footers and the different first page of `blocks`' own section breaks give way to
/// the template's (Insert › Text from File does the same).
fn unlink_sections(blocks: &mut Blocks) {
    each_para(blocks, 0, &mut |p| {
        if let Some(sec) = p.section.as_deref_mut() {
            sec.headers = Default::default();
            sec.footers = Default::default();
            sec.title_page = false;
        }
    });
}

/// The included sections in binder order, each with its top-level item and depth.
fn included<'a>(items: &'a [Item], preset: &Preset) -> Vec<(&'a Item, &'a str, usize)> {
    fn walk<'a>(items: &'a [Item], top: Option<&'a str>, depth: usize, preset: &Preset, out: &mut Vec<(&'a Item, &'a str, usize)>) {
        if depth > MAX_DEPTH {
            return;
        }
        for i in items {
            if !i.include || i.id == crate::project::TRASH {
                continue;
            }
            let top = top.unwrap_or(i.id.as_str());
            match i.kind {
                Kind::Section => {
                    if preset.include_labels.is_empty() || preset.include_labels.contains(&i.label) {
                        out.push((i, top, depth));
                    }
                }
                Kind::Folder => walk(&i.children, Some(top), depth + 1, preset, out),
            }
        }
    }
    let mut out = Vec::new();
    walk(items, None, 1, preset, &mut out);
    out
}

/// Join the project's included sections with `preset`.
pub fn join(project: &Project, preset: &Preset) -> Result<Compiled, String> {
    let mut doc = base(project, preset)?;
    let mut warnings = Vec::new();
    let mut sections = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut sources: BTreeMap<String, usize> = doc.sources.iter().enumerate().map(|(i, s)| (s.tag.clone(), i)).collect();
    let mut perms = 0u32;
    // The base body's empty paragraph goes once something is added.
    let mut body: Blocks = Vec::new();
    let mut last_top: Option<String> = None;
    for (item, top, depth) in included(&project.file.items, preset) {
        let path = project.section_path(&item.id)?;
        let opened = wordcraft_engine::file_access::check_read(&path.to_string_lossy()).and_then(|()| wordcraft_engine::io::open_path(&path));
        let mut src = match opened {
            Ok(d) => d,
            Err(e) => {
                warnings.push(format!("section {:?} ({}) left out: {e}", item.title, item.id));
                continue;
            }
        };
        rename_bookmarks(&mut src, &item.id, &mut taken);
        for s in std::mem::take(&mut src.sources) {
            match sources.get(&s.tag) {
                Some(&i) if doc.sources.get(i) != Some(&s) => {
                    warnings.push(format!("source {:?} differs between sections; the first one is used ({})", s.tag, item.id));
                }
                Some(_) => {}
                None => {
                    sources.insert(s.tag.clone(), doc.sources.len());
                    doc.sources.push(s);
                }
            }
        }
        let mut blocks: Blocks = doc.adopt_blocks(src, false, &mut perms).into_iter().map(Arc::new).collect();
        unlink_sections(&mut blocks);
        if preset.heading_from_title {
            let level = depth.clamp(1, 9);
            let style = ensure_heading(&mut doc, level);
            blocks.insert(0, Arc::new(Block::Para(Paragraph::with_text(&item.title, Default::default()).styled(&style))));
        }
        if blocks.is_empty() {
            blocks.push(Arc::new(Block::Para(Paragraph::new())));
        }
        // What goes between this section and the one before.
        if !body.is_empty() {
            let sep = if last_top.as_deref() == Some(top) { preset.section_separator } else { preset.separator };
            separate(&doc, &mut body, &mut blocks, sep);
        }
        body.extend(blocks);
        last_top = Some(top.to_string());
        sections.push(item.id.clone());
    }
    if body.is_empty() {
        warnings.push("no sections to compile: every one is excluded, empty of files, or filtered out by the preset".into());
        body.push(Arc::new(Block::Para(Paragraph::new())));
    }
    doc.body = body;
    // Floating pictures move when the text above them changes (see Figures in the design).
    let mut floating = 0usize;
    each_para_ref(&doc.body, 0, &mut |p| floating += p.objects.iter().filter(|o| o.is_floating()).count());
    if floating > 0 {
        warnings.push(format!(
            "{floating} floating picture(s) or shape(s): they can move when the text above them changes; In Line with Text keeps a figure in place"
        ));
    }
    Ok(Compiled { doc, sections, warnings })
}

/// Make sure `Heading{level}` exists in `doc` (from the built-in styles), and return its id.
fn ensure_heading(doc: &mut Document, level: usize) -> String {
    let id = format!("Heading{level}");
    if doc.styles.get(&id).is_none() {
        let builtin = wordcraft_doc::styles::StyleSheet::builtin();
        for sid in [id.clone(), format!("{id}Char")] {
            if doc.styles.get(&sid).is_none()
                && let Some(st) = builtin.get(&sid).cloned()
            {
                Arc::make_mut(&mut doc.styles).upsert(st);
            }
        }
    }
    id
}

/// Put `sep` between the body so far and `next`.
fn separate(doc: &Document, body: &mut Blocks, next: &mut Blocks, sep: Separator) {
    match sep {
        Separator::None => {}
        Separator::PageBreak => {
            if !matches!(next.first().map(|b| &**b), Some(Block::Para(_))) {
                next.insert(0, Arc::new(Block::Para(Paragraph::new())));
            }
            if let Some(Block::Para(p)) = next.first_mut().map(Arc::make_mut) {
                p.props.page_break_before = Some(true);
                p.touch();
            }
        }
        Separator::SectionBreak => {
            // A section ends at a paragraph that carries its properties: the template's page
            // setup, headers and footers, starting the next on a new page.
            if !matches!(body.last().map(|b| &**b), Some(Block::Para(p)) if p.section.is_none()) {
                body.push(Arc::new(Block::Para(Paragraph::new())));
            }
            if let Some(Block::Para(p)) = body.last_mut().map(Arc::make_mut) {
                let mut sect = doc.last_section.clone();
                sect.start = SectionStart::NextPage;
                p.section = Some(Box::new(sect));
                p.touch();
            }
        }
    }
}

/// The REF/PAGEREF/NOTEREF fields in the body whose bookmark isn't there.
fn missing_references(doc: &Document) -> Vec<String> {
    let mut marks = HashSet::new();
    let mut refs = Vec::new();
    each_para_ref(&doc.body, 0, &mut |p| {
        for o in &p.objects {
            match o {
                InlineObject::BookmarkStart { name } => {
                    marks.insert(name.clone());
                }
                InlineObject::Field { instr, .. } => {
                    if let Some((_, target)) = ref_target(instr) {
                        refs.push(target.to_string());
                    }
                }
                _ => {}
            }
        }
    });
    let mut missing: Vec<String> = refs.into_iter().filter(|r| !marks.contains(r)).collect();
    missing.sort();
    missing.dedup();
    missing.truncate(200);
    missing
}

/// Join, then update what the whole document generates: a table of contents when the preset
/// asks and there's none, every field (captions, cross-references, the bibliography…), the
/// tables of contents and figures, and heading numbers that a section file numbered with a list
/// of its own.
pub fn compile(project: &Project, preset: &Preset) -> Result<Compiled, String> {
    let Compiled { doc, sections, mut warnings } = join(project, preset)?;
    let mut s = Session::new(doc);
    let has_toc = {
        let mut found = false;
        each_para_ref(&s.doc.body, 0, &mut |p| {
            found |= p
                .objects
                .iter()
                .any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.trim_start().to_ascii_uppercase().starts_with("TOC")));
        });
        found
    };
    if preset.toc && !has_toc {
        // In a paragraph of its own at the start, so no heading is split around it.
        s.doc.body.insert(0, Arc::new(Block::Para(Paragraph::new())));
        s.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(0, 0));
        if let Err(e) = s.run("references.toc", &json!({})) {
            warnings.push(format!("table of contents: {e}"));
        }
    }
    let _ = s.run("numbering.repair", &json!({"kinds": ["otherList", "restart"]}));
    if let Err(e) = s.run("references.updateFields", &json!({"all": true})) {
        warnings.push(format!("updating fields: {e}"));
    }
    // A document without a table of contents or figures says so; that's not a problem here.
    let _ = s.run("references.updateToc", &json!({}));
    let _ = s.run("references.updateFigures", &json!({}));
    for m in missing_references(&s.doc) {
        warnings.push(format!("a cross-reference points at {m:?}, which isn't in the compiled document"));
    }
    Ok(Compiled { doc: s.doc.clone(), sections, warnings })
}

/// The compile answer as JSON.
pub fn summary(c: &Compiled, extra: Value) -> Value {
    let mut v = json!({"sections": c.sections, "warnings": c.warnings, "words": c.doc.word_count()});
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    v
}
