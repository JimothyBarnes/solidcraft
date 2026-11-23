//! `cam.*`: the Manufacture workspace's commands. Its data lives in the design as plugin data
//! under `plugins/solidcraft-cam/` (decision 0006 of the CAM work), written here as that folder's
//! built-in owner through the same storage code plugins use, so its size limits and metadata
//! are the same. Every change is one undoable edit; a design whose Manufacture data can't be read
//! keeps it as it is, and changing commands refuse rather than overwrite it.
use serde::de::DeserializeOwned;
use serde_json::Value;
use solidcraft_cam::{
    BUILTIN_POSTS, Bounds, BoxAnchor, BoxOffsets, BoxSide, ChamferEdges, ChamferOp, ContourOp, ContourSide, DrillOp, FaceOp, LengthUnit,
    MANUFACTURE_DATA_OWNER, MANUFACTURE_FORMAT, ManufactureData, Operation, OperationKind, PocketOp, PocketShape, Post, ProgramContext, ProgramTool,
    SafeHeights, Setup, SetupKind, Stock, Tool, Toolpath, Wcs, WcsOrientation, WcsOrigin,
};
use solidcraft_doc::plugin_data::DATA;
use solidcraft_geom::{Loop2Geom as _, Region2Geom as _, Seg2Geom as _};

use super::CommandSpec;
use crate::plugin::storage::{self, StorageCaller};
use crate::schema::cam::{
    CamData, CamDataParams, CamOpChamferParams, CamOpContourParams, CamOpDeleteParams, CamOpDrillParams, CamOpDuplicateParams, CamOpEditParams,
    CamOpFaceParams, CamOpManualParams, CamOpMoveParams, CamOpPocketParams, CamOpToolpath, CamOpToolpathParams, CamOpWritten, CamPostParams,
    CamPosted, CamSetupCreateParams, CamSetupDeleteParams, CamSetupEditParams, CamSetupHoles, CamSetupHolesParams, CamSetupMeasureParams,
    CamSetupMeasured, CamSetupOpenings, CamSetupOpeningsParams, CamSetupOutline, CamSetupOutlineParams, CamSetupPockets, CamSetupPocketsParams,
    CamSetupWritten, CamSimulateParams, CamSimulated, CamSimulatedHit, CamSimulatedOperation, CamToolCreateParams, CamToolDeleteParams,
    CamToolEditParams, CamToolWritten, FoundOpening, FoundPocket, HoleBottom, HoleGroup, OutlineLoop,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session, params::bad};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.data", "Manufacture Data", crate::checked!("cam.data", data)).icon("inspect").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamDataParams>::new()).output(output_schema_for::<CamData>)
        .params("→ data: the design's Manufacture data {format, tools, setups} (empty when it has none), setups, tools, operations: counts"),
    CommandSpec::new("cam.tool.create", "Add Tool", crate::checked!("cam.tool.create", tool_create)).icon("cam_tool_new").doc_only()
        .contract(&TypedContract::<CamToolCreateParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("tool: a tool as in a tool library file, its number free in the design → number, tools: how many the design has"),
    CommandSpec::new("cam.tool.edit", "Edit Tool", crate::checked!("cam.tool.edit", tool_edit)).icon("edit").doc_only()
        .contract(&TypedContract::<CamToolEditParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("number: the tool to replace, tool: the new tool (operations must still find their tool numbers) → number, tools"),
    CommandSpec::new("cam.tool.delete", "Delete Tool", crate::checked!("cam.tool.delete", tool_delete)).icon("delete").doc_only()
        .contract(&TypedContract::<CamToolDeleteParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("number → removes one of the design's tools; refused while an operation uses it"),
    CommandSpec::new("cam.tool.list", "Tool Library", crate::checked!("cam.tool.list", tool_list)).icon("library").noundo().read_only().doc_only()
        .contract(&TypedContract::<crate::schema::cam::CamToolListParams>::new()).output(output_schema_for::<crate::schema::cam::CamToolList>)
        .params("search?: words to look for in a tool's number (T3), name, kind and diameter, every one of them, in any case (default: every tool) → tools: [{number, name, kind, diameter, flutes, flute_length, overall_length, stick_out, operations: how many of the design's operations use it}] by number, lengths in mm; of: how many tools the design has"),
    CommandSpec::new("cam.setup.create", "New Setup", crate::checked!("cam.setup.create", setup_create)).icon("cam_setup").doc_only()
        .contract(&TypedContract::<CamSetupCreateParams>::new()).output(output_schema_for::<CamSetupWritten>)
        .params("name?, machine?, wcs?, work_offset?: 1-6 (G54-G59), stock?, model_bodies?, fixture_bodies?, safe_heights? → id; defaults: stock 1 mm over on the sides and top, WCS at the stock's top centre, G54, clearance 25 mm, retract 5 mm"),
    CommandSpec::new("cam.setup.edit", "Edit Setup", crate::checked!("cam.setup.edit", setup_edit)).icon("edit").doc_only()
        .contract(&TypedContract::<CamSetupEditParams>::new()).output(output_schema_for::<CamSetupWritten>)
        .params("id, then any of name, machine, wcs, work_offset, stock, model_bodies, fixture_bodies, safe_heights → replaces those fields"),
    CommandSpec::new("cam.setup.delete", "Delete Setup", crate::checked!("cam.setup.delete", setup_delete)).icon("delete").doc_only()
        .contract(&TypedContract::<CamSetupDeleteParams>::new()).output(output_schema_for::<CamSetupWritten>)
        .params("id → removes the setup and its operations; refused while a later setup takes its stock from it"),
    CommandSpec::new("cam.setup.measure", "Measure Setup", crate::checked!("cam.setup.measure", setup_measure)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamSetupMeasureParams>::new()).output(output_schema_for::<CamSetupMeasured>)
        .params("id: the setup → bodies measured, model_box in model coordinates, placement: WCS origin and axes in model coordinates, model and stock boxes and clearance_z, retract_z in the WCS (mm), from the bodies' exact boxes"),
    CommandSpec::new("cam.setup.holes", "Find Holes", crate::checked!("cam.setup.holes", setup_holes)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamSetupHolesParams>::new()).output(output_schema_for::<CamSetupHoles>)
        .params("id: the setup → groups: [{diameter, top, depth, bottom: through | flat | point, holes: [[x, y]]}] in the setup's WCS (mm): the whole cylinder walls of its bodies that run along its Z with nothing above them, from the bodies' exact faces; holes of one diameter, top and depth are a group, the group with the most holes first"),
    CommandSpec::new("cam.setup.outline", "Find Outline", crate::checked!("cam.setup.outline", setup_outline)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamSetupOutlineParams>::new()).output(output_schema_for::<CamSetupOutline>)
        .params("id: the setup, z?: a height in its WCS (default the model's bottom) → z, loops: [{segments, length, area, min, max}] in the setup's WCS (mm): the outer loop of every piece of its bodies' section at that height, lines and arcs from the bodies' exact faces, counter-clockwise; refused when a body is wider above that height than at it"),
    CommandSpec::new("cam.setup.pockets", "Find Pockets", crate::checked!("cam.setup.pockets", setup_pockets)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamSetupPocketsParams>::new()).output(output_schema_for::<CamSetupPockets>)
        .params("id: the setup → pockets: [{at: [x, y], floor, depth, widest, passes, size: [x, y], islands, area}] in the setup's WCS (mm): the closed pockets of its bodies, from their exact faces, the largest first: a flat floor facing up, walls all round and nothing seen over it; `at` is a point on the floor, which is what cam.op.pocket names a pocket by, `widest` how far across the widest circle that fits is, and `passes` how wide a tool can be and still pass everywhere in it"),
    CommandSpec::new("cam.setup.openings", "Find Openings", crate::checked!("cam.setup.openings", setup_openings)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamSetupOpeningsParams>::new()).output(output_schema_for::<CamSetupOpenings>)
        .params("id: the setup → openings: [{at: [x, y], bottom, through, depth, widest, size: [x, y], diameter?, islands, area}] in the setup's WCS (mm): what an inside contour (cam.op.contour with side inside) can go round, from the bodies' exact faces: the openings through the part, then its closed pockets, each kind the largest first; `at` is a point in the opening, which is what the contour names it by, `bottom` the Z it is cut down to unless told otherwise, `widest` how far across the widest circle that fits is, and `diameter` is given when the wall is a circle"),
    CommandSpec::new("cam.setup.slots", "Find Slots", crate::checked!("cam.setup.slots", setup_slots)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<crate::schema::cam::CamSetupSlotsParams>::new()).output(output_schema_for::<crate::schema::cam::CamSetupSlots>)
        .params("id: the setup → slots: [{at: [x, y], bottom, through, depth, width, length, curved}] in the setup's WCS (mm): its openings through the part and its closed pockets that are slots, from the bodies' exact faces: two half-round ends as wide as the slot with two straight sides, or two arcs about one centre, between them; `at` is a point in the slot, which is what cam.op.slot names it by; the ones through the part first, each kind the largest first"),
    CommandSpec::new("cam.op.manual", "Hand-made Toolpath", crate::checked!("cam.op.manual", op_manual)).icon("edit").doc_only()
        .contract(&TypedContract::<CamOpManualParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: one of the design's numbers, toolpath: {start, moves} in mm and mm/min → id of the new operation"),
    CommandSpec::new("cam.op.drill", "Drill", crate::checked!("cam.op.drill", op_drill)).icon("cam_drill").doc_only()
        .contract(&TypedContract::<CamOpDrillParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a drill, spot drill, reamer, tap or flat end mill, drill: {holes: [[x, y]] in the WCS, depth, cycle ({type: drill, dwell} | {type: peck | chip_break, peck} | {type: ream, dwell}: fed in and fed back out, for a reamer; a tap's is {type: tap, pitch}, its own pitch, and it takes a speed and no feed), order?, top?, depth_to?, r_above?, between?, rpm?, feed?} → id of the new operation; its toolpath is worked out when asked for"),
    CommandSpec::new("cam.op.face", "Face", crate::checked!("cam.op.face", op_face)).icon("cam_face").doc_only()
        .contract(&TypedContract::<CamOpFaceParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a face mill, flat end mill or bull-nose end mill, face?: {top?, bottom?, stepdown?, stepover?, clear?, pattern?: zig_zag | one_way, rpm?, feed?} → id of the new operation: the stock's top down to the model's top in straight passes along X"),
    CommandSpec::new("cam.op.contour", "Contour", crate::checked!("cam.op.contour", op_contour)).icon("cam_contour").doc_only()
        .contract(&TypedContract::<CamOpContourParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a flat or bull-nose end mill, contour?: {top?, bottom?, stepdown?, leave?, direction?: climb | conventional, clear?, lead?: mm (the arcs on to the wall and off it; 0 for none), tabs?: {count, width?, height?}, passes?: how many passes sideways at each level, the furthest from the wall first (outside; default 1), stepover?: mm between them (default 0.4 of the tool's diameter), side?: outside | inside, openings?: [[x, y], …] (inside: a point in each opening to go round), ramp_angle?: radians (inside: default 3 degrees), rpm?, feed?} → id of the new operation: round the outside of the part at its outline (read from the model at the cut's bottom when the toolpath is worked out), the tool going down beside the stock; or, inside, round the inside of each opening's wall (a pocket's, a window's, a round hole's: read from the model where its point is, down to the floor under the point or through the part), the tool going down a ramp along the wall"),
    CommandSpec::new("cam.op.bore", "Bore", crate::checked!("cam.op.bore", op_bore)).icon("cam_bore").doc_only()
        .contract(&TypedContract::<CamOpContourParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a flat or bull-nose end mill narrower than the holes, contour: {openings: [[x, y], …] (a point in each hole, in the WCS), bottom?: Z (default the hole's floor, or through the part), top?, stepdown?, leave?, direction?, lead?, ramp_angle?, rpm?, feed?} → id of the new operation, named Bore: an inside contour (as cam.op.contour with side inside) for round holes wider than the tool, cut down a helix along their wall and once round at each level"),
    CommandSpec::new("cam.op.pocket", "Pocket", crate::checked!("cam.op.pocket", op_pocket)).icon("cam_pocket").doc_only()
        .contract(&TypedContract::<CamOpPocketParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a flat or bull-nose end mill, pocket: {pockets: [[x, y]] a point on each pocket's floor, top?, stepdown?, stepover?, leave?, direction?: climb | conventional, ramp_angle?, rpm?, feed?} → id of the new operation: each pocket (read from the model at its point when the toolpath is worked out) cleared in rings from the inside out, the tool ramping down the innermost ring"),
    CommandSpec::new("cam.op.slot", "Slot", crate::checked!("cam.op.slot", op_slot)).icon("cam_slot").doc_only()
        .contract(&TypedContract::<crate::schema::cam::CamOpSlotParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a flat or bull-nose end mill no wider than the slots, slot: {slots: [[x, y]] a point in each slot, top?, bottom?: Z (default each slot's floor, or the part's bottom where it goes through), stepdown?, ramp_angle?, rpm?, feed?} → id of the new operation: each slot (read from the model at its point when the toolpath is worked out) milled along its middle line, the tool going down along the line to and fro no steeper than the ramp angle and once along it at each level; a tool narrower than a slot leaves the same on each wall, which the toolpath's notes say"),
    CommandSpec::new("cam.op.chamfer", "Chamfer", crate::checked!("cam.op.chamfer", op_chamfer)).icon("cam_chamfer").doc_only()
        .contract(&TypedContract::<CamOpChamferParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a chamfer mill or a spot drill, chamfer?: {width?: mm seen from above (default 0.5), tip_below?: mm the tool's tip is below the chamfer (default 1), top?: Z of the edge (default the model's top), edges?: outside | all, direction?: climb | conventional, lead?, rpm?, feed?} → id of the new operation: once round the part's wall (read from the model at the chamfer's lower edge when the toolpath is worked out), the tool's cone cutting the top outside edge; with edges all, round the inside of every opening in the part's top and round what stands in it as well, but for an opening too narrow for the tool or not open as far down as its point, which is left sharp and said"),
    CommandSpec::new("cam.op.toolpath", "Operation Toolpath", crate::checked!("cam.op.toolpath", op_toolpath_command)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamOpToolpathParams>::new()).output(output_schema_for::<CamOpToolpath>)
        .params("id: an operation → toolpath worked out now from its parameters, the setup's placement and the tool, with feed_length, rapid_length, cut_minutes, min, max, numbers, notes"),
    CommandSpec::new("cam.op.edit", "Edit Operation", crate::checked!("cam.op.edit", op_edit)).icon("edit").doc_only()
        .contract(&TypedContract::<CamOpEditParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("id, then any of name, tool, suppressed, and the operation's own parameters whole: drill (a drill operation), face (a facing), contour (a contour), pocket (a pocket operation), chamfer (a chamfer), slot (a slot operation) or toolpath (a hand-made one) → replaces those; parameters of another kind of operation are refused"),
    CommandSpec::new("cam.op.delete", "Delete Operation", crate::checked!("cam.op.delete", op_delete)).icon("delete").doc_only()
        .contract(&TypedContract::<CamOpDeleteParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("id → removes the operation"),
    CommandSpec::new("cam.op.move", "Move Operation", crate::checked!("cam.op.move", op_move)).icon("reorder").doc_only()
        .contract(&TypedContract::<CamOpMoveParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("id, to: its place among its setup's operations, from 0 (past the end is the end) → the operation runs there in the setup's order"),
    CommandSpec::new("cam.op.duplicate", "Duplicate Operation", crate::checked!("cam.op.duplicate", op_duplicate)).icon("copy").doc_only()
        .contract(&TypedContract::<CamOpDuplicateParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("id, name? → a copy of the operation right after it, with an id of its own (its name the operation's with (2) when none is given)"),
    CommandSpec::new("cam.simulate", "Simulate", crate::checked!("cam.simulate", simulate)).icon("cam_simulate").noundo().read_only()
        .contract(&TypedContract::<CamSimulateParams>::new()).output(output_schema_for::<CamSimulated>)
        .params("setup: id, cell?: mm → the setup's operations cut from its stock on a grid of heights: stock_volume, removed, over_part, beside_part (mm3), operations: [{id, name, removed, gouges, rapid_hits, body_hits, skipped?}], gouges and rapid_hits: [{operation, move, at: [x, y, z], depth}] with their counts, body_hits: [{operation, move, part: shank | holder, at, depth}] where a tool's shank or holder is in material, below_stock; the part's heights are read from the bodies' exact faces; a check as fine as its cells (cell, cells, tolerance), never a proof that a program is safe to run"),
    CommandSpec::new("cam.setup.sheet", "Setup Sheet", crate::checked!("cam.setup.sheet", setup_sheet)).icon("cam_setup_sheet").noundo().read_only()
        .contract(&TypedContract::<crate::schema::cam::CamSetupSheetParams>::new()).output(output_schema_for::<crate::schema::cam::CamSetupSheet>)
        .params("setup: id, units?: mm | inch (default the design's), date?: text to write on it, path?: also write it to this .html file, overwrite?: replace an existing file (default false) → html: one page for the shop with the stock, the work origin, the tools (how far each sticks out, the lowest its tip goes), the operations in order with speeds, feeds and times, the setup from above, and what to check (the operations' and the simulation's notes, a tool that goes deeper than it sticks out)"),
    CommandSpec::new("cam.post", "Post Process", crate::checked!("cam.post", post)).icon("cam_post").noundo().read_only()
        .contract(&TypedContract::<CamPostParams>::new()).output(output_schema_for::<CamPosted>)
        .params("setup: id, post: rs274 | fanuc_mill | haas_mill, program_number?: default 1, program_name?, units?: mm | inch, path?: also write it to this file, overwrite?: replace an existing file (default false), optional_stop?: write the post's optional stop (M1) before every tool change but the first, where the post has one (default true), comments?: write the tools the program takes, each operation's name and each tool's name as comments (default true) → text: the program, read back and compared move by move with what was written; never safe to run unproven: prove it on the machine (air cut, single block) first"),
];

/// The built-in owner of `plugins/solidcraft-cam/`.
fn owner() -> StorageCaller {
    StorageCaller {
        id: MANUFACTURE_DATA_OWNER.into(),
        name: "Manufacture".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        data_format: MANUFACTURE_FORMAT,
    }
}

fn parse<T: DeserializeOwned>(cmd: &str, what: &str, v: &Value) -> Result<T> {
    serde_json::from_value(v.clone()).map_err(|e| bad(cmd, format!("{what}: {e}")))
}

fn reply<T: serde::Serialize>(cmd: &str, value: T) -> Result<Value> {
    serde_json::to_value(value).map_err(|e| bad(cmd, e.to_string()))
}

/// A design's Manufacture data, empty when it has none; why not, when it can't be read (the
/// data is then kept as it is). For views such as the Manufacture browser, which only read.
pub fn manufacture_data(doc: &solidcraft_doc::Document) -> std::result::Result<ManufactureData, String> {
    let Some(bytes) = storage::blob(doc, MANUFACTURE_DATA_OWNER, DATA) else { return Ok(ManufactureData::default()) };
    let text = std::str::from_utf8(bytes).map_err(|_| "the design's Manufacture data is not text; it is kept as it is".to_string())?;
    ManufactureData::from_json(text).map_err(|e| format!("the design's Manufacture data can't be read ({e}); it is kept as it is"))
}

/// The design's Manufacture data; empty when it has none.
pub(crate) fn load(s: &Session, cmd: &str) -> Result<ManufactureData> {
    manufacture_data(&s.doc).map_err(|e| bad(cmd, e))
}

/// Checks and writes the Manufacture data as one edit.
pub(super) fn store(s: &mut Session, cmd: &str, data: &ManufactureData) -> Result<()> {
    let json = data.to_json().map_err(|e| bad(cmd, e.to_string()))?;
    storage::put(s, &owner(), DATA, Some(json.as_bytes()))
}

/// The next free `<prefix>-<n>` id among `ids`.
fn next_id<'a>(prefix: &str, ids: impl Iterator<Item = &'a str>) -> String {
    let n = ids.filter_map(|id| id.strip_prefix(prefix).and_then(|r| r.strip_prefix('-')).and_then(|n| n.parse::<u64>().ok())).max().unwrap_or(0);
    format!("{prefix}-{}", n.saturating_add(1))
}

fn data(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.data";
    let _: CamDataParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let operations = d.setups.iter().map(|x| x.operations.len()).sum();
    reply(
        CMD,
        CamData { data: serde_json::to_value(&d).map_err(|e| bad(CMD, e.to_string()))?, setups: d.setups.len(), tools: d.tools.len(), operations },
    )
}

/// The design's tools as a list shows them, by number: those in which every word of `search` is
/// found, in any case, in the tool's number written `T3`, its name, its kind in words and its
/// diameter; all of them for no words.
pub fn manufacture_tool_rows(d: &ManufactureData, search: &str) -> Vec<crate::schema::cam::ToolRow> {
    let words: Vec<String> = search.split_whitespace().map(str::to_lowercase).collect();
    let mut rows: Vec<crate::schema::cam::ToolRow> = d
        .tools
        .iter()
        .filter(|tool| {
            let text = format!("t{} {} {} {}", tool.number, tool.name, kind_words(&tool.kind), tool.geometry.diameter).to_lowercase();
            words.iter().all(|word| text.contains(word.as_str()))
        })
        .map(|tool| crate::schema::cam::ToolRow {
            number: tool.number,
            name: tool.name.clone(),
            kind: kind_words(&tool.kind).replace(' ', "_"),
            diameter: tool.geometry.diameter,
            flutes: tool.flutes,
            flute_length: tool.geometry.flute_length,
            overall_length: tool.geometry.overall_length,
            stick_out: tool.holder.stick_out,
            operations: d.setups.iter().flat_map(|setup| &setup.operations).filter(|op| op.tool == tool.number).count(),
        })
        .collect();
    rows.sort_by_key(|row| row.number);
    rows
}

fn tool_list(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.tool.list";
    let p: crate::schema::cam::CamToolListParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let tools = manufacture_tool_rows(&d, p.search.as_deref().unwrap_or(""));
    reply(CMD, crate::schema::cam::CamToolList { tools, of: d.tools.len() })
}

fn tool_create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.tool.create";
    let p: CamToolCreateParams = parse(CMD, "parameters", p)?;
    let tool: Tool = parse(CMD, "tool", &p.tool)?;
    let mut d = load(s, CMD)?;
    if d.tools.iter().any(|t| t.number == tool.number) {
        return Err(bad(CMD, format!("the design already has a tool {}", tool.number)));
    }
    let number = tool.number;
    d.tools.push(tool);
    store(s, CMD, &d)?;
    reply(CMD, CamToolWritten { number, tools: d.tools.len() })
}

fn tool_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.tool.edit";
    let p: CamToolEditParams = parse(CMD, "parameters", p)?;
    let tool: Tool = parse(CMD, "tool", &p.tool)?;
    let mut d = load(s, CMD)?;
    let slot = d.tools.iter_mut().find(|t| t.number == p.number).ok_or_else(|| bad(CMD, format!("the design has no tool {}", p.number)))?;
    let number = tool.number;
    *slot = tool;
    store(s, CMD, &d)?;
    reply(CMD, CamToolWritten { number, tools: d.tools.len() })
}

fn tool_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.tool.delete";
    let p: CamToolDeleteParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    if let Some(op) = d.setups.iter().flat_map(|x| &x.operations).find(|o| o.tool == p.number) {
        return Err(bad(CMD, format!("operation {} ({}) uses tool {}", op.id, op.name, p.number)));
    }
    let before = d.tools.len();
    d.tools.retain(|t| t.number != p.number);
    if d.tools.len() == before {
        return Err(bad(CMD, format!("the design has no tool {}", p.number)));
    }
    store(s, CMD, &d)?;
    reply(CMD, CamToolWritten { number: p.number, tools: d.tools.len() })
}

/// The usual vise setup's defaults (see `CamSetupCreateParams`).
fn default_setup(id: String, name: String) -> Setup {
    Setup {
        id,
        name,
        kind: SetupKind::Milling,
        machine: None,
        material: None,
        wcs: Wcs {
            origin: WcsOrigin::StockBox { anchor: BoxAnchor { x: BoxSide::Center, y: BoxSide::Center, z: BoxSide::Max } },
            orientation: WcsOrientation::Model,
        },
        work_offset: 1,
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 1.0 } },
        model_bodies: Vec::new(),
        fixture_bodies: Vec::new(),
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: Vec::new(),
    }
}

/// The fields a setup command may set, as given.
struct SetupFields {
    name: Option<String>,
    machine: Option<String>,
    material: Option<String>,
    wcs: Option<Value>,
    work_offset: Option<u8>,
    stock: Option<Value>,
    model_bodies: Option<Vec<String>>,
    fixture_bodies: Option<Vec<String>>,
    safe_heights: Option<Value>,
}

fn apply(cmd: &str, setup: &mut Setup, f: SetupFields) -> Result<()> {
    if let Some(v) = f.name {
        setup.name = v;
    }
    if let Some(v) = f.machine {
        setup.machine = Some(v);
    }
    if let Some(v) = f.material {
        setup.material = (!v.is_empty()).then_some(v);
    }
    if let Some(v) = f.wcs {
        setup.wcs = parse(cmd, "wcs", &v)?;
    }
    if let Some(v) = f.work_offset {
        setup.work_offset = v;
    }
    if let Some(v) = f.stock {
        setup.stock = parse(cmd, "stock", &v)?;
    }
    if let Some(v) = f.model_bodies {
        setup.model_bodies = v;
    }
    if let Some(v) = f.fixture_bodies {
        setup.fixture_bodies = v;
    }
    if let Some(v) = f.safe_heights {
        setup.safe_heights = parse(cmd, "safe_heights", &v)?;
    }
    Ok(())
}

/// The setup `cam.setup.create`'s parameters describe: the vise setup's defaults with the fields
/// given, under the next free id of `d`.
fn proposed_setup(cmd: &str, d: &ManufactureData, p: &Value) -> Result<Setup> {
    let p: CamSetupCreateParams = parse(cmd, "parameters", p)?;
    let id = next_id("setup", d.setups.iter().map(|x| x.id.as_str()));
    let mut setup = default_setup(id, format!("Setup {}", d.setups.len() + 1));
    let fields = SetupFields {
        name: p.name,
        machine: p.machine,
        material: p.material,
        wcs: p.wcs,
        work_offset: p.work_offset,
        stock: p.stock,
        model_bodies: p.model_bodies,
        fixture_bodies: p.fixture_bodies,
        safe_heights: p.safe_heights,
    };
    apply(cmd, &mut setup, fields)?;
    Ok(setup)
}

fn setup_create(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.create";
    let mut d = load(s, CMD)?;
    let setup = proposed_setup(CMD, &d, p)?;
    let id = setup.id.clone();
    d.setups.push(setup);
    store(s, CMD, &d)?;
    reply(CMD, CamSetupWritten { id, setups: d.setups.len() })
}

/// The design's setups as `cam.setup.edit`'s parameters leave them: the setup they name with the
/// fields given replaced, in its place. Returns its id too.
fn edited_setups(cmd: &str, d: &mut ManufactureData, p: &Value) -> Result<String> {
    let p: CamSetupEditParams = parse(cmd, "parameters", p)?;
    let setup = d.setups.iter_mut().find(|x| x.id == p.id).ok_or_else(|| bad(cmd, format!("the design has no setup '{}'", p.id)))?;
    let fields = SetupFields {
        name: p.name,
        machine: p.machine,
        material: p.material,
        wcs: p.wcs,
        work_offset: p.work_offset,
        stock: p.stock,
        model_bodies: p.model_bodies,
        fixture_bodies: p.fixture_bodies,
        safe_heights: p.safe_heights,
    };
    apply(cmd, setup, fields)?;
    Ok(p.id)
}

fn setup_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.edit";
    let mut d = load(s, CMD)?;
    let id = edited_setups(CMD, &mut d, p)?;
    store(s, CMD, &d)?;
    reply(CMD, CamSetupWritten { id, setups: d.setups.len() })
}

fn setup_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.delete";
    let p: CamSetupDeleteParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    if let Some(later) = d.setups.iter().find(|x| matches!(&x.stock, Stock::FromSetup { setup } if *setup == p.id)) {
        return Err(bad(CMD, format!("setup {} ({}) takes its stock from '{}'", later.id, later.name, p.id)));
    }
    let before = d.setups.len();
    d.setups.retain(|x| x.id != p.id);
    if d.setups.len() == before {
        return Err(bad(CMD, format!("the design has no setup '{}'", p.id)));
    }
    store(s, CMD, &d)?;
    reply(CMD, CamSetupWritten { id: p.id, setups: d.setups.len() })
}

/// Reads the bodies a setup machines, in model coordinates: its model bodies by name, or every
/// visible body when it lists none.
fn with_bodies<T>(s: &Session, cmd: &str, setup: &Setup, read: impl FnOnce(&[&solidcraft_doc::ModelBody]) -> Result<T>) -> Result<T> {
    let st = s.world_state()?;
    let bodies: Vec<&solidcraft_doc::ModelBody> = if setup.model_bodies.is_empty() {
        st.bodies.iter().filter(|b| s.body_visible(&b.name)).collect()
    } else {
        let mut named = Vec::with_capacity(setup.model_bodies.len());
        for name in &setup.model_bodies {
            let body = st
                .bodies
                .iter()
                .find(|b| &b.name == name)
                .ok_or_else(|| bad(cmd, format!("the setup's model body '{name}' is not in the design")))?;
            named.push(body);
        }
        named
    };
    read(&bodies)
}

/// The exact box, in model coordinates, of the bodies a setup machines. Returns the box and the
/// bodies' names.
pub(super) fn model_box(s: &Session, cmd: &str, setup: &Setup) -> Result<(Bounds, Vec<String>)> {
    with_bodies(s, cmd, setup, |bodies| {
        let mut boxes = bodies.iter().map(|b| b.body.exact_bounds().map_err(|e| bad(cmd, format!("measuring body '{}': {e}", b.name))));
        let first = boxes
            .next()
            .ok_or_else(|| bad(cmd, format!("setup '{}' has no bodies to machine: show one, or list them in its model_bodies", setup.id)))??;
        let mut b = first;
        for next in boxes {
            b = b.union(&next?);
        }
        Ok((Bounds { min: b.min, max: b.max }, bodies.iter().map(|b| b.name.clone()).collect()))
    })
}

/// Sizes and heights this close are the same when holes are grouped, mm.
const SAME_HOLE: f64 = 1e-6;

/// The holes a setup can drill (decision 0012 of the CAM work): the whole cylinder walls of its
/// bodies along its Z that are open above, in its WCS, grouped by diameter, top and depth.
fn holes(s: &Session, cmd: &str, setup: &Setup) -> Result<Vec<HoleGroup>> {
    let (model, _) = model_box(s, cmd, setup)?;
    let p = setup.place(model).map_err(|e| bad(cmd, e.to_string()))?;
    let found = with_bodies(s, cmd, setup, |bodies| {
        let mut all = Vec::new();
        for body in bodies {
            let of_body = super::cam_holes::holes_along(&body.body, p.z_axis)
                .map_err(|e| bad(cmd, format!("finding the holes of body '{}': {e}", body.name)))?;
            all.extend(of_body);
        }
        Ok(all)
    })?;
    // A WCS coordinate without the last bits of rounding, so equal holes read equal.
    let along = |v: solidcore_record::Vec3, a: solidcore_record::Vec3| ((v.x * a.x + v.y * a.y + v.z * a.z) * 1e9).round() / 1e9;
    let mut groups: Vec<HoleGroup> = Vec::new();
    for h in found {
        let from = solidcore_record::Vec3 { x: h.top.x - p.origin.x, y: h.top.y - p.origin.y, z: h.top.z - p.origin.z };
        let (x, y, top) = (along(from, p.x_axis), along(from, p.y_axis), along(from, p.z_axis));
        let (diameter, depth) = ((2.0 * h.radius * 1e9).round() / 1e9, (h.depth * 1e9).round() / 1e9);
        let bottom = match h.floor {
            None => HoleBottom::Through,
            Some(below) if below <= SAME_HOLE => HoleBottom::Flat,
            Some(_) => HoleBottom::Point,
        };
        let same = |g: &&mut HoleGroup| {
            (g.diameter - diameter).abs() <= SAME_HOLE
                && (g.top - top).abs() <= SAME_HOLE
                && (g.depth - depth).abs() <= SAME_HOLE
                && g.bottom == bottom
        };
        match groups.iter_mut().find(same) {
            Some(g) => g.holes.push([x, y]),
            None => groups.push(HoleGroup { diameter, top, depth, bottom, holes: vec![[x, y]] }),
        }
    }
    for g in &mut groups {
        g.holes.sort_by(|a, b| a[1].total_cmp(&b[1]).then(a[0].total_cmp(&b[0])));
    }
    groups.sort_by(|a, b| b.holes.len().cmp(&a.holes.len()).then(a.diameter.total_cmp(&b.diameter)).then(b.top.total_cmp(&a.top)));
    Ok(groups)
}

fn setup_holes(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.holes";
    let p: CamSetupHolesParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let groups = holes(s, CMD, setup)?;
    reply(CMD, CamSetupHoles { id: p.id, holes: groups.iter().map(|g| g.holes.len()).sum(), groups })
}

/// The holes the setup `id` can drill, for a form to offer; why not, when the setup can't be
/// placed or a body can't be read. It reads the design and changes nothing.
pub fn manufacture_holes(s: &Session, id: &str) -> std::result::Result<Vec<HoleGroup>, String> {
    const CMD: &str = "cam.setup.holes";
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
    holes(s, CMD, setup).map_err(said)
}

/// The outlines read lately ([`outline_at`]), each under a number made of its bodies' geometry and
/// its plane. Only outlines that were read are kept, never a refusal.
static OUTLINES: std::sync::Mutex<Vec<(u64, Vec<solidcore_record::Loop2>)>> = std::sync::Mutex::new(Vec::new());
const OUTLINES_KEPT: usize = 16;

/// The outline a contour goes round (decision 0013 of the CAM work): the outer loop of every piece
/// of the section of the setup's bodies at the height `z` of its WCS (the model's bottom when
/// `None` or below it: a cut past the part goes round the part's lowest outline), in the WCS, and
/// the height it was read at.
fn outline_at(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    z: Option<f64>,
) -> std::result::Result<(f64, Vec<solidcore_record::Loop2>), String> {
    let floor = p.model.min.z;
    let z = z.map_or(floor, |z| z.max(floor));
    if !(z.abs() <= solidcraft_cam::MAX_COORDINATE) {
        return Err(format!("the height to read the outline at must be finite and within {} mm", solidcraft_cam::MAX_COORDINATE));
    }
    // The WCS at that height: a section read in this plane is in the WCS's own X and Y.
    let plane = solidcore_record::Plane { origin: p.origin + p.z_axis * z, x: p.x_axis, y: p.y_axis };
    let loops = with_bodies(s, cmd, setup, |bodies| {
        // Reading an outline cuts the bodies, and a form asks again at every change of a number
        // that leaves the outline as it was: one read lately from the same geometry is kept.
        let key = bodies.iter().map(|b| b.geometry_key()).collect::<Option<Vec<u64>>>().map(|geometry| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            geometry.hash(&mut h);
            bodies.iter().for_each(|b| b.name.hash(&mut h));
            [plane.origin, plane.x, plane.y].iter().flat_map(|v| [v.x, v.y, v.z]).for_each(|v| v.to_bits().hash(&mut h));
            h.finish()
        });
        let known = key.and_then(|k| OUTLINES.lock().ok().and_then(|kept| kept.iter().find(|(at, _)| *at == k).map(|(_, loops)| loops.clone())));
        if let Some(loops) = known {
            return Ok(loops);
        }
        let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
        let loops = super::cam_outline::outline(&named, &plane).map_err(|e| bad(cmd, e))?;
        if let Some(key) = key
            && let Ok(mut kept) = OUTLINES.lock()
        {
            kept.retain(|(at, _)| *at != key);
            kept.insert(0, (key, loops.clone()));
            kept.truncate(OUTLINES_KEPT);
        }
        Ok(loops)
    })
    .map_err(said)?;
    if loops.is_empty() {
        return Err(format!("the part has no outline at Z {z} mm: nothing of it is cut by that height or stands flat on it"));
    }
    Ok((z, loops))
}

fn setup_outline(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.outline";
    let p: CamSetupOutlineParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let (model, _) = model_box(s, CMD, setup)?;
    let placement = setup.place(model).map_err(|e| bad(CMD, e.to_string()))?;
    let (z, found) = outline_at(s, CMD, setup, &placement, p.z).map_err(|e| bad(CMD, e))?;
    let mut loops = Vec::with_capacity(found.len());
    for l in &found {
        let around = l.segs.iter().fold(solidcraft_geom::Aabb2::EMPTY, |b, seg| b.union(&seg.bounds()));
        loops.push(OutlineLoop {
            segments: serde_json::to_value(&l.segs).map_err(|e| bad(CMD, e.to_string()))?,
            length: l.segs.iter().map(|seg| seg.length()).sum(),
            area: l.signed_area().abs(),
            min: [around.min.x, around.min.y],
            max: [around.max.x, around.max.y],
        });
    }
    reply(CMD, CamSetupOutline { id: p.id, z, loops })
}

/// The pockets read lately ([`pockets_of`]), each list under a number made of its bodies'
/// geometry, the plane and the points asked for. Only pockets that were read are kept.
static POCKETS: std::sync::Mutex<Vec<(u64, Vec<(solidcore_record::Vec2, super::cam_pockets::Pocket)>)>> = std::sync::Mutex::new(Vec::new());

/// Pockets of the setup's bodies (decision 0014 of the CAM work), in its WCS, each with the point
/// on its floor it was found at: the pockets at `points` in their order, or every closed pocket
/// when `None`.
fn pockets_of(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    points: Option<&[[f64; 2]]>,
) -> std::result::Result<Vec<(solidcore_record::Vec2, super::cam_pockets::Pocket)>, String> {
    // The WCS itself: floors are heights above its XY plane, regions are in its X and Y.
    let plane = solidcore_record::Plane { origin: p.origin, x: p.x_axis, y: p.y_axis };
    with_bodies(s, cmd, setup, |bodies| {
        let key = bodies.iter().map(|b| b.geometry_key()).collect::<Option<Vec<u64>>>().map(|geometry| {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            geometry.hash(&mut h);
            bodies.iter().for_each(|b| b.name.hash(&mut h));
            [plane.origin, plane.x, plane.y].iter().flat_map(|v| [v.x, v.y, v.z]).for_each(|v| v.to_bits().hash(&mut h));
            p.model.max.z.to_bits().hash(&mut h);
            points.map(|points| points.iter().flatten().map(|v| v.to_bits()).collect::<Vec<u64>>()).hash(&mut h);
            h.finish()
        });
        let known = key.and_then(|k| POCKETS.lock().ok().and_then(|kept| kept.iter().find(|(at, _)| *at == k).map(|(_, found)| found.clone())));
        if let Some(found) = known {
            return Ok(found);
        }
        let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
        let mut finder = super::cam_pockets::Finder::new(&named, &plane, p.model.max.z);
        let found = match points {
            None => finder.pockets().map_err(|e| bad(cmd, e))?,
            Some(points) => {
                let mut found = Vec::with_capacity(points.len());
                for at in points {
                    let at = solidcore_record::Vec2 { x: at[0], y: at[1] };
                    found.push((at, finder.pocket_at(at).map_err(|e| bad(cmd, e))?));
                }
                found
            }
        };
        if let Some(key) = key
            && let Ok(mut kept) = POCKETS.lock()
        {
            kept.retain(|(at, _)| *at != key);
            kept.insert(0, (key, found.clone()));
            kept.truncate(OUTLINES_KEPT);
        }
        Ok(found)
    })
    .map_err(said)
}

/// The openings in the part's top whose rims a chamfer of all the edges breaks (decision 0018 of
/// the CAM work), in the setup's WCS: the air in each, read at the height `at` and open down to
/// `tip`, where the tool's point goes; and how many were left out because they are not.
fn rims_of(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    at: f64,
    tip: f64,
) -> std::result::Result<(Vec<solidcore_record::Region2>, usize), String> {
    let plane = solidcore_record::Plane { origin: p.origin, x: p.x_axis, y: p.y_axis };
    with_bodies(s, cmd, setup, |bodies| {
        read_or_kept(&RIMS, reading_key(bodies, &plane, &[p.model.max.z, at, tip]), || {
            let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
            super::cam_pockets::Finder::new(&named, &plane, p.model.max.z).rims(at, tip).map_err(|e| bad(cmd, e))
        })
    })
    .map_err(said)
}

/// The rims and the openings read lately ([`rims_of`], [`openings_at`]), each under a number made
/// of what the reading depends on ([`reading_key`]).
static RIMS: std::sync::Mutex<Vec<(u64, (Vec<solidcore_record::Region2>, usize))>> = std::sync::Mutex::new(Vec::new());
static OPENINGS: std::sync::Mutex<Vec<(u64, Vec<super::cam_pockets::Pocket>)>> = std::sync::Mutex::new(Vec::new());

/// A number for what a reading of `bodies` in `plane` depends on: their geometry and names, the
/// plane, and what else was asked of it (`asked`: heights, points). `None` when a body's
/// geometry has no number, and nothing can be kept.
fn reading_key(bodies: &[&solidcraft_doc::ModelBody], plane: &solidcore_record::Plane, asked: &[f64]) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let geometry = bodies.iter().map(|b| b.geometry_key()).collect::<Option<Vec<u64>>>()?;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    geometry.hash(&mut h);
    bodies.iter().for_each(|b| b.name.hash(&mut h));
    [plane.origin, plane.x, plane.y].iter().flat_map(|v| [v.x, v.y, v.z]).chain(asked.iter().copied()).for_each(|v| v.to_bits().hash(&mut h));
    asked.len().hash(&mut h);
    Some(h.finish())
}

/// What `read` gives, or what it gave lately under the same `key`. Reading openings cuts the
/// bodies and checks exactly that nothing hangs over them, and a form asks again at every change
/// of a number that leaves them as they were. Only what was read is kept, never a refusal.
fn read_or_kept<T: Clone>(kept: &std::sync::Mutex<Vec<(u64, T)>>, key: Option<u64>, read: impl FnOnce() -> Result<T>) -> Result<T> {
    let known = key.and_then(|k| kept.lock().ok().and_then(|kept| kept.iter().find(|(at, _)| *at == k).map(|(_, found)| found.clone())));
    if let Some(found) = known {
        return Ok(found);
    }
    let found = read()?;
    if let Some(key) = key
        && let Ok(mut kept) = kept.lock()
    {
        kept.retain(|(at, _)| *at != key);
        kept.insert(0, (key, found.clone()));
        kept.truncate(OUTLINES_KEPT);
    }
    Ok(found)
}

/// The opening each of `points` is in ([`super::cam_pockets::Finder::opening_at`]), in their
/// order, in the setup's WCS: read from the model, or as read lately from the same geometry.
fn openings_at(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    points: &[[f64; 2]],
    bottom: Option<f64>,
) -> std::result::Result<Vec<super::cam_pockets::Pocket>, String> {
    let plane = solidcore_record::Plane { origin: p.origin, x: p.x_axis, y: p.y_axis };
    // What the reading was asked: the part's top and bottom, the bottom given (or that none
    // was), and the points.
    let asked: Vec<f64> = [p.model.max.z, p.model.min.z, f64::from(u8::from(bottom.is_some())), bottom.unwrap_or(0.0)]
        .into_iter()
        .chain(points.iter().flatten().copied())
        .collect();
    with_bodies(s, cmd, setup, |bodies| {
        read_or_kept(&OPENINGS, reading_key(bodies, &plane, &asked), || {
            let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
            let mut finder = super::cam_pockets::Finder::new(&named, &plane, p.model.max.z);
            let mut found = Vec::with_capacity(points.len());
            for at in points {
                let at = solidcore_record::Vec2 { x: at[0], y: at[1] };
                found.push(finder.opening_at(at, bottom, p.model.min.z).map_err(|e| bad(cmd, e))?);
            }
            Ok(found)
        })
    })
    .map_err(said)
}

/// The openings an inside contour goes round (decision 0021 of the CAM work), in the setup's WCS,
/// one for each of `points` in their order, and the height their walls were read at: `bottom`
/// when given (the part's own bottom when it is under that), else the floor under the points,
/// or the part's bottom where they go through it. The openings of one operation are cut to one
/// depth.
fn openings_of(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    points: &[[f64; 2]],
    bottom: Option<f64>,
) -> std::result::Result<(f64, Vec<solidcore_record::Region2>), String> {
    let found = openings_at(s, cmd, setup, p, points, bottom)?;
    let Some(floor) = found.first().map(|opening| opening.floor) else { return Err("an inside contour needs an opening to go round".into()) };
    if let Some(other) = found.iter().find(|opening| (opening.floor - floor).abs() > 1e-6) {
        return Err(format!(
            "the openings are cut to different depths (Z {floor} and Z {} mm): make an operation for each depth, or give the bottom",
            other.floor
        ));
    }
    Ok((floor, found.into_iter().map(|opening| opening.region).collect()))
}

/// About how wide a tool can be and still pass everywhere in `region`, whose widest circle is
/// `widest` across: twice how far its walls can move in before it comes apart or an island's
/// room meets a wall's or another island's. Looked for from the walls inwards (a part that
/// comes apart and then vanishes would look whole again), in sixteen steps and then six halvings.
fn passes_everywhere(region: &solidcore_record::Region2, widest: f64) -> f64 {
    let whole = |moved: f64| {
        solidcraft_cam::offset_region(region, moved)
            .is_ok_and(|left| left.len() == 1 && left.first().is_some_and(|one| one.holes.len() == region.holes.len()))
    };
    let (mut passes, mut fails) = (0.0, None);
    for step in 1..=16 {
        // Just short of the widest circle itself, where nothing is left to be whole.
        let moved = 0.5 * widest * (f64::from(step) / 16.0).min(0.999);
        if whole(moved) {
            passes = moved;
        } else {
            fails = Some(moved);
            break;
        }
    }
    let Some(mut fails) = fails else { return widest };
    for _ in 0..6 {
        let between = 0.5 * (passes + fails);
        if whole(between) {
            passes = between;
        } else {
            fails = between;
        }
    }
    (2.0 * passes * 100.0).floor() / 100.0
}

/// The closed pockets the setup `setup` can cut, as a form or a script reads them.
fn found_pockets(s: &Session, cmd: &str, setup: &Setup) -> std::result::Result<Vec<FoundPocket>, String> {
    let (model, _) = model_box(s, cmd, setup).map_err(said)?;
    let p = setup.place(model).map_err(|e| e.to_string())?;
    let round = |v: f64| (v * 1e9).round() / 1e9;
    let mut out = Vec::new();
    for (at, pocket) in pockets_of(s, cmd, setup, &p, None)? {
        let around = pocket.region.outer.segs.iter().fold(solidcraft_geom::Aabb2::EMPTY, |b, seg| b.union(&seg.bounds()));
        let widest = (2.0 * pocket.region.inradius(1e-4) * 100.0).round() / 100.0;
        out.push(FoundPocket {
            at: [round(at.x), round(at.y)],
            floor: round(pocket.floor),
            depth: round(p.model.max.z - pocket.floor),
            widest,
            passes: passes_everywhere(&pocket.region, widest),
            size: [round(around.max.x - around.min.x), round(around.max.y - around.min.y)],
            islands: pocket.region.holes.len(),
            area: pocket.region.area(),
        });
    }
    Ok(out)
}

fn setup_pockets(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.pockets";
    let p: CamSetupPocketsParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let pockets = found_pockets(s, CMD, setup).map_err(|e| bad(CMD, e))?;
    reply(CMD, CamSetupPockets { id: p.id, pockets })
}

/// The closed pockets the setup `id` can cut, for a form to offer; why not, when the setup can't
/// be placed or a body can't be read. It reads the design and changes nothing.
pub fn manufacture_pockets(s: &Session, id: &str) -> std::result::Result<Vec<FoundPocket>, String> {
    const CMD: &str = "cam.setup.pockets";
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
    found_pockets(s, CMD, setup)
}

/// The diameter of a wall that is a circle: arcs about one centre, of one radius.
fn round_wall(wall: &solidcore_record::Loop2) -> Option<f64> {
    let mut circle: Option<(solidcore_record::Vec2, f64)> = None;
    for seg in &wall.segs {
        let solidcore_record::Seg2::Arc { center, radius, .. } = *seg else { return None };
        match circle {
            None => circle = Some((center, radius)),
            Some((about, of)) if (about.x - center.x).hypot(about.y - center.y) <= 1e-6 && (of - radius).abs() <= 1e-6 => {}
            Some(_) => return None,
        }
    }
    circle.map(|(_, radius)| 2.0 * radius)
}

/// The openings the setup `setup` has for an inside contour, as a form or a script reads them:
/// the ones through the part, then its closed pockets, each kind the largest first.
fn found_openings(s: &Session, cmd: &str, setup: &Setup) -> std::result::Result<Vec<FoundOpening>, String> {
    let (model, _) = model_box(s, cmd, setup).map_err(said)?;
    let p = setup.place(model).map_err(|e| e.to_string())?;
    let plane = solidcore_record::Plane { origin: p.origin, x: p.x_axis, y: p.y_axis };
    let windows = with_bodies(s, cmd, setup, |bodies| {
        let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
        super::cam_pockets::Finder::new(&named, &plane, p.model.max.z).windows(p.model.min.z).map_err(|e| bad(cmd, e))
    })
    .map_err(said)?;
    let pockets = pockets_of(s, cmd, setup, &p, None)?;
    let round = |v: f64| (v * 1e9).round() / 1e9;
    let found = windows.into_iter().map(|found| (true, found)).chain(pockets.into_iter().map(|found| (false, found)));
    Ok(found
        .map(|(through, (at, opening))| {
            let around = opening.region.outer.segs.iter().fold(solidcraft_geom::Aabb2::EMPTY, |b, seg| b.union(&seg.bounds()));
            FoundOpening {
                at: [round(at.x), round(at.y)],
                bottom: round(opening.floor),
                through,
                depth: round(p.model.max.z - opening.floor),
                widest: (2.0 * opening.region.inradius(1e-4) * 100.0).round() / 100.0,
                size: [round(around.max.x - around.min.x), round(around.max.y - around.min.y)],
                diameter: round_wall(&opening.region.outer).map(round),
                islands: opening.region.holes.len(),
                area: opening.region.area(),
            }
        })
        .collect())
}

fn setup_openings(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.openings";
    let p: CamSetupOpeningsParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let openings = found_openings(s, CMD, setup).map_err(|e| bad(CMD, e))?;
    reply(CMD, CamSetupOpenings { id: p.id, openings })
}

/// The openings the setup `id` has for an inside contour, for a form to offer; why not, when
/// the setup can't be placed or a body can't be read. It reads the design and changes nothing.
pub fn manufacture_openings(s: &Session, id: &str) -> std::result::Result<Vec<FoundOpening>, String> {
    const CMD: &str = "cam.setup.openings";
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
    found_openings(s, CMD, setup)
}

/// The slots a slot operation cuts (decision 0026 of the CAM work), in the setup's WCS, one for
/// each of `points` in their order: the opening read from the model where the point is, and how
/// far down its wall was read: `bottom` when given (the part's own bottom when it is under
/// that), else the floor under the point, or the part's bottom where it goes through. Whether
/// each is a slot is the operation's to say.
fn slots_of(
    s: &Session,
    cmd: &str,
    setup: &Setup,
    p: &Placement,
    points: &[[f64; 2]],
    bottom: Option<f64>,
) -> std::result::Result<Vec<solidcraft_cam::SlotShape>, String> {
    let found = openings_at(s, cmd, setup, p, points, bottom)?;
    Ok(found.into_iter().map(|opening| solidcraft_cam::SlotShape { bottom: opening.floor, region: opening.region }).collect())
}

/// The slots the setup `setup` has, as a form or a script reads them: its openings through the
/// part and its closed pockets that are slots (the operation's own test, so what is offered is
/// what it cuts), the ones through the part first, each kind the largest first.
fn found_slots(s: &Session, cmd: &str, setup: &Setup) -> std::result::Result<Vec<crate::schema::cam::FoundSlot>, String> {
    let (model, _) = model_box(s, cmd, setup).map_err(said)?;
    let p = setup.place(model).map_err(|e| e.to_string())?;
    let plane = solidcore_record::Plane { origin: p.origin, x: p.x_axis, y: p.y_axis };
    let windows = with_bodies(s, cmd, setup, |bodies| {
        let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
        super::cam_pockets::Finder::new(&named, &plane, p.model.max.z).windows(p.model.min.z).map_err(|e| bad(cmd, e))
    })
    .map_err(said)?;
    let pockets = pockets_of(s, cmd, setup, &p, None)?;
    // To a millionth of a micron, and a zero without a minus sign.
    let round = |v: f64| (v * 1e9).round() / 1e9 + 0.0;
    let found = windows.into_iter().map(|found| (true, found)).chain(pockets.into_iter().map(|found| (false, found)));
    Ok(found
        .filter_map(|(through, (at, opening))| {
            let slot = solidcraft_cam::slot_line(&opening.region).ok()?;
            Some(crate::schema::cam::FoundSlot {
                at: [round(at.x), round(at.y)],
                bottom: round(opening.floor),
                through,
                depth: round(p.model.max.z - opening.floor),
                width: round(slot.width),
                length: round(solidcraft_geom::Seg2Geom::length(&slot.line)),
                curved: matches!(slot.line, solidcore_record::Seg2::Arc { .. }),
            })
        })
        .collect())
}

fn setup_slots(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.slots";
    let p: crate::schema::cam::CamSetupSlotsParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let slots = found_slots(s, CMD, setup).map_err(|e| bad(CMD, e))?;
    reply(CMD, crate::schema::cam::CamSetupSlots { id: p.id, slots })
}

/// The slots the setup `id` has, for a form to offer; why not, when the setup can't be placed or
/// a body can't be read. It reads the design and changes nothing.
pub fn manufacture_slots(s: &Session, id: &str) -> std::result::Result<Vec<crate::schema::cam::FoundSlot>, String> {
    const CMD: &str = "cam.setup.slots";
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
    found_slots(s, CMD, setup)
}

fn setup_measure(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.measure";
    let p: CamSetupMeasureParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
    let (model, bodies) = model_box(s, CMD, setup)?;
    let placement = setup.place(model).map_err(|e| bad(CMD, e.to_string()))?;
    reply(
        CMD,
        CamSetupMeasured {
            id: p.id,
            bodies,
            model_box: serde_json::to_value(model).map_err(|e| bad(CMD, e.to_string()))?,
            placement: serde_json::to_value(placement).map_err(|e| bad(CMD, e.to_string()))?,
        },
    )
}

fn op_manual(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.manual";
    let p: CamOpManualParams = parse(CMD, "parameters", p)?;
    let toolpath: Toolpath = parse(CMD, "toolpath", &p.toolpath)?;
    let mut d = load(s, CMD)?;
    let id = next_id("op", d.setups.iter().flat_map(|x| &x.operations).map(|o| o.id.as_str()));
    let setup = d.setups.iter_mut().find(|x| x.id == p.setup).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.setup)))?;
    let name = p.name.unwrap_or_else(|| format!("Hand-made toolpath {}", setup.operations.len() + 1));
    setup.operations.push(Operation { id: id.clone(), name, tool: p.tool, suppressed: false, operation: OperationKind::Manual { toolpath } });
    let operations = setup.operations.len();
    store(s, CMD, &d)?;
    reply(CMD, CamOpWritten { id, setup: p.setup, operations })
}

fn op_drill(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.drill", "Drill", p)
}

fn op_face(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.face", "Face", p)
}

fn op_contour(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.contour", "Contour", p)
}

fn op_bore(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.bore", "Bore", p)
}

fn op_pocket(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.pocket", "Pocket", p)
}

fn op_slot(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.slot", "Slot", p)
}

fn op_chamfer(s: &mut Session, p: &Value) -> Result<Value> {
    add_operation(s, "cam.op.chamfer", "Chamfer", p)
}

/// What the parameters of `cmd` (`cam.op.drill`, `cam.op.face`, `cam.op.contour`, `cam.op.pocket`
/// or `cam.op.chamfer`) describe: the setup's id, the name given, the tool's number and the operation.
fn proposed(cmd: &str, p: &Value) -> Result<(String, Option<String>, u32, OperationKind)> {
    match cmd {
        "cam.op.drill" => {
            let p: CamOpDrillParams = parse(cmd, "parameters", p)?;
            let drill: DrillOp = parse(cmd, "drill", &p.drill)?;
            Ok((p.setup, p.name, p.tool, OperationKind::Drill { drill }))
        }
        "cam.op.face" => {
            let p: CamOpFaceParams = parse(cmd, "parameters", p)?;
            let face: FaceOp = match &p.face {
                Some(v) => parse(cmd, "face", v)?,
                None => FaceOp::default(),
            };
            Ok((p.setup, p.name, p.tool, OperationKind::Face { face }))
        }
        "cam.op.contour" => {
            let p: CamOpContourParams = parse(cmd, "parameters", p)?;
            let contour: ContourOp = match &p.contour {
                Some(v) => parse(cmd, "contour", v)?,
                None => ContourOp::default(),
            };
            Ok((p.setup, p.name, p.tool, OperationKind::Contour { contour }))
        }
        "cam.op.bore" => {
            // An inside contour by another name: whatever side it is given, it is cut inside.
            let p: CamOpContourParams = parse(cmd, "parameters", p)?;
            let mut contour: ContourOp = match &p.contour {
                Some(v) => parse(cmd, "contour", v)?,
                None => ContourOp::default(),
            };
            contour.side = ContourSide::Inside;
            Ok((p.setup, p.name, p.tool, OperationKind::Contour { contour }))
        }
        "cam.op.pocket" => {
            let p: CamOpPocketParams = parse(cmd, "parameters", p)?;
            let pocket: PocketOp = parse(cmd, "pocket", &p.pocket)?;
            Ok((p.setup, p.name, p.tool, OperationKind::Pocket { pocket }))
        }
        "cam.op.slot" => {
            let p: crate::schema::cam::CamOpSlotParams = parse(cmd, "parameters", p)?;
            let slot: solidcraft_cam::SlotOp = parse(cmd, "slot", &p.slot)?;
            Ok((p.setup, p.name, p.tool, OperationKind::Slot { slot }))
        }
        "cam.op.chamfer" => {
            let p: CamOpChamferParams = parse(cmd, "parameters", p)?;
            let chamfer: ChamferOp = match &p.chamfer {
                Some(v) => parse(cmd, "chamfer", v)?,
                None => ChamferOp::default(),
            };
            Ok((p.setup, p.name, p.tool, OperationKind::Chamfer { chamfer }))
        }
        other => Err(bad(other, "not a command that adds a drill, face, contour, bore, pocket, slot or chamfer operation")),
    }
}

/// Adds the operation `cmd`'s parameters describe to its setup, named `word` and its place in
/// the setup when no name is given.
fn add_operation(s: &mut Session, cmd: &str, word: &str, p: &Value) -> Result<Value> {
    let (setup_id, name, tool, operation) = proposed(cmd, p)?;
    let mut d = load(s, cmd)?;
    let id = next_id("op", d.setups.iter().flat_map(|x| &x.operations).map(|o| o.id.as_str()));
    let setup = d.setups.iter_mut().find(|x| x.id == setup_id).ok_or_else(|| bad(cmd, format!("the design has no setup '{setup_id}'")))?;
    let name = name.unwrap_or_else(|| format!("{word} {}", setup.operations.len() + 1));
    setup.operations.push(Operation { id: id.clone(), name, tool, suppressed: false, operation });
    let operations = setup.operations.len();
    store(s, cmd, &d)?;
    reply(cmd, CamOpWritten { id, setup: setup_id, operations })
}

/// A trouble with `op`, saying which operation: one not made yet (a form's preview) has no id,
/// and its trouble is told as it is.
fn of(op: &Operation, trouble: impl std::fmt::Display) -> String {
    if op.id.is_empty() { trouble.to_string() } else { format!("operation {} ({}): {trouble}", op.id, op.name) }
}

/// One operation's toolpath, the numbers its strategy worked out, and its notes.
struct Worked {
    toolpath: Toolpath,
    numbers: Value,
    notes: Vec<String>,
    /// How far the toolpath is meant to cut into the part as the model has it, mm: a chamfer's
    /// depth on an edge the model may leave sharp. The simulation does not call that a gouge.
    into_part: f64,
}

/// The tool and the setup's placement a strategy works from. The placement is measured on the
/// model the first time an operation of the setup needs it.
fn tool_and_placement<'a>(
    s: &Session,
    cmd: &str,
    d: &'a ManufactureData,
    setup: &Setup,
    op: &Operation,
    placed: &mut Option<Placement>,
) -> Result<(&'a Tool, Placement)> {
    let tool = d.tools.iter().find(|t| t.number == op.tool).ok_or_else(|| bad(cmd, of(op, format!("the design has no tool {}", op.tool))))?;
    let placement = match placed {
        Some(p) => *p,
        None => {
            let (model, _) = model_box(s, cmd, setup)?;
            let p = setup.place(model).map_err(|e| bad(cmd, e.to_string()))?;
            *placed = Some(p);
            p
        }
    };
    Ok((tool, placement))
}

/// `op`'s toolpath from `start` (`None`: where the strategy begins): a hand-made one as given, a
/// strategy's worked out with its tool on the setup's placement.
fn work(
    s: &Session,
    cmd: &str,
    d: &ManufactureData,
    setup: &Setup,
    op: &Operation,
    start: Option<solidcore_record::Vec3>,
    placed: &mut Option<Placement>,
) -> Result<Worked> {
    let failed = |e: solidcraft_cam::Error| bad(cmd, of(op, e));
    let material = setup.material.as_deref();
    match &op.operation {
        OperationKind::Manual { toolpath } => {
            Ok(Worked { toolpath: toolpath.clone(), numbers: Value::Object(Default::default()), notes: Vec::new(), into_part: 0.0 })
        }
        OperationKind::Drill { drill } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            let done = drill.toolpath(start, tool, &placement, material).map_err(failed)?;
            let numbers = serde_json::json!({"rpm": done.rpm, "feed": done.feed, "bottom": done.bottom, "r": done.r, "clearance": done.clearance});
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: 0.0 })
        }
        OperationKind::Face { face } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            let done = face.toolpath(start, tool, &placement, material).map_err(failed)?;
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "levels": done.levels, "passes": done.passes, "stepover": done.stepover
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: 0.0 })
        }
        OperationKind::Contour { contour } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            // Its numbers and tool first: what is wrong with them is said before the model is cut.
            contour.validate(tool).map_err(failed)?;
            let done = if contour.side == ContourSide::Inside {
                // Round the inside of its openings, each read from the model where its point is,
                // down to the bottom it was given (as far as it says: a little under the part
                // breaks through) or, given none, to the one found.
                let (found, openings) = openings_of(s, cmd, setup, &placement, &contour.openings, contour.bottom).map_err(|e| bad(cmd, of(op, e)))?;
                let bottom = Some(contour.bottom.unwrap_or(found));
                ContourOp { bottom, ..contour.clone() }.toolpath_inside(start, tool, &placement, material, &openings).map_err(failed)?
            } else {
                let (_, outline) = outline_at(s, cmd, setup, &placement, contour.bottom).map_err(|e| bad(cmd, of(op, e)))?;
                contour.toolpath(start, tool, &placement, material, &outline).map_err(failed)?
            };
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "top": done.top, "levels": done.levels,
                "stepdown": done.stepdown, "offset": done.offset, "passes": done.passes, "stepover": done.stepover, "round": done.round,
                "entries": done.entries, "leads": done.leads, "tabs": done.tabs, "tab_top": done.tab_top
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: 0.0 })
        }
        OperationKind::Pocket { pocket } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            // Its numbers and tool first: what is wrong with them is said before the model is cut.
            pocket.validate(tool).map_err(failed)?;
            let found = pockets_of(s, cmd, setup, &placement, Some(&pocket.pockets)).map_err(|e| bad(cmd, of(op, e)))?;
            let shapes: Vec<PocketShape> = found.into_iter().map(|(_, found)| PocketShape { floor: found.floor, region: found.region }).collect();
            let done = pocket.toolpath(start, tool, &placement, material, &shapes).map_err(failed)?;
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "ramp_feed": done.ramp_feed, "top": done.top,
                "levels": done.levels, "rings": done.rings.iter().map(Vec::len).collect::<Vec<_>>(), "stepdown": done.stepdown,
                "stepover": done.stepover, "offset": done.offset
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: 0.0 })
        }
        OperationKind::Slot { slot } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            // Its numbers and tool first: what is wrong with them is said before the model is cut.
            slot.validate(tool).map_err(failed)?;
            let shapes = slots_of(s, cmd, setup, &placement, &slot.slots, slot.bottom).map_err(|e| bad(cmd, of(op, e)))?;
            let done = slot.toolpath(start, tool, &placement, material, &shapes).map_err(failed)?;
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "ramp_feed": done.ramp_feed, "top": done.top,
                "levels": done.levels, "stepdown": done.stepdown, "widths": done.widths, "lengths": done.lengths, "legs": done.legs,
                "left": done.left
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: 0.0 })
        }
        OperationKind::Chamfer { chamfer } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            // The wall is read a hair below the chamfer's lower edge, where the model has it
            // whether or not it has the chamfer.
            let wall = chamfer.wall_height(tool, &placement).map_err(failed)?;
            let (_, outline) = outline_at(s, cmd, setup, &placement, Some(wall - CHAMFER_WALL_BELOW)).map_err(|e| bad(cmd, of(op, e)))?;
            let done = if chamfer.edges == ChamferEdges::All {
                // The rims of the openings in the part's top too, each read at the same height
                // and open as far down as the tool's point goes.
                let tip = chamfer.tip_height(tool, &placement).map_err(failed)?;
                let (openings, left_out) = rims_of(s, cmd, setup, &placement, wall - CHAMFER_WALL_BELOW, tip).map_err(|e| bad(cmd, of(op, e)))?;
                let mut done = chamfer.toolpath_round(start, tool, &placement, material, &outline, &openings).map_err(failed)?;
                if left_out > 0 {
                    done.notes.push(format!(
                        "{left_out} opening(s) with something in the way of the tool's point, which goes {:.2} mm below the edge, left sharp",
                        done.top - tip
                    ));
                }
                done
            } else {
                chamfer.toolpath(start, tool, &placement, material, &outline).map_err(failed)?
            };
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "top": done.top, "z": done.z, "width": done.width,
                "leg": done.leg, "offset": done.offset, "round": done.round, "entries": done.entries, "leads": done.leads
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes, into_part: done.leg })
        }
    }
}

// The Manufacture types a view of a setup is made of, for the UI above the engine.
pub use solidcraft_cam::{BodyPart, Drawn, Placement, ToolShape};

/// Writes `bytes` to the file `path` in one step; a file already there is replaced only when
/// `overwrite`. `what` names the content in the error.
pub(super) fn write_file(cmd: &str, path: &str, bytes: &[u8], overwrite: bool, what: &str) -> Result<()> {
    if path.is_empty() || path.len() > 4096 || path.chars().any(char::is_control) {
        return Err(bad(cmd, "invalid file path"));
    }
    solidcraft_io::vfs::check(path).map_err(|e| bad(cmd, e.to_string()))?;
    #[cfg(not(target_arch = "wasm32"))]
    let result = if overwrite {
        solidcraft_io::write_atomic(std::path::Path::new(path), bytes, false)
    } else {
        solidcraft_io::safe_write::create_atomic(std::path::Path::new(path), bytes)
    };
    #[cfg(target_arch = "wasm32")]
    let result = if !overwrite && solidcraft_io::vfs::exists(path) {
        Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "the file already exists"))
    } else {
        solidcraft_io::vfs::write(path, bytes)
    };
    result.map_err(|e| bad(cmd, format!("could not write {what}: {e}")))
}

/// The built-in posts for a form to offer: (id, name).
pub fn manufacture_posts() -> Vec<(String, String)> {
    BUILTIN_POSTS.iter().filter_map(|(id, _)| Post::builtin(id).ok().map(|post| ((*id).to_string(), post.name))).collect()
}

/// Whether the built-in post `id` has an optional stop to write before tool changes, for a
/// form to offer the choice only where it means something.
pub fn manufacture_post_stops(id: &str) -> bool {
    Post::builtin(id).is_ok_and(|post| post.optional_stop.is_some())
}

/// One of the design's tools for a form to offer.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolChoice {
    pub number: u32,
    pub name: String,
    /// Its cutting diameter, mm.
    pub diameter: f64,
    /// Its thread's pitch, mm, when it is a tap: it cuts holes with the tap cycle at this pitch.
    pub pitch: Option<f64>,
    /// Whether it is a reamer: its holes are fed in and fed back out unless told otherwise.
    pub reams: bool,
}

/// The design's tools that can do `operation` (`face`, `drill`, `contour`, `bore`, `pocket` or `chamfer`), for a form to offer; every
/// tool for any other word. The operations' own checks decide, so a form never offers a tool its
/// command would refuse.
pub fn manufacture_tools(doc: &solidcraft_doc::Document, operation: &str) -> Vec<ToolChoice> {
    let Ok(data) = manufacture_data(doc) else { return Vec::new() };
    let drill = DrillOp {
        holes: vec![[0.0, 0.0]],
        order: Default::default(),
        top: None,
        depth: 1.0,
        depth_to: Default::default(),
        r_above: None,
        between: Default::default(),
        cycle: solidcraft_cam::DrillCycle::Drill { dwell: 0.0 },
        rpm: None,
        feed: None,
    };
    // A tap drills with the tap cycle at its own pitch.
    let pitch = |tool: &Tool| (tool.kind == solidcraft_cam::ToolKind::Tap).then_some(tool.geometry.thread_pitch);
    let taps = |tool: &Tool| {
        pitch(tool).is_some_and(|pitch| DrillOp { cycle: solidcraft_cam::DrillCycle::Tap { pitch }, ..drill.clone() }.validate(tool).is_ok())
    };
    let can = |tool: &Tool| match operation {
        "face" => FaceOp::default().validate(tool).is_ok(),
        "drill" => drill.validate(tool).is_ok() || taps(tool),
        "contour" | "bore" => ContourOp::default().validate(tool).is_ok(),
        "pocket" => PocketOp { pockets: vec![[0.0, 0.0]], ..PocketOp::default() }.validate(tool).is_ok(),
        "slot" => solidcraft_cam::SlotOp { slots: vec![[0.0, 0.0]], ..Default::default() }.validate(tool).is_ok(),
        "chamfer" => ChamferOp::default().validate(tool).is_ok(),
        _ => true,
    };
    data.tools
        .iter()
        .filter(|t| can(t))
        .map(|t| ToolChoice {
            number: t.number,
            name: t.name.clone(),
            diameter: t.geometry.diameter,
            pitch: pitch(t),
            reams: t.kind == solidcraft_cam::ToolKind::Reamer,
        })
        .collect()
}

/// An engine error in a view's own words: what is wrong, without the command's name.
fn said(e: crate::EngineError) -> String {
    match e {
        crate::EngineError::BadParams { msg, .. } => msg,
        other => other.to_string(),
    }
}

/// One operation as a view draws it.
pub struct OperationView {
    pub id: String,
    pub name: String,
    /// Its toolpath as straight pieces in the setup's WCS (how each moves, its two ends), or why
    /// it has none.
    pub pieces: std::result::Result<Vec<(Drawn, solidcore_record::Vec3, solidcore_record::Vec3)>, String>,
}

/// One setup as a view draws it: where it sits on the model, and its operations' toolpaths.
pub struct SetupView {
    pub id: String,
    pub placement: Placement,
    pub operations: Vec<OperationView>,
}

/// How far a drawn arc may stray from the true one, mm.
const VIEW_ARC_TOLERANCE: f64 = 0.02;

/// The setup `id` as a view draws it, or the design's first when `id` is `None` or names no setup
/// (one that was deleted). `Ok(None)` when the design has no setups; the reason when the setup
/// can't be placed on the model (nothing to machine, a work coordinate system this build can't
/// place yet). It reads the design and changes nothing.
/// Operations that are switched off are left out; each one starts where the one before ended, as
/// in the posted program.
pub fn manufacture_view(s: &Session, setup: Option<&str>) -> std::result::Result<Option<SetupView>, String> {
    const CMD: &str = "cam.view";
    let d = manufacture_data(&s.doc)?;
    let Some(setup) = setup.and_then(|id| d.setups.iter().find(|x| x.id == id)).or(d.setups.first()) else { return Ok(None) };
    let (model, _) = model_box(s, CMD, setup).map_err(said)?;
    let placement = setup.place(model).map_err(|e| e.to_string())?;
    let mut placed = Some(placement);
    let mut at = None;
    let mut operations = Vec::new();
    for op in setup.operations.iter().filter(|o| !o.suppressed) {
        let pieces = match work(s, CMD, &d, setup, op, at, &mut placed) {
            Ok(worked) => {
                at = Some(worked.toolpath.moves.iter().fold(worked.toolpath.start, |p, m| m.end(p)));
                worked.toolpath.drawn(VIEW_ARC_TOLERANCE).map_err(|e| e.to_string())
            }
            Err(e) => {
                at = None;
                Err(said(e))
            }
        };
        operations.push(OperationView { id: op.id.clone(), name: op.name.clone(), pieces });
    }
    Ok(Some(SetupView { id: setup.id.clone(), placement, operations }))
}

/// A preview's parameters as its command would get them: none at all is the empty object.
fn given(params: &Value) -> &Value {
    static NONE: std::sync::LazyLock<Value> = std::sync::LazyLock::new(|| Value::Object(Default::default()));
    if params.is_null() { &NONE } else { params }
}

/// An operation before it is made, as its form shows it.
pub struct OperationPreview {
    /// Where its setup sits on the model.
    pub placement: Placement,
    /// Its toolpath as straight pieces in the setup's WCS: how each moves and its two ends.
    pub pieces: Vec<(Drawn, solidcore_record::Vec3, solidcore_record::Vec3)>,
    /// Spindle speed, rev/min.
    pub rpm: f64,
    /// Cutting feed, mm/min.
    pub feed: f64,
    /// Time spent feeding and dwelling, minutes.
    pub cut_minutes: f64,
    /// What the strategy worked out, as `cam.op.toolpath` gives it (a contour's levels, a
    /// facing's passes).
    pub numbers: Value,
    /// Things worth showing beside the numbers (a capped spindle speed, a thin chip).
    pub notes: Vec<String>,
}

/// What `command` (`cam.op.face`, `cam.op.drill`, `cam.op.contour`, `cam.op.pocket` or `cam.op.chamfer`) would make from `params`,
/// without making it:
/// for a form to draw the toolpath and show the speed and feed it works out before OK, or to say
/// why there would be no toolpath. It reads the design and changes nothing.
pub fn manufacture_preview(s: &Session, command: &str, params: &Value) -> std::result::Result<OperationPreview, String> {
    const CMD: &str = "cam.preview";
    let (setup_id, name, tool, operation) = proposed(command, given(params)).map_err(said)?;
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == setup_id).ok_or_else(|| format!("the design has no setup '{setup_id}'"))?;
    // No id: it is not one of the design's operations yet.
    let op = Operation { id: String::new(), name: name.unwrap_or_default(), tool, suppressed: false, operation };
    let mut placed = None;
    let worked = work(s, CMD, &d, setup, &op, None, &mut placed).map_err(said)?;
    let placement = placed.ok_or_else(|| "the setup was not placed on the model".to_string())?;
    let stats = worked.toolpath.stats().map_err(|e| e.to_string())?;
    let number = |key: &str| worked.numbers.get(key).and_then(Value::as_f64).unwrap_or(0.0);
    Ok(OperationPreview {
        placement,
        pieces: worked.toolpath.drawn(VIEW_ARC_TOLERANCE).map_err(|e| e.to_string())?,
        rpm: number("rpm"),
        feed: number("feed"),
        cut_minutes: stats.feed_minutes + stats.dwell_minutes,
        numbers: worked.numbers,
        notes: worked.notes,
    })
}

/// Where the setup that `command` (`cam.setup.create` or `cam.setup.edit`) would leave from
/// `params` sits on the model, without making or changing it: for its form to draw the stock and
/// the work origin before OK, or to say why the command would refuse or the setup could not be
/// placed. It reads the design and changes nothing.
pub fn manufacture_setup_preview(s: &Session, command: &str, params: &Value) -> std::result::Result<Placement, String> {
    const CMD: &str = "cam.preview";
    let mut d = manufacture_data(&s.doc)?;
    let at = match command {
        "cam.setup.create" => {
            let setup = proposed_setup(CMD, &d, given(params)).map_err(said)?;
            d.setups.push(setup);
            d.setups.len() - 1
        }
        "cam.setup.edit" => {
            let id = edited_setups(CMD, &mut d, given(params)).map_err(said)?;
            d.setups.iter().position(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?
        }
        other => return Err(format!("`{other}` is not a command that makes or changes a setup")),
    };
    let setup = d.setups.get(at).ok_or_else(|| "the setup was not found again".to_string())?;
    let (model, _) = model_box(s, CMD, setup).map_err(said)?;
    let placement = setup.place(model).map_err(|e| e.to_string())?;
    // What saving it would check.
    d.validate().map_err(|e| e.to_string())?;
    Ok(placement)
}

/// What one operation did to the stock in a simulation.
#[derive(Clone, Debug, PartialEq)]
pub struct SimulatedOperation {
    pub id: String,
    pub name: String,
    /// The volume it removed, mm³.
    pub removed: f64,
    pub gouges: usize,
    pub rapid_hits: usize,
    /// How many of its moves brought the tool's shank or holder into material.
    pub body_hits: usize,
    /// Why it was left out, when its toolpath could not be worked out or its tool can't be shown.
    pub skipped: Option<String>,
}

/// A setup's operations cut from its stock (decision 0015 of the CAM work).
pub struct Simulated {
    /// Where the setup sits on the model.
    pub placement: Placement,
    /// The stock as it is left and what was found while cutting it, in the setup's WCS.
    pub stock: solidcraft_cam::Simulation,
    /// The stock's volume before, mm³.
    pub stock_volume: f64,
    pub operations: Vec<SimulatedOperation>,
}

/// How far below a chamfer's lower edge the part's wall is read for it, mm.
const CHAMFER_WALL_BELOW: f64 = 0.01;
/// How far below the part a cut is a gouge, and below the material's top a rapid is a hit, mm.
const SIMULATION_TOLERANCE: f64 = 0.01;
/// How many cells the stock's longer side is cut into when no cell is given, and the cells
/// that may give.
const SIMULATION_CELLS: f64 = 400.0;
const SIMULATION_CELL: (f64, f64) = (0.05, 1.0);

/// One operation of a setup made ready to be cut any part of the way ([`Playback`]).
pub struct PlaybackOperation {
    pub id: String,
    pub name: String,
    /// Its toolpath, its tool and how far it means to cut into the part as the model has it; or
    /// why it is left out, when its toolpath can't be worked out or its tool can't be shown.
    ready: std::result::Result<(Toolpath, Tool, f64), String>,
    /// How long its toolpath is, mm: 0 when it is left out.
    pub length: f64,
}

/// A setup made ready to be cut from its stock any part of the way (decision 0023 of the CAM
/// work): where it sits, its stock uncut with the part's heights over it, and its operations'
/// toolpaths in their order. Making it reads the model and works every toolpath out; cutting
/// any part of the way from it ([`Playback::at`]) does neither again.
pub struct Playback {
    placement: Placement,
    blank: solidcraft_cam::Simulation,
    stock_volume: f64,
    pub operations: Vec<PlaybackOperation>,
    /// How long the toolpaths are together, mm.
    pub length: f64,
}

/// Where the tool is part of the way through a setup's toolpaths.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolAt {
    /// Which of the setup's operations (those not switched off, in their order) it is in.
    pub operation: usize,
    pub tool: Tool,
    /// Its tip, in the setup's WCS.
    pub at: solidcore_record::Vec3,
    /// How far along that operation's toolpath it is, mm, and the time it has spent in it
    /// cutting and dwelling, minutes.
    pub along: f64,
    pub minutes: f64,
}

impl Playback {
    /// The stock after the first `along` mm of the setup's toolpaths (all of them at
    /// [`Playback::length`] or past it; none below 0), with what was found while cutting that
    /// far, and where the tool is then: nowhere once every toolpath is done. An operation the
    /// tool has not reached has removed nothing.
    pub fn at(&self, along: f64) -> std::result::Result<(Simulated, Option<ToolAt>), String> {
        if along.is_nan() {
            return Err("how far along a setup's toolpaths must be a number".into());
        }
        let whole = along >= self.length;
        let mut stock = self.blank.clone();
        let (mut left, mut tool_at) = (along.max(0.0), None);
        let mut operations = Vec::with_capacity(self.operations.len());
        for (index, op) in self.operations.iter().enumerate() {
            let said = |cut: Option<solidcraft_cam::CutReport>, skipped: Option<String>| SimulatedOperation {
                id: op.id.clone(),
                name: op.name.clone(),
                removed: cut.map_or(0.0, |cut| cut.removed),
                gouges: cut.map_or(0, |cut| cut.gouges),
                rapid_hits: cut.map_or(0, |cut| cut.rapid_hits),
                body_hits: cut.map_or(0, |cut| cut.body_hits),
                skipped,
            };
            operations.push(match &op.ready {
                Err(why) => said(None, Some(why.clone())),
                // Not reached: the tool is in an operation before it.
                Ok(_) if tool_at.is_some() => said(None, None),
                Ok((toolpath, tool, into_part)) => {
                    let cut = if whole || left >= op.length {
                        // Done whole: cut as it is, so that all of them done is the simulation.
                        left -= op.length;
                        stock.cut_into(index, toolpath, tool, *into_part)
                    } else {
                        let part = toolpath.upto(left).map_err(|e| e.to_string())?;
                        tool_at = Some(ToolAt { operation: index, tool: tool.clone(), at: part.at, along: part.along, minutes: part.minutes });
                        stock.cut_into(index, &part.toolpath, tool, *into_part)
                    };
                    match cut {
                        Ok(cut) => said(Some(cut), None),
                        Err(why) => said(None, Some(why.to_string())),
                    }
                }
            });
        }
        Ok((Simulated { placement: self.placement, stock, stock_volume: self.stock_volume, operations }, tool_at))
    }
}

/// The setup `id` made ready to be cut any part of the way: its operations (but those switched
/// off) in turn, on a grid of cells `cell` mm a side (`None`: the stock's longer side over 400,
/// between 0.05 and 1), the part's heights read from the bodies' exact faces. An operation
/// whose toolpath can't be worked out, or whose tool a field of heights can't show, is left
/// out, with the reason. It reads the design and changes nothing.
pub fn manufacture_playback(s: &Session, id: &str, cell: Option<f64>) -> std::result::Result<Playback, String> {
    const CMD: &str = "cam.simulate";
    let d = manufacture_data(&s.doc)?;
    let setup = d.setups.iter().find(|x| x.id == id).ok_or_else(|| format!("the design has no setup '{id}'"))?;
    let (model, _) = model_box(s, CMD, setup).map_err(said)?;
    let placement = setup.place(model).map_err(|e| e.to_string())?;
    let size = placement.stock.size();
    let cell = cell.unwrap_or_else(|| (size.x.max(size.y) / SIMULATION_CELLS).clamp(SIMULATION_CELL.0, SIMULATION_CELL.1));
    let stock = solidcraft_cam::Simulation::new(placement.stock, cell, SIMULATION_TOLERANCE).map_err(|e| e.to_string())?;
    let stock_volume = size.x * size.y * size.z;
    // The part over the stock's cells, in the WCS: anything below the stock where there is none.
    let plane = solidcore_record::Plane { origin: placement.origin, x: placement.x_axis, y: placement.y_axis };
    let part = with_bodies(s, CMD, setup, |bodies| {
        let named: Vec<(&str, &solidcraft_kernel::Body)> = bodies.iter().map(|b| (b.name.as_str(), &b.body)).collect();
        super::cam_heights::part_heights(&named, &plane, &stock.stock, placement.model.max.z, placement.stock.min.z - 1.0).map_err(|e| bad(CMD, e))
    })
    .map_err(said)?;
    let blank = stock.with_part(part).map_err(|e| e.to_string())?;
    let mut placed = Some(placement);
    let mut at = None;
    let mut operations = Vec::new();
    for op in setup.operations.iter().filter(|o| !o.suppressed) {
        let tool = d.tools.iter().find(|t| t.number == op.tool);
        let ready = work(s, CMD, &d, setup, op, at, &mut placed).map_err(said).and_then(|worked| {
            at = Some(worked.toolpath.moves.iter().fold(worked.toolpath.start, |p, m| m.end(p)));
            let tool = tool.ok_or_else(|| format!("the design has no tool {}", op.tool))?;
            solidcraft_cam::Simulation::cuts_with(tool).map_err(|e| e.to_string())?;
            Ok((worked.toolpath, tool.clone(), worked.into_part))
        });
        // Where an operation that is left out would have ended is not where the next begins.
        if ready.is_err() {
            at = None;
        }
        let length = ready.as_ref().ok().and_then(|(toolpath, ..)| toolpath.length().ok()).unwrap_or(0.0);
        operations.push(PlaybackOperation { id: op.id.clone(), name: op.name.clone(), ready, length });
    }
    let length = operations.iter().map(|op| op.length).sum();
    Ok(Playback { placement, blank, stock_volume, operations, length })
}

/// The setup `id`'s operations (but those switched off) cut in turn from its stock, and checked
/// against the part: [`manufacture_playback`] cut all the way.
pub fn manufacture_simulation(s: &Session, id: &str, cell: Option<f64>) -> std::result::Result<Simulated, String> {
    manufacture_playback(s, id, cell)?.at(f64::INFINITY).map(|(simulated, _)| simulated)
}

fn simulate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.simulate";
    let p: CamSimulateParams = parse(CMD, "parameters", p)?;
    let playback = manufacture_playback(s, &p.setup, p.cell).map_err(|e| bad(CMD, e))?;
    let (done, tool) = playback.at(p.along.unwrap_or(f64::INFINITY)).map_err(|e| bad(CMD, e))?;
    let (over_part, beside_part) = done.stock.leftover();
    let id_of = |index: usize| done.operations.get(index).map_or_else(String::new, |op| op.id.clone());
    reply(
        CMD,
        CamSimulated {
            setup: p.setup,
            cell: done.stock.stock.cell,
            cells: [done.stock.stock.nx, done.stock.stock.ny],
            tolerance: done.stock.tolerance,
            stock_volume: done.stock_volume,
            removed: done.stock.removed,
            over_part,
            beside_part,
            below_stock: done.stock.below_stock,
            length: playback.length,
            along: p.along.map_or(playback.length, |along| along.clamp(0.0, playback.length)),
            tool: tool.map(|tool| crate::schema::cam::CamSimulatedTool {
                operation: id_of(tool.operation),
                number: tool.tool.number,
                at: [tool.at.x, tool.at.y, tool.at.z],
                minutes: tool.minutes,
            }),
            operations: done
                .operations
                .iter()
                .map(|op| CamSimulatedOperation {
                    id: op.id.clone(),
                    name: op.name.clone(),
                    removed: op.removed,
                    gouges: op.gouges,
                    rapid_hits: op.rapid_hits,
                    body_hits: op.body_hits,
                    skipped: op.skipped.clone(),
                })
                .collect(),
            gouge_count: done.stock.gouge_count,
            gouges: done
                .stock
                .gouges
                .iter()
                .map(|g| CamSimulatedHit { operation: id_of(g.operation), move_index: g.move_index, at: [g.at.x, g.at.y, g.at.z], depth: g.depth })
                .collect(),
            rapid_hit_count: done.stock.rapid_hit_count,
            rapid_hits: done
                .stock
                .rapid_hits
                .iter()
                .map(|h| CamSimulatedHit { operation: id_of(h.operation), move_index: h.move_index, at: [h.at.x, h.at.y, h.at.z], depth: h.depth })
                .collect(),
            body_hit_count: done.stock.body_hit_count,
            body_hits: done
                .stock
                .body_hits
                .iter()
                .map(|h| crate::schema::cam::CamSimulatedBodyHit {
                    operation: id_of(h.operation),
                    move_index: h.move_index,
                    part: body_part(h.part).to_string(),
                    at: [h.at.x, h.at.y, h.at.z],
                    depth: h.depth,
                })
                .collect(),
        },
    )
}

/// A part of a tool that does not cut, in a word.
fn body_part(part: solidcraft_cam::BodyPart) -> &'static str {
    match part {
        solidcraft_cam::BodyPart::Shank => "shank",
        solidcraft_cam::BodyPart::Holder => "holder",
    }
}

fn op_toolpath_command(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.toolpath";
    let p: CamOpToolpathParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let (setup, op) = d
        .setups
        .iter()
        .find_map(|x| x.operations.iter().find(|o| o.id == p.id).map(|o| (x, o)))
        .ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    let worked = work(s, CMD, &d, setup, op, None, &mut None)?;
    let stats = worked.toolpath.stats().map_err(|e| bad(CMD, e.to_string()))?;
    reply(
        CMD,
        CamOpToolpath {
            id: p.id,
            setup: setup.id.clone(),
            toolpath: serde_json::to_value(&worked.toolpath).map_err(|e| bad(CMD, e.to_string()))?,
            feed_length: stats.feed_length,
            rapid_length: stats.rapid_length,
            cut_minutes: stats.feed_minutes + stats.dwell_minutes,
            min: [stats.min.x, stats.min.y, stats.min.z],
            max: [stats.max.x, stats.max.y, stats.max.z],
            numbers: worked.numbers,
            notes: worked.notes,
        },
    )
}

fn op_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.edit";
    let p: CamOpEditParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    let setup = d
        .setups
        .iter_mut()
        .find(|x| x.operations.iter().any(|o| o.id == p.id))
        .ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    let (setup_id, operations) = (setup.id.clone(), setup.operations.len());
    let op = setup.operations.iter_mut().find(|o| o.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    if let Some(name) = p.name {
        op.name = name;
    }
    if let Some(tool) = p.tool {
        op.tool = tool;
    }
    if let Some(suppressed) = p.suppressed {
        op.suppressed = suppressed;
    }
    // Its own parameters, whole; another kind's are a mistake, not something to drop quietly.
    let (kind, takes) = match &op.operation {
        OperationKind::Manual { .. } => ("a hand-made toolpath", "toolpath"),
        OperationKind::Drill { .. } => ("a drill operation", "drill"),
        OperationKind::Face { .. } => ("a facing", "face"),
        OperationKind::Contour { .. } => ("a contour", "contour"),
        OperationKind::Pocket { .. } => ("a pocket operation", "pocket"),
        OperationKind::Chamfer { .. } => ("a chamfer", "chamfer"),
        OperationKind::Slot { .. } => ("a slot operation", "slot"),
    };
    let given = [
        (&p.toolpath, "toolpath"),
        (&p.drill, "drill"),
        (&p.face, "face"),
        (&p.contour, "contour"),
        (&p.pocket, "pocket"),
        (&p.chamfer, "chamfer"),
        (&p.slot, "slot"),
    ];
    for (given, name) in given {
        if given.is_some() && name != takes {
            return Err(bad(CMD, format!("operation {} is {kind}: it takes `{takes}`, not `{name}`", op.id)));
        }
    }
    match &mut op.operation {
        OperationKind::Manual { toolpath } => {
            if let Some(v) = &p.toolpath {
                *toolpath = parse(CMD, "toolpath", v)?;
            }
        }
        OperationKind::Drill { drill } => {
            if let Some(v) = &p.drill {
                *drill = parse(CMD, "drill", v)?;
            }
        }
        OperationKind::Face { face } => {
            if let Some(v) = &p.face {
                *face = parse(CMD, "face", v)?;
            }
        }
        OperationKind::Contour { contour } => {
            if let Some(v) = &p.contour {
                *contour = parse(CMD, "contour", v)?;
            }
        }
        OperationKind::Pocket { pocket } => {
            if let Some(v) = &p.pocket {
                *pocket = parse(CMD, "pocket", v)?;
            }
        }
        OperationKind::Chamfer { chamfer } => {
            if let Some(v) = &p.chamfer {
                *chamfer = parse(CMD, "chamfer", v)?;
            }
        }
        OperationKind::Slot { slot } => {
            if let Some(v) = &p.slot {
                *slot = parse(CMD, "slot", v)?;
            }
        }
    }
    store(s, CMD, &d)?;
    reply(CMD, CamOpWritten { id: p.id, setup: setup_id, operations })
}

fn op_delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.delete";
    let p: CamOpDeleteParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    let setup = d
        .setups
        .iter_mut()
        .find(|x| x.operations.iter().any(|o| o.id == p.id))
        .ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    setup.operations.retain(|o| o.id != p.id);
    let (setup_id, operations) = (setup.id.clone(), setup.operations.len());
    store(s, CMD, &d)?;
    reply(CMD, CamOpWritten { id: p.id, setup: setup_id, operations })
}

fn op_move(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.move";
    let p: CamOpMoveParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    let found = d.setups.iter_mut().find_map(|x| x.operations.iter().position(|o| o.id == p.id).map(|at| (x, at)));
    let (setup, at) = found.ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    let op = setup.operations.remove(at);
    let to = p.to.min(setup.operations.len());
    setup.operations.insert(to, op);
    let (setup_id, operations) = (setup.id.clone(), setup.operations.len());
    store(s, CMD, &d)?;
    reply(CMD, CamOpWritten { id: p.id, setup: setup_id, operations })
}

fn op_duplicate(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.op.duplicate";
    let p: CamOpDuplicateParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    let id = next_id("op", d.setups.iter().flat_map(|x| &x.operations).map(|o| o.id.as_str()));
    let found = d.setups.iter_mut().find_map(|x| x.operations.iter().position(|o| o.id == p.id).map(|at| (x, at)));
    let (setup, at) = found.ok_or_else(|| bad(CMD, format!("the design has no operation '{}'", p.id)))?;
    let Some(mut copy) = setup.operations.get(at).cloned() else { return Err(bad(CMD, format!("the design has no operation '{}'", p.id))) };
    // Its own name: the one given, or the operation's with the first number its setup has not.
    let taken = |name: &str| setup.operations.iter().any(|o| o.name == name);
    let numbered = (2..setup.operations.len() + 3).map(|n| format!("{} ({n})", copy.name)).find(|name| !taken(name));
    copy.name = p.name.or(numbered).unwrap_or_else(|| copy.name.clone());
    copy.id.clone_from(&id);
    setup.operations.insert(at + 1, copy);
    let (setup_id, operations) = (setup.id.clone(), setup.operations.len());
    store(s, CMD, &d)?;
    reply(CMD, CamOpWritten { id, setup: setup_id, operations })
}

/// Where a setup's work origin is, in a sheet's words.
fn origin_words(origin: &WcsOrigin) -> String {
    match origin {
        WcsOrigin::StockBox { anchor } => {
            let level = match anchor.z {
                BoxSide::Max => "top",
                BoxSide::Min => "bottom",
                BoxSide::Center => "middle height",
            };
            let across = match anchor.x {
                BoxSide::Min => "left",
                BoxSide::Max => "right",
                BoxSide::Center => "",
            };
            let along = match anchor.y {
                BoxSide::Min => "front",
                BoxSide::Max => "back",
                BoxSide::Center => "",
            };
            let place = match (along, across) {
                ("", "") => "the middle".to_string(),
                ("", side) | (side, "") => format!("the middle of the {side} edge"),
                (along, across) => format!("the {along} {across} corner"),
            };
            format!("{place} of the stock's {level}")
        }
        WcsOrigin::ModelPoint { reference } => format!("the model's point {reference}"),
        WcsOrigin::Explicit { point } => format!("X {} Y {} Z {} of the model", point.x, point.y, point.z),
    }
}

/// What an operation is, in a sheet's words: its kind, but a drilling that taps is a tap, one
/// that reams is a ream, and a contour inside its openings says so.
fn operation_words(operation: &OperationKind) -> String {
    match operation {
        OperationKind::Drill { drill } if matches!(drill.cycle, solidcraft_cam::DrillCycle::Tap { .. }) => "tap".into(),
        OperationKind::Drill { drill } if matches!(drill.cycle, solidcraft_cam::DrillCycle::Ream { .. }) => "ream".into(),
        OperationKind::Contour { contour } if contour.side == ContourSide::Inside => "inside contour".into(),
        other => kind_words(other),
    }
}

/// A kind as serde names it (`flat_end_mill`), in words.
fn kind_words(kind: &impl serde::Serialize) -> String {
    let named = serde_json::to_value(kind).unwrap_or(Value::Null);
    named.as_str().or_else(|| named.get("kind").and_then(Value::as_str)).unwrap_or("").replace('_', " ")
}

fn setup_sheet(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.sheet";
    let p: crate::schema::cam::CamSetupSheetParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.setup).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.setup)))?;
    let units = match p.units.as_deref() {
        Some("mm") => LengthUnit::Millimetre,
        Some("inch") => LengthUnit::Inch,
        Some(other) => return Err(bad(CMD, format!("units must be mm or inch, not '{other}'"))),
        None => {
            // The design's own: inches where it is drawn in inches or feet.
            let per_mm = crate::doc::display_units::unit_scale(&s.doc.units).0;
            if [25.4, 304.8].iter().any(|mm| (per_mm * mm - 1.0).abs() < 1e-9) { LengthUnit::Inch } else { LengthUnit::Millimetre }
        }
    };
    if let Some(path) = &p.path
        && !(path.to_ascii_lowercase().ends_with(".html") || path.to_ascii_lowercase().ends_with(".htm"))
    {
        return Err(bad(CMD, "a setup sheet is written to an .html file"));
    }
    // The operations as Post Process works them out, in the same order; one whose toolpath can't
    // be worked out is listed with the reason, and the next starts afresh.
    let (mut placed, mut at) = (None, None);
    let (mut operations, mut notes) = (Vec::new(), Vec::new());
    for op in setup.operations.iter().filter(|o| !o.suppressed) {
        let listed = work(s, CMD, &d, setup, op, at, &mut placed).map_err(said).and_then(|worked| {
            let stats = worked.toolpath.stats().map_err(|e| e.to_string())?;
            let pieces = worked.toolpath.drawn(VIEW_ARC_TOLERANCE).map_err(|e| e.to_string())?;
            Ok((worked, stats, pieces))
        });
        let named = solidcraft_cam::SheetOperation {
            name: op.name.clone(),
            tool: op.tool,
            kind: operation_words(&op.operation),
            rpm: None,
            feed: None,
            lowest_z: None,
            minutes: None,
            trouble: None,
            cuts: Vec::new(),
        };
        operations.push(match listed {
            Ok((worked, stats, pieces)) => {
                at = Some(worked.toolpath.moves.iter().fold(worked.toolpath.start, |p, m| m.end(p)));
                notes.extend(worked.notes.iter().map(|note| format!("{}: {note}", op.name)));
                let number = |key: &str| worked.numbers.get(key).and_then(Value::as_f64);
                solidcraft_cam::SheetOperation {
                    rpm: number("rpm"),
                    feed: number("feed"),
                    lowest_z: Some(stats.min.z),
                    minutes: Some(stats.feed_minutes + stats.dwell_minutes),
                    cuts: pieces.iter().filter(|(how, ..)| *how != Drawn::Rapid).map(|(_, a, b)| [a.x, a.y, b.x, b.y]).collect(),
                    ..named
                }
            }
            Err(why) => {
                at = None;
                solidcraft_cam::SheetOperation { trouble: Some(why), ..named }
            }
        });
    }
    let placement = match placed {
        Some(placement) => placement,
        None => {
            let (model, _) = model_box(s, CMD, setup)?;
            setup.place(model).map_err(|e| bad(CMD, e.to_string()))?
        }
    };
    // What the simulation found, in a line each: it is the last look before the machine.
    match manufacture_simulation(s, &setup.id, None) {
        Ok(done) => {
            let sim = &done.stock;
            let name_of = |index: usize| done.operations.get(index).map_or("an operation", |op| op.name.as_str());
            if let Some(deepest) = sim.gouges.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                notes.push(format!(
                    "The simulation found {} move(s) that cut into the part, the deepest {:.2} mm, in {}.",
                    sim.gouge_count,
                    deepest.depth,
                    name_of(deepest.operation)
                ));
            }
            if let Some(deepest) = sim.rapid_hits.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                notes.push(format!(
                    "The simulation found {} rapid move(s) through material, the deepest {:.2} mm, in {}.",
                    sim.rapid_hit_count,
                    deepest.depth,
                    name_of(deepest.operation)
                ));
            }
            if let Some(deepest) = sim.body_hits.iter().max_by(|a, b| a.depth.total_cmp(&b.depth)) {
                notes.push(format!(
                    "The simulation found {} move(s) with a tool's shank or holder in material, the deepest {:.2} mm (the {}), in {}.",
                    sim.body_hit_count,
                    deepest.depth,
                    body_part(deepest.part),
                    name_of(deepest.operation)
                ));
            }
            if sim.below_stock > sim.tolerance {
                notes.push(format!("A tool goes {:.2} mm below the bottom of the stock: check what is under the part.", sim.below_stock));
            }
            notes.extend(done.operations.iter().filter_map(|op| op.skipped.as_ref().map(|why| format!("{} was not simulated: {why}", op.name))));
        }
        Err(why) => notes.push(format!("The simulation could not be run: {why}")),
    }
    let used: Vec<u32> = operations.iter().map(|o| o.tool).collect();
    let tools: Vec<solidcraft_cam::SheetTool> = d
        .tools
        .iter()
        .filter(|t| used.contains(&t.number))
        .map(|t| solidcraft_cam::SheetTool {
            number: t.number,
            name: t.name.clone(),
            kind: kind_words(&t.kind),
            diameter: t.geometry.diameter,
            flutes: t.flutes,
            stick_out: t.holder.stick_out,
            length_offset: t.length_offset,
            diameter_offset: t.diameter_offset,
        })
        .collect();
    let sheet = solidcraft_cam::SetupSheet {
        title: s.doc.name.clone(),
        setup: setup.name.clone(),
        date: p.date,
        units,
        material: setup.material.clone(),
        work_offset: setup.work_offset,
        origin: origin_words(&setup.wcs.origin),
        placement,
        tools,
        operations,
        notes,
    };
    let html = sheet.html().map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(path) = &p.path {
        write_file(CMD, path, html.as_bytes(), p.overwrite, "the setup sheet")?;
    }
    reply(
        CMD,
        crate::schema::cam::CamSetupSheet {
            html,
            tools: sheet.tools.len(),
            operations: sheet.operations.len(),
            left_out: sheet.operations.iter().filter(|o| o.trouble.is_some()).count(),
            minutes: sheet.operations.iter().filter_map(|o| o.minutes).sum(),
            notes: sheet.notes,
            path: p.path,
        },
    )
}

fn post(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.post";
    let p: CamPostParams = parse(CMD, "parameters", p)?;
    let d = load(s, CMD)?;
    let setup = d.setups.iter().find(|x| x.id == p.setup).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.setup)))?;
    let posts: Vec<&str> = BUILTIN_POSTS.iter().map(|(id, _)| *id).collect();
    if !posts.contains(&p.post.as_str()) {
        return Err(bad(CMD, format!("no post '{}' (built in: {})", p.post, posts.join(", "))));
    }
    let post = Post::builtin(&p.post).map_err(|e| bad(CMD, e.to_string()))?;
    let units = match p.units.as_deref() {
        None | Some("mm") => LengthUnit::Millimetre,
        Some("inch") => LengthUnit::Inch,
        Some(other) => return Err(bad(CMD, format!("units must be mm or inch, not '{other}'"))),
    };
    // One program: the setup's operations in order, each starting where the one before ended.
    let mut program: Option<Toolpath> = None;
    let mut placed = None;
    let mut notes = Vec::new();
    // Where each operation starts in the program, with its name (decision 0029 of the CAM work).
    let mut starts: Vec<(usize, String)> = Vec::new();
    for op in setup.operations.iter().filter(|o| !o.suppressed) {
        let start = program.as_ref().map(|all| all.moves.iter().fold(all.start, |at, m| m.end(at)));
        let worked = work(s, CMD, &d, setup, op, start, &mut placed)?;
        notes.extend(worked.notes.into_iter().map(|n| format!("operation {}: {n}", op.id)));
        let toolpath = &worked.toolpath;
        starts.push((program.as_ref().map_or(0, |all| all.moves.len()), op.name.clone()));
        match &mut program {
            None => program = Some(toolpath.clone()),
            Some(all) => {
                let end = all.moves.iter().fold(all.start, |at, m| m.end(at));
                let gap = ((end.x - toolpath.start.x).powi(2) + (end.y - toolpath.start.y).powi(2) + (end.z - toolpath.start.z).powi(2)).sqrt();
                if !(gap <= 1e-6) {
                    return Err(bad(
                        CMD,
                        format!("operation {} starts {gap:.4} mm away from where the one before it ends; give it a rapid from there", op.id),
                    ));
                }
                all.moves.extend(toolpath.moves.iter().copied());
            }
        }
    }
    // Operations that follow one another with the same tool keep it: no change to the tool that
    // is in the spindle, and nothing stopped for one (decision 0022 of the CAM work).
    let marks: Vec<usize> = starts.iter().map(|start| start.0).collect();
    let (program, at) =
        program.ok_or_else(|| bad(CMD, format!("setup '{}' has no operations to post", setup.id)))?.without_needless_changes_marked(&marks);
    // What the program says of itself: each operation's name where it starts, and its tools.
    let said = p.comments.unwrap_or(true);
    let ctx = ProgramContext {
        program_number: p.program_number.unwrap_or(1),
        program_name: p.program_name.unwrap_or_else(|| setup.name.clone()),
        units,
        work_offset: setup.work_offset,
        optional_stop: p.optional_stop.unwrap_or(true),
        comments: if said { at.into_iter().zip(starts.into_iter().map(|start| start.1)).collect() } else { Vec::new() },
        tools: if said {
            d.tools.iter().map(|tool| ProgramTool { number: tool.number, name: tool.name.clone(), diameter: tool.geometry.diameter }).collect()
        } else {
            Vec::new()
        },
    };
    let posted = post.write(&program, &ctx).map_err(|e| bad(CMD, e.to_string()))?;
    if let Some(path) = &p.path {
        write_file(CMD, path, posted.text.as_bytes(), p.overwrite, "the program")?;
    }
    reply(
        CMD,
        CamPosted {
            post: p.post,
            lines: posted.text.lines().count(),
            text: posted.text,
            moves: posted.written.moves.len(),
            machine_retracts: posted.read_back.machine_retracts,
            tolerance_mm: posted.tolerance,
            notes: notes.into_iter().chain(posted.notes).collect(),
            path: p.path,
        },
    )
}

#[cfg(test)]
#[path = "cam_tests.rs"]
mod tests;
