//! Bore's form (decision 0021 of the CAM work): round holes wider than the tool, cut with an end
//! mill down a helix along their wall. It offers the holes Drill offers, a group at a time,
//! builds the parameters of `cam.op.bore` (of `cam.op.edit` for a bore being changed: it is an
//! inside contour) and shows what they would cut before OK, as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{HoleBottom, HoleGroup, OperationPreview, ToolChoice, manufacture_holes, manufacture_preview, manufacture_tools};
use solidcraft_ui_text::tr;

use super::contour::DIRECTIONS;
use super::{
    Edited, SetupChoice, Worked, group_label, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed,
    window_words, word,
};
use crate::SolveApp;
use crate::ui_kit;

/// The widest tool picked for a hole, as a share of the hole's diameter: room for the helix.
const WIDEST_TOOL: f64 = 0.9;

/// Bore: round holes found on the model, a group at a time, each wider than the tool.
pub(crate) struct BoreForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// The holes the setup picked has, in groups, as the engine found them.
    found: Worked<Vec<HoleGroup>>,
    pub group: usize,
    /// The group the tool was last suited to.
    suited_to: Option<usize>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then it is the widest that goes into the holes
    /// with room for its helix.
    pub picked_tool: bool,
    pub direction: usize,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The boring these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The bore being changed, when the form changes one: its `contour` parameters as kept. Its
    /// holes stay the ones it has.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl BoreForm {
    /// For the setup the viewport draws, its largest group of holes and the tool that suits them.
    pub fn new(app: &SolveApp) -> BoreForm {
        let (setups, setup) = setups(app);
        BoreForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "bore"),
            setup,
            found: Worked::new(),
            group: 0,
            suited_to: None,
            tool: 0,
            picked_tool: false,
            direction: 0,
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

    /// The form for changing the bore `id` (an inside contour), in the setup it is in, on the
    /// holes it has; why not, when the design has no such operation.
    pub fn edit(app: &SolveApp, id: &str) -> Result<BoreForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "contour" || base["side"] != "inside" {
            return Err(format!("operation {id} is not a bore"));
        }
        let mut form = BoreForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        (form.tool, form.picked_tool) = (form.tools.iter().position(|t| t.number == tool).unwrap_or(0), true);
        form.direction = DIRECTIONS.iter().position(|d| Some(d.0) == base["direction"].as_str()).unwrap_or(0);
        (form.rpm, form.typed_rpm) = base["rpm"].as_f64().map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = base["feed"].as_f64().map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// What is missing before holes can be bored, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_contour"))
        } else {
            None
        }
    }

    /// The groups of holes found, none while they could not be.
    pub fn groups(&self) -> &[HoleGroup] {
        self.found.get().map_or(&[], |(_, groups)| groups.as_slice())
    }

    fn picked(&self) -> Option<&HoleGroup> {
        self.groups().get(self.group)
    }

    /// The tool that suits holes of `diameter`: the widest that leaves room for its helix (nine
    /// tenths of the hole at most), else the narrowest there is, which then says why it does
    /// not fit.
    fn fitting_tool(&self, diameter: f64) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let fits = self.tools.iter().enumerate().filter(|(_, tool)| tool.diameter <= WIDEST_TOOL * diameter + 1e-6).max_by(by_width);
        fits.or(self.tools.iter().enumerate().min_by(by_width)).map_or(0, |(at, _)| at)
    }

    /// The `cam.op.bore` parameters: a point in each hole of the group (its centre), and where
    /// the cut ends: at the end of the wall of a hole with a floor or a drill's point, through
    /// the part for one that goes through. A speed or feed that was not typed is left to the
    /// command. A bore being changed keeps its own holes and the rest as kept.
    pub fn params(&self) -> Value {
        let mut contour = match (&self.editing, self.picked()) {
            (Some(edited), _) => edited.base.clone(),
            (None, Some(group)) => {
                let bottom = match group.bottom {
                    HoleBottom::Through => Value::Null,
                    HoleBottom::Flat | HoleBottom::Point => json!(group.top - group.depth),
                };
                json!({"side": "inside", "openings": group.holes, "bottom": bottom})
            }
            (None, None) => json!({"side": "inside", "openings": []}),
        };
        contour["direction"] = json!(word(&DIRECTIONS, self.direction));
        set_or_clear(&mut contour, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut contour, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "contour": contour,
        })
    }

    /// Finds the setup's holes and works the boring out, when the design or a field changed, and
    /// shows what was not typed as the holes and the tool give it.
    fn refresh(&mut self, app: &SolveApp) {
        // An operation being changed keeps its holes: nothing is looked for or suited to them.
        if self.editing.is_none() {
            let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            if self.found.refresh(app, &json!(setup), || manufacture_holes(&app.session, &setup)) {
                // Other holes: start from their largest group again.
                (self.group, self.suited_to) = (0, None);
            }
            if self.group >= self.groups().len() {
                self.group = 0;
            }
            if self.suited_to != Some(self.group) {
                self.suited_to = Some(self.group);
                if let Some(diameter) = self.picked().map(|group| group.diameter).filter(|_| !self.picked_tool) {
                    self.tool = self.fitting_tool(diameter);
                }
            }
        }
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.bore", &params)) {
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

    /// Runs the command: the form again, with the reason, when the boring can't be worked out or
    /// the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<BoreForm> {
        self.refresh(app);
        if self.missing().is_some() || (self.editing.is_none() && self.picked().is_none()) || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "contour": params["contour"].take()})),
            None => app.run("cam.op.bore", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<BoreForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let group_labels: Vec<String> = self.groups().iter().map(|group| group_label(group, units, decimals)).collect();
        let group_labels: Vec<&str> = group_labels.iter().map(String::as_str).collect();
        let directions = labels(&DIRECTIONS);
        let missing = self.missing();
        let ready = missing.is_none() && (self.editing.is_some() || self.picked().is_some()) && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // The holes of an operation being changed: how many it has.
        let kept_holes = self.editing.as_ref().map(|edited| {
            let count = edited.base["openings"].as_array().map_or(0, Vec::len).to_string();
            solidcraft_ui_text::format("manufacture.drill_holes_kept", &[("count", &count)])
        });
        // The setup picked is the one the viewport draws, so the holes are seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_bore");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_bore"), "hole", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_bore_missing"));
                    return;
                }
                if let (Some(kept), Some(setup)) = (&kept_holes, setup_labels.first()) {
                    // An operation stays in its setup and on its holes.
                    ui_kit::value_row(ui, tr("manufacture.field_setup"), setup);
                    ui_kit::value_row(ui, tr("manufacture.drill_holes"), kept);
                } else {
                    ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_bore_setup", &mut self.setup, &setup_labels);
                    if group_labels.is_empty() {
                        // Why no holes could be looked for, or that there are none to bore from here.
                        match self.found.trouble().filter(|why| !why.is_empty()) {
                            Some(why) => refusal(ui, Some(why)),
                            None => {
                                ui_kit::text(ui, tr("manufacture.drill_no_holes"), ui_kit::Text::Note("manufacture_bore_no_holes"));
                            }
                        }
                        return;
                    }
                    ui_kit::choice_row(ui, tr("manufacture.drill_holes"), "manufacture_bore_holes", &mut self.group, &group_labels);
                }
                let tool = self.tool;
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_bore_tool", &mut self.tool, &tool_labels);
                self.picked_tool |= self.tool != tool;
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_bore_direction", &mut self.direction, &directions);
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_bore_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.bore_note"), ui_kit::Text::Note("manufacture_bore_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_bore_preview_note"));
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
