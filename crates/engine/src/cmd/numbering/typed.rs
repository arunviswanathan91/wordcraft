//! Structure as you type: a line typed as `2.1 Methods`, `Chapter 3: Results` or `Appendix A`
//! becomes a real heading when Enter finishes it, and a number typed in front of a heading that
//! is already numbered renumbers it. The heading then shows in the outline and the table of
//! contents, and the headings after it count on from it.
//!
//! The typed number wins: when the number a heading would get from its place differs from the
//! one typed, the heading is pinned to the typed number (a start override on a list of its own
//! that shares the heading list's counters). A pin that the numbering has caught up with is
//! removed again ([`heal`]), so numbers follow later edits wherever they can.
//!
//! What counts as a heading line is decided from the text alone ([`heading_line`],
//! [`appendix_line`]): a whole short paragraph, a number of at most a few small parts, a title
//! that starts with a capital (or a letter with no case) and doesn't end like a sentence. A wrong
//! heading is worse than a missed one, so "3.5 mg of buffer" and "2019 was a dry year" stay text.

use std::sync::Arc;

use wordcraft_doc::numbering::Counters;
use wordcraft_doc::props::NumRef;
use wordcraft_doc::{Block, Path, Pos, StoryRef};

use super::{Scheme, headings, scheme, scheme_level};
use crate::{CmdError, Selection, Session};

/// The longest title (characters after the number) taken for a heading.
const MAX_TITLE: usize = 100;
/// The most words a heading's title has.
const MAX_WORDS: usize = 14;
/// The largest value of one part of a heading number ("2019 Results" is text).
const MAX_PART: u32 = 99;
/// The deepest heading number converted (1.2.3.4.5.6).
const MAX_PARTS: usize = 6;

/// A typed heading line.
#[derive(Clone, Debug, PartialEq)]
pub struct HeadingLine {
    /// The number's parts (`2.1` → [2, 1]).
    pub parts: Vec<u32>,
    /// The bytes of the typed number, with the punctuation and spaces after it.
    pub typed: usize,
    /// The line began with the word Chapter.
    pub chapter: bool,
}

/// A typed appendix heading (`Appendix A`, `Appendix B: Survey`, `Appendix 2.2 Data`, and `A.1
/// Title` under one).
#[derive(Clone, Debug, PartialEq)]
pub struct AppendixLine {
    /// 1 for `Appendix A`; the number of parts for a dotted number.
    pub level: u8,
}

/// Whether `title` reads like a heading's title rather than a sentence.
fn title_like(title: &str) -> bool {
    let t = title.trim();
    let Some(first) = t.chars().next() else { return false };
    // A capital, or a letter of a script without case; a lowercase start is a sentence
    // ("3.5 mg of buffer").
    let starts = first.is_uppercase() || (first.is_alphabetic() && !first.is_lowercase()) || first == '"' || first == '\u{201C}' || first == '(';
    starts
        && t.chars().count() <= MAX_TITLE
        && t.split_whitespace().count() <= MAX_WORDS
        && !t.ends_with(['.', '!', '?', ';', ',', ':'])
        && !t.contains(['\t', '\n', wordcraft_doc::para::OBJ])
}

/// The decimal number at the start of `s`: its parts and its length in bytes. `3`, `3.2`,
/// `3.2.1`; each part one to three digits.
fn decimal(s: &str) -> Option<(Vec<u32>, usize)> {
    let b = s.as_bytes();
    let mut parts = Vec::new();
    let mut i = 0usize;
    loop {
        let d = b.iter().skip(i).take_while(|c| c.is_ascii_digit()).count();
        if d == 0 || d > 3 {
            return None;
        }
        let v: u32 = s.get(i..i + d)?.parse().ok()?;
        if v > MAX_PART || parts.len() >= MAX_PARTS {
            return None;
        }
        parts.push(v);
        i += d;
        // A dot followed by a digit carries on the number; any other dot ends it.
        if b.get(i) == Some(&b'.') && b.get(i + 1).is_some_and(u8::is_ascii_digit) {
            i += 1;
        } else {
            return Some((parts, i));
        }
    }
}

/// The spaces at the start of `s`, in bytes.
fn spaces(s: &str) -> usize {
    s.len() - s.trim_start_matches([' ', '\t', '\u{a0}']).len()
}

/// `word` at the start of `s` (any case, its first letter a capital) followed by a space: the
/// rest after the spaces.
fn after_word<'a>(s: &'a str, word: &str) -> Option<&'a str> {
    let head = s.get(..word.len())?;
    if !head.eq_ignore_ascii_case(word) || !head.starts_with(|c: char| c.is_uppercase()) {
        return None;
    }
    let rest = s.get(word.len()..)?;
    (spaces(rest) > 0).then(|| rest.trim_start_matches([' ', '\t', '\u{a0}']))
}

/// The separator a number may end with before its title (`.`, `:`, `)`, or a dash between
/// spaces), then the spaces: its length in bytes, and whether there was any separator or space.
fn separator(s: &str) -> (usize, bool) {
    let mut i = 0;
    if s.starts_with(['.', ':', ')']) {
        i = 1;
    }
    let sp = spaces(s.get(i..).unwrap_or(""));
    let mut n = i + sp;
    let rest = s.get(n..).unwrap_or("");
    // " — Title", " - Title"
    if sp > 0
        && let Some(dash) = rest.chars().next().filter(|c| matches!(c, '-' | '\u{2013}' | '\u{2014}'))
    {
        let after = rest.get(dash.len_utf8()..).unwrap_or("");
        let sp2 = spaces(after);
        if sp2 > 0 {
            n += dash.len_utf8() + sp2;
        }
    }
    (n, i > 0 || sp > 0)
}

/// `text` as a typed heading: `2 Methods`, `2.1 Sampling`, `2.1. Sampling`, `Chapter 3`,
/// `Chapter 3: Results`. `None` for anything that reads like body text.
pub fn heading_line(text: &str) -> Option<HeadingLine> {
    let (rest, chapter) = match after_word(text, "Chapter") {
        Some(r) => (r, true),
        None => (text, false),
    };
    let lead = text.len() - rest.len();
    let (parts, n) = decimal(rest)?;
    let after = rest.get(n..)?;
    let (sep, any) = separator(after);
    let title = after.get(sep..)?;
    if chapter {
        // "Chapter 3" alone, or with a title.
        if parts.len() != 1 || !(title.trim().is_empty() || (any && title_like(title))) {
            return None;
        }
    } else if !any || !title_like(title) {
        return None;
    }
    Some(HeadingLine { parts, typed: lead + n + sep, chapter })
}

/// `text` as a typed appendix heading: `Appendix A`, `Appendix B: Survey`, `Appendix 2.2 Data`.
pub fn appendix_line(text: &str) -> Option<AppendixLine> {
    let rest = after_word(text, "Appendix")?;
    let (n, level) = if let Some((parts, n)) = decimal(rest) {
        (n, parts.len())
    } else {
        // A letter (A), a letter and numbers (A.1), or a roman numeral (IV).
        let b = rest.as_bytes();
        let roman = b.iter().take_while(|c| matches!(c, b'I' | b'V' | b'X')).count();
        if b.first().is_some_and(u8::is_ascii_uppercase) && !b.get(1).is_some_and(u8::is_ascii_alphabetic) {
            match rest
                .get(1..)
                .filter(|r| r.starts_with('.') && r.as_bytes().get(1).is_some_and(u8::is_ascii_digit))
                .and_then(|r| decimal(r.get(1..)?))
            {
                Some((parts, n)) => (2 + n, 1 + parts.len()),
                None => (1, 1),
            }
        } else if (1..=6).contains(&roman) && !b.get(roman).is_some_and(u8::is_ascii_alphanumeric) {
            (roman, 1)
        } else {
            return None;
        }
    };
    let after = rest.get(n..)?;
    let (sep, any) = separator(after);
    let title = after.get(sep..)?;
    (title.trim().is_empty() || (any && title_like(title))).then_some(AppendixLine { level: u8::try_from(level.clamp(1, 3)).unwrap_or(1) })
}

/// `text` as a heading inside an appendix: `A.1 Survey questions`.
pub fn appendix_sub_line(text: &str) -> Option<AppendixLine> {
    let b = text.as_bytes();
    if !b.first().is_some_and(u8::is_ascii_uppercase) || b.get(1) != Some(&b'.') {
        return None;
    }
    let (parts, n) = decimal(text.get(2..)?)?;
    let after = text.get(2 + n..)?;
    let (sep, any) = separator(after);
    let title = after.get(sep..)?;
    (any && title_like(title)).then_some(AppendixLine { level: u8::try_from((1 + parts.len()).clamp(2, 4)).unwrap_or(2) })
}

/// What Enter makes of the paragraph it finishes.
#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    Heading(HeadingLine),
    Appendix(AppendixLine),
}

/// Whether the paragraph at `at` may be restyled by typing: a top-level body paragraph of plain
/// text (no pictures or fields, no tracked deletion), outside a list, in Normal or a heading
/// style, with the caret at its end, and no formatting protection on the heading styles.
pub(crate) fn plain_line<'a>(s: &'a Session, at: &Pos) -> Option<&'a wordcraft_doc::Paragraph> {
    let ac = &s.prefs.autocorrect;
    if !ac.enabled || !ac.structure || at.story != StoryRef::Body || at.path.0.len() != 1 {
        return None;
    }
    let para = s.doc.para_at(at)?;
    if at.off != para.len() || !para.objects.is_empty() || para.section.is_some() || para.run_ranges().any(|(_, c)| c.del.is_some()) {
        return None;
    }
    let style = para.props.style.as_deref();
    let r = s.doc.styles.resolve_para(&para.props);
    let plain = style.is_none_or(|st| st == "Normal") && r.outline_level.is_none();
    let heading = r.outline_level.is_some_and(|l| l < 9);
    if !(plain || heading) || r.numbering.is_some_and(|n| n.num != 0) {
        return None;
    }
    if crate::cmd::protect::enforced(&s.doc) && s.doc.settings.protect_formatting {
        return None;
    }
    Some(para)
}

/// Whether the body's last level-1 heading before block `k` is an appendix.
fn in_appendix(s: &Session, k: usize) -> bool {
    headings(s).iter().take_while(|h| h.k < k).filter(|h| h.level == 1).last().is_some_and(|h| appendix_line(h.text.trim()).is_some())
}

/// The heading Enter makes of the paragraph at `at`, if its text is one.
pub(crate) fn enter_line(s: &Session, at: &Pos) -> Option<Line> {
    let para = plain_line(s, at)?;
    let text = para.text.as_str();
    if let Some(a) = appendix_line(text) {
        return Some(Line::Appendix(a));
    }
    let k = at.path.0.first().copied()? as usize;
    if in_appendix(s, k)
        && let Some(a) = appendix_sub_line(text)
    {
        return Some(Line::Appendix(a));
    }
    heading_line(text).map(Line::Heading)
}

/// The heading list, made when the document has none: 1 / 1.1 / 1.1.1 linked to the heading
/// styles, its first level reading "Chapter 1" when `chapter`. The caret's paragraph must already
/// be in a heading style.
fn ensure_scheme(s: &mut Session, k: usize, chapter: bool) -> Result<Scheme, CmdError> {
    if let Some(sc) = scheme(s, &headings(s)) {
        return Ok(sc);
    }
    let sel = s.sel.clone();
    s.sel = Selection::caret(Pos { story: StoryRef::Body, path: Path::top(k), off: 0 });
    crate::cmd::lists::preset(s, &serde_json::json!({"preset": "headings"}))?;
    s.sel = sel;
    let sc = scheme(s, &headings(s)).ok_or_else(|| CmdError::Failed("no heading list to number with".into()))?;
    if chapter
        && let Some(abs) = Arc::make_mut(&mut s.doc.numbering).abstracts.iter_mut().find(|a| a.id == sc.abs)
        && let Some(l) = abs.levels.first_mut()
    {
        l.text = "Chapter %1".into();
    }
    Ok(sc)
}

/// The counter values each top-level body paragraph on the heading list's counters shows, in
/// order: (block, the list it uses, its level, the values of levels 0..=level).
fn shown(s: &Session, sc: Scheme) -> Vec<(usize, NumRef, Vec<u32>)> {
    let numbering = &s.doc.numbering;
    let mut counters = Counters::default();
    let mut out = Vec::new();
    for (k, b) in s.doc.body.iter().enumerate() {
        let Block::Para(p) = &**b else { continue };
        let Some(n) = p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0) else { continue };
        if counters.next_label(numbering, n.num, n.level).is_none() || numbering.abstract_of(n.num).map(|a| a.id) != Some(sc.abs) {
            continue;
        }
        let lv = (n.level as usize).min(8);
        let vals = counters.values(numbering, n.num).unwrap_or([0; 9]);
        out.push((k, n, vals.iter().take(lv + 1).copied().collect()));
    }
    out
}

/// Pin heading `k` (at list level `lv`) to `parts`: a list of its own on the heading list's
/// counters that starts each level at the typed value.
fn pin(s: &mut Session, sc: Scheme, k: usize, lv: u8, parts: &[u32]) -> Result<(), CmdError> {
    let numbering = Arc::make_mut(&mut s.doc.numbering);
    let Some(num) = numbering.restart(sc.num) else { return Ok(()) };
    if let Some(n) = numbering.nums.iter_mut().find(|n| n.id == num) {
        n.start_overrides = parts.iter().enumerate().filter_map(|(i, v)| u8::try_from(i).ok().map(|i| (i, *v))).collect();
    }
    let para = s.doc.para_mut(StoryRef::Body, &Path::top(k))?;
    para.props.numbering = Some(NumRef { num, level: lv });
    para.touch();
    Ok(())
}

/// Take away the pins the numbering has caught up with: a heading on a list of its own whose
/// number is what the heading list would give it anyway goes back to the heading list. Returns
/// how many were removed.
pub(crate) fn heal(s: &mut Session) -> Result<usize, CmdError> {
    unpin(s, false)
}

/// Take every pin away: each heading shows the number its place gives it.
pub(crate) fn unpin_all(s: &mut Session) -> Result<usize, CmdError> {
    unpin(s, true)
}

/// [`heal`], or with `all` every pin whatever number it holds.
fn unpin(s: &mut Session, all: bool) -> Result<usize, CmdError> {
    let hs = headings(s);
    let Some(sc) = scheme(s, &hs) else { return Ok(0) };
    let numbering = s.doc.numbering.clone();
    let mut counters = Counters::default();
    let mut redundant = Vec::new();
    for (k, b) in s.doc.body.iter().enumerate() {
        let Block::Para(p) = &**b else { continue };
        let by_style = s.doc.styles.resolve_para(&wordcraft_doc::props::ParaProps { numbering: None, ..p.props.clone() }).numbering;
        let Some(n) = p.props.numbering.or(by_style).filter(|n| n.num != 0) else { continue };
        let own = p.props.numbering.filter(|o| o.num != 0 && o.num != sc.num);
        let pinned = own.is_some_and(|o| {
            numbering.abstract_of(o.num).map(|a| a.id) == Some(sc.abs) && numbering.num(o.num).is_some_and(|x| !x.start_overrides.is_empty())
        });
        if pinned {
            let mut plain = counters.clone();
            let natural = plain.next_label(&numbering, sc.num, n.level).map(|(l, _)| l);
            let mut with = counters.clone();
            let pinned_label = with.next_label(&numbering, n.num, n.level).map(|(l, _)| l);
            if natural.is_some() && (all || natural == pinned_label) {
                // The style numbers it when the style is on the heading list; else directly.
                let direct = by_style.is_none_or(|b| b.num != sc.num).then_some(NumRef { num: sc.num, level: n.level });
                redundant.push((k, direct));
                counters = plain;
                continue;
            }
        }
        let _ = counters.next_label(&numbering, n.num, n.level);
    }
    let n = redundant.len();
    for (k, direct) in redundant {
        let para = s.doc.para_mut(StoryRef::Body, &Path::top(k))?;
        para.props.numbering = direct;
        para.touch();
    }
    Ok(n)
}

/// Number heading `k` at `level` with the heading list so it shows `parts`: through its style
/// when that gives the typed number, else pinned to it. Later pins that became redundant go.
fn number_as(s: &mut Session, k: usize, level: u8, parts: &[u32], chapter: bool) -> Result<(), CmdError> {
    let sc = ensure_scheme(s, k, chapter)?;
    let path = Path::top(k);
    let style = s.doc.para(StoryRef::Body, &path).and_then(|p| p.props.style.clone());
    let lv = scheme_level(s, sc, style.as_deref(), level);
    // First on the heading list as its place gives it.
    let by_style = s
        .doc
        .para(StoryRef::Body, &path)
        .map(|p| s.doc.styles.resolve_para(&wordcraft_doc::props::ParaProps { numbering: None, ..p.props.clone() }).numbering)
        .unwrap_or_default();
    let para = s.doc.para_mut(StoryRef::Body, &path)?;
    para.props.numbering = by_style.is_none_or(|b| b.num != sc.num).then_some(NumRef { num: sc.num, level: lv });
    para.touch();
    let natural = shown(s, sc).into_iter().find(|(b, _, _)| *b == k).map(|(_, _, v)| v);
    if natural.as_deref() != Some(parts) && !parts.is_empty() {
        pin(s, sc, k, lv, parts)?;
    }
    heal(s)?;
    Ok(())
}

/// Make the paragraph at `at` the heading its typed text says ([`enter_line`]): the typed number
/// goes (tracked when Track Changes is on) and the paragraph becomes Heading *n*, numbered so it
/// shows the typed number; an appendix heading keeps its text and takes no number. Returns the
/// paragraph's end.
pub(crate) fn apply_line(s: &mut Session, at: &Pos, line: &Line) -> Result<Pos, CmdError> {
    let k = at.path.0.first().copied().unwrap_or(0) as usize;
    let start = Pos { off: 0, ..at.clone() };
    let before = s.doc.para(at.story, &at.path).map(|p| p.props.clone());
    match line {
        Line::Appendix(a) => {
            crate::cmd::para::set_heading(s, at.story, &at.path, Some(a.level))?;
            // `num: 0`: the heading styles' numbering doesn't apply to it.
            let para = s.doc.para_mut(at.story, &at.path)?;
            para.props.numbering = Some(NumRef { num: 0, level: 0 });
            para.touch();
        }
        Line::Heading(h) => {
            s.sel = Selection { anchor: start.clone(), focus: Pos { off: h.typed, ..start.clone() } };
            crate::cmd::delete_selection(s)?;
            let level = u8::try_from(h.parts.len().clamp(1, 9)).unwrap_or(1);
            crate::cmd::para::set_heading(s, at.story, &at.path, Some(level))?;
            number_as(s, k, level, &h.parts, h.chapter)?;
        }
    }
    if let Some(before) = before {
        crate::cmd::fmt_revisions::record_para_props(s, at.story, &at.path, &before)?;
    }
    let end = s.doc.para(at.story, &at.path).map_or(0, |p| p.len());
    Ok(Pos { off: end, ..at.clone() })
}

/// A number typed in front of a numbered heading, on the space after it ("2.2 " before
/// "Methods"): the heading takes that number, and the level its parts give. `false` when the
/// caret's paragraph isn't a numbered heading with text after the caret, or the text before the
/// caret isn't a number.
pub(crate) fn renumber_typed(s: &mut Session) -> Result<bool, CmdError> {
    let ac = &s.prefs.autocorrect;
    let f = s.sel.focus.clone();
    if !ac.enabled || !ac.structure || !s.sel.is_collapsed() || f.story != StoryRef::Body || f.path.0.len() != 1 {
        return Ok(false);
    }
    let Some(para) = s.doc.para_at(&f) else { return Ok(false) };
    let Some(prefix) = para.text.get(..f.off) else { return Ok(false) };
    // A number, an optional full stop and the space just typed, with the title after the caret.
    let Some((parts, n)) = decimal(prefix) else { return Ok(false) };
    let tail = prefix.get(n..).unwrap_or("");
    if !(tail == " " || tail == ". ") || para.text.get(f.off..).is_none_or(|t| t.trim().is_empty() || t.starts_with(' ')) {
        return Ok(false);
    }
    let k = f.path.0.first().copied().unwrap_or(0) as usize;
    let hs = headings(s);
    let Some(sc) = scheme(s, &hs) else { return Ok(false) };
    let on_scheme = hs.iter().any(|h| h.k == k && h.num.is_some_and(|x| s.doc.numbering.abstract_of(x.num).is_some_and(|a| a.id == sc.abs)));
    if !on_scheme || para.run_ranges().any(|(_, c)| c.del.is_some()) {
        return Ok(false);
    }
    let start = Pos { off: 0, ..f.clone() };
    s.sel = Selection { anchor: start.clone(), focus: f.clone() };
    crate::cmd::delete_selection(s)?;
    let level = u8::try_from(parts.len().clamp(1, 9)).unwrap_or(1);
    crate::cmd::para::set_heading(s, f.story, &f.path, Some(level))?;
    number_as(s, k, level, &parts, false)?;
    s.sel = Selection::caret(start);
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_lines_are_told_from_text() {
        let parts = |t: &str| heading_line(t).map(|h| h.parts);
        assert_eq!(parts("1 Introduction"), Some(vec![1]));
        assert_eq!(parts("2.1 Sampling design"), Some(vec![2, 1]));
        assert_eq!(parts("2.1. Sampling design"), Some(vec![2, 1]));
        assert_eq!(parts("3.2.1\tPilot study"), Some(vec![3, 2, 1]));
        assert_eq!(parts("Chapter 3"), Some(vec![3]));
        assert_eq!(parts("Chapter 3: Results"), Some(vec![3]));
        assert_eq!(parts("Chapter 4 \u{2014} Discussion"), Some(vec![4]));
        assert_eq!(heading_line("2.1 Sampling").map(|h| h.typed), Some(4));
        assert_eq!(heading_line("Chapter 3: Results").map(|h| h.typed), Some(11));
        for body in [
            "3.5 mg of buffer was added",
            "2019 Annual report",
            "2.1 This is a sentence.",
            "1.5 times the control value",
            "12 angry men",
            "3.14159 Pi",
            "2.1",
            "2.1 ",
            "2.1Methods",
            "Chapter three",
            "chapter 3 Results",
            "1,000 Samples",
            "5% Agarose",
            "4 Is this a heading?",
        ] {
            assert_eq!(heading_line(body), None, "{body}");
        }
        let long = format!("2 {}", "Word ".repeat(20));
        assert_eq!(heading_line(long.trim()), None);
    }

    #[test]
    fn appendix_lines_are_told_from_text() {
        let level = |t: &str| appendix_line(t).map(|a| a.level);
        assert_eq!(level("Appendix A"), Some(1));
        assert_eq!(level("Appendix B: Survey questions"), Some(1));
        assert_eq!(level("Appendix 2.2 Raw data"), Some(2));
        assert_eq!(level("Appendix IV"), Some(1));
        assert_eq!(level("Appendix A.1 Consent form"), Some(2));
        assert_eq!(level("Appendix A contains the survey."), None);
        assert_eq!(level("Appendix"), None);
        assert_eq!(level("Appendices"), None);
        assert_eq!(appendix_sub_line("A.1 Survey questions").map(|a| a.level), Some(2));
        assert_eq!(appendix_sub_line("A. Smith wrote"), None);
    }
}
