//! Typed heading numbers (`numbering.detect`, `numbering.convert`): headings whose numbers were
//! typed as text ("3.2.1 Methods"), as documents from other programs, PDFs or old templates
//! often have, found and turned into real heading numbering, which then follows the document as
//! it changes. Nothing changes until `numbering.convert` is run, with the candidates the writer
//! chose.
//!
//! A candidate is a top-level body paragraph starting with a decimal number (`3`, `3.2`,
//! `3.2.1`, with or without a full stop or bracket), or `Chapter 3` / `Section 3.2`. Its level is
//! the number of parts. How sure the guess is comes from whether the numbers form a sequence
//! (3.1 after 3, 3.2 after 3.1) and whether the paragraph looks like a heading (a heading style,
//! bold or larger text, short). Numbers with letters or roman numerals ("A.1", "IV.",
//! "Appendix A") are reported but not converted: they need a number format of their own.

use serde_json::{Value, json};
use std::collections::HashSet;
use wordcraft_doc::props::{NumRef, ParaProps};
use wordcraft_doc::{Block, Path, Pos, StoryRef};

use super::{MAX_ITEMS, Scheme, headings, scheme, scheme_level, typed_number};
use crate::{CmdError, CmdResult, Session};

/// A heading this long or longer (characters after the number) is taken for body text.
const LONG: usize = 120;
/// Short enough to look like a heading on its own.
const SHORT: usize = 80;

/// How sure a guess is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(super) enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    fn name(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// A paragraph that starts with a typed number.
#[derive(Clone, Debug)]
pub(super) struct Candidate {
    pub(super) k: usize,
    /// The number's parts (`3.2.1` → [3, 2, 1]); empty for numbers that aren't converted.
    pub(super) parts: Vec<u32>,
    /// The typed number's length (bytes, with the space after it).
    pub(super) typed: usize,
    pub(super) level: u8,
    pub(super) confidence: Confidence,
    pub(super) reasons: Vec<String>,
    /// Whether it can be converted (decimal numbers only).
    pub(super) convertible: bool,
    text: String,
    number: String,
}

/// The decimal parts of a typed number (`3.2.1.` → [3, 2, 1]; `Chapter 3` → [3]); None for a
/// number with letters or roman numerals.
fn decimal_parts(typed: &str) -> Option<Vec<u32>> {
    let mut t = typed.trim();
    for w in ["chapter", "section"] {
        if let Some(rest) = t.get(..w.len()).filter(|p| p.eq_ignore_ascii_case(w)).and_then(|_| t.get(w.len()..)) {
            t = rest.trim_start_matches([' ', '\u{a0}']);
        }
    }
    let t = t.trim_end_matches(['.', ')', ':', ' ', '\t', '\u{a0}']);
    if t.is_empty() {
        return None;
    }
    let parts: Option<Vec<u32>> = t.split('.').map(|p| if p.is_empty() || p.len() > 4 { None } else { p.parse().ok() }).collect();
    parts.filter(|v| (1..=9).contains(&v.len()))
}

/// Whether the paragraph's text looks like a heading: every run with text is bold, or larger
/// than the document's body text.
fn heading_formatting(s: &Session, k: usize) -> (bool, bool) {
    let Some(p) = s.doc.body.get(k).and_then(|b| b.as_para()) else { return (false, false) };
    let normal = s.doc.styles.resolve_char(None, &wordcraft_doc::CharProps::default()).size;
    let mut runs = p.runs.iter().filter(|r| r.len > 0).peekable();
    if runs.peek().is_none() {
        return (false, false);
    }
    let (mut bold, mut larger) = (true, true);
    for r in runs {
        let c = s.doc.styles.resolve_char(p.props.style.as_deref(), &r.props);
        bold &= c.bold;
        larger &= c.size > normal + 0.5;
    }
    (bold, larger)
}

/// The candidates in the body, in order (at most [`MAX_ITEMS`]).
pub(super) fn detect(s: &Session) -> Vec<Candidate> {
    let styled: HashSet<usize> = headings(s).iter().map(|h| h.k).collect();
    let numbered_by_list = |k: usize| {
        s.doc
            .body
            .get(k)
            .and_then(|b| b.as_para())
            .is_some_and(|p| p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).is_some_and(|n| n.num != 0))
    };
    let mut out = Vec::new();
    // The heading numbers so far, per level, to check each one follows on.
    let mut counts: Vec<u32> = Vec::new();
    for (k, b) in s.doc.body.iter().enumerate() {
        if out.len() >= MAX_ITEMS {
            break;
        }
        let Block::Para(p) = &**b else { continue };
        let Some(len) = typed_number(&p.text) else { continue };
        // Already numbered by a list: that's numbering.check's doubled number, not this.
        if numbered_by_list(k) {
            continue;
        }
        let number = p.text.get(..len).unwrap_or("").trim_end().to_string();
        let after = p.text.get(len..).unwrap_or("").trim();
        let parts = decimal_parts(&number);
        let mut reasons = Vec::new();
        let convertible = parts.is_some();
        let parts = parts.unwrap_or_default();
        let level = u8::try_from(parts.len().max(1)).unwrap_or(1);
        // In sequence: the same as the numbers before it, one more at its own level.
        let in_sequence = if parts.is_empty() {
            false
        } else {
            let depth = parts.len();
            let prefix_ok = parts.iter().take(depth - 1).enumerate().all(|(i, v)| counts.get(i).is_none_or(|c| c == v));
            let expected = if counts.len() >= depth && prefix_ok { counts.get(depth - 1).map_or(1, |c| c.saturating_add(1)) } else { 1 };
            let first = counts.is_empty();
            let ok = first || (prefix_ok && parts.last() == Some(&expected));
            if !ok {
                let was = counts.iter().map(u32::to_string).collect::<Vec<_>>().join(".");
                reasons.push(format!("out of sequence: {number} after {was}"));
            }
            counts.truncate(depth);
            for (i, v) in parts.iter().enumerate() {
                match counts.get_mut(i) {
                    Some(c) => *c = *v,
                    None => counts.push(*v),
                }
            }
            ok
        };
        let (bold, larger) = heading_formatting(s, k);
        let chars = after.chars().count();
        let style = styled.contains(&k);
        if style {
            reasons.push("in a heading style".into());
        }
        if bold {
            reasons.push("bold".into());
        }
        if larger {
            reasons.push("larger than the body text".into());
        }
        let ends_sentence = after.ends_with(['.', '!', '?']) && !after.ends_with("..");
        if chars >= LONG || ends_sentence {
            reasons.push("reads like body text".into());
        }
        if !convertible {
            reasons.push("letters or roman numerals: not converted".into());
        }
        let looks = style || bold || larger;
        let body_like = chars >= LONG || ends_sentence;
        let confidence = if !convertible || body_like {
            Confidence::Low
        } else if in_sequence && (looks || chars <= SHORT) && (looks || parts.len() > 1) {
            Confidence::High
        } else if in_sequence || looks {
            Confidence::Medium
        } else {
            Confidence::Low
        };
        out.push(Candidate { k, parts, typed: len, level, confidence, reasons, convertible, text: after.chars().take(60).collect(), number });
    }
    out
}

fn candidate_json(c: &Candidate) -> Value {
    json!({
        "path": [c.k],
        "number": c.number,
        "text": c.text,
        "level": c.level,
        "confidence": c.confidence.name(),
        "convertible": c.convertible,
        "reasons": c.reasons,
    })
}

pub(super) fn report(s: &Session) -> Value {
    json!({"candidates": detect(s).iter().map(candidate_json).collect::<Vec<_>>()})
}

/// `numbering.convert`: the chosen candidates (default: those with high or medium confidence)
/// become headings at their level with real numbering.
pub(super) fn convert(s: &mut Session, v: &Value) -> CmdResult {
    let found = detect(s);
    let chosen: Vec<(Candidate, u8)> = match v.get("items") {
        Some(Value::Array(a)) => {
            let mut out = Vec::new();
            for it in a.iter().take(MAX_ITEMS) {
                let k = it
                    .get("path")
                    .and_then(Value::as_array)
                    .and_then(|p| p.first())
                    .and_then(Value::as_u64)
                    .and_then(|k| usize::try_from(k).ok())
                    .ok_or_else(|| CmdError::Params("each item needs a `path` like [3]".into()))?;
                let level = match it.get("level") {
                    None | Some(Value::Null) => None,
                    Some(l) => Some(l.as_u64().filter(|l| (1..=9).contains(l)).ok_or_else(|| CmdError::Params("`level` is 1-9".into()))?),
                };
                if let Some(c) = found.iter().find(|c| c.k == k && c.convertible) {
                    out.push((c.clone(), level.map_or(c.level, |l| l as u8)));
                }
            }
            out
        }
        Some(_) => return Err(CmdError::Params("`items` is a list".into())),
        None => found.iter().filter(|c| c.convertible && c.confidence >= Confidence::Medium).map(|c| (c.clone(), c.level)).collect(),
    };
    if chosen.is_empty() {
        return Ok(json!({"converted": [], "scheme": Value::Null}));
    }
    // Headings first, then the heading list: an existing one, else the 1 / 1.1 / 1.1.1 list
    // linked to the heading styles (selected paragraphs already in a heading style keep it).
    for (c, level) in &chosen {
        let path = Path::top(c.k);
        let at = Pos { story: StoryRef::Body, path: path.clone(), off: 0 };
        s.doc.delete_range(&at, &Pos { off: c.typed, ..at.clone() })?;
        crate::cmd::para::outline::set_heading(s, StoryRef::Body, &path, Some(*level))?;
        let para = s.doc.para_mut(StoryRef::Body, &path)?;
        para.props.numbering = None;
        para.touch();
    }
    let sc = match scheme(s, &headings(s)) {
        Some(sc) => sc,
        None => {
            let first = chosen.first().map(|(c, _)| c.k).unwrap_or(0);
            s.sel = crate::Selection::caret(Pos { story: StoryRef::Body, path: Path::top(first), off: 0 });
            crate::cmd::lists::preset(s, &json!({"preset": "headings"}))?;
            scheme(s, &headings(s)).ok_or_else(|| CmdError::Failed("no heading list to number with".into()))?
        }
    };
    // Each converted heading on the heading list: through its style when the style is linked
    // to it, else directly.
    for (c, level) in &chosen {
        own_numbering(s, sc, c.k, *level)?;
    }
    // Numbers that didn't start at 1 (a chapter file starting at 3.1) keep their values: the
    // first converted heading starts the list there, unless numbered headings come before it
    // (it then follows them, as the headings after it do).
    let numbered_before = chosen.first().is_some_and(|(first, _)| headings(s).iter().any(|h| h.k < first.k && h.num.is_some()));
    if let Some((first, level)) = chosen.first()
        && !numbered_before
        && first.parts.iter().any(|p| *p != 1)
    {
        let numbering = std::sync::Arc::make_mut(&mut s.doc.numbering);
        if let Some(num) = numbering.restart(sc.num)
            && let Some(n) = numbering.nums.iter_mut().find(|n| n.id == num)
        {
            n.start_overrides = first.parts.iter().enumerate().filter_map(|(i, v)| u8::try_from(i).ok().map(|i| (i, *v))).collect();
            let style = format!("Heading{level}");
            let lv = scheme_level(s, sc, Some(&style), *level);
            let para = s.doc.para_mut(StoryRef::Body, &Path::top(first.k))?;
            para.props.numbering = Some(NumRef { num, level: lv });
            para.touch();
        }
    }
    let converted: Vec<Value> = chosen.iter().map(|(c, level)| json!({"path": [c.k], "number": c.number, "level": level})).collect();
    Ok(json!({"converted": converted, "scheme": sc.num}))
}

/// Number heading `k` (at `level`) with the heading list: nothing of its own when its style
/// already does, else directly.
fn own_numbering(s: &mut Session, sc: Scheme, k: usize, level: u8) -> Result<(), CmdError> {
    let path = Path::top(k);
    let props = s.doc.para(StoryRef::Body, &path).map(|p| p.props.clone()).ok_or_else(|| CmdError::Failed("the heading is gone".into()))?;
    let by_style = s.doc.styles.resolve_para(&ParaProps { numbering: None, ..props.clone() }).numbering.filter(|n| n.num != 0);
    if by_style.is_some_and(|n| s.doc.numbering.abstract_of(n.num).is_some_and(|a| a.id == sc.abs)) {
        return Ok(());
    }
    let lv = scheme_level(s, sc, props.style.as_deref(), level);
    let para = s.doc.para_mut(StoryRef::Body, &path)?;
    para.props.numbering = Some(NumRef { num: sc.num, level: lv });
    para.touch();
    Ok(())
}

#[cfg(test)]
mod tests;
