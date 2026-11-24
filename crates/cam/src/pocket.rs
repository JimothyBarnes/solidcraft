//! The pocket operation (decision 0014 of the CAM work): a closed pocket cleared in rings from
//! the inside out. It reads the setup's placement, the tool, and each pocket as the engine reads
//! it from the model: its floor's height and the region of air just above its floor. It keeps no
//! geometry of its own but a point on each pocket's floor.
//!
//! The tool's centre may be anywhere in the region shrunk by its radius and the stock to leave.
//! That region, and the pocket's shrunk by one stepover more, two, and so on, give the rings
//! (each from the verified offsetter, each from the pocket's own outline: shrinking what was
//! already shrunk would compound its last bits). A ring is cut after the ring inside it and reached from that one by
//! the shortest way, one stepover long, which lies in the region because every point of a ring
//! is one stepover from the ring outside it. The innermost rings are entered down a ramp along
//! themselves, so the tool never plunges into stock and never needs room it doesn't have.
//!
//! When a finishing pass is asked for (decision 0031), the rings keep that much more from the
//! walls, and after the last level the tool goes along the pocket's wall and round each island,
//! taking it off: down in air the rings cut, on to the wall by a quarter turn and off it by
//! another, at the rings' feed, and at half of it for the last stretch into each corner. It
//! goes down the walls in as few steps as the flutes allow, and so that no moment of it is a
//! bigger cut than a ring's; where the rings did not go (too narrow with that stock on the
//! walls) it is a cut as wide as the tool, and it is taken level by level as they are.

use std::f64::consts::{FRAC_PI_2, PI};

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::{Loop2Geom, Region2Geom, Seg2Geom};

use crate::contour::{
    CutDirection, DEFAULT_LEAD_RADIUS, MAX_LEAVE, MIN_STEP, Round, SAME_HEIGHT, apart, bad, cut, moved_at_least, round_within, way_is_clear,
};
use crate::feeds::op_feeds;
use crate::offset::JOINT_TOLERANCE;
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most pockets one operation may cut.
pub const MAX_POCKETS: usize = 1_000;
/// The most levels a pocket may take.
pub const MAX_POCKET_LEVELS: usize = 1_000;
/// The most rings one pocket may be cleared in at a level.
pub const MAX_POCKET_RINGS: usize = 5_000;
/// The stepover when the operation gives none, as a share of the tool's diameter.
pub const DEFAULT_POCKET_STEPOVER: f64 = 0.4;
/// The most a stepover may be, as a share of the tool's diameter: at half or less, no corner of
/// any angle keeps an uncut sliver between two rings.
pub const MAX_POCKET_STEPOVER: f64 = 0.5;
/// The most one level takes off when the operation gives no stepdown, as a share of the tool's
/// diameter: the ramp and the first ring are cuts as wide as the tool.
pub const DEFAULT_POCKET_STEPDOWN: f64 = 0.5;
/// The angle the tool goes down the innermost ring at when the operation gives none, radians (3°).
pub const DEFAULT_RAMP_ANGLE: f64 = 3.0 * std::f64::consts::PI / 180.0;
/// The shallowest and steepest ramps, radians (0.5° and 30°).
pub(crate) const RAMP_ANGLES: (f64, f64) = (0.5 * std::f64::consts::PI / 180.0, 30.0 * std::f64::consts::PI / 180.0);
/// The most turns a ramp may take down one level.
pub(crate) const MAX_RAMP_TURNS: f64 = 500.0;
/// How far short of the next ring a move between rings is checked: it ends on that ring.
const LINK_SLACK: f64 = 1e-5;
/// How far apart at most the places along a finishing pass are where its distance from what
/// the rings cleared is measured, mm (and no further apart than half the stock it takes): what
/// lies between two of them is at most half of that further.
const FINISH_STEP: f64 = 0.5;
/// The most such places along one loop of a finishing pass: a longer loop has them further apart.
const FINISH_PLACES_MOST: f64 = 20_000.0;
/// How far from what the rings cleared a finishing pass may be in a corner, as a multiple of
/// the stock it takes: 1.41 of it in a right-angled corner, four times it in one as sharp as
/// 29 degrees. Further, the pass is taken to be a cut as wide as the tool.
const FINISH_REACH: f64 = 4.0;
/// How far from what the rings cleared a finishing pass may be anywhere else, as a multiple of
/// the stock it takes: along a wall it is that stock from them. More than this away from a
/// corner is a place the rings did not go (a gap too narrow for them with the stock on both
/// sides), where the pass is a cut as wide as the tool.
const FINISH_PLAIN: f64 = 1.1;
/// The least turn at a joint of a finishing pass that is a corner the pass slows into, radians
/// (15 degrees).
const FINISH_CORNER_TURN: f64 = 15.0 * std::f64::consts::PI / 180.0;
/// The share of its feed a finishing pass keeps for the last stretch into a corner, where the
/// tool cuts ahead of itself as well as beside: a tool maker asks for 30 to 50% less there
/// (`research/pocket-milling.md`).
const FINISH_CORNER_FEED: f64 = 0.5;
/// The deepest one step of a finishing pass is, as a multiple of the tool's diameter: deeper,
/// the tool maker asks for half the feed.
const FINISH_DEEPEST: f64 = 3.0;
/// How much further than the stepover a ring may be from the ring outside it, mm: the
/// offsetter's own tolerance each way, the micron more a ring is moved in by when exactly its
/// distance can't be verified, and rounding.
const RING_SLACK: f64 = 5e-3;

/// A pocket as it is read from the model, in the setup's WCS.
#[derive(Clone, Debug, PartialEq)]
pub struct PocketShape {
    /// The Z of its floor.
    pub floor: f64,
    /// The air just above its floor: the wall's outline, with the islands as holes.
    pub region: Region2,
}

/// A pocket operation's parameters, as the design keeps them.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PocketOp {
    /// A point on the floor of each pocket to cut, X and Y in the setup's WCS, mm: the model is
    /// read there each time the toolpath is worked out.
    pub pockets: Vec<[f64; 2]>,
    /// The Z the cut starts from; `None` for the stock's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// The most one level takes off, mm; `None` for half the tool's diameter.
    #[serde(default)]
    pub stepdown: Option<f64>,
    /// How far apart the rings are, mm; `None` for 40% of the tool's diameter.
    #[serde(default)]
    pub stepover: Option<f64>,
    /// Stock left on the walls, mm; `None` for none.
    #[serde(default)]
    pub leave: Option<f64>,
    /// Stock the rings leave on the walls for one last pass along them to take, mm; `None` or 0
    /// for no finishing pass.
    #[serde(default)]
    pub finish: Option<f64>,
    #[serde(default)]
    pub direction: CutDirection,
    /// The angle the tool goes down the innermost ring at, radians; `None` for 3 degrees.
    #[serde(default)]
    pub ramp_angle: Option<f64>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A pocket operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Pocketed {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    pub ramp_feed: f64,
    /// The Z the cut starts from.
    pub top: f64,
    /// The Z of each level of each pocket, top down.
    pub levels: Vec<Vec<f64>>,
    /// The stepdown and the stepover used, mm.
    pub stepdown: f64,
    pub stepover: f64,
    /// How far the rings keep the tool's centre from the walls, mm: its radius, the stock left
    /// and the stock a finishing pass takes.
    pub offset: f64,
    /// The rings each pocket is cleared in at a level: the length of each, in the order cut.
    pub rings: Vec<Vec<f64>>,
    /// The stock the rings left on the walls for the finishing pass, mm: 0 when there is none.
    pub finish: f64,
    /// The Z of each step of each pocket's finishing pass, top down: none without one.
    pub finish_levels: Vec<Vec<f64>>,
    /// The loops each pocket's finishing pass goes round: the length of each, in the order cut.
    pub finishing: Vec<Vec<f64>>,
    /// The radius of the quarter turns on to each of those loops and off it, mm: 0 where the
    /// loop is met straight across the stock.
    pub finish_leads: Vec<Vec<f64>>,
    /// Things worth showing beside the toolpath (a capped spindle speed).
    pub notes: Vec<String>,
}

/// One closed pass of the tool's centre, ready to cut.
struct Ring {
    /// Where it starts and ends.
    start: Vec2,
    /// From `start` back to it, the way it is cut.
    segs: Vec<Seg2>,
    /// Whether the ring cut after it is the ring outside it, reached straight from this one;
    /// when not, the tool lifts after it.
    leads_on: bool,
    /// Whether it is reached from above, down a ramp along itself: nothing leads to it.
    entered: bool,
}

impl Ring {
    fn round(&self) -> f64 {
        self.segs.iter().map(Seg2Geom::length).sum()
    }
}

/// A pocket's finishing pass.
struct Finishing {
    /// One round along the pocket's wall and one round each island, in the order cut. A round's
    /// entry is in air the rings cut; with quarter turns it is where the first begins, without
    /// them a point of the outermost ring, straight across the stock from the round's start.
    rounds: Vec<Round>,
    /// How wide the widest cut of the pass is for a moment, mm, counted as a cut at the whole
    /// feed: the stock it takes along a wall, more where the wall curves round the tool, and
    /// in a corner the width the tool is buried to there, halved as the feed is.
    moment: f64,
    /// Whether the pass is somewhere a cut as wide as the tool: out of reach of where the
    /// rings went, or further from it than a wall or a corner accounts for.
    wide: bool,
    /// Whether the pass goes everywhere: where the tool's centre may be along the walls is one
    /// piece, with a way round every island.
    everywhere: bool,
}

impl PocketOp {
    /// Checks the parameters and that `tool` can cut a pocket.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        if self.pockets.is_empty() || self.pockets.len() > MAX_POCKETS {
            return bad(format!("a pocket operation cuts 1 to {MAX_POCKETS} pockets, not {}", self.pockets.len()));
        }
        if !self.pockets.iter().all(|p| p.iter().all(|v| v.abs() <= MAX_COORDINATE)) {
            return bad(format!("the points on the pockets' floors must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.top.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the top must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.stepdown.is_none_or(|v| (MIN_STEP..=MAX_COORDINATE).contains(&v)) {
            return bad(format!("the stepdown must be at least {MIN_STEP} mm"));
        }
        let diameter = tool.geometry.diameter;
        if !self.stepover.is_none_or(|v| v >= MIN_STEP && v <= MAX_POCKET_STEPOVER * diameter + 1e-9) {
            return bad(format!(
                "the stepover must be {MIN_STEP} mm to {} mm, half of tool {}'s diameter: a wider one can leave uncut corners between the rings",
                MAX_POCKET_STEPOVER * diameter,
                tool.number
            ));
        }
        if !self.leave.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the stock to leave must be 0 to {MAX_LEAVE} mm"));
        }
        if !self.finish.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the finishing pass must take 0 to {MAX_LEAVE} mm"));
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
            other => bad(format!("tool {} ({other:?}) does not cut a pocket: use a flat or bull-nose end mill", tool.number)),
        }
    }

    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down, at
    /// the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, in `shapes`: the pockets as read from the model, one for each of the
    /// operation's points, in their order.
    pub fn toolpath(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        shapes: &[PocketShape],
    ) -> Result<Pocketed> {
        self.validate(tool)?;
        if shapes.len() != self.pockets.len() {
            return bad(format!("{} pockets were read from the model for the operation's {} points", shapes.len(), self.pockets.len()));
        }
        let g = tool.geometry;
        if !(g.diameter > 0.0 && g.diameter <= MAX_COORDINATE) {
            return bad(format!("tool {}'s diameter must be above 0", tool.number));
        }
        let top = self.top.unwrap_or(placement.stock.max.z);
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the pocket's top ({top} mm)"));
        }
        let radius = 0.5 * g.diameter;
        // Along the walls the tool's centre ends up its radius and the stock to leave from them;
        // the rings keep the stock of the finishing pass further off.
        let finish = self.finish.unwrap_or(0.0);
        let wall = radius + self.leave.unwrap_or(0.0);
        let offset = wall + finish;
        let stepover = self.stepover.unwrap_or(DEFAULT_POCKET_STEPOVER * g.diameter);
        let stepdown = self.stepdown.unwrap_or(DEFAULT_POCKET_STEPDOWN * g.diameter);
        // The quarter turns on to a finished wall and off it: as a contour's, and wide enough
        // to begin in air the rings cut.
        let lead = (DEFAULT_LEAD_RADIUS * g.diameter).max(2.0 * finish);
        let reach = if g.shoulder_length > 0.0 { g.shoulder_length } else { g.flute_length };
        let angle = self.ramp_angle.unwrap_or(DEFAULT_RAMP_ANGLE);

        // Each pocket's levels and rings, and its finishing pass, before any move is written.
        let mut plans: Vec<(Vec<f64>, Vec<Ring>)> = Vec::with_capacity(shapes.len());
        let mut finishes: Vec<(Vec<f64>, Vec<Round>)> = Vec::with_capacity(shapes.len());
        let mut said: Vec<String> = Vec::new();
        let mut deepest = 0.0_f64;
        for (shape, at) in shapes.iter().zip(&self.pockets) {
            let what = format!("the pocket at X {} Y {}", at[0], at[1]);
            let depth = top - shape.floor;
            if !(depth > SAME_HEIGHT) {
                return bad(format!("{what}: its floor (Z {} mm) is not below the top (Z {top} mm)", shape.floor));
            }
            if !(depth <= reach + 1e-9) {
                return bad(format!(
                    "{what} is {depth} mm deep, more than the {reach} mm tool {} reaches down a wall: use a longer tool",
                    tool.number
                ));
            }
            let count = (depth / stepdown - 1e-9).ceil().max(1.0);
            if !(count <= MAX_POCKET_LEVELS as f64) {
                return bad(format!("{what}: {depth} mm in steps of {stepdown} mm takes more than {MAX_POCKET_LEVELS} levels"));
            }
            let count = count as usize;
            let per_level = depth / count as f64;
            if !(per_level <= g.flute_length + 1e-9) {
                return bad(format!(
                    "{what}: each level is {per_level} mm deep, past tool {}'s {} mm of flutes: give the operation a smaller stepdown",
                    tool.number, g.flute_length
                ));
            }
            deepest = deepest.max(per_level);
            let levels: Vec<f64> = (1..=count).map(|k| if k == count { shape.floor } else { top - per_level * k as f64 }).collect();
            // Where the tool's centre may be is the region less the tool's radius and the stock
            // to leave; the rings are that and what is left of it a stepover in at a time.
            let (rings, everywhere) = rings(&shape.region, offset, stepover, self.direction).map_err(|e| Error::BadValue(format!("{what}: {e}")))?;
            if rings.is_empty() {
                let widest = 2.0 * shape.region.inradius(1e-4);
                let with = if finish > 0.0 { format!(" with {finish} mm left on its walls for the finishing pass") } else { String::new() };
                return bad(format!(
                    "tool {} ({} mm across) does not fit in {what}{with}, which is about {widest:.2} mm across at its widest: use a narrower tool",
                    tool.number, g.diameter
                ));
            }
            // The finishing pass goes down the walls in as few steps as three things allow.
            // The flutes: no step is deeper than they are long. The tool maker: none deeper
            // than three diameters. And the rings: no moment of the pass is a bigger cut than a
            // ring's, which is as wide as the tool and one level deep. Along a wall the pass is
            // as wide as the stock it takes; in a corner the tool is buried far wider for a
            // moment (at half its feed); where the rings did not go it is as wide as the tool,
            // and then its steps are the rings' levels.
            let everywhere = if finish > 0.0 {
                let pass =
                    finishing(&shape.region, radius, wall, finish, lead, self.direction).map_err(|e| Error::BadValue(format!("{what}: {e}")))?;
                let width = if pass.wide { g.diameter } else { pass.moment.min(g.diameter) };
                let by_load = depth * width / (g.diameter * per_level);
                let steps = (depth / g.flute_length - 1e-9)
                    .ceil()
                    .max((depth / (FINISH_DEEPEST * g.diameter) - 1e-9).ceil())
                    .max((by_load - 1e-9).ceil())
                    .clamp(1.0, count as f64) as usize;
                if pass.wide {
                    said.push(format!(
                        "the rings do not go everywhere in {what} with {finish} mm left on its walls: there the finishing pass is a cut as wide as tool {}, so it goes down level by level as they do",
                        tool.number
                    ));
                }
                let heights: Vec<f64> = (1..=steps).map(|k| if k == steps { shape.floor } else { top - depth * k as f64 / steps as f64 }).collect();
                finishes.push((heights, pass.rounds));
                pass.everywhere
            } else {
                finishes.push((Vec::new(), Vec::new()));
                everywhere
            };
            if !everywhere {
                said.push(format!(
                    "tool {} does not pass everywhere in {what}: where it is too narrow for the tool the material stays (a narrower tool clears it)",
                    tool.number
                ));
            }
            // The ramp down the innermost rings: whole turns, no steeper than the angle asked.
            for ring in rings.iter().filter(|ring| ring.entered) {
                let round = ring.round();
                let turns = (per_level / angle.tan() / round).ceil();
                if !(turns <= MAX_RAMP_TURNS) {
                    return bad(format!(
                        "{what}: its innermost ring is {round:.3} mm round, so the ramp down {per_level} mm would take {turns} turns: use a narrower tool or a steeper ramp"
                    ));
                }
            }
            plans.push((levels, rings));
        }
        // The feed of a cut as wide as the tool, which the ramp and the first ring are. The
        // finishing pass is fed the same, and half of it into each corner: along a wall it is
        // a far lighter cut, but in a corner of the pocket the tool for a moment cuts ahead of
        // itself as well as beside, and a feed worked out for the light cut would be several
        // times too much there.
        let f = op_feeds(tool, Engagement { axial: deepest.max(1e-4), radial: g.diameter }, material, self.rpm, self.feed)?;
        if finishes.iter().any(|(_, rounds)| rounds.iter().any(|r| r.leads.is_none())) {
            said.push(
                "there is no room for the quarter turns on to a wall and off it: the finishing pass meets it straight across the stock there".into(),
            );
        }
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };

        let Some(first) = plans.first().and_then(|(_, rings)| rings.first()).map(|ring| ring.start) else {
            return bad("there is nothing to cut".into());
        };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for ((levels, rings), (heights, passes)) in plans.iter().zip(&finishes) {
            let mut above = top;
            for &z in levels {
                // The rings come innermost first, each before the ring outside it.
                for ring in rings {
                    if ring.entered {
                        // Down to the level above in the air it left, then down the ring itself.
                        moves.push(Move::Rapid { to: at(ring.start, retract) });
                        moves.push(Move::Linear { to: at(ring.start, above), feed: f.plunge_feed });
                        ramp(&mut moves, ring.start, &ring.segs, above, z, angle, f.ramp_feed);
                    } else {
                        // From the ring inside, the shortest way: one stepover, in the region.
                        moves.push(Move::Linear { to: at(ring.start, z), feed: f.feed });
                    }
                    let mut from = ring.start;
                    for (k, seg) in ring.segs.iter().enumerate() {
                        let to = if k + 1 == ring.segs.len() { ring.start } else { seg.end() };
                        cut(&mut moves, seg, from, to, (z, z), f.feed);
                        from = to;
                    }
                    // On to the ring outside when that is the next one; up otherwise.
                    if !ring.leads_on {
                        moves.push(Move::Rapid { to: at(ring.start, retract) });
                    }
                    last = ring.start;
                }
                above = z;
            }
            // The finishing pass, once the pocket is cleared to its floor. Each round: down in
            // air the rings cut (they have been there at every level), on to the wall by a
            // quarter turn, round (slower into each corner), and off by another before the tool
            // lifts; straight across the stock and back where there is no room for the turns.
            for &z in heights {
                for r in passes {
                    moves.push(Move::Rapid { to: at(r.entry, retract) });
                    moves.push(Move::Linear { to: at(r.entry, z), feed: f.plunge_feed });
                    last = match &r.leads {
                        Some((on, off)) => {
                            cut(&mut moves, on, r.entry, r.start, (z, z), f.feed);
                            finish_round(&mut moves, r, z, finish, f.feed);
                            cut(&mut moves, off, r.start, off.end(), (z, z), f.feed);
                            off.end()
                        }
                        None => {
                            moves.push(Move::Linear { to: at(r.start, z), feed: f.feed });
                            finish_round(&mut moves, r, z, finish, f.feed);
                            moves.push(Move::Linear { to: at(r.entry, z), feed: f.feed });
                            r.entry
                        }
                    };
                    moves.push(Move::Rapid { to: at(last, retract) });
                }
            }
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;
        Ok(Pocketed {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            ramp_feed: f.ramp_feed,
            top,
            levels: plans.iter().map(|(levels, _)| levels.clone()).collect(),
            stepdown,
            stepover,
            offset,
            rings: plans.iter().map(|(_, rings)| rings.iter().map(Ring::round).collect()).collect(),
            finish,
            finish_levels: finishes.iter().map(|(heights, _)| heights.clone()).collect(),
            finishing: finishes.iter().map(|(_, rounds)| rounds.iter().map(|r| r.segs.iter().map(Seg2Geom::length).sum()).collect()).collect(),
            finish_leads: finishes.iter().map(|(_, rounds)| rounds.iter().map(Round::lead).collect()).collect(),
            notes: f.notes.into_iter().chain(said).collect(),
        })
    }
}

/// The moves down a closed path (`segs`, from `start` back to it) from the height `above` to
/// `z`: whole turns, falling evenly along its length, no steeper than `angle`.
pub(crate) fn ramp(moves: &mut Vec<Move>, start: Vec2, segs: &[Seg2], above: f64, z: f64, angle: f64, feed: f64) {
    let round: f64 = segs.iter().map(Seg2Geom::length).sum();
    let turns = ((above - z) / angle.tan() / round).ceil().clamp(1.0, MAX_RAMP_TURNS);
    let whole = turns * round;
    let height = |along: f64| if along >= whole { z } else { above - (above - z) * along / whole };
    let mut along = 0.0;
    let mut from = start;
    for turn in 0..turns as usize {
        for (k, seg) in segs.iter().enumerate() {
            let closes = k + 1 == segs.len();
            let to = if closes { start } else { seg.end() };
            let next = if closes && turn + 1 == turns as usize { whole } else { along + seg.length() };
            cut(moves, seg, from, to, (height(along), height(next)), feed);
            (along, from) = (next, to);
        }
    }
}

/// The nearest point of `lp` to `p`: which segment, where along it, the point and how far.
fn nearest(lp: &Loop2, p: Vec2) -> Option<(usize, f64, Vec2, f64)> {
    lp.segs
        .iter()
        .enumerate()
        .map(|(i, seg)| {
            let t = seg.closest_param(p);
            let q = seg.point_at(t);
            (i, t, q, apart(p, q))
        })
        .min_by(|a, b| a.3.total_cmp(&b.3))
}

/// `lp` as the same closed path starting (and ending) at the point `t` along its segment `index`.
fn started_at(lp: &Loop2, index: usize, t: f64) -> Vec<Seg2> {
    let Some(seg) = lp.segs.get(index) else { return lp.segs.clone() };
    let rest = |from: usize| lp.segs.iter().skip(from).chain(lp.segs.iter().take(from)).copied();
    // At an end of the segment the path starts on a joint and nothing is split.
    if t * seg.length() <= JOINT_TOLERANCE {
        return rest(index).collect();
    }
    if (1.0 - t) * seg.length() <= JOINT_TOLERANCE {
        return rest((index + 1) % lp.segs.len()).collect();
    }
    let (before, after) = seg.split_at(t);
    let mut segs = vec![after];
    segs.extend(lp.segs.iter().skip(index + 1).chain(lp.segs.iter().take(index)).copied());
    segs.push(before);
    segs
}

/// How far the way a path goes turns from the end of `seg` to the start of `next`, radians: 0
/// (straight on) to pi.
fn turn_between(seg: &Seg2, next: &Seg2) -> f64 {
    let (a, b) = (seg.end_tangent(), next.start_tangent());
    (a.x * b.y - a.y * b.x).abs().atan2(a.x * b.x + a.y * b.y)
}

/// How long the stretch of a finishing pass of `finish` mm is, either side of a corner it turns
/// by `turn` at, that the corner accounts for, mm: four times the stock at most. Up to a right
/// angle it is where the tool is in the stock of the wall ahead as well, `finish / sin(turn)`:
/// the stock itself at a right angle, more for a shallower turn. Past a right angle it is where
/// the pass is beyond the corner the rings turned at, `finish * tan(turn / 2)`: the sharper the
/// corner, the sooner they turned.
fn corner_stretch(finish: f64, turn: f64) -> f64 {
    finish * if turn <= FRAC_PI_2 { 1.0 / turn.sin().max(0.25) } else { (0.5 * turn).tan().clamp(1.0, 4.0) }
}

/// How wide a cut a finishing pass of `finish` mm is where the wall curves round the tool, mm:
/// the centre of the tool (of `radius`) on an arc of radius `bend`, the wall that much outside
/// it. The tool's edge meets the stock's face where it is `bend + radius - finish` from the
/// arc's centre; the triangle of that point, the arc's centre and the tool's centre gives the
/// angle the edge is in the stock over, and the width is that of the angle across the feed.
/// (A straight wall is the limit: there the width is `finish`.)
fn round_a_bend(radius: f64, finish: f64, bend: f64) -> f64 {
    if !(bend > 1e-9) {
        return 2.0 * radius;
    }
    let cos = (((bend + radius - finish).powi(2) - bend * bend - radius * radius) / (2.0 * bend * radius)).clamp(-1.0, 1.0);
    radius * (1.0 - cos)
}

/// The cutting moves once round `r`, a round of a finishing pass of `finish` mm, at the height
/// `z` from its start back to it: at `feed`, but for the last stretch into each corner
/// ([`corner_stretch`]), which has [`FINISH_CORNER_FEED`] of it.
fn finish_round(moves: &mut Vec<Move>, r: &Round, z: f64, finish: f64, feed: f64) {
    let mut from = r.start;
    for (k, seg) in r.segs.iter().enumerate() {
        let to = if k + 1 == r.segs.len() { r.start } else { seg.end() };
        let turn = r.segs.get((k + 1) % r.segs.len()).map_or(0.0, |next| turn_between(seg, next));
        let (length, slow) = (seg.length(), if turn >= FINISH_CORNER_TURN { corner_stretch(finish, turn) } else { 0.0 });
        if slow > 0.0 && length > slow + JOINT_TOLERANCE {
            let (fast, last) = seg.split_at(1.0 - slow / length);
            cut(moves, &fast, from, fast.end(), (z, z), feed);
            cut(moves, &last, fast.end(), to, (z, z), FINISH_CORNER_FEED * feed);
        } else {
            cut(moves, seg, from, to, (z, z), if slow > 0.0 { FINISH_CORNER_FEED * feed } else { feed });
        }
        from = to;
    }
}

/// The finishing pass of the pocket `region` for a tool of `radius` whose centre ends up
/// `wall` from its walls, after rings that kept `finish` more: one round along the pocket's
/// wall and one round each island.
///
/// A round starts at the middle of its longest straight side, met by a quarter turn of `lead`
/// (or a half or a quarter of it, where that is what fits) and left by another, when the first
/// begins in air the rings cut. Else it starts where it is nearest the outermost ring and is
/// met straight across the stock from there.
///
/// How wide a cut the pass is, is worked out along it. Along a wall it is as wide as the stock
/// it takes. Where the wall curves round the tool it is wider ([`round_a_bend`]). In a corner
/// it turns by `turn` at, the tool's edge is in the stock over that angle more than along a
/// wall, for the moment before it turns. And its distance from where the rings went is
/// measured at places no further apart than [`FINISH_STEP`]: where that is more than a wall
/// or a corner accounts for, the rings did not go (too narrow with the finishing stock on the
/// walls, or a corner sharper than 29 degrees), and the pass is a cut as wide as the tool there.
fn finishing(region: &Region2, radius: f64, wall: f64, finish: f64, lead: f64, direction: CutDirection) -> Result<Finishing> {
    let along = moved_at_least(std::slice::from_ref(region), wall)?;
    let cleared = moved_at_least(std::slice::from_ref(region), wall + finish)?;
    let outermost: Vec<&Loop2> = cleared.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).collect();
    let (reach, plain) = (FINISH_REACH * finish + RING_SLACK, FINISH_PLAIN * finish + RING_SLACK);
    // The angle the tool's edge is in the stock over along a straight wall.
    let straight = (1.0 - finish / radius).clamp(-1.0, 1.0).acos();
    let everywhere = along.len() == 1 && along.first().is_some_and(|whole| whole.holes.len() == region.holes.len());
    let mut rounds = Vec::new();
    let (mut moment, mut wide) = (finish.min(2.0 * radius), false);
    for part in &along {
        for hole in std::iter::once(None).chain((0..part.holes.len()).map(Some)) {
            let lp = hole.and_then(|k| part.holes.get(k)).unwrap_or(&part.outer);
            let sides = lp.segs.len();
            let step = FINISH_STEP.min(0.5 * finish).max(lp.segs.iter().map(Seg2Geom::length).sum::<f64>() / FINISH_PLACES_MOST);
            // The place of the loop nearest the outermost ring: which side, where along it, the
            // point, the ring's point and how far apart they are.
            let mut place: Option<(usize, f64, Vec2, Vec2, f64)> = None;
            for (index, seg) in lp.segs.iter().enumerate() {
                // How far the loop turns at this side's two ends.
                let before = lp.segs.get((index + sides - 1) % sides).map_or(0.0, |last| turn_between(last, seg));
                let after = lp.segs.get((index + 1) % sides).map_or(0.0, |next| turn_between(seg, next));
                // The cut for the moment before the turn at its end (a corner has half the feed).
                let buried = radius * (1.0 - (after + straight).min(PI).cos());
                moment = moment.max(if after >= FINISH_CORNER_TURN { FINISH_CORNER_FEED * buried } else { buried });
                // The cut where the wall curves round the tool: an arc the loop turns left on,
                // its air being on its left.
                if let Seg2::Arc { radius: bend, sweep, .. } = *seg
                    && sweep > 0.0
                {
                    moment = moment.max(round_a_bend(radius, finish, bend));
                }
                let length = seg.length();
                let places = (length / step).ceil().clamp(1.0, FINISH_PLACES_MOST);
                for k in 0..places as usize {
                    let t = k as f64 / places;
                    let p = seg.point_at(t);
                    let Some((_, _, q, far)) = outermost.iter().filter_map(|ring| nearest(ring, p)).min_by(|a, b| a.3.total_cmp(&b.3)) else {
                        return bad("there is no ring to reach the finishing pass from".into());
                    };
                    // Further from where the rings went than along a wall: a corner's doing
                    // within the corner's stretch (to a limit), and elsewhere a place the
                    // rings did not go.
                    let cornered = (before >= FINISH_CORNER_TURN && t * length <= corner_stretch(finish, before) + JOINT_TOLERANCE)
                        || (after >= FINISH_CORNER_TURN && (1.0 - t) * length <= corner_stretch(finish, after) + JOINT_TOLERANCE);
                    wide |= !(far <= if cornered { reach } else { plain });
                    if place.is_none_or(|(.., least)| far < least) {
                        place = Some((index, t, p, q, far));
                    }
                }
            }
            let round = round_within(part, hole, direction, lead, true);
            if round.leads.is_some() && cleared.iter().any(|c| c.contains(round.entry)) {
                rounds.push(round);
                continue;
            }
            let Some((index, t, start, from, far)) = place else { continue };
            if !(far <= reach) {
                return bad(format!(
                    "with {finish} mm left on the walls for the finishing pass the rings do not go into a part of it that the tool fits in, and the pass has nowhere to start there: leave the finishing pass out, or use a narrower tool"
                ));
            }
            let way = Vec2 { x: (start.x - from.x) / far.max(1e-300), y: (start.y - from.y) / far.max(1e-300) };
            if !way_is_clear(from, way, far - LINK_SLACK, &along) {
                return bad("the way onto the finishing pass would leave the pocket: this shape is not cut reliably yet".into());
            }
            let segs = started_at(lp, index, t);
            // Climb is the way the offsetter returns them, as for the rings.
            let segs = if direction == CutDirection::Conventional { Loop2 { segs }.reversed().segs } else { segs };
            rounds.push(Round { entry: from, start, segs, leads: None });
        }
    }
    Ok(Finishing { rounds, moment, wide, everywhere })
}

/// The rings the pocket `region` is cleared in by a tool whose centre keeps `offset` from its
/// walls, `stepover` apart, in the order they are cut: each innermost ring first, then the rings
/// outside it one by one out to a wall or an island, a ring that others lead to after all of
/// them: none when the tool does not fit. And whether the tool passes everywhere: where its
/// centre may be is still one piece, with a way round every island.
///
/// Which ring leads to which is settled from the inside out. A ring nothing leads to starts
/// where its loop does; a ring others lead to starts at its nearest point to where the last of
/// them starts. Either way it leads to the ring of the next shrinking out that is nearest its
/// own start, which is one stepover away, since every point of a ring is that far from the
/// rings outside it. So every way between rings is the shortest there is; each is checked to be
/// that short and to lie in the region.
fn rings(region: &Region2, offset: f64, stepover: f64, direction: CutDirection) -> Result<(Vec<Ring>, bool)> {
    let loops_of = |regions: &[Region2]| -> Vec<Loop2> { regions.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes)).cloned().collect() };
    // Where the tool's centre may be, and every loop of every shrinking of the pocket past
    // that, by how many stepovers it is in.
    let centre = moved_at_least(std::slice::from_ref(region), offset)?;
    if centre.is_empty() {
        return Ok((Vec::new(), false));
    }
    let everywhere = centre.len() == 1 && centre.first().is_some_and(|whole| whole.holes.len() == region.holes.len());
    let centre = centre.as_slice();
    let mut levels: Vec<Vec<Loop2>> = vec![loops_of(centre)];
    let mut count = levels.first().map_or(0, Vec::len);
    for k in 1.. {
        let inner = moved_at_least(std::slice::from_ref(region), offset + stepover * k as f64)?;
        if inner.is_empty() {
            break;
        }
        let loops = loops_of(&inner);
        count += loops.len();
        if count > MAX_POCKET_RINGS {
            return bad(format!("it takes more than {MAX_POCKET_RINGS} rings at a stepover of {stepover} mm"));
        }
        levels.push(loops);
    }
    // One node a loop, numbered level by level.
    let first: Vec<usize> = levels.iter().scan(0, |at, loops| Some(std::mem::replace(at, *at + loops.len()))).collect();
    let mut parent: Vec<Option<usize>> = vec![None; count];
    let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut cut: Vec<Option<(Vec2, Vec<Seg2>)>> = vec![None; count];
    for (k, level) in levels.iter().enumerate().rev() {
        for (i, lp) in level.iter().enumerate() {
            let node = first.get(k).copied().unwrap_or(0) + i;
            // Where the last ring that leads here starts, if any does.
            let inside = children.get(node).and_then(|list| list.last()).and_then(|&c| cut.get(c)).and_then(|c| c.as_ref()).map(|(start, _)| *start);
            let (start, segs) = match inside {
                None => (lp.segs.first().map_or(Vec2 { x: 0.0, y: 0.0 }, Seg2Geom::start), lp.segs.clone()),
                Some(from) => {
                    let Some((index, t, q, far)) = nearest(lp, from) else { return bad("a ring is empty".into()) };
                    let way = Vec2 { x: (q.x - from.x) / far.max(1e-300), y: (q.y - from.y) / far.max(1e-300) };
                    if !(far <= stepover + RING_SLACK) || !way_is_clear(from, way, far - LINK_SLACK, centre) {
                        return bad(format!(
                            "the way from one ring to the next ({far} mm) would leave the pocket: this shape is not cut reliably yet"
                        ));
                    }
                    (q, started_at(lp, index, t))
                }
            };
            // It leads to the ring outside that is nearest its start.
            if let Some(outside) = k.checked_sub(1).and_then(|out| levels.get(out).zip(first.get(out))) {
                let (loops, base) = outside;
                let Some((best, _)) =
                    loops.iter().enumerate().filter_map(|(j, out)| nearest(out, start).map(|n| (j, n.3))).min_by(|a, b| a.1.total_cmp(&b.1))
                else {
                    return bad("a ring has no ring outside it".into());
                };
                if let (Some(slot), Some(list)) = (parent.get_mut(node), children.get_mut(base + best)) {
                    *slot = Some(base + best);
                    list.push(node);
                }
            }
            if let Some(slot) = cut.get_mut(node) {
                *slot = Some((start, segs));
            }
        }
    }
    // The order they are cut in: from each wall or island loop, everything that leads to it
    // first (without recursion: a pocket can be thousands of rings deep).
    let mut order: Vec<usize> = Vec::with_capacity(count);
    for root in 0..levels.first().map_or(0, Vec::len) {
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        while let Some((node, next)) = stack.pop() {
            match children.get(node).and_then(|list| list.get(next)) {
                Some(&child) => {
                    stack.push((node, next + 1));
                    stack.push((child, 0));
                }
                None => order.push(node),
            }
        }
    }
    // A ring leads on when the next one cut is the ring it leads to; a ring nothing leads to is
    // entered from above. A ring's own children come in the order they chose it, so the last of
    // them is the one cut just before it.
    let place: Vec<Option<usize>> = (0..count).map(|node| order.iter().position(|&n| n == node)).collect();
    let mut out: Vec<Ring> = Vec::with_capacity(order.len());
    let mut led = false;
    for (at, &node) in order.iter().enumerate() {
        let Some((start, segs)) = cut.get_mut(node).and_then(Option::take) else { continue };
        // Climb is the way the offsetter returns them: counter-clockwise along the wall,
        // clockwise round an island, the cleared side on the tool's left.
        let segs = if direction == CutDirection::Conventional { Loop2 { segs }.reversed().segs } else { segs };
        let outside = parent.get(node).copied().flatten().and_then(|p| place.get(p).copied().flatten());
        let leads_on = outside == Some(at + 1);
        out.push(Ring { start, segs, leads_on, entered: !led });
        led = leads_on;
    }
    Ok((out, everywhere))
}

#[cfg(test)]
#[path = "pocket_tests.rs"]
mod tests;
