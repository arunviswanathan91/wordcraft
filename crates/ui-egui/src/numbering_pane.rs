//! The Heading Numbering pane (Home › Multilevel List › Check Heading Numbering): what
//! `numbering.check` finds wrong with the heading numbers, each a jump and a Fix, and the headings
//! whose numbers were typed as text (`numbering.detect`), to tick and convert. Kept up to date as
//! the document changes.

use egui::Ui;
use serde_json::{Value, json};

use crate::WordApp;
use crate::theme::{regular, semibold};

/// What the pane can be asked to do this frame.
enum Action {
    GoTo(u64),
    Fix(Value),
    FixAll,
    Convert(Vec<Value>),
}

pub fn pane(app: &mut WordApp, ui: &mut Ui) {
    let (problems, typed) = findings(app, ui);
    let doc = app.session.document_id();
    let mut action = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        let items = problems["items"].as_array().cloned().unwrap_or_default();
        if items.is_empty() {
            ui.label(egui::RichText::new(tl!("No heading numbering problems found.")).weak());
        } else {
            let heading = crate::i18n::fmt(tl!("Problems ({n})"), &[("n", &items.len().to_string())]);
            ui.label(egui::RichText::new(heading).font(semibold(13.0)));
            for (k, item) in items.iter().enumerate() {
                ui.push_id(("numbering_problem", k), |ui| {
                    let label = item["label"].as_str().unwrap_or("");
                    let text = item["text"].as_str().unwrap_or("");
                    let shown = if label.is_empty() { text.to_string() } else { format!("{label}  {text}") };
                    if ui.add(egui::Button::new(egui::RichText::new(shown).font(regular(12.5))).frame(false).wrap()).clicked() {
                        action = item["path"][0].as_u64().map(Action::GoTo);
                    }
                    ui.label(egui::RichText::new(item["message"].as_str().unwrap_or("")).small().weak());
                    let fix = crate::i18n::fmt(tl!("Fix: {repair}"), &[("repair", item["repair"].as_str().unwrap_or(""))]);
                    if ui.small_button(fix).clicked() {
                        action = Some(Action::Fix(json!({"kind": item["kind"], "path": item["path"]})));
                    }
                    ui.add_space(4.0);
                });
            }
            if items.len() > 1 && ui.button(tl!("Fix All")).clicked() {
                action = Some(Action::FixAll);
            }
        }
        let candidates = typed["candidates"].as_array().cloned().unwrap_or_default();
        if !candidates.is_empty() {
            ui.add_space(8.0);
            ui.separator();
            let heading = crate::i18n::fmt(tl!("Typed heading numbers ({n})"), &[("n", &candidates.len().to_string())]);
            ui.label(egui::RichText::new(heading).font(semibold(13.0)));
            ui.label(
                egui::RichText::new(tl!("Numbers typed as text don't change when headings move. Tick the headings to number automatically."))
                    .small()
                    .weak(),
            );
            let mut chosen = Vec::new();
            for (k, c) in candidates.iter().enumerate() {
                let Some(path) = c["path"][0].as_u64() else { continue };
                let convertible = c["convertible"].as_bool().unwrap_or(false);
                // Ticked to start with: the convertible ones the check is fairly sure of.
                let key = egui::Id::unique(("numbering_typed", doc, path));
                let default = convertible && c["confidence"] != "low";
                let mut on = ui.data(|d| d.get_temp::<bool>(key)).unwrap_or(default);
                ui.push_id(("numbering_typed_row", k), |ui| {
                    let text = format!("{}  {}", c["number"].as_str().unwrap_or(""), c["text"].as_str().unwrap_or(""));
                    let r = ui.add_enabled(convertible, egui::Checkbox::new(&mut on, text));
                    if r.changed() {
                        ui.data_mut(|d| d.insert_temp(key, on));
                    }
                    let level = crate::i18n::fmt(tl!("Level {n}"), &[("n", &c["level"].to_string())]);
                    let confidence = match c["confidence"].as_str() {
                        Some("high") => tl!("likely a heading"),
                        Some("medium") => tl!("maybe a heading"),
                        _ => tl!("probably not a heading"),
                    };
                    let reasons: Vec<&str> = c["reasons"].as_array().map(|a| a.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
                    let mut detail = format!("{level} · {confidence}");
                    if !reasons.is_empty() {
                        detail.push_str(" · ");
                        detail.push_str(&reasons.join(", "));
                    }
                    ui.label(egui::RichText::new(detail).small().weak());
                    if ui.small_button(tl!("Go to")).clicked() {
                        action = Some(Action::GoTo(path));
                    }
                });
                if on && convertible {
                    chosen.push(json!({"path": [path]}));
                }
            }
            ui.add_space(4.0);
            if ui.add_enabled(!chosen.is_empty(), egui::Button::new(tl!("Number Ticked Headings"))).clicked() {
                action = Some(Action::Convert(chosen));
            }
        }
    });
    match action {
        Some(Action::GoTo(k)) => {
            let pos = wordcraft_doc::Pos::body(usize::try_from(k).unwrap_or(usize::MAX), 0);
            let _ = app.run("caret.set", json!({"pos": serde_json::to_value(pos).unwrap_or(Value::Null)}));
            app.canvas.want_focus = true;
        }
        Some(Action::Fix(item)) => {
            let _ = app.run("numbering.repair", json!({"items": [item]}));
        }
        Some(Action::FixAll) => {
            let _ = app.run("numbering.repair", json!({}));
        }
        Some(Action::Convert(items)) => {
            let _ = app.run("numbering.convert", json!({"items": items}));
        }
        None => {}
    }
}

/// The check's and the detector's answers, kept until the document changes.
fn findings(app: &WordApp, ui: &Ui) -> (Value, Value) {
    let key = egui::Id::unique("numbering_findings");
    let stamp = (app.session.document_id(), app.session.rev());
    if let Some((s, v)) = ui.data(|d| d.get_temp::<((u64, u64), (Value, Value))>(key))
        && s == stamp
    {
        return v;
    }
    let v = (wordcraft_engine::cmd::numbering::report(&app.session), wordcraft_engine::cmd::numbering::typed(&app.session));
    ui.data_mut(|d| d.insert_temp(key, (stamp, v.clone())));
    v
}
