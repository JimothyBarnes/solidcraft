//! The contour operation (decision 0013 of the CAM work): round the outside of the part, the
//! tool's centre on the part's outline moved out by its radius. It reads the setup's placement
//! (the stock, to go down beside it in the air), the tool, and the outline the engine takes from
//! the model; it keeps no geometry of its own.
//!
//! Or round the inside of an opening (decision 0021): a pocket's wall, a window through the
//! part, a round hole wider than the tool. The tool's centre is on the wall moved in by its
//! radius. There is nowhere beside the stock to go down, so it goes down along the wall itself,
//! on a ramp, to each level, then once round at the level; at the bottom it leaves the wall by
//! a quarter turn into the opening. Round a hole that is a helix: boring with an end mill.
//!
//! How the tool meets the wall and what holds the part (decision 0016): it comes on to the round
//! and leaves it by a quarter turn that meets it at a tangent, and tabs, when asked for, are
//! places along the round where the tool lifts over a bridge of stock left at the bottom.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::{Loop2Geom, Region2Geom, Seg2Geom};

use crate::feeds::op_feeds;
use crate::offset::{JOINT_TOLERANCE, offset_regions};
use crate::placement::Placement;
use crate::pocket::{DEFAULT_RAMP_ANGLE, MAX_RAMP_TURNS, RAMP_ANGLES, ramp};
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
/// The most passes sideways a contour takes at one level.
pub const MAX_CONTOUR_PASSES: usize = 100;
/// How much nearer the wall each pass is than the one before it when the operation gives no
/// stepover, as a share of the tool's diameter.
pub const DEFAULT_CONTOUR_STEPOVER: f64 = 0.4;
/// The most stock a contour may leave on the wall, and the most it may clear the stock by, mm.
pub(crate) const MAX_LEAVE: f64 = 1_000.0;
/// The smallest stepdown, mm.
pub(crate) const MIN_STEP: f64 = 1e-3;
/// Heights closer than this are the same height, mm.
pub(crate) const SAME_HEIGHT: f64 = 1e-6;
/// The most places tried for the way in to one loop.
const MAX_WAYS_IN: usize = 64;
/// An arc whose middle is nearer its chord than this is cut as a line, mm.
const FLAT_ARC: f64 = 1e-7;
/// A way in longer than this many tool diameters is worth a note.
const LONG_WAY_IN: f64 = 2.0;
/// How much further than asked a boundary is moved when exactly as far can't be verified, mm.
const HAIR_MORE: f64 = 1e-3;
/// The radius of the arcs the tool meets the wall and leaves it by when the operation gives
/// none, as a share of the tool's diameter.
pub const DEFAULT_LEAD_RADIUS: f64 = 0.1;
/// The arcs tried in and out of one side, as shares of the radius asked for, before a straight
/// way in.
const LEAD_SHARES: [f64; 3] = [1.0, 0.5, 0.25];
/// The most tabs round one part.
pub const MAX_TABS: usize = 1_000;
/// The most openings one inside contour goes round.
pub const MAX_CONTOUR_OPENINGS: usize = 1_000;
/// A tab's width and height when the operation gives none, mm.
pub const DEFAULT_TAB_WIDTH: f64 = 6.0;
pub const DEFAULT_TAB_HEIGHT: f64 = 2.0;
/// The smallest tab, mm, each way.
const MIN_TAB: f64 = 0.01;
/// Angles and shares of a segment closer than this are the same.
const SAME_PLACE: f64 = 1e-9;
/// The most tabs times straight sides for which the tabs' places are chosen to suit the sides;
/// past it they are simply evenly spaced.
const MAX_TAB_PLACES: usize = 200_000;

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

/// Which side of the loops it is given a contour goes round.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContourSide {
    /// Round the outside of the part, in from beside the stock.
    #[default]
    Outside,
    /// Round the inside of an opening (a pocket's wall, a window, a round hole), down a ramp
    /// along the wall.
    Inside,
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
    /// The radius of the quarter turns the tool meets the wall and leaves it by, mm; `None` for a
    /// tenth of the tool's diameter, 0 for straight in and straight out.
    #[serde(default)]
    pub lead: Option<f64>,
    /// Bridges of stock left at the bottom to hold the part; `None` for none.
    #[serde(default)]
    pub tabs: Option<Tabs>,
    /// Outside: how many passes sideways the tool takes at each level, the one furthest from
    /// the wall first and the last one on it; `None` for one.
    #[serde(default)]
    pub passes: Option<usize>,
    /// How much nearer the wall each pass is than the one before it, mm; `None` for four tenths
    /// of the tool's diameter.
    #[serde(default)]
    pub stepover: Option<f64>,
    /// Round the outside of the part, or round the inside of the openings it is given.
    #[serde(default)]
    pub side: ContourSide,
    /// Inside: a point in each opening to go round, X and Y in the setup's WCS, mm: the model
    /// is read there each time the toolpath is worked out.
    #[serde(default)]
    pub openings: Vec<[f64; 2]>,
    /// Inside: the angle the tool goes down along the wall at, radians; `None` for 3 degrees.
    #[serde(default)]
    pub ramp_angle: Option<f64>,
}

/// The tabs a contour leaves round each part it cuts: evenly spaced, kept on straight sides
/// where they can be.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tabs {
    /// How many round each part.
    pub count: usize,
    /// How wide each is at its narrowest, mm; `None` for 6.
    #[serde(default)]
    pub width: Option<f64>,
    /// How high each stands above the cut's bottom, mm; `None` for 2.
    #[serde(default)]
    pub height: Option<f64>,
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
    /// How far the tool's centre is from the outline on its last pass, mm: its radius and the
    /// stock left.
    pub offset: f64,
    /// How many passes sideways it takes at each level, and how far apart they are, mm (0 for
    /// one pass).
    pub passes: usize,
    pub stepover: f64,
    /// The length of the tool's centre path round the part at one level, all its passes, mm.
    pub round: f64,
    /// Where the tool goes down for each loop it cuts round, X and Y.
    pub entries: Vec<[f64; 2]>,
    /// The radius of the quarter turns on to each round and off it, mm: 0 where it goes straight in.
    pub leads: Vec<f64>,
    /// The middle of each tab on the tool's centre path, X and Y; and the Z of the tabs' tops.
    pub tabs: Vec<[f64; 2]>,
    pub tab_top: Option<f64>,
    /// Things worth showing beside the toolpath (a capped spindle speed, a long way in).
    pub notes: Vec<String>,
}

/// One loop the tool's centre goes round, ready to cut.
pub(crate) struct Round {
    /// Where the tool goes down: beside the stock.
    pub entry: Vec2,
    /// Where it meets the loop and leaves it.
    pub start: Vec2,
    /// The loop from `start` back to it, the way it is cut.
    pub segs: Vec<Seg2>,
    /// The quarter turns on to the loop at `start` and off it again, when there is room: the
    /// tool comes straight in from `entry` to the first one's start, and goes up at the end of
    /// the second. Without them it comes straight in to `start` and goes back out to `entry`.
    pub leads: Option<(Seg2, Seg2)>,
}

impl Round {
    /// The radius of its quarter turns, mm: 0 when it has none.
    pub fn lead(&self) -> f64 {
        match self.leads {
            Some((Seg2::Arc { radius, .. }, _)) => radius,
            _ => 0.0,
        }
    }
}

pub(crate) fn bad<T>(message: String) -> Result<T> {
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
        if !self.lead.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the radius of the arcs in and out must be 0 to {MAX_LEAVE} mm"));
        }
        if let Some(tabs) = self.tabs {
            if !(1..=MAX_TABS).contains(&tabs.count) {
                return bad(format!("the number of tabs must be 1 to {MAX_TABS}"));
            }
            if !tabs.width.is_none_or(|v| (MIN_TAB..=MAX_LEAVE).contains(&v)) || !tabs.height.is_none_or(|v| (MIN_TAB..=MAX_LEAVE).contains(&v)) {
                return bad(format!("a tab's width and height must be {MIN_TAB} to {MAX_LEAVE} mm"));
            }
        }
        if !self.ramp_angle.is_none_or(|v| (RAMP_ANGLES.0..=RAMP_ANGLES.1).contains(&v)) {
            return bad("the ramp angle must be 0.5 to 30 degrees".into());
        }
        if !self.passes.is_none_or(|n| (1..=MAX_CONTOUR_PASSES).contains(&n)) {
            return bad(format!("the number of passes must be 1 to {MAX_CONTOUR_PASSES}"));
        }
        // Passes further apart than the tool is wide would leave a wall standing between them.
        let widest = tool.geometry.diameter;
        if !self.stepover.is_none_or(|v| v >= MIN_STEP && v <= widest) {
            return bad(format!("the stepover must be {MIN_STEP} mm to the tool's diameter ({widest} mm)"));
        }
        if self.stepover.is_some() && self.passes.is_none_or(|n| n < 2) {
            return bad("a stepover is what lies between passes: give the number of passes, two or more, with it".into());
        }
        // A tab is a bridge from the part to the stock beside it, across one pass's width: with
        // more passes that stock is further off, or gone.
        if self.tabs.is_some() && self.passes.is_some_and(|n| n > 1) {
            return bad(
                "tabs go with one pass: cut the stock away in passes down to above the bottom, and the part out in one pass with its tabs".into()
            );
        }
        match self.side {
            ContourSide::Inside => {
                if self.tabs.is_some() {
                    return bad("tabs hold a part that is cut round its outside: an inside contour takes none".into());
                }
                if self.openings.is_empty() || self.openings.len() > MAX_CONTOUR_OPENINGS {
                    return bad(format!("an inside contour goes round 1 to {MAX_CONTOUR_OPENINGS} openings, not {}", self.openings.len()));
                }
                if self.openings.iter().flatten().any(|v| !finite(*v)) {
                    return bad(format!("an opening's point must be finite and within {MAX_COORDINATE} mm"));
                }
                if self.passes.is_some_and(|n| n > 1) {
                    return bad(
                        "passes sideways are for the outside: an opening is cleared by Pocket, and an inside contour finishes its wall".into()
                    );
                }
            }
            ContourSide::Outside => {
                if self.ramp_angle.is_some() {
                    return bad("a ramp angle is for an inside contour: round the outside the tool goes down beside the stock".into());
                }
                if !self.openings.is_empty() {
                    return bad("openings are for an inside contour: round the outside it is the part's own outline that is cut".into());
                }
            }
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
        let Depths { top, bottom, levels, per_level, stepdown } = self.depths(tool, placement)?;
        let (stock, g, depth) = (placement.stock, tool.geometry, top - bottom);
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        // The tool's centre: the outline moved out by the tool's radius and the stock to leave.
        if outline.is_empty() {
            return bad("there is no outline to cut round".into());
        }
        if self.side == ContourSide::Inside {
            // Loops alone are openings with nothing standing in them.
            let openings: Vec<Region2> = outline.iter().map(|l| Region2 { outer: l.clone(), holes: Vec::new() }).collect();
            return self.toolpath_inside(start, tool, placement, material, &openings);
        }
        let radius = 0.5 * g.diameter;
        let offset = radius + self.leave.unwrap_or(0.0);
        let regions: Vec<Region2> = outline.iter().map(|l| Region2 { outer: l.clone(), holes: Vec::new() }).collect();
        // Where the tool goes down: beside the stock, its edge `clear` from it.
        let beyond = radius + self.clear.unwrap_or(DEFAULT_CONTOUR_CLEAR);
        let beside = (Vec2 { x: stock.min.x - beyond, y: stock.min.y - beyond }, Vec2 { x: stock.max.x + beyond, y: stock.max.y + beyond });
        let lead = self.lead.unwrap_or(DEFAULT_LEAD_RADIUS * g.diameter);
        // The passes, the one furthest from the wall first and the last one on it: each the
        // outline moved out by a stepover more than the next.
        let passes = self.passes.unwrap_or(1);
        let stepover = if passes > 1 { self.stepover.unwrap_or(DEFAULT_CONTOUR_STEPOVER * g.diameter) } else { 0.0 };
        let mut rounds: Vec<Round> = Vec::new();
        let mut enclosed = 0;
        for pass in (0..passes).rev() {
            let out = offset + pass as f64 * stepover;
            let moved = moved_at_least(&regions, -out).map_err(|e| Error::BadValue(format!("the part's outline: {e}")))?;
            if moved.is_empty() {
                return bad("the outline left nothing to cut round".into());
            }
            for region in &moved {
                rounds.push(round(&region.outer, &moved, self.direction, beside, lead)?);
            }
            if pass == 0 {
                enclosed = moved.iter().map(|region| region.holes.len()).sum();
            }
        }
        // Tabs: where along each round the tool is lifted, and how high.
        let tab_top = match self.tabs {
            Some(tabs) => {
                let height = tabs.height.unwrap_or(DEFAULT_TAB_HEIGHT);
                if !(depth - height > SAME_HEIGHT) {
                    return bad(format!("the tabs are {height} mm high and the cut {depth} mm deep: tabs as high as the cut leave nothing cut"));
                }
                Some(bottom + height)
            }
            None => None,
        };
        let lifts: Vec<Vec<(f64, f64)>> = match self.tabs {
            Some(tabs) => {
                rounds.iter().map(|r| tab_spans(&r.segs, tabs.count, tabs.width.unwrap_or(DEFAULT_TAB_WIDTH), tool)).collect::<Result<_>>()?
            }
            None => rounds.iter().map(|_| Vec::new()).collect(),
        };
        let f = op_feeds(tool, Engagement { axial: per_level.max(1e-4), radial: g.diameter }, material, placement.max_rpm, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };

        let Some(first) = rounds.first().map(|r| r.entry) else { return bad("the outline left nothing to cut round".into()) };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        // One round at one level, from wherever the tool is above the stock.
        let level = |moves: &mut Vec<Move>, last: &mut Vec2, r: &Round, lifts: &Vec<(f64, f64)>, z: f64| {
            // Back above where it goes down, when the round before left it elsewhere.
            let mut here = r.entry;
            if apart(*last, here) > JOINT_TOLERANCE {
                moves.push(Move::Rapid { to: at(here, retract) });
            }
            // Down beside the stock in the air, and in to the wall: straight to it, or
            // straight to the quarter turn that meets it at a tangent.
            moves.push(Move::Linear { to: at(here, z), feed: f.plunge_feed });
            let onto = r.leads.map_or(r.start, |(on, _)| on.start());
            if apart(here, onto) > JOINT_TOLERANCE {
                moves.push(Move::Linear { to: at(onto, z), feed: f.feed });
                here = onto;
            }
            if let Some((on, _)) = &r.leads {
                cut(moves, on, here, r.start, (z, z), f.feed);
            }
            // Round it, lifting over the tabs on the levels below their tops.
            let over = tab_top.filter(|&top| z < top - SAME_HEIGHT).map(|top| (top, lifts.as_slice()));
            go_round(moves, r, z, over, f.feed, f.plunge_feed);
            // Off the wall by the other quarter turn and up, or straight back out and up.
            match &r.leads {
                Some((_, off)) => {
                    cut(moves, off, r.start, off.end(), (z, z), f.feed);
                    here = off.end();
                }
                None => {
                    if apart(r.entry, r.start) > JOINT_TOLERANCE {
                        moves.push(Move::Linear { to: at(r.entry, z), feed: f.feed });
                    }
                    here = r.entry;
                }
            }
            moves.push(Move::Rapid { to: at(here, retract) });
            *last = here;
        };
        if passes == 1 {
            // Each part in turn, all its levels.
            for (r, lifts) in rounds.iter().zip(&lifts) {
                moves.push(Move::Rapid { to: at(r.entry, retract) });
                last = r.entry;
                for &z in &levels {
                    level(&mut moves, &mut last, r, lifts, z);
                }
            }
        } else {
            // Level by level, and at each the pass furthest from the wall first: no pass ever
            // cuts wider than the first, and the wall is met by the last, with a stepover to cut.
            moves.push(Move::Rapid { to: at(first, retract) });
            for &z in &levels {
                for (r, lifts) in rounds.iter().zip(&lifts) {
                    level(&mut moves, &mut last, r, lifts, z);
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

        let mut notes = f.notes;
        if enclosed > 0 {
            notes.push(format!(
                "{enclosed} enclosed place(s) between parts of the outline are not cut: the tool can't get into them from beside the stock"
            ));
        }
        let longest = rounds.iter().map(|r| apart(r.entry, r.leads.map_or(r.start, |(on, _)| on.start()))).fold(0.0, f64::max);
        if longest > LONG_WAY_IN * g.diameter {
            notes.push(format!("the way in from beside the stock is {longest:.1} mm of cutting as wide as the tool, at every level"));
        }
        if lead > 0.0 && rounds.iter().any(|r| r.leads.is_none()) {
            notes.push("there is no room for the arcs in and out at a part's wall: the tool goes straight in to it and straight out".into());
        }
        let tabs = rounds
            .iter()
            .zip(&lifts)
            .flat_map(|(r, lifts)| lifts.iter().filter_map(|&(up, down)| place_along(&r.segs, 0.5 * (up + down)).map(|p| [p.x, p.y])))
            .collect();
        Ok(Contoured {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top,
            levels,
            stepdown,
            offset,
            passes,
            stepover,
            round: rounds.iter().map(|r| r.segs.iter().map(Seg2Geom::length).sum::<f64>()).sum(),
            entries: rounds.iter().map(|r| [r.entry.x, r.entry.y]).collect(),
            leads: rounds.iter().map(Round::lead).collect(),
            tabs,
            tab_top,
            notes,
        })
    }
}

/// The heights a contour is cut at.
struct Depths {
    top: f64,
    bottom: f64,
    /// Each level's Z, top down: the last is the bottom.
    levels: Vec<f64>,
    per_level: f64,
    stepdown: f64,
}

impl ContourOp {
    /// The heights the cut is made at for `tool` in a setup placed at `placement`: from the top
    /// down to the bottom in levels of equal depth, none deeper than the stepdown, all within the
    /// tool's reach.
    fn depths(&self, tool: &Tool, placement: &Placement) -> Result<Depths> {
        let (stock, model) = (placement.stock, placement.model);
        let top = self.top.unwrap_or(stock.max.z);
        let bottom = self.bottom.unwrap_or(model.min.z);
        let depth = top - bottom;
        if !(depth > SAME_HEIGHT) {
            return bad(format!("nothing to cut: the bottom ({bottom} mm) is not below the top ({top} mm)"));
        }
        let retract = placement.retract_z;
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
        Ok(Depths { top, bottom, levels, per_level, stepdown })
    }

    /// An inside contour's toolpath round `openings`, one for each of the operation's points, in
    /// their order: each a closed wall (its outer loop) with whatever stands inside it (its
    /// holes: islands the tool must keep its radius from too), of lines and arcs, X and Y in
    /// the WCS. The rest as [`ContourOp::toolpath`].
    pub fn toolpath_inside(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        openings: &[Region2],
    ) -> Result<Contoured> {
        self.validate(tool)?;
        if self.side != ContourSide::Inside {
            return bad("an outside contour goes round the part's outline, not round openings".into());
        }
        if openings.len() != self.openings.len() {
            return bad(format!("{} openings were read from the model for the operation's {} points", openings.len(), self.openings.len()));
        }
        let d = self.depths(tool, placement)?;
        self.inside(start, tool, placement, material, openings, &d)
    }

    /// Round the inside of each of `openings`: the tool's centre on the wall moved in by its
    /// radius and the stock to leave (and as far from anything standing in the opening), down a
    /// ramp along that path to each level and once round at the level, and off the wall at the
    /// bottom by a quarter turn into the opening.
    fn inside(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        openings: &[Region2],
        d: &Depths,
    ) -> Result<Contoured> {
        let g = tool.geometry;
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        let radius = 0.5 * g.diameter;
        let offset = radius + self.leave.unwrap_or(0.0);
        let angle = self.ramp_angle.unwrap_or(DEFAULT_RAMP_ANGLE);
        let lead = self.lead.unwrap_or(DEFAULT_LEAD_RADIUS * g.diameter);
        let mut rounds = Vec::new();
        let mut notes = Vec::new();
        for (k, opening) in openings.iter().enumerate() {
            let what = format!("opening {}", k + 1);
            let region = Region2 { outer: opening.outer.ccw(), holes: opening.holes.iter().map(|island| island.ccw().reversed()).collect() };
            let moved = moved_at_least(std::slice::from_ref(&region), offset).map_err(|e| Error::BadValue(format!("{what}: {e}")))?;
            if moved.is_empty() {
                let widest = 2.0 * region.inradius(1e-4);
                return bad(format!(
                    "tool {} ({} mm across) does not fit in {what}, which is about {widest:.2} mm across at its widest: use a narrower tool",
                    tool.number, g.diameter
                ));
            }
            for path in &moved {
                let r = round_inside(path, self.direction, lead);
                let round: f64 = r.segs.iter().map(Seg2Geom::length).sum();
                let turns = (d.per_level / angle.tan() / round).ceil();
                if !(round > 0.0 && turns <= MAX_RAMP_TURNS) {
                    return bad(format!(
                        "{what}: the tool's path is {round:.3} mm round, so the ramp down {} mm would take {turns} turns: use a narrower tool or a steeper ramp",
                        d.per_level
                    ));
                }
                // What the tool does not pass over in the middle stays: loose, once the cut is
                // through.
                let core = 2.0 * (path.inradius(1e-4) - radius);
                if core > 10.0 * JOINT_TOLERANCE {
                    notes.push(format!(
                        "{what}: a core about {core:.1} mm across is left in its middle, loose once the cut is through; where it must be cut away, use Pocket"
                    ));
                }
                rounds.push(r);
            }
        }
        let f = op_feeds(tool, Engagement { axial: d.per_level.max(1e-4), radial: g.diameter }, material, placement.max_rpm, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
        let Some(first) = rounds.first().map(|r| r.start) else { return bad("the openings left nothing to cut round".into()) };
        let bottom = d.bottom;
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for r in &rounds {
            // Above where the path starts, down to the cut's top in the air, and from there
            // along the wall: a ramp to each level, and the level once round.
            moves.push(Move::Rapid { to: at(r.start, retract) });
            moves.push(Move::Linear { to: at(r.start, d.top), feed: f.plunge_feed });
            let mut above = d.top;
            for &z in &d.levels {
                ramp(&mut moves, r.start, &r.segs, above, z, angle, f.ramp_feed);
                go_round(&mut moves, r, z, None, f.feed, f.plunge_feed);
                above = z;
            }
            // Off the wall by the quarter turn and up, or straight up where there is no room.
            last = r.start;
            if let Some((_, off)) = &r.leads {
                cut(&mut moves, off, r.start, off.end(), (bottom, bottom), f.feed);
                last = off.end();
            }
            moves.push(Move::Rapid { to: at(last, retract) });
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;

        notes.extend(f.notes);
        if lead > 0.0 && rounds.iter().any(|r| r.leads.is_none()) {
            notes.push("there is no room for the arc off the wall in an opening: the tool goes straight up from the wall".into());
        }
        Ok(Contoured {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top: d.top,
            levels: d.levels.clone(),
            stepdown: d.stepdown,
            offset,
            passes: 1,
            stepover: 0.0,
            round: rounds.iter().map(|r| r.segs.iter().map(Seg2Geom::length).sum::<f64>()).sum(),
            entries: rounds.iter().map(|r| [r.start.x, r.start.y]).collect(),
            leads: rounds.iter().map(Round::lead).collect(),
            tabs: Vec::new(),
            tab_top: None,
            notes,
        })
    }
}

/// The outer loop of where the tool's centre may be in an opening, as a round to cut along the
/// inside of its wall, turned the way `direction` asks: climb with a clockwise spindle is counter-clockwise round
/// the inside. It starts at the middle of its longest straight side (the longest arc when there
/// is none; of equals, the one nearest the front, then the left), where the tool comes down from
/// above, and leaves the wall there by a quarter turn of `lead` (or a half or a quarter of it,
/// where that is what fits) into the opening.
fn round_inside(region: &Region2, direction: CutDirection, lead: f64) -> Round {
    round_within(region, None, direction, lead, false)
}

/// One loop of `region`, which is where the tool's centre may be in an opening, as a round to
/// cut along: its outer loop (along the inside of the opening's wall), or with `hole` the loop
/// round something standing in the opening. Both have the opening's air on their left (the
/// outer loop is counter-clockwise, a hole clockwise), and that is the way climb goes with a
/// clockwise spindle: the wall on the tool's right. It starts and leaves as [`round_inside`]
/// says, the quarter turn off going into the air. With `on_too` the tool meets the loop by a
/// quarter turn as well and goes down where that begins (the round's `entry`), and a radius is
/// taken only where both arcs are clear of every loop of the region.
pub(crate) fn round_within(region: &Region2, hole: Option<usize>, direction: CutDirection, lead: f64, on_too: bool) -> Round {
    let path = hole.and_then(|k| region.holes.get(k)).unwrap_or(&region.outer);
    let micron = |v: f64| (v * 1e6).round() as i64;
    let longest = path.segs.iter().enumerate().max_by_key(|(i, s)| {
        let mid = s.mid();
        (matches!(s, Seg2::Line { .. }), micron(s.length()), -micron(mid.y), -micron(mid.x), std::cmp::Reverse(*i))
    });
    let Some((at, seg)) = longest else {
        return Round { entry: Vec2 { x: 0.0, y: 0.0 }, start: Vec2 { x: 0.0, y: 0.0 }, segs: Vec::new(), leads: None };
    };
    let (before, after) = seg.split_half();
    let (start, along) = (after.start(), after.start_tangent());
    let mut segs = vec![after];
    segs.extend(path.segs.iter().skip(at + 1).copied());
    segs.extend(path.segs.iter().take(at).copied());
    segs.push(before);
    let travel = if direction == CutDirection::Climb {
        along
    } else {
        segs = Loop2 { segs }.reversed().segs;
        Vec2 { x: -along.x, y: -along.y }
    };
    // Into the opening: to the left of the loop's own way round, its air being on its left.
    let inn = Vec2 { x: -along.y, y: along.x };
    let leads = LEAD_SHARES.iter().map(|share| share * lead).filter(|&radius| radius >= MIN_ARC_RADIUS).find_map(|radius| {
        // A wall that curves round the tool has room for the arc only well inside its own curve.
        if matches!(*seg, Seg2::Arc { radius: own, sweep, .. } if sweep > 0.0 && radius > 0.5 * own) {
            return None;
        }
        let centre = Vec2 { x: start.x + radius * inn.x, y: start.y + radius * inn.y };
        // Turning the way that leaves along the travel: left when the opening is on the left.
        let sweep = FRAC_PI_2.copysign(travel.x * inn.y - travel.y * inn.x);
        let off = Seg2::Arc { center: centre, radius, start: (-inn.y).atan2(-inn.x), sweep };
        if !arc_is_clear(&off, std::slice::from_ref(region), (path, at)) {
            return None;
        }
        if !on_too {
            return Some((off, off));
        }
        // On by the quarter turn before it about the same centre: from behind the start.
        let on = Seg2::Arc { center: centre, radius, start: (-travel.y).atan2(-travel.x), sweep };
        arc_is_clear(&on, std::slice::from_ref(region), (path, at)).then_some((on, off))
    });
    let entry = match &leads {
        Some((on, _)) if on_too => on.start(),
        _ => start,
    };
    Round { entry, start, segs, leads }
}

/// The cutting moves once round `r` at the height `z` from its start back to it. With `over`
/// (the height of the tabs' tops and where along the round each tab's lift begins and ends),
/// the tool goes straight up where a lift begins, across at that height, and straight down at
/// the plunge feed where it ends.
pub(crate) fn go_round(moves: &mut Vec<Move>, r: &Round, z: f64, over: Option<(f64, &[(f64, f64)])>, feed: f64, plunge_feed: f64) {
    let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
    let (top, lifts) = over.unwrap_or((z, &[]));
    let (mut from, mut done, mut next, mut up) = (r.start, 0.0, 0, false);
    for (k, seg) in r.segs.iter().enumerate() {
        let length = seg.length();
        let mut t0 = 0.0;
        loop {
            // Where along this segment the tool next goes up or comes down, if it does.
            let change = lifts.get(next).map(|&(begin, end)| if up { end } else { begin }).filter(|&s| s < done + length && length > 0.0);
            let t1 = change.map_or(1.0, |s| ((s - done) / length).clamp(t0, 1.0));
            let height = if up { top } else { z };
            if t1 - t0 > SAME_PLACE {
                let piece = piece_of(seg, t0, t1);
                // The round ends where it began, to the last bit.
                let to = if k + 1 == r.segs.len() && change.is_none() { r.start } else { piece.end() };
                cut(moves, &piece, from, to, (height, height), feed);
                from = to;
            }
            if change.is_none() {
                break;
            }
            if up {
                moves.push(Move::Linear { to: at(from, z), feed: plunge_feed });
                next += 1;
            } else {
                moves.push(Move::Linear { to: at(from, top), feed });
            }
            up = !up;
            t0 = t1;
        }
        done += length;
    }
    if up {
        // A lift that ran to the very end of the round: down where it began.
        moves.push(Move::Linear { to: at(from, z), feed: plunge_feed });
    }
}

/// The part of a line or an arc from the share `t0` of its length to `t1`.
fn piece_of(seg: &Seg2, t0: f64, t1: f64) -> Seg2 {
    match *seg {
        Seg2::Arc { center, radius, start, sweep } => Seg2::Arc { center, radius, start: start + t0 * sweep, sweep: (t1 - t0) * sweep },
        _ => Seg2::Line { a: seg.point_at(t0), b: seg.point_at(t1) },
    }
}

/// The point `along` mm round `segs` from their start.
fn place_along(segs: &[Seg2], along: f64) -> Option<Vec2> {
    let mut done = 0.0;
    for seg in segs {
        let length = seg.length();
        if along <= done + length && length > 0.0 {
            return Some(seg.point_at(((along - done) / length).clamp(0.0, 1.0)));
        }
        done += length;
    }
    segs.last().map(Seg2Geom::end)
}

/// Where along the round `segs` (from its start, mm) the tool lifts for each of `count` tabs
/// `width` mm wide and comes down again. A tab is as wide as asked where it is narrowest, on the
/// tool's centre line, so the tool is up for the width and its own diameter.
///
/// The tabs are evenly spaced. Where the first one is (they all move together) is chosen so
/// that as many as can be lie on straight sides, each wholly on one: of the places that manage
/// that, the middle of the longest stretch of them, which keeps the tabs furthest from the
/// corners. A tab still on a corner is then moved along to the nearest straight side that
/// holds it, as far as leaves a tool's diameter of full cut between it and its neighbours.
/// The tool is down for at least half its diameter after the round starts and before it ends.
fn tab_spans(segs: &[Seg2], count: usize, width: f64, tool: &Tool) -> Result<Vec<(f64, f64)>> {
    let diameter = tool.geometry.diameter;
    let round: f64 = segs.iter().map(Seg2Geom::length).sum();
    let (lifted, space) = (width + diameter, round / count as f64);
    // Between two lifts the tool must be down for at least its own diameter.
    let spare = space - lifted - diameter;
    if !(spare >= 0.0) {
        return bad(format!(
            "{count} tab(s) {width} mm wide do not fit round a part with tool {}: each takes {} mm of the {round:.3} mm round it (its width and twice the tool's diameter)",
            tool.number,
            lifted + diameter
        ));
    }
    let half = 0.5 * lifted;
    // Where the middle of a lift may be for the whole of it to lie on one straight side.
    let mut straights: Vec<(f64, f64)> = Vec::new();
    let mut done = 0.0;
    for seg in segs {
        let length = seg.length();
        if matches!(seg, Seg2::Line { .. }) && length >= lifted {
            straights.push((done + half, done + length - half));
        }
        done += length;
    }
    // The first tab's middle may be from `lo` to `hi`: the stretches of that in which tab `i`
    // lies on a straight side, as the places where the number of tabs that do goes up and down.
    let (lo, hi) = (half + 0.5 * diameter, space - half - 0.5 * diameter);
    let mut first = 0.5 * (lo + hi);
    if count.saturating_mul(straights.len()) <= MAX_TAB_PLACES {
        let mut changes: Vec<(f64, i32)> = Vec::new();
        for i in 0..count {
            let before = i as f64 * space;
            for &(from, to) in &straights {
                let (from, to) = ((from - before).max(lo), (to - before).min(hi));
                if to > from {
                    changes.extend([(from, 1), (to, -1)]);
                }
            }
        }
        // Ends before beginnings where they coincide, so that touching stretches do not count as one.
        changes.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
        // The best stretch: the most tabs on straight sides, then the longest, then the nearest
        // the middle, then the earliest.
        let middle = 0.5 * (lo + hi);
        let (mut on, mut best) = (0, None::<(i32, f64, f64)>);
        let mut stretch: Option<(i32, f64, f64)> = None;
        let better = |candidate: (i32, f64, f64), best: &mut Option<(i32, f64, f64)>| {
            let (length, centre) = (candidate.2 - candidate.1, 0.5 * (candidate.1 + candidate.2));
            let wins = best.is_none_or(|(most, from, to)| {
                let (longest, at) = (to - from, 0.5 * (from + to));
                candidate.0 > most
                    || (candidate.0 == most
                        && (length > longest + JOINT_TOLERANCE
                            || (length >= longest - JOINT_TOLERANCE && (centre - middle).abs() < (at - middle).abs() - JOINT_TOLERANCE)))
            });
            if wins {
                *best = Some(candidate);
            }
        };
        let mut at = lo;
        for &(x, change) in &changes {
            if x > at {
                // From `at` to `x`, `on` tabs lie on straight sides: one stretch with the last
                // when the number is the same.
                stretch = match stretch {
                    Some((most, from, to)) if most == on && (to - at).abs() <= JOINT_TOLERANCE => Some((most, from, x)),
                    Some(ended) => {
                        better(ended, &mut best);
                        Some((on, at, x))
                    }
                    None => Some((on, at, x)),
                };
                at = x;
            }
            on += change;
        }
        if let Some(ended) = stretch {
            better(ended, &mut best);
        }
        if let Some((most, from, to)) = best
            && most > 0
        {
            first = 0.5 * (from + to);
        }
    }
    let spans = (0..count).map(|i| {
        let middle = first + i as f64 * space;
        let held = |at: f64| straights.iter().any(|&(from, to)| at >= from - JOINT_TOLERANCE && at <= to + JOINT_TOLERANCE);
        if held(middle) {
            return (middle - half, middle + half);
        }
        // The nearest place on a straight side that holds the whole lift, if it is near enough.
        let near = straights
            .iter()
            .filter(|&&(from, to)| to >= from)
            .map(|&(from, to)| middle.clamp(from, to))
            .filter(|there| (there - middle).abs() <= 0.5 * spare && *there >= lo && *there <= round - lo)
            .min_by(|a, b| (a - middle).abs().total_cmp(&(b - middle).abs()));
        let middle = near.unwrap_or(middle);
        (middle - half, middle + half)
    });
    Ok(spans.collect())
}

/// `regions` with their boundaries moved `distance` mm (as [`offset_regions`]), or a micron
/// further when exactly that far can't be verified. A gap exactly as wide as the tool is one
/// the offsetter can call neither open nor shut, and refuses; a micron more shuts it, and keeps
/// the tool that much further from the wall.
pub(crate) fn moved_at_least(regions: &[Region2], distance: f64) -> Result<Vec<Region2>> {
    offset_regions(regions, distance).or_else(|exactly| offset_regions(regions, distance + HAIR_MORE.copysign(distance)).map_err(|_| exactly))
}

pub(crate) fn apart(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The cutting moves along `seg` from `from` (where the tool is) to `to` (the segment's end),
/// going from the height `heights.0` at its start to `heights.1` at its end (the same for a level
/// cut, falling for a ramp): a line, or an arc (a helix when the heights differ) in at most
/// half-turns. An arc too flat or too small to be one on a machine is cut as a line, which is
/// nearer the arc than any machine resolves.
pub(crate) fn cut(moves: &mut Vec<Move>, seg: &Seg2, from: Vec2, to: Vec2, heights: (f64, f64), feed: f64) {
    let (z0, z1) = heights;
    let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
    let Seg2::Arc { center, radius, start, sweep } = *seg else {
        if apart(from, to) > 0.0 || z0 != z1 {
            moves.push(Move::Linear { to: at(to, z1), feed });
        }
        return;
    };
    let pieces = if sweep.abs() > PI + 1e-9 { 2 } else { 1 };
    let step = sweep / f64::from(pieces);
    let flat = radius < MIN_ARC_RADIUS || radius * (1.0 - (0.5 * step).cos()) < FLAT_ARC;
    for k in 1..=pieces {
        let angle = start + step * f64::from(k);
        let end = if k == pieces { to } else { Vec2 { x: center.x + radius * angle.cos(), y: center.y + radius * angle.sin() } };
        let z = if k == pieces { z1 } else { z0 + (z1 - z0) * f64::from(k) / f64::from(pieces) };
        if flat {
            moves.push(Move::Linear { to: at(end, z), feed });
        } else {
            let direction = if sweep > 0.0 { ArcDirection::CounterClockwise } else { ArcDirection::Clockwise };
            moves.push(Move::Arc { to: at(end, z), center: at(center, z), plane: Plane::Xy, direction, feed });
        }
    }
}

/// A counter-clockwise loop of the tool's centre as a round to cut, turned the way `direction`
/// asks. It starts at the middle of a side, with the tool going down straight out from there,
/// outside `beside` (the stock's box grown by the tool's radius and its clearance). The side is
/// the longest straight one (the longest arc when there is none; of equals, the one nearest the
/// front, then the left) from which that way in passes clear of every loop of `all`, the paths
/// round every part of the outline: a way in across another part would cut into it.
///
/// With `lead` above 0 the tool meets the loop and leaves it by quarter turns of that radius
/// (or a half or a quarter of it, where that is what fits) that touch the loop at the start:
/// the first side that has room for them is taken, and only when none has does the tool go
/// straight in to the first side with a clear way.
pub(crate) fn round(path: &Loop2, all: &[Region2], direction: CutDirection, beside: (Vec2, Vec2), lead: f64) -> Result<Round> {
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
    // How far from `from` along `way` until past the stock's reach; nothing when it already is.
    let far_from = |from: Vec2, way: Vec2| {
        let exit = |from: f64, way: f64, low: f64, high: f64| {
            if way > 1e-12 {
                (high - from) / way
            } else if way < -1e-12 {
                (low - from) / way
            } else {
                f64::INFINITY
            }
        };
        let within = from.x > lo.x && from.x < hi.x && from.y > lo.y && from.y < hi.y;
        if within { exit(from.x, way.x, lo.x, hi.x).min(exit(from.y, way.y, lo.y, hi.y)).max(0.0) } else { 0.0 }
    };
    // The arcs first, then straight in.
    let radii = LEAD_SHARES.iter().map(|share| share * lead).filter(|&radius| radius >= MIN_ARC_RADIUS).chain(std::iter::once(0.0));
    for radius in radii {
        for &(at, ..) in sides.iter().take(MAX_WAYS_IN) {
            let Some(seg) = path.segs.get(at) else { continue };
            let (before, after) = seg.split_half();
            let (start, along) = (after.start(), after.start_tangent());
            // Straight out of the loop: to the right of its travel, the loop being counter-clockwise.
            let out = Vec2 { x: along.y, y: -along.x };
            // Climb with a clockwise spindle is clockwise round the outside of the part.
            let travel = if direction == CutDirection::Climb { Vec2 { x: -along.x, y: -along.y } } else { along };
            let (entry, leads) = if radius > 0.0 {
                // A side that curves away from the part has room for an arc only well inside
                // its own curve.
                if matches!(*seg, Seg2::Arc { radius: own, sweep, .. } if sweep < 0.0 && radius > 0.5 * own) {
                    continue;
                }
                let centre = Vec2 { x: start.x + radius * out.x, y: start.y + radius * out.y };
                let (first, end) = (
                    Vec2 { x: centre.x - radius * travel.x, y: centre.y - radius * travel.y },
                    Vec2 { x: centre.x + radius * travel.x, y: centre.y + radius * travel.y },
                );
                // Turning the way that keeps the centre on the outside: left when the outside
                // is on the left of the travel.
                let sweep = FRAC_PI_2.copysign(travel.x * out.y - travel.y * out.x);
                let on = Seg2::Arc { center: centre, radius, start: (-travel.y).atan2(-travel.x), sweep };
                let off = Seg2::Arc { center: centre, radius, start: (-out.y).atan2(-out.x), sweep };
                let far = far_from(first, out);
                let outside = |p: Vec2| !all.iter().any(|region| region.outer.contains(p));
                let clear = far.is_finite()
                    && outside(first)
                    && outside(end)
                    && way_is_clear(first, out, far, all)
                    && arc_is_clear(&on, all, (path, at))
                    && arc_is_clear(&off, all, (path, at));
                if !clear {
                    continue;
                }
                (Vec2 { x: first.x + far * out.x, y: first.y + far * out.y }, Some((on, off)))
            } else {
                let far = far_from(start, out);
                if !far.is_finite() || !way_is_clear(start, out, far, all) {
                    continue;
                }
                (Vec2 { x: start.x + far * out.x, y: start.y + far * out.y }, None)
            };
            let mut segs = vec![after];
            segs.extend(path.segs.iter().skip(at + 1).copied());
            segs.extend(path.segs.iter().take(at).copied());
            segs.push(before);
            if direction == CutDirection::Climb {
                segs = Loop2 { segs }.reversed().segs;
            }
            return Ok(Round { entry, start, segs, leads });
        }
    }
    bad("no straight way in from beside the stock to the part's outline is clear of the part: this outline needs the tool to ramp in, which is not built yet".into())
}

/// Whether the arc `arc` meets no segment of `regions`' loops, but for the one it is drawn to
/// touch: segment `own.1` of the loop `own.0`. An arc that only touches another segment counts
/// as meeting it.
fn arc_is_clear(arc: &Seg2, regions: &[Region2], own: (&Loop2, usize)) -> bool {
    let Seg2::Arc { center, radius, start, sweep } = *arc else { return false };
    let on = |p: Vec2, c: Vec2, start: f64, sweep: f64| {
        let angle = (p.y - c.y).atan2(p.x - c.x);
        let past = if sweep >= 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
        past <= sweep.abs() + SAME_PLACE || past >= TAU - SAME_PLACE
    };
    let meets = |seg: &Seg2| match *seg {
        Seg2::Arc { center: other, radius: reach, start: from, sweep: turn } => {
            let (dx, dy) = (other.x - center.x, other.y - center.y);
            let between = dx.hypot(dy);
            // Too far apart, or one inside the other: the circles do not meet.
            let slack = SAME_PLACE * (1.0 + radius + reach);
            if between > radius + reach + slack || between < (radius - reach).abs() - slack {
                return false;
            }
            if between <= slack {
                // One circle: taken as meeting, whatever the arcs' ends.
                return true;
            }
            let along = (between * between + radius * radius - reach * reach) / (2.0 * between);
            let across = (radius * radius - along * along).max(0.0).sqrt();
            let (ux, uy) = (dx / between, dy / between);
            [1.0, -1.0].into_iter().any(|side| {
                let p = Vec2 { x: center.x + along * ux - side * across * uy, y: center.y + along * uy + side * across * ux };
                on(p, center, start, sweep) && on(p, other, from, turn)
            })
        }
        _ => {
            let (a, b) = (seg.start(), seg.end());
            let (sx, sy) = (b.x - a.x, b.y - a.y);
            let length = sx.hypot(sy);
            if !(length > 0.0) {
                return apart(a, center) <= radius + SAME_PLACE && apart(a, center) >= radius - SAME_PLACE && on(a, center, start, sweep);
            }
            let (ux, uy) = (sx / length, sy / length);
            // The foot of the centre on the line, and how far each side of it the circle crosses.
            let foot = (center.x - a.x) * ux + (center.y - a.y) * uy;
            let off = ((center.x - a.x) * uy - (center.y - a.y) * ux).abs();
            let slack = SAME_PLACE * (1.0 + radius);
            if off > radius + slack {
                return false;
            }
            let half = (radius * radius - off * off).max(0.0).sqrt();
            [foot - half, foot + half]
                .into_iter()
                .filter(|s| (-slack..=length + slack).contains(s))
                .any(|s| on(Vec2 { x: a.x + s * ux, y: a.y + s * uy }, center, start, sweep))
        }
    };
    !regions
        .iter()
        .flat_map(|r| std::iter::once(&r.outer).chain(&r.holes))
        .flat_map(|l| l.segs.iter().enumerate().map(move |(i, seg)| (l, i, seg)))
        .any(|(l, i, seg)| !(std::ptr::eq(l, own.0) && i == own.1) && meets(seg))
}

/// Whether the straight way from `from` along the unit direction `way` for `far` mm meets no
/// segment of `regions`' loops past its first [`JOINT_TOLERANCE`] (it starts on one of them).
pub(crate) fn way_is_clear(from: Vec2, way: Vec2, far: f64, regions: &[Region2]) -> bool {
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
