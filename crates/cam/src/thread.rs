//! The thread mill operation (decision 0034 of the CAM work): an internal thread cut by going
//! round a drilled hole with a tool narrower than the hole. It reads the setup's placement and
//! the tool; the holes are points, the thread its major diameter, pitch and depth.
//!
//! The tool's centre goes round a circle half the difference of the thread's major diameter and
//! the tool's own from the hole's middle, rising one pitch a turn: counter-clockwise, which is
//! climb milling with a clockwise spindle, so a right-hand thread is cut from the bottom up and
//! a left-hand one from the top down. It comes on to the circle from the hole's middle by a half
//! turn of half the radius, which starts in the hole's air whatever the sizes and meets the
//! circle at a tangent, and leaves it the same way. A tool with teeth along several pitches
//! cuts that much thread in one turn and the rest in further turns, moved a whole number of
//! pitches between them; a tool with one row of teeth takes a turn for every pitch. The depth
//! of the thread is taken in passes of equal area (`research/thread-milling.md`).

use std::f64::consts::{PI, TAU};

use serde::{Deserialize, Serialize};
use solidcore_record::{Seg2, Vec2, Vec3};

use crate::Result;
use crate::contour::{SAME_HEIGHT, bad, cut};
use crate::feeds::op_feeds;
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;

/// The most holes one operation threads.
pub const MAX_THREADS: usize = 1_000;
/// How many passes the thread's depth is taken in when the operation gives none.
pub const DEFAULT_THREAD_PASSES: u32 = 2;
/// The most passes.
pub const MAX_THREAD_PASSES: u32 = 20;
/// The most turns one pass of one hole may take.
pub const MAX_THREAD_TURNS: f64 = 2_000.0;
/// How deep an ISO metric internal thread is cut, as a share of its pitch: five eighths of the
/// height of the 60 degree triangle, (5 / 8) x (sqrt 3 / 2).
pub const THREAD_DEPTH: f64 = 0.541_265_877_365_273_9;
/// The finest and the coarsest pitch, mm.
pub const THREAD_PITCHES: (f64, f64) = (0.05, 50.0);

/// Which way a thread turns.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadHand {
    #[default]
    Right,
    Left,
}

/// A thread mill operation's parameters, as the design keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ThreadOp {
    /// The middle of each hole, X and Y in the WCS.
    pub holes: Vec<[f64; 2]>,
    /// The thread's major diameter, mm (10 for M10).
    pub diameter: f64,
    /// Its pitch, mm; `None` for the tool's own.
    #[serde(default)]
    pub pitch: Option<f64>,
    /// The Z of the holes' top, in the WCS; `None` for the model's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// How far down from the top the thread goes, mm.
    pub depth: f64,
    #[serde(default)]
    pub hand: ThreadHand,
    /// How many passes the thread's depth is taken in; `None` for two.
    #[serde(default)]
    pub passes: Option<u32>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A feed at the cutting edge to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A thread mill operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Threaded {
    pub toolpath: Toolpath,
    pub rpm: f64,
    /// The feed at the cutting edge, where the tool's chip load is meant, mm/min.
    pub feed: f64,
    /// The feed of the tool's centre round the hole, which is what the program gives, mm/min:
    /// less than at the edge, as the centre's circle is smaller than the edge's.
    pub centre_feed: f64,
    pub plunge_feed: f64,
    /// The Z of the holes' top and of the thread's bottom, in the WCS.
    pub top: f64,
    pub bottom: f64,
    pub pitch: f64,
    /// The diameter the hole is drilled to for this thread, mm: the thread's minor diameter.
    pub minor: f64,
    /// The radius of the tool centre's circle at each pass, mm: the last is the thread's size.
    pub passes: Vec<f64>,
    /// How many turns each pass takes in a hole: one for every stretch of thread as long as the
    /// tool's teeth.
    pub turns: usize,
    /// Things worth showing beside the toolpath.
    pub notes: Vec<String>,
}

/// What a thread comes to with one tool.
struct Shape {
    pitch: f64,
    /// How deep the thread is cut into the hole's wall.
    cut: f64,
    /// The radius of the tool centre's circle at full depth.
    radius: f64,
    /// How much thread one turn cuts: the pitches the tool's teeth span.
    span: f64,
    passes: u32,
}

impl ThreadOp {
    /// Checks the parameters and that `tool` can cut the thread.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        self.shape(tool).map(|_| ())
    }

    fn shape(&self, tool: &Tool) -> Result<Shape> {
        if self.holes.is_empty() || self.holes.len() > MAX_THREADS {
            return bad(format!("a thread mill operation cuts 1 to {MAX_THREADS} holes"));
        }
        if !self.holes.iter().flatten().all(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the holes' places must be finite and within {MAX_COORDINATE} mm"));
        }
        if tool.kind != ToolKind::ThreadMill {
            return bad(format!("tool {} ({:?}) does not mill a thread: use a thread mill", tool.number, tool.kind));
        }
        let g = tool.geometry;
        let pitch = self.pitch.unwrap_or(g.thread_pitch);
        if !(THREAD_PITCHES.0..=THREAD_PITCHES.1).contains(&pitch) {
            return bad(format!("the thread's pitch must be {} to {} mm", THREAD_PITCHES.0, THREAD_PITCHES.1));
        }
        // The pitches the teeth span: one row of teeth cuts any pitch, several only their own.
        let rows = (g.flute_length / pitch + 1e-9).floor().max(1.0);
        if rows >= 2.0 && (pitch - g.thread_pitch).abs() > 1e-9 {
            return bad(format!(
                "tool {} has teeth {} mm apart along {} mm: it cuts that pitch, not {pitch} mm",
                tool.number, g.thread_pitch, g.flute_length
            ));
        }
        if !(self.diameter > 0.0 && self.diameter <= MAX_COORDINATE) {
            return bad("the thread's major diameter must be above 0".into());
        }
        let cut = THREAD_DEPTH * pitch;
        let minor = self.diameter - 2.0 * cut;
        if !(g.diameter > 0.0 && g.diameter < minor - 1e-9) {
            return bad(format!(
                "tool {} ({} mm across) does not go into a hole drilled for this thread ({minor:.3} mm): use a narrower thread mill",
                tool.number, g.diameter
            ));
        }
        if !(self.depth > 0.0 && self.depth <= MAX_COORDINATE) {
            return bad("the thread's depth must be above 0".into());
        }
        if !self.top.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the holes' top must be finite and within {MAX_COORDINATE} mm"));
        }
        let passes = self.passes.unwrap_or(DEFAULT_THREAD_PASSES);
        if !(1..=MAX_THREAD_PASSES).contains(&passes) {
            return bad(format!("a thread is cut in 1 to {MAX_THREAD_PASSES} passes"));
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        // The tool goes a quarter of a pitch below the thread to come on to its circle.
        let reach = if g.shoulder_length > 0.0 { g.shoulder_length } else { tool.holder.stick_out };
        if !(self.depth + 0.25 * pitch <= reach + 1e-9) {
            return bad(format!(
                "the thread is {} mm deep and the tool starts a quarter of a pitch below it, more than the {reach} mm tool {} reaches down a hole",
                self.depth, tool.number
            ));
        }
        let span = rows * pitch;
        if !((self.depth / span).ceil() <= MAX_THREAD_TURNS) {
            return bad(format!("the thread would take more than {MAX_THREAD_TURNS} turns a pass"));
        }
        Ok(Shape { pitch, cut, radius: 0.5 * (self.diameter - g.diameter), span, passes })
    }

    /// The diameter the hole is drilled to for this thread with `tool`, mm: its minor diameter.
    pub fn minor(&self, tool: &Tool) -> Result<f64> {
        let shape = self.shape(tool)?;
        Ok(self.diameter - 2.0 * shape.cut)
    }

    /// The toolpath from `start` (where the tool is; `None` for above the first hole, at the
    /// clearance height) for `tool` in a setup placed at `placement`, with its stock `material`.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>) -> Result<Threaded> {
        let shape = self.shape(tool)?;
        let g = tool.geometry;
        let top = self.top.unwrap_or(placement.model.max.z);
        let bottom = top - self.depth;
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the holes' top ({top} mm)"));
        }
        let pitch = shape.pitch;
        // The passes: the thread's depth in parts of equal area, which for a V is the square
        // root of the share (two passes: 70.7% and all of it).
        let passes: Vec<f64> =
            (1..=shape.passes).map(|k| shape.radius - (1.0 - (f64::from(k) / f64::from(shape.passes)).sqrt()) * shape.cut).collect();
        let turns = (self.depth / shape.span - 1e-9).ceil().max(1.0) as usize;
        // The tool's own chip load, at the edge; the centre goes round a smaller circle than the
        // edge at the thread's root, and its feed is that much less.
        let f = op_feeds(tool, Engagement { axial: shape.span.min(self.depth).max(1e-4), radial: g.diameter }, material, self.rpm, self.feed)?;
        let centre_feed = f.feed * (self.diameter - g.diameter) / self.diameter;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
        let Some(first) = self.holes.first().map(|h| Vec2 { x: h[0], y: h[1] }) else { return bad("there are no holes to thread".into()) };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        // A right-hand thread rises as the tool goes round counter-clockwise; a left-hand one falls.
        let up = if self.hand == ThreadHand::Right { 1.0 } else { -1.0 };
        for hole in &self.holes {
            let c = Vec2 { x: hole[0], y: hole[1] };
            moves.push(Move::Rapid { to: at(c, retract) });
            for &radius in &passes {
                // The half turns on to the circle and off it, about the point half way out.
                let half = Vec2 { x: c.x + 0.5 * radius, y: c.y };
                let edge = Vec2 { x: c.x + radius, y: c.y };
                let on = Seg2::Arc { center: half, radius: 0.5 * radius, start: PI, sweep: PI };
                let round = Seg2::Arc { center: c, radius, start: 0.0, sweep: TAU };
                let off = Seg2::Arc { center: half, radius: 0.5 * radius, start: 0.0, sweep: PI };
                for turn in 0..turns {
                    // The stretch of thread this turn cuts, from the bottom up (from the top
                    // down for a left-hand thread), and where the tool's tip is as it goes: a
                    // quarter of a pitch before the turn coming on, one pitch round, a quarter off.
                    let stretch = if up > 0.0 { turn } else { turns - 1 - turn };
                    let low = bottom + stretch as f64 * shape.span;
                    let begin = if up > 0.0 { low } else { low + pitch };
                    let (z0, z1, z2, z3) = (begin - up * 0.25 * pitch, begin, begin + up * pitch, begin + up * 1.25 * pitch);
                    // Down (or up) the hole's middle, in its air, to where the half turn begins.
                    moves.push(Move::Linear { to: at(c, z0), feed: f.plunge_feed });
                    cut(&mut moves, &on, c, edge, (z0, z1), centre_feed);
                    cut(&mut moves, &round, edge, edge, (z1, z2), centre_feed);
                    cut(&mut moves, &off, edge, c, (z2, z3), centre_feed);
                }
            }
            moves.push(Move::Rapid { to: at(c, retract) });
        }
        let last = self.holes.last().map_or(first, |h| Vec2 { x: h[0], y: h[1] });
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;
        let mut notes = f.notes;
        notes.push(format!(
            "the program's feed is the tool's centre's, {centre_feed:.1} mm/min: its circle is {:.3} mm across where the cutting edge's is {} mm, and the edge is fed {:.1}",
            self.diameter - g.diameter,
            self.diameter,
            f.feed
        ));
        notes.push(format!(
            "the holes have to be drilled to {:.3} mm and {:.3} mm deeper than the thread: the tool starts a quarter of a pitch below it",
            self.diameter - 2.0 * shape.cut,
            0.25 * pitch
        ));
        Ok(Threaded {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            centre_feed,
            plunge_feed: f.plunge_feed,
            top,
            bottom,
            pitch,
            minor: self.diameter - 2.0 * shape.cut,
            passes,
            turns,
            notes,
        })
    }
}

#[cfg(test)]
#[path = "thread_tests.rs"]
mod tests;
