//! The Manufacture browser, in the shared Browser: the design's setups with their operations, and
//! its tools, read from the design's Manufacture data. It only reads; every change is a `cam.*`
//! command.
use egui::Id;
use solidcraft_theme::Tokens;

use crate::SolveApp;
use crate::ui_kit::{
    Row, Text,
    browser::{Event, Node, NodeMenu, RowOut, Rows},
};

pub(crate) fn show(app: &mut SolveApp, rows: &mut Rows<'_>) {
    let t = Tokens::get();
    rows.ui().add_space(t.space_4);
    let data = match solidcraft_engine::cmd::manufacture_data(&app.session.doc) {
        Ok(data) => data,
        Err(why) => {
            crate::ui_kit::text(rows.ui(), &why, Text::Note("manufacture_data"));
            return;
        }
    };
    if data.setups.is_empty() {
        // An empty browser says what to do first (UX.md section 4).
        crate::ui_kit::text(rows.ui(), app.tr("manufacture.browser_empty"), Text::Note("manufacture_setups"));
    }
    // The setup the viewport draws: the one whose radio is on, the first until one is picked.
    let active = data
        .setups
        .iter()
        .find(|s| app.manufacture_workspace.active_setup.as_deref() == Some(s.id.as_str()))
        .or(data.setups.first())
        .map(|s| s.id.clone());
    for setup in &data.setups {
        let key = format!("manufacture:browser:setup:{}", setup.id);
        let label = format!("{} (G{})", setup.name, 53 + u32::from(setup.work_offset));
        let is_active = active.as_deref() == Some(setup.id.as_str());
        let item = Row { icon: "ws_manufacture", label: &label, name: true, radio: Some(is_active), ..Row::default() };
        let (open, activated) = branch(app, rows, &key, item, true);
        if activated {
            app.manufacture_workspace.active_setup = Some(setup.id.clone());
        }
        if is_active && let Some(why) = super::scene::trouble(app) {
            crate::ui_kit::text(rows.ui(), &why, Text::Note("manufacture_setup_trouble"));
        }
        if open {
            for op in &setup.operations {
                let key = format!("manufacture:browser:op:{}", op.id);
                let label = format!("{} (T{})", op.name, op.tool);
                row(rows, &key, Row { depth: 1, icon: "edit", label: &label, name: true, ..Row::default() }, NodeMenu::None);
            }
        }
    }
    if !data.tools.is_empty() {
        let label = app.tr("manufacture.browser_tools").to_string();
        if branch(app, rows, "manufacture:browser:tools", Row { icon: "library", label: &label, name: true, ..Row::default() }, false).0 {
            for tool in &data.tools {
                let key = format!("manufacture:browser:tool:{}", tool.number);
                let label = format!("T{} {}", tool.number, tool.name);
                row(rows, &key, Row { depth: 1, icon: "library", label: &label, name: true, ..Row::default() }, NodeMenu::None);
            }
        }
    }
    rows.ui().add_space(t.space_4);
}

fn row(rows: &mut Rows<'_>, key: &str, row: Row<'_>, menu: NodeMenu<'_>) -> RowOut {
    rows.row(Node { id: crate::browser::node_id(key), key, row, menu, ..Node::default() })
}

fn fold_id(app: &SolveApp, rows: &mut Rows<'_>, key: &str) -> Id {
    rows.ui().make_persistent_id((app.session.uid, key))
}

/// A row that folds: whether it is open, and whether its radio was just picked.
fn branch(app: &SolveApp, rows: &mut Rows<'_>, key: &str, mut item: Row<'_>, default_open: bool) -> (bool, bool) {
    let id = fold_id(app, rows, key);
    let mut open = rows.ui().data(|data| data.get_temp::<bool>(id)).unwrap_or(default_open);
    item.fold = Some(open);
    let mut activated = false;
    for event in row(rows, key, item, NodeMenu::None).events {
        match event {
            Event::Fold { open: now, .. } => {
                rows.ui().data_mut(|data| data.insert_temp(id, now));
                open = now;
            }
            Event::Activate(_) => activated = true,
            _ => {}
        }
    }
    (open, activated)
}
