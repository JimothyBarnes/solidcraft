//! Offsetting regions made of lines and arcs (decision 0007 of the CAM work): the base of contour,
//! pocket and the other 2D strategies.
//!
//! The work is done by `cavalier_contours` (MIT OR Apache-2.0, the crate `solidcraft-io` reads
//! Gerber layers with); this module is the only place in the CAM engine that names it. Regions
//! come in and go out as [`Region2`], checked on the way in and on the way out, so it can be
//! replaced by our own offsetter, or by a kernel primitive, without a caller changing.
//!
//! A positive distance moves the boundary into the region: it shrinks, as the path of a tool's
//! centre does inside a pocket. A negative one moves it out. Lines stay lines and arcs stay arcs; a
//! corner the offset opens up is rounded with an arc of the offset's radius about it, so every
//! point of the result lies the distance from the source's boundary. A region may vanish, split
//! into several, or lose a hole into its outline; regions grown together become one.

use std::f64::consts::PI;

use cavalier_contours::core::math::Vector2;
use cavalier_contours::polyline::{PlineSource, PlineSourceMut, Polyline};
use cavalier_contours::shape_algorithms::{Shape, ShapeOffsetOptions};
use solidcore_record::{Loop2, Region2, Seg2, Vec2};
use solidcraft_geom::{Loop2Geom, Seg2Geom};

use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most segments the regions of one offset may have together.
pub const MAX_OFFSET_SEGMENTS: usize = 100_000;
/// How far one segment's end may be from the start of the next and still join it, mm.
pub const JOINT_TOLERANCE: f64 = 1e-6;
/// Segments shorter than this are left out, mm.
const MIN_SEGMENT: f64 = 1e-9;
/// Loops enclosing less than this are no loop: refused coming in, dust going out, mm².
const MIN_AREA: f64 = 1e-9;
/// A bulge (the tangent of a quarter of an arc's sweep) smaller than this is a line.
const LINE_BULGE: f64 = 1e-12;

fn bad<T>(message: String) -> Result<T> {
    Err(Error::BadValue(message))
}

/// `region` with its boundary moved `distance` mm into it (negative: out of it).
pub fn offset_region(region: &Region2, distance: f64) -> Result<Vec<Region2>> {
    offset_regions(std::slice::from_ref(region), distance)
}

/// `regions` (which do not overlap) with their boundaries moved `distance` mm into them (negative:
/// out of them): the regions left, none when they vanish. Outer loops come back counter-clockwise
/// and holes clockwise, each hole in the region it lies in.
pub fn offset_regions(regions: &[Region2], distance: f64) -> Result<Vec<Region2>> {
    if !(distance.abs() <= MAX_COORDINATE) {
        return bad(format!("an offset distance must be finite and within {MAX_COORDINATE} mm, not {distance}"));
    }
    let mut segments = 0;
    let mut plines = Vec::new();
    for (i, region) in regions.iter().enumerate() {
        plines.push(pline_of(&region.outer, true, &format!("region {i}'s outer loop"), &mut segments)?);
        for (k, hole) in region.holes.iter().enumerate() {
            plines.push(pline_of(hole, false, &format!("region {i}'s hole {k}"), &mut segments)?);
        }
    }
    if plines.is_empty() {
        return Ok(Vec::new());
    }
    let shape = Shape::from_plines(plines);
    let moved = if distance == 0.0 { shape } else { shape.parallel_offset(distance, &ShapeOffsetOptions::new()) };
    regions_of(&moved)
}

/// A checked loop as a closed polyline, counter-clockwise when `ccw` and clockwise otherwise.
fn pline_of(l: &Loop2, ccw: bool, what: &str, segments: &mut usize) -> Result<Polyline<f64>> {
    *segments += l.segs.len();
    if l.segs.is_empty() || *segments > MAX_OFFSET_SEGMENTS {
        return bad(format!("{what}: 1 to {MAX_OFFSET_SEGMENTS} segments (all loops together)"));
    }
    for seg in &l.segs {
        if !matches!(seg, Seg2::Line { .. } | Seg2::Arc { .. }) {
            return bad(format!("{what}: only lines and arcs can be offset; fit arcs to its curves first"));
        }
        seg.check(MAX_COORDINATE).map_err(|e| Error::BadValue(format!("{what}: {e}")))?;
    }
    for (a, b) in l.joints() {
        let (end, start) = (a.end(), b.start());
        let gap = (end.x - start.x).hypot(end.y - start.y);
        if !(gap <= JOINT_TOLERANCE) {
            return bad(format!("{what} is not closed: a gap of {gap} mm between two of its segments"));
        }
    }
    let mut p = Polyline::new_closed();
    for seg in &l.segs {
        match *seg {
            Seg2::Line { a, b } => {
                if (b.x - a.x).hypot(b.y - a.y) >= MIN_SEGMENT {
                    p.add(a.x, a.y, 0.0);
                }
            }
            Seg2::Arc { center, radius, start, sweep } => {
                if radius * sweep.abs() < MIN_SEGMENT {
                    continue;
                }
                // The polyline's arcs turn at most half a circle each.
                let pieces = (sweep.abs() / PI - 1e-12).ceil().max(1.0);
                let step = sweep / pieces;
                let bulge = (0.25 * step).tan();
                for k in 0..pieces as usize {
                    let angle = start + step * k as f64;
                    p.add(center.x + radius * angle.cos(), center.y + radius * angle.sin(), bulge);
                }
            }
            Seg2::Cubic { .. } | Seg2::Conic { .. } | Seg2::Bezier { .. } => {}
        }
    }
    let area = if p.vertex_count() < 2 { 0.0 } else { p.area() };
    if !(area.abs() >= MIN_AREA) {
        return bad(format!("{what} encloses no area"));
    }
    if (area > 0.0) != ccw {
        p.invert_direction_mut();
    }
    Ok(p)
}

/// The shape's loops as regions: each counter-clockwise loop an outer loop, each clockwise one a
/// hole of the smallest outer loop it lies in.
fn regions_of(shape: &Shape<f64>) -> Result<Vec<Region2>> {
    let mut outers: Vec<(&Polyline<f64>, f64, Region2)> = Vec::new();
    for outer in &shape.ccw_plines {
        if let Some(l) = loop_of(&outer.polyline)? {
            outers.push((&outer.polyline, outer.polyline.area().abs(), Region2 { outer: l, holes: Vec::new() }));
        }
    }
    for hole in &shape.cw_plines {
        let Some(l) = loop_of(&hole.polyline)? else { continue };
        let Some(at) = hole.polyline.get(0) else { continue };
        let owner = outers.iter_mut().filter(|(outer, _, _)| outer.winding_number(Vector2::new(at.x, at.y)) != 0).min_by(|a, b| a.1.total_cmp(&b.1));
        match owner {
            Some((_, _, region)) => region.holes.push(l),
            None => return bad("the offset left a hole outside every region: the regions given overlap or cross themselves".into()),
        }
    }
    Ok(outers.into_iter().map(|(_, _, region)| region).collect())
}

/// A closed polyline as a loop of lines and arcs; `None` for dust. Each arc is checked to end on
/// the next vertex.
fn loop_of(p: &Polyline<f64>) -> Result<Option<Loop2>> {
    let n = p.vertex_count();
    if n < 2 || !(p.area().abs() >= MIN_AREA) {
        return Ok(None);
    }
    let mut segs = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(v), Some(w)) = (p.get(i), p.get((i + 1) % n)) else { continue };
        let (a, b) = (Vec2 { x: v.x, y: v.y }, Vec2 { x: w.x, y: w.y });
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let chord = dx.hypot(dy);
        if chord < MIN_SEGMENT {
            continue;
        }
        if v.bulge.abs() < LINE_BULGE {
            segs.push(Seg2::Line { a, b });
            continue;
        }
        // The bulge is the tangent of a quarter of the sweep, positive counter-clockwise. The
        // centre is off the chord's middle along its left normal.
        let bulge = v.bulge;
        let sweep = 4.0 * bulge.atan();
        let radius = chord * (1.0 + bulge * bulge) / (4.0 * bulge.abs());
        let off = (1.0 - bulge * bulge) / (4.0 * bulge);
        let center = Vec2 { x: a.x + 0.5 * dx - dy * off, y: a.y + 0.5 * dy + dx * off };
        let start = (a.y - center.y).atan2(a.x - center.x);
        let arc = Seg2::Arc { center, radius, start, sweep };
        let end = arc.end();
        let miss = (end.x - b.x).hypot(end.y - b.y);
        if !(miss <= 1e-9 + 1e-12 * (radius + b.x.abs() + b.y.abs())) {
            return bad(format!("an offset arc ends {miss} mm from the next segment"));
        }
        segs.push(arc);
    }
    Ok((!segs.is_empty()).then_some(Loop2 { segs }))
}

#[cfg(test)]
#[path = "offset_tests.rs"]
mod tests;
