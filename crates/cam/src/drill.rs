//! The drill operation (decision 0009): holes at points of the setup's WCS, cut with one drilling
//! cycle (decision 0008). Its toolpath is worked out from the setup's placement, its safe heights
//! and the tool, never stored.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::cycle::DrillCycle;
use crate::feeds::op_feeds;
use crate::manufacture::SafeHeights;
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most holes one drill operation may have.
pub const MAX_HOLES: usize = 10_000;
/// The highest R plane above the holes' top accepted, mm.
const MAX_R_ABOVE: f64 = 1_000.0;

/// The order holes are drilled in.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HoleOrder {
    /// From the first hole listed, each time to the nearest one not yet drilled.
    #[default]
    Nearest,
    /// As listed.
    AsGiven,
}

/// What a hole's depth is measured to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DepthTo {
    /// The tool's tip.
    #[default]
    Tip,
    /// Where a pointed tool reaches its full diameter: the point's length deeper.
    FullDiameter,
}

/// Where the tool goes between holes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Between {
    /// Back to the R plane (G99).
    #[default]
    RPlane,
    /// Up to the clearance height, over clamps (G98).
    Clearance,
}

/// A drill operation's parameters, as the design keeps them.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DrillOp {
    /// Hole centres, X and Y in the setup's WCS, mm.
    pub holes: Vec<[f64; 2]>,
    #[serde(default)]
    pub order: HoleOrder,
    /// The Z of the holes' top in the WCS; `None` for the stock's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// How far below the top the hole goes, mm.
    pub depth: f64,
    #[serde(default)]
    pub depth_to: DepthTo,
    /// The R plane's height above the top, mm; `None` for the setup's retract height.
    #[serde(default)]
    pub r_above: Option<f64>,
    #[serde(default)]
    pub between: Between,
    pub cycle: DrillCycle,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A drill operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Drilled {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    /// The Z of the holes' bottom, the R plane and the clearance height in the WCS, mm.
    pub bottom: f64,
    pub r: f64,
    pub clearance: f64,
    /// Things worth showing beside the toolpath (a capped spindle speed, a thin chip).
    pub notes: Vec<String>,
}

fn finite(v: f64) -> bool {
    v.abs() <= MAX_COORDINATE
}

impl DrillOp {
    /// Checks the parameters and that `tool` can drill them.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        let bad = |m: String| Err(Error::BadValue(m));
        if self.holes.is_empty() || self.holes.len() > MAX_HOLES {
            return bad(format!("a drill operation has 1 to {MAX_HOLES} holes, not {}", self.holes.len()));
        }
        if !self.holes.iter().all(|h| h.iter().all(|&v| finite(v))) {
            return bad(format!("hole positions must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.top.is_none_or(finite) {
            return bad(format!("the holes' top must be finite and within {MAX_COORDINATE} mm"));
        }
        if !(self.depth > 0.0 && self.depth <= MAX_COORDINATE) {
            return bad(format!("the depth must be above 0 and at most {MAX_COORDINATE} mm, not {}", self.depth));
        }
        if !self.r_above.is_none_or(|r| r > 0.0 && r <= MAX_R_ABOVE) {
            return bad(format!("the R plane must be above the holes' top, at most {MAX_R_ABOVE} mm"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        match tool.kind {
            ToolKind::Drill | ToolKind::SpotDrill | ToolKind::Reamer | ToolKind::FlatEndMill => Ok(()),
            ToolKind::Tap => bad(format!("tool {} is a tap: tapping (G84) is not supported yet", tool.number)),
            other => bad(format!("tool {} ({other:?}) does not drill: use a drill, spot drill, reamer or flat end mill", tool.number)),
        }
    }

    /// The holes in the order they are drilled.
    fn ordered(&self) -> Vec<[f64; 2]> {
        match self.order {
            HoleOrder::AsGiven => self.holes.clone(),
            HoleOrder::Nearest => {
                let mut left = self.holes.clone();
                let mut out = Vec::with_capacity(left.len());
                let mut at = match left.first() {
                    Some(&h) => h,
                    None => return out,
                };
                while !left.is_empty() {
                    let d = |h: &[f64; 2]| (h[0] - at[0]).powi(2) + (h[1] - at[1]).powi(2);
                    // The first of equally near holes, so the order never depends on rounding ties.
                    let mut best = 0;
                    for (k, h) in left.iter().enumerate() {
                        if left.get(best).is_some_and(|b| d(h) < d(b)) {
                            best = k;
                        }
                    }
                    at = left.remove(best);
                    out.push(at);
                }
                out
            }
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above the first hole at the
    /// clearance height) for `tool` in a setup placed at `placement`, with its safe heights and
    /// stock `material`.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, safe: SafeHeights, material: Option<&str>) -> Result<Drilled> {
        self.validate(tool)?;
        let bad = |m: String| Err(Error::BadValue(m));
        let top = self.top.unwrap_or(placement.stock.max.z);
        let g = tool.geometry;
        let point = match self.depth_to {
            DepthTo::FullDiameter if g.tip_angle > 0.0 => 0.5 * g.diameter / (0.5 * g.tip_angle).tan(),
            DepthTo::Tip | DepthTo::FullDiameter => 0.0,
        };
        let deep = self.depth + point;
        if deep > g.flute_length + 1e-9 {
            return bad(format!("the holes go {deep} mm deep, past tool {}'s {} mm of flutes: chips would pack", tool.number, g.flute_length));
        }
        let bottom = top - deep;
        let r = top + self.r_above.unwrap_or(safe.retract);
        let clearance = placement.clearance_z;
        if r > clearance {
            return bad(format!("the R plane ({r} mm) is above the clearance height ({clearance} mm)"));
        }
        // A plunging end mill engages its whole diameter and feeds at its plunge feed; a drill's
        // radial engagement does not apply. A feed the operation gives is the feed down the hole.
        let radial = if tool.kind.is_axial() { 0.0 } else { g.diameter };
        let f = op_feeds(tool, Engagement { axial: deep.max(1e-3), radial }, material, self.rpm, self.feed)?;
        let (rpm, notes) = (f.rpm, f.notes);
        let feed = if self.feed.is_some() || tool.kind.is_axial() { f.feed } else { f.plunge_feed };
        let holes = self.ordered();
        let first = holes.first().copied().unwrap_or([0.0, 0.0]);
        let above = |h: [f64; 2]| Vec3 { x: h[0], y: h[1], z: clearance };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: above(first) });
        let retract = match self.between {
            Between::RPlane => r,
            Between::Clearance => clearance,
        };
        for h in &holes {
            moves.push(Move::Drill { to: Vec3 { x: h[0], y: h[1], z: bottom }, r, retract, cycle: self.cycle, feed });
        }
        if retract < clearance {
            moves.push(Move::Rapid { to: above(holes.last().copied().unwrap_or(first)) });
        }
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(above(first)), moves };
        toolpath.validate()?;
        Ok(Drilled { toolpath, rpm, feed, bottom, r, clearance, notes })
    }
}

#[cfg(test)]
#[path = "drill_tests.rs"]
mod tests;
