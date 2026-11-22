//! Offsetting regions made of lines and arcs (decision 0007 of the CAM work): the base of contour,
//! pocket and the other 2D strategies.
//!
//! A positive distance moves the boundary into the region: it shrinks, as the path of a tool's
//! centre does inside a pocket. A negative one moves it out. Lines stay lines and arcs stay arcs; a
//! corner the offset opens up is rounded with an arc of the offset's radius about it, so every
//! point of the result lies the distance from the source's boundary. A region may vanish, split
//! into several, or lose a hole into its outline; regions grown together become one.
//!
//! The work is done by `cavalier_contours` (MIT OR Apache-2.0, the crate `solidcraft-io` reads
//! Gerber layers with); this module is the only place in the CAM engine that names it. It is
//! exact on ordinary lines and arcs and unreliable on curves chopped into thousands of tiny sides,
//! in ways its settings can't be trusted to fix. A wrong offset is a gouge or an uncut wall, so
//! nothing it returns is believed: **every result is verified here**, both ways, with our own
//! distances ([`verify`]). An attempt that fails, or panics inside the crate, is tried again at
//! another scale; if none verifies, the answer is an error. An unverified offset is never
//! returned. Regions come in and go out as [`Region2`], so the crate can be replaced by our own
//! offsetter, or by a kernel primitive, behind the same check and tests.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use cavalier_contours::core::math::Vector2;
use cavalier_contours::polyline::{PlineSource, PlineSourceMut, Polyline};
use cavalier_contours::shape_algorithms::{Shape, ShapeOffsetOptions};
use cavalier_contours::static_aabb2d_index::{Control, StaticAABB2DIndex, StaticAABB2DIndexBuilder};
use solidcore_record::{Loop2, Region2, Seg2, Vec2};
use solidcraft_geom::{Loop2Geom, Seg2Geom};

use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most segments the regions of one offset may have together.
pub const MAX_OFFSET_SEGMENTS: usize = 100_000;
/// How far one segment's end may be from the start of the next and still join it, mm.
pub const JOINT_TOLERANCE: f64 = 1e-6;
/// How far a point of an offset may be from where it belongs, mm: what every result is verified
/// to. On ordinary lines and arcs results are a million times closer than this.
pub const OFFSET_TOLERANCE: f64 = 1e-4;
/// Segments shorter than this are left out, mm.
const MIN_SEGMENT: f64 = 1e-9;
/// Loops enclosing less than this are no loop: refused coming in, dust going out, mm².
const MIN_AREA: f64 = 1e-9;
/// A bulge (the tangent of a quarter of an arc's sweep) smaller than this is a line.
const LINE_BULGE: f64 = 1e-12;
/// The scales an offset is tried at, in turn. The crate's tolerances are absolute, so a larger
/// scale makes them finer against the shape. Powers of two lose no precision.
const SCALES: [f64; 3] = [1.0, 128.0, 16_384.0];
/// How much deeper than the offset a region must be at a place for the result to have to pass
/// there, in tolerances: slivers thinner than this may be dropped.
const DEEPER: f64 = 10.0;

fn bad<T>(message: String) -> Result<T> {
    Err(Error::BadValue(message))
}

fn point(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn apart(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// `region` with its boundary moved `distance` mm into it (negative: out of it).
pub fn offset_region(region: &Region2, distance: f64) -> Result<Vec<Region2>> {
    offset_regions(std::slice::from_ref(region), distance)
}

/// `regions` (which do not overlap) with their boundaries moved `distance` mm into them (negative:
/// out of them): the regions left, none when they vanish. Outer loops come back counter-clockwise
/// and holes clockwise, each hole in the region it lies in. Every point of the result is verified
/// to lie within [`OFFSET_TOLERANCE`] of where it belongs; a shape that can't be offset that
/// reliably is an error.
pub fn offset_regions(regions: &[Region2], distance: f64) -> Result<Vec<Region2>> {
    if !(distance.abs() <= MAX_COORDINATE) {
        return bad(format!("an offset distance must be finite and within {MAX_COORDINATE} mm, not {distance}"));
    }
    let mut segments = 0;
    let mut source: Vec<Vec<Loop2>> = Vec::with_capacity(regions.len());
    for (i, region) in regions.iter().enumerate() {
        let mut loops = vec![checked(&region.outer, true, &format!("region {i}'s outer loop"), &mut segments)?];
        for (k, hole) in region.holes.iter().enumerate() {
            loops.push(checked(hole, false, &format!("region {i}'s hole {k}"), &mut segments)?);
        }
        source.push(loops);
    }
    if source.is_empty() {
        return Ok(Vec::new());
    }
    if distance == 0.0 {
        return Ok(source.into_iter().map(|mut loops| Region2 { outer: loops.remove(0), holes: loops }).collect());
    }
    let Some(boundary) = Boundary::new(source.iter().flatten().flat_map(|l| l.segs.iter().copied()).collect()) else {
        return bad("the outline could not be indexed".into());
    };
    let mut why = String::new();
    for scale in SCALES {
        match attempt(&source, distance, scale).and_then(|result| verify(&source, &boundary, &result, distance).map(|()| result)) {
            Ok(result) => return Ok(result),
            Err(reason) => why = reason,
        }
    }
    bad(format!("this outline could not be offset by {distance} mm reliably ({why}); fit arcs to it or simplify it first"))
}

/// A loop checked and put in order: only lines and arcs, finite, closed, counter-clockwise when
/// `ccw` and clockwise otherwise, its vanishing segments left out.
fn checked(l: &Loop2, ccw: bool, what: &str, segments: &mut usize) -> Result<Loop2> {
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
        let gap = apart(a.end(), b.start());
        if !(gap <= JOINT_TOLERANCE) {
            return bad(format!("{what} is not closed: a gap of {gap} mm between two of its segments"));
        }
    }
    let kept = Loop2 { segs: l.segs.iter().copied().filter(|s| s.length() >= MIN_SEGMENT).collect() };
    let area = kept.signed_area();
    if kept.segs.is_empty() || !(area.abs() >= MIN_AREA) {
        return bad(format!("{what} encloses no area"));
    }
    Ok(if (area > 0.0) == ccw { kept } else { kept.reversed() })
}

/// A checked loop as a closed polyline, its coordinates times `scale`.
fn pline_of(l: &Loop2, scale: f64) -> Polyline<f64> {
    let mut p = Polyline::new_closed();
    for seg in &l.segs {
        match *seg {
            Seg2::Arc { center, radius, start, sweep } => {
                // The polyline's arcs turn at most half a circle each.
                let pieces = (sweep.abs() / PI - 1e-12).ceil().max(1.0);
                let step = sweep / pieces;
                let bulge = (0.25 * step).tan();
                for k in 0..pieces as usize {
                    let angle = start + step * k as f64;
                    p.add(scale * (center.x + radius * angle.cos()), scale * (center.y + radius * angle.sin()), bulge);
                }
            }
            _ => {
                let a = seg.start();
                p.add(scale * a.x, scale * a.y, 0.0);
            }
        }
    }
    p
}

/// One try at the offset with the crate, at `scale`: the regions it gives, unverified, or why it
/// gave none. A panic inside the crate (its debug builds assert on some inputs) is a failed try.
fn attempt(source: &[Vec<Loop2>], distance: f64, scale: f64) -> std::result::Result<Vec<Region2>, String> {
    let plines: Vec<Polyline<f64>> = source.iter().flatten().map(|l| pline_of(l, scale)).collect();
    let moved = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Shape::from_plines(plines).parallel_offset(distance * scale, &ShapeOffsetOptions::new())
    }))
    .map_err(|_| "the offsetter gave up on it".to_string())?;
    regions_of(&moved, scale)
}

/// The shape's loops as regions, their coordinates divided by `scale`: each counter-clockwise loop
/// an outer loop, each clockwise one a hole of the smallest outer loop it lies in.
fn regions_of(shape: &Shape<f64>, scale: f64) -> std::result::Result<Vec<Region2>, String> {
    let mut outers: Vec<(&Polyline<f64>, f64, Region2)> = Vec::new();
    for outer in &shape.ccw_plines {
        if let Some(l) = loop_of(&outer.polyline, scale)? {
            outers.push((&outer.polyline, outer.polyline.area().abs(), Region2 { outer: l, holes: Vec::new() }));
        }
    }
    for hole in &shape.cw_plines {
        let Some(l) = loop_of(&hole.polyline, scale)? else { continue };
        let Some(at) = hole.polyline.get(0) else { continue };
        let owner = outers.iter_mut().filter(|(outer, _, _)| outer.winding_number(Vector2::new(at.x, at.y)) != 0).min_by(|a, b| a.1.total_cmp(&b.1));
        match owner {
            Some((_, _, region)) => region.holes.push(l),
            None => return Err("it left a hole outside every region".into()),
        }
    }
    Ok(outers.into_iter().map(|(_, _, region)| region).collect())
}

/// A closed polyline as a loop of lines and arcs, its coordinates divided by `scale`; `None` for
/// dust. Each arc is checked to end on the next vertex.
fn loop_of(p: &Polyline<f64>, scale: f64) -> std::result::Result<Option<Loop2>, String> {
    let n = p.vertex_count();
    if n < 2 || !(p.area().abs() / (scale * scale) >= MIN_AREA) {
        return Ok(None);
    }
    let mut segs = Vec::with_capacity(n);
    for i in 0..n {
        let (Some(v), Some(w)) = (p.get(i), p.get((i + 1) % n)) else { continue };
        let (a, b) = (point(v.x / scale, v.y / scale), point(w.x / scale, w.y / scale));
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
        let center = point(a.x + 0.5 * dx - dy * off, a.y + 0.5 * dy + dx * off);
        let arc = Seg2::Arc { center, radius, start: (a.y - center.y).atan2(a.x - center.x), sweep };
        let miss = apart(arc.end(), b);
        if !(miss <= 1e-9 + 1e-12 * (radius + b.x.abs() + b.y.abs())) {
            return Err(format!("an arc of it ends {miss} mm from the next segment"));
        }
        segs.push(arc);
    }
    Ok((!segs.is_empty()).then_some(Loop2 { segs }))
}

/// The segments of some loops with an index over their boxes, for nearest distances.
struct Boundary {
    segs: Vec<Seg2>,
    index: StaticAABB2DIndex<f64>,
}

impl Boundary {
    fn new(segs: Vec<Seg2>) -> Option<Boundary> {
        if segs.is_empty() {
            return None;
        }
        let mut builder = StaticAABB2DIndexBuilder::new(segs.len());
        for s in &segs {
            let (lo, hi) = box_of(s);
            builder.add(lo.x, lo.y, hi.x, hi.y);
        }
        let index = builder.build().ok()?;
        Some(Boundary { segs, index })
    }

    /// The distance from `p` to the nearest segment: boxes are visited nearest first, until the
    /// next box is farther than the nearest segment found.
    fn distance(&self, p: Vec2) -> f64 {
        let mut best = f64::INFINITY;
        let mut visit = |i: usize, box_distance_squared: f64| -> Control<()> {
            if box_distance_squared > best * best {
                return Control::Break(());
            }
            if let Some(s) = self.segs.get(i) {
                best = best.min(distance_to(p, s));
            }
            Control::Continue
        };
        let _: Control<()> = self.index.visit_neighbors(p.x, p.y, &mut visit);
        best
    }
}

/// Whether `angle` is on an arc from `start` sweeping `sweep`.
fn on_arc(angle: f64, start: f64, sweep: f64) -> bool {
    let along = if sweep >= 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
    along <= sweep.abs()
}

/// A box around a line or arc, a hair wider than it.
fn box_of(s: &Seg2) -> (Vec2, Vec2) {
    let (a, b) = (s.start(), s.end());
    let (mut lo, mut hi) = (point(a.x.min(b.x), a.y.min(b.y)), point(a.x.max(b.x), a.y.max(b.y)));
    if let Seg2::Arc { center, radius, start, sweep } = *s {
        // An arc also reaches the east, north, west and south points of its circle that it passes.
        for quarter in 0..4 {
            let angle = FRAC_PI_2 * f64::from(quarter);
            if on_arc(angle, start, sweep) {
                let p = point(center.x + radius * angle.cos(), center.y + radius * angle.sin());
                lo = point(lo.x.min(p.x), lo.y.min(p.y));
                hi = point(hi.x.max(p.x), hi.y.max(p.y));
            }
        }
    }
    let pad = 1e-9 * (1.0 + lo.x.abs().max(lo.y.abs()).max(hi.x.abs()).max(hi.y.abs()));
    (point(lo.x - pad, lo.y - pad), point(hi.x + pad, hi.y + pad))
}

/// The distance from `p` to a line or arc.
fn distance_to(p: Vec2, s: &Seg2) -> f64 {
    match *s {
        Seg2::Arc { center, radius, start, sweep } => {
            if on_arc((p.y - center.y).atan2(p.x - center.x), start, sweep) {
                (apart(p, center) - radius).abs()
            } else {
                apart(p, s.start()).min(apart(p, s.end()))
            }
        }
        _ => {
            let (a, b) = (s.start(), s.end());
            let (ux, uy) = (b.x - a.x, b.y - a.y);
            let length_squared = ux * ux + uy * uy;
            let t = if length_squared > 0.0 { (((p.x - a.x) * ux + (p.y - a.y) * uy) / length_squared).clamp(0.0, 1.0) } else { 0.0 };
            apart(p, point(a.x + t * ux, a.y + t * uy))
        }
    }
}

/// A point of a line or arc at `t` (0 to 1) and the unit normal to the left of its direction of
/// travel there.
fn left_of(s: &Seg2, t: f64) -> (Vec2, Vec2) {
    match *s {
        Seg2::Arc { center, radius, start, sweep } => {
            let angle = start + t * sweep;
            let (sin, cos) = angle.sin_cos();
            // Left of a counter-clockwise arc is toward its centre; of a clockwise one, away.
            let toward = if sweep >= 0.0 { -1.0 } else { 1.0 };
            (point(center.x + radius * cos, center.y + radius * sin), point(toward * cos, toward * sin))
        }
        _ => {
            let (a, b) = (s.start(), s.end());
            let (ux, uy) = (b.x - a.x, b.y - a.y);
            let length = ux.hypot(uy).max(f64::MIN_POSITIVE);
            (point(a.x + t * ux, a.y + t * uy), point(-uy / length, ux / length))
        }
    }
}

/// How many times the loop `l` winds counter-clockwise about `p`, which is not on it: the turns
/// of the direction from `p` to a point going round the loop.
pub(crate) fn winding(p: Vec2, l: &Loop2) -> i64 {
    let mut turned = 0.0;
    for s in &l.segs {
        let (a, b) = (s.start(), s.end());
        let (ax, ay, bx, by) = (a.x - p.x, a.y - p.y, b.x - p.x, b.y - p.y);
        let mut turn = (ax * by - ay * bx).atan2(ax * bx + ay * by);
        if let Seg2::Arc { center, radius, sweep, .. } = *s
            && apart(p, center) < radius
        {
            // Seen from inside its circle an arc turns the way it sweeps, a full circle once round.
            if sweep.abs() >= TAU - 1e-9 {
                turn = TAU.copysign(sweep);
            } else if sweep > 0.0 && turn < 0.0 {
                turn += TAU;
            } else if sweep < 0.0 && turn > 0.0 {
                turn -= TAU;
            }
        }
        turned += turn;
    }
    (turned / TAU).round() as i64
}

/// Checks that `result` is `source` offset by `distance`, with our own distances and nothing of
/// the offsetter's:
/// - every segment of the result, at points along it, lies the distance from the source's
///   boundary within [`OFFSET_TOLERANCE`], and each of its loops on the right side of it (inside
///   the source when it shrank, outside when it grew);
/// - wherever the source is clearly deeper than the distance (by [`DEEPER`] tolerances, so the
///   offset there can't have been cut away by another part of the boundary), the result passes
///   within the tolerance of where the offset must be. A loop that vanished fails here.
///
/// `source` holds each region's loops in order, outer counter-clockwise and holes clockwise, and
/// `boundary` all their segments.
fn verify(source: &[Vec<Loop2>], boundary: &Boundary, result: &[Region2], distance: f64) -> std::result::Result<(), String> {
    let (d, tol) = (distance.abs(), OFFSET_TOLERANCE);
    for l in result.iter().flat_map(Region2::loops) {
        for (k, s) in l.segs.iter().enumerate() {
            let along: &[f64] = if matches!(s, Seg2::Arc { .. }) { &[0.0, 0.25, 0.5, 0.75] } else { &[0.0, 0.5] };
            for &t in along {
                let far = boundary.distance(left_of(s, t).0);
                if !((far - d).abs() <= tol) {
                    return Err(format!("a point of the result is {far} mm from the outline"));
                }
            }
            if k == 0 {
                let p = left_of(s, 0.5).0;
                let inside = source.iter().flatten().map(|l| winding(p, l)).sum::<i64>() > 0;
                if inside != (distance > 0.0) {
                    return Err("part of the result is on the wrong side of the outline".into());
                }
            }
        }
    }
    let found = Boundary::new(result.iter().flat_map(Region2::loops).flat_map(|l| l.segs.iter().copied()).collect());
    let deeper = distance + (DEEPER * tol).copysign(distance);
    for s in source.iter().flatten().flat_map(|l| &l.segs) {
        for t in [0.25, 0.5, 0.75] {
            let (p, n) = left_of(s, t);
            let probe = point(p.x + deeper * n.x, p.y + deeper * n.y);
            if boundary.distance(probe) < deeper.abs() - 0.1 * tol {
                continue;
            }
            let must = point(p.x + distance * n.x, p.y + distance * n.y);
            let near = found.as_ref().map_or(f64::INFINITY, |f| f.distance(must));
            if !(near <= tol) {
                return Err(format!("the result is missing where the offset must pass ({near} mm away at {}, {})", must.x, must.y));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[path = "offset_tests.rs"]
mod tests;
