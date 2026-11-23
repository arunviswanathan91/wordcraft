//! The document's structure, found as it is typed and listed for navigation (`structure.*`):
//!
//! - **Captions.** Enter after a line typed as `Figure 2.1: Cell counts`, `Table 3 — Reagents` or
//!   `Fig. A.1 Map` makes it a caption: the Caption style, with the number a `SEQ` field (and the
//!   chapter number a `STYLEREF` field) so it renumbers with the figures around it, lists in the
//!   table of figures and can be referred to. The typed number wins: a number that differs from
//!   the one its place gives is pinned (`SEQ … \r n`), and a pin the numbering has caught up with
//!   is dropped again.
//! - **Mentions.** `Figure 2.1` typed in the text becomes a reference to that caption (a `REF`
//!   field to a bookmark on its number), so it follows the figure's number from then on.
//! - **Outline** (`structure.outline`): every heading, appendix, figure, table and equation with
//!   its number and place, for the navigator; and what in the document was typed as structure but
//!   is still plain text, which `structure.convert` turns into the real thing in one step.
//! - **Figures that stay put** (`structure.settleFigures`): floating pictures become In Line with
//!   Text in a paragraph of their own that is kept with the caption after it, so no edit elsewhere
//!   can move them.
//!
//! - **Lists of figures and tables** (`structure.list`): a table of figures for any caption label,
//!   with or without a heading above it, whose entries show either the whole caption or only its
//!   heading: the part of the caption set in bold (`Figure 2.1: **Cell counts.** Counted on day
//!   3…` lists as "Figure 2.1: Cell counts"). The choice is kept in the document as a hidden
//!   bookmark (`_wcTitles_<label>`) on the list, so it survives saving as a Word document.
//!
//! Headings typed with their numbers are in [`crate::cmd::numbering::typed`].

use serde_json::{Value, json};
use wordcraft_doc::para::{InlineObject, OBJ, Wrap};
use wordcraft_doc::{Block, Paragraph, Path, Pos, StoryRef};

use super::numbering::typed::{self, Line};
use super::references::FieldCode;
use crate::{CmdError, CmdResult, CommandSpec, Selection, Session};

/// Most items of each kind `structure.outline` lists.
pub const MAX_ITEMS: usize = 5000;
/// The longest caption line converted (characters).
const MAX_CAPTION: usize = 1000;
/// The most characters before the caret read for a typed mention.
const MENTION_SCAN: usize = 48;

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("structure.outline", "Document Structure", "View › Show", |s, _| Ok(outline(s)))
            .params(r#"{} → {"headings": [{"path": [block], "level": 1-9, "number": shown number | null, "text", "appendix": bool}], "figures" | "tables" | "equations" | "others": [{"path": [block], "label", "number", "text", "title": the caption's heading (its bold part) | null, "id": SEQ name}], "typed": [{"path": [block], "kind": "heading|appendix|caption", "text"}] (typed as structure but still plain text), "floating": n (pictures and shapes that can move)} (at most 5000 of each)"#)
            .pure(),
        CommandSpec::new("structure.convert", "Detect Headings and Captions", "References › Captions", convert).params(
            r#"{"kinds"?: ["heading", "appendix", "caption"] (default all)} → {"converted": n, "headings": n, "captions": n}: every body paragraph typed as a numbered heading ("2.1 Methods"), an appendix heading or a caption ("Figure 2.1: …") becomes a real one, as Enter after it would have made it. One undo step"#,
        ),
        CommandSpec::new("structure.settleFigures", "Keep Figures in Place", "Picture Format › Arrange", settle_figures).params(
            r#"{} → {"settled": n, "floating": n left}: floating pictures become In Line with Text, each in a centred paragraph of its own kept with the paragraph after it (its caption), so text edits can't move them. Shapes and text boxes are left as they are. One undo step"#,
        ),
        CommandSpec::new("structure.list", "List of Figures or Tables", "References › Captions", list).params(
            r#"{"of"?: "figures" (default) | "tables" | "equations" | a caption label of your own, "heading"?: bool (default true: a heading above the list) | string (the heading's text), "titles"?: "bold" (default: each entry shows the caption's heading, its part in bold; the whole caption when nothing in it is bold) | "full" (the whole caption), "includeLabel"?: bool (default true: "Figure 2.1" before the text), "pageNumbers"?: bool, "tabLeader"?: "dot|hyphen|underscore|none"} → {"entries": n}: inserted at the caret; References › Update Table refreshes it"#,
        ),
        CommandSpec::new("structure.update", "Update Numbers", "References › Captions", |s, _| {
            update_numbers(s)?;
            Ok(outline(s))
        })
        .params(r#"{} → structure.outline's answer: figure, table and equation numbers, and the references to them, recounted"#),
    ]
}

/// The labels a caption line starts with, and the sequence each numbers in. Longer first.
const LABELS: [(&str, &str); 13] = [
    ("Figure", "Figure"),
    ("Fig.", "Figure"),
    ("Fig", "Figure"),
    ("Table", "Table"),
    ("Tab.", "Table"),
    ("Equation", "Equation"),
    ("Eq.", "Equation"),
    ("Scheme", "Scheme"),
    ("Listing", "Listing"),
    ("Plate", "Plate"),
    ("Chart", "Chart"),
    ("Box", "Box"),
    ("Map", "Map"),
];

/// A typed caption line or mention: `Figure 2.1`, `Table 3`, `Fig. A.1`, `Figure 2-1`, `Figure S1`.
#[derive(Clone, Debug, PartialEq)]
pub struct CaptionLine {
    /// The sequence it numbers in (`Figure` for `Fig.` too).
    pub id: &'static str,
    /// The chapter part, when the number has one (`2` of 2.1, `A` of A.1).
    pub chapter: Option<String>,
    /// What stands between the chapter part and the number (`.`, `-` or nothing).
    pub sep: &'static str,
    /// The figure's own number.
    pub n: u32,
    /// Where the number starts and ends in the text (bytes).
    pub number: (usize, usize),
}

/// The label at the start of `text` and the number after it; `None` unless both are there.
fn label_number(text: &str) -> Option<CaptionLine> {
    let (word, id) =
        LABELS.iter().find(|(w, _)| text.get(..w.len()).is_some_and(|h| h.eq_ignore_ascii_case(w) && h.starts_with(char::is_uppercase)))?;
    let rest = text.get(word.len()..)?;
    let sp = rest.len() - rest.trim_start_matches([' ', '\u{a0}']).len();
    if sp == 0 {
        return None;
    }
    let start = word.len() + sp;
    let num = text.get(start..)?;
    let b = num.as_bytes();
    let digits = |i: usize| b.iter().skip(i).take_while(|c| c.is_ascii_digit()).count();
    let value = |from: usize, len: usize| num.get(from..from + len).and_then(|d| d.parse::<u32>().ok()).filter(|_| (1..=4).contains(&len));
    // A letter in front: an appendix's (A.1, A1) or a supplement's (S1).
    let letter = usize::from(b.first().is_some_and(u8::is_ascii_uppercase) && (digits(1) > 0 || (b.get(1) == Some(&b'.') && digits(2) > 0)));
    let (chapter, sep, n, len): (Option<String>, &'static str, u32, usize) = if letter == 1 {
        let dot = usize::from(b.get(1) == Some(&b'.'));
        let d = digits(1 + dot);
        (num.get(..1).map(str::to_string), if dot == 1 { "." } else { "" }, value(1 + dot, d)?, 1 + dot + d)
    } else {
        let d = digits(0);
        value(0, d)?;
        // Further parts: 2.1, 2.1.3, 2-1.
        let mut end = d;
        let mut last = (0usize, d);
        let mut sep = "";
        loop {
            let c = b.get(end).copied();
            let nd = digits(end + 1);
            let joins = match c {
                Some(b'.') => nd > 0,
                Some(b'-') => nd > 0 && sep.is_empty() && last.0 == 0,
                _ => false,
            };
            if !joins || nd > 4 || end > 24 {
                break;
            }
            sep = if c == Some(b'-') { "-" } else { "." };
            last = (end + 1, nd);
            end += 1 + nd;
        }
        let n = value(last.0, last.1)?;
        let chapter = (last.0 > 0).then(|| num.get(..last.0 - 1).unwrap_or("").to_string());
        (chapter, sep, n, end)
    };
    Some(CaptionLine { id, chapter, sep, n, number: (start, start + len) })
}

/// `text` as a typed caption: a label and number, then nothing, a `:` or `.`, a dash, or a title
/// that starts with a capital. "Figure 2.1 shows the counts" is a sentence, not a caption.
pub fn caption_line(text: &str) -> Option<CaptionLine> {
    if text.chars().count() > MAX_CAPTION || text.contains(['\t', '\n']) {
        return None;
    }
    let c = label_number(text)?;
    let rest = text.get(c.number.1..)?;
    let after = rest.trim_start_matches([' ', '\u{a0}']);
    let spaced = after.len() < rest.len();
    let ok = rest.is_empty()
        || rest.starts_with([':', '.'])
        || (spaced && after.starts_with(['-', '\u{2013}', '\u{2014}', '|']))
        || (spaced
            && after
                .chars()
                .next()
                .is_some_and(|ch| ch.is_uppercase() || (ch.is_alphabetic() && !ch.is_lowercase()) || matches!(ch, '(' | '"' | '\u{201C}')));
    ok.then_some(c)
}

/// Whether paragraph `p` holds a `SEQ` field, and the sequence's name.
fn seq_id(p: &Paragraph) -> Option<String> {
    p.objects.iter().find_map(|o| match o {
        InlineObject::Field { instr, .. } => {
            let f = FieldCode::parse(instr);
            (f.name == "SEQ").then(|| f.args.first().cloned()).flatten()
        }
        _ => None,
    })
}

fn is_caption_style(s: &Session, p: &Paragraph) -> bool {
    p.props.style.as_deref().is_some_and(|id| id == "Caption" || s.doc.styles.chain(id).iter().any(|st| st.id == "Caption"))
}

/// The caption Enter makes of the paragraph at `at`, if its text is one: a top-level body
/// paragraph of plain text in Normal or the Caption style, the caret at its end.
pub(crate) fn enter_caption(s: &Session, at: &Pos) -> Option<CaptionLine> {
    let ac = &s.prefs.autocorrect;
    if !ac.enabled || !ac.structure || at.story != StoryRef::Body || at.path.0.len() != 1 {
        return None;
    }
    let para = s.doc.para_at(at)?;
    if at.off != para.len() || !para.objects.is_empty() || para.section.is_some() || para.run_ranges().any(|(_, c)| c.del.is_some()) {
        return None;
    }
    let r = s.doc.styles.resolve_para(&para.props);
    let plain = para.props.style.as_deref().is_none_or(|st| st == "Normal") && r.outline_level.is_none();
    if !(plain || is_caption_style(s, para)) || r.numbering.is_some_and(|n| n.num != 0) {
        return None;
    }
    if crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting {
        return None;
    }
    caption_line(&para.text)
}

fn field(instr: String) -> InlineObject {
    InlineObject::Field { instr, result: String::new(), locked: false, dirty: false, code: false, form: String::new() }
}

/// The results of the paragraph's STYLEREF and SEQ fields (first of each), with their offsets.
fn caption_fields(p: &Paragraph) -> (Option<(usize, String)>, Option<(usize, String)>) {
    let (mut chapter, mut seq) = (None, None);
    for (off, o) in p.objects_with_offsets() {
        if let InlineObject::Field { instr, result, .. } = o {
            match FieldCode::parse(instr).name.as_str() {
                "STYLEREF" if chapter.is_none() => chapter = Some((off, result.clone())),
                "SEQ" if seq.is_none() => seq = Some((off, result.clone())),
                _ => {}
            }
        }
    }
    (chapter, seq)
}

/// Make the paragraph at `at` the caption its typed text says ([`enter_caption`]): the typed
/// number becomes fields that show it, the paragraph takes the Caption style, and it is kept on
/// the page of the picture above it or the table below it. Returns the paragraph's end.
pub(crate) fn apply_caption(s: &mut Session, at: &Pos, c: &CaptionLine) -> Result<Pos, CmdError> {
    let start = Pos { off: c.number.0, ..at.clone() };
    let props = s.doc.para(at.story, &at.path).map(|p| p.props_at(c.number.0).clone().without_symbol()).unwrap_or_default();
    let before = s.doc.para(at.story, &at.path).map(|p| p.props.clone());
    s.sel = Selection { anchor: start.clone(), focus: Pos { off: c.number.1, ..at.clone() } };
    let mut ins = crate::cmd::delete_selection(s)?;
    let mut seq = format!("SEQ {} \\* ARABIC", c.id);
    if c.chapter.is_some() {
        ins = s.doc.insert_object(&ins, field("STYLEREF 1 \\s".into()), &props)?;
        if !c.sep.is_empty() {
            ins = s.doc.insert_text(&ins, c.sep, &props)?;
        }
        seq.push_str(" \\s 1");
    }
    s.doc.insert_object(&ins, field(seq), &props)?;
    {
        let para = s.doc.para_mut(at.story, &at.path)?;
        para.props.style = Some("Caption".into());
        para.touch();
    }
    crate::cmd::references::update_seq(s)?;
    // The typed number wins over the one its place gives.
    let (chapter, number) = s.doc.para(at.story, &at.path).map(caption_fields).unwrap_or_default();
    if let (Some(want), Some((off, got))) = (&c.chapter, chapter)
        && *want != got
    {
        // No chapter heading with that number here: the chapter part stays as typed.
        let para = s.doc.para_mut(at.story, &at.path)?;
        para.delete(off, off + OBJ.len_utf8())?;
        para.insert_text(off, want, &props)?;
        para.touch();
    }
    if number.is_some_and(|(_, got)| got != c.n.to_string()) {
        let para = s.doc.para_mut(at.story, &at.path)?;
        for o in para.objects.iter_mut() {
            if let InlineObject::Field { instr, .. } = o
                && FieldCode::parse(instr).name == "SEQ"
            {
                instr.push_str(&format!(" \\r {}", c.n));
                break;
            }
        }
        para.touch();
    }
    keep_with_neighbour(s, at, c.id)?;
    if let Some(before) = before {
        crate::cmd::fmt_revisions::record_para_props(s, at.story, &at.path, &before)?;
    }
    update_numbers(s)?;
    let end = s.doc.para(at.story, &at.path).map_or(0, |p| p.len());
    Ok(Pos { off: end, ..at.clone() })
}

/// Keep a caption on the page of what it describes: a table caption with the table after it,
/// else the picture's paragraph before it with the caption.
fn keep_with_neighbour(s: &mut Session, at: &Pos, id: &str) -> Result<(), CmdError> {
    let k = at.path.last();
    let next_is_table = matches!(s.doc.block(at.story, &at.path.with_last(k.saturating_add(1))), Some(Block::Table(_)));
    if id == "Table" && next_is_table {
        let para = s.doc.para_mut(at.story, &at.path)?;
        para.props.keep_next = Some(true);
        para.touch();
        return Ok(());
    }
    if k == 0 {
        return Ok(());
    }
    let prev = at.path.with_last(k - 1);
    let holds_picture = s.doc.para(at.story, &prev).is_some_and(|p| {
        p.objects.iter().any(|o| {
            matches!(o, InlineObject::Image { .. } | InlineObject::Graphic { .. } | InlineObject::Shape { .. } | InlineObject::Group { .. })
                && !o.is_floating()
        })
    });
    if holds_picture {
        let para = s.doc.para_mut(at.story, &prev)?;
        para.props.keep_next = Some(true);
        para.touch();
    }
    Ok(())
}

/// Recount the document's figure, table and equation numbers and the references to them, and
/// drop the pins the numbering has caught up with (captions' `\r n` and headings' start values).
pub fn update_numbers(s: &mut Session) -> Result<(), CmdError> {
    typed::heal(s)?;
    crate::cmd::references::heal_seq_pins(s)?;
    crate::cmd::references::update_seq(s)?;
    crate::cmd::references::update_refs(s)
}

/// One caption of the body.
struct Caption {
    k: usize,
    id: String,
    /// The label and number as shown ("Figure 2.1").
    label: String,
    /// The number alone ("2.1").
    number: String,
    /// The caption's text after the number and its separator.
    text: String,
    /// Its heading: the bold part of the text, when it has one.
    title: Option<String>,
    /// Where the number starts and ends in the paragraph's own text (bytes, fields as objects).
    span: (usize, usize),
}

/// The captions among the body's top-level paragraphs: those holding a `SEQ` field.
fn captions(s: &Session) -> Vec<Caption> {
    let mut out = Vec::new();
    for (k, b) in s.doc.body.iter().enumerate() {
        if out.len() >= MAX_ITEMS {
            break;
        }
        let Block::Para(p) = &**b else { continue };
        let Some(id) = seq_id(p) else { continue };
        // The number: from the first field to the SEQ field, with what lies between.
        let fields: Vec<(usize, bool)> = p
            .objects_with_offsets()
            .filter_map(|(off, o)| match o {
                InlineObject::Field { instr, .. } => Some((off, FieldCode::parse(instr).name == "SEQ")),
                _ => None,
            })
            .collect();
        let Some(seq_at) = fields.iter().find(|(_, seq)| *seq).map(|(o, _)| *o) else { continue };
        let first = fields
            .iter()
            .map(|(o, _)| *o)
            .find(|o| *o <= seq_at && p.text.get(*o..seq_at).is_some_and(|t| t.chars().filter(|c| *c != OBJ).count() <= 3))
            .unwrap_or(seq_at);
        // A typed chapter part right before the SEQ field ("2." of a pinned chapter) belongs too.
        let lead = p.text.get(..first).unwrap_or("");
        let typed_chapter = lead.len() - lead.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).len();
        let from = if typed_chapter > 0
            && lead.trim_end_matches(|c: char| c.is_ascii_alphanumeric() || matches!(c, '.' | '-')).ends_with([' ', '\u{a0}', OBJ])
        {
            first - typed_chapter
        } else {
            first
        };
        let to = seq_at + OBJ.len_utf8();
        let part = |a: usize, b: usize| {
            let mut t = String::new();
            let mut objs = p.objects_with_offsets().peekable();
            for (i, ch) in p.text.char_indices() {
                if i < a || i >= b {
                    if ch == OBJ {
                        objs.next();
                    }
                    continue;
                }
                if ch == OBJ {
                    if let Some((_, o)) = objs.next() {
                        t.push_str(o.plain_text());
                    }
                } else {
                    t.push(ch);
                }
            }
            t
        };
        let number = part(from, to);
        let label = format!("{}{number}", part(0, from));
        let text =
            part(to, p.text.len()).trim_start_matches([':', '.', ' ', '\u{a0}', '-', '\u{2013}', '\u{2014}', '|']).trim().chars().take(200).collect();
        let title = caption_entry(&s.doc, p, true);
        out.push(Caption { k, id, label: label.trim().to_string(), number, text, title, span: (from, to) });
    }
    out
}

/// The body's headings with the numbers they show: (block, level, number, text).
fn numbered_headings(s: &Session) -> Vec<(usize, u8, Option<String>, String)> {
    use wordcraft_doc::numbering::Counters;
    use wordcraft_doc::section::NumFormat;
    let doc = &s.doc;
    let mut counters = Counters::default();
    let mut out = Vec::new();
    for (k, b) in doc.body.iter().enumerate() {
        let Block::Para(p) = &**b else { continue };
        let rp = doc.styles.resolve_para(&p.props);
        let number = p
            .props
            .numbering
            .or(rp.numbering)
            .filter(|n| n.num != 0)
            .and_then(|n| counters.next_label(&doc.numbering, n.num, n.level))
            .filter(|(label, def)| !label.is_empty() && !matches!(def.format, NumFormat::Bullet | NumFormat::None))
            .map(|(label, _)| label);
        let Some(lv) = rp.outline_level.filter(|l| *l < 9) else { continue };
        let text = p.plain_text().trim().to_string();
        if text.is_empty() || rp.style.starts_with("TOC") || out.len() >= MAX_ITEMS {
            continue;
        }
        out.push((k, lv + 1, number, text.chars().take(200).collect()));
    }
    out
}

/// The prefix of the hidden bookmark that makes a list of `<label>` captions show their headings
/// only ([`caption_title`]).
pub const TITLES_BOOKMARK: &str = "_wcTitles_";

/// The heading of caption `p`: the first stretch of bold text after its number, as a byte range
/// of its text. `None` when nothing after the number is bold, or all of it is (a caption whose
/// style is bold has no heading of its own).
pub fn caption_title(doc: &wordcraft_doc::Document, p: &Paragraph) -> Option<(usize, usize)> {
    let seq_end = p.objects_with_offsets().find_map(|(off, o)| match o {
        InlineObject::Field { instr, .. } if FieldCode::parse(instr).name == "SEQ" => Some(off + OBJ.len_utf8()),
        _ => None,
    })?;
    let mut range: Option<(usize, usize)> = None;
    let mut plain = false;
    for (r, props) in p.run_ranges() {
        let (a, b) = (r.start.max(seq_end), r.end);
        if a >= b {
            continue;
        }
        let text = p.text.get(a..b).unwrap_or("");
        // Separators, spaces and bookmark marks neither start nor end the heading.
        let says = text.chars().any(|c| c.is_alphanumeric());
        let bold = doc.styles.resolve_char(p.props.style.as_deref(), props).bold;
        match (&mut range, bold) {
            (None, true) if says => range = Some((a, b)),
            (None, _) => plain |= says,
            (Some(r), true) if r.1 == a => r.1 = b,
            (Some(r), false) if !says && r.1 == a => r.1 = b,
            (Some(_), _) => {
                plain |= says;
                break;
            }
        }
    }
    let (a, b) = range?;
    // The rest of the caption after the heading.
    plain |= p.text.get(b..).is_some_and(|t| t.chars().any(char::is_alphanumeric));
    plain.then_some((a, b))
}

/// The text of `p` from byte `a` to `b`, fields as their results.
pub fn para_part(p: &Paragraph, a: usize, b: usize) -> String {
    let mut t = String::new();
    let mut k = 0usize;
    for (i, ch) in p.text.char_indices() {
        if ch == OBJ {
            if i >= a
                && i < b
                && let Some(o) = p.objects.get(k)
            {
                t.push_str(o.plain_text());
            }
            k += 1;
        } else if i >= a && i < b {
            t.push(ch);
        }
    }
    t
}

/// What a list of captions shows for `p` when it lists headings only: the caption up to the end
/// of its heading (`bare`: the heading alone), without the full stop or colon that ends it.
/// `None` when the caption has no heading of its own.
pub fn caption_entry(doc: &wordcraft_doc::Document, p: &Paragraph, bare: bool) -> Option<String> {
    let (a, b) = caption_title(doc, p)?;
    let t = para_part(p, if bare { a } else { 0 }, b);
    let t = t.trim().trim_end_matches(['.', ':', ';', ',', ' ', '\u{2013}', '\u{2014}']).trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// `structure.list`.
fn list(s: &mut Session, v: &Value) -> CmdResult {
    let of = crate::p::str(v, "of").unwrap_or("figures");
    let (label, plural) = match of.to_ascii_lowercase().as_str() {
        "figures" | "figure" => ("Figure".to_string(), "Figures".to_string()),
        "tables" | "table" => ("Table".to_string(), "Tables".to_string()),
        "equations" | "equation" => ("Equation".to_string(), "Equations".to_string()),
        _ => {
            let l: String = of.chars().filter(|c| !matches!(c, '"' | '\\') && !c.is_control()).take(40).collect();
            let l = l.trim().to_string();
            if l.is_empty() {
                return Err(CmdError::Params("`of` is figures, tables, equations or a caption label".into()));
            }
            (l.clone(), format!("{l}s"))
        }
    };
    let title = match v.get("heading") {
        Some(Value::Bool(false)) => String::new(),
        Some(Value::String(t)) => t.chars().filter(|c| !c.is_control()).take(120).collect(),
        _ => format!("List of {plural}"),
    };
    let bold = match crate::p::str(v, "titles").unwrap_or("bold") {
        "bold" => true,
        "full" => false,
        x => return Err(CmdError::Params(format!("`titles` is bold or full, not {:?}", x.chars().take(20).collect::<String>()))),
    };
    let mut opts = json!({"label": label});
    for k in ["includeLabel", "pageNumbers", "tabLeader"] {
        if let Some(x) = v.get(k) {
            opts[k] = x.clone();
        }
    }
    let instr = crate::cmd::citations::figures_instr(&opts);
    let ident = label.split_whitespace().collect::<Vec<_>>().join("_");
    let mark = format!("{TITLES_BOOKMARK}{ident}");
    // The mark says how every list of this label reads: set or cleared with the newest list.
    let old: Vec<(usize, usize)> = s
        .doc
        .body
        .iter()
        .enumerate()
        .filter_map(|(k, b)| Some((k, b.as_para()?)))
        .flat_map(|(k, p)| {
            p.objects_with_offsets()
                .filter(|(_, o)| matches!(o, InlineObject::BookmarkStart { name } | InlineObject::BookmarkEnd { name } if name.eq_ignore_ascii_case(&mark)))
                .map(|(off, _)| (k, off))
                .collect::<Vec<_>>()
        })
        .collect();
    for (k, off) in old.into_iter().rev() {
        let para = s.doc.para_mut(StoryRef::Body, &Path::top(k))?;
        para.delete(off, off + OBJ.len_utf8())?;
        para.touch();
    }
    s.clamp_selection();
    let from = s.sel.focus.path.0.first().copied().unwrap_or(0) as usize;
    s.run("references.tableOfFigures", &{
        let mut o = opts.clone();
        o["title"] = json!(title);
        o
    })?;
    if bold {
        let head = s.doc.body.iter().enumerate().skip(from.saturating_sub(1)).find_map(|(k, b)| {
            b.as_para().filter(|p| p.objects.iter().any(|o| matches!(o, InlineObject::Field { instr: i, .. } if *i == instr))).map(|_| k)
        });
        if let Some(k) = head {
            let at = Pos { story: StoryRef::Body, path: Path::top(k), off: 0 };
            let props = wordcraft_doc::props::CharProps::default();
            s.doc.insert_object(&at, InlineObject::BookmarkEnd { name: mark.clone() }, &props)?;
            s.doc.insert_object(&at, InlineObject::BookmarkStart { name: mark }, &props)?;
        }
    }
    crate::cmd::citations::update_generated(s)?;
    s.clamp_selection();
    let entries = captions(s).iter().filter(|c| c.id.eq_ignore_ascii_case(&ident)).count();
    Ok(json!({"entries": entries}))
}

/// `structure.outline`'s answer.
pub fn outline(s: &Session) -> Value {
    let headings: Vec<Value> = numbered_headings(s)
        .into_iter()
        .map(|(k, level, number, text)| {
            let appendix = number.is_none() && (typed::appendix_line(&text).is_some() || typed::appendix_sub_line(&text).is_some());
            json!({"path": [k], "level": level, "number": number, "text": text, "appendix": appendix})
        })
        .collect();
    let (mut figures, mut tables, mut equations, mut others) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let caps = captions(s);
    let captioned: std::collections::HashSet<usize> = caps.iter().map(|c| c.k).collect();
    for c in caps {
        let v = json!({"path": [c.k], "label": c.label, "number": c.number, "text": c.text, "title": c.title, "id": c.id});
        match c.id.as_str() {
            "Figure" => figures.push(v),
            "Table" => tables.push(v),
            "Equation" => equations.push(v),
            _ => others.push(v),
        }
    }
    // Typed as structure, still plain text.
    let mut typed_items = Vec::new();
    let mut floating = 0usize;
    for (k, b) in s.doc.body.iter().enumerate() {
        let Block::Para(p) = &**b else { continue };
        floating += p.objects.iter().filter(|o| o.is_floating() && matches!(o, InlineObject::Image { .. } | InlineObject::Graphic { .. })).count();
        if typed_items.len() >= MAX_ITEMS || captioned.contains(&k) {
            continue;
        }
        let at = Pos { story: StoryRef::Body, path: Path::top(k), off: p.len() };
        let kind = match typed::enter_line(s, &at) {
            Some(Line::Heading(_)) => Some("heading"),
            Some(Line::Appendix(_)) if typed::plain_line(s, &at).is_some_and(|p| s.doc.styles.resolve_para(&p.props).outline_level.is_none()) => {
                Some("appendix")
            }
            Some(Line::Appendix(_)) => None,
            None => enter_caption(s, &at).map(|_| "caption"),
        };
        if let Some(kind) = kind {
            typed_items.push(json!({"path": [k], "kind": kind, "text": p.text.chars().take(120).collect::<String>()}));
        }
    }
    json!({"headings": headings, "figures": figures, "tables": tables, "equations": equations, "others": others, "typed": typed_items, "floating": floating})
}

/// `structure.convert`.
fn convert(s: &mut Session, v: &Value) -> CmdResult {
    let kinds: Option<Vec<String>> = match v.get("kinds") {
        Some(Value::Array(a)) => Some(a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        Some(Value::Null) | None => None,
        Some(_) => return Err(CmdError::Params("`kinds` is a list".into())),
    };
    let wants = |k: &str| kinds.as_ref().is_none_or(|ks| ks.iter().any(|x| x == k));
    let sel = s.sel.clone();
    let (mut headings, mut captions) = (0usize, 0usize);
    let count = s.doc.body.len();
    for k in 0..count {
        if headings + captions >= MAX_ITEMS {
            break;
        }
        let Some(len) = s.doc.body.get(k).and_then(|b| b.as_para()).map(Paragraph::len) else { continue };
        let at = Pos { story: StoryRef::Body, path: Path::top(k), off: len };
        match typed::enter_line(s, &at) {
            Some(line @ Line::Heading(_)) if wants("heading") => {
                typed::apply_line(s, &at, &line)?;
                headings += 1;
            }
            Some(line @ Line::Appendix(_)) if wants("appendix") => {
                // One already in a heading style is an appendix heading as it is.
                if s.doc.para_at(&at).is_some_and(|p| s.doc.styles.resolve_para(&p.props).outline_level.is_none()) {
                    typed::apply_line(s, &at, &line)?;
                    headings += 1;
                }
            }
            Some(_) => {}
            None => {
                if wants("caption")
                    && let Some(c) = enter_caption(s, &at)
                {
                    apply_caption(s, &at, &c)?;
                    captions += 1;
                }
            }
        }
    }
    s.sel = sel;
    s.clamp_selection();
    update_numbers(s)?;
    Ok(json!({"converted": headings + captions, "headings": headings, "captions": captions}))
}

/// `structure.settleFigures`.
fn settle_figures(s: &mut Session, _: &Value) -> CmdResult {
    let mut settled = 0usize;
    let mut left = 0usize;
    let count = s.doc.body.len();
    for k in 0..count {
        let path = Path::top(k);
        let Some(p) = s.doc.para(StoryRef::Body, &path) else { continue };
        let floats = p.objects.iter().filter(|o| o.is_floating()).count();
        if floats == 0 {
            continue;
        }
        let pictures = p.objects.iter().filter(|o| o.is_floating() && matches!(o, InlineObject::Image { .. } | InlineObject::Graphic { .. })).count();
        left += floats - pictures;
        if pictures == 0 {
            continue;
        }
        // A paragraph holding only its pictures becomes the figure's own, centred and kept with
        // the caption after it; pictures anchored in a paragraph of text sit in its line.
        let alone = p.text.chars().all(|c| c == OBJ || c.is_whitespace());
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        for o in para.objects.iter_mut() {
            if o.is_floating()
                && matches!(o, InlineObject::Image { .. } | InlineObject::Graphic { .. })
                && let Some(f) = o.float_mut()
            {
                f.wrap = Wrap::Inline;
                settled += 1;
            }
        }
        if alone {
            para.props.keep_next = Some(true);
            para.props.align = Some(wordcraft_doc::props::Align::Center);
        }
        para.touch();
    }
    s.clamp_selection();
    Ok(json!({"settled": settled, "floating": left}))
}

/// The label and number a mention ends with, just before byte `end` of `text`: the mention's
/// start, the number's start, and what it names.
fn mention_before(text: &str, end: usize) -> Option<(usize, usize, CaptionLine)> {
    let head = text.get(..end)?;
    let from = head.char_indices().rev().take(MENTION_SCAN).last().map_or(0, |(i, _)| i);
    let window = head.get(from..)?;
    // The last label word in the window that starts a word.
    let mut best: Option<(usize, CaptionLine)> = None;
    for (i, _) in window.char_indices() {
        let starts_word = i == 0 || window.get(..i).and_then(|w| w.chars().next_back()).is_some_and(|c| !c.is_alphanumeric());
        if !starts_word {
            continue;
        }
        if let Some(c) = window.get(i..).and_then(label_number)
            && i + c.number.1 == window.len()
        {
            best = Some((from + i, c));
        }
    }
    let (at, c) = best?;
    let number = at + c.number.0;
    Some((at, number, c))
}

/// "Figure 2.1" typed in the text, on the space or punctuation after it: when the document has
/// that figure, the number becomes a reference to it. `trigger` is the length of the character
/// just typed. `false` when nothing was changed.
pub(crate) fn mention_autoformat(s: &mut Session, trigger: usize) -> Result<bool, CmdError> {
    let ac = &s.prefs.autocorrect;
    let f = s.sel.focus.clone();
    if !ac.enabled || !ac.structure || !s.sel.is_collapsed() || f.story != StoryRef::Body {
        return Ok(false);
    }
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    let Some(end) = f.off.checked_sub(trigger) else { return Ok(false) };
    // In a caption, the label and number are the caption's own.
    if seq_id(para).is_some() {
        return Ok(false);
    }
    let Some((label_at, number_at, m)) = mention_before(&para.text, end) else { return Ok(false) };
    // A paragraph that starts with the label may be a caption being typed: Enter decides.
    if para.text.get(..label_at).is_none_or(|lead| lead.trim().is_empty()) {
        return Ok(false);
    }
    let typed_number = para.text.get(number_at..end).unwrap_or("").to_string();
    // A trailing full stop ends the sentence, not the number.
    let Some(target) = captions(s).into_iter().find(|c| c.id == m.id && c.number == typed_number) else { return Ok(false) };
    if para.run_ranges().any(|(_, c)| c.del.is_some()) {
        return Ok(false);
    }
    let props = para.props_at(number_at).clone().without_symbol();
    let target_pos = Pos { story: StoryRef::Body, path: Path::top(target.k), off: 0 };
    let name = number_bookmark(s, &target_pos, target.span)?;
    let from = Pos { off: number_at, ..f.clone() };
    s.sel = Selection { anchor: from.clone(), focus: Pos { off: end, ..f.clone() } };
    let at = crate::cmd::delete_selection(s)?;
    let after = s.doc.insert_object(
        &at,
        InlineObject::Field { instr: format!("REF {name} \\h"), result: typed_number, locked: false, dirty: false, code: false, form: String::new() },
        &props,
    )?;
    s.sel = Selection::caret(Pos { off: after.off + trigger, ..after });
    Ok(true)
}

/// The hidden `_Ref` bookmark around the number of the caption at `pos` (`span` in its text),
/// reusing one that is already exactly there. Returns its name.
fn number_bookmark(s: &mut Session, pos: &Pos, span: (usize, usize)) -> Result<String, CmdError> {
    const MARK: usize = OBJ.len_utf8();
    let para = s.doc.para(pos.story, &pos.path).ok_or_else(|| CmdError::Failed("the caption is gone".into()))?;
    // A bookmark start right before the number whose end is right after it.
    if let Some(off) = span.0.checked_sub(MARK)
        && let Some(InlineObject::BookmarkStart { name }) = para.object_at(off)
        && name.starts_with("_Ref")
        && matches!(para.object_at(span.1), Some(InlineObject::BookmarkEnd { name: n }) if n == name)
    {
        return Ok(name.clone());
    }
    let next =
        s.doc.bookmarks().iter().filter_map(|(n, _)| n.strip_prefix("_Ref").and_then(|d| d.parse::<u64>().ok())).max().unwrap_or(0).saturating_add(1);
    let name = format!("_Ref{next:09}");
    let props = wordcraft_doc::props::CharProps::default();
    s.doc.insert_object(&Pos { off: span.1, ..pos.clone() }, InlineObject::BookmarkEnd { name: name.clone() }, &props)?;
    s.doc.insert_object(&Pos { off: span.0, ..pos.clone() }, InlineObject::BookmarkStart { name: name.clone() }, &props)?;
    Ok(name)
}

#[cfg(test)]
mod tests;
