//! Engrave's form (decision 0032 of the CAM work): the tool's centre along the drawn curves of
//! a sketch, a little below the face they are drawn on. It builds the parameters of
//! `cam.op.engrave` (of `cam.op.edit` for an engraving being changed) and shows what they would
//! cut before OK, as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{FoundSketch, OperationPreview, ToolChoice, manufacture_preview, manufacture_sketches, manufacture_tools};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{
    Edited, SetupChoice, Worked, kept_operation, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
};
use crate::SolveApp;
use crate::ui_kit;

/// How deep an engraving goes when no depth is typed, mm (the engine's own default, shown in
/// the field).
const DEPTH: f64 = 0.2;

/// Engrave: a sketch's curves, followed.
pub(crate) struct EngraveForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    pub tool: usize,
    /// The sketches the setup can engrave: those that face the tool and have drawn curves.
    sketches: Worked<Vec<FoundSketch>>,
    /// Which of them, and its name: the name is what the engraving keeps, so a sketch that is
    /// no longer offered (one being changed that was renamed or turned away) is still said.
    pub sketch: usize,
    pub sketch_name: String,
    /// The choice as the name last left it: another one since is the row's own.
    left_at: usize,
    /// How deep the tool's tip goes, mm: 0.2 until it is typed.
    pub depth: f64,
    pub typed_depth: bool,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The engraving these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The engraving being changed, when the form changes one: its `engrave` parameters as kept.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
    /// Its heights (decision 0037 of the CAM work).
    pub heights: super::heights::HeightsForm,
}

impl EngraveForm {
    /// Takes the tool numbered `number`, just made with Add Tool opened from this form: the
    /// tools that can cut it again, with that one picked.
    pub(super) fn take_tool(&mut self, app: &SolveApp, number: u32) {
        self.tools = manufacture_tools(&app.session.doc, "engrave");
        if let Some(at) = self.tools.iter().position(|t| t.number == number) {
            self.tool = at;
        }
    }

    /// For the setup the viewport draws, the first sketch it can engrave and the first tool that
    /// can engrave.
    pub fn new(app: &SolveApp) -> EngraveForm {
        let (setups, setup) = setups(app);
        EngraveForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "engrave"),
            setup,
            tool: 0,
            sketches: Worked::new(),
            sketch: 0,
            sketch_name: String::new(),
            left_at: 0,
            depth: DEPTH,
            typed_depth: false,
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            worked: Worked::new(),
            drawn_setup: None,
            editing: None,
            heights: super::heights::HeightsForm::new([true, true, true, false]),
            error: None,
        }
    }

    /// The form for changing the engraving `id`, in the setup it is in; why not, when the design
    /// has no such engraving.
    pub fn edit(app: &SolveApp, id: &str) -> Result<EngraveForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "engrave" {
            return Err(format!("operation {id} is not an engraving"));
        }
        let number = |key: &str| base[key].as_f64();
        let mut form = EngraveForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        form.tool = form.tools.iter().position(|t| t.number == tool).unwrap_or(0);
        form.sketch_name = base["sketch"].as_str().unwrap_or_default().to_string();
        (form.depth, form.typed_depth) = number("depth").map_or((DEPTH, false), |v| (v, true));
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.heights = super::heights::HeightsForm::kept(app, id, [true, true, true, false]);
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// The sketches offered, when they have been found.
    pub fn sketches(&self) -> &[FoundSketch] {
        self.sketches.get().map_or(&[], |(_, found)| found.as_slice())
    }

    /// What is missing before an engraving can be made, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_engrave"))
        } else if self.sketch_name.is_empty() {
            Some(tr("manufacture.engrave_none"))
        } else {
            None
        }
    }

    /// The `cam.op.engrave` parameters. What was not typed is left to the command: the depth
    /// its own, the speed and feed the tool's cutting data. An engraving being changed starts
    /// from its parameters as kept, so what the form does not show (a surface of its own) stays.
    pub fn params(&self) -> Value {
        let mut engrave = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        engrave["sketch"] = json!(self.sketch_name);
        set_or_clear(&mut engrave, "depth", self.typed_depth.then_some(self.depth));
        set_or_clear(&mut engrave, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut engrave, "feed", self.typed_feed.then_some(self.feed));
        super::with_heights(
            &self.heights,
            json!({
                "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
                "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
                "engrave": engrave,
            }),
        )
    }

    /// Finds the setup's sketches and works the engraving out, if its parameters or the design
    /// changed, and shows the numbers that were not typed as it gives them.
    fn refresh(&mut self, app: &SolveApp) {
        self.depth = self.depth.max(0.0);
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        self.sketches.refresh(app, &json!(setup), || manufacture_sketches(&app.session, &setup));
        // The row's own choice names the sketch; else the name says which row it is, and with
        // no name yet the first sketch is the one.
        let found: Vec<String> = self.sketches().iter().map(|sketch| sketch.name.clone()).collect();
        if self.sketch != self.left_at {
            if let Some(name) = found.get(self.sketch) {
                self.sketch_name = name.clone();
            }
        } else if self.sketch_name.is_empty()
            && let Some(first) = found.first()
        {
            self.sketch_name = first.clone();
        }
        self.sketch = found.iter().position(|name| *name == self.sketch_name).unwrap_or(0);
        self.left_at = self.sketch;
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.engrave", &params)) {
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

    /// Runs the command: the form again, with the reason, when the engraving can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<EngraveForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "heights": params.get("heights").cloned().unwrap_or_else(|| json!({})), "engrave": params["engrave"].take()})),
            None => app.run("cam.op.engrave", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<EngraveForm> {
        // A point clicked on the model for one of its heights is that height's, not a pick of the form's own.
        self.heights.take_point(app, ctx);
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        // The sketches found; and one being changed that is not among them, by its name alone.
        let mut sketch_labels: Vec<String> = self.sketches().iter().map(|sketch| sketch.name.clone()).collect();
        if !self.sketch_name.is_empty() && !sketch_labels.contains(&self.sketch_name) {
            sketch_labels.insert(0, self.sketch_name.clone());
        }
        let sketch_labels: Vec<&str> = sketch_labels.iter().map(String::as_str).collect();
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let number = |key: &str| self.worked.get().and_then(|(_, preview)| preview.numbers.get(key)?.as_f64());
        let length = number("length").map(|v| format_length(v, units, decimals));
        let width = number("width").map(|v| format_length(v, units, decimals));
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // The setup picked is the one the viewport draws, so the engraving is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_engrave");
        let mut new_tool = false;
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_engrave"), "cam_engrave", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_engrave_missing"));
                    // With no tool to cut with, one can be made here and the form comes back with it.
                    if self.tools.is_empty() {
                        new_tool = ui_kit::icon_label_button(ui, tr("manufacture.new_tool"), "cam_tool_new", true, false).clicked();
                    }
                    return;
                }
                // An operation stays in its setup: the row is a choice only for a new one.
                match (&self.editing, setup_labels.first()) {
                    (Some(_), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                    }
                    _ => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_engrave_setup", &mut self.setup, &setup_labels);
                    }
                }
                ui_kit::choice_row(ui, tr("manufacture.engrave_sketch"), "manufacture_engrave_sketch", &mut self.sketch, &sketch_labels);
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_engrave_tool", &mut self.tool, &tool_labels);
                new_tool = ui_kit::icon_label_button(ui, tr("manufacture.new_tool"), "cam_tool_new", true, false).clicked();
                if ui_kit::row(ui, tr("manufacture.engrave_depth"), |ui| ui_kit::length_field(ui, &mut self.depth, units, decimals)).changed() {
                    self.typed_depth = true;
                }
                self.heights.show(ui, units, decimals);
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(length) = &length {
                    ui_kit::value_row(ui, tr("manufacture.engrave_length"), length);
                }
                if let Some(width) = &width {
                    ui_kit::value_row(ui, tr("manufacture.engrave_width"), width);
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_engrave_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.engrave_note"), ui_kit::Text::Note("manufacture_engrave_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_engrave_preview_note"));
                }
                refusal(ui, self.error.as_deref().or(self.worked.trouble()));
            });
        if new_tool {
            let (kind, setup) = ("chamfer_mill", self.setups.get(self.setup).map(|s| s.id.clone()));
            super::new_tool_for(app, super::Dialog::Engrave(self), kind, setup.as_deref());
            return None;
        }
        match action {
            ui_kit::Action::Ok => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}
