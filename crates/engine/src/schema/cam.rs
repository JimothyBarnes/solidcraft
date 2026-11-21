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
