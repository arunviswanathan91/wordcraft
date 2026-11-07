//! Heading numbering check and repair (`numbering.check`, `numbering.repair`): find what makes
//! heading numbers like 3, 3.2 and 3.2.1 go wrong, and fix it. A heading's number should come
//! only from its level and its place in one list, the document's heading list (its *scheme*:
//! the list Heading 1's style numbers with, else the list the first numbered heading uses).
//! The problems found:
//!
//! - `otherList`: a heading numbered by a different list, which counts on its own (pasted from
//!   another document, or numbered with the button instead of through its style): it restarts
//!   at 1 or carries on a different count.
//! - `restart`: a heading where the heading list starts again (Restart Numbering, Set Numbering
//!   Value), which is easy to set by mistake and invisible afterwards.
//! - `skippedLevel`: a heading more than one level below the heading before it (a Heading 3
//!   straight after a Heading 1), so its number invents a level that isn't there.
//! - `doubleNumber`: a numbered heading whose text starts with a typed number as well
//!   ("3.2 3.2 Methods").
//!
//! Only the body's top-level paragraphs are looked at (headings in tables, text boxes and notes
//! aren't). Both commands work on any document.

use serde_json::{Value, json};
use std::collections::HashSet;
use wordcraft_doc::numbering::Counters;
use wordcraft_doc::props::{NumRef, ParaProps};
use wordcraft_doc::section::NumFormat;
use wordcraft_doc::{Block, Path, Pos, StoryRef};

use crate::{CmdError, CmdResult, CommandSpec, Session};

/// Most problems reported (and repaired by one command): a hostile document can't make the
/// answer, or the repair loop, unbounded.
pub const MAX_ITEMS: usize = 1000;

/// The most characters of a paragraph's start read for a typed number.
const TYPED_NUMBER_SCAN: usize = 40;

/// The kinds of problem, as the commands name them.
pub const KINDS: [&str; 4] = ["otherList", "restart", "skippedLevel", "doubleNumber"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("numbering.check", "Check Heading Numbering", "Home › Paragraph › Multilevel List", |s, _| Ok(report(s)))
            .params(r#"{} → {"scheme": {"num", "abstract"} | null, "headings": n, "items": [{"kind": "otherList|restart|skippedLevel|doubleNumber", "path": [block], "level": 1-9, "label": the number shown | null, "text": the heading's start, "message", "repair"}] (at most 1000)}"#)
            .pure(),
        CommandSpec::new("numbering.repair", "Repair Heading Numbering", "Home › Paragraph › Multilevel List", repair).params(
            r#"{"items"?: [{"kind", "path": [block]}] (from numbering.check; default every problem), "kinds"?: ["otherList"|"restart"|"skippedLevel"|"doubleNumber"] (only these)} → {"fixed": [{"kind", "path", "repair"}], "remaining": numbering.check's items} — otherList and restart: the heading goes back to the heading list; skippedLevel: the heading and the headings under it go up to the level after the heading before them; doubleNumber: the typed number is deleted. One undo step"#,
        ),
    ]
}

/// A top-level body paragraph that is a heading.
struct Heading {
    /// Block index in the body.
    k: usize,
    /// 1–9.
    level: u8,
    /// Its numbering: its own or its style's (None: not numbered).
    num: Option<NumRef>,
    /// Its own paragraph numbering (not its style's).
    own: Option<NumRef>,
    style: Option<String>,
    /// The paragraph's text (objects as placeholder characters).
    text: String,
}

/// The document's heading list: a num, and the abstract list whose counters it uses.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Scheme {
    num: u32,
    abs: u32,
}

/// One problem found.
#[derive(Clone, Debug)]
struct Item {
    kind: &'static str,
    k: usize,
    level: u8,
    label: Option<String>,
    text: String,
    message: String,
    repair: String,
    /// skippedLevel: the level the heading should have.
    want_level: u8,
    /// doubleNumber: the typed number's length (bytes, with the space after it).
    typed: usize,
}

/// Headings of the body in order.
fn headings(s: &Session) -> Vec<Heading> {
    let doc = &s.doc;
    doc.body
        .iter()
        .enumerate()
        .filter_map(|(k, b)| {
            let Block::Para(p) = &**b else { return None };
            let r = doc.styles.resolve_para(&p.props);
            let level = r.outline_level.filter(|l| *l < 9)? + 1;
            let num = r.numbering.filter(|n| n.num != 0 && doc.numbering.num(n.num).is_some());
            Some(Heading { k, level, num, own: p.props.numbering, style: p.props.style.clone(), text: p.text.clone() })
        })
        .collect()
}

/// Whether a list level counts in numbers (not bullets or nothing).
fn numeric(s: &Session, n: NumRef) -> bool {
    s.doc.numbering.level(n.num, n.level).is_some_and(|l| !matches!(l.format, NumFormat::Bullet | NumFormat::None))
}

/// The heading list: Heading 1's style's list, else the first numbered heading's.
fn scheme(s: &Session, hs: &[Heading]) -> Option<Scheme> {
    let h1 = ParaProps { style: Some("Heading1".into()), ..ParaProps::default() };
    let from_style = s.doc.styles.resolve_para(&h1).numbering.filter(|n| n.num != 0 && numeric(s, *n));
    let num = from_style.or_else(|| hs.iter().filter_map(|h| h.num).find(|n| numeric(s, *n)))?.num;
    let abs = s.doc.numbering.abstract_of(num)?.id;
    Some(Scheme { num, abs })
}

/// The length (bytes) of a typed number at the start of `text`, with the spaces after it:
/// `3`, `3.2`, `3.2.1` (optionally ending in `.` or `)`), `II.`, `A.`, `A.1`, `(a)`, and those
/// after `Chapter`, `Section`, `Part` or `Appendix`. Something must follow it. A lone number
/// without a full stop or bracket counts only up to two digits ("2024 results" is text). Only the
/// first few characters are read.
pub fn typed_number(text: &str) -> Option<usize> {
    let head: String = text.chars().take(TYPED_NUMBER_SCAN).collect();
    let mut rest = head.as_str();
    let mut worded = false;
    for w in ["chapter", "section", "part", "appendix"] {
        if let Some(after) = rest.get(..w.len()).filter(|p| p.eq_ignore_ascii_case(w)).and_then(|_| rest.get(w.len()..))
            && after.starts_with([' ', '\u{a0}'])
        {
            rest = after.trim_start_matches([' ', '\u{a0}']);
            worded = true;
            break;
        }
    }
    let token = number_token(rest, worded)?;
    let after = rest.get(token..)?;
    let spaces = after.len() - after.trim_start_matches([' ', '\t', '\u{a0}']).len();
    // A space (or tab) after the number, then text.
    if spaces == 0 || after.get(spaces..).is_none_or(|t| t.trim().is_empty()) {
        return None;
    }
    let consumed = head.len() - rest.len() + token + spaces;
    // `head` is a prefix of `text`, so the length is a char boundary there too.
    text.is_char_boundary(consumed).then_some(consumed)
}

/// The length of the number at the start of `s` (see [`typed_number`]); `worded`: it follows a
/// word such as Chapter, so a bare letter or roman numeral counts without punctuation.
fn number_token(s: &str, worded: bool) -> Option<usize> {
    let b = s.as_bytes();
    let punct = |i: usize| matches!(b.get(i), Some(b'.' | b')'));
    let digits = |i: usize| b.iter().skip(i).take_while(|c| c.is_ascii_digit()).count();
    // (a), (iv), (1)
    if b.first() == Some(&b'(') {
        let inner = b.iter().skip(1).take_while(|c| c.is_ascii_alphanumeric()).count();
        return (1..=4).contains(&inner).then_some(inner + 1).filter(|&i| b.get(i) == Some(&b')')).map(|i| i + 1);
    }
    let first = digits(0);
    if first > 0 {
        if first > 4 {
            return None;
        }
        let (mut i, mut parts) = (first, 1);
        while b.get(i) == Some(&b'.') && parts < 9 {
            let d = digits(i + 1);
            if d == 0 || d > 4 {
                break;
            }
            i += 1 + d;
            parts += 1;
        }
        if punct(i) {
            return Some(i + 1);
        }
        return (parts > 1 || worded || first <= 2).then_some(i);
    }
    // Roman numerals (I., IV.) or a letter (A., a), A.1).
    let roman = b.iter().take_while(|c| matches!(c, b'I' | b'V' | b'X' | b'L' | b'C')).count();
    if (1..=7).contains(&roman) && (punct(roman) || worded) {
        return Some(if punct(roman) { roman + 1 } else { roman });
    }
    if b.first().is_some_and(u8::is_ascii_alphabetic) && !b.get(1).is_some_and(u8::is_ascii_alphanumeric) {
        // A.1, A.1.2
        if b.get(1) == Some(&b'.') && digits(2) > 0 {
            let mut i = 1;
            while b.get(i) == Some(&b'.') && (1..=4).contains(&digits(i + 1)) {
                i += 1 + digits(i + 1);
            }
            return Some(if punct(i) { i + 1 } else { i });
        }
        if punct(1) {
            return Some(2);
        }
        return worded.then_some(1);
    }
    None
}

/// The problems in the body's heading numbering, in document order (at most [`MAX_ITEMS`]).
fn check(s: &Session) -> (Option<Scheme>, usize, Vec<Item>) {
    let hs = headings(s);
    let scheme = scheme(s, &hs);
    let numbering = &s.doc.numbering;
    let mut counters = Counters::default();
    let mut seen: HashSet<(u32, u8)> = HashSet::new();
    let mut items = Vec::new();
    let mut prev_level: Option<u8> = None;
    let mut next_heading = hs.iter().peekable();
    for (k, b) in s.doc.body.iter().enumerate() {
        let Block::Para(p) = &**b else { continue };
        let heading = next_heading.next_if(|h| h.k == k);
        let num = match heading {
            Some(h) => h.num,
            None => p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0),
        };
        let Some(n) = num else {
            if let Some(h) = heading {
                prev_level = Some(h.level);
            }
            continue;
        };
        let lv = n.level.min(8);
        let abs = numbering.abstract_of(n.num).map(|a| a.id);
        // Before counting: whether this level of the list has counted already, for restarts.
        let counted = counters.started(numbering, n.num).is_some_and(|st| st.get(lv as usize).copied().unwrap_or(false));
        let first_use = seen.insert((n.num, lv));
        let label = counters.next_label(numbering, n.num, n.level).map(|(l, _)| l);
        let Some(h) = heading else { continue };
        let level = h.level;
        let text: String = p.plain_text().chars().take(60).collect();
        let mut push = |kind: &'static str, message: String, repair: String, want_level: u8, typed: usize| {
            if items.len() < MAX_ITEMS {
                items.push(Item { kind, k, level, label: label.clone(), text: text.clone(), message, repair, want_level, typed });
            }
        };
        if let Some(sc) = scheme
            && numeric(s, n)
        {
            if abs != Some(sc.abs) {
                push(
                    "otherList",
                    "numbered by a different list from the other headings, so it counts on its own".into(),
                    "number it with the heading list".into(),
                    0,
                    0,
                );
            } else if n.num != sc.num
                && first_use
                && counted
                && let Some(start) = numbering.num(n.num).and_then(|x| x.start_overrides.iter().find(|(l, _)| *l == lv).map(|(_, v)| *v))
            {
                push(
                    "restart",
                    format!("the heading numbers start again here, at {start}"),
                    "continue the numbering of the headings before it".into(),
                    0,
                    0,
                );
            }
            if let Some(prev) = prev_level
                && level > prev.saturating_add(1)
            {
                let want = prev.saturating_add(1);
                push(
                    "skippedLevel",
                    format!("a level-{level} heading straight after a level-{prev} one: its number has a level that isn't there"),
                    format!("make it a level-{want} heading, with the headings under it"),
                    want,
                    0,
                );
            }
            if let Some(len) = typed_number(&h.text) {
                let typed: String = h.text.get(..len).unwrap_or("").trim_end().to_string();
                push(
                    "doubleNumber",
                    format!("numbered, and its text starts with the typed number {typed:?} too"),
                    "delete the typed number".into(),
                    0,
                    len,
                );
            }
        }
        prev_level = Some(level);
    }
    (scheme, hs.len(), items)
}

fn item_json(i: &Item) -> Value {
    json!({
        "kind": i.kind,
        "path": [i.k],
        "level": i.level,
        "label": i.label,
        "text": i.text,
        "message": i.message,
        "repair": i.repair,
    })
}

fn report(s: &Session) -> Value {
    let (scheme, headings, items) = check(s);
    json!({
        "scheme": scheme.map(|sc| json!({"num": sc.num, "abstract": sc.abs})),
        "headings": headings,
        "items": items.iter().map(item_json).collect::<Vec<_>>(),
    })
}

/// The block index in an item's `path` (`[k]`).
fn item_key(v: &Value) -> Result<(String, usize), CmdError> {
    let kind = v.get("kind").and_then(Value::as_str).ok_or_else(|| CmdError::Params("each item needs a `kind`".into()))?;
    if !KINDS.contains(&kind) {
        return Err(CmdError::Params(format!("unknown kind {kind:?}")));
    }
    let k = v
        .get("path")
        .and_then(Value::as_array)
        .and_then(|a| a.first())
        .and_then(Value::as_u64)
        .and_then(|k| usize::try_from(k).ok())
        .ok_or_else(|| CmdError::Params("each item needs a `path` like [3]".into()))?;
    Ok((kind.to_string(), k))
}

fn repair(s: &mut Session, v: &Value) -> CmdResult {
    let wanted: Option<HashSet<(String, usize)>> = match v.get("items") {
        Some(Value::Array(a)) => Some(a.iter().take(MAX_ITEMS).map(item_key).collect::<Result<_, _>>()?),
        Some(_) => return Err(CmdError::Params("`items` is a list".into())),
        None => None,
    };
    let kinds: Option<Vec<String>> = match v.get("kinds") {
        Some(Value::Array(a)) => {
            let ks: Vec<String> = a.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if let Some(bad) = ks.iter().find(|k| !KINDS.contains(&k.as_str())) {
                return Err(CmdError::Params(format!("unknown kind {bad:?}")));
            }
            Some(ks)
        }
        Some(_) => return Err(CmdError::Params("`kinds` is a list".into())),
        None => None,
    };
    // One problem at a time, checking again after each: a repair can change what follows (a
    // heading moved up a level takes the headings under it along).
    let mut done: HashSet<(&'static str, usize)> = HashSet::new();
    let mut fixed = Vec::new();
    while fixed.len() < MAX_ITEMS {
        let (scheme, _, items) = check(s);
        let next = items.into_iter().find(|i| {
            !done.contains(&(i.kind, i.k))
                && kinds.as_ref().is_none_or(|ks| ks.iter().any(|k| k == i.kind))
                && wanted.as_ref().is_none_or(|w| w.contains(&(i.kind.to_string(), i.k)))
        });
        let (Some(item), Some(scheme)) = (next, scheme) else { break };
        done.insert((item.kind, item.k));
        apply(s, scheme, &item)?;
        fixed.push(json!({"kind": item.kind, "path": [item.k], "repair": item.repair}));
    }
    let (_, _, remaining) = check(s);
    Ok(json!({"fixed": fixed, "remaining": remaining.iter().map(item_json).collect::<Vec<_>>()}))
}

/// The heading list's level for a heading of `level` in `style`: the level linked to that style,
/// else the one its outline level gives.
fn scheme_level(s: &Session, scheme: Scheme, style: Option<&str>, level: u8) -> u8 {
    let linked = s
        .doc
        .numbering
        .abstract_of(scheme.num)
        .and_then(|a| a.levels.iter().position(|l| l.style.is_some() && l.style.as_deref() == style).and_then(|i| u8::try_from(i).ok()));
    linked.unwrap_or(level.saturating_sub(1).min(8))
}

fn apply(s: &mut Session, scheme: Scheme, item: &Item) -> Result<(), CmdError> {
    let path = Path::top(item.k);
    match item.kind {
        "otherList" | "restart" => {
            let props = s.doc.para(StoryRef::Body, &path).map(|p| p.props.clone()).ok_or_else(|| CmdError::Failed("the heading is gone".into()))?;
            // The style's own numbering, if it is the heading list's, is enough: the paragraph's
            // own goes. Otherwise the paragraph is numbered with the heading list directly.
            let by_style = s.doc.styles.resolve_para(&ParaProps { numbering: None, ..props.clone() }).numbering.filter(|n| n.num != 0);
            let style_in_scheme =
                by_style.is_some_and(|n| s.doc.numbering.abstract_of(n.num).is_some_and(|a| a.id == scheme.abs) && n.num == scheme.num);
            let own = if style_in_scheme {
                None
            } else {
                Some(NumRef { num: scheme.num, level: scheme_level(s, scheme, props.style.as_deref(), item.level) })
            };
            let para = s.doc.para_mut(StoryRef::Body, &path)?;
            para.props.numbering = own;
            para.touch();
        }
        "skippedLevel" => {
            let delta = item.level.saturating_sub(item.want_level);
            // The heading and the headings under it (until one at its level or above).
            let hs = headings(s);
            let moved: Vec<(usize, u8, Option<NumRef>, Option<String>)> = hs
                .iter()
                .skip_while(|h| h.k != item.k)
                .enumerate()
                .take_while(|(i, h)| *i == 0 || h.level > item.level)
                .map(|(_, h)| (h.k, h.level.saturating_sub(delta).max(1), h.own, h.style.clone()))
                .collect();
            for (k, level, own, _) in moved {
                let path = Path::top(k);
                super::para::outline::set_heading(s, StoryRef::Body, &path, Some(level))?;
                // Numbered directly with the heading list: the number's level follows.
                if let Some(n) = own.filter(|n| n.num != 0 && s.doc.numbering.abstract_of(n.num).is_some_and(|a| a.id == scheme.abs)) {
                    let style = format!("Heading{level}");
                    let lv = scheme_level(s, scheme, Some(&style), level);
                    let para = s.doc.para_mut(StoryRef::Body, &path)?;
                    para.props.numbering = Some(NumRef { num: n.num, level: lv });
                    para.touch();
                }
            }
        }
        "doubleNumber" => {
            let at = Pos { story: StoryRef::Body, path, off: 0 };
            let end = Pos { off: item.typed, ..at.clone() };
            s.doc.delete_range(&at, &end)?;
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests;
