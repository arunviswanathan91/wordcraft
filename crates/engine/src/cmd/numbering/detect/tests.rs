use super::*;
use serde_json::json;
use wordcraft_doc::Document;
use wordcraft_doc::numbering::Counters;

fn session(text: &str) -> Session {
    Session::new(Document::from_text(text))
}

/// (number, level, confidence) of each candidate.
fn found(s: &mut Session) -> Vec<(String, u64, String)> {
    let r = s.run("numbering.detect", &json!({})).unwrap();
    r["candidates"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["number"].as_str().unwrap().to_string(), c["level"].as_u64().unwrap(), c["confidence"].as_str().unwrap().to_string()))
        .collect()
}

/// (text, style, number shown) of each top-level body paragraph.
fn shown(s: &Session) -> Vec<(String, Option<String>, String)> {
    let mut c = Counters::default();
    s.doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .map(|p| {
            let n = p.props.numbering.or_else(|| s.doc.styles.resolve_para(&p.props).numbering).filter(|n| n.num != 0);
            let label = n.and_then(|n| c.next_label(&s.doc.numbering, n.num, n.level)).map(|(l, _)| l).unwrap_or_default();
            (p.plain_text(), p.props.style.clone(), label)
        })
        .collect()
}

const THESIS: &str = "1 Introduction\nWe study kilns in this chapter, which is long enough to be body text and ends with a full stop.\n1.1 Background\nMore text that is body text and ends with a full stop.\n1.2 Aims\n2 Methods\n2.1 Sites\n2.1.1 North\n2.1.2 South\n3 Results";

#[test]
fn typed_headings_are_found_with_their_levels() {
    let mut s = session(THESIS);
    let f = found(&mut s);
    let numbers: Vec<(&str, u64)> = f.iter().map(|(n, l, _)| (n.as_str(), *l)).collect();
    assert_eq!(numbers, [("1", 1), ("1.1", 2), ("1.2", 2), ("2", 1), ("2.1", 2), ("2.1.1", 3), ("2.1.2", 3), ("3", 1)]);
    // Multi-part numbers in sequence and short are sure; a lone number without heading looks less so.
    assert!(f.iter().filter(|(n, _, _)| n.contains('.')).all(|(_, _, c)| c == "high"), "{f:?}");
    assert!(f.iter().all(|(_, _, c)| c != "low"), "{f:?}");
}

#[test]
fn converting_makes_real_numbered_headings() {
    let mut s = session(THESIS);
    let depth = s.undo_depth();
    let r = s.run("numbering.convert", &json!({})).unwrap();
    assert_eq!(r["converted"].as_array().unwrap().len(), 8, "{r}");
    assert_eq!(s.undo_depth(), depth + 1, "one undo step");
    let v = shown(&s);
    let heads: Vec<(&str, Option<&str>, &str)> =
        v.iter().filter(|(_, st, _)| st.is_some()).map(|(t, st, l)| (t.as_str(), st.as_deref(), l.as_str())).collect();
    assert_eq!(
        heads,
        [
            ("Introduction", Some("Heading1"), "1"),
            ("Background", Some("Heading2"), "1.1"),
            ("Aims", Some("Heading2"), "1.2"),
            ("Methods", Some("Heading1"), "2"),
            ("Sites", Some("Heading2"), "2.1"),
            ("North", Some("Heading3"), "2.1.1"),
            ("South", Some("Heading3"), "2.1.2"),
            ("Results", Some("Heading1"), "3"),
        ]
    );
    // Body text is left alone, and the result has nothing for numbering.check to report.
    assert!(v[1].0.starts_with("We study") && v[1].1.is_none() && v[1].2.is_empty());
    assert_eq!(s.run("numbering.check", &json!({})).unwrap()["items"], json!([]));
    // Undo puts the typed numbers back.
    s.run("edit.undo", &json!({})).unwrap();
    assert_eq!(s.doc.body.first().and_then(|b| b.as_para()).map(|p| p.plain_text()).as_deref(), Some("1 Introduction"));
}

/// A chapter file starting at 3.1 keeps its numbers.
#[test]
fn numbers_that_do_not_start_at_one_keep_their_values() {
    let mut s = session("3.1 Sites\nText.\n3.2 Samples\n3.2.1 Pottery\n3.3 Dating");
    s.run("numbering.convert", &json!({})).unwrap();
    let labels: Vec<String> = shown(&s).into_iter().filter(|(_, st, _)| st.is_some()).map(|(_, _, l)| l).collect();
    assert_eq!(labels, ["3.1", "3.2", "3.2.1", "3.3"]);
    assert_eq!(s.run("numbering.check", &json!({})).unwrap()["items"], json!([]));
}

/// A jump in the numbers is reported and lowers confidence; body text starting with a number,
/// numbers with letters and long sentences aren't taken for headings.
#[test]
fn jumps_body_text_and_letters_are_told_apart() {
    let mut s = session(
        "1 Intro\n1.1 One\n1.3 Three\n2024 was a year of many results.\n5. This list item is a full sentence that ends with a full stop.\nA.1 Appendix table\nIV. Results",
    );
    let r = s.run("numbering.detect", &json!({})).unwrap();
    let c = r["candidates"].as_array().unwrap();
    let get = |n: &str| c.iter().find(|x| x["number"] == n).cloned().unwrap_or(Value::Null);
    assert!(get("1.3")["reasons"].to_string().contains("out of sequence"), "{r}");
    assert_eq!(get("2024"), Value::Null, "a year isn't a number to convert");
    assert_eq!(get("5.")["confidence"], "low", "a sentence");
    assert_eq!((get("A.1")["convertible"].clone(), get("IV.")["convertible"].clone()), (json!(false), json!(false)));
    // Converting the defaults leaves the sentence, the letters and the roman numeral alone.
    s.run("numbering.convert", &json!({})).unwrap();
    let texts: Vec<String> = shown(&s).into_iter().map(|(t, _, _)| t).collect();
    assert!(texts.iter().any(|t| t.starts_with("5. This list item")));
    assert!(texts.iter().any(|t| t == "A.1 Appendix table") && texts.iter().any(|t| t == "IV. Results"));
}

/// The writer picks the candidates and may change a level; anything else is refused or ignored.
#[test]
fn convert_takes_chosen_items() {
    let mut s = session("1 Intro\n1.1 Background\n2 Methods");
    s.run("numbering.convert", &json!({"items": [{"path": [1], "level": 3}]})).unwrap();
    let v = shown(&s);
    assert_eq!(v[0].0, "1 Intro", "not chosen");
    assert_eq!((v[1].0.as_str(), v[1].1.as_deref()), ("Background", Some("Heading3")));
    assert!(s.run("numbering.convert", &json!({"items": [{"path": [0], "level": 12}]})).is_err());
    assert!(s.run("numbering.convert", &json!({"items": [{"level": 1}]})).is_err());
    assert!(s.run("numbering.convert", &json!({"items": 3})).is_err());
    let r = s.run("numbering.convert", &json!({"items": [{"path": [999]}]})).unwrap();
    assert_eq!(r["converted"], json!([]));
}

/// With a heading list already in the document, converted headings join it.
#[test]
fn converted_headings_join_an_existing_heading_list() {
    let mut s = session("Overview\n5 Methods\n5.1 Sites");
    s.run("select.text", &json!({"text": "Overview"})).unwrap();
    s.run("styles.apply", &json!({"style": "Heading 1"})).unwrap();
    s.run("list.multilevelPreset", &json!({"preset": "headings"})).unwrap();
    s.run("numbering.convert", &json!({})).unwrap();
    let labels: Vec<String> = shown(&s).into_iter().map(|(_, _, l)| l).collect();
    assert_eq!(labels, ["1", "2", "2.1"], "Methods follows Overview's 1, whatever was typed");
    assert_eq!(s.run("numbering.check", &json!({})).unwrap()["items"], json!([]));
}
