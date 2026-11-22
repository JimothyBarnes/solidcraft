//! Typed contracts for the `cam.*` commands: the Manufacture data a design keeps under
//! `plugins/solidcraft-cam/` (decision 0006 of the CAM work) and posting it as G-code. Tools,
//! toolpaths, WCS, stock and safe heights travel as JSON in `solidcraft_cam`'s own shapes (a tool
//! as in a tool library file, a toolpath as `{start, moves}`), checked as hostile input there.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Read the design's Manufacture data.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamDataParams {}

/// The design's Manufacture data: `{format, tools, setups}`, empty when it has none.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamData {
    pub data: Value,
    pub setups: usize,
    pub tools: usize,
    pub operations: usize,
}

/// Add a tool to the design's tools.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamToolCreateParams {
    /// A tool as in a tool library file; its number must be free in the design.
    pub tool: Value,
}

/// Replace one of the design's tools.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamToolEditParams {
    /// The number of the tool to replace.
    pub number: u32,
    /// The new tool; operations using `number` must still find a tool by that number.
    pub tool: Value,
}

/// Remove one of the design's tools. Refused while an operation uses it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamToolDeleteParams {
    pub number: u32,
}

/// What a tool command left.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamToolWritten {
    pub number: u32,
    /// How many tools the design has now.
    pub tools: usize,
}

/// Create a setup. Every field is optional; the defaults are the usual vise setup: stock 1 mm
/// over the model on the sides and the top, WCS at the top centre of the stock with the model's
/// axes, G54, clearance 25 mm and retract 5 mm above the stock.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupCreateParams {
    #[serde(default)]
    pub name: Option<String>,
    /// A machine definition's id; none for a generic machine.
    #[serde(default)]
    pub machine: Option<String>,
    /// The stock's material, as the tools' cutting data names it (`aluminium`); feeds come from
    /// it. An empty string clears it.
    #[serde(default)]
    pub material: Option<String>,
    /// `{origin, orientation}` as in the Manufacture data.
    #[serde(default)]
    pub wcs: Option<Value>,
    /// 1 to 6 for G54 to G59.
    #[serde(default)]
    pub work_offset: Option<u8>,
    /// `{kind: relative_box | fixed_box | relative_cylinder | from_body | from_setup, …}`.
    #[serde(default)]
    pub stock: Option<Value>,
    /// The bodies to machine, by stable name (none: every visible body).
    #[serde(default)]
    pub model_bodies: Option<Vec<String>>,
    /// Bodies to keep clear of, by stable name.
    #[serde(default)]
    pub fixture_bodies: Option<Vec<String>>,
    /// `{clearance, retract}` in mm above the stock top.
    #[serde(default)]
    pub safe_heights: Option<Value>,
}

/// Change a setup: the fields given replace the setup's.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupEditParams {
    /// The setup's id (`setup-1`, …).
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub machine: Option<String>,
    /// The stock's material, as the tools' cutting data names it (`aluminium`); feeds come from
    /// it. An empty string clears it.
    #[serde(default)]
    pub material: Option<String>,
    #[serde(default)]
    pub wcs: Option<Value>,
    #[serde(default)]
    pub work_offset: Option<u8>,
    #[serde(default)]
    pub stock: Option<Value>,
    #[serde(default)]
    pub model_bodies: Option<Vec<String>>,
    #[serde(default)]
    pub fixture_bodies: Option<Vec<String>>,
    #[serde(default)]
    pub safe_heights: Option<Value>,
}

/// Delete a setup and its operations. Refused while a later setup takes its stock from it.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupDeleteParams {
    pub id: String,
}

/// Measure a setup on the model: its stock box, work coordinate system and safe heights.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupMeasureParams {
    pub id: String,
}

/// A setup placed on the model, measured on the bodies' exact boxes (never a display mesh).
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSetupMeasured {
    pub id: String,
    /// The bodies measured: the setup's model bodies, or every visible body when it lists none.
    pub bodies: Vec<String>,
    /// Their box in model coordinates: `{min: [x, y, z], max: [x, y, z]}`, mm.
    pub model_box: Value,
    /// `{origin, x_axis, y_axis, z_axis}` in model coordinates, then `model` and `stock` boxes and
    /// `clearance_z`, `retract_z` in the WCS, mm.
    pub placement: Value,
}

/// Find the holes a setup can drill.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupHolesParams {
    pub id: String,
}

/// How a hole ends.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HoleBottom {
    /// It comes out of the part.
    Through,
    /// A flat floor at the end of its wall.
    Flat,
    /// A drill's point below the end of its wall.
    Point,
}

/// Holes of one diameter, one top and one depth, in a setup's WCS, mm.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct HoleGroup {
    pub diameter: f64,
    /// The Z of the upper end of the holes' walls.
    pub top: f64,
    /// The length of the walls down from there.
    pub depth: f64,
    pub bottom: HoleBottom,
    /// The holes' centres, `[x, y]`.
    pub holes: Vec<[f64; 2]>,
}

/// The holes of a setup's bodies that open along its Z, found on their exact faces (never a
/// display mesh) and grouped, the group with the most holes first.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSetupHoles {
    pub id: String,
    /// How many holes there are in all.
    pub holes: usize,
    pub groups: Vec<HoleGroup>,
}

/// What a setup command left.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSetupWritten {
    pub id: String,
    /// How many setups the design has now.
    pub setups: usize,
}

/// Add a hand-made toolpath operation to a setup.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpManualParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers.
    pub tool: u32,
    /// `{start: [x, y, z], moves: [{kind: rapid | linear | arc | dwell | tool_change | spindle |
    /// spindle_stop | coolant | drill, …}]}`, points as `[x, y, z]` in mm, feeds in mm/min. A
    /// `drill` is one hole: `{to, r, retract, cycle: {type: drill, dwell} | {type: peck | chip_break,
    /// peck}, feed}`.
    pub toolpath: Value,
}

/// Delete an operation.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpDeleteParams {
    /// The operation's id (`op-1`, …).
    pub id: String,
}

/// Add a drill operation to a setup (decision 0009 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpDrillParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a drill, spot drill, reamer or flat end mill.
    pub tool: u32,
    /// `{holes: [[x, y], …] in the WCS, depth, cycle: {type: drill, dwell} | {type: peck |
    /// chip_break, peck}, order?: nearest | as_given, top?: Z (default the stock top), depth_to?:
    /// tip | full_diameter, r_above?: mm above the top (default the setup's retract), between?:
    /// r_plane | clearance, rpm?, feed?}`, mm and mm/min.
    pub drill: Value,
}

/// Add a facing operation to a setup (decision 0010 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpFaceParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a face mill, flat end mill or bull-nose end mill.
    pub tool: u32,
    /// `{top?: Z (default the stock top), bottom?: Z (default the model's top), stepdown?: mm
    /// (default one level), stepover?: mm (default 70% of the tool's diameter), clear?: mm past the
    /// stock (default 2), pattern?: zig_zag | one_way, rpm?, feed?}`; every field has a default.
    #[serde(default)]
    pub face: Option<Value>,
}

/// Work out one operation's toolpath.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpToolpathParams {
    /// The operation's id.
    pub id: String,
}

/// One operation's toolpath, worked out from its parameters now (never stored), and its measures.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamOpToolpath {
    pub id: String,
    pub setup: String,
    /// `{start, moves}` in the setup's WCS, mm and mm/min; holes are one `drill` move each.
    pub toolpath: Value,
    /// Length cut at feed and moved at rapid (holes as the generic control runs them), mm.
    pub feed_length: f64,
    pub rapid_length: f64,
    /// Time cutting and dwelling, minutes; rapids depend on the machine and are left out.
    pub cut_minutes: f64,
    /// The box the tool tip passes through, in the WCS.
    pub min: [f64; 3],
    pub max: [f64; 3],
    /// What the strategy worked out (a drill: rpm, feed, bottom, r, clearance; a facing: rpm, feed,
    /// plunge_feed, levels, passes, stepover); empty for a hand-made toolpath.
    pub numbers: Value,
    pub notes: Vec<String>,
}

/// What an operation command left.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamOpWritten {
    pub id: String,
    pub setup: String,
    /// How many operations the setup has now.
    pub operations: usize,
}

/// Post a setup's operations as one program.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamPostParams {
    /// The setup's id.
    pub setup: String,
    /// A built-in post: `rs274`, `fanuc_mill` or `haas_mill`.
    pub post: String,
    /// Default 1.
    #[serde(default)]
    pub program_number: Option<u32>,
    /// Default: the setup's name.
    #[serde(default)]
    pub program_name: Option<String>,
    /// `mm` (default) or `inch`.
    #[serde(default)]
    pub units: Option<String>,
    /// Also write the program to this file.
    #[serde(default)]
    pub path: Option<String>,
    /// Replace a file that is already there (default false).
    #[serde(default)]
    pub overwrite: bool,
}

/// A posted program and what reading it back found. The program is never "safe to run": it says
/// so in its first lines, and must be proven on the machine first.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamPosted {
    pub post: String,
    pub text: String,
    pub lines: usize,
    /// Moves written (after arcs were split or turned into lines), all read back within tolerance.
    pub moves: usize,
    pub machine_retracts: usize,
    /// The largest position difference read-back accepted, mm.
    pub tolerance_mm: f64,
    pub notes: Vec<String>,
    /// The file the program was written to, when a path was given.
    pub path: Option<String>,
}

/// Write the design's tools to a tool library file.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamLibraryExportParams {
    /// A `.json` file.
    pub path: String,
    /// The library's name; default the design's.
    #[serde(default)]
    pub name: Option<String>,
    /// Replace an existing file. Default false: an existing file is refused.
    #[serde(default)]
    pub overwrite: bool,
}

/// What an export wrote.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamLibraryExported {
    pub path: String,
    pub tools: usize,
    pub bytes: usize,
}

/// Read a tool library file and compare it with the design's tools.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamLibraryReadParams {
    /// A tool library `.json` file.
    pub path: String,
}

/// How a library tool compares with the design's tool of the same number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CamLibraryToolStatus {
    /// The design has no tool of that number.
    New,
    /// The design's copy is the same.
    Same,
    /// The design's copy differs: importing with `replace` updates it.
    Differs,
}

/// One tool of a library.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamLibraryTool {
    pub number: u32,
    pub name: String,
    pub kind: Value,
    /// mm.
    pub diameter: f64,
    pub status: CamLibraryToolStatus,
}

/// A library's tools.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamLibraryTools {
    pub name: String,
    pub tools: Vec<CamLibraryTool>,
}

/// Copy tools from a library file into the design.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamLibraryImportParams {
    pub path: String,
    /// The tool numbers to copy; default all.
    #[serde(default)]
    pub numbers: Option<Vec<u32>>,
    /// Update design tools that differ from the library's. Default false: a difference is refused.
    #[serde(default)]
    pub replace: bool,
}

/// What an import did, by tool number.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamLibraryImported {
    pub added: Vec<u32>,
    pub updated: Vec<u32>,
    pub same: Vec<u32>,
}
