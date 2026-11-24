//! The engrave operation (decision 0032 of the CAM work): the tool's centre traced along curves
//! drawn in a sketch, a little below the face they are drawn on. It reads the setup's
//! placement, the tool, and the curves as the engine reads them from the sketch: lines and arcs
//! in the setup's WCS, and the height of the sketch's plane. It keeps no geometry of its own
//! but the sketch's name.
//!
//! Pieces that meet are joined end to end into runs, and the runs are cut each after the one
//! that ends nearest, a run turned round when its far end is the nearer. The tool goes down at
//! the plunge feed where a run starts, along it at the cutting feed, and up where it ends.

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use solidcore_record::{Seg2, Vec2, Vec3};
use solidcraft_geom::Seg2Geom;

use crate::Result;
use crate::contour::{MAX_LEAVE, SAME_HEIGHT, apart, bad, cut};
use crate::feeds::op_feeds;
use crate::offset::JOINT_TOLERANCE;
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;

/// How deep an engraving goes when the operation gives no depth, mm.
pub const DEFAULT_ENGRAVE_DEPTH: f64 = 0.2;
/// The shallowest engraving, mm.
pub const MIN_ENGRAVE_DEPTH: f64 = 0.001;
/// The most pieces of curve one engraving follows.
pub const MAX_ENGRAVE_PIECES: usize = 200_000;
/// The longest name a sketch is asked for by.
const MAX_NAME: usize = 256;
/// The most runs that are put in order, each after the one that ends nearest: more are cut in
/// the order they were joined in (looking for the nearest of very many takes too long).
const ORDER_MOST: usize = 5_000;

/// An engraving's parameters, as the design keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EngraveOp {
    /// The sketch whose curves are followed: its name in the design.
    pub sketch: String,
    /// How deep the tool's tip goes below the surface, mm; `None` for 0.2.
    #[serde(default)]
    pub depth: Option<f64>,
    /// The Z of the surface that is engraved, in the WCS; `None` for the sketch's own plane.
    #[serde(default)]
    pub top: Option<f64>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// An engraving's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Engraved {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// The Z of the surface and of the tool's tip as it cuts, in the WCS.
    pub top: f64,
    pub z: f64,
    /// How deep the cut is, mm.
    pub depth: f64,
    /// How wide the groove is at the surface, mm.
    pub width: f64,
    /// The runs the curves were joined into: the length of each, in the order cut.
    pub runs: Vec<f64>,
    /// Things worth showing beside the toolpath (a capped spindle speed).
    pub notes: Vec<String>,
}

impl EngraveOp {
    /// Checks the parameters and that `tool` can engrave.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        if self.sketch.trim().is_empty() || self.sketch.len() > MAX_NAME {
            return bad(format!("an engraving follows a sketch: its name, at most {MAX_NAME} bytes long"));
        }
        if !self.depth.is_none_or(|v| (MIN_ENGRAVE_DEPTH..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the depth must be {MIN_ENGRAVE_DEPTH} to {MAX_LEAVE} mm"));
        }
        if !self.top.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the surface's height must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        let pointed = matches!(tool.kind, ToolKind::Engraving | ToolKind::ChamferMill | ToolKind::SpotDrill);
        if !pointed && !matches!(tool.kind, ToolKind::FlatEndMill | ToolKind::BallEndMill | ToolKind::BullNoseEndMill) {
            return bad(format!(
                "tool {} ({:?}) does not engrave: use an engraving tool, a chamfer mill, a spot drill or an end mill",
                tool.number, tool.kind
            ));
        }
        let depth = self.depth.unwrap_or(DEFAULT_ENGRAVE_DEPTH);
        if !(depth <= tool.geometry.flute_length + 1e-9) {
            return bad(format!("the engraving is {depth} mm deep, past tool {}'s {} mm of flutes", tool.number, tool.geometry.flute_length));
        }
        Ok(())
    }

    /// How wide the groove `tool` leaves at the surface is, mm, at the operation's depth: a
    /// pointed tool's cone at that height (no wider than the tool), a ball's chord, an end
    /// mill's own width.
    pub fn width(&self, tool: &Tool) -> f64 {
        let (g, depth) = (tool.geometry, self.depth.unwrap_or(DEFAULT_ENGRAVE_DEPTH));
        match tool.kind {
            ToolKind::Engraving | ToolKind::ChamferMill | ToolKind::SpotDrill => {
                (g.tip_diameter + 2.0 * depth * (0.5 * g.tip_angle).tan()).min(g.diameter)
            }
            ToolKind::BallEndMill if depth < 0.5 * g.diameter => 2.0 * (depth * (g.diameter - depth)).max(0.0).sqrt(),
            _ => g.diameter,
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down,
    /// at the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, along `pieces`: the sketch's curves as lines and arcs, X and Y in the WCS, in
    /// any order; `surface` is the Z of the sketch's plane in the WCS.
    pub fn toolpath(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        surface: f64,
        pieces: &[Seg2],
    ) -> Result<Engraved> {
        self.validate(tool)?;
        if pieces.is_empty() {
            return bad(format!("sketch '{}' has no curves to follow", self.sketch));
        }
        if pieces.len() > MAX_ENGRAVE_PIECES {
            return bad(format!(
                "sketch '{}' comes to {} pieces of curve, more than the {MAX_ENGRAVE_PIECES} an engraving follows",
                self.sketch,
                pieces.len()
            ));
        }
        let finite = |p: Vec2| p.x.abs() <= MAX_COORDINATE && p.y.abs() <= MAX_COORDINATE;
        if !pieces
            .iter()
            .all(|seg| matches!(seg, Seg2::Line { .. } | Seg2::Arc { .. }) && finite(seg.start()) && finite(seg.end()) && seg.length().is_finite())
        {
            return bad(format!("sketch '{}': its curves must be lines and arcs within {MAX_COORDINATE} mm", self.sketch));
        }
        let top = self.top.unwrap_or(surface);
        if !(top.abs() <= MAX_COORDINATE) {
            return bad("the surface's height is not a number".into());
        }
        let depth = self.depth.unwrap_or(DEFAULT_ENGRAVE_DEPTH);
        let z = top - depth;
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the engraved surface ({top} mm)"));
        }
        let runs = ordered(joined(pieces), start.map(|at| Vec2 { x: at.x, y: at.y }));
        // The tool's own chip load, as a chamfer's: the cut is narrow and made near the tip.
        let g = tool.geometry;
        let f = op_feeds(tool, Engagement { axial: depth.max(1e-4), radial: g.diameter }, material, placement.max_rpm, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
        let Some(first) = runs.first().and_then(|run| run.first()).map(Seg2Geom::start) else {
            return bad(format!("sketch '{}' has no curves to follow", self.sketch));
        };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for run in &runs {
            let Some(begin) = run.first().map(Seg2Geom::start) else { continue };
            moves.push(Move::Rapid { to: at(begin, retract) });
            moves.push(Move::Linear { to: at(begin, z), feed: f.plunge_feed });
            let mut from = begin;
            for seg in run {
                let to = seg.end();
                cut(&mut moves, seg, from, to, (z, z), f.feed);
                from = to;
            }
            moves.push(Move::Rapid { to: at(from, retract) });
            last = from;
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;
        Ok(Engraved {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top,
            z,
            depth,
            width: self.width(tool),
            runs: runs.iter().map(|run| run.iter().map(Seg2Geom::length).sum()).collect(),
            notes: f.notes,
        })
    }
}

/// The cell of a grid of [`JOINT_TOLERANCE`] a point is in: points that are one place are in
/// the same cell or in cells next to each other.
fn cell(p: Vec2) -> (i64, i64) {
    ((p.x / JOINT_TOLERANCE).floor() as i64, (p.y / JOINT_TOLERANCE).floor() as i64)
}

/// `pieces` joined end to end into runs: a piece goes on after another when it starts where
/// that one ends (turned round when it is its end that is there) and no other piece does. Where
/// three or more ends meet, the runs end: which way to go on there is not the tool's to
/// choose. Pieces of no length are dropped.
fn joined(pieces: &[Seg2]) -> Vec<Vec<Seg2>> {
    let pieces: Vec<Seg2> = pieces.iter().copied().filter(|seg| seg.length() > JOINT_TOLERANCE).collect();
    // Every end of every piece, by the cell it is in.
    let mut ends: HashMap<(i64, i64), Vec<(usize, bool)>> = HashMap::new();
    for (k, seg) in pieces.iter().enumerate() {
        ends.entry(cell(seg.start())).or_default().push((k, false));
        ends.entry(cell(seg.end())).or_default().push((k, true));
    }
    let place = |k: usize, far: bool| pieces.get(k).map(|seg| if far { seg.end() } else { seg.start() });
    // The other ends at the place of the end `far` of piece `k`.
    let meeting = |k: usize, far: bool| -> Vec<(usize, bool)> {
        let Some(at) = place(k, far) else { return Vec::new() };
        let (cx, cy) = cell(at);
        let mut found = Vec::new();
        for dx in -1..=1 {
            for dy in -1..=1 {
                for &(other, end) in ends.get(&(cx + dx, cy + dy)).map_or(&[][..], Vec::as_slice) {
                    if (other, end) != (k, far) && place(other, end).is_some_and(|p| apart(p, at) <= JOINT_TOLERANCE) {
                        found.push((other, end));
                    }
                }
            }
        }
        found
    };
    // The one piece that goes on from an end, when exactly one other end is there.
    let onward = |k: usize, far: bool| -> Option<(usize, bool)> {
        let found = meeting(k, far);
        (found.len() == 1).then(|| found.first().copied()).flatten()
    };
    let mut used = vec![false; pieces.len()];
    let mut runs: Vec<Vec<Seg2>> = Vec::new();
    // Runs with a free end first, from that end; then what is left, which is closed loops.
    let starts = (0..pieces.len()).flat_map(|k| [(k, false), (k, true)]).filter(|&(k, far)| onward(k, far).is_none());
    let loops = (0..pieces.len()).map(|k| (k, false));
    for (k, from_far) in starts.collect::<Vec<_>>().into_iter().chain(loops) {
        if used.get(k).copied().unwrap_or(true) {
            continue;
        }
        let mut run = Vec::new();
        // Along the piece from the end it was reached at to its other end, and on from there.
        let (mut at, mut entered_far) = (k, from_far);
        while let Some(seg) = pieces.get(at) {
            if let Some(slot) = used.get_mut(at) {
                *slot = true;
            }
            run.push(if entered_far { seg.reversed() } else { *seg });
            match onward(at, !entered_far) {
                Some((next, end)) if !used.get(next).copied().unwrap_or(true) => (at, entered_far) = (next, end),
                _ => break,
            }
        }
        runs.push(run);
    }
    runs
}

/// `runs` in the order they are cut: each after the one that ends nearest it, from `from` (the
/// first run as given when the tool is nowhere yet), a run turned round when its far end is the
/// nearer.
fn ordered(mut runs: Vec<Vec<Seg2>>, from: Option<Vec2>) -> Vec<Vec<Seg2>> {
    if runs.len() > ORDER_MOST {
        return runs;
    }
    let mut out: Vec<Vec<Seg2>> = Vec::with_capacity(runs.len());
    let mut at = from;
    while !runs.is_empty() {
        let pick = match at {
            None => (0, false),
            Some(p) => runs
                .iter()
                .enumerate()
                .flat_map(|(k, run)| {
                    let near = run.first().map(|seg| (k, false, apart(seg.start(), p)));
                    let far = run.last().map(|seg| (k, true, apart(seg.end(), p)));
                    near.into_iter().chain(far)
                })
                .min_by(|a, b| a.2.total_cmp(&b.2))
                .map_or((0, false), |(k, turned, _)| (k, turned)),
        };
        let mut run = runs.swap_remove(pick.0.min(runs.len() - 1));
        if pick.1 {
            run.reverse();
            run.iter_mut().for_each(|seg| *seg = seg.reversed());
        }
        at = run.last().map(Seg2Geom::end).or(at);
        out.push(run);
    }
    out
}

#[cfg(test)]
#[path = "engrave_tests.rs"]
mod tests;
