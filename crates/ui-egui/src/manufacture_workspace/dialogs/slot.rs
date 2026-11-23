//! Slot's form (decision 0026 of the CAM work): the slots found on the model, milled along
//! their middle: all of them unless one is picked. It builds the parameters of `cam.op.slot` (of
//! `cam.op.edit` for a slot operation being changed) and shows what they would cut before OK, as
//! the other operations' forms do.
use serde_json::{Value, json};
use solidcraft_engine::cmd::{FoundSlot, OperationPreview, ToolChoice, manufacture_preview, manufacture_slots, manufacture_tools};
use solidcraft_engine::doc::display_units::{format_length, length_number};
use solidcraft_ui_text::tr;

use super::{
    Edited, SetupChoice, Worked, kept_operation, minutes_seconds, preview_note, refusal, set_or_clear, setups, speed_and_feed, window_words,
};
use crate::SolveApp;
use crate::ui_kit;

/// The stepdown a slot takes when none is typed, as a share of the tool's diameter (the engine's
/// own default, shown in the field).
const STEPDOWN_SHARE: f64 = 0.5;

/// Slot: slots found on the model, each milled along its middle.
pub(crate) struct SlotForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// The slots the setup picked has, as the engine found them.
    found: Worked<Vec<FoundSlot>>,
    /// Which of the form's choices is picked: with more than one slot found the first choice is
    /// all of them, and the slots follow it one by one.
    pub slot: usize,
    /// The choice the tool was last suited to.
    suited_to: Option<usize>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then it is the widest that goes into every
    /// slot picked.
    pub picked_tool: bool,
    /// The Z the cut ends at: where the slots picked end, when they end at one height, until it
    /// is typed (a little under the part, to break through).
    pub bottom: f64,
    pub typed_bottom: bool,
    /// mm: half the tool's diameter until it is typed.
    pub stepdown: f64,
    pub typed_stepdown: bool,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material, until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The slotting these parameters would make.
    pub(super) worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    /// The slot operation being changed, when the form changes one: its `slot` parameters as
    /// kept. Its slots stay the ones it has.
    pub(super) editing: Option<Edited>,
    pub error: Option<String>,
}

impl SlotForm {
    /// For the setup the viewport draws, all its slots and the widest tool that goes into them.
    pub fn new(app: &SolveApp) -> SlotForm {
        let (setups, setup) = setups(app);
        SlotForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "slot"),
            setup,
            found: Worked::new(),
            slot: 0,
            suited_to: None,
            tool: 0,
            picked_tool: false,
            bottom: 0.0,
            typed_bottom: false,
            stepdown: 0.0,
            typed_stepdown: false,
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

    /// The form for changing the slot operation `id`, on the slots it has; why not, when the
    /// design has no such slot operation.
    pub fn edit(app: &SolveApp, id: &str) -> Result<SlotForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "slot" {
            return Err(format!("operation {id} is not a slot operation"));
        }
        let number = |key: &str| base[key].as_f64();
        let mut form = SlotForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        (form.tool, form.picked_tool) = (form.tools.iter().position(|t| t.number == tool).unwrap_or(0), true);
        (form.bottom, form.typed_bottom) = number("bottom").map_or((0.0, false), |v| (v, true));
        (form.stepdown, form.typed_stepdown) = number("stepdown").map_or((0.0, false), |v| (v, true));
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
    }

    /// The slots found, none while they could not be.
    pub fn slots(&self) -> &[FoundSlot] {
        self.found.get().map_or(&[], |(_, found)| found.as_slice())
    }

    /// Whether the choices start with all the slots at once: when more than one was found.
    fn offers_all(&self) -> bool {
        self.slots().len() > 1
    }

    /// The slots the choice picked stands for: all of them, or one.
    pub fn picked(&self) -> &[FoundSlot] {
        let all = self.slots();
        let one = |at: usize| all.get(at..at.saturating_add(1)).unwrap_or(&[]);
        match (self.offers_all(), self.slot) {
            (true, 0) => all,
            (true, choice) => one(choice - 1),
            (false, choice) => one(choice),
        }
    }

    /// Where the slots to cut end, when they all end at one height: the slots picked as they
    /// were found, or, for an operation being changed, as it was last worked out.
    fn common_bottom(&self) -> Option<f64> {
        let mut bottoms: Vec<f64> = match &self.editing {
            None => self.picked().iter().map(|slot| slot.bottom).collect(),
            Some(_) => {
                let (_, preview) = self.worked.get()?;
                preview.numbers.get("levels")?.as_array()?.iter().filter_map(|levels| levels.as_array()?.last()?.as_f64()).collect()
            }
        };
        let first = bottoms.pop()?;
        bottoms.iter().all(|other| (other - first).abs() <= 1e-6).then_some(first)
    }

    /// What is missing before a slot can be cut, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_contour"))
        } else if self.editing.is_none() && self.found.result.as_ref().is_ok_and(Vec::is_empty) {
            Some(tr("manufacture.slot_none"))
        } else {
            None
        }
    }

    /// The tool that suits `slots`: the widest that is no wider than the narrowest of them, so
    /// that it goes into every one; else the narrowest there is, which then says why it does not.
    fn fitting_tool(&self, slots: &[FoundSlot]) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let narrowest = slots.iter().map(|slot| slot.width).fold(f64::INFINITY, f64::min);
        let fits = self.tools.iter().enumerate().filter(|(_, tool)| tool.diameter <= narrowest + 1e-6).max_by(by_width);
        fits.or(self.tools.iter().enumerate().min_by(by_width)).map_or(0, |(at, _)| at)
    }

    /// Whether the tool picked is wider than one of the slots picked.
    pub fn tool_too_wide(&self) -> bool {
        let too_wide = |tool: &ToolChoice| self.picked().iter().any(|slot| tool.diameter > slot.width + 1e-6);
        self.editing.is_none() && self.tools.get(self.tool).is_some_and(too_wide)
    }

    /// The `cam.op.slot` parameters: the point in each slot picked, and what was typed. What was
    /// not typed is left to the command, so it keeps following the tool. A slot operation being
    /// changed keeps its own slots and the rest as kept.
    pub fn params(&self) -> Value {
        let mut slot = match &self.editing {
            Some(edited) => edited.base.clone(),
            None => json!({"slots": self.picked().iter().map(|found| found.at).collect::<Vec<_>>()}),
        };
        set_or_clear(&mut slot, "bottom", self.typed_bottom.then_some(self.bottom));
        set_or_clear(&mut slot, "stepdown", self.typed_stepdown.then_some(self.stepdown));
        set_or_clear(&mut slot, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut slot, "feed", self.typed_feed.then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "slot": slot,
        })
    }

    /// Finds the setup's slots and works the slotting out, when the design or a field changed,
    /// and shows what was not typed as the slots and the tool give it.
    fn refresh(&mut self, app: &SolveApp) {
        // An operation being changed keeps its slots: nothing is looked for or suited to them.
        if self.editing.is_none() {
            let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
            if self.found.refresh(app, &json!({"id": setup}), || manufacture_slots(&app.session, &setup)) {
                self.suited_to = None;
            }
            let choices = self.slots().len() + usize::from(self.offers_all());
            self.slot = self.slot.min(choices.saturating_sub(1));
            if self.suited_to != Some(self.slot) && !self.picked().is_empty() {
                // Other slots: their own bottom again, and the tool that suits them.
                self.typed_bottom = false;
                if !self.picked_tool {
                    self.tool = self.fitting_tool(self.picked());
                }
                self.suited_to = Some(self.slot);
            }
        }
        if !self.typed_bottom
            && let Some(bottom) = self.common_bottom()
        {
            self.bottom = bottom;
        }
        if !self.typed_stepdown {
            self.stepdown = self.tools.get(self.tool).map_or(self.stepdown, |tool| STEPDOWN_SHARE * tool.diameter);
        }
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.slot", &params)) {
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

    /// A slot found, as the form offers it: its width, its length from end to end and its depth
    /// below the part's top ("10.00 × 50.00 × 6.00 mm"), or that it goes through the part.
    fn label(found: &FoundSlot, units: &str, decimals: u8) -> String {
        let width = length_number(found.width, units, decimals);
        let whole = found.length + found.width;
        if found.through {
            let length = format_length(whole, units, decimals);
            solidcraft_ui_text::format("manufacture.slot_label_through", &[("width", &width), ("length", &length)])
        } else {
            let (length, depth) = (length_number(whole, units, decimals), format_length(found.depth, units, decimals));
            solidcraft_ui_text::format("manufacture.slot_label", &[("width", &width), ("length", &length), ("depth", &depth)])
        }
    }

    /// The levels a slot would be cut in, as the form says them: for one slot, since each of
    /// several has its own.
    fn worked_out(&self, units: &str, decimals: u8) -> Option<String> {
        let (_, preview) = self.worked.get()?;
        let levels = preview.numbers.get("levels")?.as_array().filter(|each| each.len() == 1)?.first()?.as_array()?;
        let (top, bottom) = (preview.numbers.get("top")?.as_f64()?, levels.last()?.as_f64()?);
        let each = format_length((top - bottom) / levels.len().max(1) as f64, units, decimals);
        Some(solidcraft_ui_text::format("manufacture.contour_levels_value", &[("count", &levels.len().to_string()), ("depth", &each)]))
    }

    /// Runs the command: the form again, with the reason, when the slotting can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<SlotForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "slot": params["slot"].take()})),
            None => app.run("cam.op.slot", params),
        };
        match made {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<SlotForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        // All of them first, when there is more than one; then each by its size.
        let all = solidcraft_ui_text::format("manufacture.slot_all", &[("count", &self.slots().len().to_string())]);
        let slot_labels: Vec<String> =
            self.offers_all().then_some(all).into_iter().chain(self.slots().iter().map(|found| Self::label(found, units, decimals))).collect();
        let slot_labels: Vec<&str> = slot_labels.iter().map(String::as_str).collect();
        let kept = self.editing.as_ref().map(|edited| {
            let count = edited.base["slots"].as_array().map_or(0, Vec::len).to_string();
            solidcraft_ui_text::format("manufacture.slot_kept", &[("count", &count)])
        });
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        let levels = self.worked_out(units, decimals);
        let too_wide = self.tool_too_wide();
        // One bottom to show: slots that end at different heights each keep their own.
        let one_bottom = self.typed_bottom || self.common_bottom().is_some();
        // The setup picked is the one the viewport draws, so the slot is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_slot");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_slot"), "edit", &title)
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
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_slot_setup", &mut self.setup, &setup_labels);
                    }
                    _ => {}
                }
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_slot_missing"));
                    return;
                }
                match &kept {
                    Some(kept) => {
                        ui_kit::value_row(ui, tr("manufacture.slot_choice"), kept);
                    }
                    None => {
                        ui_kit::choice_row(ui, tr("manufacture.slot_choice"), "manufacture_slot_choice", &mut self.slot, &slot_labels);
                    }
                }
                if ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_slot_tool", &mut self.tool, &tool_labels).changed() {
                    self.picked_tool = true;
                }
                if too_wide {
                    ui_kit::text(ui, tr("manufacture.slot_tool_wide"), ui_kit::Text::Note("manufacture_slot_tool_wide"));
                }
                if one_bottom
                    && ui_kit::row(ui, tr("manufacture.contour_bottom"), |ui| ui_kit::length_field(ui, &mut self.bottom, units, decimals)).changed()
                {
                    self.typed_bottom = true;
                }
                if ui_kit::row(ui, tr("manufacture.face_stepdown"), |ui| ui_kit::length_field(ui, &mut self.stepdown, units, decimals)).changed() {
                    self.typed_stepdown = true;
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
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_slot_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.slot_note"), ui_kit::Text::Note("manufacture_slot_note"));
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_slot_preview_note"));
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
