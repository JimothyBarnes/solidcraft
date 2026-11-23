//! What the Manufacture workspace saves with a design (decision 0006): the design's copies of
//! tools, and its setups (stock, work coordinate system, work offset, safe heights, the bodies to
//! machine and to avoid) with their operations. Saved as `plugins/solidcraft-cam/data.json`.
//!
//! Toolpaths are never saved here; they are derived from the model and these parameters. Model
//! geometry is referred to by the design's own stable names, kept here as opaque strings: this
//! crate knows nothing of documents, and the engine resolves the names.
//!
//! Read as hostile input: every count is capped, every number checked, every reference resolved
//! within the data (an operation's tool exists, a setup that takes its stock from another names
//! one before it).

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::chamfer::ChamferOp;
use crate::contour::ContourOp;
use crate::drill::DrillOp;
use crate::face::FaceOp;
use crate::pocket::PocketOp;
use crate::slot::SlotOp;
use crate::tool::Tool;
use crate::toolpath::Toolpath;
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The format of `data.json` this build reads and writes (our own number, decision 0006).
pub const MANUFACTURE_FORMAT: u32 = 1;
/// The plugin data folder Manufacture keeps its files in (`plugins/solidcraft-cam/`). Our own id, not
/// upstream's `manufacture` (its RFC 0015 example of a third-party plugin, which a future official
/// plugin might take): two writers of one folder in different formats would corrupt each other.
pub const MANUFACTURE_DATA_OWNER: &str = "solidcraft-cam";
pub const MAX_SETUPS: usize = 1000;
pub const MAX_OPERATIONS: usize = 10_000;
pub const MAX_DESIGN_TOOLS: usize = 1000;
/// The most references (bodies, faces) one setup may list.
pub const MAX_REFERENCES: usize = 1000;
const MAX_NAME: usize = 200;
const MAX_REFERENCE: usize = 512;
/// The largest stock offset accepted, in mm.
const MAX_STOCK_OFFSET: f64 = 10_000.0;

/// Where on the stock box a point sits along one axis.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoxSide {
    Min,
    Center,
    Max,
}

/// A point of the stock box: one of its corners, edge or face centres, or its centre.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxAnchor {
    pub x: BoxSide,
    pub y: BoxSide,
    pub z: BoxSide,
}

/// Where the work coordinate system's origin is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WcsOrigin {
    /// A point of the stock box (the shop default: top of stock, at a corner or the centre).
    StockBox { anchor: BoxAnchor },
    /// A vertex or point of the model, by its stable name.
    ModelPoint { reference: String },
    /// A point in model coordinates, mm.
    Explicit { point: Vec3 },
}

/// How the work coordinate system is turned.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum WcsOrientation {
    /// The model's own axes.
    Model,
    /// Z along a face's normal (out of the material), X along an edge when given.
    FromFace { face: String, x_edge: Option<String> },
    /// Z and X directions in model coordinates; normalised when read.
    Explicit { z: Vec3, x: Vec3 },
}

/// The work coordinate system.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Wcs {
    pub origin: WcsOrigin,
    pub orientation: WcsOrientation,
}

/// How much stock is added around the model's box, per side, mm.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxOffsets {
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub z_min: f64,
    pub z_max: f64,
}

/// The material before machining.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Stock {
    /// The model's box in the WCS axes, grown by offsets (the familiar default).
    RelativeBox { offsets: BoxOffsets },
    /// A box of a fixed size, placed with the model's box at `anchor` of the stock's.
    FixedBox { size: Vec3, anchor: BoxAnchor },
    /// A cylinder around the WCS Z axis, grown by offsets.
    RelativeCylinder { radial: f64, z_min: f64, z_max: f64 },
    /// A body of the design, by its stable name.
    FromBody { body: String },
    /// What an earlier setup left (its simulated result), by that setup's id.
    FromSetup { setup: String },
}

/// Clearance and retract heights above the stock top, mm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SafeHeights {
    pub clearance: f64,
    pub retract: f64,
}

/// What an operation does: a hand-made toolpath, or a strategy whose toolpath is worked out from
/// its parameters, the setup's placement and the tool (P1 adds them one by one).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum OperationKind {
    /// A toolpath given move by move (tests, the P0 demo, scripts). Its moves are its parameters.
    Manual { toolpath: Toolpath },
    /// Holes at points, drilled with one cycle (decision 0009 of the CAM work).
    Drill { drill: DrillOp },
    /// The stock's top taken down to the part's top in straight passes (decision 0010).
    Face { face: FaceOp },
    /// Round the outside of the part, its outline taken from the model (decision 0013).
    Contour { contour: ContourOp },
    /// Closed pockets cleared in rings, each named by a point on its floor (decision 0014).
    Pocket { pocket: PocketOp },
    /// The top outside edge of the part broken with a chamfer mill (decision 0018).
    Chamfer { chamfer: ChamferOp },
    /// Slots milled along their middle, each named by a point in it (decision 0026).
    Slot { slot: SlotOp },
}

/// One operation of a setup.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Operation {
    /// Stable within the design (`op-1`, …): commands and the toolpath cache refer to it.
    pub id: String,
    pub name: String,
    /// The tool number, one of the design's tools.
    pub tool: u32,
    pub suppressed: bool,
    pub operation: OperationKind,
}

/// The kind of machining a setup is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupKind {
    Milling,
}

/// One setup: one way the part is held on the machine, and what is done to it there.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub id: String,
    pub name: String,
    pub kind: SetupKind,
    /// The machine definition's id; `None` for a generic machine.
    pub machine: Option<String>,
    /// The stock's material, as named in the tools' cutting data (`aluminium`); `None` when not set.
    #[serde(default)]
    pub material: Option<String>,
    pub wcs: Wcs,
    /// 1 to 6 for G54 to G59.
    pub work_offset: u8,
    pub stock: Stock,
    /// The bodies to machine (empty: every visible body), by stable name.
    pub model_bodies: Vec<String>,
    /// Bodies to keep clear of (vise, clamps), by stable name.
    pub fixture_bodies: Vec<String>,
    pub safe_heights: SafeHeights,
    pub operations: Vec<Operation>,
}

/// Everything the Manufacture workspace keeps in a design.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManufactureData {
    pub format: u32,
    /// The design's copies of tools (a library may differ: the UI offers to update them).
    pub tools: Vec<Tool>,
    pub setups: Vec<Setup>,
}

impl Default for ManufactureData {
    fn default() -> Self {
        Self { format: MANUFACTURE_FORMAT, tools: Vec::new(), setups: Vec::new() }
    }
}

fn check_name(what: &str, name: &str) -> Result<()> {
    if name.chars().count() > MAX_NAME {
        return Err(Error::BadValue(format!("{what}: a name of at most {MAX_NAME} characters")));
    }
    Ok(())
}

fn check_reference(what: &str, r: &str) -> Result<()> {
    if r.is_empty() || r.len() > MAX_REFERENCE || r.chars().any(char::is_control) {
        return Err(Error::BadValue(format!("{what}: a reference of 1 to {MAX_REFERENCE} bytes without control characters")));
    }
    Ok(())
}

fn check_id(what: &str, id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(Error::BadValue(format!("{what}: an id of 1 to 64 letters, digits, '-' and '_'")));
    }
    Ok(())
}

fn finite_point(p: Vec3) -> bool {
    [p.x, p.y, p.z].iter().all(|c| c.abs() <= MAX_COORDINATE)
}

fn offset_ok(v: f64) -> bool {
    (0.0..=MAX_STOCK_OFFSET).contains(&v)
}

impl ManufactureData {
    /// Reads `data.json` and checks it all.
    pub fn from_json(json: &str) -> Result<ManufactureData> {
        let data: ManufactureData = serde_json::from_str(json).map_err(|e| Error::BadValue(format!("manufacture data: {e}")))?;
        data.validate()?;
        Ok(data)
    }

    /// The data as `data.json` (pretty, so a design in git diffs readably).
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| Error::BadValue(format!("manufacture data: {e}")))
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != MANUFACTURE_FORMAT {
            return Err(Error::BadValue(format!("manufacture data format {} (this build reads {MANUFACTURE_FORMAT})", self.format)));
        }
        if self.tools.len() > MAX_DESIGN_TOOLS || self.setups.len() > MAX_SETUPS {
            return Err(Error::BadValue(format!("at most {MAX_DESIGN_TOOLS} tools and {MAX_SETUPS} setups")));
        }
        for (i, t) in self.tools.iter().enumerate() {
            t.validate()?;
            if self.tools.iter().take(i).any(|u| u.number == t.number) {
                return Err(Error::BadValue(format!("tool number {} is used twice in the design", t.number)));
            }
        }
        let operations: usize = self.setups.iter().map(|s| s.operations.len()).sum();
        if operations > MAX_OPERATIONS {
            return Err(Error::BadValue(format!("at most {MAX_OPERATIONS} operations")));
        }
        for (i, s) in self.setups.iter().enumerate() {
            self.check_setup(i, s)?;
        }
        Ok(())
    }

    fn check_setup(&self, index: usize, s: &Setup) -> Result<()> {
        let what = format!("setup {}", s.id);
        check_id(&what, &s.id)?;
        check_name(&what, &s.name)?;
        if self.setups.iter().take(index).any(|t| t.id == s.id) {
            return Err(Error::BadValue(format!("{what}: the id is used twice")));
        }
        if let Some(m) = &s.machine {
            check_id(&format!("{what} machine"), m)?;
        }
        if let Some(m) = &s.material
            && (m.is_empty() || m.len() > 64 || m.chars().any(char::is_control))
        {
            return Err(Error::BadValue(format!("{what}: a material name of 1 to 64 bytes without control characters")));
        }
        match &s.wcs.origin {
            WcsOrigin::StockBox { .. } => {}
            WcsOrigin::ModelPoint { reference } => check_reference(&what, reference)?,
            WcsOrigin::Explicit { point } if finite_point(*point) => {}
            WcsOrigin::Explicit { .. } => return Err(Error::BadValue(format!("{what}: the WCS origin must be finite and within range"))),
        }
        match &s.wcs.orientation {
            WcsOrientation::Model => {}
            WcsOrientation::FromFace { face, x_edge } => {
                check_reference(&what, face)?;
                if let Some(e) = x_edge {
                    check_reference(&what, e)?;
                }
            }
            WcsOrientation::Explicit { z, x } => {
                let len = |v: &Vec3| (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
                let dot = (z.x * x.x + z.y * x.y + z.z * x.z) / (len(z) * len(x));
                if !(len(z) > 1e-9 && len(x) > 1e-9 && finite_point(*z) && finite_point(*x) && dot.abs() < 1.0 - 1e-6) {
                    return Err(Error::BadValue(format!("{what}: WCS Z and X must be finite, non-zero and not parallel")));
                }
            }
        }
        if !(1..=6).contains(&s.work_offset) {
            return Err(Error::BadValue(format!("{what}: work offset 1 to 6 (G54 to G59)")));
        }
        match &s.stock {
            Stock::RelativeBox { offsets: o } if [o.x_min, o.x_max, o.y_min, o.y_max, o.z_min, o.z_max].into_iter().all(offset_ok) => {}
            Stock::FixedBox { size, .. } if [size.x, size.y, size.z].iter().all(|&v| v > 0.0 && v <= MAX_COORDINATE) => {}
            Stock::RelativeCylinder { radial, z_min, z_max } if [*radial, *z_min, *z_max].into_iter().all(offset_ok) => {}
            Stock::FromBody { body } => check_reference(&what, body)?,
            Stock::FromSetup { setup } if self.setups.iter().take(index).any(|t| &t.id == setup) => {}
            Stock::FromSetup { setup } => return Err(Error::BadValue(format!("{what}: stock from setup '{setup}', which is not an earlier setup"))),
            _ => return Err(Error::BadValue(format!("{what}: stock sizes and offsets must be finite, 0 to {MAX_STOCK_OFFSET} mm"))),
        }
        if s.model_bodies.len() > MAX_REFERENCES || s.fixture_bodies.len() > MAX_REFERENCES {
            return Err(Error::BadValue(format!("{what}: at most {MAX_REFERENCES} model and fixture bodies")));
        }
        for r in s.model_bodies.iter().chain(&s.fixture_bodies) {
            check_reference(&what, r)?;
        }
        let h = s.safe_heights;
        if !(offset_ok(h.clearance) && offset_ok(h.retract) && h.clearance >= h.retract) {
            return Err(Error::BadValue(format!("{what}: safe heights 0 to {MAX_STOCK_OFFSET} mm, clearance at or above retract")));
        }
        for op in &s.operations {
            let what = format!("{what} operation {}", op.id);
            check_id(&what, &op.id)?;
            check_name(&what, &op.name)?;
            if self.setups.iter().flat_map(|t| &t.operations).filter(|o| o.id == op.id).count() > 1 {
                return Err(Error::BadValue(format!("{what}: the id is used twice")));
            }
            let Some(tool) = self.tools.iter().find(|t| t.number == op.tool) else {
                return Err(Error::BadValue(format!("{what}: tool {} is not one of the design's tools", op.tool)));
            };
            match &op.operation {
                OperationKind::Manual { toolpath } => toolpath.validate().map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Drill { drill } => drill.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Face { face } => face.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Contour { contour } => contour.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Pocket { pocket } => pocket.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Chamfer { chamfer } => chamfer.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
                OperationKind::Slot { slot } => slot.validate(tool).map_err(|e| Error::BadValue(format!("{what}: {e}")))?,
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "manufacture_tests.rs"]
mod tests;
