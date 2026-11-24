//! Thread Mill's form (decision 0034 of the CAM work): internal threads cut by going round
//! drilled holes with a thread mill. It offers the holes Drill offers, a group at a time, builds
//! the parameters of `cam.op.thread` (of `cam.op.edit` for a thread mill operation being
//! changed) and shows what they would cut before OK, as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{HoleBottom, HoleGroup, OperationPreview, ToolChoice, manufacture_holes, manufacture_preview, manufacture_tools};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{
    Edited, SetupChoice, Worked, group_label, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed,
    window_words, word,
};
use crate::SolveApp;
use crate::manufacture_workspace::picking;
use crate::ui_kit;

/// Which way the thread turns: the command's word and the label.
const HANDS: [(&str, &str); 2] = [("right", "manufacture.thread_hand_right"), ("left", "manufacture.thread_hand_left")];
/// How many passes the thread's depth is taken in until another number is picked: the engine's
/// own default.
const PASSES: u32 = 2;
/// How much wider than its minor diameter an ISO metric thread's major diameter is, in pitches.
const MAJOR_OVER_MINOR: f64 = 1.082_531_754_730_548;

/// Thread Mill: round holes found on the model, a group at a time.
pub(crate) struct ThreadForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// The holes the setup picked has, in groups, as the engine found them.
    found: Worked<Vec<HoleGroup>>,
    pub group: usize,
    /// The holes of the group picked on the model, when they are not all of them.
    pub holes: picking::HolePicks,
    /// The group and the tool the thread's size and depth were last suggested for.
    suited_to: Option<(usize, usize)>,
    pub tool: usize,
    /// The thread's major diameter, mm: suggested from the holes' own and the tool's pitch (the
    /// nearest half millimetre to the hole's diameter and 1.0825 pitches) until typed.
    pub diameter: f64,
    pub typed_diameter: bool,
    /// How far down from the holes' top the thread goes, mm: all of a hole that goes through,
    /// one pitch short of the bottom of one that does not, until typed.
    pub depth: f64,
    pub typed_depth: bool,
    pub passes: u32,
    /// Which of [`HANDS`].
    pub hand: usize,
    /// Spindle speed, rev/min, and the feed at the cutting edge, mm/min: what the tool's
    /// cutting data gives for the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The threads these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The operation being changed, when the form changes one: its `thread` parameters as kept.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl ThreadForm {
    /// For the setup the viewport draws, its largest group of holes and the first thread mill.
    pub fn new(app: &SolveApp) -> ThreadForm {
        let (setups, setup) = setups(app);
        ThreadForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "thread"),
            setup,
            found: Worked::new(),
            group: 0,
            holes: picking::HolePicks::new(),
            suited_to: None,
            tool: 0,
            diameter: 0.0,
            typed_diameter: false,
            depth: 0.0,
            typed_depth: false,
            passes: PASSES,
            hand: 0,
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

    /// The form for changing the thread mill operation `id`, in the setup it is in; why not,
    /// when the design has no such operation. Its holes stay the ones it has.
    pub fn edit(app: &SolveApp, id: &str) -> Result<ThreadForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "thread" {
            return Err(format!("operation {id} is not a thread mill operation"));
        }
        let number = |key: &str| base[key].as_f64();
        let mut form = ThreadForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        form.tool = form.tools.iter().position(|t| t.number == tool).unwrap_or(0);
        (form.diameter, form.typed_diameter) = (number("diameter").unwrap_or(0.0), true);
        (form.depth, form.typed_depth) = (number("depth").unwrap_or(0.0), true);
        form.passes = base["passes"].as_u64().and_then(|passes| u32::try_from(passes).ok()).unwrap_or(PASSES);
        form.hand = HANDS.iter().position(|hand| Some(hand.0) == base["hand"].as_str()).unwrap_or(0);
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// What is missing before threads can be milled, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_thread"))
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

    /// The pitch of the tool picked, mm.
    fn pitch(&self) -> Option<f64> {
        self.tools.get(self.tool).and_then(|tool| tool.pitch)
    }

    /// The `cam.op.thread` parameters: the middle of each hole of the group, their top, the
    /// thread's size and depth, and what was picked or typed. An operation being changed starts
    /// from its parameters as kept, so its holes and what the form does not show stay.
    pub fn params(&self) -> Value {
        let mut thread = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        if self.editing.is_none()
            && let Some(group) = self.picked()
        {
            thread["holes"] = json!(self.holes.of(group));
            thread["top"] = json!(group.top);
        }
        thread["diameter"] = json!(self.diameter);
        thread["depth"] = json!(self.depth);
        // Only what is other than usual is written: a whole number of passes, a left hand.
        if let Some(kept) = thread.as_object_mut() {
            kept.remove("passes");
            kept.remove("hand");
        }
        if self.passes != PASSES {
            thread["passes"] = json!(self.passes);
        }
        if word(&HANDS, self.hand) == "left" {
            thread["hand"] = json!("left");
        }
        set_or_clear(&mut thread, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut thread, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "thread": thread,
        })
    }

    /// Finds the setup's holes and works the threads out, when the design or a field changed,
    /// and shows what was not typed as the holes and the tool give it.
    fn refresh(&mut self, app: &SolveApp) {
        (self.diameter, self.depth, self.passes) = (self.diameter.max(0.0), self.depth.max(0.0), self.passes.clamp(1, 20));
        if self.editing.is_none() {
            let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            if self.found.refresh(app, &json!(setup), || manufacture_holes(&app.session, &setup)) {
                // Other holes: start from their largest group again.
                (self.group, self.suited_to) = (0, None);
            }
            if self.group >= self.groups().len() {
                self.group = 0;
            }
            if self.suited_to != Some((self.group, self.tool)) {
                // Other holes or another tool: the thread they suggest, again.
                (self.suited_to, self.typed_diameter, self.typed_depth) = (Some((self.group, self.tool)), false, false);
            }
            if let (Some(group), Some(pitch)) = (self.picked().cloned(), self.pitch()) {
                if !self.typed_diameter {
                    self.diameter = (2.0 * (group.diameter + MAJOR_OVER_MINOR * pitch)).round() / 2.0;
                }
                if !self.typed_depth {
                    self.depth = match group.bottom {
                        HoleBottom::Through => group.depth,
                        HoleBottom::Flat | HoleBottom::Point => (group.depth - pitch).max(0.5 * group.depth),
                    };
                }
            }
        }
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.thread", &params)) {
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

    /// Runs the command: the form again, with the reason, when the threads can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ThreadForm> {
        self.refresh(app);
        if self.missing().is_some() || (self.editing.is_none() && self.picked().is_none()) || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "thread": params["thread"].take()})),
            None => app.run("cam.op.thread", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<ThreadForm> {
        // What is picked on the model is what is threaded (decision 0030 of the CAM work): a
        // group chosen from the row's list forgets it, and a hole clicked is taken and worked out.
        let picks = self.editing.is_none();
        if picks {
            self.holes.follow_row(app, self.group);
        }
        self.refresh(app);
        if picks {
            let found = self.found.get().map(|(key, _)| key);
            let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            let groups = self.found.get().map_or(&[][..], |(_, groups)| groups.as_slice());
            if self.holes.take(app, ctx, found, &setup, groups, &mut self.group) {
                self.refresh(app);
            }
        }
        let picking = self.picked().filter(|_| picks).map(|group| self.holes.says(group.holes.len()));
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let group_labels: Vec<String> = self.groups().iter().map(|group| group_label(group, units, decimals)).collect();
        let group_labels: Vec<&str> = group_labels.iter().map(String::as_str).collect();
        let hands = labels(&HANDS);
        let missing = self.missing();
        let ready = missing.is_none() && (self.editing.is_some() || self.picked().is_some()) && self.worked.result.is_ok();
        let number = |key: &str| self.worked.get().and_then(|(_, preview)| preview.numbers.get(key)?.as_f64());
        let pitch = number("pitch").or(self.pitch()).map(|v| format_length(v, units, decimals));
        let drill = number("minor").map(|v| format_length(v, units, decimals));
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // The holes of an operation being changed are the ones it has: how many.
        let kept = self.editing.as_ref().map(|edited| {
            let count = edited.base["holes"].as_array().map_or(0, Vec::len);
            solidcraft_ui_text::format("manufacture.slot_kept", &[("count", &count.to_string())])
        });
        // The setup picked is the one the viewport draws, so the holes are seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_thread");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_thread"), "cam_thread", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_thread_missing"));
                    return;
                }
                // An operation stays in its setup and keeps its holes: rows to read, not to choose.
                match (&kept, setup_labels.first()) {
                    (Some(kept), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                        ui_kit::value_row(ui, tr("manufacture.drill_holes"), kept);
                    }
                    _ => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_thread_setup", &mut self.setup, &setup_labels);
                        if group_labels.is_empty() {
                            // Why no holes could be looked for, or that there are none from here.
                            match self.found.trouble().filter(|why| !why.is_empty()) {
                                Some(why) => refusal(ui, Some(why)),
                                None => {
                                    ui_kit::text(ui, tr("manufacture.drill_no_holes"), ui_kit::Text::Note("manufacture_thread_no_holes"));
                                }
                            }
                            return;
                        }
                        ui_kit::choice_row(ui, tr("manufacture.drill_holes"), "manufacture_thread_holes", &mut self.group, &group_labels);
                        if let Some(picking) = &picking {
                            ui_kit::text(ui, picking, ui_kit::Text::Note("manufacture_thread_pick"));
                        }
                    }
                }
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_thread_tool", &mut self.tool, &tool_labels);
                if ui_kit::row(ui, tr("manufacture.thread_diameter"), |ui| ui_kit::length_field(ui, &mut self.diameter, units, decimals)).changed() {
                    self.typed_diameter = true;
                }
                if let Some(pitch) = &pitch {
                    ui_kit::value_row(ui, tr("manufacture.tool_thread_pitch"), pitch);
                }
                if ui_kit::row(ui, tr("manufacture.thread_depth"), |ui| ui_kit::length_field(ui, &mut self.depth, units, decimals)).changed() {
                    self.typed_depth = true;
                }
                ui_kit::row(ui, tr("manufacture.thread_passes"), |ui| ui_kit::whole_number_field(ui, &mut self.passes, 1..=20, ""));
                ui_kit::choice_row(ui, tr("manufacture.thread_hand"), "manufacture_thread_hand", &mut self.hand, &hands);
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(drill) = &drill {
                    ui_kit::value_row(ui, tr("manufacture.thread_drill"), drill);
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_thread_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.thread_note"), ui_kit::Text::Note("manufacture_thread_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_thread_preview_note"));
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
