//! A setup placed on the model: its stock box, work coordinate system and safe heights as the
//! numbers an operation works from. The engine measures the model's exact box (never a display
//! mesh) and this works out the rest, so it can be tested with hand-derived boxes.
//!
//! Placed now: the model's own axes (or explicit axes equal to them), relative and fixed stock
//! boxes, the origin at a point of the stock box or at an explicit point. The rest is refused by
//! name until it lands: a turned work coordinate system needs an exact box along other axes, a
//! model point needs the engine to find it, and cylinder, body and earlier-setup stock need their
//! own measures.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::manufacture::{BoxAnchor, BoxSide, Setup, Stock, WcsOrientation, WcsOrigin};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// How far a fixed stock box may fall short of the model and still hold it, in mm.
const HOLD_SLOP: f64 = 1e-9;
/// How close explicit axes must be to the model's to count as them.
const AXIS_SLOP: f64 = 1e-12;

/// An axis-aligned box, least corner then greatest, mm.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bounds {
    pub min: Vec3,
    pub max: Vec3,
}

impl Bounds {
    /// Checks the corners: finite, within [`MAX_COORDINATE`], in order.
    pub fn check(&self, what: &str) -> Result<()> {
        let c = [self.min.x, self.min.y, self.min.z, self.max.x, self.max.y, self.max.z];
        if !c.iter().all(|v| v.abs() <= MAX_COORDINATE) {
            return Err(Error::BadValue(format!("{what}: a box must be finite and within {MAX_COORDINATE} mm")));
        }
        if !(self.min.x <= self.max.x && self.min.y <= self.max.y && self.min.z <= self.max.z) {
            return Err(Error::BadValue(format!("{what}: a box's least corner must not pass its greatest")));
        }
        Ok(())
    }

    /// The box's size along each axis.
    pub fn size(&self) -> Vec3 {
        Vec3 { x: self.max.x - self.min.x, y: self.max.y - self.min.y, z: self.max.z - self.min.z }
    }

    /// The point of the box at `anchor`: a corner, an edge or face centre, or the centre.
    pub fn anchor(&self, anchor: BoxAnchor) -> Vec3 {
        Vec3 { x: side(self.min.x, self.max.x, anchor.x), y: side(self.min.y, self.max.y, anchor.y), z: side(self.min.z, self.max.z, anchor.z) }
    }

    fn moved(&self, by: Vec3) -> Bounds {
        Bounds { min: add(self.min, by), max: add(self.max, by) }
    }
}

fn side(lo: f64, hi: f64, side: BoxSide) -> f64 {
    match side {
        BoxSide::Min => lo,
        BoxSide::Center => 0.5 * (lo + hi),
        BoxSide::Max => hi,
    }
}

fn add(a: Vec3, b: Vec3) -> Vec3 {
    Vec3 { x: a.x + b.x, y: a.y + b.y, z: a.z + b.z }
}

fn neg(a: Vec3) -> Vec3 {
    Vec3 { x: -a.x, y: -a.y, z: -a.z }
}

const X: Vec3 = Vec3 { x: 1.0, y: 0.0, z: 0.0 };
const Y: Vec3 = Vec3 { x: 0.0, y: 1.0, z: 0.0 };
const Z: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

/// A setup placed on the model. Boxes and heights are in the WCS; the origin and axes say where
/// the WCS is in model coordinates.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Placement {
    /// The WCS origin, in model coordinates.
    pub origin: Vec3,
    /// The WCS X, Y and Z directions, in model coordinates.
    pub x_axis: Vec3,
    pub y_axis: Vec3,
    pub z_axis: Vec3,
    /// The box of the bodies to machine, in the WCS.
    pub model: Bounds,
    /// The stock box, in the WCS.
    pub stock: Bounds,
    /// The clearance height, as Z in the WCS: the stock top plus the setup's clearance.
    pub clearance_z: f64,
    /// The retract height, as Z in the WCS: the stock top plus the setup's retract.
    pub retract_z: f64,
    /// The top spindle speed of the machine the setup is on, rev/min (decision 0036).
    #[serde(default = "generic_max_rpm")]
    pub max_rpm: f64,
}

fn generic_max_rpm() -> f64 {
    crate::feeds::GENERIC_MAX_RPM
}

/// Whether `v` points along `axis` (it is checked to be finite and non-zero when the data is read).
fn along(v: Vec3, axis: Vec3) -> bool {
    let len = (v.x * v.x + v.y * v.y + v.z * v.z).sqrt();
    len > 0.0 && (v.x / len - axis.x).abs() <= AXIS_SLOP && (v.y / len - axis.y).abs() <= AXIS_SLOP && (v.z / len - axis.z).abs() <= AXIS_SLOP
}

impl Setup {
    /// Places this setup on a model whose bodies to machine fill `model` (model coordinates).
    pub fn place(&self, model: Bounds) -> Result<Placement> {
        let what = format!("setup {}", self.id);
        let not_yet = |m: &str| Err(Error::BadValue(format!("{what}: {m} is not supported yet")));
        model.check(&format!("{what}: the model's box"))?;
        match &self.wcs.orientation {
            WcsOrientation::Model => {}
            WcsOrientation::Explicit { z, x } if along(*z, Z) && along(*x, X) => {}
            WcsOrientation::Explicit { .. } | WcsOrientation::FromFace { .. } => {
                return not_yet("a work coordinate system turned from the model's axes");
            }
        }
        let stock = match self.stock {
            Stock::RelativeBox { offsets: o } => Bounds {
                min: Vec3 { x: model.min.x - o.x_min, y: model.min.y - o.y_min, z: model.min.z - o.z_min },
                max: Vec3 { x: model.max.x + o.x_max, y: model.max.y + o.y_max, z: model.max.z + o.z_max },
            },
            Stock::FixedBox { size, anchor } => {
                // The model's box and the stock's share the point at `anchor`.
                let at = model.anchor(anchor);
                let of_stock = Bounds { min: Vec3::default(), max: size }.anchor(anchor);
                let min = Vec3 { x: at.x - of_stock.x, y: at.y - of_stock.y, z: at.z - of_stock.z };
                let stock = Bounds { min, max: add(min, size) };
                let holds = |lo: f64, hi: f64, m_lo: f64, m_hi: f64| lo <= m_lo + HOLD_SLOP && hi >= m_hi - HOLD_SLOP;
                for (axis, s, m) in [
                    ("X", (stock.min.x, stock.max.x), (model.min.x, model.max.x)),
                    ("Y", (stock.min.y, stock.max.y), (model.min.y, model.max.y)),
                    ("Z", (stock.min.z, stock.max.z), (model.min.z, model.max.z)),
                ] {
                    if !holds(s.0, s.1, m.0, m.1) {
                        return Err(Error::BadValue(format!(
                            "{what}: the fixed stock box does not hold the model along {axis} (stock {} mm, model {} mm, placed at its {})",
                            s.1 - s.0,
                            m.1 - m.0,
                            anchor_name(anchor)
                        )));
                    }
                }
                stock
            }
            Stock::RelativeCylinder { .. } => return not_yet("cylinder stock"),
            Stock::FromBody { .. } => return not_yet("stock from a body"),
            Stock::FromSetup { .. } => return not_yet("stock from an earlier setup"),
        };
        let origin = match &self.wcs.origin {
            WcsOrigin::StockBox { anchor } => stock.anchor(*anchor),
            WcsOrigin::Explicit { point } => *point,
            WcsOrigin::ModelPoint { .. } => return not_yet("a WCS origin at a model point"),
        };
        let stock = stock.moved(neg(origin));
        let model = model.moved(neg(origin));
        let h = self.safe_heights;
        let Some(machine) = crate::machine::machine(self.machine.as_deref()) else {
            let id = self.machine.as_deref().unwrap_or_default();
            let known: Vec<&str> = crate::machine::BUILTIN_MACHINES.iter().map(|m| m.id).collect();
            return Err(Error::BadValue(format!("{what} is on machine '{id}', which this version does not know: one of {}", known.join(", "))));
        };
        let placed = Placement {
            origin,
            x_axis: X,
            y_axis: Y,
            z_axis: Z,
            model,
            stock,
            clearance_z: stock.max.z + h.clearance,
            retract_z: stock.max.z + h.retract,
            max_rpm: machine.max_rpm,
        };
        placed.stock.check(&format!("{what}: the stock"))?;
        placed.model.check(&format!("{what}: the model"))?;
        if !(placed.clearance_z.abs() <= MAX_COORDINATE) {
            return Err(Error::BadValue(format!("{what}: the clearance height is beyond {MAX_COORDINATE} mm")));
        }
        Ok(placed)
    }
}

fn anchor_name(a: BoxAnchor) -> String {
    let name = |s: BoxSide| match s {
        BoxSide::Min => "min",
        BoxSide::Center => "center",
        BoxSide::Max => "max",
    };
    format!("{} {} {}", name(a.x), name(a.y), name(a.z))
}

#[cfg(test)]
#[path = "placement_tests.rs"]
mod tests;
