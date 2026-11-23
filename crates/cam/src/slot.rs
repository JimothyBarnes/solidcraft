//! The slot operation (decision 0026 of the CAM work): a slot milled along its middle. It reads
//! the setup's placement, the tool, and each slot as the engine reads it from the model: how far
//! down it goes and its outline. It keeps no geometry of its own but a point in each slot.
//!
//! A slot here is closed, with two half-round ends as wide as itself and, between them, two
//! straight sides or two arcs about one centre. Its middle line runs from the centre of one end
//! to the centre of the other, and the tool's centre never leaves it.
//!
//! The tool goes down along the middle line itself, to and fro, no steeper than the ramp angle;
//! then once along the line at the level, which cuts away what the ramp left; and so on down. A
//! tool as wide as the slot cuts it in one go, both walls at once. A narrower one leaves the
//! same on each wall, and the operation says how much: an inside contour finishes them, which
//! makes the more accurate slot (`research/slot-milling.md`).

use std::f64::consts::{PI, TAU};

use serde::{Deserialize, Serialize};
use solidcore_record::{Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::{Loop2Geom, Seg2Geom};

use crate::contour::{MIN_STEP, SAME_HEIGHT, apart, bad, cut};
use crate::feeds::op_feeds;
use crate::offset::JOINT_TOLERANCE;
use crate::placement::Placement;
use crate::pocket::{DEFAULT_RAMP_ANGLE, MAX_RAMP_TURNS, RAMP_ANGLES};
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most slots one operation may cut.
pub const MAX_SLOTS: usize = 1_000;
/// The most levels a slot may take.
pub const MAX_SLOT_LEVELS: usize = 1_000;
/// The most one level takes off when the operation gives no stepdown, as a share of the tool's
/// diameter: every cut along a slot is as wide as the tool.
pub const DEFAULT_SLOT_STEPDOWN: f64 = 0.5;
/// How far two sizes read from a model may differ and still be the same, mm.
const SAME: f64 = JOINT_TOLERANCE;
/// How far an end's arc may be from half a turn and still be one, radians.
const HALF_TURN_SLACK: f64 = 1e-6;

/// A slot as it is read from the model, in the setup's WCS.
#[derive(Clone, Debug, PartialEq)]
pub struct SlotShape {
    /// The Z it goes down to: its floor, or the part's bottom where it goes through.
    pub bottom: f64,
    /// Its outline: the wall just above its bottom.
    pub region: Region2,
}

/// A slot operation's parameters, as the design keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlotOp {
    /// A point in each slot to cut, X and Y in the setup's WCS, mm: the model is read there each
    /// time the toolpath is worked out.
    pub slots: Vec<[f64; 2]>,
    /// The Z the cut starts from; `None` for the stock's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// The Z every slot is cut down to; `None` for each slot's own, as the model has it.
    #[serde(default)]
    pub bottom: Option<f64>,
    /// The most one level takes off, mm; `None` for half the tool's diameter.
    #[serde(default)]
    pub stepdown: Option<f64>,
    /// The angle the tool goes down along the slot at, radians; `None` for 3 degrees.
    #[serde(default)]
    pub ramp_angle: Option<f64>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A slot operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Slotted {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    pub ramp_feed: f64,
    /// The Z the cut starts from.
    pub top: f64,
    /// The Z of each level of each slot, top down.
    pub levels: Vec<Vec<f64>>,
    /// The stepdown used, mm.
    pub stepdown: f64,
    /// Each slot's width, mm.
    pub widths: Vec<f64>,
    /// The length of each slot's middle line, from the centre of one end to the other's, mm.
    pub lengths: Vec<f64>,
    /// How many times along its middle line the tool goes to get down one level, for each slot.
    pub legs: Vec<usize>,
    /// The stock left on each wall of each slot, mm: half of what it is wider than the tool.
    pub left: Vec<f64>,
    /// Things worth showing beside the toolpath (a capped spindle speed, walls left to finish).
    pub notes: Vec<String>,
}

/// The middle of a slot, as [`slot_line`] reads it from an outline.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SlotLine {
    /// From the centre of one end to the centre of the other: a line, or an arc.
    pub line: Seg2,
    /// The slot's width: twice its ends' radius.
    pub width: f64,
}

/// One slot, ready to cut.
struct Plan {
    /// The middle line from the end the tool starts at.
    line: Seg2,
    levels: Vec<f64>,
    legs: usize,
    width: f64,
}

impl SlotOp {
    /// Checks the parameters and that `tool` can cut a slot.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        if self.slots.is_empty() || self.slots.len() > MAX_SLOTS {
            return bad(format!("a slot operation cuts 1 to {MAX_SLOTS} slots, not {}", self.slots.len()));
        }
        if !self.slots.iter().all(|p| p.iter().all(|v| v.abs() <= MAX_COORDINATE)) {
            return bad(format!("the points in the slots must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.top.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the top must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.bottom.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the bottom must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.stepdown.is_none_or(|v| (MIN_STEP..=MAX_COORDINATE).contains(&v)) {
            return bad(format!("the stepdown must be at least {MIN_STEP} mm"));
        }
        if !self.ramp_angle.is_none_or(|v| (RAMP_ANGLES.0..=RAMP_ANGLES.1).contains(&v)) {
            return bad("the ramp angle must be 0.5 to 30 degrees".into());
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        match tool.kind {
            ToolKind::FlatEndMill | ToolKind::BullNoseEndMill => Ok(()),
            other => bad(format!("tool {} ({other:?}) does not cut a slot: use a flat or bull-nose end mill", tool.number)),
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down, at
    /// the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, in `shapes`: the slots as read from the model, one for each of the operation's
    /// points, in their order.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>, shapes: &[SlotShape]) -> Result<Slotted> {
        self.validate(tool)?;
        if shapes.len() != self.slots.len() {
            return bad(format!("{} slots were read from the model for the operation's {} points", shapes.len(), self.slots.len()));
        }
        let g = tool.geometry;
        if !(g.diameter > 0.0 && g.diameter <= MAX_COORDINATE) {
            return bad(format!("tool {}'s diameter must be above 0", tool.number));
        }
        let top = self.top.unwrap_or(placement.stock.max.z);
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the slot's top ({top} mm)"));
        }
        let stepdown = self.stepdown.unwrap_or(DEFAULT_SLOT_STEPDOWN * g.diameter);
        let reach = if g.shoulder_length > 0.0 { g.shoulder_length } else { g.flute_length };
        let angle = self.ramp_angle.unwrap_or(DEFAULT_RAMP_ANGLE);

        // Each slot's middle line, levels and ramp, before any move is written.
        let mut plans: Vec<Plan> = Vec::with_capacity(shapes.len());
        let mut notes: Vec<String> = Vec::new();
        let mut deepest = 0.0_f64;
        let mut tool_at: Option<Vec2> = None;
        for (shape, at) in shapes.iter().zip(&self.slots) {
            // The point as given, but for a zero that is written with a minus sign.
            let what = format!("the slot at X {} Y {}", at[0] + 0.0, at[1] + 0.0);
            let m = slot_line(&shape.region).map_err(|why| Error::BadValue(format!("{what}: {why}")))?;
            if g.diameter > m.width + SAME {
                return bad(format!(
                    "tool {} ({} mm across) is wider than {what}, which is {:.3} mm wide: use a narrower tool",
                    tool.number, g.diameter, m.width
                ));
            }
            let bottom = self.bottom.unwrap_or(shape.bottom);
            let depth = top - bottom;
            if !(depth > SAME_HEIGHT) {
                return bad(format!("{what}: its bottom (Z {bottom} mm) is not below the top (Z {top} mm)"));
            }
            if !(depth <= reach + 1e-9) {
                return bad(format!(
                    "{what} is {depth} mm deep, more than the {reach} mm tool {} reaches down a wall: use a longer tool",
                    tool.number
                ));
            }
            let count = (depth / stepdown - 1e-9).ceil().max(1.0);
            if !(count <= MAX_SLOT_LEVELS as f64) {
                return bad(format!("{what}: {depth} mm in steps of {stepdown} mm takes more than {MAX_SLOT_LEVELS} levels"));
            }
            let count = count as usize;
            let per_level = depth / count as f64;
            if !(per_level <= g.flute_length + 1e-9) {
                return bad(format!(
                    "{what}: each level is {per_level} mm deep, past tool {}'s {} mm of flutes: give the operation a smaller stepdown",
                    tool.number, g.flute_length
                ));
            }
            // Down one level in whole lengths of the middle line, none steeper than the angle.
            let length = m.line.length();
            let legs = (per_level / angle.tan() / length).ceil();
            if !(length > 0.0 && legs <= MAX_RAMP_TURNS) {
                return bad(format!(
                    "{what} is {length:.3} mm from the centre of one end to the other's, so the ramp down {per_level} mm would go along it {legs} times: use a steeper ramp"
                ));
            }
            let legs = (legs as usize).max(1);
            deepest = deepest.max(per_level);
            let levels: Vec<f64> = (1..=count).map(|k| if k == count { bottom } else { top - per_level * k as f64 }).collect();
            // From the end nearer where the tool is; the first slot from its end nearer the
            // front, then the left.
            let (a, b) = (m.line.start(), m.line.end());
            let turned = match tool_at {
                Some(p) => apart(p, b) < apart(p, a),
                None => (b.y, b.x) < (a.y, a.x),
            };
            let line = if turned { m.line.reversed() } else { m.line };
            // A level is its legs and once more along the line: where that leaves the tool.
            let times = count * (legs + 1);
            tool_at = Some(if times.is_multiple_of(2) { line.start() } else { line.end() });
            let left = 0.5 * (m.width - g.diameter);
            if left > SAME {
                notes.push(format!(
                    "tool {} ({} mm across) is narrower than {what} ({:.3} mm wide): {left:.3} mm stays on each wall, for an inside contour to finish",
                    tool.number, g.diameter, m.width
                ));
            }
            plans.push(Plan { line, levels, legs, width: m.width });
        }
        let f = op_feeds(tool, Engagement { axial: deepest.max(1e-4), radial: g.diameter }, material, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };

        let Some(first) = plans.first().map(|plan| plan.line.start()) else {
            return bad("there is nothing to cut".into());
        };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for plan in &plans {
            let back = plan.line.reversed();
            let (a, b) = (plan.line.start(), plan.line.end());
            // Down to the top in the air above it, then along the line.
            moves.push(Move::Rapid { to: at(a, retract) });
            moves.push(Move::Linear { to: at(a, top), feed: f.plunge_feed });
            let (mut above, mut at_a) = (top, true);
            let along = |moves: &mut Vec<Move>, at_a: &mut bool, heights: (f64, f64), feed: f64| {
                let (seg, from, to) = if *at_a { (&plan.line, a, b) } else { (&back, b, a) };
                cut(moves, seg, from, to, heights, feed);
                *at_a = !*at_a;
            };
            for &z in &plan.levels {
                let fall = (above - z) / plan.legs as f64;
                for k in 0..plan.legs {
                    let to = if k + 1 == plan.legs { z } else { above - fall * (k + 1) as f64 };
                    along(&mut moves, &mut at_a, (above - fall * k as f64, to), f.ramp_feed);
                }
                // Once along it at the level: the last leg left a wedge, highest where it began.
                along(&mut moves, &mut at_a, (z, z), f.feed);
                above = z;
            }
            last = if at_a { a } else { b };
            moves.push(Move::Rapid { to: at(last, retract) });
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;
        Ok(Slotted {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            ramp_feed: f.ramp_feed,
            top,
            levels: plans.iter().map(|plan| plan.levels.clone()).collect(),
            stepdown,
            widths: plans.iter().map(|plan| plan.width).collect(),
            lengths: plans.iter().map(|plan| plan.line.length()).collect(),
            legs: plans.iter().map(|plan| plan.legs).collect(),
            left: plans.iter().map(|plan| (0.5 * (plan.width - g.diameter)).max(0.0)).collect(),
            notes: f.notes.into_iter().chain(notes).collect(),
        })
    }
}

/// The middle line of `region` when it is a slot, or why it is not one: what a form offers as
/// slots and what the operation cuts are the same test.
pub fn slot_line(region: &Region2) -> std::result::Result<SlotLine, String> {
    const NOT_A_SLOT: &str =
        "it is not a slot: a slot has two half-round ends as wide as itself with two straight sides, or two arcs about one centre, between them";
    if !region.holes.is_empty() {
        return Err("something stands in it: clear it with Pocket".into());
    }
    let sides = sides(&region.outer.ccw().segs);
    let (ends, between) = match sides.as_slice() {
        [Seg2::Arc { .. }] => return Err("it is round: drill it or bore it".into()),
        [a @ Seg2::Arc { .. }, b, c @ Seg2::Arc { .. }, d] if half_turn(a) && half_turn(c) => ([*a, *c], [*b, *d]),
        [a, b @ Seg2::Arc { .. }, c, d @ Seg2::Arc { .. }] if half_turn(b) && half_turn(d) => ([*b, *d], [*a, *c]),
        _ => return Err(NOT_A_SLOT.into()),
    };
    let ([Seg2::Arc { center: c1, radius, .. }, Seg2::Arc { center: c2, radius: other, .. }], true) = (ends, sides.len() == 4) else {
        return Err(NOT_A_SLOT.into());
    };
    if !(radius > 0.0 && (radius - other).abs() <= SAME) {
        return Err(NOT_A_SLOT.into());
    }
    let near = |p: Vec2, q: Vec2| apart(p, q) <= 4.0 * SAME;
    let line = match between {
        [Seg2::Line { a: p0, b: p1 }, Seg2::Line { a: q0, b: q1 }] => {
            // Straight: the sides are the line between the ends' centres, moved its radius
            // to either hand.
            let length = apart(c1, c2);
            if !(length > SAME) {
                return Err(NOT_A_SLOT.into());
            }
            let n = Vec2 { x: -(c2.y - c1.y) / length * radius, y: (c2.x - c1.x) / length * radius };
            let moved = |c: Vec2, sign: f64| Vec2 { x: c.x + sign * n.x, y: c.y + sign * n.y };
            let side = |from: Vec2, to: Vec2, sign: f64| {
                (near(from, moved(c1, sign)) && near(to, moved(c2, sign))) || (near(from, moved(c2, sign)) && near(to, moved(c1, sign)))
            };
            if !((side(p0, p1, 1.0) && side(q0, q1, -1.0)) || (side(p0, p1, -1.0) && side(q0, q1, 1.0))) {
                return Err(NOT_A_SLOT.into());
            }
            Seg2::Line { a: c1, b: c2 }
        }
        [Seg2::Arc { center: o1, radius: r1, start: s1, sweep: w1 }, Seg2::Arc { center: o2, radius: r2, start: s2, sweep: w2 }] => {
            // Round a centre: the sides are arcs about it, the ends' radius either side of
            // the arc through the ends' centres. The outer one goes the way the loop does.
            let (outer, inner, from, turn) = if w1 > 0.0 { (r1, r2, s1, w1) } else { (r2, r1, s2, w2) };
            let mid = 0.5 * (outer + inner);
            let same_turn = (w1 + w2).abs() <= HALF_TURN_SLACK && w1 * w2 < 0.0 && turn < TAU - HALF_TURN_SLACK;
            if !(near(o1, o2) && same_turn && (outer - inner - 2.0 * radius).abs() <= 2.0 * SAME && inner > 0.0) {
                return Err(NOT_A_SLOT.into());
            }
            let on = |angle: f64| Vec2 { x: o1.x + mid * angle.cos(), y: o1.y + mid * angle.sin() };
            let ends_fit = (near(on(from), c1) && near(on(from + turn), c2)) || (near(on(from), c2) && near(on(from + turn), c1));
            if !(ends_fit && mid * turn > SAME) {
                return Err(NOT_A_SLOT.into());
            }
            Seg2::Arc { center: o1, radius: mid, start: from, sweep: turn }
        }
        _ => return Err(NOT_A_SLOT.into()),
    };
    Ok(SlotLine { line, width: 2.0 * radius })
}

/// Whether `seg` is an arc of half a turn, counter-clockwise: a slot's end on its outline.
fn half_turn(seg: &Seg2) -> bool {
    matches!(*seg, Seg2::Arc { sweep, .. } if (sweep - PI).abs() <= HALF_TURN_SLACK)
}

/// The sides of a closed loop: its segments, with neighbours that lie on one line or on one
/// circle joined into one (a model's section gives an arc in as many pieces as it has faces).
fn sides(segs: &[Seg2]) -> Vec<Seg2> {
    let mut out: Vec<Seg2> = Vec::with_capacity(segs.len());
    for seg in segs {
        match out.last().and_then(|last| joined(last, seg)) {
            Some(one) => {
                out.pop();
                out.push(one);
            }
            None => out.push(*seg),
        }
    }
    // The loop may begin part of the way along a side.
    while out.len() > 1 {
        let Some(one) = out.last().zip(out.first()).and_then(|(last, first)| joined(last, first)) else { break };
        out.pop();
        out[0] = one;
    }
    out
}

/// `a` and `b`, which follows it, as one segment when they lie on one line or on one circle and
/// go the same way.
fn joined(a: &Seg2, b: &Seg2) -> Option<Seg2> {
    match (*a, *b) {
        (Seg2::Line { a: p, b: q }, Seg2::Line { a: _, b: r }) => {
            let (ux, uy, length) = (q.x - p.x, q.y - p.y, apart(p, q));
            if !(length > 0.0) {
                return None;
            }
            let off = ((r.x - p.x) * uy - (r.y - p.y) * ux) / length;
            let beyond = ((r.x - q.x) * ux + (r.y - q.y) * uy) / length;
            (off.abs() <= SAME && beyond > 0.0).then_some(Seg2::Line { a: p, b: r })
        }
        (Seg2::Arc { center, radius, start, sweep }, Seg2::Arc { center: other, radius: r, sweep: more, .. }) => {
            let same = apart(center, other) <= SAME && (radius - r).abs() <= SAME && sweep * more > 0.0;
            (same && (sweep + more).abs() <= TAU + HALF_TURN_SLACK).then_some(Seg2::Arc { center, radius, start, sweep: sweep + more })
        }
        _ => None,
    }
}

#[cfg(test)]
#[path = "slot_tests.rs"]
mod tests;
