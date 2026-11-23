//! Contour's form (decisions 0013 and 0021 of the CAM work): round the outside of the part, or
//! round the inside of openings found on the model. It builds the parameters of `cam.op.contour`
//! (of `cam.op.edit` for a contour being changed) and shows what they would cut before OK, as
//! the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{
    FoundOpening, OperationPreview, Placement, ToolChoice, manufacture_openings, manufacture_preview, manufacture_setup_preview, manufacture_tools,
};
use solidcraft_engine::doc::display_units::{format_length, length_number};
use solidcraft_ui_text::tr;

use super::{
    Edited, SetupChoice, Worked, kept_operation, labels, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
    word,
};
use crate::SolveApp;
use crate::ui_kit;

/// Which way round the cut goes: the command's word and the label.
pub(super) const DIRECTIONS: [(&str, &str); 2] = [("climb", "manufacture.direction_climb"), ("conventional", "manufacture.direction_conventional")];
/// Which side of a wall the cut goes round: the command's word and the label.
const SIDES: [(&str, &str); 2] = [("outside", "manufacture.contour_side_outside"), ("inside", "manufacture.contour_side_inside")];
/// The widest tool picked for an opening, as a share of the widest circle that fits in it.
const WIDEST_TOOL: f64 = 0.9;
/// The stepdown a contour takes when none is typed, as a share of the tool's diameter (the
/// engine's own default, shown in the field).
const STEPDOWN_SHARE: f64 = 0.5;
/// How far apart sideways a contour's passes are when no stepover is typed, as a share of the
/// tool's diameter (the engine's own default, shown in the field), and the most passes there are.
const STEPOVER_SHARE: f64 = 0.4;
const MOST_PASSES: u32 = 100;
/// The radius of the arcs on to the wall and off it when none is typed, as a share of the
/// tool's diameter, and a tab's width and height, mm (the engine's own defaults, shown in the
/// fields).
const LEAD_SHARE: f64 = 0.1;
const TAB_WIDTH: f64 = 6.0;
const TAB_HEIGHT: f64 = 2.0;

/// Contour: round the outside of the part, or round the inside of its openings.
pub(crate) struct ContourForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// Which of [`SIDES`]: round the outside of the part, or round the inside of openings.
    pub side: usize,
    /// The openings the setup picked has, as the engine found them: looked for once the side
    /// is inside.
    found: Worked<Vec<FoundOpening>>,
    /// Which of the choices of openings is cut ([`ContourForm::choices`]).
    pub opening: usize,
    /// The choice of openings the tool was last suited to.
    suited_to: Option<usize>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then, inside, it is the widest that fits the
    /// openings picked.
    pub picked_tool: bool,
    pub direction: usize,
    /// The Z the cut goes down to in the setup's WCS, mm: the part's bottom until it is typed.
    pub bottom: f64,
    pub typed_bottom: bool,
    /// mm: half the tool's diameter until it is typed.
    pub stepdown: f64,
    pub typed_stepdown: bool,
    /// Stock left on the wall, mm.
    pub leave: f64,
    /// Outside: how many passes sideways at each level, the last one on the wall: one until
    /// typed. Their distance apart, mm: four tenths of the tool's diameter until it is typed.
    pub passes: u32,
    pub stepover: f64,
    pub typed_stepover: bool,
    /// The radius of the arcs on to the wall and off it, mm: a tenth of the tool's diameter
    /// until it is typed. 0 is straight in and out.
    pub lead: f64,
    pub typed_lead: bool,
    /// How many tabs hold each part: none until typed. Their width and height, mm.
    pub tabs: u32,
    pub tab_width: f64,
    pub tab_height: f64,
    pub typed_tab_width: bool,
    pub typed_tab_height: bool,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// Where the setup sits on the model: the part's bottom is read from it. (Boxed: it is the
    /// largest thing here, and the form is one of several kept in one enum.)
    placed: Worked<Box<Placement>>,
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
        let lead = tools.first().map_or(0.0, |t| LEAD_SHARE * t.diameter);
        let stepover = tools.first().map_or(0.0, |t| STEPOVER_SHARE * t.diameter);
        ContourForm {
            setups,
            tools,
            setup,
            side: 0,
            found: Worked::new(),
            opening: 0,
            suited_to: None,
            tool: 0,
            picked_tool: false,
            direction: 0,
            bottom: 0.0,
            typed_bottom: false,
            stepdown,
            typed_stepdown: false,
            leave: 0.0,
            passes: 1,
            stepover,
            typed_stepover: false,
            lead,
            typed_lead: false,
            tabs: 0,
            tab_width: TAB_WIDTH,
            tab_height: TAB_HEIGHT,
            typed_tab_width: false,
            typed_tab_height: false,
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
        (form.tool, form.picked_tool) = (tools.iter().position(|t| t.number == tool).unwrap_or(0), true);
        form.tools = tools;
        form.side = SIDES.iter().position(|side| Some(side.0) == base["side"].as_str()).unwrap_or(0);
        form.direction = DIRECTIONS.iter().position(|d| Some(d.0) == base["direction"].as_str()).unwrap_or(0);
        (form.bottom, form.typed_bottom) = number("bottom").map_or((0.0, false), |v| (v, true));
        (form.stepdown, form.typed_stepdown) = number("stepdown").map_or((form.stepdown, false), |v| (v, true));
        form.leave = number("leave").unwrap_or(0.0);
        form.passes = base["passes"].as_u64().map_or(1, |passes| u32::try_from(passes).unwrap_or(MOST_PASSES)).clamp(1, MOST_PASSES);
        (form.stepover, form.typed_stepover) = number("stepover").map_or((form.stepover, false), |v| (v, true));
        (form.lead, form.typed_lead) = number("lead").map_or((form.lead, false), |v| (v, true));
        let tabs = &base["tabs"];
        form.tabs = tabs["count"].as_u64().map_or(0, |count| u32::try_from(count).unwrap_or(u32::MAX));
        (form.tab_width, form.typed_tab_width) = tabs["width"].as_f64().map_or((TAB_WIDTH, false), |v| (v, true));
        (form.tab_height, form.typed_tab_height) = tabs["height"].as_f64().map_or((TAB_HEIGHT, false), |v| (v, true));
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

    /// Whether the cut goes round the inside of openings.
    pub fn inside(&self) -> bool {
        word(&SIDES, self.side) == "inside"
    }

    /// The openings found, none while they could not be (or the side is the outside).
    pub fn openings(&self) -> &[FoundOpening] {
        self.found.get().map_or(&[], |(_, found)| found.as_slice())
    }

    /// What the form offers to cut round the inside of, as places in [`ContourForm::openings`]:
    /// every two or more openings that are cut to one depth, together, then each one alone.
    pub fn choices(&self) -> Vec<Vec<usize>> {
        let found = self.openings();
        let mut together: Vec<Vec<usize>> = Vec::new();
        for (at, opening) in found.iter().enumerate() {
            let alike = |group: &Vec<usize>| {
                group
                    .first()
                    .and_then(|first| found.get(*first))
                    .is_some_and(|first| first.through == opening.through && (first.bottom - opening.bottom).abs() <= 1e-6)
            };
            match together.iter_mut().find(|group| alike(group)) {
                Some(group) => group.push(at),
                None => together.push(vec![at]),
            }
        }
        together.retain(|group| group.len() > 1);
        together.into_iter().chain((0..found.len()).map(|at| vec![at])).collect()
    }

    /// The openings the choice picked stands for.
    pub fn picked(&self) -> Vec<&FoundOpening> {
        let found = self.openings();
        self.choices().get(self.opening).map(|choice| choice.iter().filter_map(|at| found.get(*at)).collect()).unwrap_or_default()
    }

    /// The choices of openings as the form offers them: "40.00 × 30.00 mm, through the part",
    /// "Ø24.00 mm, 8.00 mm deep", "All 3 through the part".
    pub fn choice_labels(&self, units: &str, decimals: u8) -> Vec<String> {
        let found = self.openings();
        let label = |choice: &Vec<usize>| {
            let Some(first) = choice.first().and_then(|at| found.get(*at)) else { return String::new() };
            let (count, depth) = (choice.len().to_string(), format_length(first.depth, units, decimals));
            let (width, height) = (length_number(first.size[0], units, decimals), format_length(first.size[1], units, decimals));
            let diameter = first.diameter.map(|diameter| format_length(diameter, units, decimals)).unwrap_or_default();
            let key = match (choice.len() > 1, first.through, first.diameter.is_some()) {
                (true, true, _) => "manufacture.contour_openings_through",
                (true, false, _) => "manufacture.contour_openings_deep",
                (false, true, true) => "manufacture.contour_round_through",
                (false, false, true) => "manufacture.contour_round_deep",
                (false, true, false) => "manufacture.contour_opening_through",
                (false, false, false) => "manufacture.contour_opening_deep",
            };
            solidcraft_ui_text::format(key, &[("count", &count), ("depth", &depth), ("width", &width), ("height", &height), ("diameter", &diameter)])
        };
        self.choices().iter().map(label).collect()
    }

    /// The tool that suits openings the narrowest of which lets a circle `widest` across in: the
    /// widest tool that is no more than nine tenths of that, else the narrowest there is, which
    /// then says why it does not fit.
    fn fitting_tool(&self, widest: f64) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let fits = self.tools.iter().enumerate().filter(|(_, tool)| tool.diameter <= WIDEST_TOOL * widest + 1e-6).max_by(by_width);
        fits.or(self.tools.iter().enumerate().min_by(by_width)).map_or(0, |(at, _)| at)
    }

    /// The `cam.op.contour` parameters. What was not typed is left to the command, so it keeps
    /// following the model and the tool: the bottom the part's (inside, the opening's floor, or
    /// through), the stepdown and the arcs the tool's diameter, the speed and feed its cutting
    /// data. Inside, a point in each opening picked names it; there are no tabs there. A contour
    /// being changed starts from its parameters as kept, so what the form does not show (its
    /// top, its clearance, its side and its openings) stays.
    pub fn params(&self) -> Value {
        let mut contour = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        if self.editing.is_none() && self.inside() {
            contour["side"] = json!("inside");
            contour["openings"] = json!(self.picked().iter().map(|opening| opening.at).collect::<Vec<_>>());
        }
        contour["direction"] = json!(word(&DIRECTIONS, self.direction));
        set_or_clear(&mut contour, "bottom", self.typed_bottom.then_some(self.bottom));
        set_or_clear(&mut contour, "stepdown", self.typed_stepdown.then_some(self.stepdown));
        set_or_clear(&mut contour, "leave", (self.leave > 0.0).then_some(self.leave));
        set_or_clear(&mut contour, "lead", self.typed_lead.then_some(self.lead));
        // Passes sideways are the outside's, and a stepover is what lies between two or more.
        let passes = if self.inside() { 1 } else { self.passes };
        if passes > 1 {
            contour["passes"] = json!(passes);
        } else if let Some(kept) = contour.as_object_mut() {
            kept.remove("passes");
        }
        set_or_clear(&mut contour, "stepover", (passes > 1 && self.typed_stepover).then_some(self.stepover));
        if self.tabs > 0 && !self.inside() && passes == 1 {
            let mut tabs = json!({"count": self.tabs});
            set_or_clear(&mut tabs, "width", self.typed_tab_width.then_some(self.tab_width));
            set_or_clear(&mut tabs, "height", self.typed_tab_height.then_some(self.tab_height));
            contour["tabs"] = tabs;
        } else if let Some(kept) = contour.as_object_mut() {
            kept.remove("tabs");
        }
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
        self.placed.refresh(app, &setup, || manufacture_setup_preview(&app.session, "cam.setup.edit", &setup).map(Box::new));
        // Inside, the openings are looked for, and the tool follows the ones picked until it
        // is picked by hand. A contour being changed keeps its openings: nothing is looked for.
        if self.inside() && self.editing.is_none() {
            let id = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            if self.found.refresh(app, &setup, || manufacture_openings(&app.session, &id)) {
                (self.opening, self.suited_to) = (0, None);
            }
            self.opening = self.opening.min(self.choices().len().saturating_sub(1));
            if self.suited_to != Some(self.opening) {
                self.suited_to = Some(self.opening);
                let widest = self.picked().iter().map(|opening| opening.widest).fold(f64::INFINITY, f64::min);
                if !self.picked_tool && widest.is_finite() {
                    self.tool = self.fitting_tool(widest);
                }
            }
        }
        if !self.typed_bottom {
            // Where the cut ends unless typed: inside, where the openings picked end (a
            // contour being changed says where it ended when it was last worked out).
            let worked = || self.worked.get().and_then(|(_, preview)| preview.numbers.get("levels")?.as_array()?.last()?.as_f64());
            let found = match (self.inside(), &self.editing) {
                (true, None) => self.picked().first().map(|opening| opening.bottom),
                (true, Some(_)) => worked(),
                (false, _) => None,
            };
            if let Some(bottom) = found.or(self.placed.result.as_ref().ok().map(|placement| placement.model.min.z)) {
                self.bottom = bottom;
            }
        }
        if !self.typed_stepdown {
            self.stepdown = self.tools.get(self.tool).map_or(self.stepdown, |t| STEPDOWN_SHARE * t.diameter);
        }
        if !self.typed_lead {
            self.lead = self.tools.get(self.tool).map_or(self.lead, |t| LEAD_SHARE * t.diameter);
        }
        if !self.typed_stepover {
            self.stepover = self.tools.get(self.tool).map_or(self.stepover, |t| STEPOVER_SHARE * t.diameter);
        }
        self.passes = self.passes.clamp(1, MOST_PASSES);
        (self.leave, self.lead) = (self.leave.max(0.0), self.lead.max(0.0));
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
        let nothing_picked = self.inside() && self.editing.is_none() && self.picked().is_empty();
        if self.missing().is_some() || nothing_picked || self.worked.result.is_err() {
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
        let sides = labels(&SIDES);
        let inside = self.inside();
        let choice_labels = self.choice_labels(units, decimals);
        let choice_labels: Vec<&str> = choice_labels.iter().map(String::as_str).collect();
        // The openings of a contour being changed: how many it has.
        let kept_openings = self.editing.as_ref().map(|edited| {
            let count = edited.base["openings"].as_array().map_or(0, Vec::len).to_string();
            solidcraft_ui_text::format("manufacture.pocket_kept", &[("count", &count)])
        });
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok() && (!inside || self.editing.is_some() || !self.picked().is_empty());
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
                // Its side, and inside, its openings: a contour being changed stays on both.
                match &kept_openings {
                    Some(kept) => {
                        ui_kit::value_row(ui, tr("manufacture.contour_side"), sides.get(self.side).copied().unwrap_or_default());
                        if inside {
                            ui_kit::value_row(ui, tr("manufacture.contour_opening"), kept);
                        }
                    }
                    None => {
                        ui_kit::choice_row(ui, tr("manufacture.contour_side"), "manufacture_contour_side", &mut self.side, &sides);
                        if inside && choice_labels.is_empty() {
                            // Why no openings could be looked for, or that the part has none.
                            match self.found.trouble().filter(|why| !why.is_empty()) {
                                Some(why) => refusal(ui, Some(why)),
                                None => {
                                    ui_kit::text(ui, tr("manufacture.contour_no_openings"), ui_kit::Text::Note("manufacture_contour_no_openings"));
                                }
                            }
                            return;
                        }
                        if inside {
                            ui_kit::choice_row(
                                ui,
                                tr("manufacture.contour_opening"),
                                "manufacture_contour_opening",
                                &mut self.opening,
                                &choice_labels,
                            );
                        }
                    }
                }
                let tool = self.tool;
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_contour_tool", &mut self.tool, &tool_labels);
                self.picked_tool |= self.tool != tool;
                ui_kit::choice_row(ui, tr("manufacture.contour_direction"), "manufacture_contour_direction", &mut self.direction, &directions);
                if ui_kit::row(ui, tr("manufacture.contour_bottom"), |ui| ui_kit::length_field(ui, &mut self.bottom, units, decimals)).changed() {
                    self.typed_bottom = true;
                }
                if ui_kit::row(ui, tr("manufacture.face_stepdown"), |ui| ui_kit::length_field(ui, &mut self.stepdown, units, decimals)).changed() {
                    self.typed_stepdown = true;
                }
                ui_kit::row(ui, tr("manufacture.contour_leave"), |ui| ui_kit::length_field(ui, &mut self.leave, units, decimals));
                if ui_kit::row(ui, tr("manufacture.contour_lead"), |ui| ui_kit::length_field(ui, &mut self.lead, units, decimals)).changed() {
                    self.typed_lead = true;
                }
                // Passes sideways, from the outside in to the wall: the outside's.
                if !inside {
                    ui_kit::row(ui, tr("manufacture.contour_passes"), |ui| ui_kit::whole_number_field(ui, &mut self.passes, 1..=MOST_PASSES, ""));
                    if self.passes > 1
                        && ui_kit::row(ui, tr("manufacture.face_stepover"), |ui| ui_kit::length_field(ui, &mut self.stepover, units, decimals))
                            .changed()
                    {
                        self.typed_stepover = true;
                    }
                }
                // Tabs hold a part cut round its outside in one pass: there are none inside, and
                // none with passes, which cut away the stock a tab would hold on to.
                let tabbed = !inside && self.passes <= 1;
                if tabbed {
                    ui_kit::row(ui, tr("manufacture.contour_tabs"), |ui| ui_kit::whole_number_field(ui, &mut self.tabs, 0..=1000, ""));
                }
                if self.tabs > 0 && tabbed {
                    if ui_kit::row(ui, tr("manufacture.contour_tab_width"), |ui| ui_kit::length_field(ui, &mut self.tab_width, units, decimals))
                        .changed()
                    {
                        self.typed_tab_width = true;
                    }
                    if ui_kit::row(ui, tr("manufacture.contour_tab_height"), |ui| ui_kit::length_field(ui, &mut self.tab_height, units, decimals))
                        .changed()
                    {
                        self.typed_tab_height = true;
                    }
                }
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
                if inside {
                    ui_kit::text(ui, tr("manufacture.contour_inside_note"), ui_kit::Text::Note("manufacture_contour_note"));
                } else {
                    ui_kit::text(ui, tr("manufacture.contour_note"), ui_kit::Text::Note("manufacture_contour_note"));
                    if self.passes > 1 {
                        ui_kit::text(ui, tr("manufacture.contour_passes_note"), ui_kit::Text::Note("manufacture_contour_passes_note"));
                    }
                    // What holds a part that is cut all the way down: its tabs, or a word that
                    // nothing does. Down to a shoulder's floor the part stands where it was.
                    let all_the_way = self.placed.result.as_ref().is_ok_and(|placement| self.bottom <= placement.model.min.z + 1e-6);
                    if self.tabs > 0 && self.passes <= 1 {
                        ui_kit::text(ui, tr("manufacture.contour_tabs_note"), ui_kit::Text::Note("manufacture_contour_loose"));
                    } else if all_the_way {
                        ui_kit::text(ui, tr("manufacture.contour_loose"), ui_kit::Text::Note("manufacture_contour_loose"));
                    }
                }
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
