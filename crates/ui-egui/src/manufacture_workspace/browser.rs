//! The Manufacture browser, in the shared Browser: the design's setups with their operations, and
//! its tools, read from the design's Manufacture data. It only reads; every change is a `cam.*`
//! command. A row's menu has Edit (its form, also a double click) and Delete (its command).
use egui::Id;
use serde_json::{Value, json};
use solidcraft_theme::Tokens;

use super::dialogs::Target;
use crate::SolveApp;
use crate::ui_kit::{
    Item, Row, Text,
    browser::{Event, Node, NodeMenu, RowOut, Rows},
};

/// The menu entry that opens a row's form; the other entries are commands.
const EDIT: &str = "manufacture.edit";

/// What a row's menu or a double click asked for.
enum Ask {
    Edit(Target),
    Run(String, Value),
}

/// A row's menu: Edit, and Delete with the command and parameters that do it.
fn menu(delete: &str, params: Value) -> Vec<Item> {
    vec![Item::action(EDIT, "Edit", "edit"), Item::action(delete, "Delete", "delete").with(params)]
}

/// Notes what a row's events ask of the thing the row stands for.
fn asked(events: Vec<Event>, target: impl Fn() -> Target, asks: &mut Vec<Ask>) {
    for event in events {
        match event {
            Event::Open(_) => asks.push(Ask::Edit(target())),
            Event::Action { action, .. } if action == EDIT => asks.push(Ask::Edit(target())),
            Event::Action { action, params, .. } => asks.push(Ask::Run(action, params.unwrap_or(Value::Null))),
            _ => {}
        }
    }
}

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
    let mut asks = Vec::new();
    for setup in &data.setups {
        let key = format!("manufacture:browser:setup:{}", setup.id);
        let label = format!("{} (G{})", setup.name, 53 + u32::from(setup.work_offset));
        let is_active = active.as_deref() == Some(setup.id.as_str());
        let item = Row { icon: "ws_manufacture", label: &label, name: true, radio: Some(is_active), ..Row::default() };
        let entries = || menu("cam.setup.delete", json!({"id": setup.id}));
        let (open, events) = branch(app, rows, &key, item, true, NodeMenu::Items(&entries));
        if events.iter().any(|event| matches!(event, Event::Activate(_))) {
            app.manufacture_workspace.active_setup = Some(setup.id.clone());
        }
        asked(events, || Target::Setup(setup.id.clone()), &mut asks);
        if is_active && let Some(why) = super::scene::trouble(app) {
            crate::ui_kit::text(rows.ui(), &why, Text::Note("manufacture_setup_trouble"));
        }
        if open {
            for op in &setup.operations {
                let key = format!("manufacture:browser:op:{}", op.id);
                let label = format!("{} (T{})", op.name, op.tool);
                let entries = || menu("cam.op.delete", json!({"id": op.id}));
                let out = row(rows, &key, Row { depth: 1, icon: "edit", label: &label, name: true, ..Row::default() }, NodeMenu::Items(&entries));
                asked(out.events, || Target::Operation(op.id.clone()), &mut asks);
                // An operation whose toolpath can't be worked out says why, where it is listed.
                if is_active && let Some(why) = super::scene::operation_trouble(app, &op.id) {
                    crate::ui_kit::text(rows.ui(), &why, Text::Note("manufacture_operation_trouble"));
                }
            }
        }
    }
    if !data.tools.is_empty() {
        let label = app.tr("manufacture.browser_tools").to_string();
        let tools = Row { icon: "library", label: &label, name: true, ..Row::default() };
        if branch(app, rows, "manufacture:browser:tools", tools, false, NodeMenu::None).0 {
            for tool in &data.tools {
                let key = format!("manufacture:browser:tool:{}", tool.number);
                let label = format!("T{} {}", tool.number, tool.name);
                let entries = || menu("cam.tool.delete", json!({"number": tool.number}));
                let out = row(rows, &key, Row { depth: 1, icon: "library", label: &label, name: true, ..Row::default() }, NodeMenu::Items(&entries));
                asked(out.events, || Target::Tool(tool.number), &mut asks);
            }
        }
    }
    rows.ui().add_space(t.space_4);
    for ask in asks {
        match ask {
            Ask::Edit(target) => super::dialogs::edit(app, &target),
            // A refusal (a tool an operation still uses) goes to the status line.
            Ask::Run(command, params) => {
                let _ = app.run(&command, params);
            }
        }
    }
}

fn row(rows: &mut Rows<'_>, key: &str, row: Row<'_>, menu: NodeMenu<'_>) -> RowOut {
    rows.row(Node { id: crate::browser::node_id(key), key, row, menu, ..Node::default() })
}

fn fold_id(app: &SolveApp, rows: &mut Rows<'_>, key: &str) -> Id {
    rows.ui().make_persistent_id((app.session.uid, key))
}

/// A row that folds: whether it is open, and what else happened on it.
fn branch(app: &SolveApp, rows: &mut Rows<'_>, key: &str, mut item: Row<'_>, default_open: bool, menu: NodeMenu<'_>) -> (bool, Vec<Event>) {
    let id = fold_id(app, rows, key);
    let mut open = rows.ui().data(|data| data.get_temp::<bool>(id)).unwrap_or(default_open);
    item.fold = Some(open);
    let mut events = row(rows, key, item, menu).events;
    events.retain(|event| match event {
        Event::Fold { open: now, .. } => {
            rows.ui().data_mut(|data| data.insert_temp(id, *now));
            open = *now;
            false
        }
        _ => true,
    });
    (open, events)
}
