//! The Heights section of an operation's form (decision 0037 of the CAM work): clearance,
//! retract, top and bottom, each from a reference with an offset, as `cam.op.*`'s `heights`
//! takes them. Folded until opened; a form whose heights are left alone gives none, so the
//! operation's own are used, as before.
use serde_json::{Value, json};
use solidcraft_engine::Sel;
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{labels, word};
use crate::SolveApp;
use crate::manufacture_workspace::picking;
use crate::ui_kit;

/// What clearance and retract are measured from: the setup's own first.
const FROM_TRAVEL: [(&str, &str); 5] = [
    ("setup", "manufacture.height_from_setup"),
    ("stock_top", "manufacture.height_from_stock_top"),
    ("model_top", "manufacture.height_from_model_top"),
    ("point", "manufacture.height_from_point"),
    ("z", "manufacture.height_from_z"),
];
/// What a top is measured from: the operation's own first.
const FROM_TOP: [(&str, &str); 6] = [
    ("feature", "manufacture.height_from_feature"),
    ("stock_top", "manufacture.height_from_stock_top"),
    ("model_top", "manufacture.height_from_model_top"),
    ("model_bottom", "manufacture.height_from_model_bottom"),
    ("point", "manufacture.height_from_point"),
    ("z", "manufacture.height_from_z"),
];
/// What a bottom is measured from: the operation's own first.
const FROM_BOTTOM: [(&str, &str); 7] = [
    ("feature", "manufacture.height_from_feature"),
    ("model_bottom", "manufacture.height_from_model_bottom"),
    ("stock_bottom", "manufacture.height_from_stock_bottom"),
    ("model_top", "manufacture.height_from_model_top"),
    ("stock_top", "manufacture.height_from_stock_top"),
    ("point", "manufacture.height_from_point"),
    ("z", "manufacture.height_from_z"),
];
/// The heights, in the order the section shows them, with their keys and labels.
const HEIGHTS: [(&str, &str); 4] = [
    ("clearance", "manufacture.height_clearance"),
    ("retract", "manufacture.height_retract"),
    ("top", "manufacture.height_top"),
    ("bottom", "manufacture.height_bottom"),
];

fn froms(height: usize) -> &'static [(&'static str, &'static str)] {
    match height {
        0 | 1 => &FROM_TRAVEL,
        2 => &FROM_TOP,
        _ => &FROM_BOTTOM,
    }
}

/// One height as the section has it.
#[derive(Clone, Debug, Default, PartialEq)]
struct Row {
    /// Which of its references.
    from: usize,
    /// mm, up positive.
    offset: f64,
    /// The point picked on the model, in model coordinates, for a height from one.
    at: Option<[f64; 3]>,
}

/// An operation's heights in its form.
#[derive(Clone, Debug)]
pub(crate) struct HeightsForm {
    rows: [Row; 4],
    /// Which of the heights the operation takes.
    shown: [bool; 4],
    /// The height waiting for a point to be clicked on the model, and what was picked for the
    /// form when it began (put back after the click, so the form's own picks stay as they are),
    /// read at the next frame.
    picking: Option<(usize, Option<Vec<Sel>>)>,
}

impl HeightsForm {
    /// The operation's own heights: `shown` says which of clearance, retract, top and bottom it
    /// takes.
    pub(crate) fn new(shown: [bool; 4]) -> HeightsForm {
        HeightsForm { rows: Default::default(), shown, picking: None }
    }

    /// The heights of the operation `id` as the design keeps them.
    pub(crate) fn kept(app: &SolveApp, id: &str, shown: [bool; 4]) -> HeightsForm {
        let mut form = HeightsForm::new(shown);
        let kept = solidcraft_engine::cmd::manufacture_data(&app.session.doc)
            .ok()
            .and_then(|data| data.setups.iter().flat_map(|s| s.operations.iter()).find(|o| o.id == id).and_then(|o| o.heights))
            .and_then(|heights| serde_json::to_value(heights).ok())
            .unwrap_or(Value::Null);
        for (k, (key, _)) in HEIGHTS.iter().enumerate() {
            let Some(height) = kept.get(*key) else { continue };
            let from = height["from"].as_str().unwrap_or("feature");
            // The setup's and the feature's are one choice, each in its own list.
            let at = froms(k).iter().position(|f| f.0 == from || (k < 2 && from == "feature") || (k >= 2 && from == "setup")).unwrap_or(0);
            let point = height["at"].as_array().and_then(|v| Some([v.first()?.as_f64()?, v.get(1)?.as_f64()?, v.get(2)?.as_f64()?]));
            if let Some(row) = form.rows.get_mut(k) {
                *row = Row { from: at, offset: height["offset"].as_f64().unwrap_or(0.0), at: point };
            }
        }
        form
    }

    /// The `heights` parameter: `None` when every height shown is the operation's own.
    pub(crate) fn params(&self) -> Option<Value> {
        let mut heights = serde_json::Map::new();
        for (k, (key, _)) in HEIGHTS.iter().enumerate() {
            let (Some(row), true) = (self.rows.get(k), self.shown.get(k).copied().unwrap_or(false)) else { continue };
            if row.from == 0 && row.offset == 0.0 {
                continue;
            }
            let from = word(froms(k), row.from);
            let mut height = json!({"from": from, "offset": row.offset});
            if from == "point" {
                match row.at {
                    Some(at) => height["at"] = json!(at),
                    // Not picked yet: no height of its own until it is.
                    None => continue,
                }
            }
            heights.insert((*key).into(), height);
        }
        (!heights.is_empty()).then_some(Value::Object(heights))
    }

    /// Whether a height is waiting for a point on the model: the form's own picking waits.
    pub(crate) fn picking(&self) -> bool {
        self.picking.is_some()
    }

    /// Takes the point clicked on the model for the height waiting for one, if one was clicked
    /// since the last frame, and puts back what was picked for the form. Whether it took one.
    pub(crate) fn take_point(&mut self, app: &mut SolveApp, ctx: &egui::Context) -> bool {
        let Some((height, before)) = self.picking.take() else { return false };
        let before = before.unwrap_or_else(|| picking::picked_now(app));
        match picking::clicked_point(app, ctx, &before) {
            Some(point) => {
                if let Some(row) = self.rows.get_mut(height) {
                    // To the micron: a click is no more exact than that.
                    let micron = |v: f64| (v * 1e6).round() / 1e6;
                    row.at = Some([micron(point.x), micron(point.y), micron(point.z)]);
                }
                true
            }
            None => {
                self.picking = Some((height, Some(before)));
                false
            }
        }
    }

    /// The section: a row for where each height the operation takes is measured from, its
    /// offset, and for a point, the point and a button to pick it.
    pub(crate) fn show(&mut self, ui: &mut egui::Ui, units: &str, decimals: u8) {
        let changed = self.rows.iter().zip(self.shown).any(|(row, shown)| shown && *row != Row::default());
        ui_kit::editor_section(ui, "manufacture_heights", tr("manufacture.heights"), changed, |ui| {
            for (k, (key, label)) in HEIGHTS.iter().enumerate() {
                if !self.shown.get(k).copied().unwrap_or(false) {
                    continue;
                }
                let names = labels(froms(k));
                let Some(row) = self.rows.get_mut(k) else { continue };
                ui_kit::choice_row(ui, tr(label), &format!("manufacture_height_{key}"), &mut row.from, &names);
                ui_kit::row(ui, tr("manufacture.height_offset"), |ui| ui_kit::length_field(ui, &mut row.offset, units, decimals));
                if word(froms(k), row.from) != "point" {
                    continue;
                }
                match (self.picking.as_ref().is_some_and(|(at, _)| *at == k), row.at) {
                    (true, _) => {
                        ui_kit::text(ui, tr("manufacture.height_picking"), ui_kit::Text::Note("manufacture_height_picking"));
                    }
                    (false, at) => {
                        if let Some(at) = at {
                            let place = at.iter().map(|v| format_length(*v, units, decimals)).collect::<Vec<_>>().join(", ");
                            ui_kit::value_row(ui, tr("manufacture.height_point"), &place);
                        }
                        if ui_kit::icon_label_button(ui, tr("manufacture.height_pick"), "point", true, false).clicked() {
                            self.picking = Some((k, None));
                        }
                    }
                }
            }
        });
    }
}

#[cfg(test)]
impl HeightsForm {
    /// Sets the height `key` to be from `from` with `offset`, as its rows would.
    pub(crate) fn set(&mut self, key: &str, from: &str, offset: f64) {
        let Some(k) = HEIGHTS.iter().position(|h| h.0 == key) else { return };
        if let (Some(row), Some(at)) = (self.rows.get_mut(k), froms(k).iter().position(|f| f.0 == from)) {
            (row.from, row.offset) = (at, offset);
        }
    }

    /// Presses the height `key`'s button to pick its point on the model.
    pub(crate) fn pick(&mut self, key: &str) {
        self.set(key, "point", 0.0);
        self.picking = HEIGHTS.iter().position(|h| h.0 == key).map(|k| (k, None));
    }
}
