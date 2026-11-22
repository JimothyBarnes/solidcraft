//! Contour's form (decision 0013 of the CAM work): round the outside of the part. It builds the
//! parameters of `cam.op.contour` (of `cam.op.edit` for a contour being changed) and shows what
//! they would cut before OK, as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{OperationPreview, Placement, ToolChoice, manufacture_preview, manufacture_setup_preview, manufacture_tools};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{
    Edited, SetupChoice, Worked, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
    word,
};
use crate::SolveApp;
use crate::ui_kit;

/// Which way round the cut goes: the command's word and the label.
pub(super) const DIRECTIONS: [(&str, &str); 2] = [("climb", "manufacture.direction_climb"), ("conventional", "manufacture.direction_conventional")];
/// The stepdown a contour takes when none is typed, as a share of the tool's diameter (the
/// engine's own default, shown in the field).
const STEPDOWN_SHARE: f64 = 0.5;

/// Contour: round the outside of the part.
pub(crate) struct ContourForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    pub tool: usize,
    pub direction: usize,
    /// The Z the cut goes down to in the setup's WCS, mm: the part's bottom until it is typed.
    pub bottom: f64,
    pub typed_bottom: bool,
    /// mm: half the tool's diameter until it is typed.
    pub stepdown: f64,
    pub typed_stepdown: bool,
    /// Stock left on the wall, mm.
    pub leave: f64,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// Where the setup sits on the model: the part's bottom is read from it.
    placed: Worked<Placement>,
    /// The contour these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The contour being changed, when the form changes one: its `contour` parameters as kept.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl ContourForm {
    /// For the setup the viewport draws and the first tool that can cut a wall.
    pub fn new(app: &SolveApp) -> ContourForm {
        let (setups, setup) = setups(app);
        let tools = manufacture_tools(&app.session.doc, "contour");
        let stepdown = tools.first().map_or(0.0, |t| STEPDOWN_SHARE * t.diameter);
        ContourForm {
            setups,
            tools,
            setup,
            tool: 0,
            direction: 0,
            bottom: 0.0,
            typed_bottom: false,
            stepdown,
            typed_stepdown: false,
            leave: 0.0,
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            placed: Worked::new(),
            worked: Worked::new(),
            drawn_setup: None,
            editing: None,
            error: None,
        }
    }

    /// The form for changing the contour `id`, in the setup it is in; why not, when the design
    /// has no such contour.
    pub fn edit(app: &SolveApp, id: &str) -> Result<ContourForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "contour" {
            return Err(format!("operation {id} is not a contour"));
        }
        let tools = manufacture_tools(&app.session.doc, "contour");
        let number = |key: &str| base[key].as_f64();
        let mut form = ContourForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        form.tool = tools.iter().position(|t| t.number == tool).unwrap_or(0);
        form.tools = tools;
        form.direction = DIRECTIONS.iter().position(|d| Some(d.0) == base["direction"].as_str()).unwrap_or(0);
        (form.bottom, form.typed_bottom) = number("bottom").map_or((0.0, false), |v| (v, true));
        (form.stepdown, form.typed_stepdown) = number("stepdown").map_or((form.stepdown, false), |v| (v, true));
        form.leave = number("leave").unwrap_or(0.0);
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// What is missing before a contour can be made, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_contour"))
        } else {
            None
        }
    }

    /// The `cam.op.contour` parameters. What was not typed is left to the command, so it keeps
    /// following the model and the tool: the bottom the part's, the stepdown the tool's diameter,
    /// the speed and feed its cutting data. A contour being changed starts from its parameters as
    /// kept, so what the form does not show (its top, its clearance) stays.
    pub fn params(&self) -> Value {
        let mut contour = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        contour["direction"] = json!(word(&DIRECTIONS, self.direction));
        set_or_clear(&mut contour, "bottom", self.typed_bottom.then_some(self.bottom));
        set_or_clear(&mut contour, "stepdown", self.typed_stepdown.then_some(self.stepdown));
        set_or_clear(&mut contour, "leave", (self.leave > 0.0).then_some(self.leave));
        set_or_clear(&mut contour, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut contour, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "contour": contour,
        })
    }

    /// Works the contour out, if its parameters or the design changed, and shows the numbers that
    /// were not typed as it gives them.
    fn refresh(&mut self, app: &SolveApp) {
        // The part's bottom, read where the setup sits: known even when the contour can't be
        // worked out, which is when it most needs changing.
        let setup = json!({"id": self.setups.get(self.setup).map_or("", |s| s.id.as_str())});
        self.placed.refresh(app, &setup, || manufacture_setup_preview(&app.session, "cam.setup.edit", &setup));
        if !self.typed_bottom
            && let Ok(placement) = &self.placed.result
        {
            self.bottom = placement.model.min.z;
        }
        if !self.typed_stepdown {
            self.stepdown = self.tools.get(self.tool).map_or(self.stepdown, |t| STEPDOWN_SHARE * t.diameter);
        }
        self.leave = self.leave.max(0.0);
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.contour", &params)) {
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

    /// The levels the contour would cut, as the form says them: how many, and how deep each.
    fn levels(&self, units: &str, decimals: u8) -> Option<String> {
        let (_, preview) = self.worked.get()?;
        let levels = preview.numbers.get("levels")?.as_array()?;
        let (top, bottom) = (preview.numbers.get("top")?.as_f64()?, levels.last()?.as_f64()?);
        let each = format_length((top - bottom) / levels.len().max(1) as f64, units, decimals);
        Some(solidcraft_ui_text::format("manufacture.contour_levels_value", &[("count", &levels.len().to_string()), ("depth", &each)]))
    }

    /// Runs the command: the form again, with the reason, when the contour can't be worked out or
    /// the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ContourForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "contour": params["contour"].take()})),
            None => app.run("cam.op.contour", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<ContourForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let directions = labels(&DIRECTIONS);
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        let levels = self.levels(units, decimals);
        // The setup picked is the one the viewport draws, so the contour is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_contour");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_contour"), "edit", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_contour_missing"));
                    return;
                }
                // An operation stays in its setup: the row is a choice only for a new one.
                match (&self.editing, setup_labels.first()) {
                    (Some(_), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                    }
                    _ => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_contour_setup", &mut self.setup, &setup_labels);
                    }
                }
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_contour_tool", &mut self.tool, &tool_labels);
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_contour_direction", &mut self.direction, &directions);
                if ui_kit::row(ui, tr("manufacture.contour_bottom"), |ui| ui_kit::length_field(ui, &mut self.bottom, units, decimals)).changed() {
                    self.typed_bottom = true;
                }
                if ui_kit::row(ui, tr("manufacture.face_stepdown"), |ui| ui_kit::length_field(ui, &mut self.stepdown, units, decimals)).changed() {
                    self.typed_stepdown = true;
                }
                ui_kit::row(ui, tr("manufacture.contour_leave"), |ui| ui_kit::length_field(ui, &mut self.leave, units, decimals));
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(levels) = &levels {
                    ui_kit::value_row(ui, tr("manufacture.contour_levels"), levels);
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_contour_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.contour_note"), ui_kit::Text::Note("manufacture_contour_note"));
                ui_kit::text(ui, tr("manufacture.contour_loose"), ui_kit::Text::Note("manufacture_contour_loose"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_contour_preview_note"));
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
