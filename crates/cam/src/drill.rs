//! The drill operation (decision 0009): holes at points of the setup's WCS, cut with one drilling
//! cycle (decision 0008). Its toolpath is worked out from the setup's placement and the tool,
//! never stored. Holes picked on the model, of any size, and points picked on it (decision 0040)
//! are found there by the engine, which gives each hole its own top and depth ([`HoleAt`]).
//!
//! Depths are the part's: holes are measured from the model's top unless the operation says, so
//! stock left on the part (or faced off it) never changes how deep a hole is. The R plane stays
//! above the stock, which may still be there.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::cycle::{DrillCycle, MAX_PITCH, MIN_PITCH};
use crate::feeds::op_feeds;
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
    /// Hole centres, X and Y in the setup's WCS, mm, all with one top and one depth.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holes: Vec<[f64; 2]>,
    /// Holes picked on the model, of any size, in place of `holes` (decision 0040 of the CAM
    /// work): a point on each one's wall or bottom, in model coordinates. The engine finds the
    /// holes each time the toolpath is worked out; each is drilled from its own top, to `depth`
    /// below it when one is given, else to its own bottom.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub picked: Vec<[f64; 3]>,
    /// Points picked on the model, in place of `holes` (decision 0040): each drilled at its X
    /// and Y from its own height, `depth` deep. In model coordinates.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub points: Vec<[f64; 3]>,
    #[serde(default)]
    pub order: HoleOrder,
    /// The Z of the holes' top in the WCS; `None` for the model's top (the part's highest face).
    #[serde(default)]
    pub top: Option<f64>,
    /// How far below the top the hole goes, mm. For holes picked, 0 (or none given) drills
    /// each to its own bottom.
    #[serde(default)]
    pub depth: f64,
    #[serde(default)]
    pub depth_to: DepthTo,
    /// The R plane's height above the holes' top, mm, for when the stock above them is gone;
    /// `None` for the setup's retract height above the stock's top.
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

/// One hole as it is drilled: its centre, X and Y in the setup's WCS, the Z of its top, and how
/// deep below that it goes, measured to what `depth_to` says, mm.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoleAt {
    pub at: [f64; 2],
    pub top: f64,
    pub depth: f64,
    pub depth_to: DepthTo,
}

/// A drill operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Drilled {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    /// The Z of the holes' bottom, the R plane and the clearance height in the WCS, mm: of
    /// holes at several heights, the lowest bottom and the highest R plane.
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
        let given = [self.holes.len(), self.picked.len(), self.points.len()];
        let all: usize = given.iter().sum();
        if all == 0 || given.iter().any(|&n| n > MAX_HOLES) {
            return bad(format!("a drill operation has 1 to {MAX_HOLES} holes, not {all}"));
        }
        if given.iter().filter(|&&n| n > 0).count() > 1 {
            return bad(
                "a drill operation drills holes at points of the WCS (holes), holes picked on the model (picked) or points picked on it (points): one of them, not several".into(),
            );
        }
        if !self.holes.iter().all(|h| h.iter().all(|&v| finite(v))) {
            return bad(format!("hole positions must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.picked.iter().chain(&self.points).all(|p| p.iter().all(|&v| finite(v))) {
            return bad(format!("points picked must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.top.is_none_or(finite) {
            return bad(format!("the holes' top must be finite and within {MAX_COORDINATE} mm"));
        }
        if self.top.is_some() && self.holes.is_empty() {
            return bad("holes and points picked on the model are drilled from their own tops: `top` is for `holes`".into());
        }
        // Holes picked go to their own bottoms unless a depth is given.
        let own = !self.picked.is_empty() && self.depth == 0.0;
        if !own && !(self.depth > 0.0 && self.depth <= MAX_COORDINATE) {
            return bad(format!("the depth must be above 0 and at most {MAX_COORDINATE} mm, not {}", self.depth));
        }
        self.numbers(tool)
    }

    /// Checks the parameters but the holes, and that `tool` can drill with them.
    fn numbers(&self, tool: &Tool) -> Result<()> {
        let bad = |m: String| Err(Error::BadValue(m));
        if !self.r_above.is_none_or(|r| r > 0.0 && r <= MAX_R_ABOVE) {
            return bad(format!("the R plane must be above the holes' top, at most {MAX_R_ABOVE} mm"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        // A tap cuts with the tap cycle at its own pitch, and nothing else does.
        let pitch = tool.geometry.thread_pitch;
        match (tool.kind, self.cycle) {
            (ToolKind::Tap, DrillCycle::Tap { pitch: asked }) => {
                if !(MIN_PITCH..=MAX_PITCH).contains(&pitch) {
                    return bad(format!(
                        "tool {} is a tap with no thread pitch ({pitch} mm): give it one of {MIN_PITCH} to {MAX_PITCH} mm",
                        tool.number
                    ));
                }
                if !((asked - pitch).abs() <= 1e-9 * pitch) {
                    return bad(format!("the cycle's pitch ({asked} mm) is not tool {}'s ({pitch} mm)", tool.number));
                }
                if self.feed.is_some() {
                    return bad("a tap's feed is its pitch times its spindle speed: give the speed (rpm), not a feed".into());
                }
                Ok(())
            }
            (ToolKind::Tap, _) => bad(format!("tool {} is a tap: it cuts with the tap cycle, at its pitch of {pitch} mm", tool.number)),
            (other, DrillCycle::Tap { .. }) => bad(format!("the tap cycle needs a tap, and tool {} is a {other:?}", tool.number)),
            (ToolKind::Drill | ToolKind::SpotDrill | ToolKind::Reamer | ToolKind::FlatEndMill, _) => Ok(()),
            (other, _) => bad(format!("tool {} ({other:?}) does not drill: use a drill, spot drill, reamer, tap or flat end mill", tool.number)),
        }
    }

    /// `holes` in the order they are drilled.
    fn ordered(&self, holes: &[HoleAt]) -> Vec<HoleAt> {
        match self.order {
            HoleOrder::AsGiven => holes.to_vec(),
            HoleOrder::Nearest => {
                let mut left = holes.to_vec();
                let mut out = Vec::with_capacity(left.len());
                let mut at = match left.first() {
                    Some(h) => h.at,
                    None => return out,
                };
                while !left.is_empty() {
                    let d = |h: &HoleAt| (h.at[0] - at[0]).powi(2) + (h.at[1] - at[1]).powi(2);
                    // The first of equally near holes, so the order never depends on rounding ties.
                    let mut best = 0;
                    for (k, h) in left.iter().enumerate() {
                        if left.get(best).is_some_and(|b| d(h) < d(b)) {
                            best = k;
                        }
                    }
                    let next = left.remove(best);
                    at = next.at;
                    out.push(next);
                }
                out
            }
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above the first hole at the
    /// clearance height) for `tool` in a setup placed at `placement`, with its stock `material`:
    /// of `holes`, all from `top` (the model's top unless given) `depth` deep. Holes and points
    /// picked on the model are found by the engine and drilled with [`DrillOp::toolpath_of`].
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>) -> Result<Drilled> {
        self.validate(tool)?;
        if self.holes.is_empty() {
            return Err(Error::BadValue("holes and points picked on the model are found there when the toolpath is worked out".into()));
        }
        let top = self.top.unwrap_or(placement.model.max.z);
        let holes: Vec<HoleAt> = self.holes.iter().map(|&at| HoleAt { at, top, depth: self.depth, depth_to: self.depth_to }).collect();
        self.toolpath_of(&holes, start, tool, placement, material)
    }

    /// [`DrillOp::toolpath`] of `holes`, each from its own top to its own depth, in place of the
    /// operation's own holes, top and depth (decision 0040 of the CAM work). Each hole's R plane
    /// is its own (its top and `r_above`) or the setup's retract height; between holes the tool
    /// goes up to the highest R plane, or to the clearance height when the operation asks.
    pub fn toolpath_of(&self, holes: &[HoleAt], start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>) -> Result<Drilled> {
        self.numbers(tool)?;
        let bad = |m: String| Err(Error::BadValue(m));
        if holes.is_empty() || holes.len() > MAX_HOLES {
            return bad(format!("a drill operation has 1 to {MAX_HOLES} holes, not {}", holes.len()));
        }
        if !holes.iter().all(|h| h.at.iter().all(|&v| finite(v)) && finite(h.top)) {
            return bad(format!("hole positions must be finite and within {MAX_COORDINATE} mm"));
        }
        if let Some(h) = holes.iter().find(|h| !(h.depth > 0.0 && h.depth <= MAX_COORDINATE)) {
            return bad(format!("the depth must be above 0 and at most {MAX_COORDINATE} mm, not {}", h.depth));
        }
        let g = tool.geometry;
        let point = |to: DepthTo| match to {
            DepthTo::FullDiameter if g.tip_angle > 0.0 => 0.5 * g.diameter / (0.5 * g.tip_angle).tan(),
            DepthTo::Tip | DepthTo::FullDiameter => 0.0,
        };
        let deep = |h: &HoleAt| h.depth + point(h.depth_to);
        let r_of = |h: &HoleAt| self.r_above.map_or(placement.retract_z, |above| h.top + above);
        let Some(deepest) = holes.iter().max_by(|a, b| deep(a).total_cmp(&deep(b))).copied() else {
            return bad("a drill operation has holes to drill".into());
        };
        if deep(&deepest) > g.flute_length + 1e-9 {
            let which = if holes.iter().all(|h| deep(h) == deep(&deepest)) {
                "the holes go".to_string()
            } else {
                format!("the deepest hole (X {:.3} Y {:.3}) goes", deepest.at[0], deepest.at[1])
            };
            return bad(format!("{which} {} mm deep, past tool {}'s {} mm of flutes: chips would pack", deep(&deepest), tool.number, g.flute_length));
        }
        let clearance = placement.clearance_z;
        let highest = holes.iter().map(r_of).fold(f64::NEG_INFINITY, f64::max);
        if highest > clearance {
            return bad(format!("the R plane ({highest} mm) is above the clearance height ({clearance} mm)"));
        }
        let bottom = holes.iter().map(|h| h.top - deep(h)).fold(f64::INFINITY, f64::min);
        // A plunging end mill engages its whole diameter and feeds at its plunge feed; a drill's
        // radial engagement does not apply. A feed the operation gives is the feed down the hole.
        let radial = if tool.kind.is_axial() { 0.0 } else { g.diameter };
        let f = op_feeds(tool, Engagement { axial: deep(&deepest).max(1e-3), radial }, material, placement.max_rpm, self.rpm, self.feed)?;
        let (rpm, notes) = (f.rpm, f.notes);
        let feed = if self.feed.is_some() || tool.kind.is_axial() { f.feed } else { f.plunge_feed };
        // A tap turns a whole number of times a minute, so that the speed and the feed a program
        // gives say its pitch exactly, and feeds one pitch a turn.
        let (rpm, feed) = match self.cycle {
            DrillCycle::Tap { pitch } => (rpm.round().max(1.0), pitch * rpm.round().max(1.0)),
            _ => (rpm, feed),
        };
        let holes = self.ordered(holes);
        let first = holes.first().copied().unwrap_or(deepest);
        let above = |h: [f64; 2], z: f64| Vec3 { x: h[0], y: h[1], z };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: above(first.at, clearance) });
        let retract = match self.between {
            Between::RPlane => highest,
            Between::Clearance => clearance,
        };
        // A hole whose R plane is below the height the tool goes across at starts from there, so
        // that each hole returns to its R plane or to where it started, as a canned cycle can.
        if retract < clearance && r_of(&first) < retract {
            moves.push(Move::Rapid { to: above(first.at, retract) });
        }
        for h in &holes {
            moves.push(Move::Drill { to: above(h.at, h.top - deep(h)), r: r_of(h), retract, cycle: self.cycle, feed });
        }
        if retract < clearance {
            moves.push(Move::Rapid { to: above(holes.last().map_or(first.at, |h| h.at), clearance) });
        }
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(above(first.at, clearance)), moves };
        toolpath.validate()?;
        Ok(Drilled { toolpath, rpm, feed, bottom, r: highest, clearance, notes })
    }
}

#[cfg(test)]
#[path = "drill_tests.rs"]
mod tests;
