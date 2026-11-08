//! The Project tab (thesis mode, WordCraft's own): make or open a manuscript project, add
//! sections and folders, show the Binder and take snapshots. See `project_ui`.

use super::{Ui, WordApp, big, group, json};

pub(super) fn project(app: &mut WordApp, ui: &mut Ui) {
    group(ui, "Project", None, app, |ui, app| {
        big(ui, app, "coverPage", "New\nProject", "ui.project.new", json!({}), false);
        big(ui, app, "open", "Open\nProject", "ui.project.open", json!({}), false);
        big(ui, app, "close", "Close\nProject", "ui.project.close", json!({}), false);
    });
    group(ui, "Binder", None, app, |ui, app| {
        big(ui, app, "blankPage", "New\nSection", "ui.project.add", json!({"kind": "section"}), false);
        big(ui, app, "folder", "New\nFolder", "ui.project.add", json!({"kind": "folder"}), false);
        let shown = app.project.as_ref().is_some_and(|p| p.binder);
        crate::widgets::big_toggle(ui, app, "navPane", "Binder", "ui.project.binder", shown);
    });
    group(ui, "Versions", None, app, |ui, app| {
        big(ui, app, "screenshot", "Take\nSnapshot", "ui.project.snapshot", json!({}), false);
    });
}
