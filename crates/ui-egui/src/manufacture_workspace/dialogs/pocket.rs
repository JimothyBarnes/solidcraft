//! Pocket's form (decision 0014 of the CAM work): the closed pockets found on the model, cleared
//! in rings: all of them unless one is picked. It builds the parameters of `cam.op.pocket` (of
//! `cam.op.edit` for a pocket operation being changed) and shows what they would cut before OK,
//! as the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{
    FoundPocket, OperationPreview, ToolChoice, manufacture_pocket_at, manufacture_pockets, manufacture_preview, manufacture_tools,
};
use solidcraft_engine::doc::display_units::{format_length, length_number};
use solidcraft_ui_text::tr;

use super::contour::DIRECTIONS;
use super::{
    Edited, SetupChoice, Worked, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
    word,
};
use crate::SolveApp;
use crate::manufacture_workspace::picking;
use crate::ui_kit;

/// What the form's picks on the model are called.
const PICKED: &str = "Pockets";
/// The stepover and the stepdown a pocket takes when none is typed, as shares of the tool's
/// diameter (the engine's own defaults, shown in the fields).
const STEPOVER_SHARE: f64 = 0.4;
const STEPDOWN_SHARE: f64 = 0.5;

/// Pocket: a closed pocket found on the model.
pub(crate) struct PocketForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// The closed pockets the setup picked can cut, as the engine found them.
    found: Worked<Vec<FoundPocket>>,
    /// Which of the form's choices is picked: with more than one pocket found the first choice
    /// is all of them, and the pockets follow it one by one; after them, while several are
    /// picked on the model, a choice that stands for those.
    pub pocket: usize,
    /// The pockets picked on the model, when they are several and not all: their places among
    /// those found, in order. One pocket picked, or every one, is a choice of the row's own.
    pub picks: Vec<usize>,
    /// What was clicked on the model: the faces, each with the pocket it is of.
    clicked: picking::Picks<usize>,
    /// The choice as the clicks on the model last left it: another one since is the row's own.
    left_at: usize,
    /// The choice and the picks the tool was last suited to.
    suited_to: Option<(usize, Vec<usize>)>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then it is the widest that passes everywhere
    /// in the pockets picked.
    pub picked_tool: bool,
    pub direction: usize,
    /// mm: 40% of the tool's diameter until it is typed.
    pub stepover: f64,
    pub typed_stepover: bool,
    /// mm: half the tool's diameter until it is typed.
    pub stepdown: f64,
    pub typed_stepdown: bool,
    /// Stock left on the walls, mm.
    pub leave: f64,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The pocketing these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The pocket operation being changed, when the form changes one: its `pocket` parameters as
    /// kept. Its pockets stay the ones it has.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl PocketForm {
    /// For the setup the viewport draws, all its pockets and the widest tool that clears them.
    pub fn new(app: &SolveApp) -> PocketForm {
        let (setups, setup) = setups(app);
        PocketForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "pocket"),
            setup,
            found: Worked::new(),
            pocket: 0,
            picks: Vec::new(),
            clicked: picking::Picks::new(),
            left_at: 0,
            suited_to: None,
            tool: 0,
            picked_tool: false,
            direction: 0,
            stepover: 0.0,
            typed_stepover: false,
            stepdown: 0.0,
            typed_stepdown: false,
            leave: 0.0,
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

    /// The form for changing the pocket operation `id`, on the pockets it has; why not, when the
    /// design has no such pocket operation.
    pub fn edit(app: &SolveApp, id: &str) -> Result<PocketForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "pocket" {
            return Err(format!("operation {id} is not a pocket operation"));
        }
        let number = |key: &str| base[key].as_f64();
        let mut form = PocketForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        (form.tool, form.picked_tool) = (form.tools.iter().position(|t| t.number == tool).unwrap_or(0), true);
        form.direction = DIRECTIONS.iter().position(|d| Some(d.0) == base["direction"].as_str()).unwrap_or(0);
        (form.stepover, form.typed_stepover) = number("stepover").map_or((0.0, false), |v| (v, true));
        (form.stepdown, form.typed_stepdown) = number("stepdown").map_or((0.0, false), |v| (v, true));
        form.leave = number("leave").unwrap_or(0.0);
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// The pockets found, none while they could not be.
    pub fn pockets(&self) -> &[FoundPocket] {
        self.found.get().map_or(&[], |(_, found)| found.as_slice())
    }

    /// Whether the choices start with all the pockets at once: when more than one was found.
    fn offers_all(&self) -> bool {
        self.pockets().len() > 1
    }

    /// The pockets the choice picked stands for: all of them, one, or the several picked on
    /// the model.
    pub fn picked(&self) -> Vec<&FoundPocket> {
        let all = self.pockets();
        if !self.picks.is_empty() {
            return self.picks.iter().filter_map(|at| all.get(*at)).collect();
        }
        match (self.offers_all(), self.pocket) {
            (true, 0) => all.iter().collect(),
            (true, choice) => all.get(choice - 1).into_iter().collect(),
            (false, choice) => all.get(choice).into_iter().collect(),
        }
    }

    /// Whether the last click on the model was on no pocket.
    #[cfg(test)]
    pub fn pick_missed(&self) -> bool {
        self.clicked.missed
    }

    /// Takes what was clicked on the model since the last frame (decision 0030 of the CAM
    /// work) and makes it the choice: one pocket, all of them, or several. Whether the choice
    /// changed.
    fn take_picks(&mut self, app: &mut SolveApp, ctx: &egui::Context) -> bool {
        if self.editing.is_some() {
            return false;
        }
        let found = self.found.get().map(|(key, _)| key);
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        let on_pocket = |app: &SolveApp, point| manufacture_pocket_at(&app.session, &setup, point).ok().flatten();
        let Some(mut picks) = self.clicked.take(app, ctx, PICKED, found, on_pocket) else { return false };
        picks.sort_unstable();
        let (count, first) = (self.pockets().len(), usize::from(self.offers_all()));
        // None left, or every one: all of them, the row's first choice. One: the row's own
        // choice of it. Several: the choice the row gains for them, after the pockets.
        let own = match picks.as_slice() {
            [] => Some(0),
            every if every.len() == count => Some(0),
            [one] => Some(first + one),
            _ => None,
        };
        (self.pocket, self.picks) = match own {
            Some(choice) => (choice, Vec::new()),
            None => (first + count, picks),
        };
        true
    }

    /// Forgets what was picked on the model: a choice from the row's list is the choice.
    fn forget_picks(&mut self, app: &mut SolveApp) {
        self.clicked.forget(app);
        self.picks.clear();
    }

    /// What is missing before a pocket can be cut, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_contour"))
        } else if self.editing.is_none() && self.found.result.as_ref().is_ok_and(Vec::is_empty) {
            Some(tr("manufacture.pocket_none"))
        } else {
            None
        }
    }

    /// The tool that suits `pockets`: the widest that passes everywhere in every one of them
    /// (through their necks, round their islands), so each is cleared whole; when none does, the
    /// widest that fits in them all; else the narrowest there is.
    fn fitting_tool(&self, pockets: &[&FoundPocket]) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let narrower = |than: f64| self.tools.iter().enumerate().filter(move |(_, tool)| tool.diameter < than - 1e-6).max_by(by_width);
        let least = |of: fn(&FoundPocket) -> f64| pockets.iter().map(|pocket| of(pocket)).fold(f64::INFINITY, f64::min);
        narrower(least(|pocket| pocket.passes))
            .or(narrower(least(|pocket| pocket.widest)))
            .or(self.tools.iter().enumerate().min_by(by_width))
            .map_or(0, |(at, _)| at)
    }

    /// Whether the tool picked is too wide for one of the pockets picked.
    pub fn tool_too_wide(&self) -> bool {
        let too_wide = |tool: &ToolChoice| self.picked().iter().any(|pocket| tool.diameter >= pocket.widest - 1e-6);
        self.editing.is_none() && self.tools.get(self.tool).is_some_and(too_wide)
    }

    /// The `cam.op.pocket` parameters: the point on the floor of each pocket picked, and what was
    /// typed. What was not typed is left to the command, so it keeps following the tool. A pocket
    /// operation being changed keeps its own pockets and the rest as kept.
    pub fn params(&self) -> Value {
        let mut pocket = match &self.editing {
            Some(edited) => edited.base.clone(),
            None => json!({"pockets": self.picked().iter().map(|found| found.at).collect::<Vec<_>>()}),
        };
        pocket["direction"] = json!(word(&DIRECTIONS, self.direction));
        set_or_clear(&mut pocket, "stepover", self.typed_stepover.then_some(self.stepover));
        set_or_clear(&mut pocket, "stepdown", self.typed_stepdown.then_some(self.stepdown));
        set_or_clear(&mut pocket, "leave", (self.leave > 0.0).then_some(self.leave));
        set_or_clear(&mut pocket, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut pocket, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "pocket": pocket,
        })
    }

    /// Finds the setup's pockets and works the pocketing out, when the design or a field
    /// changed, and shows what was not typed as the pocket and the tool give it.
    fn refresh(&mut self, app: &SolveApp) {
        // An operation being changed keeps its pockets: nothing is looked for or suited to them.
        if self.editing.is_none() {
            let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            if self.found.refresh(app, &json!({"id": setup}), || manufacture_pockets(&app.session, &setup)) {
                self.suited_to = None;
            }
            let choices = self.pockets().len() + usize::from(self.offers_all()) + usize::from(!self.picks.is_empty());
            self.pocket = self.pocket.min(choices.saturating_sub(1));
            let suited = self.suited_to.as_ref().is_some_and(|(choice, picks)| *choice == self.pocket && *picks == self.picks);
            if !suited && !self.picked().is_empty() {
                if !self.picked_tool {
                    self.tool = self.fitting_tool(&self.picked());
                }
                self.suited_to = Some((self.pocket, self.picks.clone()));
            }
        }
        let diameter = self.tools.get(self.tool).map(|t| t.diameter);
        if !self.typed_stepover {
            self.stepover = diameter.map_or(self.stepover, |d| STEPOVER_SHARE * d);
        }
        if !self.typed_stepdown {
            self.stepdown = diameter.map_or(self.stepdown, |d| STEPDOWN_SHARE * d);
        }
        self.leave = self.leave.max(0.0);
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.pocket", &params)) {
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

    /// A pocket found, as the form offers it: its size across and its depth below the part's
    /// top, and its islands when it has any ("70.00 × 44.00 × 8.00 mm, islands: 1").
    fn label(found: &FoundPocket, units: &str, decimals: u8) -> String {
        let (width, height, depth) = (
            length_number(found.size[0], units, decimals),
            length_number(found.size[1], units, decimals),
            format_length(found.depth, units, decimals),
        );
        let islands = found.islands.to_string();
        let key = if found.islands > 0 { "manufacture.pocket_label_islands" } else { "manufacture.pocket_label" };
        solidcraft_ui_text::format(key, &[("width", &width), ("height", &height), ("depth", &depth), ("islands", &islands)])
    }

    /// The levels and the rings a pocket would be cut in, as the form says them: for one
    /// pocket, since each of several has its own.
    fn worked_out(&self, units: &str, decimals: u8) -> Option<(String, String)> {
        let (_, preview) = self.worked.get()?;
        let levels = preview.numbers.get("levels")?.as_array().filter(|each| each.len() == 1)?.first()?.as_array()?;
        let (top, floor) = (preview.numbers.get("top")?.as_f64()?, levels.last()?.as_f64()?);
        let each = format_length((top - floor) / levels.len().max(1) as f64, units, decimals);
        let levels = solidcraft_ui_text::format("manufacture.contour_levels_value", &[("count", &levels.len().to_string()), ("depth", &each)]);
        let rings = preview.numbers.get("rings")?.as_array()?.first()?.as_u64()?.to_string();
        Some((levels, rings))
    }

    /// Runs the command: the form again, with the reason, when the pocketing can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<PocketForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "pocket": params["pocket"].take()})),
            None => app.run("cam.op.pocket", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<PocketForm> {
        // A choice from the row's list is the choice: what was picked on the model is forgotten.
        if self.pocket != self.left_at {
            self.forget_picks(app);
        }
        self.refresh(app);
        // What was clicked on the model is the choice, and what the choice makes is worked out.
        if self.take_picks(app, ctx) {
            self.refresh(app);
        }
        self.left_at = self.pocket;
        let picking = self.editing.is_none();
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        // All of them first, when there is more than one; then each by its size.
        let all = solidcraft_ui_text::format("manufacture.pocket_all", &[("count", &self.pockets().len().to_string())]);
        // After them, while several are picked on the model, the choice that stands for those.
        let several = (!self.picks.is_empty()).then(|| {
            let (count, of) = (self.picks.len().to_string(), self.pockets().len().to_string());
            solidcraft_ui_text::format("manufacture.pocket_picked", &[("count", &count), ("of", &of)])
        });
        let pocket_labels: Vec<String> = self
            .offers_all()
            .then_some(all)
            .into_iter()
            .chain(self.pockets().iter().map(|found| Self::label(found, units, decimals)))
            .chain(several)
            .collect();
        let pocket_labels: Vec<&str> = pocket_labels.iter().map(String::as_str).collect();
        let kept = self.editing.as_ref().map(|edited| {
            let count = edited.base["pockets"].as_array().map_or(0, Vec::len).to_string();
            solidcraft_ui_text::format("manufacture.pocket_kept", &[("count", &count)])
        });
        let directions = labels(&DIRECTIONS);
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        let worked_out = self.worked_out(units, decimals);
        let too_wide = self.tool_too_wide();
        // The setup picked is the one the viewport draws, so the pocket is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_pocket");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_pocket"), "cam_pocket", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                // An operation stays in its setup: the row is a choice only for a new one.
                match (&self.editing, setup_labels.first()) {
                    (Some(_), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                    }
                    _ if !setup_labels.is_empty() => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_pocket_setup", &mut self.setup, &setup_labels);
                    }
                    _ => {}
                }
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_pocket_missing"));
                    return;
                }
                match &kept {
                    Some(kept) => {
                        ui_kit::value_row(ui, tr("manufacture.pocket_choice"), kept);
                    }
                    None => {
                        ui_kit::choice_row(ui, tr("manufacture.pocket_choice"), "manufacture_pocket_choice", &mut self.pocket, &pocket_labels);
                        if picking {
                            let said = if self.clicked.missed { "manufacture.pocket_pick_missed" } else { "manufacture.pocket_pick_hint" };
                            ui_kit::text(ui, tr(said), ui_kit::Text::Note("manufacture_pocket_pick"));
                        }
                    }
                }
                if ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_pocket_tool", &mut self.tool, &tool_labels).changed() {
                    self.picked_tool = true;
                }
                if too_wide {
                    ui_kit::text(ui, tr("manufacture.pocket_tool_wide"), ui_kit::Text::Note("manufacture_pocket_tool_wide"));
                }
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_pocket_direction", &mut self.direction, &directions);
                if ui_kit::row(ui, tr("manufacture.face_stepover"), |ui| ui_kit::length_field(ui, &mut self.stepover, units, decimals)).changed() {
                    self.typed_stepover = true;
                }
                if ui_kit::row(ui, tr("manufacture.face_stepdown"), |ui| ui_kit::length_field(ui, &mut self.stepdown, units, decimals)).changed() {
                    self.typed_stepdown = true;
                }
                ui_kit::row(ui, tr("manufacture.contour_leave"), |ui| ui_kit::length_field(ui, &mut self.leave, units, decimals));
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some((levels, rings)) = &worked_out {
                    ui_kit::value_row(ui, tr("manufacture.contour_levels"), levels);
                    ui_kit::value_row(ui, tr("manufacture.pocket_rings"), rings);
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_pocket_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.pocket_note"), ui_kit::Text::Note("manufacture_pocket_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_pocket_preview_note"));
                }
                refusal(ui, self.error.as_deref().or(self.found.trouble()).or(self.worked.trouble()));
            });
        match action {
            ui_kit::Action::Ok => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}
