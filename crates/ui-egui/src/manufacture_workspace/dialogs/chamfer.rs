//! Chamfer's form (decision 0018 of the CAM work): the top outside edge of the part broken with
//! a chamfer mill. It builds the parameters of `cam.op.chamfer` (of `cam.op.edit` for a chamfer
//! being changed) and shows what they would cut before OK, as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{OperationPreview, ToolChoice, manufacture_preview, manufacture_tools};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::contour::DIRECTIONS;
use super::{
    Edited, SetupChoice, Worked, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
    word,
};
use crate::SolveApp;
use crate::ui_kit;

/// A chamfer's width and how far the tool's tip is below it when neither is typed, mm (the
/// engine's own defaults, shown in the fields).
const WIDTH: f64 = 0.5;
const TIP_BELOW: f64 = 1.0;

/// Which edges are broken: the command's word and the label.
const EDGES: [(&str, &str); 2] = [("outside", "manufacture.chamfer_edges_outside"), ("all", "manufacture.chamfer_edges_all")];

/// Chamfer: the top outside edge of the part, or all its top edges.
pub(crate) struct ChamferForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    pub tool: usize,
    /// Which of [`EDGES`]: the outside edge alone, or the rims of the openings in the top too.
    pub edges: usize,
    pub direction: usize,
    /// How wide the chamfer is seen from above, mm: half a millimetre until it is typed.
    pub width: f64,
    pub typed_width: bool,
    /// How far the tool's tip is below the chamfer, mm: 1 until it is typed.
    pub tip_below: f64,
    pub typed_tip_below: bool,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The chamfer these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The chamfer being changed, when the form changes one: its `chamfer` parameters as kept.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl ChamferForm {
    /// For the setup the viewport draws and the first tool that can cut a chamfer.
    pub fn new(app: &SolveApp) -> ChamferForm {
        let (setups, setup) = setups(app);
        ChamferForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "chamfer"),
            setup,
            tool: 0,
            edges: 0,
            direction: 0,
            width: WIDTH,
            typed_width: false,
            tip_below: TIP_BELOW,
            typed_tip_below: false,
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            worked: Worked::new(),
            drawn_setup: None,
            editing: None,
            error: None,
        }
    }

    /// The form for changing the chamfer `id`, in the setup it is in; why not, when the design
    /// has no such chamfer.
    pub fn edit(app: &SolveApp, id: &str) -> Result<ChamferForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "chamfer" {
            return Err(format!("operation {id} is not a chamfer"));
        }
        let number = |key: &str| base[key].as_f64();
        let mut form = ChamferForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        form.tool = form.tools.iter().position(|t| t.number == tool).unwrap_or(0);
        form.edges = EDGES.iter().position(|e| Some(e.0) == base["edges"].as_str()).unwrap_or(0);
        form.direction = DIRECTIONS.iter().position(|d| Some(d.0) == base["direction"].as_str()).unwrap_or(0);
        (form.width, form.typed_width) = number("width").map_or((WIDTH, false), |v| (v, true));
        (form.tip_below, form.typed_tip_below) = number("tip_below").map_or((TIP_BELOW, false), |v| (v, true));
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// What is missing before a chamfer can be made, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_chamfer"))
        } else {
            None
        }
    }

    /// The `cam.op.chamfer` parameters. What was not typed is left to the command: the width
    /// and the tip's depth its own, the speed and feed the tool's cutting data. A chamfer being
    /// changed starts from its parameters as kept, so what the form does not show (the height of
    /// its edge, its arcs) stays.
    pub fn params(&self) -> Value {
        let mut chamfer = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        // The outside edge alone is what a chamfer is unless it says otherwise: only the other
        // choice is written.
        if word(&EDGES, self.edges) == "all" {
            chamfer["edges"] = json!("all");
        } else if let Some(kept) = chamfer.as_object_mut() {
            kept.remove("edges");
        }
        chamfer["direction"] = json!(word(&DIRECTIONS, self.direction));
        set_or_clear(&mut chamfer, "width", self.typed_width.then_some(self.width));
        set_or_clear(&mut chamfer, "tip_below", self.typed_tip_below.then_some(self.tip_below));
        set_or_clear(&mut chamfer, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut chamfer, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "chamfer": chamfer,
        })
    }

    /// Works the chamfer out, if its parameters or the design changed, and shows the numbers that
    /// were not typed as it gives them.
    fn refresh(&mut self, app: &SolveApp) {
        (self.width, self.tip_below) = (self.width.max(0.0), self.tip_below.max(0.0));
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.chamfer", &params)) {
            return;
        }
        self.error = None;
        if let Ok(preview) = &self.worked.result {
            if !self.typed_rpm {
                self.rpm = preview.rpm.round();
            }
            if !self.typed_feed {
                self.feed = preview.feed;
            }
        }
    }

    /// Runs the command: the form again, with the reason, when the chamfer can't be worked out or
    /// the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ChamferForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "chamfer": params["chamfer"].take()})),
            None => app.run("cam.op.chamfer", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<ChamferForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let directions = labels(&DIRECTIONS);
        let edges = labels(&EDGES);
        let all = word(&EDGES, self.edges) == "all";
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // Where the tool's tip runs: the number to check against what is beside the part.
        let tip = self.worked.get().and_then(|(_, preview)| preview.numbers.get("z")?.as_f64()).map(|z| format_length(z, units, decimals));
        // The setup picked is the one the viewport draws, so the chamfer is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_chamfer");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_chamfer"), "cam_chamfer", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_chamfer_missing"));
                    return;
                }
                // An operation stays in its setup: the row is a choice only for a new one.
                match (&self.editing, setup_labels.first()) {
                    (Some(_), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                    }
                    _ => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_chamfer_setup", &mut self.setup, &setup_labels);
                    }
                }
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_chamfer_tool", &mut self.tool, &tool_labels);
                ui_kit::choice_row(ui, tr("manufacture.chamfer_edges"), "manufacture_chamfer_edges", &mut self.edges, &edges);
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_chamfer_direction", &mut self.direction, &directions);
                if ui_kit::row(ui, tr("manufacture.chamfer_width"), |ui| ui_kit::length_field(ui, &mut self.width, units, decimals)).changed() {
                    self.typed_width = true;
                }
                if ui_kit::row(ui, tr("manufacture.chamfer_tip_below"), |ui| ui_kit::length_field(ui, &mut self.tip_below, units, decimals)).changed()
                {
                    self.typed_tip_below = true;
                }
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(tip) = &tip {
                    ui_kit::value_row(ui, tr("manufacture.chamfer_tip_z"), tip);
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_chamfer_engine_note"));
                }
                let note = if all { "manufacture.chamfer_all_note" } else { "manufacture.chamfer_note" };
                ui_kit::text(ui, tr(note), ui_kit::Text::Note("manufacture_chamfer_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_chamfer_preview_note"));
                }
                refusal(ui, self.error.as_deref().or(self.worked.trouble()));
            });
        match action {
            ui_kit::Action::Ok => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}
