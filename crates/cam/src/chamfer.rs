//! The chamfer operation (decision 0018 of the CAM work): the top outside edge of the part
//! broken with the side of a chamfer mill. The tool's centre goes once round the part's wall, a
//! little outside it and below the edge, so that its cone cuts the chamfer; how far outside and
//! how far below follow from the tool's angle, its tip and the chamfer's width. Asked for all
//! the edges, it goes round the inside of the openings in the part's top as well, as far from
//! their walls and from whatever stands in them.
//!
//! With the tool's half angle `a` from its axis, its tip's radius `t`, a chamfer `w` wide seen
//! from above and the tip `b` below the chamfer's lower edge: the chamfer's lower edge is `w /
//! tan a` below the top edge, the tip is `b` below that, and the tool's axis is `t + b tan a`
//! outside the wall. The cone is then `w` further out at the top edge's height: on the chamfer's
//! upper edge.

use serde::{Deserialize, Serialize};
use solidcore_record::{Loop2, Region2, Vec2, Vec3};
use solidcraft_geom::{Loop2Geom, Seg2Geom};

use crate::contour::{CutDirection, MAX_LEAVE, Round, SAME_HEIGHT, apart, bad, cut, go_round, moved_at_least, round, round_within};
use crate::feeds::op_feeds;
use crate::offset::{JOINT_TOLERANCE, MAX_OFFSET_SEGMENTS, winding};
use crate::placement::Placement;
use crate::tool::{Engagement, Tool, ToolKind};
use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, Move, SpindleDirection, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// How wide a chamfer is when the operation gives no width, mm: enough to break an edge.
pub const DEFAULT_CHAMFER_WIDTH: f64 = 0.5;
/// How far below the chamfer the tool's tip is when the operation does not say, mm: a pointed
/// tool's tip turns too slowly to cut, and is the first part of it to chip.
pub const DEFAULT_CHAMFER_TIP_BELOW: f64 = 1.0;
/// The narrowest chamfer, mm.
pub const MIN_CHAMFER_WIDTH: f64 = 0.01;
/// The sharpest and the bluntest tool that chamfers: its whole angle at the tip, radians.
pub const CHAMFER_TIP_ANGLES: (f64, f64) = (0.3490658503988659, 2.792526803190927);
/// How much more than the chamfer's width the arcs on to the wall and off it are when none is
/// given, mm: the tool goes down where the first begins, and its cone must be clear of the edge
/// there.
const LEAD_CLEAR: f64 = 0.5;
/// The arcs on and off when none is given, as a share of the tool's diameter (when that is more).
const LEAD_SHARE: f64 = 0.1;
/// The most separate outlines a chamfer is given.
pub const MAX_CHAMFER_OUTLINES: usize = 1_000;

/// Which of the part's top edges a chamfer breaks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChamferEdges {
    /// The outside edge of the part alone.
    #[default]
    Outside,
    /// The outside edge, and the rims of the openings in the part's top with the edges of
    /// whatever stands in them.
    All,
    /// The edges picked: the outside edge when the chamfer says so ([`ChamferOp::outside`]), and
    /// the rims of the openings it names by a point in each ([`ChamferOp::openings`]), with the
    /// edges of whatever stands in those.
    Picked,
    /// Closed chains of edges picked on the model (decision 0042 of the CAM work), each broken at
    /// its own height on the side away from the part ([`ChamferOp::chains`]).
    Chains,
}

/// A chamfer operation's parameters, as the design keeps them. Every one has a default.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChamferOp {
    /// How wide the chamfer is, seen from above, mm; `None` for half a millimetre.
    #[serde(default)]
    pub width: Option<f64>,
    /// How far the tool's tip is below the chamfer's lower edge, mm; `None` for 1.
    #[serde(default)]
    pub tip_below: Option<f64>,
    /// The Z of the edge that is chamfered, in the WCS; `None` for the model's top.
    #[serde(default)]
    pub top: Option<f64>,
    /// Which edges at that height: the outside one alone unless told otherwise.
    #[serde(default)]
    pub edges: ChamferEdges,
    /// With picked edges: whether the outside edge of the part is one of them.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub outside: bool,
    /// With picked edges: a point in each opening of the part's top whose rim is broken, X and Y
    /// in the WCS.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub openings: Vec<[f64; 2]>,
    /// With edges picked on the model (`chains`): a point on an edge of each closed chain to
    /// break, in model coordinates. The engine reads the chains, and which side of each the part
    /// is on, each time the toolpath is worked out.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub chains: Vec<[f64; 3]>,
    #[serde(default)]
    pub direction: CutDirection,
    /// The radius of the quarter turns on to the wall and off it, mm; `None` for the wider of a
    /// tenth of the tool's diameter and the chamfer's width and half a millimetre; 0 for none.
    #[serde(default)]
    pub lead: Option<f64>,
    /// A spindle speed to use instead of the tool's cutting data, rev/min.
    #[serde(default)]
    pub rpm: Option<f64>,
    /// A cutting feed to use instead of the tool's cutting data, mm/min.
    #[serde(default)]
    pub feed: Option<f64>,
}

/// A chamfer operation's toolpath and the numbers it was worked out with.
#[derive(Clone, Debug, PartialEq)]
pub struct Chamfered {
    pub toolpath: Toolpath,
    pub rpm: f64,
    pub feed: f64,
    pub plunge_feed: f64,
    /// The Z of the edge that is chamfered, and of the tool's tip as it goes round, in the WCS:
    /// of the first round, where the edges are at several heights.
    pub top: f64,
    pub z: f64,
    /// The Z of the edge each round breaks, in their order.
    pub heights: Vec<f64>,
    /// The chamfer's width seen from above and how far down the wall it goes, mm.
    pub width: f64,
    pub leg: f64,
    /// How far the tool's centre is outside the wall, mm.
    pub offset: f64,
    /// The length of the tool's centre path round the part, mm.
    pub round: f64,
    /// Where the tool goes down for each loop it cuts round, X and Y.
    pub entries: Vec<[f64; 2]>,
    /// The radius of the quarter turns on to each round and off it, mm: 0 where there are none.
    pub leads: Vec<f64>,
    /// Things worth showing beside the toolpath.
    pub notes: Vec<String>,
}

/// A closed chain of the part's edges a chamfer of edges picked breaks (decision 0042 of the CAM
/// work), as the engine reads it from the model, X and Y in the WCS.
#[derive(Clone, Debug, PartialEq)]
pub struct ChamferChain {
    /// The Z of the edge: the chamfer's upper edge.
    pub top: f64,
    /// The wall under it, as the model has it at the chamfer's lower edge.
    pub wall: ChamferWall,
}

/// Which side of a chain the part is on, and so which side the tool goes along.
#[derive(Clone, Debug, PartialEq)]
pub enum ChamferWall {
    /// The part is inside the loop (its outside edge, a boss's or an island's top edge): the
    /// tool goes round its outside.
    Outside(Loop2),
    /// The loop is an opening's wall (a pocket's or a hole's rim): its air, the wall as the
    /// outer loop and whatever stands in it as holes. The tool goes round inside along the wall,
    /// keeping off what stands there.
    Inside(Region2),
}

/// What a chamfer comes to with one tool: the numbers the toolpath and the outline's height
/// both follow from.
struct Shape {
    width: f64,
    /// How far down the wall the chamfer goes.
    leg: f64,
    /// How far below the chamfer's lower edge the tip is.
    below: f64,
    /// How far outside the wall the tool's axis is.
    offset: f64,
}

impl ChamferOp {
    /// Checks the parameters and that `tool` can cut the chamfer.
    pub fn validate(&self, tool: &Tool) -> Result<()> {
        self.shape(tool).map(|_| ())
    }

    fn shape(&self, tool: &Tool) -> Result<Shape> {
        if !self.top.is_none_or(|v| v.abs() <= MAX_COORDINATE) {
            return bad(format!("the chamfered edge's height must be finite and within {MAX_COORDINATE} mm"));
        }
        if !self.width.is_none_or(|v| (MIN_CHAMFER_WIDTH..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the chamfer's width must be {MIN_CHAMFER_WIDTH} to {MAX_LEAVE} mm"));
        }
        if !self.tip_below.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the tip must be 0 to {MAX_LEAVE} mm below the chamfer"));
        }
        if !self.lead.is_none_or(|v| (0.0..=MAX_LEAVE).contains(&v)) {
            return bad(format!("the radius of the arcs in and out must be 0 to {MAX_LEAVE} mm"));
        }
        if self.edges == ChamferEdges::Picked {
            if !self.outside && self.openings.is_empty() {
                return bad("no edge is picked: the outside edge (outside: true), or a point in an opening (openings)".into());
            }
            if self.openings.len() > MAX_CHAMFER_OUTLINES || !self.openings.iter().flatten().all(|v| v.abs() <= MAX_COORDINATE) {
                return bad(format!("the openings picked: at most {MAX_CHAMFER_OUTLINES} points, each within {MAX_COORDINATE} mm"));
            }
        } else if self.outside || !self.openings.is_empty() {
            return bad("`outside` and `openings` say which edges are picked: they go with edges: picked".into());
        }
        if self.edges == ChamferEdges::Chains {
            if self.chains.is_empty() || self.chains.len() > MAX_CHAMFER_OUTLINES || !self.chains.iter().flatten().all(|v| v.abs() <= MAX_COORDINATE)
            {
                return bad(format!("the edges picked: 1 to {MAX_CHAMFER_OUTLINES} points, each within {MAX_COORDINATE} mm"));
            }
            if self.top.is_some() {
                return bad("edges picked on the model are each broken at their own height: a chamfer of them takes no top".into());
            }
        } else if !self.chains.is_empty() {
            return bad("`chains` are edges picked on the model: they go with edges: chains".into());
        }
        if !self.rpm.is_none_or(|r| r > 0.0 && r <= MAX_SPINDLE_RPM) {
            return bad(format!("rpm must be above 0 and at most {MAX_SPINDLE_RPM}"));
        }
        if !self.feed.is_none_or(|f| f > 0.0 && f <= MAX_FEED) {
            return bad(format!("feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        if !matches!(tool.kind, ToolKind::ChamferMill | ToolKind::SpotDrill) {
            return bad(format!("tool {} ({:?}) does not cut a chamfer: use a chamfer mill or a spot drill", tool.number, tool.kind));
        }
        let g = tool.geometry;
        if !(CHAMFER_TIP_ANGLES.0..=CHAMFER_TIP_ANGLES.1).contains(&g.tip_angle) {
            return bad(format!(
                "tool {}'s tip angle must be {:.0} to {:.0} degrees to cut a chamfer, not {:.1}",
                tool.number,
                CHAMFER_TIP_ANGLES.0.to_degrees(),
                CHAMFER_TIP_ANGLES.1.to_degrees(),
                g.tip_angle.to_degrees()
            ));
        }
        // The tangent to twelve places: a 90 degree tool's is 1, not a hair under it, so that
        // its chamfers come out in the round numbers they were asked in.
        let slope = ((0.5 * g.tip_angle).tan() * 1e12).round() / 1e12;
        let (width, below) = (self.width.unwrap_or(DEFAULT_CHAMFER_WIDTH), self.tip_below.unwrap_or(DEFAULT_CHAMFER_TIP_BELOW));
        let (leg, offset) = (width / slope, 0.5 * g.tip_diameter + below * slope);
        // The cone must reach the chamfer's upper edge, and its flutes that far up.
        let across = 2.0 * (offset + width);
        if !(across <= g.diameter + 1e-9) {
            return bad(format!(
                "a chamfer {width} mm wide with the tip {below} mm below it is cut {across} mm across on the tool, and tool {} is {} mm: a narrower chamfer, the tip less far below, or a bigger tool",
                tool.number, g.diameter
            ));
        }
        if !(leg + below <= g.flute_length + 1e-9) {
            return bad(format!("the chamfer is cut {} mm up the tool, past tool {}'s {} mm of flutes", leg + below, tool.number, g.flute_length));
        }
        Ok(Shape { width, leg, below, offset })
    }

    /// How far below an edge it breaks the chamfer's lower edge is, and the tool's tip, mm.
    pub fn depths(&self, tool: &Tool) -> Result<(f64, f64)> {
        let shape = self.shape(tool)?;
        Ok((shape.leg, shape.leg + shape.below))
    }

    /// How far from a wall the tool reaches as it goes along it, its arcs on and off included, mm:
    /// what beside an edge must be open from the tool's tip up.
    pub fn reach(&self, tool: &Tool) -> Result<f64> {
        let shape = self.shape(tool)?;
        Ok(2.0 * shape.offset + shape.width + 2.0 * self.lead_radius(tool, &shape))
    }

    /// The radius of the arcs on to a wall and off it.
    fn lead_radius(&self, tool: &Tool, shape: &Shape) -> f64 {
        self.lead.unwrap_or((LEAD_SHARE * tool.geometry.diameter).max(shape.width + LEAD_CLEAR))
    }

    /// The Z, in the WCS, of the chamfer's lower edge: the part's outline there is the wall the
    /// tool goes round, whether the model has the chamfer on it or a sharp edge.
    pub fn wall_height(&self, tool: &Tool, placement: &Placement) -> Result<f64> {
        Ok(self.top.unwrap_or(placement.model.max.z) - self.shape(tool)?.leg)
    }

    /// The Z, in the WCS, of the tool's tip as it goes round: an opening whose rim is broken must
    /// be open that far down.
    pub fn tip_height(&self, tool: &Tool, placement: &Placement) -> Result<f64> {
        let shape = self.shape(tool)?;
        Ok(self.top.unwrap_or(placement.model.max.z) - shape.leg - shape.below)
    }

    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down, at
    /// the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, round `outline`: the closed loops of the part's wall at the chamfer's lower
    /// edge ([`ChamferOp::wall_height`]), of lines and arcs, X and Y in the WCS.
    pub fn toolpath(&self, start: Option<Vec3>, tool: &Tool, placement: &Placement, material: Option<&str>, outline: &[Loop2]) -> Result<Chamfered> {
        self.toolpath_round(start, tool, placement, material, outline, &[])
    }

    /// As [`ChamferOp::toolpath`], and for a chamfer of all the edges round the inside of
    /// `openings` as well: the air in each opening of the part's top at the same height whose
    /// rim is to be broken, its wall as the outer loop and whatever stands in it as holes.
    /// The tool's centre keeps as far inside each wall, and outside each thing standing there,
    /// as it keeps outside the part: anywhere that far from them its cone cuts no more than
    /// the chamfer. An opening too narrow for that is left sharp, and said.
    pub fn toolpath_round(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        outline: &[Loop2],
        openings: &[Region2],
    ) -> Result<Chamfered> {
        let shape = self.shape(tool)?;
        if self.edges == ChamferEdges::Chains {
            return bad("a chamfer of edges picked on the model goes along the chains read from the model, not an outline".into());
        }
        if self.edges == ChamferEdges::Outside && !openings.is_empty() {
            return bad("openings are for a chamfer of all the edges: the outside edge alone is the part's own outline".into());
        }
        if self.edges == ChamferEdges::Picked && openings.len() != self.openings.len() {
            return bad(format!("{} openings were read from the model for the chamfer's {} points", openings.len(), self.openings.len()));
        }
        // Picked edges leave the outside one out unless it is one of them.
        let round_the_part = self.edges != ChamferEdges::Picked || self.outside;
        let loops_inside = |opening: &Region2| std::iter::once(&opening.outer).chain(&opening.holes).map(|l| l.segs.len()).sum::<usize>();
        if openings.len() > MAX_CHAMFER_OUTLINES || openings.iter().map(loops_inside).sum::<usize>() > MAX_OFFSET_SEGMENTS {
            return bad(format!("the openings to chamfer: at most {MAX_CHAMFER_OUTLINES}, of {MAX_OFFSET_SEGMENTS} segments together"));
        }
        let top = self.top.unwrap_or(placement.model.max.z);
        let retract = placement.retract_z;
        if !(retract - top > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the chamfered edge ({top} mm)"));
        }
        if round_the_part && outline.is_empty() {
            return bad("there is no outline to chamfer".into());
        }
        if outline.len() > MAX_CHAMFER_OUTLINES || outline.iter().map(|l| l.segs.len()).sum::<usize>() > MAX_OFFSET_SEGMENTS {
            return bad(format!("the outline to chamfer: 1 to {MAX_CHAMFER_OUTLINES} loops of {MAX_OFFSET_SEGMENTS} segments together"));
        }
        // The outside only. Just under its top, a part with an island standing in a pocket has
        // the island's outline too, inside its own: an inside edge, which is left sharp and said.
        let within = |k: usize, l: &Loop2| {
            l.segs.first().is_some_and(|first| outline.iter().enumerate().any(|(other, o)| other != k && winding(first.start(), o) != 0))
        };
        let outside: Vec<&Loop2> = outline.iter().enumerate().filter(|(k, l)| round_the_part && !within(*k, l)).map(|(_, l)| l).collect();
        let islands = if round_the_part { outline.len() - outside.len() } else { 0 };
        let regions: Vec<Region2> = outside.iter().map(|l| Region2 { outer: (*l).clone(), holes: Vec::new() }).collect();
        let moved = moved_at_least(&regions, -shape.offset).map_err(|e| Error::BadValue(format!("the part's outline: {e}")))?;
        if round_the_part && moved.is_empty() {
            return bad("the outline left nothing to go round".into());
        }
        // The tool goes down where the arc on to the wall begins: nowhere beside the stock, as the
        // edge is above anything a contour has left round the part.
        let lead = self.lead_radius(tool, &shape);
        let nowhere = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.0, y: 0.0 });
        let mut rounds: Vec<Round> = moved.iter().map(|region| round(&region.outer, &moved, self.direction, nowhere, lead)).collect::<Result<_>>()?;
        // Inside each opening: along its wall, and round whatever stands in it.
        let mut narrow = 0;
        for (k, opening) in openings.iter().enumerate() {
            let air = Region2 { outer: opening.outer.ccw(), holes: opening.holes.iter().map(|island| island.ccw().reversed()).collect() };
            let within = moved_at_least(std::slice::from_ref(&air), shape.offset).map_err(|e| Error::BadValue(format!("opening {}: {e}", k + 1)))?;
            if within.is_empty() {
                narrow += 1;
            }
            for region in &within {
                let along = std::iter::once(None).chain((0..region.holes.len()).map(Some));
                rounds.extend(along.map(|hole| round_within(region, hole, self.direction, lead, true)).filter(|r| !r.segs.is_empty()));
            }
        }
        let mut said = Vec::new();
        if narrow > 0 {
            said.push(format!("{narrow} opening(s) too narrow for the tool to go round inside left sharp"));
        }
        if islands > 0 && self.edges == ChamferEdges::Outside {
            said.push(format!(
                "{islands} edge(s) inside the part's outline (an island standing in a pocket) left sharp: a chamfer goes round the outside only"
            ));
        }
        let nothing = if round_the_part {
            "the outline left nothing to go round".to_string()
        } else {
            format!("tool {} does not fit in any opening that is picked: there is nothing to go round", tool.number)
        };
        let rounds: Vec<(Round, f64)> = rounds.into_iter().map(|r| (r, top)).collect();
        self.cut_rounds(start, tool, placement, material, &shape, lead, &rounds, said, nothing)
    }

    /// The toolpath of a chamfer of edges picked on the model (decision 0042 of the CAM work), from
    /// `start` for `tool` in a setup placed at `placement`, with its stock `material`: along each
    /// of `chains`, as the engine read them, at its own height, on the side away from the part;
    /// the heights in the order their first chain is in, under one tool change, the tool up at
    /// the retract height between rounds.
    pub fn toolpath_chains(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        chains: &[ChamferChain],
    ) -> Result<Chamfered> {
        let shape = self.shape(tool)?;
        if self.edges != ChamferEdges::Chains {
            return bad("chains read from the model are for a chamfer of edges picked on the model (edges: chains)".into());
        }
        let segments = |c: &ChamferChain| match &c.wall {
            ChamferWall::Outside(l) => l.segs.len(),
            ChamferWall::Inside(air) => std::iter::once(&air.outer).chain(&air.holes).map(|l| l.segs.len()).sum(),
        };
        if chains.is_empty() || chains.len() > MAX_CHAMFER_OUTLINES || chains.iter().map(segments).sum::<usize>() > MAX_OFFSET_SEGMENTS {
            return bad(format!("the edges picked to chamfer: 1 to {MAX_CHAMFER_OUTLINES} chains, of {MAX_OFFSET_SEGMENTS} segments together"));
        }
        if !chains.iter().all(|c| c.top.abs() <= MAX_COORDINATE) {
            return bad(format!("the height of each edge picked must be finite and within {MAX_COORDINATE} mm"));
        }
        let lead = self.lead_radius(tool, &shape);
        let nowhere = (Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 0.0, y: 0.0 });
        let mut tops: Vec<f64> = Vec::new();
        for c in chains {
            if !tops.iter().any(|top| (top - c.top).abs() <= SAME_HEIGHT) {
                tops.push(c.top);
            }
        }
        let mut rounds: Vec<(Round, f64)> = Vec::new();
        let mut narrow = 0;
        for &top in &tops {
            let here = chains.iter().filter(|c| (c.top - top).abs() <= SAME_HEIGHT);
            // Round the outside edges of one height together: two too near for the tool between
            // them are gone round as one, and the way in to each keeps clear of the others.
            let outside: Vec<Region2> = here
                .clone()
                .filter_map(|c| match &c.wall {
                    ChamferWall::Outside(l) => Some(Region2 { outer: l.ccw(), holes: Vec::new() }),
                    ChamferWall::Inside(_) => None,
                })
                .collect();
            if !outside.is_empty() {
                let moved = moved_at_least(&outside, -shape.offset).map_err(|e| Error::BadValue(format!("the edges picked at Z {top}: {e}")))?;
                for region in &moved {
                    rounds.push((round(&region.outer, &moved, self.direction, nowhere, lead)?, top));
                }
            }
            // Inside each rim: along its wall alone, keeping off whatever stands in the opening.
            for air in here.filter_map(|c| match &c.wall {
                ChamferWall::Inside(air) => Some(air),
                ChamferWall::Outside(_) => None,
            }) {
                let air = Region2 { outer: air.outer.ccw(), holes: air.holes.iter().map(|island| island.ccw().reversed()).collect() };
                let within =
                    moved_at_least(std::slice::from_ref(&air), shape.offset).map_err(|e| Error::BadValue(format!("a rim picked at Z {top}: {e}")))?;
                if within.is_empty() {
                    narrow += 1;
                }
                rounds.extend(
                    within
                        .iter()
                        .map(|region| round_within(region, None, self.direction, lead, true))
                        .filter(|r| !r.segs.is_empty())
                        .map(|r| (r, top)),
                );
            }
        }
        let said =
            if narrow > 0 { vec![format!("{narrow} rim(s) picked too narrow for the tool to go round inside left sharp")] } else { Vec::new() };
        let nothing = format!("tool {} does not fit along any edge picked: there is nothing to go round", tool.number);
        self.cut_rounds(start, tool, placement, material, &shape, lead, &rounds, said, nothing)
    }

    /// The toolpath of `rounds`, each with the Z of the edge it breaks, under one tool change,
    /// and its numbers: `said` is what is worth saying of the rounds, and `nothing` why there is
    /// no toolpath when there are none.
    #[allow(clippy::too_many_arguments)]
    fn cut_rounds(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        shape: &Shape,
        lead: f64,
        rounds: &[(Round, f64)],
        said: Vec<String>,
        nothing: String,
    ) -> Result<Chamfered> {
        let g = tool.geometry;
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        let depth = shape.leg + shape.below;
        // The tool's own chip load, with nothing added for a thin chip: the cut is narrow, but it
        // is made near the tip, on a small diameter where the tool is weakest, and it is a
        // finishing cut. (A radial engagement of the whole diameter is how that is said.)
        let f = op_feeds(tool, Engagement { axial: shape.leg.max(1e-4), radial: g.diameter }, material, placement.max_rpm, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };

        let Some(first) = rounds.first().map(|(r, _)| r.entry) else {
            return bad(nothing);
        };
        let highest = rounds.iter().map(|(_, top)| *top).fold(f64::NEG_INFINITY, f64::max);
        if !(retract - highest > SAME_HEIGHT) {
            return bad(format!("the retract height ({retract} mm) must be above the chamfered edge ({highest} mm)"));
        }
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for (r, top) in rounds {
            let z = top - depth;
            // Down where the arc begins, on to the wall by it, once round, and off by the other.
            moves.push(Move::Rapid { to: at(r.entry, retract) });
            moves.push(Move::Linear { to: at(r.entry, z), feed: f.plunge_feed });
            let mut here = r.entry;
            let onto = r.leads.map_or(r.start, |(on, _)| on.start());
            if apart(here, onto) > JOINT_TOLERANCE {
                moves.push(Move::Linear { to: at(onto, z), feed: f.feed });
                here = onto;
            }
            if let Some((on, _)) = &r.leads {
                cut(&mut moves, on, here, r.start, (z, z), f.feed);
            }
            go_round(&mut moves, r, z, None, f.feed, f.plunge_feed);
            here = r.start;
            if let Some((_, off)) = &r.leads {
                cut(&mut moves, off, r.start, off.end(), (z, z), f.feed);
                here = off.end();
            }
            moves.push(Move::Rapid { to: at(here, retract) });
            last = here;
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;

        let mut notes = f.notes;
        notes.extend(said);
        if lead > 0.0 && rounds.iter().any(|(r, _)| r.leads.is_none()) {
            notes.push("there is no room for the arcs in and out at a part's wall: the tool goes straight down on to the edge".into());
        }
        Ok(Chamfered {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top: rounds.first().map_or(highest, |(_, top)| *top),
            z: rounds.first().map_or(highest, |(_, top)| *top) - depth,
            heights: rounds.iter().map(|(_, top)| *top).collect(),
            width: shape.width,
            leg: shape.leg,
            offset: shape.offset,
            round: rounds.iter().map(|(r, _)| r.segs.iter().map(Seg2Geom::length).sum::<f64>()).sum(),
            entries: rounds.iter().map(|(r, _)| [r.entry.x, r.entry.y]).collect(),
            leads: rounds.iter().map(|(r, _)| r.lead()).collect(),
            notes,
        })
    }
}

#[cfg(test)]
#[path = "chamfer_tests.rs"]
mod tests;
