//! The contour operation (decision 0013 of the CAM work): round the outside of the part, the
//! tool's centre on the part's outline moved out by its radius. It reads the setup's placement
//! (the stock, to go down beside it in the air), the tool, and the outline the engine takes from
//! the model; it keeps no geometry of its own.

use std::f64::consts::{PI, TAU};

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::{Loop2Geom, Seg2Geom};

use crate::feeds::op_feeds;
use crate::offset::{JOINT_TOLERANCE, offset_regions};
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{ArcDirection, Coolant, MAX_FEED, MAX_SPINDLE_RPM, MIN_ARC_RADIUS, Move, Plane, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most levels a contour may take.
pub const MAX_CONTOUR_LEVELS: usize = 1_000;
/// How far past the stock the tool's edge is where it goes down, when the operation gives none, mm.
pub const DEFAULT_CONTOUR_CLEAR: f64 = 2.0;
/// The most one level takes off when the operation gives no stepdown, as a share of the tool's
/// diameter: the cut is as wide as the tool.
pub const DEFAULT_CONTOUR_STEPDOWN: f64 = 0.5;
/// The most stock a contour may leave on the wall, and the most it may clear the stock by, mm.
const MAX_LEAVE: f64 = 1_000.0;
/// The smallest stepdown, mm.
const MIN_STEP: f64 = 1e-3;
/// Heights closer than this are the same height, mm.
const SAME_HEIGHT: f64 = 1e-6;
/// The most places tried for the way in to one loop.
const MAX_WAYS_IN: usize = 64;
/// An arc whose middle is nearer its chord than this is cut as a line, mm.
const FLAT_ARC: f64 = 1e-7;
/// A way in longer than this many tool diameters is worth a note.
const LONG_WAY_IN: f64 = 2.0;

/// Which way round the cut goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CutDirection {
    /// The tooth meets the stock at its thickest: with a clockwise spindle, clockwise round the
    /// outside of the part.
    #[default]
    Climb,
    /// The other way round.
    Conventional,
}

/// A contour operation's parameters, as the design keeps them. Every one has a default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContourOp {
    /// The Z the cut starts from in the WCS; `None` for the stock's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// The Z it cuts down to; `None` for the model's bottom.
    #[serde(default)]
    pub bottom: Option<f64>,
    /// The most one level takes off, mm; `None` for half the tool's diameter.
    #[serde(default)]
    pub stepdown: Option<f64>,
    /// Stock left on the wall, mm; `None` for none.
    #[serde(default)]
    pub leave: Option<f64>,
    #[serde(default)]
    pub direction: CutDirection,
    /// How far past the stock the tool's edge is where it goes down, mm; `None` for 2.
    #[serde(default)]
    pub clear: Option<f64>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A contour operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Contoured {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// The Z the cut starts from in the WCS.
    pub top: f64,
    /// The Z of each level in the WCS, top down.
    pub levels: Vec<f64>,
    /// The stepdown used, mm: the most one level may take off.
    pub stepdown: f64,
    /// How far the tool's centre is from the outline, mm: its radius and the stock left.
    pub offset: f64,
    /// The length of the tool's centre path round the part at one level, mm.
    pub round: f64,
    /// Where the tool goes down for each loop it cuts round, X and Y.
    pub entries: Vec<[f64; 2]>,
    /// Things worth showing beside the toolpath (a capped spindle speed, a long way in).
    pub notes: Vec<String>,
}

/// One loop the tool's centre goes round, ready to cut.
struct Round {
    /// Where the tool goes down and comes up: beside the stock.
    entry: Vec2,
    /// Where it meets the loop and leaves it.
    start: Vec2,
    /// The loop from `start` back to it, the way it is cut.
    segs: Vec<Seg2>,
}

fn bad<T>(message: String) -> Result<T> {
    Err(Error::BadValue(message))
}

impl ContourOp {
    /// Checks the parameters and that `tool` can cut a wall.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
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
        if !self.leave.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the stock to leave must be 0 to {MAX_LEAVE} mm"));
        }
        if !self.clear.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the clearance past the stock must be 0 to {MAX_LEAVE} mm"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        match tool.kind {
            ToolKind::FlatEndMill | ToolKind::BullNoseEndMill => Ok(()),
            other => bad(format!("tool {} ({other:?}) does not cut a contour: use a flat or bull-nose end mill", tool.number)),
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down, at
    /// the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, round `outline`: the closed loops round the part at the cut's bottom, of lines
    /// and arcs, X and Y in the WCS, none overlapping another.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>, outline: &[Loop2]) -> Result<Contoured> {
        self.validate(tool)?;
        let (stock, model) = (placement.stock, placement.model);
        let top = self.top.unwrap_or(stock.max.z);
        let bottom = self.bottom.unwrap_or(model.min.z);
        let depth = top - bottom;
        if !(depth > SAME_HEIGHT) {
            return bad(format!("nothing to cut: the bottom ({bottom} mm) is not below the top ({top} mm)"));
        }
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the contour's top ({top} mm)"));
        }
        // The tool runs down the finished wall: the whole of it must be within the tool's reach
        // below anything wider than its cutting diameter.
        let g = tool.geometry;
        if !(g.diameter > 0.0 && g.diameter <= MAX_COORDINATE) {
            return bad(format!("tool {}'s diameter must be above 0", tool.number));
        }
        let reach = if g.shoulder_length > 0.0 { g.shoulder_length } else { g.flute_length };
        if !(depth <= reach + 1e-9) {
            return bad(format!(
                "the cut is {depth} mm deep, more than the {reach} mm tool {} reaches down a wall: raise the bottom or use a longer tool",
                tool.number
            ));
        }
        // Levels of equal depth, none deeper than the stepdown.
        let stepdown = self.stepdown.unwrap_or(DEFAULT_CONTOUR_STEPDOWN * g.diameter);
        let count = (depth / stepdown - 1e-9).ceil().max(1.0);
        if !(count <= MAX_CONTOUR_LEVELS as f64) {
            return bad(format!("{depth} mm in steps of {stepdown} mm takes more than {MAX_CONTOUR_LEVELS} levels"));
        }
        let count = count as usize;
        let per_level = depth / count as f64;
        if !(per_level <= g.flute_length + 1e-9) {
            return bad(format!(
                "each level is {per_level} mm deep, past tool {}'s {} mm of flutes: give the operation a smaller stepdown",
                tool.number, g.flute_length
            ));
        }
        let levels: Vec<f64> = (1..=count).map(|k| if k == count { bottom } else { top - per_level * k as f64 }).collect();
        // The tool's centre: the outline moved out by the tool's radius and the stock to leave.
        if outline.is_empty() {
            return bad("there is no outline to cut round".into());
        }
        let radius = 0.5 * g.diameter;
        let offset = radius + self.leave.unwrap_or(0.0);
        let regions: Vec<Region2> = outline.iter().map(|l| Region2 { outer: l.clone(), holes: Vec::new() }).collect();
        let moved = offset_regions(&regions, -offset).map_err(|e| Error::BadValue(format!("the part's outline: {e}")))?;
        if moved.is_empty() {
            return bad("the outline left nothing to cut round".into());
        }
        // Where the tool goes down: beside the stock, its edge `clear` from it.
        let beyond = radius + self.clear.unwrap_or(DEFAULT_CONTOUR_CLEAR);
        let beside = (Vec2 { x: stock.min.x - beyond, y: stock.min.y - beyond }, Vec2 { x: stock.max.x + beyond, y: stock.max.y + beyond });
        let rounds: Vec<Round> = moved.iter().map(|region| round(&region.outer, &moved, self.direction, beside)).collect::<Result<_>>()?;
        let f = op_feeds(tool, Engagement { axial: per_level.max(1e-4), radial: g.diameter }, material, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };

        let Some(first) = rounds.first().map(|r| r.entry) else { return bad("the outline left nothing to cut round".into()) };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for r in &rounds {
            let led = apart(r.entry, r.start) > JOINT_TOLERANCE;
            moves.push(Move::Rapid { to: at(r.entry, retract) });
            for &z in &levels {
                // Down beside the stock in the air, in to the wall, round it, and out again.
                moves.push(Move::Linear { to: at(r.entry, z), feed: f.plunge_feed });
                if led {
                    moves.push(Move::Linear { to: at(r.start, z), feed: f.feed });
                }
                let mut from = r.start;
                for (k, seg) in r.segs.iter().enumerate() {
                    // The last one ends where the first began, to the last bit.
                    let to = if k + 1 == r.segs.len() { r.start } else { seg.end() };
                    cut(&mut moves, seg, from, to, z, f.feed);
                    from = to;
                }
                if led {
                    moves.push(Move::Linear { to: at(r.entry, z), feed: f.feed });
                }
                moves.push(Move::Rapid { to: at(r.entry, retract) });
            }
            last = r.entry;
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;

        let mut notes = f.notes;
        let enclosed: usize = moved.iter().map(|region| region.holes.len()).sum();
        if enclosed > 0 {
            notes.push(format!(
                "{enclosed} enclosed place(s) between parts of the outline are not cut: the tool can't get into them from beside the stock"
            ));
        }
        let longest = rounds.iter().map(|r| apart(r.entry, r.start)).fold(0.0, f64::max);
        if longest > LONG_WAY_IN * g.diameter {
            notes.push(format!("the way in from beside the stock is {longest:.1} mm of cutting as wide as the tool, at every level"));
        }
        Ok(Contoured {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top,
            levels,
            stepdown,
            offset,
            round: rounds.iter().map(|r| r.segs.iter().map(Seg2Geom::length).sum::<f64>()).sum(),
            entries: rounds.iter().map(|r| [r.entry.x, r.entry.y]).collect(),
            notes,
        })
    }
}

fn apart(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The cutting moves along `seg` at height `z`, from `from` (where the tool is) to `to` (the
/// segment's end): a line, or an arc in at most half-turns. An arc too flat or too small to be
/// one on a machine is cut as a line, which is nearer the arc than any machine resolves.
fn cut(moves: &mut Vec<Move>, seg: &Seg2, from: Vec2, to: Vec2, z: f64, feed: f64) {
    let at = |p: Vec2| Vec3 { x: p.x, y: p.y, z };
    let Seg2::Arc { center, radius, start, sweep } = *seg else {
        if apart(from, to) > 0.0 {
            moves.push(Move::Linear { to: at(to), feed });
        }
        return;
    };
    let pieces = if sweep.abs() > PI + 1e-9 { 2 } else { 1 };
    let step = sweep / f64::from(pieces);
    let flat = radius < MIN_ARC_RADIUS || radius * (1.0 - (0.5 * step).cos()) < FLAT_ARC;
    for k in 1..=pieces {
        let angle = start + step * f64::from(k);
        let end = if k == pieces { to } else { Vec2 { x: center.x + radius * angle.cos(), y: center.y + radius * angle.sin() } };
        if flat {
            moves.push(Move::Linear { to: at(end), feed });
        } else {
            let direction = if sweep > 0.0 { ArcDirection::CounterClockwise } else { ArcDirection::Clockwise };
            moves.push(Move::Arc { to: at(end), center: at(center), plane: Plane::Xy, direction, feed });
        }
    }
}

/// A counter-clockwise loop of the tool's centre as a round to cut, turned the way `direction`
/// asks. It starts at the middle of a side, with the tool going down straight out from there,
/// outside `beside` (the stock's box grown by the tool's radius and its clearance). The side is
/// the longest straight one (the longest arc when there is none; of equals, the one nearest the
/// front, then the left) from which that way in passes clear of every loop of `all`, the paths
/// round every part of the outline: a way in across another part would cut into it.
fn round(path: &Loop2, all: &[Region2], direction: CutDirection, beside: (Vec2, Vec2)) -> Result<Round> {
    // Lengths and places to a micron, so that equal sides are equal whatever their last bits.
    let micron = |v: f64| (v * 1e6).round() as i64;
    let mut sides: Vec<(usize, bool, i64, i64, i64)> = path
        .segs
        .iter()
        .enumerate()
        .map(|(i, s)| {
            let mid = s.mid();
            (i, matches!(s, Seg2::Line { .. }), micron(s.length()), micron(mid.y), micron(mid.x))
        })
        .collect();
    sides.sort_by(|a, b| b.1.cmp(&a.1).then(b.2.cmp(&a.2)).then(a.3.cmp(&b.3)).then(a.4.cmp(&b.4)).then(a.0.cmp(&b.0)));
    let (lo, hi) = beside;
    for &(at, ..) in sides.iter().take(MAX_WAYS_IN) {
        let Some(seg) = path.segs.get(at) else { continue };
        let (before, after) = seg.split_half();
        let (start, along) = (after.start(), after.start_tangent());
        // Straight out of the loop: to the right of its travel, the loop being counter-clockwise.
        let out = Vec2 { x: along.y, y: -along.x };
        // How far out until past the stock's reach on that side; nothing when it already is.
        let exit = |from: f64, way: f64, low: f64, high: f64| {
            if way > 1e-12 {
                (high - from) / way
            } else if way < -1e-12 {
                (low - from) / way
            } else {
                f64::INFINITY
            }
        };
        let within = start.x > lo.x && start.x < hi.x && start.y > lo.y && start.y < hi.y;
        let far = if within { exit(start.x, out.x, lo.x, hi.x).min(exit(start.y, out.y, lo.y, hi.y)).max(0.0) } else { 0.0 };
        if !far.is_finite() || !way_is_clear(start, out, far, all) {
            continue;
        }
        let mut segs = vec![after];
        segs.extend(path.segs.iter().skip(at + 1).copied());
        segs.extend(path.segs.iter().take(at).copied());
        segs.push(before);
        // Climb with a clockwise spindle is clockwise round the outside of the part.
        if direction == CutDirection::Climb {
            segs = Loop2 { segs }.reversed().segs;
        }
        return Ok(Round { entry: Vec2 { x: start.x + far * out.x, y: start.y + far * out.y }, start, segs });
    }
    bad("no straight way in from beside the stock to the part's outline is clear of the part: this outline needs the tool to ramp in, which is not built yet".into())
}

/// Whether the straight way from `from` along the unit direction `way` for `far` mm meets no
/// segment of `regions`' loops past its first [`JOINT_TOLERANCE`] (it starts on one of them).
fn way_is_clear(from: Vec2, way: Vec2, far: f64, regions: &[Region2]) -> bool {
    let cross = |a: Vec2, b: Vec2| a.x * b.y - a.y * b.x;
    let dot = |a: Vec2, b: Vec2| a.x * b.x + a.y * b.y;
    let on_the_way = |t: f64| t > JOINT_TOLERANCE && t <= far;
    let meets = |seg: &Seg2| match *seg {
        Seg2::Arc { center, radius, start, sweep } => {
            // |from + t way - center| = radius.
            let f = Vec2 { x: from.x - center.x, y: from.y - center.y };
            let (half, rest) = (dot(f, way), dot(f, f) - radius * radius);
            let inside = half * half - rest;
            if inside < 0.0 {
                return false;
            }
            [-half - inside.sqrt(), -half + inside.sqrt()].into_iter().filter(|&t| on_the_way(t)).any(|t| {
                let angle = (f.y + t * way.y).atan2(f.x + t * way.x);
                let past = if sweep >= 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
                past <= sweep.abs() + 1e-9 || past >= TAU - 1e-9
            })
        }
        _ => {
            let (a, b) = (seg.start(), seg.end());
            let (side, to_a) = (Vec2 { x: b.x - a.x, y: b.y - a.y }, Vec2 { x: a.x - from.x, y: a.y - from.y });
            let turn = cross(way, side);
            if turn.abs() <= 1e-12 * side.x.hypot(side.y).max(1e-300) {
                // Alongside: in the way only when it lies on the way itself.
                let (near, other) = (dot(to_a, way), dot(Vec2 { x: b.x - from.x, y: b.y - from.y }, way));
                return cross(to_a, way).abs() <= JOINT_TOLERANCE && near.max(other) > JOINT_TOLERANCE && near.min(other) <= far;
            }
            let (t, u) = (cross(to_a, side) / turn, cross(to_a, way) / turn);
            on_the_way(t) && (-1e-9..=1.0 + 1e-9).contains(&u)
        }
    };
    !regions.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).flat_map(|l| &l.segs).any(meets)
}

#[cfg(test)]
#[path = "contour_tests.rs"]
mod tests;
