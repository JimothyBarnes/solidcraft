//! Stock simulation (decision 0015 of the CAM work): the stock as a field of heights over a grid,
//! lowered by every cutting move of a toolpath and checked against the part as it goes. A
//! sampled preview and check, as SPEC §4.4 has it: what it reports is as fine as its cells.
//!
//! One setup cuts from above, so the material left has one height over each point of the table.
//! A tool is its profile: how far its cutting surface is above its tip at each distance from its
//! axis. A cutting move lowers every cell the tool passes over to the tool's surface there: for
//! a flat tool exactly, whatever the move's slope (the lowest the tool gets over a cell is at one
//! end of the stretch during which the cell is under it); for other profiles the lowest of a few
//! places along that stretch. A gouge is a cell lowered below the part; a rapid that passes below
//! the material's top is a hit.

use std::f64::consts::PI;

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
/// The most chords an arc is cut as.
const MAX_CHORDS: usize = 10_000;
/// How far inside the tool's edge a cell must be for a cut there to cut into the part, in
/// cells: along a wall the part and the cut share a row of cells, and that row is the grid's
/// own error. Nearer the edge than this the part's own height stands, and no gouge is counted.
const WALL_SLACK: f64 = 0.75;
/// How many places along the stretch a cell is under a tool that is not flat are tried, besides
/// its ends and the nearest.
const PROFILE_SAMPLES: usize = 5;

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

/// A tool as the simulation cuts with it: a radius and the shape below it.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Cutter {
    radius: f64,
    profile: Profile,
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
            _ => Profile::Flat,
        };
        Ok(Cutter { radius, profile })
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

/// What one move of a toolpath has been found to do, as its pieces are passed.
struct Found {
    operation: usize,
    move_index: usize,
    /// Its deepest cut below the part, and where as a rapid it was furthest below the material.
    gouge: Option<Gouge>,
    hit: Option<RapidHit>,
}

/// What one toolpath did to the stock.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct CutReport {
    /// The volume it removed, mm³.
    pub removed: f64,
    /// How many of its moves cut into the part, and how many of its rapids passed through material.
    pub gouges: usize,
    pub rapid_hits: usize,
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
            tolerance,
            removed: 0.0,
            gouge_count: 0,
            gouges: Vec::new(),
            rapid_hit_count: 0,
            rapid_hits: Vec::new(),
            below_stock: 0.0,
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
        Ok(self)
    }

    /// The part's heights, when the simulation has them.
    pub fn part(&self) -> Option<&[f64]> {
        self.part.as_deref()
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
        toolpath.validate()?;
        let cutter = Cutter::of(tool)?;
        let motion = CycleMotion::default();
        let before = (self.removed, self.gouge_count, self.rapid_hit_count);
        let mut plain: Vec<Move> = Vec::new();
        for (index, (from, m)) in toolpath.walk().enumerate() {
            plain.clear();
            match *m {
                Move::Drill { to, r, retract, cycle, feed } => cycle::expand(from, to, r, retract, cycle, feed, &motion, &mut plain),
                other => plain.push(other),
            }
            let mut at = from;
            let mut found = Found { operation, move_index: index, gouge: None, hit: None };
            for step in &plain {
                match *step {
                    Move::Rapid { to } => self.pass(at, to, &cutter, false, &mut found),
                    Move::Linear { to, .. } => self.pass(at, to, &cutter, true, &mut found),
                    Move::Arc { to, center, plane, direction, .. } => {
                        let arc = ArcGeom::new(at, to, center, plane, direction).map_err(|reason| Error::BadMove { index, reason })?;
                        // Chords that stay within a quarter of a cell of the arc.
                        let sag = 0.25 * self.stock.dx.min(self.stock.dy);
                        let each = if sag >= arc.radius { 0.5 * PI } else { 2.0 * (1.0 - sag / arc.radius).acos() };
                        let chords = ((arc.sweep.abs() / each).ceil() as usize).clamp(1, MAX_CHORDS);
                        let mut from = at;
                        for k in 1..=chords {
                            let end = if k == chords { to } else { arc.point(k as f64 / chords as f64) };
                            self.pass(from, end, &cutter, true, &mut found);
                            from = end;
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
        }
        Ok(CutReport { removed: self.removed - before.0, gouges: self.gouge_count - before.1, rapid_hits: self.rapid_hit_count - before.2 })
    }

    /// One straight move of the tool's tip from `a` to `b`: cutting, it lowers the cells under
    /// the tool and notes the deepest cut below the part; at rapid, it notes where the tool is
    /// furthest below the material's top.
    fn pass(&mut self, a: Vec3, b: Vec3, cutter: &Cutter, cutting: bool, found: &mut Found) {
        let (operation, move_index) = (found.operation, found.move_index);
        let field = &mut self.stock;
        let r = cutter.radius;
        // The cells whose centres can be under the tool somewhere along the move.
        let Some((i0, i1)) = field.columns(a.x.min(b.x) - r, a.x.max(b.x) + r) else { return };
        let Some((j0, j1)) = field.rows(a.y.min(b.y) - r, a.y.max(b.y) + r) else { return };
        let (dx, dy, dz) = (b.x - a.x, b.y - a.y, b.z - a.z);
        let along = dx * dx + dy * dy;
        let area = field.dx * field.dy;
        let inner = (r - WALL_SLACK * field.dx.max(field.dy)).max(0.0);
        for j in j0..=j1 {
            let y = field.y + (j as f64 + 0.5) * field.dy;
            for i in i0..=i1 {
                let x = field.x + (i as f64 + 0.5) * field.dx;
                let (px, py) = (x - a.x, y - a.y);
                // The stretch of the move (as shares of it) during which the cell is under the
                // tool, and how near the tool's axis comes to it.
                let (t0, t1, nearest, least) = if along <= 1e-18 {
                    let out = px.hypot(py);
                    if out > r {
                        continue;
                    }
                    (0.0, 1.0, 0.0, out)
                } else {
                    let middle = (px * dx + py * dy) / along;
                    let off = (px * dy - py * dx).abs() / along.sqrt();
                    if off > r {
                        continue;
                    }
                    let half = (r * r - off * off).sqrt() / along.sqrt();
                    let (t0, t1) = ((middle - half).max(0.0), (middle + half).min(1.0));
                    if t0 > t1 {
                        continue;
                    }
                    let nearest = middle.clamp(0.0, 1.0);
                    (t0, t1, nearest, (px - nearest * dx).hypot(py - nearest * dy))
                };
                // The lowest the tool's surface gets over the cell.
                let surface = |t: f64| a.z + t * dz + cutter.rise((px - t * dx).hypot(py - t * dy).min(r));
                let z = match cutter.profile {
                    Profile::Flat => (a.z + t0 * dz).min(a.z + t1 * dz),
                    _ => {
                        let mut lowest = surface(t0).min(surface(t1)).min(surface(nearest));
                        for k in 1..=PROFILE_SAMPLES {
                            lowest = lowest.min(surface(t0 + (t1 - t0) * k as f64 / (PROFILE_SAMPLES + 1) as f64));
                        }
                        lowest
                    }
                };
                let Some(height) = field.heights.get_mut(j * field.nx + i) else { continue };
                // At the tool's very edge a cell is the grid's own error (a wall and the cut
                // beside it share the cell, and an arc is cut as chords a hair inside it): there
                // the part is neither cut into nor said to be, and a wall the tool stands
                // beside is not material a rapid passes through.
                let edge = least > inner;
                // There is material, and part, only above the stock's bottom: a tool that has
                // gone through (a drill past the part, a cut into a spoil board) is in air there.
                let within = z.max(self.floor);
                if !cutting {
                    let under = *height - within;
                    if !edge && under > self.tolerance && found.hit.is_none_or(|worst| under > worst.depth) {
                        found.hit = Some(RapidHit { operation, move_index, at: Vec3 { x, y, z }, depth: under });
                    }
                    continue;
                }
                // How far the tool's tip is below the stock's bottom while it is over the cell.
                self.below_stock = self.below_stock.max(self.floor - (a.z + t0 * dz).min(a.z + t1 * dz));
                let part = self.part.as_ref().and_then(|part| part.get(j * field.nx + i)).copied();
                let to = match part {
                    Some(part) if edge => within.max(part),
                    _ => within,
                };
                if to < *height {
                    self.removed += (*height - to) * area;
                    *height = to;
                }
                // A cut into the part: where there is none (a hole through it, beside it) its
                // height is below the stock's bottom, and nothing is cut into.
                if let Some(part) = part
                    && !edge
                {
                    let under = part - within;
                    if under > self.tolerance && found.gouge.is_none_or(|worst| under > worst.depth) {
                        found.gouge = Some(Gouge { operation, move_index, at: Vec3 { x, y, z }, depth: under });
                    }
                }
            }
        }
    }
}

#[cfg(test)]
#[path = "sim_tests.rs"]
mod tests;
