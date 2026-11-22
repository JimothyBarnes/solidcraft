//! Stock simulation (decision 0015 of the CAM work): the stock as a field of heights over a grid,
//! lowered by every cutting move of a toolpath and checked against the part as it goes. A
//! sampled preview and check, as SPEC §4.4 has it: what it reports is as fine as its cells.
//!
//! One setup cuts from above, so the material left has one height over each point of the table.
//! A tool is its profile: how far its cutting surface is above its tip at each distance from its
//! axis. A cutting move lowers every cell the tool passes over to the tool's surface there: for
//! a flat tool exactly, whatever the move's slope (the lowest the tool gets over a cell is at one
//! end of the stretch during which the cell is under it); for other profiles the lowest of a few
//! places along that stretch. A straight move and an arc in the XY plane are each cut as they
//! are: which cells the tool passes over, and for how much of the move, is worked out from the
//! line or the circle itself. A gouge is a cell lowered below the part; a rapid that passes below
//! the material's top is a hit. A rapid in more than one axis has no one path: a control may
//! move each axis at its own top speed, the shortest arriving first (Haas's workbook says so of
//! G00, p. 28), so it is checked over the whole box between its ends, at the lower end's height.
//!
//! Above its flutes a tool does not cut: its neck, its shank and its holder are discs at their
//! heights above the tip, and a move that brings one of them into material is a hit of its own
//! ([`BodyHit`]). The holder is checked where the tool's data describes one.

use std::f64::consts::{PI, TAU};

use solidcore_record::{Vec2, Vec3};

use crate::cycle::{self, CycleMotion};
use crate::placement::Bounds;
use crate::tool::{Tool, ToolKind};
use crate::toolpath::{ArcGeom, Move, Toolpath};
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most cells a field may have.
pub const MAX_FIELD_CELLS: usize = 4_000_000;
/// The smallest cell, mm.
pub const MIN_CELL: f64 = 0.01;
/// The most gouges and the most rapid hits a simulation lists (it counts them all).
pub const MAX_LISTED: usize = 100;
/// The most chords an arc is cut as, where it is cut as chords (in the ZX and YZ planes).
const MAX_CHORDS: usize = 10_000;
/// The finest such chords are made: how far inside the arc they may run, as a share of a cell.
const FINEST_SAG: f64 = 0.001;
/// How far inside the tool's edge a cell must be for a cut there to cut into the part, in
/// cells: along a wall the part and the cut share a row of cells, and that row is the grid's
/// own error. Nearer the edge than this the part's own height stands, and no gouge is counted.
const WALL_SLACK: f64 = 0.75;
/// How many places along the stretch a cell is under a tool that is not flat are tried, besides
/// its ends and the nearest.
const PROFILE_SAMPLES: usize = 5;
/// How many steps a holder's cone is checked as, each as wide as its wider end.
const HOLDER_STEPS: usize = 4;
/// The most pieces of a holder that are checked.
const MAX_BODY_DISCS: usize = 256;

fn bad<T>(message: String) -> Result<T> {
    Err(Error::BadValue(message))
}

/// Heights over a grid of cells: for each cell, a height over its centre.
#[derive(Clone, Debug, PartialEq)]
pub struct HeightField {
    /// The grid's least corner, X and Y.
    pub x: f64,
    pub y: f64,
    /// The cell asked for: the most a side of a cell is, mm.
    pub cell: f64,
    /// The sides of a cell along X and along Y, mm: the box's sides in whole numbers of cells,
    /// so the grid covers the box and no more.
    pub dx: f64,
    pub dy: f64,
    /// Cells along X and along Y.
    pub nx: usize,
    pub ny: usize,
    /// Row by row from the least Y, each row from the least X.
    pub heights: Vec<f64>,
}

impl HeightField {
    /// A field over the box from `min` to `max` in cells of side `cell` at most, every height
    /// `height`. Each side of the box is cut into the fewest equal cells no longer than `cell`.
    pub fn flat(min: Vec2, max: Vec2, cell: f64, height: f64) -> Result<HeightField> {
        let finite = [min.x, min.y, max.x, max.y, height].iter().all(|v| v.abs() <= MAX_COORDINATE);
        if !finite || !(max.x > min.x && max.y > min.y) {
            return bad("a height field needs a box with a size, finite and in range".into());
        }
        if !(MIN_CELL..=MAX_COORDINATE).contains(&cell) {
            return bad(format!("a height field's cell must be at least {MIN_CELL} mm"));
        }
        let (nx, ny) = (((max.x - min.x) / cell - 1e-9).ceil().max(1.0), ((max.y - min.y) / cell - 1e-9).ceil().max(1.0));
        if !(nx * ny <= MAX_FIELD_CELLS as f64) {
            return bad(format!("{nx} x {ny} cells of {cell} mm is more than {MAX_FIELD_CELLS}: use a larger cell"));
        }
        let (dx, dy) = ((max.x - min.x) / nx, (max.y - min.y) / ny);
        let (nx, ny) = (nx as usize, ny as usize);
        Ok(HeightField { x: min.x, y: min.y, cell, dx, dy, nx, ny, heights: vec![height; nx * ny] })
    }

    /// The centre of the cell `i` along X and `j` along Y.
    pub fn centre(&self, i: usize, j: usize) -> Vec2 {
        Vec2 { x: self.x + (i as f64 + 0.5) * self.dx, y: self.y + (j as f64 + 0.5) * self.dy }
    }

    /// The first and the last column whose centres lie from `lo` to `hi` along X; none when no
    /// centre does.
    pub fn columns(&self, lo: f64, hi: f64) -> Option<(usize, usize)> {
        within(lo, hi, self.x, self.dx, self.nx)
    }

    /// The first and the last row whose centres lie from `lo` to `hi` along Y.
    pub fn rows(&self, lo: f64, hi: f64) -> Option<(usize, usize)> {
        within(lo, hi, self.y, self.dy, self.ny)
    }

    /// The height of the cell `i` along X and `j` along Y.
    pub fn at(&self, i: usize, j: usize) -> Option<f64> {
        if i < self.nx && j < self.ny { self.heights.get(j * self.nx + i).copied() } else { None }
    }

    /// The volume between the field and the height `floor`, mm³; nothing where it is below.
    pub fn volume_above(&self, floor: f64) -> f64 {
        self.heights.iter().map(|h| (h - floor).max(0.0)).sum::<f64>() * self.dx * self.dy
    }
}

/// The first and the last of `n` cells of side `side` from `origin` whose centres lie from `lo`
/// to `hi`; none when no centre does (or a bound is not a number).
fn within(lo: f64, hi: f64, origin: f64, side: f64, n: usize) -> Option<(usize, usize)> {
    let first = ((lo - origin) / side - 0.5).ceil().max(0.0);
    let last = ((hi - origin) / side - 0.5).floor().min(n as f64 - 1.0);
    (last >= first).then_some((first as usize, last as usize))
}

/// A tool as the simulation cuts with it: a radius and the shape below it, and above its
/// flutes what does not cut.
#[derive(Clone, Debug, PartialEq)]
struct Cutter {
    radius: f64,
    profile: Profile,
    /// The tool above its flutes, from the tip up.
    body: Vec<Disc>,
}

/// A part of a tool that does not cut: from `above` mm above the tip it is `radius` wide.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Disc {
    radius: f64,
    above: f64,
    part: BodyPart,
}

/// Which part of a tool that does not cut.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BodyPart {
    /// Above its flutes: its neck or its shank.
    Shank,
    /// Its holder.
    Holder,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Profile {
    /// A flat bottom.
    Flat,
    /// A flat bottom with its corner rounded to this radius (the tool's own radius: a ball).
    Corner(f64),
    /// A cone from a flat tip of this radius, rising this much for each millimetre out.
    Point(f64, f64),
}

impl Cutter {
    fn of(tool: &Tool) -> Result<Cutter> {
        let g = tool.geometry;
        let radius = 0.5 * g.diameter;
        if !(radius > 0.0 && radius <= MAX_COORDINATE) {
            return bad(format!("tool {}'s diameter must be above 0", tool.number));
        }
        let cone = || {
            let half = 0.5 * g.tip_angle;
            if half > 1e-6 && half < 0.5 * PI - 1e-6 {
                Profile::Point((0.5 * g.tip_diameter).clamp(0.0, radius), 1.0 / half.tan())
            } else {
                Profile::Flat
            }
        };
        let profile = match tool.kind {
            ToolKind::BallEndMill => Profile::Corner(radius),
            ToolKind::BullNoseEndMill | ToolKind::FaceMill if g.corner_radius > 0.0 => Profile::Corner(g.corner_radius.min(radius)),
            ToolKind::Drill | ToolKind::SpotDrill | ToolKind::ChamferMill | ToolKind::Engraving => cone(),
            ToolKind::Dovetail | ToolKind::Lollipop | ToolKind::SlotCutter => {
                return bad(format!("tool {} ({:?}) cuts under itself, which a field of heights can't show", tool.number, tool.kind));
            }
            ToolKind::Tap | ToolKind::ThreadMill => {
                return bad(format!("tool {} ({:?}) cuts a thread, which a field of heights can't show", tool.number, tool.kind));
            }
            _ => Profile::Flat,
        };
        // Above the flutes nothing cuts: the neck, as wide as the cutter at most; the shank where
        // it is wider, from the shoulder; and the holder's pieces from where the tool leaves it.
        let usable = |v: f64| v.is_finite() && v > 0.0;
        let mut body = Vec::new();
        if usable(g.flute_length) {
            body.push(Disc { radius, above: g.flute_length, part: BodyPart::Shank });
            let shank = 0.5 * g.shank_diameter;
            if usable(shank) && shank > radius + 1e-9 {
                let from = if usable(g.shoulder_length) { g.shoulder_length.max(g.flute_length) } else { g.flute_length };
                body.push(Disc { radius: shank, above: from, part: BodyPart::Shank });
            }
        }
        let mut above = tool.holder.stick_out;
        for piece in &tool.holder.segments {
            let (lower, upper) = (0.5 * piece.lower_diameter, 0.5 * piece.upper_diameter);
            if !(usable(above) && usable(piece.length) && usable(lower.max(upper))) || body.len() + HOLDER_STEPS > MAX_BODY_DISCS {
                break;
            }
            // A cone in steps, each as wide as its wider end: never narrower than the holder.
            let steps = if (upper - lower).abs() <= 1e-9 { 1 } else { HOLDER_STEPS };
            for k in 0..steps {
                let (from, to) = (k as f64 / steps as f64, (k + 1) as f64 / steps as f64);
                let widest = (lower + from * (upper - lower)).max(lower + to * (upper - lower));
                body.push(Disc { radius: widest, above: above + from * piece.length, part: BodyPart::Holder });
            }
            above += piece.length;
        }
        Ok(Cutter { radius, profile, body })
    }

    /// How much its cutting surface rises for each millimetre out, where it slopes all the way
    /// (a cone); 0 for the others. It is what an arc's chords cost in depth, where an arc is cut
    /// as chords.
    fn steepness(&self) -> f64 {
        match self.profile {
            Profile::Point(_, slope) => slope,
            _ => 0.0,
        }
    }

    /// How far the cutting surface is above the tip at `out` mm from the axis (within the radius).
    fn rise(&self, out: f64) -> f64 {
        match self.profile {
            Profile::Flat => 0.0,
            Profile::Corner(corner) => {
                let past = out - (self.radius - corner);
                if past <= 0.0 { 0.0 } else { corner - (corner * corner - past.min(corner).powi(2)).max(0.0).sqrt() }
            }
            Profile::Point(tip, slope) => (out - tip).max(0.0) * slope,
        }
    }
}

/// A place a toolpath cut into the part.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Gouge {
    /// Which toolpath of the simulation (as numbered by the caller) and which of its moves.
    pub operation: usize,
    pub move_index: usize,
    /// Where: the centre of the cell cut deepest below the part in that move, and the height it
    /// was cut to.
    pub at: Vec3,
    /// How far below the part, mm.
    pub depth: f64,
}

/// A rapid move that passed through material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RapidHit {
    pub operation: usize,
    pub move_index: usize,
    /// Where: the centre of the cell where the tool was furthest below the material's top, and
    /// the tool's height there.
    pub at: Vec3,
    /// How far below the material's top, mm.
    pub depth: f64,
}

/// A move that brought a part of the tool that does not cut into material.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct BodyHit {
    pub operation: usize,
    pub move_index: usize,
    pub part: BodyPart,
    /// Where: the centre of the cell whose material was furthest above that part of the tool,
    /// and that part's own height there.
    pub at: Vec3,
    /// How far into the material, mm.
    pub depth: f64,
}

/// A stretch of a piece of a move during which a point is under the tool: where it begins and
/// ends and where the tool's axis is nearest the point, as shares of the piece, and how near
/// that is, mm.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Stretch {
    from: f64,
    to: f64,
    nearest: f64,
    least: f64,
}

/// The stretch of a straight move during which a point `(px, py)` from the move's start is
/// within `r` of the tool's axis; `None` when it never is. `(dx, dy)` is the move seen from above.
fn stretch(px: f64, py: f64, dx: f64, dy: f64, r: f64) -> Option<Stretch> {
    let along = dx * dx + dy * dy;
    if along <= 1e-18 {
        let out = px.hypot(py);
        return (out <= r).then_some(Stretch { from: 0.0, to: 1.0, nearest: 0.0, least: out });
    }
    let middle = (px * dx + py * dy) / along;
    let off = (px * dy - py * dx).abs() / along.sqrt();
    if off > r {
        return None;
    }
    let half = (r * r - off * off).sqrt() / along.sqrt();
    let (from, to) = ((middle - half).max(0.0), (middle + half).min(1.0));
    if from > to {
        return None;
    }
    let nearest = middle.clamp(0.0, 1.0);
    Some(Stretch { from, to, nearest, least: (px - nearest * dx).hypot(py - nearest * dy) })
}

/// The stretches of an arc during which a point is within `r` of the tool's axis, as [`stretch`]
/// has the one of a line: at most two (a full circle begins and ends over the same cells). The
/// arc is about the origin, of `radius`, from the angle `start` through the signed `sweep`; the
/// point is `(px, py)`.
///
/// With the point `d` from the centre, the axis at an angle `u` round from the point's own
/// direction is sqrt(d² + R² - 2 d R cos u) from it: within `r` while cos u is at least
/// (d² + R² - r²) / 2 d R.
fn around(px: f64, py: f64, radius: f64, start: f64, sweep: f64, r: f64) -> [Option<Stretch>; 2] {
    let total = sweep.abs();
    let d = px.hypot(py);
    let (cosine, limit) = (d * d + radius * radius - r * r, 2.0 * d * radius);
    if !(total > 0.0 && radius > 0.0) || cosine > limit {
        return [None, None];
    }
    // How far round the arc, the way it goes, the point's own direction is from its start.
    let middle = if d > 0.0 { (sweep.signum() * (py.atan2(px) - start)).rem_euclid(TAU) } else { 0.0 };
    let apart = |u: f64| (d * d + radius * radius - 2.0 * d * radius * (u - middle).cos()).max(0.0).sqrt();
    if cosine <= -limit {
        // Within reach all the way round: nearest where the arc passes the point's direction,
        // or at whichever end is nearer when it stops short of that.
        let nearest = if middle <= total || apart(total) <= apart(0.0) { middle.min(total) } else { 0.0 };
        return [Some(Stretch { from: 0.0, to: 1.0, nearest: nearest / total, least: apart(nearest) }), None];
    }
    let half = (cosine / limit).clamp(-1.0, 1.0).acos();
    let mut found = [None, None];
    let mut slots = found.iter_mut();
    for turn in [-TAU, 0.0, TAU] {
        let passing = middle + turn;
        let (from, to) = ((passing - half).max(0.0), (passing + half).min(total));
        if from <= to
            && let Some(slot) = slots.next()
        {
            let nearest = passing.clamp(from, to);
            *slot = Some(Stretch { from: from / total, to: to / total, nearest: nearest / total, least: apart(nearest) });
        }
    }
    found
}

/// One piece of a move, as the tool's tip goes along it: straight, round an arc in the XY
/// plane (a helix when its ends differ in height), or anywhere between two points.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Piece {
    Line {
        a: Vec3,
        b: Vec3,
    },
    /// A rapid in more than one axis: the tool may be anywhere in the box between its ends, as
    /// low as the lower of them.
    Across {
        a: Vec3,
        b: Vec3,
    },
    Arc {
        centre: Vec2,
        radius: f64,
        /// The angle it starts at about its centre and how far round it goes (counter-clockwise
        /// when above 0), radians.
        start: f64,
        sweep: f64,
        /// The heights it starts and ends at.
        z: (f64, f64),
        /// The box round it seen from above: the least and the greatest X, then Y.
        bounds: (f64, f64, f64, f64),
    },
}

impl Piece {
    /// The arc `arc` from `a` to `b` as a piece, when it lies in the XY plane.
    fn round(arc: &ArcGeom, a: Vec3, b: Vec3) -> Option<Piece> {
        let (centre, start, z) = arc.level()?;
        let mut bounds = (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y));
        for p in arc.extremes() {
            bounds = (bounds.0.min(p.x), bounds.1.max(p.x), bounds.2.min(p.y), bounds.3.max(p.y));
        }
        (arc.sweep.abs() > 0.0).then_some(Piece::Arc { centre, radius: arc.radius, start, sweep: arc.sweep, z, bounds })
    }

    /// A rapid from `a` to `b`: a line when it moves along one axis, the box between its ends
    /// when it moves along more.
    fn rapid(a: Vec3, b: Vec3) -> Piece {
        let axes = [a.x != b.x, a.y != b.y, a.z != b.z].into_iter().filter(|moves| *moves).count();
        if axes > 1 { Piece::Across { a, b } } else { Piece::Line { a, b } }
    }

    /// The box round the piece seen from above: the least and the greatest X, then Y.
    fn bounds(&self) -> (f64, f64, f64, f64) {
        match *self {
            Piece::Line { a, b } | Piece::Across { a, b } => (a.x.min(b.x), a.x.max(b.x), a.y.min(b.y), a.y.max(b.y)),
            Piece::Arc { bounds, .. } => bounds,
        }
    }

    /// Where along X the cells of the row at `y` can be within `r` of the piece: the whole of
    /// its box for a line; for an arc the ring round its circle, which the row crosses once or,
    /// where it passes through the circle's inside, on each side.
    fn spans(&self, y: f64, r: f64) -> [Option<(f64, f64)>; 2] {
        let (x0, x1, ..) = self.bounds();
        let Piece::Arc { centre, radius, .. } = *self else { return [Some((x0 - r, x1 + r)), None] };
        let up = y - centre.y;
        let outer = (radius + r) * (radius + r) - up * up;
        if !(outer >= 0.0) {
            return [None, None];
        }
        let (outer, hollow) = (outer.sqrt(), (radius - r).max(0.0));
        let inner = (hollow * hollow - up * up).max(0.0).sqrt();
        let clip = |lo: f64, hi: f64| (lo.max(x0 - r) <= hi.min(x1 + r)).then_some((lo.max(x0 - r), hi.min(x1 + r)));
        if inner > 0.0 {
            [clip(centre.x - outer, centre.x - inner), clip(centre.x + inner, centre.x + outer)]
        } else {
            [clip(centre.x - outer, centre.x + outer), None]
        }
    }

    /// The tip's height a share `t` of the way along.
    fn height(&self, t: f64) -> f64 {
        match *self {
            Piece::Line { a, b } => a.z + t * (b.z - a.z),
            Piece::Across { a, b } => a.z.min(b.z),
            Piece::Arc { z, .. } => z.0 + t * (z.1 - z.0),
        }
    }

    /// How far the point `(x, y)` is from the tool's axis a share `t` of the way along, seen
    /// from above.
    fn away(&self, x: f64, y: f64, t: f64) -> f64 {
        match *self {
            Piece::Line { a, b } => (x - a.x - t * (b.x - a.x)).hypot(y - a.y - t * (b.y - a.y)),
            Piece::Across { .. } => self.outside(x, y),
            Piece::Arc { centre, radius, start, sweep, .. } => {
                let (sin, cos) = (start + t * sweep).sin_cos();
                (x - centre.x - radius * cos).hypot(y - centre.y - radius * sin)
            }
        }
    }

    /// How far the point `(x, y)` is outside the box round the piece, seen from above (0 within).
    fn outside(&self, x: f64, y: f64) -> f64 {
        let (x0, x1, y0, y1) = self.bounds();
        (x0 - x).max(x - x1).max(0.0).hypot((y0 - y).max(y - y1).max(0.0))
    }

    /// The stretches of the piece during which the point `(x, y)` is within `r` of the tool's
    /// axis.
    fn over(&self, x: f64, y: f64, r: f64) -> [Option<Stretch>; 2] {
        match *self {
            Piece::Line { a, b } => [stretch(x - a.x, y - a.y, b.x - a.x, b.y - a.y, r), None],
            Piece::Across { .. } => {
                let least = self.outside(x, y);
                [(least <= r).then_some(Stretch { from: 0.0, to: 1.0, nearest: 0.0, least }), None]
            }
            Piece::Arc { centre, radius, start, sweep, .. } => around(x - centre.x, y - centre.y, radius, start, sweep, r),
        }
    }
}

/// What one move of a toolpath has been found to do, as its pieces are passed.
struct Found {
    operation: usize,
    move_index: usize,
    /// How far into the part the toolpath is meant to cut ([`Simulation::cut_into`]).
    allowed: f64,
    /// How much lower than the true move the piece being passed may cut, mm: an arc's chords,
    /// where it is cut as chords.
    slack: f64,
    /// Its deepest cut below the part, and where as a rapid it was furthest below the material.
    gouge: Option<Gouge>,
    hit: Option<RapidHit>,
    /// Where a part of the tool that does not cut was deepest in material.
    body: Option<BodyHit>,
}

/// What one toolpath did to the stock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CutReport {
    /// The volume it removed, mm³.
    pub removed: f64,
    /// How many of its moves cut into the part, and how many of its rapids passed through material.
    pub gouges: usize,
    pub rapid_hits: usize,
    /// How many of its moves brought the tool's shank or holder into material.
    pub body_hits: usize,
}

/// A stock being cut.
#[derive(Clone, Debug, PartialEq)]
pub struct Simulation {
    /// The material left: its top over each cell.
    pub stock: HeightField,
    /// The stock's bottom: nothing is removed below it.
    pub floor: f64,
    /// The part over the same cells: its highest point over each (below the floor where there is
    /// none), when it is checked against.
    part: Option<Vec<f64>>,
    /// The part as the model has it, kept from the first toolpath that is meant to cut into it:
    /// how far such a toolpath may go is measured from here, not from what one before it left.
    model: Option<Vec<f64>>,
    /// How far below the part a cut may go before it is a gouge, and how far below the
    /// material's top a rapid may pass before it is a hit, mm.
    pub tolerance: f64,
    /// The volume removed so far, mm³.
    pub removed: f64,
    /// How many moves have cut into the part, and the first [`MAX_LISTED`] of them.
    pub gouge_count: usize,
    pub gouges: Vec<Gouge>,
    /// How many rapids have passed through material, and the first [`MAX_LISTED`] of them.
    pub rapid_hit_count: usize,
    pub rapid_hits: Vec<RapidHit>,
    /// How far below the stock's bottom the lowest cut went, mm (0 when none did).
    pub below_stock: f64,
    /// The stock's top before anything was cut: no material is above it.
    pub top: f64,
    /// How many moves have brought a tool's shank or holder into material, and the first
    /// [`MAX_LISTED`] of them.
    pub body_hit_count: usize,
    pub body_hits: Vec<BodyHit>,
}

impl Simulation {
    /// A simulation of the stock box `stock` in cells of side `cell`, with the checks' tolerance.
    pub fn new(stock: Bounds, cell: f64, tolerance: f64) -> Result<Simulation> {
        stock.check("the stock")?;
        if !(0.0..=MAX_COORDINATE).contains(&tolerance) {
            return bad("a simulation's tolerance must be 0 or more".into());
        }
        let field = HeightField::flat(Vec2 { x: stock.min.x, y: stock.min.y }, Vec2 { x: stock.max.x, y: stock.max.y }, cell, stock.max.z)?;
        Ok(Simulation {
            stock: field,
            floor: stock.min.z,
            part: None,
            model: None,
            tolerance,
            removed: 0.0,
            gouge_count: 0,
            gouges: Vec::new(),
            rapid_hit_count: 0,
            rapid_hits: Vec::new(),
            below_stock: 0.0,
            top: stock.max.z,
            body_hit_count: 0,
            body_hits: Vec::new(),
        })
    }

    /// Checks cuts against the part from here on: `heights` is the part's highest point over each
    /// cell of the stock's grid, in its order (anything below the stock's bottom where there is
    /// no part).
    pub fn with_part(mut self, heights: Vec<f64>) -> Result<Simulation> {
        if heights.len() != self.stock.heights.len() || heights.iter().any(|h| h.is_nan()) {
            return bad(format!("the part's heights must be {} numbers, one a cell", self.stock.heights.len()));
        }
        self.part = Some(heights);
        self.model = None;
        Ok(self)
    }

    /// The part's heights, when the simulation has them: the part as it is meant, which is
    /// lower than the model where a toolpath was meant to cut into it ([`Simulation::cut_into`]).
    pub fn part(&self) -> Option<&[f64]> {
        self.part.as_deref()
    }

    /// The part's heights as the model has them, whatever was meant to be cut into it.
    pub fn model(&self) -> Option<&[f64]> {
        self.model.as_deref().or(self.part.as_deref())
    }

    /// The volume of material still standing above the part where there is a part under it, and
    /// the volume of stock standing where there is none, mm³.
    pub fn leftover(&self) -> (f64, f64) {
        let Some(part) = &self.part else { return (0.0, self.stock.volume_above(self.floor)) };
        let (mut over, mut beside) = (0.0, 0.0);
        for (h, p) in self.stock.heights.iter().zip(part) {
            if *p > self.floor {
                over += (h - p).max(0.0);
            } else {
                beside += (h - self.floor).max(0.0);
            }
        }
        let area = self.stock.dx * self.stock.dy;
        (over * area, beside * area)
    }

    /// Cuts `toolpath` with `tool`, as toolpath number `operation` in what is reported.
    pub fn cut(&mut self, operation: usize, toolpath: &Toolpath, tool: &Tool) -> Result<CutReport> {
        self.cut_into(operation, toolpath, tool, 0.0)
    }

    /// As [`Simulation::cut`], for a toolpath that is meant to cut up to `into_part` mm into the
    /// part as the model has it: a chamfer on an edge the model leaves sharp. Such a cut is not
    /// a gouge, and where it is made the part is taken to be what it leaves, so that whatever
    /// is cut afterwards is judged against the part as it was meant. How far is measured from
    /// the model ([`Simulation::model`]), so passes that are each within it do not add up.
    pub fn cut_into(&mut self, operation: usize, toolpath: &Toolpath, tool: &Tool, into_part: f64) -> Result<CutReport> {
        if !(0.0..=MAX_COORDINATE).contains(&into_part) {
            return bad(format!("how far a toolpath may cut into the part must be 0 to {MAX_COORDINATE} mm, not {into_part}"));
        }
        toolpath.validate()?;
        let cutter = Cutter::of(tool)?;
        if into_part > 0.0 && self.model.is_none() {
            self.model.clone_from(&self.part);
        }
        let motion = CycleMotion::default();
        let before = (self.removed, self.gouge_count, self.rapid_hit_count, self.body_hit_count);
        let mut plain: Vec<Move> = Vec::new();
        for (index, (from, m)) in toolpath.walk().enumerate() {
            plain.clear();
            match *m {
                Move::Drill { to, r, retract, cycle, feed } => cycle::expand(from, to, r, retract, cycle, feed, &motion, &mut plain),
                other => plain.push(other),
            }
            let mut at = from;
            let mut found = Found { operation, move_index: index, allowed: into_part, slack: 0.0, gouge: None, hit: None, body: None };
            for step in &plain {
                match *step {
                    Move::Rapid { to } => self.pass(&Piece::rapid(at, to), &cutter, false, &mut found),
                    Move::Linear { to, .. } => self.pass(&Piece::Line { a: at, b: to }, &cutter, true, &mut found),
                    Move::Arc { to, center, plane, direction, .. } => {
                        let arc = ArcGeom::new(at, to, center, plane, direction).map_err(|reason| Error::BadMove { index, reason })?;
                        if let Some(round) = Piece::round(&arc, at, to) {
                            self.pass(&round, &cutter, true, &mut found);
                        } else {
                            // In the other two planes, as chords. A chord runs inside its arc,
                            // which there is lower or to one side: the tool cuts lower by as
                            // much and, where its side slopes, by its slope times as much
                            // again. The chords are fine enough to keep that within the
                            // tolerance (between a thousandth and a quarter of a cell), and
                            // what is left of it, worked out from the chords used, is the
                            // simulation's own error and not a cut into the part.
                            let cell = self.stock.dx.min(self.stock.dy);
                            let steep = 1.0 + cutter.steepness();
                            let sag = (self.tolerance / steep).clamp(FINEST_SAG * cell, 0.25 * cell);
                            let each = if sag >= arc.radius { 0.5 * PI } else { 2.0 * (1.0 - sag / arc.radius).acos() };
                            let chords = ((arc.sweep.abs() / each).ceil() as usize).clamp(1, MAX_CHORDS);
                            let inside = arc.radius * (1.0 - (0.5 * arc.sweep.abs() / chords as f64).cos());
                            found.slack = inside * steep;
                            let mut from = at;
                            for k in 1..=chords {
                                let end = if k == chords { to } else { arc.point(k as f64 / chords as f64) };
                                self.pass(&Piece::Line { a: from, b: end }, &cutter, true, &mut found);
                                from = end;
                            }
                            found.slack = 0.0;
                        }
                    }
                    _ => {}
                }
                at = step.end(at);
            }
            if let Some(gouge) = found.gouge {
                self.gouge_count += 1;
                if self.gouges.len() < MAX_LISTED {
                    self.gouges.push(gouge);
                }
            }
            if let Some(hit) = found.hit {
                self.rapid_hit_count += 1;
                if self.rapid_hits.len() < MAX_LISTED {
                    self.rapid_hits.push(hit);
                }
            }
            if let Some(hit) = found.body {
                self.body_hit_count += 1;
                if self.body_hits.len() < MAX_LISTED {
                    self.body_hits.push(hit);
                }
            }
        }
        Ok(CutReport {
            removed: self.removed - before.0,
            gouges: self.gouge_count - before.1,
            rapid_hits: self.rapid_hit_count - before.2,
            body_hits: self.body_hit_count - before.3,
        })
    }

    /// Notes where a part of the tool that does not cut is in material during one piece of a
    /// move, cutting or not: the material as it stands before the piece. A part higher than the
    /// stock ever was is not looked at, which is nearly every move.
    fn body_pass(&self, piece: &Piece, cutter: &Cutter, found: &mut Found) {
        let field = &self.stock;
        let lowest = piece.height(0.0).min(piece.height(1.0));
        let (.., y0, y1) = piece.bounds();
        for disc in &cutter.body {
            if !(lowest + disc.above < self.top - self.tolerance) {
                continue;
            }
            // Under the cutter itself the wall's own row of cells is the grid's error, as it is
            // for a cut; a wider shank and a holder are checked to their full width.
            let r = if disc.radius <= cutter.radius + 1e-9 { (disc.radius - WALL_SLACK * field.dx.max(field.dy)).max(0.0) } else { disc.radius };
            let Some((j0, j1)) = field.rows(y0 - r, y1 + r) else { continue };
            for j in j0..=j1 {
                let y = field.y + (j as f64 + 0.5) * field.dy;
                for (i0, i1) in piece.spans(y, r).into_iter().flatten().filter_map(|(lo, hi)| field.columns(lo, hi)) {
                    for i in i0..=i1 {
                        let x = field.x + (i as f64 + 0.5) * field.dx;
                        let Some(height) = field.heights.get(j * field.nx + i) else { continue };
                        for s in piece.over(x, y, r).into_iter().flatten() {
                            // The lowest that part of the tool gets over the cell.
                            let z = piece.height(s.from).min(piece.height(s.to)) + disc.above;
                            let depth = height - z;
                            if depth > self.tolerance && found.body.is_none_or(|worst| depth > worst.depth) {
                                found.body = Some(BodyHit {
                                    operation: found.operation,
                                    move_index: found.move_index,
                                    part: disc.part,
                                    at: Vec3 { x, y, z },
                                    depth,
                                });
                            }
                        }
                    }
                }
            }
        }
    }

    /// One piece of a move of the tool's tip: cutting, it lowers the cells under the tool and
    /// notes the deepest cut below the part; at rapid, it notes where the tool is furthest below
    /// the material's top.
    fn pass(&mut self, piece: &Piece, cutter: &Cutter, cutting: bool, found: &mut Found) {
        self.body_pass(piece, cutter, found);
        let r = cutter.radius;
        let (.., y0, y1) = piece.bounds();
        let Some((j0, j1)) = self.stock.rows(y0 - r, y1 + r) else { return };
        let (left, front, dx, dy) = (self.stock.x, self.stock.y, self.stock.dx, self.stock.dy);
        let inner = (r - WALL_SLACK * dx.max(dy)).max(0.0);
        for j in j0..=j1 {
            let y = front + (j as f64 + 0.5) * dy;
            // The cells of the row whose centres can be under the tool somewhere along the piece.
            for (lo, hi) in piece.spans(y, r).into_iter().flatten() {
                let Some((i0, i1)) = self.stock.columns(lo, hi) else { continue };
                for i in i0..=i1 {
                    let x = left + (i as f64 + 0.5) * dx;
                    // The lowest the tool's surface and its tip get over the cell and the nearest
                    // its axis comes to it, over every stretch the cell is under the tool.
                    let mut under: Option<(f64, f64, f64)> = None;
                    for s in piece.over(x, y, r).into_iter().flatten() {
                        let tip = piece.height(s.from).min(piece.height(s.to));
                        let z = match cutter.profile {
                            Profile::Flat => tip,
                            _ => {
                                let surface = |t: f64| piece.height(t) + cutter.rise(piece.away(x, y, t).min(r));
                                let mut lowest = surface(s.from).min(surface(s.to)).min(surface(s.nearest));
                                for k in 1..=PROFILE_SAMPLES {
                                    lowest = lowest.min(surface(s.from + (s.to - s.from) * k as f64 / (PROFILE_SAMPLES + 1) as f64));
                                }
                                lowest
                            }
                        };
                        under = Some(under.map_or((z, tip, s.least), |(a, b, c)| (a.min(z), b.min(tip), c.min(s.least))));
                    }
                    if let Some((z, tip, least)) = under {
                        self.touch(i, j, Vec3 { x, y, z }, tip, least > inner, cutting, found);
                    }
                }
            }
        }
    }

    /// What the tool does to the cell `i` along X and `j` along Y, whose centre is at `at`'s X
    /// and Y: the lowest the tool's surface gets over it is `at`'s Z, and the lowest its tip
    /// gets `tip`. `edge`: the cell is at the tool's very edge.
    fn touch(&mut self, i: usize, j: usize, at: Vec3, tip: f64, edge: bool, cutting: bool, found: &mut Found) {
        let (operation, move_index) = (found.operation, found.move_index);
        let cell = j * self.stock.nx + i;
        let area = self.stock.dx * self.stock.dy;
        let Some(height) = self.stock.heights.get_mut(cell) else { return };
        // At the tool's very edge a cell is the grid's own error (a wall and the cut beside it
        // share the cell): there the part is neither cut into nor said to be, and a wall the
        // tool stands beside is not material a rapid passes through.
        //
        // There is material, and part, only above the stock's bottom: a tool that has gone
        // through (a drill past the part, a cut into a spoil board) is in air there.
        let within = at.z.max(self.floor);
        if !cutting {
            let under = *height - within;
            if !edge && under > self.tolerance && found.hit.is_none_or(|worst| under > worst.depth) {
                found.hit = Some(RapidHit { operation, move_index, at, depth: under });
            }
            return;
        }
        // How far the tool's tip is below the stock's bottom while it is over the cell.
        self.below_stock = self.below_stock.max(self.floor - tip);
        let part = self.part.as_ref().and_then(|part| part.get(cell)).copied();
        let to = match part {
            Some(part) if edge => within.max(part),
            _ => within,
        };
        if to < *height {
            self.removed += (*height - to) * area;
            *height = to;
        }
        // A cut into the part: where there is none (a hole through it, beside it) its height is
        // below the stock's bottom, and nothing is cut into. It is a gouge when it goes below
        // the part as it stands, and below the part as the model has it by more than the
        // toolpath means to: what an earlier pass was let cut does not let the next go as far
        // again.
        let under = part.map_or(0.0, |part| part - within);
        let below_model = self.model.as_ref().and_then(|model| model.get(cell)).map_or(under, |model| model - within);
        let least = self.tolerance + found.slack;
        if edge || under <= 0.0 {
            return;
        }
        if under > least && below_model > least + found.allowed {
            if found.gouge.is_none_or(|worst| under > worst.depth) {
                found.gouge = Some(Gouge { operation, move_index, at, depth: under });
            }
        } else if found.allowed > 0.0
            && let Some(meant) = self.part.as_mut().and_then(|part| part.get_mut(cell))
        {
            // Cut into, as meant: the part is what the cut leaves there.
            *meant = within;
        }
    }
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
