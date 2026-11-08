use super::*;
use serde_json::json;
use wordcraft_doc::Document;

/// A document of `lines` (style, text), each line a paragraph in that style ("" = Normal), with
/// the 1 / 1.1 / 1.1.1 heading list applied.
fn doc(lines: &[(&str, &str)]) -> Session {
    let text: Vec<&str> = lines.iter().map(|(_, t)| *t).collect();
    let mut s = Session::new(Document::from_text(&text.join("\n")));
    for (style, t) in lines {
        if !style.is_empty() {
            s.run("select.text", &json!({"text": t})).unwrap();
            s.run("styles.apply", &json!({"style": style})).unwrap();
        }
    }
    let first = lines.iter().find(|(st, _)| !st.is_empty()).map(|(_, t)| *t).unwrap();
    s.run("select.text", &json!({"text": first})).unwrap();
    s.run("list.multilevelPreset", &json!({"preset": "headings"})).unwrap();
    s
}

/// The number each top-level body paragraph shows ("" for none).
fn labels(s: &Session) -> Vec<String> {
    let mut c = Counters::default();
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| {
            let n = p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
            n.and_then(|n| c.next_label(&s.doc.numbering, n.num, n.level)).map(|(l, _)| l).unwrap_or_default()
        })
        .collect()
}

fn kinds(s: &mut Session) -> Vec<(String, u64)> {
    let r = s.run("numbering.check", &json!({})).unwrap();
    r["items"].as_array().unwrap().iter().map(|i| (i["kind"].as_str().unwrap().to_string(), i["path"][0].as_u64().unwrap())).collect()
}

fn select(s: &mut Session, text: &str) {
    s.run("select.text", &json!({"text": text})).unwrap();
}

/// The heading list's num (what Heading 1's style numbers with).
fn heading_num(s: &Session) -> NumRef {
    let h1 = ParaProps { style: Some("Heading1".into()), ..ParaProps::default() };
    s.doc.styles.resolve_para(&h1).numbering.unwrap()
}

/// Number top-level paragraph `k` itself with `n`, as a file from Word stores a pasted or
/// restarted heading.
fn number(s: &mut Session, k: usize, n: NumRef) {
    let para = s.doc.para_mut(StoryRef::Body, &Path::top(k)).unwrap();
    para.props.numbering = Some(n);
    para.touch();
}

/// Paragraph `k` numbered by a list of its own (another document's heading list, say).
fn other_list(s: &mut Session, k: usize) {
    let num = std::sync::Arc::make_mut(&mut s.doc.numbering).add_list(wordcraft_doc::numbering::ListKind::Legal);
    number(s, k, NumRef { num, level: 0 });
}

#[test]
fn a_clean_heading_list_has_nothing_to_report() {
    let mut s = doc(&[("Heading 1", "Intro"), ("", "Body text"), ("Heading 2", "Background"), ("Heading 1", "Methods")]);
    let r = s.run("numbering.check", &json!({})).unwrap();
    assert!(r["scheme"].is_object(), "{r}");
    assert_eq!(r["headings"], 3);
    assert_eq!(kinds(&mut s), vec![]);
    assert_eq!(labels(&s), ["1", "", "1.1", "2"]);
}

/// A heading numbered with the Numbering button gets a list of its own and starts again at 1;
/// the repair puts it back on the heading list, one undo step.
#[test]
fn a_heading_on_another_list_is_found_and_put_back() {
    let mut s = doc(&[("Heading 1", "Intro"), ("Heading 1", "Methods"), ("Heading 1", "Results")]);
    other_list(&mut s, 2);
    assert_eq!(labels(&s)[2], "1.", "numbered on its own: {:?}", labels(&s));
    assert_eq!(kinds(&mut s), vec![("otherList".to_string(), 2)]);
    let depth = s.undo_depth();
    let r = s.run("numbering.repair", &json!({})).unwrap();
    assert_eq!(r["fixed"].as_array().unwrap().len(), 1, "{r}");
    assert_eq!(r["remaining"], json!([]));
    assert_eq!(labels(&s), ["1", "2", "3"]);
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(kinds(&mut s), vec![("otherList".to_string(), 2)]);
}

/// Restart at 1 on a heading is reported where it happens and undone by the repair.
#[test]
fn a_restart_on_a_heading_is_found_and_removed() {
    let mut s = doc(&[("Heading 1", "Intro"), ("Heading 1", "Methods"), ("Heading 1", "Results")]);
    let h = heading_num(&s).num;
    let restarted = std::sync::Arc::make_mut(&mut s.doc.numbering).restart(h).unwrap();
    number(&mut s, 2, NumRef { num: restarted, level: 0 });
    assert_eq!(labels(&s), ["1", "2", "1"]);
    assert_eq!(kinds(&mut s), vec![("restart".to_string(), 2)]);
    s.run("numbering.repair", &json!({"kinds": ["restart"]})).unwrap();
    assert_eq!(labels(&s), ["1", "2", "3"]);
    assert_eq!(kinds(&mut s), vec![]);
}

/// A Heading 3 straight after a Heading 1 is a skipped level; the repair moves it, and the
/// heading under it, up one level.
#[test]
fn a_skipped_level_moves_up_with_the_headings_under_it() {
    let mut s = doc(&[("Heading 1", "Intro"), ("Heading 3", "Detail"), ("Heading 4", "Finer"), ("Heading 1", "Methods")]);
    assert_eq!(kinds(&mut s), vec![("skippedLevel".to_string(), 1)]);
    s.run("numbering.repair", &json!({"items": [{"kind": "skippedLevel", "path": [1]}]})).unwrap();
    let styles: Vec<Option<String>> = s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.props.style.clone()).collect();
    assert_eq!(styles[1].as_deref(), Some("Heading2"));
    assert_eq!(styles[2].as_deref(), Some("Heading3"), "the heading under it moves too");
    assert_eq!(labels(&s), ["1", "1.1", "1.1.1", "2"]);
    assert_eq!(kinds(&mut s), vec![]);
}

/// A numbered heading that also starts with a typed number shows it twice; the repair deletes
/// the typed one.
#[test]
fn a_typed_number_on_a_numbered_heading_is_deleted() {
    let mut s = doc(&[("Heading 1", "Intro"), ("Heading 2", "1.1 Background"), ("Heading 1", "Methods")]);
    assert_eq!(kinds(&mut s), vec![("doubleNumber".to_string(), 1)]);
    s.run("numbering.repair", &json!({})).unwrap();
    let text = s.doc.body.get(1).and_then(|b| b.as_para()).map(|p| p.plain_text()).unwrap_or_default();
    assert_eq!(text, "Background");
    assert_eq!(kinds(&mut s), vec![]);
}

/// Repairing only some items leaves the others; unknown kinds and bad items are refused.
#[test]
fn repair_takes_only_what_it_is_given() {
    let mut s = doc(&[("Heading 1", "Intro"), ("Heading 2", "1.1 Background"), ("Heading 1", "Methods"), ("Heading 1", "Results")]);
    other_list(&mut s, 3);
    assert_eq!(kinds(&mut s).len(), 2);
    s.run("numbering.repair", &json!({"kinds": ["doubleNumber"]})).unwrap();
    assert_eq!(kinds(&mut s), vec![("otherList".to_string(), 3)]);
    assert!(s.run("numbering.repair", &json!({"kinds": ["everything"]})).is_err());
    assert!(s.run("numbering.repair", &json!({"items": [{"kind": "otherList"}]})).is_err());
    assert!(s.run("numbering.repair", &json!({"items": "all"})).is_err());
    // An item that isn't there (any more) fixes nothing.
    let r = s.run("numbering.repair", &json!({"items": [{"kind": "otherList", "path": [0]}]})).unwrap();
    assert_eq!(r["fixed"], json!([]));
}

/// Without numbered headings there is no heading list and nothing to report.
#[test]
fn unnumbered_headings_have_no_scheme() {
    let mut s = Session::new(Document::from_text("Intro\n1.1 Background"));
    select(&mut s, "Intro");
    s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    let r = s.run("numbering.check", &json!({})).unwrap();
    assert_eq!(r["scheme"], Value::Null);
    assert_eq!(r["items"], json!([]));
}

#[test]
fn typed_numbers() {
    let yes = [
        ("3 Methods", "3 "),
        ("3. Methods", "3. "),
        ("3.2 Sampling", "3.2 "),
        ("3.2.1\tSites", "3.2.1\t"),
        ("12) Twelve", "12) "),
        ("IV. Results", "IV. "),
        ("A. Appendix text", "A. "),
        ("A.1 Tables", "A.1 "),
        ("(a) First", "(a) "),
        ("Chapter 3 Methods", "Chapter 3 "),
        ("Section 2.1 Scope", "Section 2.1 "),
        ("Appendix A Data", "Appendix A "),
        ("Part II The rest", "Part II "),
    ];
    for (text, prefix) in yes {
        assert_eq!(typed_number(text), Some(prefix.len()), "{text:?}");
    }
    for text in [
        "2024 results",
        "Methods",
        "A study of things",
        "I think so",
        "3",
        "3.2 ",
        "Chapter one",
        "Part of the problem",
        "12345 things",
        "",
        "Ünïcode héading",
    ] {
        assert_eq!(typed_number(text), None, "{text:?}");
    }
}
