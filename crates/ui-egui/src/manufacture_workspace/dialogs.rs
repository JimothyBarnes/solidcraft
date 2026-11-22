//! Manufacture's forms: Add Tool, New Setup, Face, Drill and Post Process. Each is a front end that
//! builds the parameters of one `cam.*` command and runs it: every change is a command, and a
//! script calls the command directly and sees no window. They are standalone windows, opened
//! when the ribbon starts one of those commands ([`start`]) and drawn once a frame ([`show`]).
//!
//! New Setup, Face and Drill show what they would make before OK: the engine works it out from the
//! parameters as they are typed (`manufacture_setup_preview`, `manufacture_preview`), the viewport
//! draws it (`super::scene`), and the form says why when there would be nothing to make. A form
//! also stays open with the command's own words when the command refuses, so the reason is
//! beside the fields that fix it.
//!
//! Numbers are held in mm, mm/min, m/min and degrees, as the commands take them, and shown in
//! the design's units.
use std::hash::{Hash, Hasher};

use serde_json::{Value, json};
use solidcraft_engine::cmd::{
    HoleBottom, HoleGroup, OperationPreview, Placement, ToolChoice, manufacture_data, manufacture_holes, manufacture_posts, manufacture_preview,
    manufacture_setup_preview, manufacture_tools,
};
use solidcraft_engine::doc::display_units::{format_length, length_number, unit_scale};
use solidcraft_ui_text::{tr, tr_label};

use crate::SolveApp;
use crate::ui_kit;

/// The open form.
pub(crate) enum Dialog {
    Tool(ToolForm),
    Setup(SetupForm),
    Face(FaceForm),
    Drill(DrillForm),
    Post(PostForm),
    Posted(Posted),
}

/// What an open form adds to the viewport.
pub(crate) enum Shown<'a> {
    /// A setup not made yet: its stock and work origin, in place of the active setup's.
    Setup(&'a Placement),
    /// An operation not made yet: its toolpath, with the active setup.
    Operation(&'a OperationPreview),
}

impl Dialog {
    /// What the form adds to the viewport, with a number that changes when that does.
    pub(crate) fn shown(&self) -> Option<(u64, Shown<'_>)> {
        match self {
            Dialog::Setup(form) => form.placed.get().map(|(key, placement)| (key, Shown::Setup(placement))),
            Dialog::Face(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            Dialog::Drill(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            _ => None,
        }
    }
}

/// The tool kinds Add Tool offers: the command's word and the label.
const KINDS: [(&str, &str); 7] = [
    ("flat_end_mill", "manufacture.kind_flat_end_mill"),
    ("face_mill", "manufacture.kind_face_mill"),
    ("bull_nose_end_mill", "manufacture.kind_bull_nose_end_mill"),
    ("ball_end_mill", "manufacture.kind_ball_end_mill"),
    ("drill", "manufacture.kind_drill"),
    ("spot_drill", "manufacture.kind_spot_drill"),
    ("reamer", "manufacture.kind_reamer"),
];
const COOLANTS: [(&str, &str); 3] =
    [("flood", "manufacture.coolant_flood"), ("mist", "manufacture.coolant_mist"), ("off", "manufacture.coolant_off")];
/// Where New Setup can put the work origin on the stock: the label and the anchor's x, y, z.
const ORIGINS: [(&str, [&str; 3]); 5] = [
    ("manufacture.origin_top_center", ["center", "center", "max"]),
    ("manufacture.origin_top_front_left", ["min", "min", "max"]),
    ("manufacture.origin_top_front_right", ["max", "min", "max"]),
    ("manufacture.origin_top_back_left", ["min", "max", "max"]),
    ("manufacture.origin_top_back_right", ["max", "max", "max"]),
];
/// The drilling cycles Drill offers: the command's word and the label.
const CYCLES: [(&str, &str); 3] =
    [("drill", "manufacture.cycle_drill"), ("peck", "manufacture.cycle_peck"), ("chip_break", "manufacture.cycle_chip_break")];
/// How far past the part a through hole is drilled, at the drill's full diameter, mm.
const BREAKTHROUGH: f64 = 1.0;
const PATTERNS: [(&str, &str); 2] = [("zig_zag", "manufacture.pattern_zig_zag"), ("one_way", "manufacture.pattern_one_way")];
const UNITS: [(&str, &str); 2] = [("mm", "manufacture.units_mm"), ("inch", "manufacture.units_inch")];
const WORK_OFFSETS: [&str; 6] = ["G54", "G55", "G56", "G57", "G58", "G59"];
/// The file kinds a program is saved as.
const PROGRAM_EXTENSIONS: [&str; 4] = ["nc", "tap", "gcode", "txt"];
/// The fastest spindle and feed a field takes; the commands have the last word.
const MAX_RPM: f64 = 1_000_000.0;
const MAX_FEED: f64 = 1_000_000.0;

/// The command's word of a choice, the first when the index is out of range.
fn word(choices: &[(&'static str, &'static str)], index: usize) -> &'static str {
    choices.get(index).or(choices.first()).map_or("", |c| c.0)
}

fn labels(choices: &[(&'static str, &'static str)]) -> Vec<&'static str> {
    choices.iter().map(|c| tr(c.1)).collect()
}

/// Whether the design's unit is the inch or the foot: speeds then read in feet a minute, and a
/// new form starts from round inch sizes.
fn imperial(units: &str) -> bool {
    let per_mm = unit_scale(units).0;
    [25.4, 304.8].iter().any(|mm| (per_mm * mm - 1.0).abs() < 1e-9)
}

/// A length as a name says it, in the design's units: "10 mm", "0.375 in".
fn plain_length(mm: f64, units: &str) -> String {
    let shown = format_length(mm, units, 4);
    match shown.split_once(' ') {
        Some((number, unit)) if number.contains('.') => format!("{} {unit}", number.trim_end_matches('0').trim_end_matches('.')),
        _ => shown,
    }
}

/// A number held in one unit and typed in another (`shown = held * per`). The field is given
/// back the number it set while that still stands for the held one, so a round trip that lands a
/// hair off never rewrites what is being typed (as `ui_kit::angle_field` does for radians).
fn scaled_field(ui: &mut egui::Ui, held: &mut f64, per: f64, most: f64, suffix: &str) -> egui::Response {
    let memory = ui.next_auto_id().with("manufacture_scaled_field");
    let mut shown = *held * per;
    if let Some(last) = ui.data(|d| d.get_temp::<f64>(memory)).filter(|last| (last - shown).abs() <= 1e-9 * last.abs().max(1.0)) {
        shown = last;
    }
    let response = ui_kit::number_field(ui, &mut shown, 0.0..=most * per, 1.0, suffix);
    if response.changed() {
        *held = shown / per;
        ui.data_mut(|d| d.insert_temp(memory, shown));
    }
    response
}

/// A feed held in mm/min, typed in the design's unit a minute.
fn feed_field(ui: &mut egui::Ui, feed: &mut f64, units: &str) -> egui::Response {
    let (per, unit) = unit_scale(units);
    scaled_field(ui, feed, per, MAX_FEED, &format!(" {unit}/min"))
}

/// A surface speed held in m/min, typed in feet a minute where the design is in inches or feet.
fn surface_speed_field(ui: &mut egui::Ui, speed: &mut f64, units: &str) -> egui::Response {
    if imperial(units) { scaled_field(ui, speed, 1.0 / 0.3048, 10_000.0, " ft/min") } else { scaled_field(ui, speed, 1.0, 10_000.0, " m/min") }
}

/// An operation's spindle speed and cutting feed rows; whether each was typed just now.
fn speed_and_feed(ui: &mut egui::Ui, rpm: &mut f64, feed: &mut f64, units: &str) -> (bool, bool) {
    let rpm = ui_kit::row(ui, tr("manufacture.face_spindle_speed"), |ui| ui_kit::number_field(ui, rpm, 0.0..=MAX_RPM, 10.0, " rpm")).changed();
    let feed = ui_kit::row(ui, tr("manufacture.face_feed"), |ui| feed_field(ui, feed, units)).changed();
    (rpm, feed)
}

/// A command's refusal, or why a preview has nothing to show, in the form.
fn refusal(ui: &mut egui::Ui, why: Option<&str>) {
    if let Some(why) = why.filter(|why| !why.is_empty()) {
        ui_kit::text(ui, why, ui_kit::Text::Error("manufacture_refusal"));
    }
}

/// What a form's parameters would make, worked out again when they or the design change.
struct Worked<T> {
    key: Option<u64>,
    result: Result<T, String>,
}

impl<T> Worked<T> {
    fn new() -> Worked<T> {
        Worked { key: None, result: Err(String::new()) }
    }

    /// Works it out with `make` if `params` or the design changed since the last time; whether
    /// it did.
    fn refresh(&mut self, app: &SolveApp, params: &Value, make: impl FnOnce() -> Result<T, String>) -> bool {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        app.session.uid.hash(&mut h);
        app.session.revision.hash(&mut h);
        params.to_string().hash(&mut h);
        let key = h.finish();
        if self.key == Some(key) {
            return false;
        }
        self.key = Some(key);
        self.result = make();
        true
    }

    /// What was worked out, with a number that changes when it does.
    fn get(&self) -> Option<(u64, &T)> {
        Some((self.key?, self.result.as_ref().ok()?))
    }

    fn trouble(&self) -> Option<&str> {
        self.result.as_ref().err().map(String::as_str)
    }
}

/// One of the design's setups for a form to offer.
struct SetupChoice {
    id: String,
    name: String,
    /// The name with its work offset, as the Browser shows it.
    label: String,
}

/// The design's setups, and the one the viewport draws.
fn setups(app: &SolveApp) -> (Vec<SetupChoice>, usize) {
    let Ok(data) = manufacture_data(&app.session.doc) else { return (Vec::new(), 0) };
    let list: Vec<SetupChoice> = data
        .setups
        .iter()
        .map(|s| SetupChoice { id: s.id.clone(), name: s.name.clone(), label: format!("{} (G{})", s.name, 53 + u32::from(s.work_offset)) })
        .collect();
    let active = app.manufacture_workspace.active_setup.as_deref();
    let at = list.iter().position(|s| Some(s.id.as_str()) == active).unwrap_or(0);
    (list, at)
}

/// Add Tool: one of the design's tools.
pub(crate) struct ToolForm {
    pub number: u32,
    /// The name typed; empty for the diameter and the kind ("10 mm Flat end mill").
    pub name: String,
    pub kind: usize,
    /// Lengths in mm.
    pub diameter: f64,
    pub corner_radius: f64,
    pub flutes: u32,
    typed_flutes: bool,
    pub flute_length: f64,
    pub overall_length: f64,
    /// Degrees, for a drill or a spot drill.
    pub tip_angle: f64,
    pub coolant: usize,
    /// The material the cutting data is for; no cutting data when empty.
    pub material: String,
    /// m/min.
    pub surface_speed: f64,
    /// mm a tooth.
    pub chip_load: f64,
    pub error: Option<String>,
}

impl ToolForm {
    /// A flat end mill at the first free tool number: 10 mm, or half an inch in an inch design.
    pub fn new(app: &SolveApp) -> ToolForm {
        let used: Vec<u32> = manufacture_data(&app.session.doc).map(|d| d.tools.iter().map(|t| t.number).collect()).unwrap_or_default();
        let number = (1..=99_999).find(|n| !used.contains(n)).unwrap_or(1);
        let inch = imperial(&app.session.doc.units);
        let (diameter, corner_radius, flute_length, overall_length) = if inch { (12.7, 0.79375, 31.75, 76.2) } else { (10.0, 1.0, 22.0, 72.0) };
        ToolForm {
            number,
            name: String::new(),
            kind: 0,
            diameter,
            corner_radius,
            flutes: 3,
            typed_flutes: false,
            flute_length,
            overall_length,
            tip_angle: 118.0,
            coolant: 0,
            material: String::new(),
            // 650 ft/min and 0.002 in a tooth, or 200 m/min and 0.05 mm.
            surface_speed: if inch { 198.12 } else { 200.0 },
            chip_load: if inch { 0.0508 } else { 0.05 },
            error: None,
        }
    }

    fn pointed(&self) -> bool {
        matches!(word(&KINDS, self.kind), "drill" | "spot_drill")
    }

    /// The flutes a tool of this kind usually has, until the number is typed.
    fn usual_flutes(&self) -> u32 {
        match word(&KINDS, self.kind) {
            "drill" | "spot_drill" => 2,
            "face_mill" => 5,
            "reamer" => 6,
            _ => 3,
        }
    }

    /// The name the tool gets when none is typed: its diameter in the design's `units` and its kind.
    fn usual_name(&self, units: &str) -> String {
        format!("{} {}", plain_length(self.diameter, units), labels(&KINDS).get(self.kind).copied().unwrap_or(""))
    }

    /// The `cam.tool.create` parameters, the tool named in the design's `units` when no name is typed.
    pub fn params(&self, units: &str) -> Value {
        let kind = word(&KINDS, self.kind);
        let corner_radius = match kind {
            "bull_nose_end_mill" => self.corner_radius,
            "ball_end_mill" => 0.5 * self.diameter,
            _ => 0.0,
        };
        let name = match self.name.trim() {
            "" => self.usual_name(units),
            typed => typed.to_string(),
        };
        let cutting_data = match self.material.trim() {
            "" => json!([]),
            material => {
                json!([{"material": material, "surface_speed": self.surface_speed, "chip_load": self.chip_load, "plunge_factor": 0.5, "ramp_factor": 0.5}])
            }
        };
        json!({"tool": {
            "number": self.number, "name": name, "kind": kind,
            "geometry": {
                "diameter": self.diameter, "corner_radius": corner_radius, "flute_length": self.flute_length, "shoulder_length": 0.0,
                "overall_length": self.overall_length, "shank_diameter": self.diameter,
                "tip_angle": if self.pointed() { self.tip_angle.to_radians() } else { 0.0 }, "tip_diameter": 0.0, "thread_pitch": 0.0
            },
            "flutes": self.flutes,
            "holder": {"stick_out": self.overall_length.min(self.flute_length + 10.0), "segments": []},
            "length_offset": self.number, "diameter_offset": self.number,
            "coolant": word(&COOLANTS, self.coolant),
            "cutting_data": cutting_data,
            "overrides": {"rpm": null, "feed": null}
        }})
    }

    /// Runs the command: the form again, with the reason, when it refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ToolForm> {
        let params = self.params(&app.session.doc.units);
        match app.run("cam.tool.create", params) {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<ToolForm> {
        let (kinds, coolants) = (labels(&KINDS), labels(&COOLANTS));
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_tool"), "library", tr("cmd.cam_tool_create"))
            .placed(pivot, at)
            .fields()
            .actions(tr("ui.create"), tr("ui.cancel"), None)
            .show(ctx, true, |ui| {
                ui_kit::row(ui, tr("manufacture.tool_number"), |ui| ui_kit::whole_number_field(ui, &mut self.number, 1..=99_999, ""));
                let usual_name = self.usual_name(units);
                ui_kit::text_row(ui, tr_label("Name"), &mut self.name, &usual_name);
                let kind = self.kind;
                ui_kit::choice_row(ui, tr_label("Type"), "manufacture_tool_kind", &mut self.kind, &kinds);
                if self.kind != kind && !self.typed_flutes {
                    self.flutes = self.usual_flutes();
                }
                ui_kit::row(ui, tr("ui.diameter"), |ui| ui_kit::length_field(ui, &mut self.diameter, units, decimals));
                if word(&KINDS, self.kind) == "bull_nose_end_mill" {
                    ui_kit::row(ui, tr("manufacture.tool_corner_radius"), |ui| ui_kit::length_field(ui, &mut self.corner_radius, units, decimals));
                }
                if self.pointed() {
                    ui_kit::row(ui, tr("manufacture.tool_tip_angle"), |ui| ui_kit::angle_field(ui, &mut self.tip_angle));
                }
                if ui_kit::row(ui, tr("manufacture.tool_flutes"), |ui| ui_kit::whole_number_field(ui, &mut self.flutes, 1..=1000, "")).changed() {
                    self.typed_flutes = true;
                }
                ui_kit::row(ui, tr("manufacture.tool_flute_length"), |ui| ui_kit::length_field(ui, &mut self.flute_length, units, decimals));
                ui_kit::row(ui, tr("manufacture.tool_overall_length"), |ui| ui_kit::length_field(ui, &mut self.overall_length, units, decimals));
                ui_kit::choice_row(ui, tr("manufacture.tool_coolant"), "manufacture_tool_coolant", &mut self.coolant, &coolants);
                ui.add_space(6.0);
                ui_kit::text(ui, tr("manufacture.tool_cutting_data"), ui_kit::Text::Heading("manufacture_tool_cutting_data"));
                ui_kit::text(ui, tr("manufacture.tool_cutting_data_note"), ui_kit::Text::Note("manufacture_tool_cutting_data_note"));
                ui_kit::text_row(ui, tr_label("Material"), &mut self.material, tr("manufacture.tool_material_hint"));
                ui_kit::row(ui, tr("manufacture.tool_surface_speed"), |ui| surface_speed_field(ui, &mut self.surface_speed, units));
                ui_kit::row(ui, tr("manufacture.tool_chip_load"), |ui| ui_kit::length_field(ui, &mut self.chip_load, units, decimals.max(3)));
                refusal(ui, self.error.as_deref());
            });
        match action {
            ui_kit::Action::Ok => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}

/// New Setup: how the part is held and where its work origin is.
pub(crate) struct SetupForm {
    /// The name typed; empty for the command's own ("Setup 2").
    pub name: String,
    /// The stock's material, as the tools' cutting data names it; none when empty.
    pub material: String,
    /// Stock beyond the model, mm: on each side, on top and underneath.
    pub side: f64,
    pub top: f64,
    pub bottom: f64,
    pub origin: usize,
    /// 0 for G54 up to 5 for G59.
    pub work_offset: usize,
    /// The name the command gives the setup when none is typed.
    usual_name: String,
    /// The materials the design's tools have cutting data for.
    known_materials: Vec<String>,
    /// Where the setup would sit on the model.
    placed: Worked<Placement>,
    pub error: Option<String>,
}

impl SetupForm {
    /// The vise setup the command starts from: 1 mm of stock on the sides and on top (0.04 in in
    /// an inch design), the origin at the stock's top centre, G54. Its stock material is the
    /// first one the design's tools have cutting data for, so operations find their feeds.
    pub fn new(app: &SolveApp) -> SetupForm {
        let data = manufacture_data(&app.session.doc).unwrap_or_default();
        let known_materials: Vec<String> = data.tools.iter().flat_map(|t| t.cutting_data.iter().map(|c| c.material.clone())).collect();
        let over = if imperial(&app.session.doc.units) { 1.016 } else { 1.0 };
        SetupForm {
            name: String::new(),
            material: known_materials.first().cloned().unwrap_or_default(),
            side: over,
            top: over,
            bottom: 0.0,
            origin: 0,
            work_offset: 0,
            usual_name: format!("Setup {}", data.setups.len() + 1),
            known_materials,
            placed: Worked::new(),
            error: None,
        }
    }

    /// The `cam.setup.create` parameters.
    pub fn params(&self) -> Value {
        let [x, y, z] = ORIGINS.get(self.origin).or(ORIGINS.first()).map_or(["center", "center", "max"], |o| o.1);
        let mut params = json!({
            "stock": {"kind": "relative_box", "offsets": {
                "x_min": self.side, "x_max": self.side, "y_min": self.side, "y_max": self.side, "z_min": self.bottom, "z_max": self.top
            }},
            "wcs": {"origin": {"kind": "stock_box", "anchor": {"x": x, "y": y, "z": z}}, "orientation": {"kind": "model"}},
            "work_offset": self.work_offset.min(5) + 1,
        });
        if !self.name.trim().is_empty() {
            params["name"] = json!(self.name.trim());
        }
        if !self.material.trim().is_empty() {
            params["material"] = json!(self.material.trim());
        }
        params
    }

    /// Works out where the setup would sit, if its parameters or the design changed.
    fn refresh(&mut self, app: &SolveApp) {
        let params = self.params();
        if self.placed.refresh(app, &params, || manufacture_setup_preview(&app.session, &params)) {
            self.error = None;
        }
    }

    /// Runs the command and makes the new setup the one drawn: the form again, with the reason,
    /// when the setup can't be placed or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<SetupForm> {
        self.refresh(app);
        if self.placed.result.is_err() {
            return Some(self);
        }
        match app.run("cam.setup.create", self.params()) {
            Ok(made) => {
                app.manufacture_workspace.active_setup = made.get("id").and_then(Value::as_str).map(str::to_string);
                None
            }
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    /// Whether a stock material is typed that no tool has cutting data for: operations of the
    /// setup would then have to be given their speed and feed.
    pub fn material_unknown(&self) -> bool {
        let typed = self.material.trim();
        !typed.is_empty() && !self.known_materials.iter().any(|known| known == typed)
    }

    /// The stock's size along X, Y and Z, in the design's units.
    fn stock_size(&self, units: &str, decimals: u8) -> Option<String> {
        let size = self.placed.get()?.1.stock.size();
        let (per, unit) = unit_scale(units);
        let places = usize::from(decimals.min(8));
        Some(format!("{:.places$} × {:.places$} × {:.places$} {unit}", size.x * per, size.y * per, size.z * per))
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<SetupForm> {
        self.refresh(app);
        let origins: Vec<&str> = ORIGINS.iter().map(|o| tr(o.0)).collect();
        let size = self.stock_size(units, decimals);
        let ready = self.placed.result.is_ok();
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_setup"), "ws_manufacture", tr("cmd.cam_setup_create"))
            .placed(pivot, at)
            .fields()
            .actions(tr("ui.create"), tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                ui_kit::text_row(ui, tr_label("Name"), &mut self.name, &self.usual_name);
                ui_kit::text_row(ui, tr("manufacture.setup_material"), &mut self.material, tr("manufacture.tool_material_hint"));
                if self.material_unknown() {
                    ui_kit::text(ui, tr("manufacture.setup_material_unknown"), ui_kit::Text::Note("manufacture_setup_material_unknown"));
                }
                ui_kit::row(ui, tr("manufacture.setup_stock_side"), |ui| ui_kit::length_field(ui, &mut self.side, units, decimals));
                ui_kit::row(ui, tr("manufacture.setup_stock_top"), |ui| ui_kit::length_field(ui, &mut self.top, units, decimals));
                ui_kit::row(ui, tr("manufacture.setup_stock_bottom"), |ui| ui_kit::length_field(ui, &mut self.bottom, units, decimals));
                if let Some(size) = &size {
                    ui_kit::value_row(ui, tr("manufacture.setup_stock_size"), size);
                }
                ui_kit::choice_row(ui, tr("manufacture.setup_origin"), "manufacture_setup_origin", &mut self.origin, &origins);
                ui_kit::choice_row(ui, tr("manufacture.setup_work_offset"), "manufacture_setup_offset", &mut self.work_offset, &WORK_OFFSETS);
                if ready {
                    ui_kit::text(ui, tr("manufacture.preview_note"), ui_kit::Text::Note("manufacture_setup_preview_note"));
                }
                refusal(ui, self.error.as_deref().or(self.placed.trouble()));
            });
        match action {
            ui_kit::Action::Ok => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}

/// Face: the stock's top down to the part.
pub(crate) struct FaceForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    pub tool: usize,
    pub pattern: usize,
    /// mm: 70% of the tool's diameter until it is typed.
    pub stepover: f64,
    pub typed_stepover: bool,
    pub multiple_depths: bool,
    /// mm, when there are multiple depths.
    pub stepdown: f64,
    /// Spindle speed, rev/min, and cutting feed, mm/min: what the tool's cutting data gives for
    /// the setup's stock material (the speed shown to the whole turn, as a program words it),
    /// until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The facing these parameters would make.
    worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    pub error: Option<String>,
}

impl FaceForm {
    /// For the setup the viewport draws and the first tool that can face.
    pub fn new(app: &SolveApp) -> FaceForm {
        let (setups, setup) = setups(app);
        let tools = manufacture_tools(&app.session.doc, "face");
        let stepover = tools.first().map_or(0.0, |t| 0.7 * t.diameter);
        FaceForm {
            setups,
            tools,
            setup,
            tool: 0,
            pattern: 0,
            stepover,
            typed_stepover: false,
            multiple_depths: false,
            stepdown: if imperial(&app.session.doc.units) { 1.016 } else { 1.0 },
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            worked: Worked::new(),
            drawn_setup: None,
            error: None,
        }
    }

    /// What is missing before a facing can be made, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_face"))
        } else {
            None
        }
    }

    /// The `cam.op.face` parameters. What was not typed is left to the command, so it keeps
    /// following the tool: the stepover its diameter, the speed and feed its cutting data.
    pub fn params(&self) -> Value {
        let mut face = json!({"pattern": word(&PATTERNS, self.pattern)});
        if self.typed_stepover {
            face["stepover"] = json!(self.stepover);
        }
        if self.multiple_depths {
            face["stepdown"] = json!(self.stepdown);
        }
        if self.typed_rpm {
            face["rpm"] = json!(self.rpm);
        }
        if self.typed_feed {
            face["feed"] = json!(self.feed);
        }
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "face": face,
        })
    }

    /// Works the facing out, if its parameters or the design changed, and shows the numbers that
    /// were not typed as it gives them.
    fn refresh(&mut self, app: &SolveApp) {
        if !self.typed_stepover {
            self.stepover = self.tools.get(self.tool).map_or(self.stepover, |t| 0.7 * t.diameter);
        }
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.face", &params)) {
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

    /// Runs the command: the form again, with the reason, when the facing can't be worked out or
    /// the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<FaceForm> {
        self.refresh(app);
        if self.missing().is_some() || self.worked.result.is_err() {
            return Some(self);
        }
        match app.run("cam.op.face", self.params()) {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<FaceForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let patterns = labels(&PATTERNS);
        let missing = self.missing();
        let ready = missing.is_none() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // The setup picked is the one the viewport draws, so the facing is seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_face"), "edit", tr("cmd.cam_op_face"))
            .placed(pivot, at)
            .fields()
            .actions(tr("ui.create"), tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_face_missing"));
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_face_setup", &mut self.setup, &setup_labels);
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_face_tool", &mut self.tool, &tool_labels);
                ui_kit::choice_row(ui, tr_label("Pattern"), "manufacture_face_pattern", &mut self.pattern, &patterns);
                if ui_kit::row(ui, tr("manufacture.face_stepover"), |ui| ui_kit::length_field(ui, &mut self.stepover, units, decimals)).changed() {
                    self.typed_stepover = true;
                }
                ui_kit::checkbox(ui, tr("manufacture.face_multiple"), &mut self.multiple_depths);
                if self.multiple_depths {
                    ui_kit::row(ui, tr("manufacture.face_stepdown"), |ui| ui_kit::length_field(ui, &mut self.stepdown, units, decimals));
                }
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_face_engine_note"));
                }
                ui_kit::text(ui, tr("manufacture.face_note"), ui_kit::Text::Note("manufacture_face_note"));
                if ready {
                    ui_kit::text(ui, tr("manufacture.preview_note"), ui_kit::Text::Note("manufacture_face_preview_note"));
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

/// The depth a group of holes is drilled to when none is typed, from their top, and what it is
/// measured to (decision 0012 of the CAM work): a through hole to the drill's full diameter a
/// little past the part, a pointed hole to the full diameter at the end of its wall, and a
/// flat-floored hole to the drill's tip, so that a point never cuts below the floor.
fn drill_depth(group: &HoleGroup) -> (f64, &'static str) {
    match group.bottom {
        HoleBottom::Through => (group.depth + BREAKTHROUGH, "full_diameter"),
        HoleBottom::Point => (group.depth, "full_diameter"),
        HoleBottom::Flat => (group.depth, "tip"),
    }
}

/// A group of holes as Drill offers it, diameter by depth: "4 × Ø6.00 × 20.00 mm, through".
fn group_label(group: &HoleGroup, units: &str, decimals: u8) -> String {
    let bottom = tr(match group.bottom {
        HoleBottom::Through => "manufacture.bottom_through",
        HoleBottom::Flat => "manufacture.bottom_flat",
        HoleBottom::Point => "manufacture.bottom_point",
    });
    let (count, diameter, depth) =
        (group.holes.len().to_string(), length_number(group.diameter, units, decimals), format_length(group.depth, units, decimals));
    solidcraft_ui_text::format("manufacture.drill_group", &[("count", &count), ("diameter", &diameter), ("depth", &depth), ("bottom", bottom)])
}

/// Drill: holes found on the model, a group at a time.
pub(crate) struct DrillForm {
    setups: Vec<SetupChoice>,
    tools: Vec<ToolChoice>,
    pub setup: usize,
    /// The holes the setup picked can drill, in groups, as the engine found them.
    found: Worked<Vec<HoleGroup>>,
    pub group: usize,
    /// The group the depth and the tool were last suited to.
    suited_to: Option<usize>,
    pub tool: usize,
    /// Whether the tool was picked by hand: until then it follows the holes' diameter.
    pub picked_tool: bool,
    pub cycle: usize,
    /// mm a peck, for the pecking cycles: the tool's diameter until it is typed.
    pub peck: f64,
    pub typed_peck: bool,
    /// mm below the holes' top: what their own bottom asks for until it is typed.
    pub depth: f64,
    pub typed_depth: bool,
    /// Spindle speed, rev/min, and feed, mm/min: what the tool's cutting data gives until typed.
    pub rpm: f64,
    pub feed: f64,
    pub typed_rpm: bool,
    pub typed_feed: bool,
    /// The drilling these parameters would make.
    worked: Worked<OperationPreview>,
    /// The setup of the list the viewport was last told to draw.
    drawn_setup: Option<usize>,
    pub error: Option<String>,
}

impl DrillForm {
    /// For the setup the viewport draws, its largest group of holes and the tool that suits them.
    pub fn new(app: &SolveApp) -> DrillForm {
        let (setups, setup) = setups(app);
        DrillForm {
            setups,
            tools: manufacture_tools(&app.session.doc, "drill"),
            setup,
            found: Worked::new(),
            group: 0,
            suited_to: None,
            tool: 0,
            picked_tool: false,
            cycle: 0,
            peck: 0.0,
            typed_peck: false,
            depth: 0.0,
            typed_depth: false,
            rpm: 0.0,
            feed: 0.0,
            typed_rpm: false,
            typed_feed: false,
            worked: Worked::new(),
            drawn_setup: None,
            error: None,
        }
    }

    /// What is missing before holes can be drilled, if anything.
    pub fn missing(&self) -> Option<&'static str> {
        if self.setups.is_empty() {
            Some(tr("manufacture.no_setup"))
        } else if self.tools.is_empty() {
            Some(tr("manufacture.no_tool_for_drill"))
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

    /// The tool that suits holes of `diameter`: the widest that is no wider, else the narrowest.
    fn fitting_tool(&self, diameter: f64) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let fits = self.tools.iter().enumerate().filter(|(_, tool)| tool.diameter <= diameter + 1e-6).max_by(by_width);
        fits.or(self.tools.iter().enumerate().min_by(by_width)).map_or(0, |(at, _)| at)
    }

    /// The `cam.op.drill` parameters: the group's centres and top, the depth its bottom asks for
    /// or the one typed, and the cycle. A speed or feed that was not typed is left to the command.
    pub fn params(&self) -> Value {
        let (holes, top, depth, depth_to) = match self.picked() {
            Some(group) => {
                let (suited, to) = drill_depth(group);
                (json!(group.holes), json!(group.top), if self.typed_depth { self.depth } else { suited }, to)
            }
            None => (json!([]), Value::Null, self.depth, "tip"),
        };
        let cycle = match word(&CYCLES, self.cycle) {
            "drill" => json!({"type": "drill", "dwell": 0.0}),
            pecking => json!({"type": pecking, "peck": self.peck}),
        };
        let mut drill = json!({"holes": holes, "top": top, "depth": depth, "depth_to": depth_to, "cycle": cycle});
        if self.typed_rpm {
            drill["rpm"] = json!(self.rpm);
        }
        if self.typed_feed {
            drill["feed"] = json!(self.feed);
        }
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "drill": drill,
        })
    }

    /// Finds the setup's holes and works the drilling out, when the design or a field changed,
    /// and shows what was not typed as the holes and the tool give it.
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
            // Another group: its own depth, and the tool that suits it unless one was picked.
            (self.suited_to, self.typed_depth) = (Some(self.group), false);
            if let Some(diameter) = self.picked().map(|group| group.diameter).filter(|_| !self.picked_tool) {
                self.tool = self.fitting_tool(diameter);
            }
        }
        if let Some(suited) = self.picked().map(|group| drill_depth(group).0).filter(|_| !self.typed_depth) {
            self.depth = suited;
        }
        if !self.typed_peck {
            self.peck = self.tools.get(self.tool).map_or(self.peck, |tool| tool.diameter);
        }
        let params = self.params();
        if !self.worked.refresh(app, &params, || manufacture_preview(&app.session, "cam.op.drill", &params)) {
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

    /// Runs the command: the form again, with the reason, when the drilling can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<DrillForm> {
        self.refresh(app);
        if self.missing().is_some() || self.picked().is_none() || self.worked.result.is_err() {
            return Some(self);
        }
        match app.run("cam.op.drill", self.params()) {
            Ok(_) => None,
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<DrillForm> {
        self.refresh(app);
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let tool_labels: Vec<String> = self.tools.iter().map(|t| format!("T{} {}", t.number, t.name)).collect();
        let tool_labels: Vec<&str> = tool_labels.iter().map(String::as_str).collect();
        let group_labels: Vec<String> = self.groups().iter().map(|group| group_label(group, units, decimals)).collect();
        let group_labels: Vec<&str> = group_labels.iter().map(String::as_str).collect();
        let cycles = labels(&CYCLES);
        // The widths the form compares while a tool is being picked.
        let hole_widths: Vec<f64> = self.groups().iter().map(|group| group.diameter).collect();
        let tool_widths: Vec<f64> = self.tools.iter().map(|tool| tool.diameter).collect();
        let missing = self.missing();
        let ready = missing.is_none() && self.picked().is_some() && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // How the depth is measured, for the holes picked.
        let past = format_length(BREAKTHROUGH, units, decimals);
        let measured = self.picked().map(|group| match group.bottom {
            HoleBottom::Through => solidcraft_ui_text::format("manufacture.drill_depth_through", &[("past", &past)]),
            HoleBottom::Point => tr("manufacture.drill_depth_point").to_string(),
            HoleBottom::Flat => tr("manufacture.drill_depth_flat").to_string(),
        });
        // The setup picked is the one the viewport draws, so the holes are seen on its own stock.
        if self.drawn_setup != Some(self.setup)
            && let Some(picked) = self.setups.get(self.setup)
        {
            app.manufacture_workspace.active_setup = Some(picked.id.clone());
            self.drawn_setup = Some(self.setup);
        }
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_drill"), "hole", tr("cmd.cam_op_drill"))
            .placed(pivot, at)
            .fields()
            .actions(tr("ui.create"), tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_drill_missing"));
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_drill_setup", &mut self.setup, &setup_labels);
                if group_labels.is_empty() {
                    // Why no holes could be looked for, or that there are none to drill from here.
                    match self.found.trouble().filter(|why| !why.is_empty()) {
                        Some(why) => refusal(ui, Some(why)),
                        None => {
                            ui_kit::text(ui, tr("manufacture.drill_no_holes"), ui_kit::Text::Note("manufacture_drill_no_holes"));
                        }
                    }
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.drill_holes"), "manufacture_drill_holes", &mut self.group, &group_labels);
                let tool = self.tool;
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_drill_tool", &mut self.tool, &tool_labels);
                self.picked_tool |= self.tool != tool;
                if hole_widths.get(self.group).zip(tool_widths.get(self.tool)).is_some_and(|(hole, tool)| *tool > hole + 1e-6) {
                    ui_kit::status_line(ui, tr("manufacture.drill_tool_wider"), ui_kit::StatusTone::Warning);
                }
                ui_kit::choice_row(ui, tr("manufacture.drill_cycle"), "manufacture_drill_cycle", &mut self.cycle, &cycles);
                if word(&CYCLES, self.cycle) != "drill" {
                    self.typed_peck |=
                        ui_kit::row(ui, tr("manufacture.drill_peck"), |ui| ui_kit::length_field(ui, &mut self.peck, units, decimals)).changed();
                }
                self.typed_depth |=
                    ui_kit::row(ui, tr("manufacture.drill_depth"), |ui| ui_kit::length_field(ui, &mut self.depth, units, decimals)).changed();
                if let Some(measured) = &measured {
                    ui_kit::text(ui, measured, ui_kit::Text::Note("manufacture_drill_measured"));
                }
                let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                self.typed_rpm |= rpm;
                self.typed_feed |= feed;
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_drill_engine_note"));
                }
                if ready {
                    ui_kit::text(ui, tr("manufacture.preview_note"), ui_kit::Text::Note("manufacture_drill_preview_note"));
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

/// Minutes as `min:s`, to the nearest second.
fn minutes_seconds(minutes: f64) -> String {
    let seconds = (minutes * 60.0).round().clamp(0.0, 1e9) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

/// Post Process: a setup's program, for one controller, saved to a file.
pub(crate) struct PostForm {
    setups: Vec<SetupChoice>,
    posts: Vec<(String, String)>,
    pub setup: usize,
    pub post: usize,
    pub units: usize,
    pub program_number: u32,
    /// The program's name; the setup's when empty.
    pub program_name: String,
    pub error: Option<String>,
}

impl PostForm {
    /// For the setup the viewport draws, in the design's units, for the Haas-style post.
    pub fn new(app: &SolveApp) -> PostForm {
        let (setups, setup) = setups(app);
        let posts = manufacture_posts();
        let post = posts.iter().position(|(id, _)| id == "haas_mill").unwrap_or(0);
        let units = usize::from(imperial(&app.session.doc.units));
        PostForm { setups, posts, setup, post, units, program_number: 1, program_name: String::new(), error: None }
    }

    /// The `cam.post` parameters, without the file.
    pub fn params(&self) -> Value {
        let mut params = json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "post": self.posts.get(self.post).map_or("", |p| p.0.as_str()),
            "units": word(&UNITS, self.units),
            "program_number": self.program_number,
        });
        if !self.program_name.trim().is_empty() {
            params["program_name"] = json!(self.program_name.trim());
        }
        params
    }

    /// The file name offered: the design's and the setup's names, with letters, digits and `_`
    /// kept and every run of anything else a `-`.
    pub fn file_name(&self, design: &str) -> String {
        let setup = self.setups.get(self.setup).map_or("", |s| s.name.as_str());
        let plain: String = format!("{design} {setup}").chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '-' }).collect();
        let plain = plain.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
        format!("{}.nc", if plain.is_empty() { "program" } else { plain.as_str() })
    }

    /// Asks where to save, then posts there and shows what was written. The form comes back as
    /// it was when no file is chosen, and with the reason when the program can't be posted. The
    /// save prompt has already asked before replacing a file.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<PostForm> {
        if app.file_picker_busy() {
            return Some(self);
        }
        let params = self.params();
        let name = self.file_name(&app.session.doc.name);
        app.pick_save(&name, &PROGRAM_EXTENSIONS, move |app, path| {
            let Some(path) = path else {
                app.manufacture_workspace.dialog = Some(Dialog::Post(self));
                return;
            };
            let mut params = params;
            params["path"] = json!(path);
            params["overwrite"] = json!(true);
            app.manufacture_workspace.dialog = Some(match app.run("cam.post", params) {
                Ok(result) => Dialog::Posted(Posted::new(&result, path)),
                Err(why) => {
                    self.error = Some(why);
                    Dialog::Post(self)
                }
            });
        });
        None
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context) -> Option<PostForm> {
        let setup_labels: Vec<&str> = self.setups.iter().map(|s| s.label.as_str()).collect();
        let post_labels: Vec<&str> = self.posts.iter().map(|p| p.1.as_str()).collect();
        let units = labels(&UNITS);
        let ready = !self.setups.is_empty();
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_post"), "export", tr("cmd.cam_post"))
            .placed(pivot, at)
            .fields()
            .actions(tr("manufacture.post_save"), tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if !ready {
                    ui_kit::text(ui, tr("manufacture.no_setup"), ui_kit::Text::Note("manufacture_post_missing"));
                    return;
                }
                ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_post_setup", &mut self.setup, &setup_labels);
                ui_kit::choice_row(ui, tr("manufacture.post_controller"), "manufacture_post_controller", &mut self.post, &post_labels);
                ui_kit::choice_row(ui, tr("ui.units"), "manufacture_post_units", &mut self.units, &units);
                ui_kit::row(ui, tr("manufacture.post_program_number"), |ui| ui_kit::whole_number_field(ui, &mut self.program_number, 0..=99_999, ""));
                let usual_name = self.setups.get(self.setup).map_or("", |s| s.name.as_str());
                ui_kit::text_row(ui, tr("manufacture.post_program_name"), &mut self.program_name, usual_name);
                ui_kit::status_line(ui, tr("manufacture.post_warning"), ui_kit::StatusTone::Warning);
                refusal(ui, self.error.as_deref());
            });
        match action {
            ui_kit::Action::Ok if ready => self.accept(app),
            ui_kit::Action::None if !closed => Some(self),
            _ => None,
        }
    }
}

/// What Post Process wrote.
pub(crate) struct Posted {
    pub path: String,
    pub lines: u64,
    /// The moves written, each read back and matched with its toolpath.
    pub moves: u64,
    pub notes: Vec<String>,
}

impl Posted {
    /// From `cam.post`'s reply for the file at `path`.
    fn new(result: &Value, path: String) -> Posted {
        let count = |key: &str| result.get(key).and_then(Value::as_u64).unwrap_or(0);
        let notes = result.get("notes").and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect());
        Posted { path, lines: count("lines"), moves: count("moves"), notes: notes.unwrap_or_default() }
    }

    fn show(self, ctx: &egui::Context) -> Option<Posted> {
        let written = solidcraft_ui_text::format("manufacture.posted_lines", &[("lines", &self.lines.to_string()), ("path", &self.path)]);
        let checked = solidcraft_ui_text::format("manufacture.posted_checked", &[("moves", &self.moves.to_string())]);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_posted"), "export", tr("manufacture.posted_title"))
            .actions("", tr("ui.close"), None)
            .show(ctx, true, |ui| {
                ui_kit::text(ui, &written, ui_kit::Text::Report("manufacture_posted_lines", false));
                ui_kit::text(ui, &checked, ui_kit::Text::Note("manufacture_posted_checked"));
                for note in &self.notes {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_posted_note"));
                }
                ui_kit::status_line(ui, tr("manufacture.post_warning"), ui_kit::StatusTone::Warning);
            });
        (action == ui_kit::Action::None && !closed).then_some(self)
    }
}

/// Opens the form of a Manufacture command that was started (the ribbon, the command search, a
/// shortcut): whether `id` has one. Started outside Manufacture, the form opens there; when
/// Manufacture can't be entered now, the reason goes to the status line and nothing opens.
pub(crate) fn start(app: &mut SolveApp, id: &str) -> bool {
    if !matches!(id, "cam.tool.create" | "cam.setup.create" | "cam.op.face" | "cam.op.drill" | "cam.post") {
        return false;
    }
    if !super::active(app)
        && let Err(why) = crate::workspace::enter(app, "manufacture")
    {
        app.set_status(why, true);
        return true;
    }
    app.manufacture_workspace.dialog = Some(match id {
        "cam.tool.create" => Dialog::Tool(ToolForm::new(app)),
        "cam.setup.create" => Dialog::Setup(SetupForm::new(app)),
        "cam.op.face" => Dialog::Face(FaceForm::new(app)),
        "cam.op.drill" => Dialog::Drill(DrillForm::new(app)),
        _ => Dialog::Post(PostForm::new(app)),
    });
    true
}

/// The open form, if any. Called once a frame. One command's form is open at a time: a form
/// closes when Manufacture is left, and when another command opens its own dialog or tool.
pub(crate) fn show(app: &mut SolveApp, ctx: &egui::Context) {
    let Some(dialog) = app.manufacture_workspace.dialog.take() else { return };
    if !super::active(app) || app.dialog.is_some() || app.tool.is_some() {
        return;
    }
    let (units, decimals) = (app.session.doc.units.clone(), app.preferences.length_decimals);
    let kept = match dialog {
        Dialog::Tool(form) => form.show(app, ctx, &units, decimals).map(Dialog::Tool),
        Dialog::Setup(form) => form.show(app, ctx, &units, decimals).map(Dialog::Setup),
        Dialog::Face(form) => form.show(app, ctx, &units, decimals).map(Dialog::Face),
        Dialog::Drill(form) => form.show(app, ctx, &units, decimals).map(Dialog::Drill),
        Dialog::Post(form) => form.show(app, ctx).map(Dialog::Post),
        Dialog::Posted(done) => done.show(ctx).map(Dialog::Posted),
    };
    // A form that opened another while it ran keeps that one.
    if app.manufacture_workspace.dialog.is_none() {
        app.manufacture_workspace.dialog = kept;
    }
}

#[cfg(test)]
#[path = "dialogs_tests.rs"]
mod tests;
