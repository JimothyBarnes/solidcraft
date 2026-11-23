//! The tool library window (UX.md section 5): the design's tools in a table that can be searched
//! and sorted, the tool picked drawn in outline, and the buttons that add, change, remove, bring
//! in and send out tools. The window only reads: every change is a `cam.tool.*` or
//! `cam.library.*` command, through the forms and prompts those have already, and the window
//! comes back when a tool's form closes.
use serde_json::json;
use solidcraft_engine::cmd::{ToolRow, ToolShape, manufacture_data, manufacture_tool_rows};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_theme::Tokens;
use solidcraft_ui_text::{tr, tr_label};

use super::{Dialog, ImportForm, KINDS, ToolForm, export_library, refusal};
use crate::SolveApp;
use crate::ui_kit;

/// The columns whose headings sort the table: the number, the name and the diameter.
const NUMBER: usize = 0;
const NAME: usize = 1;
const DIAMETER: usize = 3;
/// The share of the window's width each column takes, beside the others.
const WIDTHS: [f32; 8] = [0.9, 2.4, 1.5, 1.0, 0.7, 1.1, 1.0, 1.0];
/// How wide the window is, in the widths of a form.
const CARDS: f32 = 3.0;
/// How tall the picked tool's outline is drawn, in rows of a form.
const OUTLINE_ROWS: f32 = 6.0;

/// What a button of the window asks for.
enum Ask {
    Add,
    Edit(u32),
    Delete(u32),
    Import,
    Export,
}

/// The tool library: the design's tools, searched and sorted.
pub(crate) struct LibraryForm {
    /// The words looked for: every one must be in a tool's number, name, kind or diameter.
    pub search: String,
    /// The column the rows are sorted by and which way; as listed is by number.
    pub sort: (usize, ui_kit::Sort),
    /// The tool picked, by its number.
    pub picked: Option<u32>,
    pub error: Option<String>,
}

impl LibraryForm {
    pub fn new() -> LibraryForm {
        LibraryForm { search: String::new(), sort: (NUMBER, ui_kit::Sort::Listed), picked: None, error: None }
    }

    /// The tools the window lists: those the search finds, in the order asked for.
    pub fn rows(&self, app: &SolveApp) -> Vec<ToolRow> {
        let mut rows = manufacture_data(&app.session.doc).map(|data| manufacture_tool_rows(&data, &self.search)).unwrap_or_default();
        let (column, sort) = self.sort;
        if sort != ui_kit::Sort::Listed {
            match column {
                NAME => rows.sort_by(|a, b| ui_kit::natural_cmp(&a.name, &b.name)),
                DIAMETER => rows.sort_by(|a, b| a.diameter.total_cmp(&b.diameter)),
                _ => {}
            }
            if sort == ui_kit::Sort::Descending {
                rows.reverse();
            }
        }
        rows
    }

    /// A kind as the window names it: Add Tool's own word for it, else the tool file's.
    fn kind_label(kind: &str) -> String {
        KINDS.iter().find(|known| known.0 == kind).map_or_else(|| kind.replace('_', " "), |known| tr(known.1).to_string())
    }

    /// Opens `form` in the window's place; the window comes back when the form closes.
    fn open(self, app: &mut SolveApp, form: Dialog) -> Option<LibraryForm> {
        app.manufacture_workspace.dialog = Some(form);
        app.manufacture_workspace.resume = Some(Box::new(Dialog::Library(self)));
        None
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<LibraryForm> {
        let t = Tokens::get();
        let data = manufacture_data(&app.session.doc);
        let of = data.as_ref().map_or(0, |data| data.tools.len());
        let rows = self.rows(app);
        // A tool stays picked while it is listed.
        if self.picked.is_some_and(|number| !rows.iter().any(|row| row.number == number)) {
            self.picked = None;
        }
        let picked = self.picked.and_then(|number| data.as_ref().ok()?.tools.iter().find(|tool| tool.number == number));
        let shape = picked.map(ToolShape::of);
        let picked_label = picked.map(|tool| format!("T{} {}", tool.number, tool.name));
        let labels = [
            tr("manufacture.tool_number"),
            tr_label("Name"),
            tr_label("Type"),
            tr("ui.diameter"),
            tr("manufacture.tool_flutes"),
            tr("manufacture.tool_flute_length"),
            tr("manufacture.library_stick_out"),
            tr("manufacture.library_used"),
        ];
        let columns: Vec<ui_kit::Column<'_>> = labels.iter().zip(WIDTHS).map(|(label, width)| ui_kit::Column::new(label, width).flex()).collect();
        let empty = if of == 0 { tr("manufacture.library_none") } else { tr("manufacture.library_none_found") };
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let mut ask = None;
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_library"), "library", tr("cmd.cam_tool_list"))
            .placed(pivot, at)
            .cards(CARDS)
            .fields()
            .actions("", tr("ui.close"), None)
            .show(ctx, true, |ui| {
                let search =
                    ui_kit::search_field(ui, egui::Id::unique("sc_manufacture_library_search"), &mut self.search, tr("manufacture.library_search"));
                crate::scenario::publish_handle("manufacture:library:search", search.rect.center());
                // Add, and for the tool picked Edit and Delete; then the library files.
                let on_picked = self.picked.unwrap_or(0);
                ui.horizontal(|ui| {
                    for (what, label, icon, handle, enabled, danger) in [
                        (Ask::Add, tr("cmd.cam_tool_create"), "library", "manufacture:library:add", true, false),
                        (Ask::Edit(on_picked), tr_label("Edit"), "edit", "manufacture:library:edit", self.picked.is_some(), false),
                        (Ask::Delete(on_picked), tr_label("Delete"), "delete", "manufacture:library:delete", self.picked.is_some(), true),
                        (Ask::Import, tr("cmd.cam_library_import"), "import_tray", "manufacture:library:import", true, false),
                        (Ask::Export, tr("cmd.cam_library_export"), "export_tray", "manufacture:library:export", true, false),
                    ] {
                        let button = ui_kit::icon_label_button(ui, label, icon, enabled, danger);
                        crate::scenario::publish_handle(handle, button.rect.center());
                        if button.clicked() {
                            ask = Some(what);
                        }
                    }
                });
                ui_kit::table_custom(ui, "manufacture_library", &columns, empty, |ui, table| {
                    table.headers(ui, |ui, index| {
                        if ![NUMBER, NAME, DIAMETER].contains(&index) {
                            return false;
                        }
                        let mut sort = if self.sort.0 == index { self.sort.1 } else { ui_kit::Sort::Listed };
                        let before = sort;
                        let heading = ui_kit::sort_header(ui, labels.get(index).copied().unwrap_or(""), &mut sort);
                        crate::scenario::publish_handle(&format!("manufacture:library:sort:{index}"), heading.rect.center());
                        if sort != before {
                            self.sort = (index, sort);
                        }
                        true
                    });
                    for row in &rows {
                        let cells = [
                            format!("T{}", row.number),
                            row.name.clone(),
                            Self::kind_label(&row.kind),
                            format_length(row.diameter, units, decimals),
                            row.flutes.to_string(),
                            format_length(row.flute_length, units, decimals),
                            format_length(row.stick_out, units, decimals),
                            row.operations.to_string(),
                        ];
                        // A click anywhere on the row picks the tool, and a double click opens its
                        // form: on a cell's text, which takes its own clicks, as between the cells.
                        let (mut clicked, mut twice) = (false, false);
                        let line = table.row(ui, &format!("tool:{}", row.number), self.picked == Some(row.number), |line| {
                            for (index, cell) in cells.iter().enumerate() {
                                let style = if index == NAME { ui_kit::Text::NameCell(false) } else { ui_kit::Text::Cell };
                                let text = line.cell(|ui| ui_kit::text(ui, cell, style));
                                (clicked, twice) = (clicked || text.clicked(), twice || text.double_clicked());
                            }
                        });
                        crate::scenario::publish_handle(&format!("manufacture:library:tool:{}", row.number), line.rect.center());
                        if twice || line.double_clicked() {
                            ask = Some(Ask::Edit(row.number));
                        } else if clicked || line.clicked() {
                            (self.picked, self.error) = (Some(row.number), None);
                        }
                    }
                });
                ui.add_space(t.space_6);
                match (&picked_label, &shape) {
                    (Some(label), Some(shape)) => {
                        ui_kit::text(ui, label, ui_kit::Text::Note("manufacture_library_picked"));
                        outline(ui, shape);
                    }
                    _ if of > 0 => {
                        ui_kit::text(ui, tr("manufacture.library_pick"), ui_kit::Text::Note("manufacture_library_pick"));
                    }
                    _ => {}
                }
                refusal(ui, self.error.as_deref());
            });
        let stays = matches!(action, ui_kit::Action::None) && !closed;
        match ask {
            Some(Ask::Add) => {
                let form = Dialog::Tool(ToolForm::new(app));
                self.open(app, form)
            }
            Some(Ask::Edit(number)) => match ToolForm::edit(app, number) {
                Ok(form) => self.open(app, Dialog::Tool(form)),
                Err(why) => {
                    self.error = Some(why);
                    Some(self)
                }
            },
            Some(Ask::Delete(number)) => {
                // A refusal (a tool an operation still uses) is said in the window.
                self.error = app.run("cam.tool.delete", json!({"number": number})).err();
                Some(self)
            }
            Some(Ask::Import) => self.open(app, Dialog::Import(ImportForm::new())),
            Some(Ask::Export) => {
                export_library(app);
                Some(self)
            }
            None => stays.then_some(self),
        }
    }
}

/// A tool's outline, as the simulation cuts and checks with it, lying on its side with its tip
/// at the left: what cuts, the shank and the holder, each in its colour, to one scale both ways.
fn outline(ui: &mut egui::Ui, shape: &ToolShape) {
    let t = Tokens::get();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), ui_kit::row_height() * OUTLINE_ROWS), egui::Sense::hover());
    // What cuts and the shank are solid; the holder is see-through, as the viewport draws it.
    let parts = [
        (&shape.cutting, t.manipulator, t.body_edge),
        (&shape.shank, t.body, t.body_edge),
        (&shape.holder, t.construction_plane, t.construction_geom),
    ];
    let (mut widest, mut longest) = (0.0_f64, 0.0_f64);
    for [out, above] in parts.iter().flat_map(|(part, ..)| part.iter().copied()) {
        (widest, longest) = (widest.max(out), longest.max(above));
    }
    if !(widest > 0.0 && longest > 0.0) {
        return;
    }
    let margin = f64::from(t.space_8);
    let scale = ((f64::from(rect.width()) - 2.0 * margin) / longest).min((0.5 * f64::from(rect.height()) - margin) / widest);
    if !(scale > 0.0 && scale.is_finite()) {
        return;
    }
    let at = |out: f64, above: f64| egui::pos2(rect.left() + (margin + above * scale) as f32, rect.center().y + (out * scale) as f32);
    let painter = ui.painter_at(rect);
    for (part, fill, edge) in parts {
        // One piece between two rings of the outline, on both sides of the axis.
        for (&[out0, above0], &[out1, above1]) in part.iter().zip(part.iter().skip(1)) {
            let corners = vec![at(-out0, above0), at(out0, above0), at(out1, above1), at(-out1, above1)];
            painter.add(egui::Shape::convex_polygon(corners, fill, egui::Stroke::NONE));
        }
        // Its edges: along each side, and across where it begins and ends.
        let stroke = egui::Stroke::new(1.0, edge);
        for side in [-1.0, 1.0] {
            painter.add(egui::Shape::line(part.iter().map(|&[out, above]| at(side * out, above)).collect(), stroke));
        }
        for &[out, above] in part.first().into_iter().chain(part.last()) {
            painter.line_segment([at(-out, above), at(out, above)], stroke);
        }
    }
}
