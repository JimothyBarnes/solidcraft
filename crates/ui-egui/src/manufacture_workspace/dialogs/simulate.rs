//! Simulate's window (decisions 0015 and 0023 of the CAM work): the setup's operations cut from
//! its stock on a grid of heights, what was found, and the stock as it is left drawn on the model
//! (`super::super::scene`): all the way, or as far along the program as its slider is, with the
//! tool where it is then; and played through. It runs no command and changes nothing:
//! `cam.simulate` gives a script the same numbers.
use serde_json::json;
use solidcraft_engine::cmd::{Playback, Simulated, ToolAt, manufacture_playback};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{SetupChoice, Worked, imperial, minutes_seconds, refusal, setups};
use crate::SolveApp;
use crate::ui_kit;

/// How long playing a setup's toolpaths through takes, seconds, whatever their length.
const PLAY_SECONDS: f64 = 20.0;
/// The longest one frame plays for, seconds: after a frame that took longer the tool does not jump.
const LONGEST_FRAME: f64 = 0.1;

/// Simulate: a setup's stock as its operations leave it, or as far as they have come.
pub(crate) struct SimForm {
    setups: Vec<SetupChoice>,
    pub setup: usize,
    /// The setup picked made ready to be cut: the model read and its toolpaths worked out, once.
    /// (Boxed, as what is shown is: the window is one of several forms kept in one enum.)
    prepared: Worked<Box<Playback>>,
    /// How far along the setup's toolpaths the stock is shown, mm: `None` is all the way, where
    /// the window starts.
    pub along: Option<f64>,
    /// Whether it is playing: `along` goes on by itself, frame by frame, to the end.
    pub playing: bool,
    /// The stock that far along, and where the tool is then.
    pub(super) worked: Worked<Box<(Simulated, Option<ToolAt>)>>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
}

/// A volume held in mm³ as a person reads it: cubic centimetres, or cubic inches in an inch design.
pub(super) fn volume(mm3: f64, units: &str) -> String {
    if imperial(units) { format!("{:.3} in³", mm3 / 16_387.064) } else { format!("{:.2} cm³", mm3 / 1000.0) }
}

impl SimForm {
    /// For the setup the viewport draws.
    pub fn new(app: &SolveApp) -> SimForm {
        let (setups, setup) = setups(app);
        SimForm { setups, setup, prepared: Worked::new(), along: None, playing: false, worked: Worked::new(), drawn_setup: None }
    }

    /// The simulation as far as it is shown, none while it could not be run.
    #[cfg(test)]
    pub fn simulated(&self) -> Option<&Simulated> {
        self.worked.get().map(|(_, shown)| &shown.0)
    }

    /// Where the tool is, when the stock is shown part of the way.
    #[cfg(test)]
    pub fn tool(&self) -> Option<&ToolAt> {
        self.worked.get().and_then(|(_, shown)| shown.1.as_ref())
    }

    /// How long the setup's toolpaths are together, mm: 0 while they could not be worked out.
    pub fn length(&self) -> f64 {
        self.prepared.get().map_or(0.0, |(_, playback)| playback.length)
    }

    /// Starts playing, from the start when the stock is shown all the way; or stops.
    pub fn play(&mut self, on: bool) {
        if on && self.along.is_none() {
            self.along = Some(0.0);
        }
        self.playing = on && self.length() > 0.0;
    }

    /// Makes the setup picked ready again when it or the design changed, and cuts it as far as
    /// it is shown when that or the place changed.
    fn refresh(&mut self, app: &SolveApp) {
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        if setup.is_empty() {
            return;
        }
        if self.prepared.refresh(app, &json!({"setup": setup}), || manufacture_playback(&app.session, &setup, None).map(Box::new)) {
            // Another setup, or another design: all the way again.
            (self.along, self.playing) = (None, false);
        }
        let (prepared, along) = (&self.prepared, self.along);
        self.worked.refresh(app, &json!({"setup": setup, "along": along}), || match &prepared.result {
            Ok(playback) => playback.at(along.unwrap_or(f64::INFINITY)).map(Box::new),
            Err(why) => Err(why.clone()),
        });
    }

    /// One frame of playing: on by the frame's share of [`PLAY_SECONDS`], and all the way and
    /// stopped at the end.
    fn advance(&mut self, ctx: &egui::Context) {
        let length = self.length();
        if !self.playing || !(length > 0.0) {
            self.playing = false;
            return;
        }
        let frame = f64::from(ctx.input(|input| input.stable_dt)).clamp(0.0, LONGEST_FRAME);
        let next = self.along.unwrap_or(length) + frame * length / PLAY_SECONDS;
        if next >= length {
            (self.along, self.playing) = (None, false);
        } else {
            self.along = Some(next);
        }
        ctx.request_repaint();
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<SimForm> {
        self.advance(ctx);
        self.refresh(app);
        let length = self.length();
        // Where the tool is, in words: its number, the operation it is in and the time cut in it.
        let tool_words = self.worked.get().map(|(_, shown)| match &shown.1 {
            Some(tool) => {
                let operation = shown.0.operations.get(tool.operation).map_or("", |operation| operation.name.as_str());
                let (number, time) = (tool.tool.number.to_string(), minutes_seconds(tool.minutes));
                solidcraft_ui_text::format("manufacture.sim_tool_value", &[("tool", &number), ("operation", operation), ("time", &time)])
            }
            None => tr("manufacture.sim_tool_done").to_string(),
        });
        let (mut shown_to, mut play) = (self.along.unwrap_or(length), None);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        // The setup picked is the one the viewport draws.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_simulate"), "cam_simulate", tr("cmd.cam_simulate"))
            .placed(pivot, at)
            .fields()
            .actions("", tr("ui.close"), None)
            .show(ctx, true, |ui| {
                if self.setups.is_empty() {
                    ui_kit::text(ui, tr("manufacture.no_setup"), ui_kit::Text::Note("manufacture_sim_missing"));
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_sim_setup", &mut self.setup, &setup_labels);
                let Some(simulated) = self.worked.get().map(|(_, shown)| &shown.0) else {
                    refusal(ui, self.worked.trouble());
                    return;
                };
                // How far along the program the stock is shown, and playing it through.
                if length > 0.0 {
                    // Play or Pause, and beside it the slider, as wide as what is left of the row.
                    let slider = ui_kit::Slider { show_value: false, ..Default::default() };
                    let (label, icon) = if self.playing { ("manufacture.sim_pause", "stop") } else { ("manufacture.sim_play", "play") };
                    let mut pressed = false;
                    let moved = ui_kit::row(ui, tr("manufacture.sim_position"), |ui| {
                        ui.horizontal(|ui| {
                            pressed = ui_kit::icon_label_button(ui, tr(label), icon, true, false).clicked();
                            ui_kit::slider(ui, &mut shown_to, 0.0..=length, slider)
                        })
                        .inner
                    })
                    .changed();
                    if moved {
                        play = Some(false);
                    } else {
                        shown_to = f64::NAN;
                    }
                    if pressed {
                        play = Some(!self.playing);
                    }
                    if let Some(words) = &tool_words {
                        ui_kit::value_row(ui, tr("manufacture.sim_tool"), words);
                    }
                }
                let stock = &simulated.stock;
                let (over, _) = stock.leftover();
                let cell = solidcraft_ui_text::format("manufacture.sim_grid_value", &[("cell", &format_length(stock.stock.cell, units, decimals))]);
                ui_kit::value_row(ui, tr("manufacture.sim_grid"), &cell);
                ui_kit::value_row(ui, tr("manufacture.sim_stock"), &volume(simulated.stock_volume, units));
                ui_kit::value_row(ui, tr("manufacture.sim_removed"), &volume(stock.removed, units));
                ui_kit::value_row(ui, tr("manufacture.sim_left"), &volume(over, units));
                for operation in &simulated.operations {
                    match &operation.skipped {
                        None => {
                            ui_kit::value_row(ui, &operation.name, &volume(operation.removed, units));
                        }
                        Some(why) => {
                            let said = solidcraft_ui_text::format("manufacture.sim_skipped", &[("operation", &operation.name), ("why", why)]);
                            ui_kit::status_line(ui, &said, ui_kit::StatusTone::Warning);
                        }
                    }
                }
                let name_of = |index: usize| simulated.operations.get(index).map_or("", |operation| operation.name.as_str());
                match stock.gouges.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                    None => {
                        ui_kit::status_line(ui, tr("manufacture.sim_no_gouge"), ui_kit::StatusTone::Dim);
                    }
                    Some(worst) => {
                        let (count, depth) = (stock.gouge_count.to_string(), format_length(worst.depth, units, decimals));
                        let said = solidcraft_ui_text::format(
                            "manufacture.sim_gouges",
                            &[("count", &count), ("depth", &depth), ("operation", name_of(worst.operation))],
                        );
                        ui_kit::status_line(ui, &said, ui_kit::StatusTone::Error);
                    }
                }
                match stock.rapid_hits.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                    None => {
                        ui_kit::status_line(ui, tr("manufacture.sim_no_rapid"), ui_kit::StatusTone::Dim);
                    }
                    Some(worst) => {
                        let (count, depth) = (stock.rapid_hit_count.to_string(), format_length(worst.depth, units, decimals));
                        let said = solidcraft_ui_text::format(
                            "manufacture.sim_rapids",
                            &[("count", &count), ("depth", &depth), ("operation", name_of(worst.operation))],
                        );
                        ui_kit::status_line(ui, &said, ui_kit::StatusTone::Error);
                    }
                }
                match stock.body_hits.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                    None => {
                        ui_kit::status_line(ui, tr("manufacture.sim_no_body"), ui_kit::StatusTone::Dim);
                    }
                    Some(worst) => {
                        let (count, depth) = (stock.body_hit_count.to_string(), format_length(worst.depth, units, decimals));
                        let part = tr(match worst.part {
                            solidcraft_engine::cmd::BodyPart::Shank => "manufacture.sim_part_shank",
                            solidcraft_engine::cmd::BodyPart::Holder => "manufacture.sim_part_holder",
                        });
                        let said = solidcraft_ui_text::format(
                            "manufacture.sim_body",
                            &[("count", &count), ("depth", &depth), ("part", part), ("operation", name_of(worst.operation))],
                        );
                        ui_kit::status_line(ui, &said, ui_kit::StatusTone::Error);
                    }
                }
                if stock.below_stock > stock.tolerance {
                    let said = solidcraft_ui_text::format("manufacture.sim_below", &[("depth", &format_length(stock.below_stock, units, decimals))]);
                    ui_kit::status_line(ui, &said, ui_kit::StatusTone::Warning);
                }
                ui_kit::text(ui, tr("manufacture.sim_legend"), ui_kit::Text::Note("manufacture_sim_legend"));
                ui_kit::status_line(ui, tr("manufacture.sim_note"), ui_kit::StatusTone::Warning);
            });
        // The slider moved: shown there (all the way at its end), and not playing.
        if !shown_to.is_nan() && length > 0.0 {
            self.along = (shown_to < length).then_some(shown_to.max(0.0));
        }
        if let Some(on) = play {
            self.play(on);
            ctx.request_repaint();
        }
        (action == ui_kit::Action::None && !closed).then_some(self)
    }
}
