use serde_json::{Value, json};

use super::*;

fn run(s: &mut Session, id: &str, v: Value) -> Value {
    s.run(id, &v).unwrap_or_else(|e| panic!("{id}: {e}"))
}

/// Type like a person: `\n` is Enter, everything else one character at a time.
fn typ(s: &mut Session, keys: &str) {
    for c in keys.chars() {
        match c {
            '\n' => run(s, "text.newParagraph", json!({})),
            c => run(s, "text.insert", json!({"text": c.to_string()})),
        };
    }
}

fn typed(keys: &str) -> Session {
    let mut s = Session::new(wordcraft_doc::Document::new());
    typ(&mut s, keys);
    s
}

/// The outline's headings as "number text" (or "text" when unnumbered).
fn headings(s: &mut Session) -> Vec<String> {
    run(s, "structure.outline", json!({}))["headings"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| match h["number"].as_str() {
            Some(n) => format!("{n} {}", h["text"].as_str().unwrap()),
            None => h["text"].as_str().unwrap().to_string(),
        })
        .collect()
}

/// The outline's captions of `kind` as "label|text".
fn captions(s: &mut Session, kind: &str) -> Vec<String> {
    run(s, "structure.outline", json!({}))[kind]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| format!("{}|{}", h["label"].as_str().unwrap(), h["text"].as_str().unwrap()))
        .collect()
}

fn body_text(s: &Session) -> Vec<String> {
    s.doc.body.iter().filter_map(|b| b.as_para()).map(|p| p.plain_text()).collect()
}

#[test]
fn caption_lines_are_told_from_text() {
    let c = |t: &str| caption_line(t).map(|c| (c.id, c.chapter, c.sep, c.n));
    assert_eq!(c("Figure 2.1: Cell counts"), Some(("Figure", Some("2".into()), ".", 1)));
    assert_eq!(c("Figure 3"), Some(("Figure", None, "", 3)));
    assert_eq!(c("Fig. 4. Growth curves"), Some(("Figure", None, "", 4)));
    assert_eq!(c("Table 3.2 \u{2014} Reagents"), Some(("Table", Some("3".into()), ".", 2)));
    assert_eq!(c("Table 2-1 Sample sizes"), Some(("Table", Some("2".into()), "-", 1)));
    assert_eq!(c("Figure A.1: Map"), Some(("Figure", Some("A".into()), ".", 1)));
    assert_eq!(c("Figure S1: Supplementary"), Some(("Figure", Some("S".into()), "", 1)));
    assert_eq!(c("Figure 2.1.3: Deep"), Some(("Figure", Some("2.1".into()), ".", 3)));
    assert_eq!(c("Equation 5"), Some(("Equation", None, "", 5)));
    for body in [
        "Figure 2.1 shows the counts",
        "Table 3 lists the reagents.",
        "Figures 2 and 3",
        "Figure",
        "Figure x",
        "figure 2: lower",
        "Figure 2and",
        "Tables 1",
    ] {
        assert_eq!(c(body), None, "{body}");
    }
}

#[test]
fn typed_numbers_become_headings_on_enter() {
    let mut s = typed("1 Introduction\nSome text here.\n1.1 Background\nMore text.\n1.2 Aims\n2 Methods\n2.1 Sampling\n");
    assert_eq!(headings(&mut s), ["1 Introduction", "1.1 Background", "1.2 Aims", "2 Methods", "2.1 Sampling"]);
    // The typed numbers are gone from the text: the list shows them.
    assert_eq!(body_text(&s).iter().take(3).map(String::as_str).collect::<Vec<_>>(), ["Introduction", "Some text here.", "Background"]);
    // Body text that starts with a number stays text.
    let mut s = typed("3.5 mg of buffer was added\n2019 was a dry year\n12 angry men\n");
    assert!(headings(&mut s).is_empty());
    assert_eq!(body_text(&s)[0], "3.5 mg of buffer was added");
}

#[test]
fn the_typed_number_wins() {
    // A chapter file that starts at 3.
    let mut s = typed("3 Results\n3.1 Yield\n3.2 Purity\n4 Discussion\n");
    assert_eq!(headings(&mut s), ["3 Results", "3.1 Yield", "3.2 Purity", "4 Discussion"]);
    // A section file that starts in the middle of a chapter.
    let mut s = typed("2.2 Sampling\n2.3 Analysis\n2.3.1 Models\n3 Results\n");
    assert_eq!(headings(&mut s), ["2.2 Sampling", "2.3 Analysis", "2.3.1 Models", "3 Results"]);
    // A jump: the heading shows what was typed, and the next ones count on from it.
    let mut s = typed("1 One\n5 Five\n6 Six\n");
    assert_eq!(headings(&mut s), ["1 One", "5 Five", "6 Six"]);
}

#[test]
fn a_number_typed_before_a_heading_renumbers_it() {
    let mut s = typed("1 One\n1.1 First\n1.2 Second\n2 Two\n");
    // "2.2 " typed in front of "First": caret to its start.
    let k = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "First")).unwrap();
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(k), off: 0 }}));
    typ(&mut s, "4.2 ");
    assert_eq!(headings(&mut s), ["1 One", "1.2 First", "1.3 Second", "2 Two"], "a parent that counted keeps its number");
    // Changing the level by the number of parts: "3 " makes it a chapter.
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(k), off: 0 }}));
    typ(&mut s, "3 ");
    assert_eq!(headings(&mut s), ["1 One", "3 First", "3.1 Second", "4 Two"]);
    // Back in sequence: the pin goes, so later edits renumber it.
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(k), off: 0 }}));
    typ(&mut s, "2 ");
    assert_eq!(headings(&mut s), ["1 One", "2 First", "2.1 Second", "3 Two"]);
    assert!(s.doc.para(StoryRef::Body, &Path::top(k)).unwrap().props.numbering.is_none(), "numbered by its style again");
}

#[test]
fn chapter_and_appendix_lines() {
    let mut s = typed("Chapter 1: Beginnings\n1.1 Home\nChapter 2 Journeys\nAppendix A: Maps\nA.1 The north\nAppendix B\n");
    assert_eq!(headings(&mut s), ["Chapter 1 Beginnings", "1.1 Home", "Chapter 2 Journeys", "Appendix A: Maps", "A.1 The north", "Appendix B"]);
    let o = run(&mut s, "structure.outline", json!({}));
    let appendix: Vec<bool> = o["headings"].as_array().unwrap().iter().map(|h| h["appendix"].as_bool().unwrap()).collect();
    assert_eq!(appendix, [false, false, false, true, true, true]);
    let levels: Vec<u64> = o["headings"].as_array().unwrap().iter().map(|h| h["level"].as_u64().unwrap()).collect();
    assert_eq!(levels, [1, 2, 1, 1, 2, 1]);
}

#[test]
fn typed_captions_become_captions_that_renumber() {
    let mut s = typed("1 Introduction\nFigure 1.1: First\nText between.\nFigure 1.2: Second\n2 Methods\nFigure 2.1: Third\nTable 2.1: Reagents\n");
    assert_eq!(captions(&mut s, "figures"), ["Figure 1.1|First", "Figure 1.2|Second", "Figure 2.1|Third"]);
    assert_eq!(captions(&mut s, "tables"), ["Table 2.1|Reagents"]);
    // A figure typed between the two renumbers what follows.
    let k = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "Text between.")).unwrap();
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(k), off: "Text between.".len() }}));
    typ(&mut s, "\nFigure 1.2: Inserted\n");
    assert_eq!(captions(&mut s, "figures"), ["Figure 1.1|First", "Figure 1.2|Inserted", "Figure 1.3|Second", "Figure 2.1|Third"]);
    // Continuous numbers, and a sentence that only mentions a figure.
    let mut s = typed("Figure 1: One\nFigure 2 shows nothing\nFig. 2. Two\n");
    assert_eq!(captions(&mut s, "figures"), ["Figure 1|One", "Fig. 2|Two"]);
    assert_eq!(body_text(&s)[1], "Figure 2 shows nothing");
}

#[test]
fn a_typed_caption_number_wins_until_the_numbering_catches_up() {
    // Typed out of order: the number stays as typed.
    let mut s = typed("Figure 3: Third\n");
    assert_eq!(captions(&mut s, "figures"), ["Figure 3|Third"]);
    // No chapter heading here: the chapter part stays as typed too.
    let mut s = typed("Figure 4.2: In chapter four\n");
    assert_eq!(captions(&mut s, "figures"), ["Figure 4.2|In chapter four"]);
    // Figures 1 and 2 typed above it: the pin is no longer needed and goes.
    let mut s = typed("Figure 3: Third\n");
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(0), off: 0 }}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(0), off: 0 }}));
    typ(&mut s, "Figure 1: First\nFigure 2: Second");
    run(&mut s, "text.newParagraph", json!({}));
    assert_eq!(captions(&mut s, "figures"), ["Figure 1|First", "Figure 2|Second", "Figure 3|Third"]);
    let pinned = s
        .doc
        .body
        .iter()
        .filter_map(|b| b.as_para())
        .flat_map(|p| p.objects.iter())
        .any(|o| matches!(o, InlineObject::Field { instr, .. } if instr.contains("\\r")));
    assert!(!pinned, "the pin is dropped once 3 is the figure's own number");
}

#[test]
fn a_figure_named_in_the_text_follows_its_number() {
    let mut s = typed("Figure 1: Growth\nFigure 2: Decay\nAs Figure 2 shows, and Fig. 1 too. Figure 9 is not there.\n");
    let para = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with("As ")).unwrap();
    let refs = para.objects.iter().filter(|o| matches!(o, InlineObject::Field { instr, .. } if instr.starts_with("REF "))).count();
    assert_eq!(refs, 2);
    assert_eq!(para.plain_text(), "As Figure 2 shows, and Fig. 1 too. Figure 9 is not there.");
    // A new first figure: the mentions follow their figures.
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(0), off: 0 }}));
    run(&mut s, "text.newParagraph", json!({}));
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(0), off: 0 }}));
    typ(&mut s, "Figure 1: New");
    run(&mut s, "text.newParagraph", json!({}));
    assert_eq!(captions(&mut s, "figures"), ["Figure 1|New", "Figure 2|Growth", "Figure 3|Decay"]);
    let para = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with("As ")).unwrap();
    assert_eq!(para.plain_text(), "As Figure 3 shows, and Fig. 2 too. Figure 9 is not there.");
}

#[test]
fn lists_of_figures_and_tables_show_the_bold_heading() {
    let mut s = typed(
        "1 Introduction\nFigure 1.1: Cell counts. Counted on day three in every well.\nTable 1.1: Reagents\nFigure 1.2: Growth curves\nList here\n",
    );
    // Bold "Cell counts." in the first caption: its heading.
    let k = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.text.contains("Cell counts"))).unwrap();
    let text = s.doc.body[k].as_para().unwrap().text.clone();
    let a = text.find("Cell").unwrap();
    let b = a + "Cell counts.".len();
    let pos = |off: usize| Pos { story: StoryRef::Body, path: Path::top(k), off };
    run(&mut s, "select.range", json!({"anchor": pos(a), "focus": pos(b)}));
    run(&mut s, "format.bold", json!({}));
    let o = run(&mut s, "structure.outline", json!({}));
    assert_eq!(o["figures"][0]["title"], "Cell counts");
    assert_eq!(o["figures"][1]["title"], Value::Null);
    let last = s.doc.body.len() - 1;
    let end = Pos { story: StoryRef::Body, path: Path::top(last), off: 0 };
    run(&mut s, "caret.set", json!({"pos": end}));
    let r = run(&mut s, "structure.list", json!({"of": "figures", "pageNumbers": false}));
    assert_eq!(r["entries"], 2);
    let all = body_text(&s);
    assert!(all.iter().any(|t| t == "List of Figures"), "{all:?}");
    assert!(all.iter().any(|t| t == "Figure 1.1: Cell counts"), "{all:?}");
    assert!(all.iter().any(|t| t == "Figure 1.2: Growth curves"), "{all:?}");
    // Tables, without a heading above and with the whole caption.
    let last = s.doc.body.len() - 1;
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(last), off: 0 }}));
    run(&mut s, "structure.list", json!({"of": "tables", "heading": false, "titles": "full", "pageNumbers": false}));
    let all = body_text(&s);
    assert!(!all.iter().any(|t| t == "List of Tables"), "{all:?}");
    assert_eq!(all.iter().filter(|t| *t == "Table 1.1: Reagents").count(), 2, "{all:?}");
    // The whole caption again for figures.
    let last = s.doc.body.len() - 1;
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(last), off: 0 }}));
    run(&mut s, "structure.list", json!({"of": "figures", "titles": "full", "heading": "Figures", "pageNumbers": false}));
    let all = body_text(&s);
    assert!(all.iter().filter(|t| t.starts_with("Figure 1.1: Cell counts. Counted")).count() >= 2, "{all:?}");
    assert!(s.run("structure.list", &json!({"titles": "nope"})).is_err());
}

#[test]
fn renumber_lets_typed_numbers_go() {
    // A chapter file typed as chapter 3, with a figure typed with another chapter's number.
    let mut s = typed(
        "3 Results
Figure 5.4: Early
3.1 Yield
Figure 3.7: Late
7 Discussion
",
    );
    assert_eq!(headings(&mut s), ["3 Results", "3.1 Yield", "7 Discussion"]);
    assert_eq!(captions(&mut s, "figures"), ["Figure 5.4|Early", "Figure 3.7|Late"]);
    let r = run(&mut s, "structure.renumber", json!({}));
    assert!(r["headings"].as_u64().unwrap() >= 1);
    assert_eq!(headings(&mut s), ["1 Results", "1.1 Yield", "2 Discussion"]);
    assert_eq!(captions(&mut s, "figures"), ["Figure 1.1|Early", "Figure 1.2|Late"]);
}

#[test]
fn a_section_named_in_the_text_follows_its_number() {
    let mut s =
        typed("1 Introduction\n1.1 Background\n2 Methods\n2.1 Sampling\nAs Section 2.1 says, and Chapter 1 before it. Section 9.9 is not there.\n");
    let find = |s: &Session| s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.text.starts_with("As ")).unwrap().clone();
    let refs = |p: &wordcraft_doc::Paragraph| {
        p.objects.iter().filter(|o| matches!(o, InlineObject::Field { instr, .. } if instr.starts_with("REF "))).count()
    };
    assert_eq!(refs(&find(&s)), 2);
    assert_eq!(find(&s).plain_text(), "As Section 2.1 says, and Chapter 1 before it. Section 9.9 is not there.");
    // A chapter typed before Methods: its sections, and what the text says of them, move on.
    let k = s.doc.body.iter().position(|b| b.as_para().is_some_and(|p| p.text == "Background")).unwrap();
    run(&mut s, "caret.set", json!({"pos": Pos { story: StoryRef::Body, path: Path::top(k), off: "Background".len() }}));
    typ(&mut s, "\n2 Theory\n");
    assert_eq!(headings(&mut s), ["1 Introduction", "1.1 Background", "2 Theory", "3 Methods", "3.1 Sampling"]);
    run(&mut s, "structure.update", json!({}));
    assert_eq!(find(&s).plain_text(), "As Section 3.1 says, and Chapter 1 before it. Section 9.9 is not there.");
}

#[test]
fn mentions_written_as_text_are_linked_in_one_step() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.prefs.autocorrect.structure = false;
    typ(&mut s, "1 Results\nFigure 1.1: Yield\nTable 1.1: Reagents\nFigure 1.1 shows the yield; see also Table 1.1, Section 1 and Figure 7.7.\n");
    s.prefs.autocorrect.structure = true;
    let r = run(&mut s, "structure.convert", json!({}));
    assert_eq!((r["headings"].as_u64(), r["captions"].as_u64(), r["linked"].as_u64()), (Some(1), Some(2), Some(3)));
    let p = s.doc.body.iter().filter_map(|b| b.as_para()).find(|p| p.plain_text().contains("shows the yield")).unwrap();
    assert_eq!(p.plain_text(), "Figure 1.1 shows the yield; see also Table 1.1, Section 1 and Figure 7.7.");
    assert_eq!(run(&mut s, "structure.linkMentions", json!({}))["linked"], 0, "nothing is linked twice");
}

#[test]
fn undo_brings_the_typed_line_back() {
    let mut s = typed("2.1 Sampling\n");
    assert_eq!(headings(&mut s), ["2.1 Sampling"]);
    run(&mut s, "edit.undo", json!({}));
    assert!(headings(&mut s).is_empty());
    assert_eq!(body_text(&s), ["2.1 Sampling", ""]);
}

#[test]
fn off_switch_and_other_places_leave_text_alone() {
    let mut s = Session::new(wordcraft_doc::Document::new());
    s.prefs.autocorrect.structure = false;
    typ(&mut s, "2.1 Sampling\nFigure 1: One\n");
    assert!(headings(&mut s).is_empty());
    assert!(captions(&mut s, "figures").is_empty());
    // The outline still says what was typed as structure, and convert makes it real.
    s.prefs.autocorrect.structure = true;
    let o = run(&mut s, "structure.outline", json!({}));
    let kinds: Vec<&str> = o["typed"].as_array().unwrap().iter().map(|t| t["kind"].as_str().unwrap()).collect();
    assert_eq!(kinds, ["heading", "caption"]);
    let r = run(&mut s, "structure.convert", json!({}));
    assert_eq!((r["headings"].as_u64(), r["captions"].as_u64()), (Some(1), Some(1)));
    assert_eq!(headings(&mut s), ["2.1 Sampling"]);
    assert_eq!(captions(&mut s, "figures"), ["Figure 1|One"]);
    assert!(run(&mut s, "structure.outline", json!({}))["typed"].as_array().unwrap().is_empty());
}

#[test]
fn hostile_lines_never_panic() {
    for text in [
        "9".repeat(5000),
        format!("1{}", ".1".repeat(3000)),
        format!("Figure {}", "1.".repeat(3000)),
        "Figure \u{a0}\u{a0}\u{a0}".to_string(),
        "Chapter \u{202e}3".to_string(),
        "Appendix \u{1F600}".to_string(),
        "Fig.\u{0301} 1".to_string(),
        "1 \u{0645}\u{0642}\u{062F}\u{0645}\u{0629}".to_string(),
        "É.1 Été".to_string(),
        "Figure 4294967296: Big".to_string(),
    ] {
        let _ = caption_line(&text);
        let _ = typed::heading_line(&text);
        let _ = typed::appendix_line(&text);
        let _ = typed::appendix_sub_line(&text);
        let _ = mention_before(&text, text.len());
        let _ = heading_mention(&text);
        let mut s = Session::new(wordcraft_doc::Document::new());
        run(&mut s, "text.insert", json!({"text": text, "raw": true}));
        run(&mut s, "text.newParagraph", json!({}));
        run(&mut s, "structure.outline", json!({}));
        run(&mut s, "structure.convert", json!({}));
    }
}
