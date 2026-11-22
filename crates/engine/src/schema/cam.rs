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

/// Read the outline of a setup's bodies at a height.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupOutlineParams {
    pub id: String,
    /// The height, as Z in the setup's WCS; left out, or below the model, the model's bottom.
    #[serde(default)]
    pub z: Option<f64>,
}

/// One closed loop of an outline, in a setup's WCS, mm.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct OutlineLoop {
    /// Its lines and arcs in order, counter-clockwise, as a sketch profile writes them: `{type:
    /// line, a: [x, y], b: [x, y]}` or `{type: arc, center: [x, y], radius, start, sweep}` (angles
    /// in radians, a positive sweep counter-clockwise).
    pub segments: Value,
    pub length: f64,
    /// The area it encloses.
    pub area: f64,
    /// The box round it, `[x, y]` of its least and greatest corners.
    pub min: [f64; 2],
    pub max: [f64; 2],
}

/// The outline of a setup's bodies at a height: the outer loop of every piece of their section
/// there, read from their exact faces (never a display mesh). What a contour goes round.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSetupOutline {
    pub id: String,
    /// The Z the outline was read at.
    pub z: f64,
    pub loops: Vec<OutlineLoop>,
}

/// Find the closed pockets a setup can cut.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSetupPocketsParams {
    pub id: String,
}

/// A closed pocket of a setup's bodies, in the setup's WCS, mm.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct FoundPocket {
    /// A point on its floor, `[x, y]`: what a pocket operation names it by.
    pub at: [f64; 2],
    /// The Z of its floor.
    pub floor: f64,
    /// How far its floor is below the part's top.
    pub depth: f64,
    /// About how far across the widest circle that fits in it is (from a search, never more than
    /// what fits): a tool must be narrower.
    pub widest: f64,
    /// About how wide a tool can be and still pass everywhere in it: through its necks, and
    /// between its islands and its walls. A wider tool that fits clears only part of it.
    pub passes: f64,
    /// The sizes along X and Y of the box round its wall.
    pub size: [f64; 2],
    /// How many islands stand in it.
    pub islands: usize,
    /// The area of the air above its floor, the islands taken out.
    pub area: f64,
}

/// The closed pockets of a setup's bodies, found on their exact faces (never a display mesh): a
/// flat floor facing up, walls all round, and nothing seen over it from places along its wall
/// (a pocket is checked exactly, for a fillet at the foot of its walls too, when it is cut).
/// The largest first.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSetupPockets {
    pub id: String,
    pub pockets: Vec<FoundPocket>,
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

/// Change an operation: the fields given replace the operation's.
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpEditParams {
    /// The operation's id (`op-1`, …).
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers, of a kind the operation can use.
    #[serde(default)]
    pub tool: Option<u32>,
    /// Leave the operation out of toolpaths and programs (true), or put it back (false).
    #[serde(default)]
    pub suppressed: Option<bool>,
    /// A drill operation's parameters, whole, as `cam.op.drill` takes them.
    #[serde(default)]
    pub drill: Option<Value>,
    /// A facing operation's parameters, whole, as `cam.op.face` takes them.
    #[serde(default)]
    pub face: Option<Value>,
    /// A contour operation's parameters, whole, as `cam.op.contour` takes them.
    #[serde(default)]
    pub contour: Option<Value>,
    /// A pocket operation's parameters, whole, as `cam.op.pocket` takes them.
    #[serde(default)]
    pub pocket: Option<Value>,
    /// A chamfer's parameters, whole, as `cam.op.chamfer` takes them.
    #[serde(default)]
    pub chamfer: Option<Value>,
    /// A hand-made operation's toolpath, whole, as `cam.op.manual` takes it.
    #[serde(default)]
    pub toolpath: Option<Value>,
}

/// Add a drill operation to a setup (decision 0009 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpDrillParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a drill, spot drill, reamer, tap or flat end mill.
    pub tool: u32,
    /// `{holes: [[x, y], …] in the WCS, depth, cycle: {type: drill, dwell} | {type: peck |
    /// chip_break, peck} | {type: tap, pitch} (with a tap, at its own pitch; it takes a speed and
    /// no feed), order?: nearest | as_given, top?: Z (default the stock top), depth_to?:
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

/// Add a contour operation to a setup (decision 0013 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpContourParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a flat or bull-nose end mill.
    pub tool: u32,
    /// `{top?: Z (default the stock top), bottom?: Z (default the model's bottom), stepdown?: mm
    /// (default half the tool's diameter), leave?: mm of stock left on the wall (default 0),
    /// direction?: climb | conventional, clear?: mm past the stock where the tool goes down
    /// (default 2), lead?: mm, the radius of the arcs on to the wall and off it (default a tenth
    /// of the tool's diameter; 0 for straight in and out), tabs?: {count, width?: mm (default 6),
    /// height?: mm above the bottom (default 2)} bridges of stock left round each part (default
    /// none), rpm?, feed?}`; every field has a default.
    #[serde(default)]
    pub contour: Option<Value>,
}

/// Add a pocket operation to a setup (decision 0014 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpPocketParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a flat or bull-nose end mill that fits the pockets.
    pub tool: u32,
    /// `{pockets: [[x, y], …] a point on the floor of each pocket, in the WCS (as
    /// `cam.setup.pockets` gives them), top?: Z (default the stock top), stepdown?: mm (default
    /// half the tool's diameter), stepover?: mm (default 40% of the tool's diameter, at most
    /// half), leave?: mm of stock left on the walls (default 0), direction?: climb | conventional,
    /// ramp_angle?: radians (default 3 degrees), rpm?, feed?}`.
    pub pocket: Value,
}

/// Add a chamfer to a setup (decision 0018 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamOpChamferParams {
    /// The setup's id.
    pub setup: String,
    #[serde(default)]
    pub name: Option<String>,
    /// One of the design's tool numbers: a chamfer mill or a spot drill.
    pub tool: u32,
    /// `{width?: mm, the chamfer seen from above (default 0.5), tip_below?: mm the tool's tip is
    /// below the chamfer's lower edge (default 1), top?: Z of the edge that is chamfered (default
    /// the model's top), direction?: climb | conventional, lead?: mm, the radius of the arcs on to
    /// the wall and off it, rpm?, feed?}`; every field has a default.
    #[serde(default)]
    pub chamfer: Option<Value>,
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
    /// plunge_feed, levels, passes, stepover; a contour: rpm, feed, plunge_feed, top, levels,
    /// stepdown, offset, round, entries; a pocket: rpm, feed, plunge_feed, ramp_feed, top, levels
    /// and rings for each pocket, stepdown, stepover, offset); empty for a hand-made toolpath.
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

/// Simulate a setup's operations on its stock (decision 0015 of the CAM work).
#[derive(Debug, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CamSimulateParams {
    /// The setup's id.
    pub setup: String,
    /// The side of a cell of the grid the stock is held on, mm; left out, the stock's longer
    /// side over 400, kept between 0.05 and 1.
    #[serde(default)]
    pub cell: Option<f64>,
}

/// What one operation did to the stock in a simulation.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CamSimulatedOperation {
    pub id: String,
    pub name: String,
    /// The volume it removed, mm³.
    pub removed: f64,
    /// How many of its moves cut into the part, and how many of its rapids passed through
    /// material.
    pub gouges: usize,
    pub rapid_hits: usize,
    /// Why it was left out, when its toolpath could not be worked out or its tool can't be shown.
    pub skipped: Option<String>,
}

/// A place a simulation found a cut into the part, or a rapid through material.
#[derive(Clone, Debug, PartialEq, Serialize, JsonSchema)]
pub struct CamSimulatedHit {
    /// The operation's id and which of its moves (from 0).
    pub operation: String,
    #[serde(rename = "move")]
    pub move_index: usize,
    /// Where, `[x, y, z]` in the setup's WCS: the tool's height over the cell where it was
    /// deepest.
    pub at: [f64; 3],
    /// How far below the part (a gouge) or below the material's top (a rapid), mm.
    pub depth: f64,
}

/// A setup's operations cut from its stock on a grid of heights, and what was found: a check as
/// fine as its cells, never a proof that a program is safe to run.
#[derive(Debug, Serialize, JsonSchema)]
pub struct CamSimulated {
    pub setup: String,
    /// The side of a cell, mm, the cells along X and Y, and how far below the part or the
    /// material's top counts, mm.
    pub cell: f64,
    pub cells: [usize; 2],
    pub tolerance: f64,
    /// The stock's volume before, and the volume removed, mm³.
    pub stock_volume: f64,
    pub removed: f64,
    /// The volume still standing above the part where there is part under it, and standing
    /// where there is none, mm³.
    pub over_part: f64,
    pub beside_part: f64,
    /// How far below the stock's bottom the lowest cut went, mm (0 when none did).
    pub below_stock: f64,
    pub operations: Vec<CamSimulatedOperation>,
    /// How many moves cut into the part, and the first hundred of them.
    pub gouge_count: usize,
    pub gouges: Vec<CamSimulatedHit>,
    /// How many rapids passed through material, and the first hundred of them.
    pub rapid_hit_count: usize,
    pub rapid_hits: Vec<CamSimulatedHit>,
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
