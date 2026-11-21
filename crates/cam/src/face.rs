//! The facing operation (decision 0010): the stock's top taken down to the part's top in straight
//! passes along X, each running in from beyond the stock and out past it. Its toolpath is worked
//! out from the setup's placement and the tool, never stored.
//!
//! Passes start at the stock's +Y edge and move toward -Y. A one-way pass travels +X with the
//! uncut stock on its right, which is a climb cut for a spindle turning clockwise.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::feeds::op_feeds;
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most passes one level may take.
pub const MAX_FACE_PASSES: usize = 10_000;
/// The most levels one facing operation may take.
pub const MAX_FACE_LEVELS: usize = 1_000;
/// How far the tool's edge clears the stock at the ends of a pass unless the operation says, mm.
pub const DEFAULT_FACE_CLEAR: f64 = 2.0;
/// The stepover unless the operation says, as a share of the tool's diameter.
pub const DEFAULT_FACE_STEPOVER: f64 = 0.7;
/// The largest clearance past the stock accepted, mm.
const MAX_CLEAR: f64 = 1_000.0;
/// The smallest stepover or stepdown accepted, mm.
const MIN_STEP: f64 = 1e-3;
/// Heights closer than this are one height, mm.
const SAME_HEIGHT: f64 = 1e-6;

/// How the passes of one level follow each other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacePattern {
    /// Back and forth: each pass cuts the other way, stepping over beside the stock.
    #[default]
    ZigZag,
    /// Every pass the same way (+X), lifting to the retract height and going back between them.
    OneWay,
}

/// A facing operation's parameters, as the design keeps them. Every one has a default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FaceOp {
    /// The Z facing starts from in the WCS; `None` for the stock's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// The Z it cuts down to; `None` for the model's top.
    #[serde(default)]
    pub bottom: Option<f64>,
    /// The most one level takes off, mm; `None` for the whole depth in one level.
    #[serde(default)]
    pub stepdown: Option<f64>,
    /// The most between passes, mm, at most the tool's diameter; `None` for 70% of it.
    #[serde(default)]
    pub stepover: Option<f64>,
    /// How far the tool's edge clears the stock at each end of a pass, mm; `None` for 2.
    #[serde(default)]
    pub clear: Option<f64>,
    #[serde(default)]
    pub pattern: FacePattern,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A facing operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Faced {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// The Z of each level in the WCS, top down.
    pub levels: Vec<f64>,
    /// The Y of each pass, in the order cut.
    pub passes: Vec<f64>,
    /// The stepover used, mm.
    pub stepover: f64,
    /// Things worth showing beside the toolpath (a capped spindle speed, a thin chip).
    pub notes: Vec<String>,
}

impl FaceOp {
    /// Checks the parameters and that `tool` can face.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        let bad = |m: String| Err(Error::BadValue(m));
        let finite = |v: f64| v.abs() <= MAX_COORDINATE;
        if !self.top.is_none_or(finite) || !self.bottom.is_none_or(finite) {
            return bad(format!("the top and bottom must be finite and within {MAX_COORDINATE} mm"));
        }
        if let (Some(top), Some(bottom)) = (self.top, self.bottom)
            && !(top - bottom > SAME_HEIGHT)
        {
            return bad(format!("the bottom ({bottom} mm) must be below the top ({top} mm)"));
        }
        if !self.stepdown.is_none_or(|v| (MIN_STEP..=MAX_COORDINATE).contains(&v)) {
            return bad(format!("the stepdown must be at least {MIN_STEP} mm"));
        }
        let diameter = tool.geometry.diameter;
        if !self.stepover.is_none_or(|v| v >= MIN_STEP && v <= diameter + 1e-9) {
            return bad(format!("the stepover must be at least {MIN_STEP} mm and at most the tool's diameter ({diameter} mm)"));
        }
        if !self.clear.is_none_or(|v| (0.0..=MAX_CLEAR).contains(&v)) {
            return bad(format!("the clearance past the stock must be 0 to {MAX_CLEAR} mm"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        match tool.kind {
            ToolKind::FaceMill | ToolKind::FlatEndMill | ToolKind::BullNoseEndMill => Ok(()),
            other => bad(format!("tool {} ({other:?}) does not face: use a face mill, flat end mill or bull-nose end mill", tool.number)),
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for beside the stock at the clearance
    /// height) for `tool` in a setup placed at `placement`, with its stock `material`.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>) -> Result<Faced> {
        self.validate(tool)?;
        let bad = |m: String| Err(Error::BadValue(m));
        let stock = placement.stock;
        let top = self.top.unwrap_or(stock.max.z);
        let bottom = self.bottom.unwrap_or(placement.model.max.z);
        let depth = top - bottom;
        if !(depth > SAME_HEIGHT) {
            return bad(format!(
                "nothing to face: the bottom ({bottom} mm) is not below the top ({top} mm). Give the setup stock on top, or the operation its heights"
            ));
        }
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the facing's top ({top} mm)"));
        }
        // Levels of equal depth, none deeper than the stepdown.
        let count = (depth / self.stepdown.unwrap_or(depth) - 1e-9).ceil().max(1.0);
        if !(count <= MAX_FACE_LEVELS as f64) {
            return bad(format!("{depth} mm in steps of {} mm takes more than {MAX_FACE_LEVELS} levels", self.stepdown.unwrap_or(depth)));
        }
        let count = count as usize;
        let per_level = depth / count as f64;
        let g = tool.geometry;
        if per_level > g.flute_length + 1e-9 {
            return bad(format!(
                "each level is {per_level} mm deep, past tool {}'s {} mm of flutes: give the operation a stepdown",
                tool.number, g.flute_length
            ));
        }
        let levels: Vec<f64> = (1..=count).map(|k| if k == count { bottom } else { top - per_level * k as f64 }).collect();
        // Passes: the first and last overlap the stock's edges by the stepover, the rest evenly
        // between; a tool as wide as the stock's width allows cuts one pass down the middle.
        let radius = 0.5 * g.diameter;
        let stepover = self.stepover.unwrap_or(DEFAULT_FACE_STEPOVER * g.diameter);
        let (y_lo, y_hi) = (stock.min.y, stock.max.y);
        let passes: Vec<f64> = if y_hi - y_lo <= stepover + 1e-9 {
            vec![0.5 * (y_lo + y_hi)]
        } else {
            let first = y_hi + radius - stepover;
            let last = y_lo - radius + stepover;
            let gaps = ((first - last) / stepover - 1e-9).ceil().max(1.0);
            if !(gaps < MAX_FACE_PASSES as f64) {
                return bad(format!("a stepover of {stepover} mm across {} mm takes more than {MAX_FACE_PASSES} passes", y_hi - y_lo));
            }
            let gaps = gaps as usize;
            (0..=gaps).map(|k| if k == gaps { last } else { first - (first - last) * k as f64 / gaps as f64 }).collect()
        };
        let clear = self.clear.unwrap_or(DEFAULT_FACE_CLEAR);
        let (x_in, x_out) = (stock.min.x - radius - clear, stock.max.x + radius + clear);
        let first_y = passes.first().copied().unwrap_or(0.5 * (y_lo + y_hi));
        let f = op_feeds(tool, Engagement { axial: per_level.max(1e-4), radial: stepover }, material, self.rpm, self.feed)?;
        let at = |x: f64, y: f64, z: f64| Vec3 { x, y, z };

        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(x_in, first_y, clearance) });
        let (mut x, mut y) = (x_in, first_y);
        for &z in &levels {
            // Beside the stock at the retract height, then down to the level in air.
            moves.push(Move::Rapid { to: at(x_in, first_y, retract) });
            moves.push(Move::Linear { to: at(x_in, first_y, z), feed: f.plunge_feed });
            x = x_in;
            for (k, &pass) in passes.iter().enumerate() {
                if k > 0 {
                    match self.pattern {
                        // The tool's edge is `clear` from the stock here: the step is in air.
                        FacePattern::ZigZag => moves.push(Move::Linear { to: at(x, pass, z), feed: f.feed }),
                        FacePattern::OneWay => {
                            moves.push(Move::Rapid { to: at(x, y, retract) });
                            moves.push(Move::Rapid { to: at(x_in, pass, retract) });
                            moves.push(Move::Linear { to: at(x_in, pass, z), feed: f.plunge_feed });
                            x = x_in;
                        }
                    }
                }
                let end = if x == x_in { x_out } else { x_in };
                moves.push(Move::Linear { to: at(end, pass, z), feed: f.feed });
                (x, y) = (end, pass);
            }
            moves.push(Move::Rapid { to: at(x, y, retract) });
        }
        moves.push(Move::Rapid { to: at(x, y, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(x_in, first_y, clearance)), moves };
        toolpath.validate()?;
        Ok(Faced { toolpath, rpm: f.rpm, feed: f.feed, plunge_feed: f.plunge_feed, levels, passes, stepover, notes: f.notes })
    }
}

#[cfg(test)]
#[path = "face_tests.rs"]
mod tests;
