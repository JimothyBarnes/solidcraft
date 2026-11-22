//! Manufacture's forms: Add Tool, Import Tools, New Setup, Face, Contour, Pocket, Drill, Simulate and Post Process. Each is a front end that
//! builds the parameters of one `cam.*` command and runs it: every change is a command, and a
//! script calls the command directly and sees no window. They are standalone windows, opened
//! when the ribbon starts one of those commands ([`start`]) and drawn once a frame ([`show`]).
//!
//! New Setup, Face, Contour, Pocket and Drill show what they would make before OK: the engine works it out from the
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
    CamLibraryToolStatus, CamLibraryTools, HoleBottom, HoleGroup, OperationPreview, Placement, Simulated, ToolChoice, manufacture_data,
    manufacture_holes, manufacture_library, manufacture_posts, manufacture_preview, manufacture_setup_preview, manufacture_tools,
};
use solidcraft_engine::doc::display_units::{format_length, length_number, unit_scale};
use solidcraft_ui_text::{tr, tr_label};

use crate::SolveApp;
use crate::ui_kit;

mod contour;
mod pocket;
mod simulate;
pub(crate) use contour::ContourForm;
pub(crate) use pocket::PocketForm;
pub(crate) use simulate::SimForm;

/// The open form.
pub(crate) enum Dialog {
    Tool(ToolForm),
    Import(ImportForm),
    Setup(SetupForm),
    Face(FaceForm),
    Contour(ContourForm),
    Pocket(PocketForm),
    Drill(DrillForm),
    Simulate(SimForm),
    Post(PostForm),
    Posted(Posted),
}

/// What the Browser asks a form to change.
pub(crate) enum Target {
    Tool(u32),
    Setup(String),
    Operation(String),
}

/// What a form in edit mode changes: the thing's id as its command takes it, the title its window
/// carries, and its parameters as the design keeps them, so that what the form does not show stays
/// as it is.
struct Edited {
    id: Value,
    title: String,
    base: Value,
}

/// What an open form adds to the viewport.
pub(crate) enum Shown<'a> {
    /// A setup not made yet: its stock and work origin, in place of the active setup's.
    Setup(&'a Placement),
    /// An operation not made yet: its toolpath, with the active setup.
    Operation(&'a OperationPreview),
    /// A simulation: the stock as the setup's operations leave it, in place of the stock box.
    Stock(&'a Simulated),
}

impl Dialog {
    /// The operation the open form is changing, if it is changing one: its own toolpath gives way
    /// to the form's preview in the viewport.
    pub(crate) fn edited_operation(&self) -> Option<&str> {
        match self {
            Dialog::Face(FaceForm { editing: Some(edited), .. })
            | Dialog::Contour(ContourForm { editing: Some(edited), .. })
            | Dialog::Pocket(PocketForm { editing: Some(edited), .. })
            | Dialog::Drill(DrillForm { editing: Some(edited), .. }) => edited.id.as_str(),
            _ => None,
        }
    }

    /// What the form adds to the viewport, with a number that changes when that does.
    pub(crate) fn shown(&self) -> Option<(u64, Shown<'_>)> {
        match self {
            Dialog::Setup(form) => form.placed.get().map(|(key, placement)| (key, Shown::Setup(placement))),
            Dialog::Face(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            Dialog::Contour(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            Dialog::Pocket(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            Dialog::Drill(form) => form.worked.get().map(|(key, preview)| (key, Shown::Operation(preview))),
            Dialog::Simulate(form) => form.worked.get().map(|(key, simulated)| (key, Shown::Stock(simulated))),
            _ => None,
        }
    }
}

/// The tool kinds Add Tool offers: the command's word and the label.
const KINDS: [(&str, &str); 8] = [
    ("flat_end_mill", "manufacture.kind_flat_end_mill"),
    ("face_mill", "manufacture.kind_face_mill"),
    ("bull_nose_end_mill", "manufacture.kind_bull_nose_end_mill"),
    ("ball_end_mill", "manufacture.kind_ball_end_mill"),
    ("drill", "manufacture.kind_drill"),
    ("spot_drill", "manufacture.kind_spot_drill"),
    ("reamer", "manufacture.kind_reamer"),
    ("tap", "manufacture.kind_tap"),
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
/// How many pitches further a tap's tip goes through a through hole, so that its full thread
/// is past the part and not only its chamfer.
const TAP_LEAD: f64 = 3.0;
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

/// Sets a number an operation keeps, or takes it out so the command's own default applies.
fn set_or_clear(object: &mut Value, key: &str, value: Option<f64>) {
    match value {
        Some(value) => object[key] = json!(value),
        None => {
            if let Some(fields) = object.as_object_mut() {
                fields.remove(key);
            }
        }
    }
}

/// The title and the OK label of a form's window: the command's own to make something, the
/// thing's name and "OK" to change it.
fn window_words(editing: &Option<Edited>, command: &'static str) -> (String, &'static str) {
    match editing {
        Some(edited) => (edited.title.clone(), tr_label("OK")),
        None => (tr(command).to_string(), tr("ui.create")),
    }
}

/// The line that marks what is drawn on the model as a preview: of something new, or of a change.
fn preview_note(editing: &Option<Edited>) -> &'static str {
    tr(if editing.is_some() { "manufacture.preview_note_edit" } else { "manufacture.preview_note" })
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
    /// mm a turn, for a tap.
    pub pitch: f64,
    pub coolant: usize,
    /// The material the cutting data is for; no cutting data when empty.
    pub material: String,
    /// m/min.
    pub surface_speed: f64,
    /// mm a tooth.
    pub chip_load: f64,
    /// The tool being changed, when the form changes one.
    editing: Option<Edited>,
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
            // 20 threads an inch, or 1 mm.
            pitch: if inch { 1.27 } else { 1.0 },
            coolant: 0,
            material: String::new(),
            // 650 ft/min and 0.002 in a tooth, or 200 m/min and 0.05 mm.
            surface_speed: if inch { 198.12 } else { 200.0 },
            chip_load: if inch { 0.0508 } else { 0.05 },
            editing: None,
            error: None,
        }
    }

    /// The form for changing the design's tool `number`; why not, when it has no such tool or the
    /// tool is of a kind Add Tool does not offer (one from a tool library file).
    pub fn edit(app: &SolveApp, number: u32) -> Result<ToolForm, String> {
        let data = manufacture_data(&app.session.doc)?;
        let tool = data.tools.iter().find(|t| t.number == number).ok_or_else(|| format!("the design has no tool {number}"))?;
        let base = serde_json::to_value(tool).map_err(|e| e.to_string())?;
        let kind =
            KINDS.iter().position(|k| Some(k.0) == base["kind"].as_str()).ok_or_else(|| tr("manufacture.edit_unsupported_tool").to_string())?;
        let coolant = COOLANTS.iter().position(|c| Some(c.0) == base["coolant"].as_str()).unwrap_or(0);
        let data = tool.cutting_data.first();
        Ok(ToolForm {
            number,
            name: tool.name.clone(),
            kind,
            diameter: tool.geometry.diameter,
            corner_radius: tool.geometry.corner_radius,
            flutes: tool.flutes,
            typed_flutes: true,
            flute_length: tool.geometry.flute_length,
            overall_length: tool.geometry.overall_length,
            tip_angle: if tool.geometry.tip_angle > 0.0 { tool.geometry.tip_angle.to_degrees() } else { 118.0 },
            pitch: if tool.geometry.thread_pitch > 0.0 { tool.geometry.thread_pitch } else { 1.0 },
            coolant,
            material: data.map(|c| c.material.clone()).unwrap_or_default(),
            surface_speed: data.map_or(200.0, |c| c.surface_speed),
            chip_load: data.map_or(0.05, |c| c.chip_load),
            editing: Some(Edited { id: json!(number), title: format!("T{number} {}", tool.name), base }),
            error: None,
        })
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

    /// The `cam.tool.create` parameters, the tool named in the design's `units` when no name is
    /// typed; or `cam.tool.edit`'s, the tool as the design keeps it with the form's fields put in
    /// (its holder, offsets, other materials' cutting data and typed-in speeds stay as they are).
    pub fn params(&self, units: &str) -> Value {
        let kind = word(&KINDS, self.kind);
        let name = match self.name.trim() {
            "" => self.usual_name(units),
            typed => typed.to_string(),
        };
        let shown = json!({"material": self.material.trim(), "surface_speed": self.surface_speed, "chip_load": self.chip_load});
        let Some(edited) = &self.editing else {
            let corner_radius = match kind {
                "bull_nose_end_mill" => self.corner_radius,
                "ball_end_mill" => 0.5 * self.diameter,
                _ => 0.0,
            };
            let cutting_data = match self.material.trim() {
                "" => json!([]),
                _ => {
                    json!([{"material": shown["material"], "surface_speed": shown["surface_speed"], "chip_load": shown["chip_load"], "plunge_factor": 0.5, "ramp_factor": 0.5}])
                }
            };
            return json!({"tool": {
                "number": self.number, "name": name, "kind": kind,
                "geometry": {
                    "diameter": self.diameter, "corner_radius": corner_radius, "flute_length": self.flute_length, "shoulder_length": 0.0,
                    "overall_length": self.overall_length, "shank_diameter": self.diameter,
                    "tip_angle": if self.pointed() { self.tip_angle.to_radians() } else { 0.0 }, "tip_diameter": 0.0,
                    "thread_pitch": if kind == "tap" { self.pitch } else { 0.0 }
                },
                "flutes": self.flutes,
                "holder": {"stick_out": self.overall_length.min(self.flute_length + 10.0), "segments": []},
                "length_offset": self.number, "diameter_offset": self.number,
                "coolant": word(&COOLANTS, self.coolant),
                "cutting_data": cutting_data,
                "overrides": {"rpm": null, "feed": null}
            }});
        };
        let mut tool = edited.base.clone();
        let same_kind = tool["kind"].as_str() == Some(kind);
        let was = tool["geometry"].clone();
        let geometry = &mut tool["geometry"];
        // A shank that was the tool's own diameter follows it.
        if was["shank_diameter"] == was["diameter"] {
            geometry["shank_diameter"] = json!(self.diameter);
        }
        geometry["diameter"] = json!(self.diameter);
        geometry["corner_radius"] = match kind {
            "bull_nose_end_mill" => json!(self.corner_radius),
            "ball_end_mill" => json!(0.5 * self.diameter),
            _ if same_kind => was["corner_radius"].clone(),
            _ => json!(0.0),
        };
        geometry["flute_length"] = json!(self.flute_length);
        geometry["overall_length"] = json!(self.overall_length);
        // The angle as kept, unless it was changed: degrees and back would move its last digit.
        let kept_angle = was["tip_angle"].as_f64().filter(|kept| same_kind && (kept.to_degrees() - self.tip_angle).abs() < 1e-9);
        geometry["tip_angle"] = match (self.pointed(), kept_angle) {
            (true, Some(kept)) => json!(kept),
            (true, None) => json!(self.tip_angle.to_radians()),
            (false, _) if same_kind => was["tip_angle"].clone(),
            (false, _) => json!(0.0),
        };
        geometry["thread_pitch"] = match kind {
            "tap" => json!(self.pitch),
            _ if same_kind => was["thread_pitch"].clone(),
            _ => json!(0.0),
        };
        tool["number"] = json!(self.number);
        tool["name"] = json!(name);
        tool["kind"] = json!(kind);
        tool["flutes"] = json!(self.flutes);
        tool["coolant"] = json!(word(&COOLANTS, self.coolant));
        if tool["holder"]["stick_out"].as_f64().is_some_and(|out| out > self.overall_length) {
            tool["holder"]["stick_out"] = json!(self.overall_length);
        }
        // The cutting data shown is the tool's first; any for other materials stays.
        let mut data = tool["cutting_data"].as_array().cloned().unwrap_or_default();
        match (self.material.trim().is_empty(), data.first_mut()) {
            (true, Some(_)) => {
                data.remove(0);
            }
            (true, None) => {}
            (false, Some(first)) => {
                for key in ["material", "surface_speed", "chip_load"] {
                    first[key] = shown[key].clone();
                }
            }
            (false, None) => data.push(json!({"material": shown["material"], "surface_speed": shown["surface_speed"], "chip_load": shown["chip_load"], "plunge_factor": 0.5, "ramp_factor": 0.5})),
        }
        tool["cutting_data"] = Value::Array(data);
        json!({"number": edited.id, "tool": tool})
    }

    /// Runs the command: the form again, with the reason, when it refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ToolForm> {
        let params = self.params(&app.session.doc.units);
        let command = if self.editing.is_some() { "cam.tool.edit" } else { "cam.tool.create" };
        match app.run(command, params) {
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
        let (title, ok) = window_words(&self.editing, "cmd.cam_tool_create");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_tool"), "library", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
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
                if word(&KINDS, self.kind) == "tap" {
                    ui_kit::row(ui, tr("manufacture.tool_thread_pitch"), |ui| ui_kit::length_field(ui, &mut self.pitch, units, decimals));
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

/// A file name from `words`: letters, digits and `_` kept, every run of anything else one `-`;
/// `fallback` when nothing is left.
fn file_name(words: &str, fallback: &str, extension: &str) -> String {
    let plain: String = words.chars().map(|c| if c.is_alphanumeric() || c == '_' { c } else { '-' }).collect();
    let plain = plain.split('-').filter(|part| !part.is_empty()).collect::<Vec<_>>().join("-");
    format!("{}.{extension}", if plain.is_empty() { fallback } else { plain.as_str() })
}

/// Export Tool Library: asks where to save and writes the design's tools there as a tool library
/// file, for another design or machine to import. The save prompt asks before replacing a file.
fn export_library(app: &mut SolveApp) {
    if manufacture_data(&app.session.doc).is_ok_and(|data| data.tools.is_empty()) {
        app.set_status(tr("manufacture.library_no_tools"), true);
        return;
    }
    let name = file_name(&format!("{} tools", app.session.doc.name), "tools", "json");
    app.pick_save(&name, &["json"], |app, path| {
        let Some(path) = path else { return };
        // A refusal (a file that is not .json) is on the status line already.
        if let Ok(written) = app.run("cam.library.export", json!({"path": path, "overwrite": true})) {
            let tools = written.get("tools").and_then(Value::as_u64).unwrap_or(0).to_string();
            app.set_status(solidcraft_ui_text::format("manufacture.library_exported", &[("tools", &tools), ("path", &path)]), false);
        }
    });
}

/// Import Tools: copies tools from a tool library file into the design.
pub(crate) struct ImportForm {
    /// The library file.
    pub path: String,
    /// What the file holds, each tool compared with the design's of the same number.
    listed: Worked<CamLibraryTools>,
    /// The numbers of the tools ticked.
    pub picked: std::collections::BTreeSet<u32>,
    /// The listing the ticks were last set for: a new listing ticks its new tools.
    ticked_for: Option<u64>,
    pub error: Option<String>,
}

impl ImportForm {
    pub fn new() -> ImportForm {
        ImportForm { path: String::new(), listed: Worked::new(), picked: Default::default(), ticked_for: None, error: None }
    }

    /// The library's name and its tools, once a file is read.
    pub fn library(&self) -> Option<&CamLibraryTools> {
        self.listed.get().map(|(_, library)| library)
    }

    /// Reads the file again when the path or the design changed. A new listing starts with its
    /// new tools ticked; a tool that differs from the design's is only updated when ticked by hand.
    fn refresh(&mut self, app: &SolveApp) {
        let path = self.path.trim().to_string();
        let read = || if path.is_empty() { Err(String::new()) } else { manufacture_library(&app.session.doc, &path) };
        if self.listed.refresh(app, &json!(path), read) {
            self.error = None;
        }
        let listing = self.listed.get().map(|(key, _)| key);
        if self.ticked_for != listing {
            self.ticked_for = listing;
            let new = |library: &CamLibraryTools| library.tools.iter().filter(|t| t.status == CamLibraryToolStatus::New).map(|t| t.number).collect();
            self.picked = self.library().map(new).unwrap_or_default();
        }
    }

    /// The `cam.library.import` parameters: the tools ticked that the design does not already
    /// have as they are, updating the design's where a ticked one differs.
    pub fn params(&self) -> Value {
        let tools = self.library().map(|library| library.tools.as_slice()).unwrap_or_default();
        let ticked =
            |status: CamLibraryToolStatus| tools.iter().filter(move |t| t.status == status && self.picked.contains(&t.number)).map(|t| t.number);
        let differing: Vec<u32> = ticked(CamLibraryToolStatus::Differs).collect();
        let mut numbers: Vec<u32> = ticked(CamLibraryToolStatus::New).chain(differing.iter().copied()).collect();
        numbers.sort_unstable();
        json!({"path": self.path.trim(), "numbers": numbers, "replace": !differing.is_empty()})
    }

    fn anything_to_import(&self) -> bool {
        self.params()["numbers"].as_array().is_some_and(|numbers| !numbers.is_empty())
    }

    /// Runs the command and says what it did on the status line: the form again, with the reason,
    /// when it refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<ImportForm> {
        self.refresh(app);
        if !self.anything_to_import() {
            return Some(self);
        }
        match app.run("cam.library.import", self.params()) {
            Ok(done) => {
                let count = |key: &str| done.get(key).and_then(Value::as_array).map_or(0, Vec::len).to_string();
                let said = solidcraft_ui_text::format("manufacture.library_imported", &[("added", &count("added")), ("updated", &count("updated"))]);
                app.set_status(said, false);
                None
            }
            Err(why) => {
                self.error = Some(why);
                Some(self)
            }
        }
    }

    fn show(mut self, app: &mut SolveApp, ctx: &egui::Context, units: &str, decimals: u8) -> Option<ImportForm> {
        self.refresh(app);
        // Each tool as a line: what it is, and how it compares with the design's.
        let lines: Vec<(u32, CamLibraryToolStatus, String)> = self
            .library()
            .map(|library| library.tools.as_slice())
            .unwrap_or_default()
            .iter()
            .map(|t| {
                let status = tr(match t.status {
                    CamLibraryToolStatus::New => "manufacture.library_new",
                    CamLibraryToolStatus::Same => "manufacture.library_same",
                    CamLibraryToolStatus::Differs => "manufacture.library_differs",
                });
                let (number, diameter) = (t.number.to_string(), format_length(t.diameter, units, decimals));
                let line = solidcraft_ui_text::format(
                    "manufacture.library_tool",
                    &[("number", &number), ("name", &t.name), ("diameter", &diameter), ("status", status)],
                );
                (t.number, t.status, line)
            })
            .collect();
        let named = self.library().map(|library| library.name.clone());
        let ready = self.anything_to_import();
        let mut browse = false;
        let (pivot, at) = crate::shell::floating_editor_anchor(app, ctx);
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_import"), "library", tr("cmd.cam_library_import"))
            .placed(pivot, at)
            .fields()
            .actions(tr("manufacture.library_import"), tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                ui_kit::text_row(ui, tr("manufacture.library_file"), &mut self.path, tr("manufacture.library_file_hint"));
                browse = ui_kit::text_button(ui, tr("manufacture.library_browse"), true).clicked();
                match (&named, self.listed.trouble()) {
                    (Some(name), _) => {
                        ui_kit::text(ui, name, ui_kit::Text::Heading("manufacture_library_name"));
                        if lines.is_empty() {
                            ui_kit::text(ui, tr("manufacture.library_empty"), ui_kit::Text::Note("manufacture_library_empty"));
                        }
                        for (number, status, line) in &lines {
                            if *status == CamLibraryToolStatus::Same {
                                ui_kit::text(ui, line, ui_kit::Text::Note("manufacture_library_same"));
                                continue;
                            }
                            let mut ticked = self.picked.contains(number);
                            if ui_kit::checkbox(ui, line, &mut ticked).changed() {
                                if ticked {
                                    self.picked.insert(*number);
                                } else {
                                    self.picked.remove(number);
                                }
                            }
                        }
                    }
                    (None, Some(why)) if !why.is_empty() => refusal(ui, Some(why)),
                    _ => {
                        ui_kit::text(ui, tr("manufacture.library_choose"), ui_kit::Text::Note("manufacture_library_choose"));
                    }
                }
                refusal(ui, self.error.as_deref());
            });
        if browse {
            // The system's prompt: the form stays, and takes the file chosen when there is one.
            app.pick_open_for(&["json"], |app, path| {
                if let (Some(path), Some(Dialog::Import(form))) = (path, app.manufacture_workspace.dialog.as_mut()) {
                    form.path = path;
                }
            });
        }
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
    /// The setup being changed, when the form changes one.
    editing: Option<Edited>,
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
            editing: None,
            error: None,
        }
    }

    /// The form for changing the setup `id`; why not, when the design has no such setup or its
    /// stock or work origin is of a kind the form does not show (one a script made).
    pub fn edit(app: &SolveApp, id: &str) -> Result<SetupForm, String> {
        let data = manufacture_data(&app.session.doc)?;
        let setup = data.setups.iter().find(|s| s.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
        let base = serde_json::to_value(setup).map_err(|e| e.to_string())?;
        let other = || tr("manufacture.edit_unsupported_setup").to_string();
        // A box round the model with the same stock on all four sides, and the origin at one of
        // the stock's top points the form offers, turned as the model is.
        let offset = |side: &str| base["stock"]["offsets"][side].as_f64();
        let (Some(side), Some(top), Some(bottom)) = (offset("x_min"), offset("z_max"), offset("z_min")) else { return Err(other()) };
        let even = ["x_max", "y_min", "y_max"].iter().all(|k| offset(k).is_some_and(|v| (v - side).abs() < 1e-9));
        let anchor = &base["wcs"]["origin"]["anchor"];
        let origin = ORIGINS.iter().position(|o| json!({"x": o.1[0], "y": o.1[1], "z": o.1[2]}) == *anchor);
        let shown = base["stock"]["kind"] == "relative_box"
            && even
            && base["wcs"]["origin"]["kind"] == "stock_box"
            && base["wcs"]["orientation"]["kind"] == "model";
        let Some(origin) = origin.filter(|_| shown) else { return Err(other()) };
        Ok(SetupForm {
            name: setup.name.clone(),
            material: setup.material.clone().unwrap_or_default(),
            side,
            top,
            bottom,
            origin,
            work_offset: usize::from(setup.work_offset.clamp(1, 6)) - 1,
            usual_name: setup.name.clone(),
            known_materials: data.tools.iter().flat_map(|t| t.cutting_data.iter().map(|c| c.material.clone())).collect(),
            placed: Worked::new(),
            editing: Some(Edited { id: json!(id), title: setup.name.clone(), base: Value::Null }),
            error: None,
        })
    }

    /// The `cam.setup.create` parameters, or `cam.setup.edit`'s for the setup being changed (its
    /// bodies and safe heights are not given, so they stay as they are).
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
        match &self.editing {
            // An empty material takes the setup's away.
            Some(edited) => {
                params["id"] = edited.id.clone();
                params["material"] = json!(self.material.trim());
            }
            None if !self.material.trim().is_empty() => params["material"] = json!(self.material.trim()),
            None => {}
        }
        params
    }

    fn command(&self) -> &'static str {
        if self.editing.is_some() { "cam.setup.edit" } else { "cam.setup.create" }
    }

    /// Works out where the setup would sit, if its parameters or the design changed.
    fn refresh(&mut self, app: &SolveApp) {
        let (command, params) = (self.command(), self.params());
        if self.placed.refresh(app, &params, || manufacture_setup_preview(&app.session, command, &params)) {
            self.error = None;
        }
    }

    /// Runs the command (a new setup becomes the one drawn): the form again, with the reason,
    /// when the setup can't be placed or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<SetupForm> {
        self.refresh(app);
        if self.placed.result.is_err() {
            return Some(self);
        }
        match app.run(self.command(), self.params()) {
            Ok(made) => {
                if self.editing.is_none() {
                    app.manufacture_workspace.active_setup = made.get("id").and_then(Value::as_str).map(str::to_string);
                }
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
        let (title, ok) = window_words(&self.editing, "cmd.cam_setup_create");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_setup"), "ws_manufacture", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
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
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_setup_preview_note"));
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
    /// The facing being changed, when the form changes one: its `face` parameters as kept.
    editing: Option<Edited>,
    pub error: Option<String>,
}

/// The setup an operation is in and the operation, as the design keeps them: `(setup, name, tool,
/// kind, parameters)`, the parameters being the `drill`, `face`, `contour`, `pocket` or `toolpath` object.
fn kept_operation(app: &SolveApp, id: &str) -> Result<(SetupChoice, String, u32, String, Value), String> {
    let data = manufacture_data(&app.session.doc)?;
    let (setup, op) = data
        .setups
        .iter()
        .find_map(|s| s.operations.iter().find(|o| o.id == id).map(|o| (s, o)))
        .ok_or_else(|| format!("the design has no operation '{id}'"))?;
    let kept = serde_json::to_value(&op.operation).map_err(|e| e.to_string())?;
    let kind = kept["kind"].as_str().unwrap_or_default().to_string();
    let params = kept.get(match kind.as_str() {
        "manual" => "toolpath",
        other => other,
    });
    let choice =
        SetupChoice { id: setup.id.clone(), name: setup.name.clone(), label: format!("{} (G{})", setup.name, 53 + u32::from(setup.work_offset)) };
    Ok((choice, op.name.clone(), op.tool, kind, params.cloned().unwrap_or(Value::Null)))
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
            editing: None,
            error: None,
        }
    }

    /// The form for changing the facing `id`, in the setup it is in; why not, when the design has
    /// no such facing.
    pub fn edit(app: &SolveApp, id: &str) -> Result<FaceForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "face" {
            return Err(format!("operation {id} is not a facing"));
        }
        let tools = manufacture_tools(&app.session.doc, "face");
        let number = |key: &str| base[key].as_f64();
        let mut form = FaceForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        form.tool = tools.iter().position(|t| t.number == tool).unwrap_or(0);
        form.tools = tools;
        form.pattern = PATTERNS.iter().position(|p| Some(p.0) == base["pattern"].as_str()).unwrap_or(0);
        (form.stepover, form.typed_stepover) = number("stepover").map_or((form.stepover, false), |v| (v, true));
        (form.stepdown, form.multiple_depths) = number("stepdown").map_or((form.stepdown, false), |v| (v, true));
        (form.rpm, form.typed_rpm) = number("rpm").map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = number("feed").map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
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
    /// following the tool: the stepover its diameter, the speed and feed its cutting data. A
    /// facing being changed starts from its parameters as kept, so its heights and the like stay.
    pub fn params(&self) -> Value {
        let mut face = self.editing.as_ref().map_or_else(|| json!({}), |edited| edited.base.clone());
        face["pattern"] = json!(word(&PATTERNS, self.pattern));
        set_or_clear(&mut face, "stepover", self.typed_stepover.then_some(self.stepover));
        set_or_clear(&mut face, "stepdown", self.multiple_depths.then_some(self.stepdown));
        set_or_clear(&mut face, "rpm", self.typed_rpm.then_some(self.rpm));
        set_or_clear(&mut face, "feed", self.typed_feed.then_some(self.feed));
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
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "face": params["face"].take()})),
            None => app.run("cam.op.face", params),
        };
        match made {
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
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_face");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_face"), "edit", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_face_missing"));
                    return;
                }
                // An operation stays in its setup: the row is a choice only for a new one.
                match (&self.editing, setup_labels.first()) {
                    (Some(_), Some(label)) => {
                        ui_kit::value_row(ui, tr("manufacture.field_setup"), label);
                    }
                    _ => {
                        ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_face_setup", &mut self.setup, &setup_labels);
                    }
                }
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
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_face_preview_note"));
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
    /// The drill operation being changed, when the form changes one: its `drill` parameters as
    /// kept. Its holes stay the ones it has.
    editing: Option<Edited>,
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
            editing: None,
            error: None,
        }
    }

    /// The form for changing the drill operation `id`: its tool, cycle, depth, speed and feed, on
    /// the holes it has. Why not, when the design has no such drill operation.
    pub fn edit(app: &SolveApp, id: &str) -> Result<DrillForm, String> {
        let (setup, name, tool, kind, base) = kept_operation(app, id)?;
        if kind != "drill" {
            return Err(format!("operation {id} is not a drill operation"));
        }
        let mut form = DrillForm::new(app);
        form.setups = vec![setup];
        form.setup = 0;
        (form.tool, form.picked_tool) = (form.tools.iter().position(|t| t.number == tool).unwrap_or(0), true);
        form.cycle = CYCLES.iter().position(|c| Some(c.0) == base["cycle"]["type"].as_str()).unwrap_or(0);
        (form.peck, form.typed_peck) = base["cycle"]["peck"].as_f64().map_or((0.0, false), |v| (v, true));
        (form.depth, form.typed_depth) = (base["depth"].as_f64().unwrap_or(0.0), true);
        (form.rpm, form.typed_rpm) = base["rpm"].as_f64().map_or((0.0, false), |v| (v, true));
        (form.feed, form.typed_feed) = base["feed"].as_f64().map_or((0.0, false), |v| (v, true));
        form.editing = Some(Edited { id: json!(id), title: name, base });
        Ok(form)
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
    /// A tap is never picked this way: it is wider than the hole it threads, and whether a hole
    /// is to be tapped is not the model's to say.
    fn fitting_tool(&self, diameter: f64) -> usize {
        let by_width = |a: &(usize, &ToolChoice), b: &(usize, &ToolChoice)| a.1.diameter.total_cmp(&b.1.diameter);
        let drills = || self.tools.iter().enumerate().filter(|(_, tool)| tool.pitch.is_none());
        let fits = drills().filter(|(_, tool)| tool.diameter <= diameter + 1e-6).max_by(by_width);
        fits.or(drills().min_by(by_width)).map_or(0, |(at, _)| at)
    }

    /// The pitch of the tool picked, when it is a tap.
    fn tap(&self) -> Option<f64> {
        self.tools.get(self.tool).and_then(|tool| tool.pitch)
    }

    /// The depth the tool picked takes `group` to when none is typed, and what it is measured
    /// to: a drill's by the holes' bottom ([`drill_depth`]); a tap's to its tip, a few pitches
    /// past a through hole and one pitch short of a blind one's bottom.
    fn suited_depth(&self, group: &HoleGroup) -> (f64, &'static str) {
        match (self.tap(), &group.bottom) {
            (None, _) => drill_depth(group),
            (Some(pitch), HoleBottom::Through) => (group.depth + BREAKTHROUGH + TAP_LEAD * pitch, "tip"),
            (Some(pitch), _) => ((group.depth - pitch).max(pitch), "tip"),
        }
    }

    /// The `cam.op.drill` parameters: the group's centres and top, the depth its bottom asks for
    /// or the one typed, and the cycle. A speed or feed that was not typed is left to the command.
    /// A drill operation being changed keeps its own holes, top and the rest as kept.
    pub fn params(&self) -> Value {
        let cycle = match (self.tap(), word(&CYCLES, self.cycle)) {
            (Some(pitch), _) => json!({"type": "tap", "pitch": pitch}),
            (None, "drill") => json!({"type": "drill", "dwell": 0.0}),
            (None, pecking) => json!({"type": pecking, "peck": self.peck}),
        };
        let mut drill = match (&self.editing, self.picked()) {
            (Some(edited), _) => {
                let mut kept = edited.base.clone();
                // A plain cycle's dwell stays as kept; the form does not show it.
                let plain = cycle["type"] == "drill" && kept["cycle"]["type"] == "drill";
                kept["depth"] = json!(self.depth);
                if !plain {
                    kept["cycle"] = cycle;
                }
                kept
            }
            (None, Some(group)) => {
                let (suited, to) = self.suited_depth(group);
                let depth = if self.typed_depth { self.depth } else { suited };
                json!({"holes": group.holes, "top": group.top, "depth": depth, "depth_to": to, "cycle": cycle})
            }
            (None, None) => json!({"holes": [], "top": null, "depth": self.depth, "depth_to": "tip", "cycle": cycle}),
        };
        set_or_clear(&mut drill, "rpm", self.typed_rpm.then_some(self.rpm));
        // A tap's feed is its pitch times its speed: never typed.
        set_or_clear(&mut drill, "feed", (self.typed_feed && self.tap().is_none()).then_some(self.feed));
        json!({
            "setup": self.setups.get(self.setup).map_or("", |s| s.id.as_str()),
            "tool": self.tools.get(self.tool).map_or(0, |t| t.number),
            "drill": drill,
        })
    }

    /// Finds the setup's holes and works the drilling out, when the design or a field changed,
    /// and shows what was not typed as the holes and the tool give it.
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
                // Another group: its own depth, and the tool that suits it unless one was picked.
                (self.suited_to, self.typed_depth) = (Some(self.group), false);
                if let Some(diameter) = self.picked().map(|group| group.diameter).filter(|_| !self.picked_tool) {
                    self.tool = self.fitting_tool(diameter);
                }
            }
            if let Some(suited) = self.picked().map(|group| self.suited_depth(group).0).filter(|_| !self.typed_depth) {
                self.depth = suited;
            }
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
            if !self.typed_feed || self.tap().is_some() {
                self.feed = preview.feed;
            }
        }
    }

    /// Runs the command: the form again, with the reason, when the drilling can't be worked out
    /// or the command refuses.
    pub fn accept(mut self, app: &mut SolveApp) -> Option<DrillForm> {
        self.refresh(app);
        if self.missing().is_some() || (self.editing.is_none() && self.picked().is_none()) || self.worked.result.is_err() {
            return Some(self);
        }
        let mut params = self.params();
        let made = match &self.editing {
            Some(edited) => app.run("cam.op.edit", json!({"id": edited.id, "tool": params["tool"], "drill": params["drill"].take()})),
            None => app.run("cam.op.drill", params),
        };
        match made {
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
        let ready = missing.is_none() && (self.editing.is_some() || self.picked().is_some()) && self.worked.result.is_ok();
        let time = self.worked.get().map(|(_, preview)| minutes_seconds(preview.cut_minutes));
        // How the depth is measured: for the holes picked, or as the operation being changed keeps it.
        let past = format_length(BREAKTHROUGH, units, decimals);
        let tap = self.tap();
        let measured = match &self.editing {
            _ if tap.is_some() => Some(tr("manufacture.drill_depth_tap").to_string()),
            Some(edited) if edited.base["depth_to"] == "full_diameter" => Some(tr("manufacture.drill_depth_point").to_string()),
            Some(_) => Some(tr("manufacture.drill_depth_tip").to_string()),
            None => self.picked().map(|group| match group.bottom {
                HoleBottom::Through => solidcraft_ui_text::format("manufacture.drill_depth_through", &[("past", &past)]),
                HoleBottom::Point => tr("manufacture.drill_depth_point").to_string(),
                HoleBottom::Flat => tr("manufacture.drill_depth_flat").to_string(),
            }),
        };
        // The holes of an operation being changed: how many it has.
        let kept_holes = self.editing.as_ref().map(|edited| {
            let count = edited.base["holes"].as_array().map_or(0, Vec::len).to_string();
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
        let (title, ok) = window_words(&self.editing, "cmd.cam_op_drill");
        let (closed, action) = ui_kit::Window::new(egui::Id::unique("sc_manufacture_drill"), "hole", &title)
            .placed(pivot, at)
            .fields()
            .actions(ok, tr("ui.cancel"), None)
            .show(ctx, ready, |ui| {
                if let Some(missing) = missing {
                    ui_kit::text(ui, missing, ui_kit::Text::Note("manufacture_drill_missing"));
                    return;
                }
                if let (Some(kept), Some(setup)) = (&kept_holes, setup_labels.first()) {
                    // An operation stays in its setup and on its holes.
                    ui_kit::value_row(ui, tr("manufacture.field_setup"), setup);
                    ui_kit::value_row(ui, tr("manufacture.drill_holes"), kept);
                } else {
                    ui_kit::choice_row(ui, tr("manufacture.field_setup"), "manufacture_drill_setup", &mut self.setup, &setup_labels);
                }
                if kept_holes.is_some() {
                } else if group_labels.is_empty() {
                    // Why no holes could be looked for, or that there are none to drill from here.
                    match self.found.trouble().filter(|why| !why.is_empty()) {
                        Some(why) => refusal(ui, Some(why)),
                        None => {
                            ui_kit::text(ui, tr("manufacture.drill_no_holes"), ui_kit::Text::Note("manufacture_drill_no_holes"));
                        }
                    }
                    return;
                } else {
                    ui_kit::choice_row(ui, tr("manufacture.drill_holes"), "manufacture_drill_holes", &mut self.group, &group_labels);
                }
                let tool = self.tool;
                ui_kit::choice_row(ui, tr("manufacture.field_tool"), "manufacture_drill_tool", &mut self.tool, &tool_labels);
                self.picked_tool |= self.tool != tool;
                // A tap is wider than the hole it threads, and cuts with its own cycle at its pitch.
                let tap = self.tools.get(self.tool).and_then(|tool| tool.pitch);
                let wider = hole_widths.get(self.group).zip(tool_widths.get(self.tool)).is_some_and(|(hole, tool)| *tool > hole + 1e-6);
                if kept_holes.is_none() && tap.is_none() && wider {
                    ui_kit::status_line(ui, tr("manufacture.drill_tool_wider"), ui_kit::StatusTone::Warning);
                }
                if let Some(pitch) = tap {
                    ui_kit::value_row(ui, tr("manufacture.tool_thread_pitch"), &format_length(pitch, units, decimals));
                } else {
                    ui_kit::choice_row(ui, tr("manufacture.drill_cycle"), "manufacture_drill_cycle", &mut self.cycle, &cycles);
                    if word(&CYCLES, self.cycle) != "drill" {
                        self.typed_peck |=
                            ui_kit::row(ui, tr("manufacture.drill_peck"), |ui| ui_kit::length_field(ui, &mut self.peck, units, decimals)).changed();
                    }
                }
                self.typed_depth |=
                    ui_kit::row(ui, tr("manufacture.drill_depth"), |ui| ui_kit::length_field(ui, &mut self.depth, units, decimals)).changed();
                if let Some(measured) = &measured {
                    ui_kit::text(ui, measured, ui_kit::Text::Note("manufacture_drill_measured"));
                }
                if tap.is_some() {
                    // A tap's speed is typed; its feed follows, one pitch a turn.
                    self.typed_rpm |= ui_kit::row(ui, tr("manufacture.face_spindle_speed"), |ui| {
                        ui_kit::number_field(ui, &mut self.rpm, 0.0..=MAX_RPM, 10.0, " rpm")
                    })
                    .changed();
                    let (per, unit) = unit_scale(units);
                    ui_kit::value_row(ui, tr("manufacture.face_feed"), &format!("{:.2} {unit}/min", self.feed * per));
                    ui_kit::text(ui, tr("manufacture.drill_tap_feed"), ui_kit::Text::Note("manufacture_drill_tap_feed"));
                } else {
                    let (rpm, feed) = speed_and_feed(ui, &mut self.rpm, &mut self.feed, units);
                    self.typed_rpm |= rpm;
                    self.typed_feed |= feed;
                }
                if let Some(time) = &time {
                    ui_kit::value_row(ui, tr("manufacture.face_time"), time);
                }
                for note in self.worked.get().map(|(_, preview)| preview.notes.as_slice()).unwrap_or_default() {
                    ui_kit::text(ui, note, ui_kit::Text::Note("manufacture_drill_engine_note"));
                }
                if ready {
                    ui_kit::text(ui, preview_note(&self.editing), ui_kit::Text::Note("manufacture_drill_preview_note"));
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
        file_name(&format!("{design} {setup}"), "program", "nc")
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
    if !matches!(
        id,
        "cam.tool.create"
            | "cam.library.import"
            | "cam.library.export"
            | "cam.setup.create"
            | "cam.op.face"
            | "cam.op.contour"
            | "cam.op.pocket"
            | "cam.op.drill"
            | "cam.simulate"
            | "cam.post"
    ) {
        return false;
    }
    if !super::active(app)
        && let Err(why) = crate::workspace::enter(app, "manufacture")
    {
        app.set_status(why, true);
        return true;
    }
    if id == "cam.library.export" {
        // No form: the save prompt is all it asks.
        export_library(app);
        return true;
    }
    app.manufacture_workspace.dialog = Some(match id {
        "cam.tool.create" => Dialog::Tool(ToolForm::new(app)),
        "cam.library.import" => Dialog::Import(ImportForm::new()),
        "cam.setup.create" => Dialog::Setup(SetupForm::new(app)),
        "cam.op.face" => Dialog::Face(FaceForm::new(app)),
        "cam.op.contour" => Dialog::Contour(ContourForm::new(app)),
        "cam.op.pocket" => Dialog::Pocket(PocketForm::new(app)),
        "cam.op.drill" => Dialog::Drill(DrillForm::new(app)),
        "cam.simulate" => Dialog::Simulate(SimForm::new(app)),
        _ => Dialog::Post(PostForm::new(app)),
    });
    true
}

/// Opens the form that changes a tool, a setup or an operation (the Browser's Edit, or a double
/// click on its row). What has no form, or is of a kind its form does not show, says so on the
/// status line.
pub(crate) fn edit(app: &mut SolveApp, target: &Target) {
    let form = match target {
        Target::Tool(number) => ToolForm::edit(app, *number).map(Dialog::Tool),
        Target::Setup(id) => SetupForm::edit(app, id).map(Dialog::Setup),
        Target::Operation(id) => kept_operation(app, id).and_then(|(.., kind, _)| match kind.as_str() {
            "face" => FaceForm::edit(app, id).map(Dialog::Face),
            "contour" => ContourForm::edit(app, id).map(Dialog::Contour),
            "pocket" => PocketForm::edit(app, id).map(Dialog::Pocket),
            "drill" => DrillForm::edit(app, id).map(Dialog::Drill),
            _ => Err(tr("manufacture.edit_unsupported_operation").to_string()),
        }),
    };
    match form {
        Ok(form) => app.manufacture_workspace.dialog = Some(form),
        Err(why) => app.set_status(why, true),
    }
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
        Dialog::Import(form) => form.show(app, ctx, &units, decimals).map(Dialog::Import),
        Dialog::Setup(form) => form.show(app, ctx, &units, decimals).map(Dialog::Setup),
        Dialog::Face(form) => form.show(app, ctx, &units, decimals).map(Dialog::Face),
        Dialog::Contour(form) => form.show(app, ctx, &units, decimals).map(Dialog::Contour),
        Dialog::Pocket(form) => form.show(app, ctx, &units, decimals).map(Dialog::Pocket),
        Dialog::Drill(form) => form.show(app, ctx, &units, decimals).map(Dialog::Drill),
        Dialog::Simulate(form) => form.show(app, ctx, &units, decimals).map(Dialog::Simulate),
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
