//! `cam.*`: the Manufacture workspace's commands. Its data lives in the design as plugin data
//! under `plugins/solidcraft-cam/` (decision 0006 of the CAM work), written here as that folder's
//! built-in owner through the same storage code plugins use, so its size limits and metadata
//! are the same. Every change is one undoable edit; a design whose Manufacture data can't be read
//! keeps it as it is, and changing commands refuse rather than overwrite it.
use serde::de::DeserializeOwned;
use serde_json::Value;
use solidcraft_cam::{
    BUILTIN_POSTS, Bounds, BoxAnchor, BoxOffsets, BoxSide, DrillOp, FaceOp, LengthUnit, MANUFACTURE_DATA_OWNER, MANUFACTURE_FORMAT, ManufactureData,
    Operation, OperationKind, Post, ProgramContext, SafeHeights, Setup, SetupKind, Stock, Tool, Toolpath, Wcs, WcsOrientation, WcsOrigin,
};
use solidcraft_doc::plugin_data::DATA;

use super::CommandSpec;
use crate::plugin::storage::{self, StorageCaller};
use crate::schema::cam::{
    CamData, CamDataParams, CamOpDeleteParams, CamOpDrillParams, CamOpFaceParams, CamOpManualParams, CamOpToolpath, CamOpToolpathParams,
    CamOpWritten, CamPostParams, CamPosted, CamSetupCreateParams, CamSetupDeleteParams, CamSetupEditParams, CamSetupMeasureParams, CamSetupMeasured,
    CamSetupWritten, CamToolCreateParams, CamToolDeleteParams, CamToolEditParams, CamToolWritten,
};
use crate::schema::{TypedContract, output_schema_for};
use crate::{Result, Session, params::bad};

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec::new("cam.data", "Manufacture Data", crate::checked!("cam.data", data)).icon("inspect").noundo().read_only().doc_only()
        .contract(&TypedContract::<CamDataParams>::new()).output(output_schema_for::<CamData>)
        .params("→ data: the design's Manufacture data {format, tools, setups} (empty when it has none), setups, tools, operations: counts"),
    CommandSpec::new("cam.tool.create", "Add Tool", crate::checked!("cam.tool.create", tool_create)).icon("edit").doc_only()
        .contract(&TypedContract::<CamToolCreateParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("tool: a tool as in a tool library file, its number free in the design → number, tools: how many the design has"),
    CommandSpec::new("cam.tool.edit", "Edit Tool", crate::checked!("cam.tool.edit", tool_edit)).icon("edit").doc_only()
        .contract(&TypedContract::<CamToolEditParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("number: the tool to replace, tool: the new tool (operations must still find their tool numbers) → number, tools"),
    CommandSpec::new("cam.tool.delete", "Delete Tool", crate::checked!("cam.tool.delete", tool_delete)).icon("delete").doc_only()
        .contract(&TypedContract::<CamToolDeleteParams>::new()).output(output_schema_for::<CamToolWritten>)
        .params("number → removes one of the design's tools; refused while an operation uses it"),
    CommandSpec::new("cam.setup.create", "New Setup", crate::checked!("cam.setup.create", setup_create)).icon("edit").doc_only()
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
    CommandSpec::new("cam.op.manual", "Hand-made Toolpath", crate::checked!("cam.op.manual", op_manual)).icon("edit").doc_only()
        .contract(&TypedContract::<CamOpManualParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: one of the design's numbers, toolpath: {start, moves} in mm and mm/min → id of the new operation"),
    CommandSpec::new("cam.op.drill", "Drill", crate::checked!("cam.op.drill", op_drill)).icon("edit").doc_only()
        .contract(&TypedContract::<CamOpDrillParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a drill, spot drill, reamer or flat end mill, drill: {holes: [[x, y]] in the WCS, depth, cycle, order?, top?, depth_to?, r_above?, between?, rpm?, feed?} → id of the new operation; its toolpath is worked out when asked for"),
    CommandSpec::new("cam.op.face", "Face", crate::checked!("cam.op.face", op_face)).icon("edit").doc_only()
        .contract(&TypedContract::<CamOpFaceParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("setup: id, name?, tool: a face mill, flat end mill or bull-nose end mill, face?: {top?, bottom?, stepdown?, stepover?, clear?, pattern?: zig_zag | one_way, rpm?, feed?} → id of the new operation: the stock's top down to the model's top in straight passes along X"),
    CommandSpec::new("cam.op.toolpath", "Operation Toolpath", crate::checked!("cam.op.toolpath", op_toolpath_command)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamOpToolpathParams>::new()).output(output_schema_for::<CamOpToolpath>)
        .params("id: an operation → toolpath worked out now from its parameters, the setup's placement and the tool, with feed_length, rapid_length, cut_minutes, min, max, numbers, notes"),
    CommandSpec::new("cam.op.delete", "Delete Operation", crate::checked!("cam.op.delete", op_delete)).icon("delete").doc_only()
        .contract(&TypedContract::<CamOpDeleteParams>::new()).output(output_schema_for::<CamOpWritten>)
        .params("id → removes the operation"),
    CommandSpec::new("cam.post", "Post Process", crate::checked!("cam.post", post)).icon("inspect").noundo().read_only()
        .contract(&TypedContract::<CamPostParams>::new()).output(output_schema_for::<CamPosted>)
        .params("setup: id, post: rs274 | fanuc_mill | haas_mill, program_number?: default 1, program_name?, units?: mm | inch, path?: also write it to this file, overwrite?: replace an existing file (default false) → text: the program, read back and compared move by move with what was written; never safe to run unproven: prove it on the machine (air cut, single block) first"),
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

fn setup_edit(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "cam.setup.edit";
    let p: CamSetupEditParams = parse(CMD, "parameters", p)?;
    let mut d = load(s, CMD)?;
    let setup = d.setups.iter_mut().find(|x| x.id == p.id).ok_or_else(|| bad(CMD, format!("the design has no setup '{}'", p.id)))?;
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
    apply(CMD, setup, fields)?;
    store(s, CMD, &d)?;
    reply(CMD, CamSetupWritten { id: p.id, setups: d.setups.len() })
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

/// The exact box, in model coordinates, of the bodies a setup machines: its model bodies by name,
/// or every visible body when it lists none. Returns the box and the bodies' names.
pub(super) fn model_box(s: &Session, cmd: &str, setup: &Setup) -> Result<(Bounds, Vec<String>)> {
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
    let mut boxes = bodies.iter().map(|b| b.body.exact_bounds().map_err(|e| bad(cmd, format!("measuring body '{}': {e}", b.name))));
    let first = boxes
        .next()
        .ok_or_else(|| bad(cmd, format!("setup '{}' has no bodies to machine: show one, or list them in its model_bodies", setup.id)))??;
    let mut b = first;
    for next in boxes {
        b = b.union(&next?);
    }
    Ok((Bounds { min: b.min, max: b.max }, bodies.iter().map(|b| b.name.clone()).collect()))
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

/// What the parameters of `cmd` (`cam.op.drill` or `cam.op.face`) describe: the setup's id, the
/// name given, the tool's number and the operation.
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
        other => Err(bad(other, "not a command that adds a drill or face operation")),
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
            Ok(Worked { toolpath: toolpath.clone(), numbers: Value::Object(Default::default()), notes: Vec::new() })
        }
        OperationKind::Drill { drill } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            let done = drill.toolpath(start, tool, &placement, material).map_err(failed)?;
            let numbers = serde_json::json!({"rpm": done.rpm, "feed": done.feed, "bottom": done.bottom, "r": done.r, "clearance": done.clearance});
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes })
        }
        OperationKind::Face { face } => {
            let (tool, placement) = tool_and_placement(s, cmd, d, setup, op, placed)?;
            let done = face.toolpath(start, tool, &placement, material).map_err(failed)?;
            let numbers = serde_json::json!({
                "rpm": done.rpm, "feed": done.feed, "plunge_feed": done.plunge_feed, "levels": done.levels, "passes": done.passes, "stepover": done.stepover
            });
            Ok(Worked { toolpath: done.toolpath, numbers, notes: done.notes })
        }
    }
}

// The Manufacture types a view of a setup is made of, for the UI above the engine.
pub use solidcraft_cam::{Drawn, Placement};

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

/// One of the design's tools for a form to offer.
#[derive(Clone, Debug, PartialEq)]
pub struct ToolChoice {
    pub number: u32,
    pub name: String,
    /// Its cutting diameter, mm.
    pub diameter: f64,
}

/// The design's tools that can do `operation` (`face` or `drill`), for a form to offer; every
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
    let can = |tool: &Tool| match operation {
        "face" => FaceOp::default().validate(tool).is_ok(),
        "drill" => drill.validate(tool).is_ok(),
        _ => true,
    };
    data.tools.iter().filter(|t| can(t)).map(|t| ToolChoice { number: t.number, name: t.name.clone(), diameter: t.geometry.diameter }).collect()
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
    /// Things worth showing beside the numbers (a capped spindle speed, a thin chip).
    pub notes: Vec<String>,
}

/// What `command` (`cam.op.face` or `cam.op.drill`) would make from `params`, without making it:
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
        notes: worked.notes,
    })
}

/// Where the setup `cam.setup.create` would make from `params` sits on the model, without making
/// it: for its form to draw the stock and the work origin before OK, or to say why the command
/// would refuse or the setup could not be placed. It reads the design and changes nothing.
pub fn manufacture_setup_preview(s: &Session, params: &Value) -> std::result::Result<Placement, String> {
    const CMD: &str = "cam.preview";
    let mut d = manufacture_data(&s.doc)?;
    let setup = proposed_setup(CMD, &d, given(params)).map_err(said)?;
    let (model, _) = model_box(s, CMD, &setup).map_err(said)?;
    let placement = setup.place(model).map_err(|e| e.to_string())?;
    // What saving it would check.
    d.setups.push(setup);
    d.validate().map_err(|e| e.to_string())?;
    Ok(placement)
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
    for op in setup.operations.iter().filter(|o| !o.suppressed) {
        let start = program.as_ref().map(|all| all.moves.iter().fold(all.start, |at, m| m.end(at)));
        let worked = work(s, CMD, &d, setup, op, start, &mut placed)?;
        notes.extend(worked.notes.into_iter().map(|n| format!("operation {}: {n}", op.id)));
        let toolpath = &worked.toolpath;
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
    let program = program.ok_or_else(|| bad(CMD, format!("setup '{}' has no operations to post", setup.id)))?;
    let ctx = ProgramContext {
        program_number: p.program_number.unwrap_or(1),
        program_name: p.program_name.unwrap_or_else(|| setup.name.clone()),
        units,
        work_offset: setup.work_offset,
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
