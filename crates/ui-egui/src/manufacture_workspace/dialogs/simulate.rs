//! Simulate's window (decision 0015 of the CAM work): the setup's operations cut from its stock
//! on a grid of heights, what was found, and the stock as it is left drawn on the model
//! (`super::super::scene`). It runs no command and changes nothing: `cam.simulate` gives a script
//! the same numbers.
use serde_json::json;
use solidcraft_engine::cmd::{Simulated, manufacture_simulation};
use solidcraft_engine::doc::display_units::format_length;
use solidcraft_ui_text::tr;

use super::{SetupChoice, Worked, imperial, refusal, setups};
use crate::SolveApp;
use crate::ui_kit;

/// Simulate: a setup's stock as its operations leave it.
pub(crate) struct SimForm {
    setups: Vec<SetupChoice>,
    pub setup: usize,
    /// The simulation of the setup picked.
    pub(super) worked: Worked<Simulated>,
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
        SimForm { setups, setup, worked: Worked::new(), drawn_setup: None }
    }

    /// The simulation, none while it could not be run.
    #[cfg(test)]
    pub fn simulated(&self) -> Option<&Simulated> {
        self.worked.get().map(|(_, simulated)| simulated)
    }

    /// Simulates the setup picked again when it or the design changed.
    fn refresh(&mut self, app: &SolveApp) {
        let setup = self.setups.get(self.setup).map_or("", |s| s.id.as_str()).to_string();
        if !setup.is_empty() {
            self.worked.refresh(app, &json!({"setup": setup}), || manufacture_simulation(&app.session, &setup, None));
        }
    }

    pub(super) fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<SimForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        // The setup picked is the one the viewport draws.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_simulate"), "inspect", tr("cmd.cam_simulate"))
            .placed(pivot, at)
            .fields()
            .actions("", tr("ui.close"), None)
            .show(ctx, true, |ui| {
                if self.setups.is_empty() {
                    ui_kit::text(ui, tr("manufacture.no_setup"), ui_kit::Text::Note("manufacture_sim_missing"));
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_sim_setup", &mut self.setup, &setup_labels);
                let Some(simulated) = self.worked.get().map(|(_, simulated)| simulated) else {
                    refusal(ui, self.worked.trouble());
                    return;
                };
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
                if stock.below_stock > stock.tolerance {
                    let said = solidcraft_ui_text::format("manufacture.sim_below", &[("depth", &format_length(stock.below_stock, units, decimals))]);
                    ui_kit::status_line(ui, &said, ui_kit::StatusTone::Warning);
                }
                ui_kit::text(ui, tr("manufacture.sim_legend"), ui_kit::Text::Note("manufacture_sim_legend"));
                ui_kit::status_line(ui, tr("manufacture.sim_note"), ui_kit::StatusTone::Warning);
            });
        (action == ui_kit::Action::None && !closed).then_some(self)
    }
}
