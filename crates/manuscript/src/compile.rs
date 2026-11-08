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
//!
//! The binder also says how the manuscript is put together:
//! - **Roles.** An item whose role is `contents`, `figures`, `tables`, `equations`,
//!   `bibliography` or `index` has no text of its own: compile puts that generated list there.
//! - **Page numbering zones** ([`NumberingRule`], inherited through folders): front matter in
//!   roman numerals from i, the main text in arabic from 1, appendices carrying on, items with no
//!   number shown or none on their first page, items starting on a right-hand page. Each change
//!   starts a new section; the numbering goes in that section's properties.
//! - **Captions per chapter** (`caption_numbering = "per-chapter"`): Figure 3.2 rather than
//!   Figure 14, written the way Insert Caption writes chapter numbers.
//! - **Figures** (`normalize_figures`): floating pictures can become In Line with Text, or sit at
//!   the top of the page, so the text can't move them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ, Wrap};
use wordcraft_doc::section::{NumFormat, SectionProps, SectionStart};
use wordcraft_doc::{Block, Blocks, CharProps, Document, Paragraph};
use wordcraft_engine::Session;

use crate::project::{Item, Kind, MAX_DEPTH, NumberingRule, Project};

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
            Some(x) => Err(format!("separator {:?} isn't section-break, page-break or none", short(x))),
        }
    }
}

/// What a compile does with floating pictures.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Figures {
    /// Leave them; each is a warning.
    Keep,
    /// Square, Tight, Through and Top and Bottom pictures become In Line with Text, kept with
    /// the next paragraph.
    Inline,
    /// They sit centred across the top of the page, Top and Bottom wrapped, anchor locked.
    TopBottom,
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
    /// A table of contents at the start, unless the document already has one (or a contents item).
    pub toc: bool,
    /// Only items with these labels (empty: every included item).
    pub include_labels: Vec<String>,
    /// Captions numbered per chapter (`Figure 3.2`) instead of straight through.
    pub per_chapter_captions: bool,
    /// What goes between a caption's chapter number and its own.
    pub caption_separator: String,
    pub figures: Figures,
    /// Comments, tracked changes and placeholder markers are errors, not warnings (`compile.check`).
    pub final_check: bool,
    /// Text that marks unfinished work (`compile.check`).
    pub markers: Vec<String>,
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
            per_chapter_captions: false,
            caption_separator: ".".into(),
            figures: Figures::Keep,
            final_check: false,
            markers: vec!["TODO".into(), "[CITE]".into()],
        }
    }
}

fn short(s: &str) -> String {
    s.chars().take(40).collect()
}

/// The strings in array `k` of `t` (at most 1000).
fn strings(t: &toml::Table, k: &str) -> Option<Vec<String>> {
    t.get(k)
        .and_then(toml::Value::as_array)
        .map(|a| a.iter().filter_map(toml::Value::as_str).take(1000).map(|s| s.chars().take(200).collect()).collect())
}

impl Preset {
    /// A preset from its table (unknown keys are ignored; a wrong value is an error naming it).
    pub fn from_table(t: &toml::Table) -> Result<Preset, String> {
        let d = Preset::default();
        let s = |k: &str| t.get(k).and_then(toml::Value::as_str);
        let b = |k: &str, def: bool| t.get(k).and_then(toml::Value::as_bool).unwrap_or(def);
        let per_chapter = match s("caption_numbering") {
            None | Some("continuous") => false,
            Some("per-chapter") => true,
            Some(x) => return Err(format!("caption_numbering {:?} isn't continuous or per-chapter", short(x))),
        };
        let figures = match s("normalize_figures") {
            None | Some("keep") => Figures::Keep,
            Some("inline") => Figures::Inline,
            Some("top-bottom") => Figures::TopBottom,
            Some(x) => return Err(format!("normalize_figures {:?} isn't keep, inline or top-bottom", short(x))),
        };
        Ok(Preset {
            name: s("name").unwrap_or("Untitled preset").chars().take(200).collect(),
            template: s("template").map(str::to_string),
            output: s("output").unwrap_or(&d.output).chars().take(10).collect(),
            separator: Separator::parse(s("separator"), d.separator)?,
            section_separator: Separator::parse(s("section_separator"), d.section_separator)?,
            heading_from_title: b("heading_from_title", d.heading_from_title),
            toc: b("toc", d.toc),
            include_labels: strings(t, "include_labels").unwrap_or_default(),
            per_chapter_captions: per_chapter,
            caption_separator: s("caption_separator").unwrap_or(&d.caption_separator).chars().take(4).collect(),
            figures,
            final_check: b("final", d.final_check),
            markers: strings(t, "markers").unwrap_or(d.markers),
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
        Some(n) => all.into_iter().find(|p| p.name == n).ok_or_else(|| format!("no preset {:?}", short(n))),
    }
}

/// A page numbering zone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Zone {
    /// As the item before.
    #[default]
    Continue,
    Front,
    Main,
    Appendix,
    None,
}

/// Where a page number is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Show {
    #[default]
    All,
    NotFirst,
    None,
}

/// What page a section starts on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum StartOn {
    #[default]
    Next,
    Odd,
    Even,
}

/// An item's numbering with the folders above it applied.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct Rule {
    pub zone: Zone,
    pub show: Show,
    pub start_on: StartOn,
}

impl Rule {
    /// `self` with what `r` sets.
    pub fn with(self, r: &NumberingRule) -> Result<Rule, String> {
        let zone = match r.zone.as_deref() {
            None => self.zone,
            Some("continue") => Zone::Continue,
            Some("front") => Zone::Front,
            Some("main") => Zone::Main,
            Some("appendix") => Zone::Appendix,
            Some("none") => Zone::None,
            Some(x) => return Err(format!("numbering zone {:?} isn't continue, front, main, appendix or none", short(x))),
        };
        let show = match r.show.as_deref() {
            None => self.show,
            Some("all") => Show::All,
            Some("not-first") => Show::NotFirst,
            Some("none") => Show::None,
            Some(x) => return Err(format!("numbering show {:?} isn't all, not-first or none", short(x))),
        };
        let start_on = match r.start_on.as_deref() {
            None => self.start_on,
            Some("next") => StartOn::Next,
            Some("odd") => StartOn::Odd,
            Some("even") => StartOn::Even,
            Some(x) => return Err(format!("numbering start_on {:?} isn't next, odd or even", short(x))),
        };
        Ok(Rule { zone, show, start_on })
    }
}

/// Roles whose items hold a generated list rather than text.
pub const GENERATED: [&str; 6] = ["contents", "figures", "tables", "equations", "bibliography", "index"];

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

/// Open section `id`'s document.
fn open_section(project: &Project, id: &str) -> Result<Document, String> {
    let path = project.section_path(id)?;
    wordcraft_engine::file_access::check_read(&path.to_string_lossy())?;
    wordcraft_engine::io::open_path(&path)
}

/// The base document, with an empty body: the template's styles, numbering, page setup,
/// headers and footers; without a template, the first included section's, so the manuscript
/// keeps the styles its sections were written with (a heading list in their heading styles
/// numbers on through every chapter); a new document when there's neither.
fn base(project: &Project, preset: &Preset, entries: &[Entry<'_>]) -> Result<Document, String> {
    let Some(t) = &preset.template else {
        let first = entries.iter().filter(|e| !e.generated()).find_map(|e| open_section(project, &e.item.id).ok());
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

/// Caption labels numbered per chapter.
const CAPTION_LABELS: [&str; 3] = ["Figure", "Table", "Equation"];

/// Number `blocks`' captions per chapter: each `SEQ Figure` (Table, Equation) field without a
/// chapter part gets `STYLEREF 1 \s` and `sep` before it and restarts after each Heading 1
/// (`\s 1`), as Insert Caption writes them with Include chapter number.
fn per_chapter_captions(blocks: &mut Blocks, sep: &str) {
    each_para(blocks, 0, &mut |p| {
        // The objects to change, last first, so earlier offsets stay right.
        let at: Vec<(usize, usize)> = p
            .text
            .char_indices()
            .filter(|(_, c)| *c == OBJ)
            .enumerate()
            .filter_map(|(k, (off, _))| match p.objects.get(k) {
                Some(InlineObject::Field { instr, .. }) => {
                    let mut w = instr.split_whitespace();
                    let seq = w.next().is_some_and(|n| n.eq_ignore_ascii_case("SEQ"));
                    let label = w.next().is_some_and(|l| CAPTION_LABELS.iter().any(|c| c.eq_ignore_ascii_case(l)));
                    let plain = !instr.split_whitespace().any(|t| matches!(t, "\\s" | "\\c" | "\\h" | "\\r"));
                    (seq && label && plain).then_some((k, off))
                }
                _ => None,
            })
            .collect();
        for (k, off) in at.into_iter().rev() {
            if let Some(InlineObject::Field { instr, .. }) = p.objects.get_mut(k) {
                instr.push_str(" \\s 1");
            }
            let props = p.props_of_char(off).clone();
            let field = InlineObject::Field {
                instr: "STYLEREF 1 \\s".into(),
                result: String::new(),
                locked: false,
                dirty: true,
                code: false,
                form: String::new(),
            };
            if p.insert_text(off, sep, &props).is_ok() {
                let _ = p.insert_object(off, field, &props);
            }
        }
    });
}

/// Floating pictures and shapes in `blocks` as `figures` says; returns how many are left floating.
fn normalize_figures(blocks: &mut Blocks, figures: Figures) -> usize {
    use wordcraft_doc::para::{Anchor, FloatAlign};
    let mut left = 0usize;
    each_para(blocks, 0, &mut |p| {
        let mut moved = false;
        for o in p.objects.iter_mut() {
            if !o.is_floating() {
                continue;
            }
            let wraps_text = o.frame().is_some_and(|(_, _, f)| matches!(f.wrap, Wrap::Square | Wrap::Tight | Wrap::Through | Wrap::TopAndBottom));
            match (figures, o.float_mut()) {
                (Figures::Inline, Some(f)) if wraps_text => {
                    f.wrap = Wrap::Inline;
                    moved = true;
                }
                (Figures::TopBottom, Some(f)) if wraps_text => {
                    f.wrap = Wrap::TopAndBottom;
                    (f.h_rel, f.v_rel) = (Anchor::Margin, Anchor::Margin);
                    (f.h_align, f.v_align) = (Some(FloatAlign::Center), Some(FloatAlign::Start));
                    (f.x, f.y) = (0.0, 0.0);
                    if f.dist_bottom == 0.0 {
                        f.dist_bottom = 12.0;
                    }
                    f.locked = true;
                    moved = true;
                }
                _ => left += 1,
            }
        }
        if moved {
            if figures == Figures::Inline {
                p.props.keep_next = Some(true);
            }
            p.touch();
        }
    });
    left
}

/// An included item, as compile meets it.
struct Entry<'a> {
    item: &'a Item,
    /// The top-level item it is under (itself at the top level).
    top: &'a str,
    depth: usize,
    rule: Rule,
}

impl Entry<'_> {
    fn generated(&self) -> bool {
        GENERATED.contains(&self.item.role.as_str())
    }
}

/// The included sections in binder order, with their top-level item, depth and numbering.
fn included<'a>(items: &'a [Item], preset: &Preset) -> Result<Vec<Entry<'a>>, String> {
    fn walk<'a>(items: &'a [Item], top: Option<&'a str>, depth: usize, rule: Rule, preset: &Preset, out: &mut Vec<Entry<'a>>) -> Result<(), String> {
        if depth > MAX_DEPTH {
            return Ok(());
        }
        for i in items {
            if !i.include || i.id == crate::project::TRASH {
                continue;
            }
            let top = top.unwrap_or(i.id.as_str());
            let rule = match &i.numbering {
                Some(r) => rule.with(r).map_err(|e| format!("{} ({}): {e}", i.title, i.id))?,
                None => rule,
            };
            match i.kind {
                Kind::Section => {
                    if preset.include_labels.is_empty() || preset.include_labels.contains(&i.label) {
                        out.push(Entry { item: i, top, depth, rule });
                    }
                }
                Kind::Folder => walk(&i.children, Some(top), depth + 1, rule, preset, out)?,
            }
        }
        Ok(())
    }
    let mut out = Vec::new();
    walk(items, None, 1, Rule::default(), preset, &mut out)?;
    Ok(out)
}

/// A generated list waiting in the joined body: its placeholder paragraph's index and role.
struct Placeholder {
    at: usize,
    role: String,
    title: String,
}

/// A section of the compiled document as it is laid out: the page numbering its items share.
#[derive(Clone, Copy)]
struct Sect {
    rule: Rule,
    /// The zone in force (`Continue` resolved), and whether this section starts it.
    zone: Zone,
    starts_zone: bool,
}

/// Put the numbering of `sect` into the section properties `p`.
fn apply(p: &mut SectionProps, sect: Sect) {
    match sect.zone {
        Zone::Front => {
            p.page_num_format = NumFormat::LowerRoman;
            p.page_num_set = true;
            if sect.starts_zone {
                p.page_num_start = Some(1);
            }
        }
        Zone::Main => {
            p.page_num_format = NumFormat::Decimal;
            p.page_num_set = true;
            if sect.starts_zone {
                p.page_num_start = Some(1);
            }
        }
        Zone::Appendix => {
            p.page_num_format = NumFormat::Decimal;
            p.page_num_set = true;
        }
        Zone::None => {
            p.page_num_format = NumFormat::None;
            p.page_num_set = true;
        }
        Zone::Continue => {}
    }
    match sect.rule.show {
        Show::All => {}
        Show::NotFirst => p.title_page = true,
        Show::None => {
            p.page_num_format = NumFormat::None;
            p.page_num_set = true;
        }
    }
    match sect.rule.start_on {
        StartOn::Next => {}
        StartOn::Odd => p.start = SectionStart::OddPage,
        StartOn::Even => p.start = SectionStart::EvenPage,
    }
}

/// Join the project's included sections with `preset`; the generated lists are left as
/// placeholders for [`compile`].
fn join_with_placeholders(project: &Project, preset: &Preset) -> Result<(Compiled, Vec<Placeholder>), String> {
    let entries = included(&project.file.items, preset)?;
    let mut doc = base(project, preset, &entries)?;
    let template_section = doc.last_section.clone();
    let mut warnings = Vec::new();
    let mut sections = Vec::new();
    let mut placeholders = Vec::new();
    let mut taken: HashSet<String> = HashSet::new();
    let mut sources: BTreeMap<String, usize> = doc.sources.iter().enumerate().map(|(i, s)| (s.tag.clone(), i)).collect();
    let mut perms = 0u32;
    let mut body: Blocks = Vec::new();
    let mut last: Option<(&str, Sect)> = None;
    let mut floating = 0usize;
    for e in &entries {
        let item = e.item;
        let generated = e.generated();
        let mut blocks: Blocks = if generated {
            // A heading of its own (the table of contents and the bibliography bring theirs), then
            // the list's place.
            let mut b: Blocks = Vec::new();
            if !matches!(item.role.as_str(), "contents" | "bibliography") {
                let style = ensure_style(&mut doc, "TOCHeading");
                b.push(Arc::new(Block::Para(Paragraph::with_text(&item.title, CharProps::default()).styled(&style))));
            }
            b.push(Arc::new(Block::Para(Paragraph::new())));
            // And a paragraph after it, which carries a section break that follows: the
            // command filling the list in replaces the placeholder, never the break.
            b.push(Arc::new(Block::Para(Paragraph::new())));
            b
        } else {
            let mut src = match open_section(project, &item.id) {
                Ok(d) => d,
                Err(err) => {
                    warnings.push(format!("section {:?} ({}) left out: {err}", item.title, item.id));
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
            if preset.per_chapter_captions {
                per_chapter_captions(&mut blocks, &preset.caption_separator);
            }
            floating += normalize_figures(&mut blocks, preset.figures);
            if preset.heading_from_title {
                let style = ensure_style(&mut doc, &format!("Heading{}", e.depth.clamp(1, 9)));
                blocks.insert(0, Arc::new(Block::Para(Paragraph::with_text(&item.title, CharProps::default()).styled(&style))));
            }
            blocks
        };
        if blocks.is_empty() {
            blocks.push(Arc::new(Block::Para(Paragraph::new())));
        }
        // This item's numbering: a change from the item before starts a new section.
        let prev = last.map(|(_, s)| s);
        let zone = if e.rule.zone == Zone::Continue { prev.map_or(Zone::Continue, |p| p.zone) } else { e.rule.zone };
        let here = Sect { rule: e.rule, zone, starts_zone: prev.is_none_or(|p| p.zone != zone) && zone != Zone::Continue };
        let mut starts = last.is_none();
        if let Some((top, before)) = last {
            let wanted = if top == e.top { preset.section_separator } else { preset.separator };
            let changes = before.rule != here.rule || before.zone != here.zone;
            let sep = if changes { Separator::SectionBreak } else { wanted };
            if sep == Separator::SectionBreak {
                let mut sect = template_section.clone();
                sect.start = SectionStart::NextPage;
                apply(&mut sect, before);
                end_section(&mut body, sect);
                starts = true;
            } else {
                separate(&mut blocks, sep);
            }
        }
        // A new section takes this item's numbering; one carrying on keeps the section's.
        let current = if starts { here } else { last.map_or(here, |(_, s)| s) };
        if generated {
            let at = body.len() + blocks.len().saturating_sub(2);
            placeholders.push(Placeholder { at, role: item.role.clone(), title: item.title.clone() });
        }
        body.extend(blocks);
        last = Some((e.top, current));
        sections.push(item.id.clone());
    }
    if body.is_empty() {
        warnings.push("no sections to compile: every one is excluded, empty of files, or filtered out by the preset".into());
        body.push(Arc::new(Block::Para(Paragraph::new())));
    }
    if let Some((_, s)) = last {
        apply(&mut doc.last_section, s);
    }
    doc.body = body;
    if floating > 0 {
        warnings.push(format!("{floating} floating picture(s) or shape(s): they can move when the text above them changes; In Line with Text (normalize_figures = \"inline\") keeps a figure in place"));
    }
    Ok((Compiled { doc, sections, warnings }, placeholders))
}

/// Join the project's included sections with `preset` (generated lists not filled in).
pub fn join(project: &Project, preset: &Preset) -> Result<Compiled, String> {
    join_with_placeholders(project, preset).map(|(c, _)| c)
}

/// Make sure style `id` exists in `doc` (from the built-in styles), and return its id.
fn ensure_style(doc: &mut Document, id: &str) -> String {
    if doc.styles.get(id).is_none() {
        let builtin = wordcraft_doc::styles::StyleSheet::builtin();
        for sid in [id.to_string(), format!("{id}Char")] {
            if doc.styles.get(&sid).is_none()
                && let Some(st) = builtin.get(&sid).cloned()
            {
                Arc::make_mut(&mut doc.styles).upsert(st);
            }
        }
    }
    id.to_string()
}

/// Put a page break (or nothing) before `next`.
fn separate(next: &mut Blocks, sep: Separator) {
    if sep != Separator::PageBreak {
        return;
    }
    if !matches!(next.first().map(|b| &**b), Some(Block::Para(_))) {
        next.insert(0, Arc::new(Block::Para(Paragraph::new())));
    }
    if let Some(Block::Para(p)) = next.first_mut().map(Arc::make_mut) {
        p.props.page_break_before = Some(true);
        p.touch();
    }
}

/// End the section so far with properties `sect`: a section ends at a paragraph that carries
/// its properties.
fn end_section(body: &mut Blocks, sect: SectionProps) {
    if !matches!(body.last().map(|b| &**b), Some(Block::Para(p)) if p.section.is_none()) {
        body.push(Arc::new(Block::Para(Paragraph::new())));
    }
    if let Some(Block::Para(p)) = body.last_mut().map(Arc::make_mut) {
        p.section = Some(Box::new(sect));
        p.touch();
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

/// A field's code, simple or spanning paragraphs.
fn field_instr(o: &InlineObject) -> Option<&str> {
    match o {
        InlineObject::Field { instr, .. } | InlineObject::FieldStart { instr, .. } => Some(instr),
        _ => None,
    }
}

/// Whether a field code is a table of contents (not a table of figures).
fn is_table_of_contents(instr: &str) -> bool {
    let mut w = instr.split_whitespace();
    w.next().is_some_and(|n| n.eq_ignore_ascii_case("TOC")) && !w.any(|t| matches!(t, "\\c" | "\\a"))
}

/// How many captions of `label` (`SEQ label` fields) the body has.
fn captions(doc: &Document, label: &str) -> usize {
    let mut n = 0usize;
    each_para_ref(&doc.body, 0, &mut |p| {
        n += p
            .objects
            .iter()
            .filter_map(field_instr)
            .filter(|instr| {
                let mut w = instr.split_whitespace();
                w.next().is_some_and(|x| x.eq_ignore_ascii_case("SEQ")) && w.next().is_some_and(|x| x.eq_ignore_ascii_case(label))
            })
            .count();
    });
    n
}

/// Fill a generated list in at paragraph `at` with the engine's own command.
fn generate(s: &mut Session, ph: &Placeholder) -> Result<(), String> {
    // A list of figures with no captions to list has nothing to show yet (its heading stays).
    let label = match ph.role.as_str() {
        "figures" => Some("Figure"),
        "tables" => Some("Table"),
        "equations" => Some("Equation"),
        _ => None,
    };
    if let Some(l) = label
        && captions(&s.doc, l) == 0
    {
        return Err(format!("{}: no {l} captions yet, so the list is empty", ph.title));
    }
    s.sel = wordcraft_engine::Selection::caret(wordcraft_doc::Pos::body(ph.at, 0));
    let (id, params) = match ph.role.as_str() {
        "contents" => ("references.toc", json!({"title": ph.title})),
        "figures" => ("references.tableOfFigures", json!({"label": "Figure"})),
        "tables" => ("references.tableOfFigures", json!({"label": "Table"})),
        "equations" => ("references.tableOfFigures", json!({"label": "Equation"})),
        "bibliography" => ("references.bibliography", json!({"title": ph.title})),
        "index" => ("references.index", json!({})),
        _ => return Ok(()),
    };
    s.run(id, &params).map(|_| ()).map_err(|e| format!("{} ({}): {e}", ph.title, ph.role))
}

/// Join, then fill in and update what the whole document generates: the lists the binder's
/// roles place, a table of contents when the preset asks and there's none, heading numbers
/// that a section file numbered with a list of its own, every field (captions,
/// cross-references, the bibliography…), and the tables of contents and figures.
pub fn compile(project: &Project, preset: &Preset) -> Result<Compiled, String> {
    let (Compiled { doc, sections, mut warnings }, mut placeholders) = join_with_placeholders(project, preset)?;
    let mut s = Session::new(doc);
    // Last first: filling one in moves the paragraphs after it.
    placeholders.sort_by_key(|p| std::cmp::Reverse(p.at));
    let has_contents_item = placeholders.iter().any(|p| p.role == "contents");
    for ph in &placeholders {
        if let Err(e) = generate(&mut s, ph) {
            warnings.push(e);
        }
    }
    let has_toc = {
        let mut found = false;
        each_para_ref(&s.doc.body, 0, &mut |p| {
            // A table of figures is a TOC field with \c (or \a): not a table of contents. A
            // field spanning paragraphs (as a reopened file has it) is a FieldStart.
            found |= p.objects.iter().filter_map(field_instr).any(is_table_of_contents);
        });
        found
    };
    if preset.toc && !has_toc && !has_contents_item {
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

/// Checks before submission on a compiled document: comments and tracked changes left in, and
/// placeholder markers (`TODO`, `[CITE]`…) in the text. With a `final` preset they are errors.
pub fn check(c: &Compiled, preset: &Preset) -> Value {
    let mut problems = Vec::new();
    let mut push = |kind: &str, message: String, error: bool| problems.push(json!({"kind": kind, "message": message, "error": error}));
    if !c.doc.comments.is_empty() {
        push("comments", format!("{} comment(s) left in", c.doc.comments.len()), preset.final_check);
    }
    if !c.doc.revisions.is_empty() {
        push("trackedChanges", format!("{} tracked change(s) not accepted or rejected", c.doc.revisions.len()), preset.final_check);
    }
    let text = c.doc.plain_text(wordcraft_doc::StoryRef::Body);
    for m in preset.markers.iter().filter(|m| !m.is_empty()) {
        let n = text.matches(m.as_str()).count();
        if n > 0 {
            push("marker", format!("{m:?} appears {n} time(s)"), preset.final_check);
        }
    }
    for w in &c.warnings {
        push("warning", w.clone(), false);
    }
    let ok = !problems.iter().any(|p| p["error"] == true);
    json!({"ok": ok, "problems": problems, "sections": c.sections, "words": c.doc.word_count()})
}

/// The compile answer as JSON.
pub fn summary(c: &Compiled, extra: Value) -> Value {
    let mut v = json!({"sections": c.sections, "warnings": c.warnings, "words": c.doc.word_count()});
    if let (Some(o), Value::Object(e)) = (v.as_object_mut(), extra) {
        o.extend(e);
    }
    v
}
