//! Bore's form (decision 0021 of the CAM work): round holes wider than the tool, cut with an end
//! mill down a helix along their wall. It offers the holes Drill offers, a group at a time,
//! builds the parameters of `cam.op.bore` and shows what they would cut before Create, as the
//! other operations' forms do. What it makes is an inside contour, and is changed in Contour's
//! form.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{HoleBottom, HoleGroup, OperationPreview, ToolChoice, manufacture_holes, manufacture_preview, manufacture_tools};
use solidcraft_ui_text::tr;

use super::contour::DIRECTIONS;
use super::{
    SetupChoice, Worked, group_label, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words, word,
};
use crate::SolveApp;
use crate::manufacture_workspace::picking;
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
    /// The holes of the group picked on the model, when they are not all of them.
    pub holes: picking::HolePicks,
    /// The group the tool was last suited to.
    suited_to: Option<usize>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then it is the widest that goes into the holes
    /// with room for its helix.
    pub picked_tool: bool,
    pub direction: usize,
    /// The Z the cut ends at: the end of the holes' wall until it is typed (a little under the
    /// part, to break through).
    pub bottom: f64,
    pub typed_bottom: bool,
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
            holes: picking::HolePicks::new(),
            suited_to: None,
            tool: 0,
            picked_tool: false,
            direction: 0,
            bottom: 0.0,
            typed_bottom: false,
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            worked: Worked::new(),
            drawn_setup: None,
            error: None,
        }
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
    /// the cut ends: where it was typed to, else at the end of the wall of a hole with a floor or
    /// a drill's point, and through the part for one that goes through. A speed or feed that was
    /// not typed is left to the command.
    pub fn params(&self) -> Value {
        let mut contour = match self.picked() {
            Some(group) => {
                let bottom = match group.bottom {
                    _ if self.typed_bottom => json!(self.bottom),
                    HoleBottom::Through => Value::Null,
                    HoleBottom::Flat | HoleBottom::Point => json!(group.top - group.depth),
                };
                json!({"side": "inside", "openings": self.holes.of(group), "bottom": bottom})
            }
            None => json!({"side": "inside", "openings": []}),
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
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        if self.found.refresh(app, &json!(setup), || manufacture_holes(&app.session, &setup)) {
            // Other holes: start from their largest group again.
            (self.group, self.suited_to) = (0, None);
        }
        if self.group >= self.groups().len() {
            self.group = 0;
        }
        if self.suited_to != Some(self.group) {
            // Other holes: their own bottom again, and the tool that suits them.
            (self.suited_to, self.typed_bottom) = (Some(self.group), false);
            if let Some(diameter) = self.picked().map(|group| group.diameter).filter(|_| !self.picked_tool) {
                self.tool = self.fitting_tool(diameter);
            }
        }
        if let Some(ends) = self.picked().map(|group| group.top - group.depth).filter(|_| !self.typed_bottom) {
            self.bottom = ends;
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
        if self.missing().is_some() || self.picked().is_none() || self.worked.result.is_err() {
            return Some(self);
        }
        match app.run("cam.op.bore", self.params()) {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<BoreForm> {
        // What is picked on the model is what is bored (decision 0030 of the CAM work): a group
        // chosen from the row's list forgets it, and a hole clicked is taken and worked out.
        self.holes.follow_row(app, self.group);
        self.refresh(app);
        let found = self.found.get().map(|(key, _)| key);
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        let groups = self.found.get().map_or(&[][..], |(_, groups)| groups.as_slice());
        if self.holes.take(app, ctx, found, &setup, groups, &mut self.group) {
            self.refresh(app);
        }
        let picking = self.picked().map(|group| self.holes.says(group.holes.len()));
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let group_labels: Vec<String> = self.groups().iter().map(|group| group_label(group, units, decimals)).collect();
        let group_labels: Vec<&str> = group_labels.iter().map(String::as_str).collect();
        let directions = labels(&DIRECTIONS);
        let missing = self.missing();
        let ready = missing.is_none() && self.picked().is_some() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // The setup picked is the one the viewport draws, so the holes are seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&None, "cmd.cam_op_bore");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_bore"), "cam_bore", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_bore_missing"));
                    return;
                }
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
                if let Some(picking) = &picking {
                    ui_kit::text(ui, picking, ui_kit::Text::Note("manufacture_bore_pick"));
                }
                let tool = self.tool;
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_bore_tool", &mut self.tool, &tool_labels);
                self.picked_tool |= self.tool != tool;
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_bore_direction", &mut self.direction, &directions);
                if ui_kit::row(ui, tr("manufacture.contour_bottom"), |ui| ui_kit::length_field(ui, &mut self.bottom, units, decimals)).changed() {
                    self.typed_bottom = true;
                }
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
                    ui_kit::text(ui, preview_note(&None), ui_kit::Text::Note("manufacture_bore_preview_note"));
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
