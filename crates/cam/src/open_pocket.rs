//! A pocket open at a side (decision 0035 of the CAM work): a step, a shoulder, a channel open
//! at an end, the floor round a boss. It has no wall all round to shrink rings from, so it is
//! cut in passes along the walls it has, the one furthest from them first and the last one on
//! them, as a contour's passes are. A pass is where the tool's centre is one distance from
//! everything that stands above the floor (the verified offsetter's own loops, the walls moved
//! out), kept where the cutter is over the floor: so each pass takes one stepover, whatever the
//! floor's shape, and the first of them comes in from the open side.
//!
//! Where a wall ends at an open edge the pass does not roll round its end: it goes straight
//! on. And where the stock beside the part is thin, a pass is lengthened straight through it at
//! both ends, so that the tool goes down in the air beside the stock and comes in from the side
//! (at half the feed until it is in the cut, as a tool maker's guide asks for a side entry). A
//! pass that can't be started in the air is started from the pass before, in the air that one
//! cut; or cut from its other end; or, with nothing else left, entered down a ramp along itself.
//!
//! A contour along an open chain of edges (decision 0043) is cut the same way: its passes are
//! the stretches of these loops that run along the chain, rather than over a floor.

use solidcore_record::{Loop2, Region2, Seg2, Vec2};
use solidcraft_geom::{Loop2Geom, Region2Geom, Seg2Geom};

use crate::Result;
use crate::contour::{CutDirection, DEFAULT_CONTOUR_CLEAR, apart, bad, moved_at_least};
use crate::offset::JOINT_TOLERANCE;
use crate::pocket::{Entry, MAX_POCKET_RINGS, Step, nearest, started_at};
use crate::units::MAX_COORDINATE;

/// How far over the floor the cutter must reach for a pass to be cut there, mm.
const TOUCH: f64 = 0.01;
/// The shortest stretch of a pass that is cut, mm: a shorter one is the cutter grazing the
/// floor's edge.
const MIN_RUN: f64 = 0.1;
/// How far apart a pass is asked whether it is over the floor, mm at most (and no more than a
/// quarter of the tool's radius), and the most places one pass is asked at.
const ASK_STEP: f64 = 0.25;
const ASK_MOST: f64 = 50_000.0;
/// How near the place where the answer changes along a pass is found, mm.
const ASK_EDGE: f64 = 1e-3;
/// A corner of the walls that turns the tool by less than this is a joint in a curved wall, not
/// a wall's end, radians.
const CORNER_TURN: f64 = 15.0 * std::f64::consts::PI / 180.0;
/// How near a corner of the walls an arc's centre is for the arc to be the pass going round
/// that corner, mm.
const AT_CORNER: f64 = 1e-6;
/// How far along the two sides of a corner of the walls they are asked whether they border the
/// floor, and how near the floor's outline a place on a wall is for it to border it, mm: a
/// fillet at the foot of a wall keeps the floor's edge a little off the wall.
const PROBE: f64 = 2.0;
const BORDER: f64 = 0.5;
/// How much stock beside the part a pass is lengthened through to start or end in the air, in
/// tool diameters: past that the stock there is no thin skin and the pass stops where it is.
const THROUGH: f64 = 1.0;
/// How much nearer the walls than its pass's own distance a lengthening or a way between two
/// passes may come, mm: the offsetter's tolerance each way.
const WAY_SLACK: f64 = 2e-4;
/// How much further than the distance between two passes the way from one to the next may be,
/// mm (as between a closed pocket's rings).
const LINK_SLACK: f64 = 5e-3;
/// The most places across the floor that are asked whether the passes reach them.
const REACH_PLACES: f64 = 60.0;
/// How many places across the floor are asked how far they are from the walls.
const ACROSS_PLACES: usize = 40;
/// How much nearer the walls the pass on them is taken so that a tool exactly as wide as a
/// channel goes down its middle, mm. At the pass's own distance the channel's two walls give one
/// and the same line, which the offsetter may call a way through or none; a micron nearer it
/// is a way through for certain, and a micron is what a program's numbers resolve.
const FIT: f64 = 1e-3;
/// How near the pass on the walls a place of the pass a micron nearer them is for the two to be
/// the same pass there, mm.
const SAME_PASS: f64 = 0.05;

/// An open pocket as it is read from the model: what its passes are worked out from, in the
/// setup's WCS.
#[derive(Clone, Debug, PartialEq)]
pub struct OpenFloor {
    /// The floor: the outline of each flat face of it, without the openings in it (the tool
    /// passes over those).
    pub floor: Vec<Region2>,
    /// What stands above the floor, walls and islands alike: the outline of each piece of the
    /// part's section just above it. They do not overlap.
    pub walls: Vec<Region2>,
}

impl OpenFloor {
    /// About how wide a tool can be and still go along the walls. Where something stands across
    /// the floor from them (a channel's other wall), twice the furthest a place on the floor is
    /// from the walls, and 2 microns: a tool must be narrower, and one exactly as wide as a
    /// channel is. Where nothing stands across from them (a step, the floor round a boss),
    /// there is no limit, and the answer is [`MAX_COORDINATE`]. Places along the floor's outline
    /// and on a grid are asked.
    pub fn across(&self) -> f64 {
        let around = loops_of(&self.floor).flat_map(|l| l.segs.iter()).fold(solidcraft_geom::Aabb2::EMPTY, |b, seg| b.union(&seg.bounds()));
        let (wide, high) = (around.max.x - around.min.x, around.max.y - around.min.y);
        let size = wide.max(high);
        if !(size > 0.0 && size <= MAX_COORDINATE) || self.walls.is_empty() {
            return 0.0;
        }
        let along = loops_of(&self.floor).flat_map(|l| l.segs.iter()).flat_map(|seg| [seg.start(), seg.mid()]);
        let grid = (0..ACROSS_PLACES * ACROSS_PLACES).map(|k| {
            let (i, j) = ((k % ACROSS_PLACES) as f64 + 0.5, (k / ACROSS_PLACES) as f64 + 0.5);
            Vec2 { x: around.min.x + wide * i / ACROSS_PLACES as f64, y: around.min.y + high * j / ACROSS_PLACES as f64 }
        });
        let furthest = along
            .chain(grid.filter(|p| self.floor.iter().any(|f| f.contains(*p))))
            .map(|p| (away(&self.walls, p), p))
            .filter(|(far, _)| far.is_finite())
            .max_by(|a, b| a.0.total_cmp(&b.0));
        let Some((far, place)) = furthest.filter(|(far, _)| *far > JOINT_TOLERANCE) else { return 0.0 };
        // Straight on from the walls through that place: if they only get further, nothing
        // stands across from them, and a tool of any width goes along them.
        let foot = loops_of(&self.walls)
            .flat_map(|l| l.segs.iter())
            .map(|seg| seg.point_at(seg.closest_param(place)))
            .min_by(|a, b| apart(place, *a).total_cmp(&apart(place, *b)))
            .unwrap_or(place);
        let way = Vec2 { x: (place.x - foot.x) / far, y: (place.y - foot.y) / far };
        let places = (size / ASK_STEP).ceil().clamp(1.0, 4000.0);
        let nearer = (1..=places as usize).any(|j| {
            let on = size * j as f64 / places;
            away(&self.walls, Vec2 { x: place.x + way.x * on, y: place.y + way.y * on }) < far - WAY_SLACK
        });
        // A tool exactly as wide goes through, by the fit it is given a side.
        if nearer { 2.0 * (far + FIT) } else { MAX_COORDINATE }
    }

    /// How many pieces of what stands above the floor stand on it: the floor is all round
    /// them, every side of theirs on it or along its outline.
    pub fn islands(&self) -> usize {
        self.walls
            .iter()
            .filter(|wall| !wall.outer.segs.is_empty() && wall.outer.segs.iter().all(|seg| away(&self.floor, seg.mid()) <= BORDER))
            .count()
    }
}

/// How much further from the walls than a pass's own distance a place of it may be and still be
/// along a chain, mm: the offsetter's tolerance, and the hair it may move a boundary further.
const ALONG_SLACK: f64 = 2e-3;
/// How near a chain's end the nearest place of it is for a place of a pass to be past the end, mm.
const AT_END: f64 = 1e-6;

/// Where along the loops a pass is cut.
#[derive(Clone, Copy)]
enum Keep<'a> {
    /// Where the cutter reaches over the floor: an open pocket.
    Floor,
    /// Where the pass runs along the chain (its pieces end to end), not past its ends: a contour
    /// along an open chain of edges.
    Along(&'a [Seg2]),
}

/// An open pocket's cuts at a level.
pub(crate) struct OpenPlan {
    /// The passes, in the order cut: the furthest from the walls first.
    pub steps: Vec<Step>,
    /// The finishing pass along the walls, when stock was left for one.
    pub finishing: Vec<Step>,
    /// Whether the passes reach every place on the floor the tool's centre may be over.
    pub everywhere: bool,
    /// How many passes are cut the other way than asked, their start being out of reach.
    pub against: usize,
}

/// One stretch of a pass.
struct Run {
    /// The pass itself, the way it is cut.
    core: Vec<Seg2>,
    /// It ends where it starts.
    closed: bool,
    /// The straight way in from the air beside the stock to where `core` starts: nothing when
    /// it starts there already, `None` when there is no such way.
    on: Option<Vec<Seg2>>,
    /// The straight way out into the air from where `core` ends, likewise.
    off: Option<Vec<Seg2>>,
}

fn loops_of(regions: &[Region2]) -> impl Iterator<Item = &Loop2> {
    regions.iter().flat_map(|r| std::iter::once(&r.outer).chain(&r.holes))
}

/// How far `p` is from the outlines of `regions`.
fn from_outline(regions: &[Region2], p: Vec2) -> f64 {
    loops_of(regions).flat_map(|l| l.segs.iter()).map(|seg| apart(p, seg.point_at(seg.closest_param(p)))).fold(f64::INFINITY, f64::min)
}

/// How far `p` is from `regions`: 0 in one of them.
fn away(regions: &[Region2], p: Vec2) -> f64 {
    if regions.iter().any(|r| r.contains(p)) { 0.0 } else { from_outline(regions, p) }
}

fn length(segs: &[Seg2]) -> f64 {
    segs.iter().map(Seg2Geom::length).sum()
}

fn backwards(segs: &[Seg2]) -> Vec<Seg2> {
    segs.iter().rev().map(Seg2Geom::reversed).collect()
}

/// How far from `from` along the unit direction `way` until out of the box `beside`; 0 when it
/// is out of it already.
fn far_from(beside: (Vec2, Vec2), from: Vec2, way: Vec2) -> f64 {
    let (lo, hi) = beside;
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
}

/// Whether `seg`, a piece of a pass, is the pass going round the end of a wall: an arc about a
/// corner of the walls one of whose two sides borders the floor and the other does not. There
/// the floor's edge leaves the wall, and a pass that followed the arc would roll round the
/// wall's end and along a side that is no wall of this floor.
fn round_a_wall_end(open: &OpenFloor, seg: &Seg2) -> bool {
    let Seg2::Arc { center, sweep, .. } = *seg else { return false };
    if sweep.abs() < CORNER_TURN {
        return false;
    }
    for lp in loops_of(&open.walls) {
        for (side, next) in lp.segs.iter().zip(lp.segs.iter().cycle().skip(1)) {
            if apart(side.end(), center) > AT_CORNER {
                continue;
            }
            let (before, after) = (side.length(), next.length());
            if !(before > 0.0 && after > 0.0) {
                continue;
            }
            let borders = |p: Vec2| from_outline(&open.floor, p) <= BORDER;
            return borders(side.point_at(1.0 - (PROBE / before).min(0.5))) != borders(next.point_at((PROBE / after).min(0.5)));
        }
    }
    false
}

/// `segs` cut where `keep` changes along them (it is asked with the segment's place in `segs`
/// and a point of it), each bit with the answer along it. The answer is asked for no further
/// apart than `step` and never at a segment's ends, where two answers meet.
fn bits(segs: &[Seg2], keep: &dyn Fn(usize, Vec2) -> bool, step: f64) -> Vec<(Seg2, bool)> {
    let mut out: Vec<(Seg2, bool)> = Vec::new();
    for (k, seg) in segs.iter().enumerate() {
        let long = seg.length();
        if !(long > JOINT_TOLERANCE) {
            continue;
        }
        let places = (long / step).ceil().clamp(1.0, ASK_MOST);
        let at = |j: usize| (j as f64 + 0.5) / places;
        let mut was = keep(k, seg.point_at(at(0)));
        let mut from = 0.0;
        for j in 1..places as usize {
            if keep(k, seg.point_at(at(j))) == was {
                continue;
            }
            // Between the last place and this one the answer changes: halve the stretch.
            let (mut lo, mut hi) = (at(j - 1), at(j));
            for _ in 0..64 {
                if (hi - lo) * long <= ASK_EDGE {
                    break;
                }
                let mid = 0.5 * (lo + hi);
                if keep(k, seg.point_at(mid)) == was {
                    lo = mid;
                } else {
                    hi = mid;
                }
            }
            let change = 0.5 * (lo + hi);
            if (change - from) * long > JOINT_TOLERANCE {
                out.push((seg.part(from, change), was));
            }
            (from, was) = (change, !was);
        }
        if from == 0.0 {
            out.push((*seg, was));
        } else if (1.0 - from) * long > JOINT_TOLERANCE {
            out.push((seg.part(from, 1.0), was));
        }
    }
    out
}

/// Whether the place `p` of a pass `distance` from the walls runs along `chain`: the nearest
/// place of the chain is the pass's distance away (so the chain is on the walls' outline, where
/// they are nearest), and not one of the chain's two ends (the pass going round the end of the
/// wall).
fn along(chain: &[Seg2], p: Vec2, distance: f64) -> bool {
    let (Some(first), Some(last)) = (chain.first(), chain.last()) else { return false };
    let (begin, end) = (first.start(), last.end());
    let nearest = chain
        .iter()
        .map(|seg| {
            let q = seg.point_at(seg.closest_param(p));
            (apart(p, q), q)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0));
    nearest.is_some_and(|(far, q)| (far - distance).abs() <= ALONG_SLACK && apart(q, begin) > AT_END && apart(q, end) > AT_END)
}

/// The stretches of the pass `distance` from the walls that are cut (`keep` says where), each the
/// way `direction` asks and lengthened into the air beside the stock where it can be: `beside`
/// is the stock's box grown by the tool's radius and its clearance, `through` the furthest a
/// lengthening goes.
fn runs_at(
    open: &OpenFloor,
    keep: Keep,
    beside: (Vec2, Vec2),
    (distance, radius): (f64, f64),
    through: f64,
    direction: CutDirection,
) -> Result<Vec<Run>> {
    let grown = moved_at_least(&open.walls, -distance)?;
    let mut runs: Vec<Run> = Vec::new();
    for lp in loops_of(&grown) {
        // The offsetter's loops have what stands on their left, which is conventional milling
        // with a clockwise spindle; climb is the other way round.
        let segs: Vec<Seg2> = if direction == CutDirection::Climb { lp.reversed().segs } else { lp.segs.clone() };
        let ends: Vec<bool> = segs.iter().map(|seg| round_a_wall_end(open, seg)).collect();
        // Along a wall the pass is cut where the cutter reaches over the floor; round a wall's
        // end only where the tool's centre is over the floor itself, which goes on there.
        let keep = |k: usize, p: Vec2| match (keep, ends.get(k)) {
            (Keep::Along(chain), _) => along(chain, p, distance),
            (Keep::Floor, Some(true)) => open.floor.iter().any(|f| f.contains(p)),
            (Keep::Floor, _) => away(&open.floor, p) <= radius - TOUCH,
        };
        let step = ASK_STEP.min(0.25 * radius).max(length(&segs) / ASK_MOST);
        let pieces = bits(&segs, &keep, step);
        if pieces.is_empty() || !pieces.iter().any(|piece| piece.1) {
            continue;
        }
        if pieces.iter().all(|piece| piece.1) {
            runs.push(Run { core: segs, closed: true, on: None, off: None });
            continue;
        }
        // Round from where a kept stretch begins, so that none is split by where the loop starts.
        let count = pieces.len();
        let kept = |k: usize| pieces.get(k % count).is_some_and(|piece| piece.1);
        let Some(begin) = (0..count).find(|&k| kept(k) && !kept(k + count - 1)) else { continue };
        let mut core: Vec<Seg2> = Vec::new();
        for (seg, cut_here) in pieces.iter().cycle().skip(begin).take(count) {
            if *cut_here {
                core.push(*seg);
            } else if !core.is_empty() {
                runs.push(lengthened(std::mem::take(&mut core), open, beside, distance, through));
            }
        }
        if !core.is_empty() {
            runs.push(lengthened(core, open, beside, distance, through));
        }
    }
    runs.retain(|run| run.closed || length(&run.core) >= MIN_RUN);
    Ok(runs)
}

/// The stretch `core` of the pass `distance` from the walls with its straight ways in and out:
/// from each of its ends straight on, the way the pass goes there, until the cutter is beside
/// the stock, when that is no further than `through` and no nearer the walls than the pass.
fn lengthened(core: Vec<Seg2>, open: &OpenFloor, beside: (Vec2, Vec2), distance: f64, through: f64) -> Run {
    let way_out = |from: Vec2, way: Vec2| -> Option<Vec<Seg2>> {
        let far = far_from(beside, from, way);
        if !(far <= through) {
            return None;
        }
        if far <= JOINT_TOLERANCE {
            return Some(Vec::new());
        }
        let places = (far / ASK_STEP).ceil().max(1.0);
        let at = |j: f64| Vec2 { x: from.x + way.x * far * j / places, y: from.y + way.y * far * j / places };
        let clear = (1..=places as usize).all(|j| away(&open.walls, at(j as f64)) >= distance - WAY_SLACK);
        clear.then(|| vec![Seg2::Line { a: from, b: at(places) }])
    };
    let on = core.first().and_then(|seg| {
        let along = seg.start_tangent();
        way_out(seg.start(), Vec2 { x: -along.x, y: -along.y }).map(|out| backwards(&out))
    });
    let off = core.last().and_then(|seg| way_out(seg.end(), seg.end_tangent()));
    Run { core, closed: false, on, off }
}

/// The nearest place of the passes `before` to `to`, when it is no further than `most` and the
/// straight way from it keeps `distance` from the walls: the tool goes down there, in the air
/// that pass cut, and across.
fn from_before(before: &[Run], to: Vec2, most: f64, open: &OpenFloor, distance: f64) -> Option<Vec2> {
    let (far, from) = before
        .iter()
        .flat_map(|run| run.core.iter())
        .map(|seg| {
            let q = seg.point_at(seg.closest_param(to));
            (apart(to, q), q)
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))?;
    let places = (far / ASK_STEP).ceil().max(1.0);
    let at = |j: f64| Vec2 { x: from.x + (to.x - from.x) * j / places, y: from.y + (to.y - from.y) * j / places };
    (far <= most && (1..places as usize).all(|j| away(&open.walls, at(j as f64)) >= distance - WAY_SLACK)).then_some(from)
}

/// The shortest way from the passes `before` to the closed pass `core`, when it is no longer
/// than `most`: where it leaves them, and where it meets `core` (the segment, how far along it,
/// and the point).
fn way_to_closed(before: &[Run], core: &[Seg2], most: f64) -> Option<(Vec2, usize, f64, Vec2)> {
    let lp = Loop2 { segs: core.to_vec() };
    let mut best: Option<(f64, Vec2, usize, f64, Vec2)> = None;
    for seg in before.iter().flat_map(|run| run.core.iter()) {
        let places = (seg.length() / ASK_STEP).ceil().clamp(1.0, 400.0);
        for j in 0..=places as usize {
            let from = seg.point_at(j as f64 / places);
            if let Some((index, t, onto, far)) = nearest(&lp, from)
                && best.is_none_or(|b| far < b.0)
            {
                best = Some((far, from, index, t, onto));
            }
        }
    }
    best.filter(|b| b.0 <= most).map(|(_, from, index, t, onto)| (from, index, t, onto))
}

/// The cut `run` is, with how the tool gets to it: `before` are the passes cut just before it,
/// `apart_most` how far they are from it at most. And whether it is cut the other way than
/// asked.
fn entered(run: &Run, before: &[Run], apart_most: f64, open: &OpenFloor, distance: f64) -> Option<(Step, bool)> {
    let start = run.core.first().map(Seg2Geom::start)?;
    if run.closed {
        // From the pass before, in the air it cut; else down a ramp along itself.
        if let Some((from, index, t, onto)) = way_to_closed(before, &run.core, apart_most + LINK_SLACK) {
            let segs = started_at(&Loop2 { segs: run.core.clone() }, index, t);
            return Some((Step { start: onto, segs, closed: true, entry: Entry::Air, lift: true, from: Some(from), slow: 0.0 }, false));
        }
        return Some((Step { start, segs: run.core.clone(), closed: true, entry: Entry::Ramp, lift: true, from: None, slow: 0.0 }, false));
    }
    let joined = |on: &[Seg2], off: Option<&Vec<Seg2>>| -> Vec<Seg2> {
        on.iter().chain(&run.core).chain(off.into_iter().flatten()).copied().filter(|seg| seg.length() > JOINT_TOLERANCE).collect()
    };
    let cut = |segs: Vec<Seg2>, entry: Entry, from: Option<Vec2>, slow: f64| {
        let start = segs.first().map_or(start, Seg2Geom::start);
        Step { start, segs, closed: false, entry, lift: true, from, slow }
    };
    if let Some(on) = &run.on {
        // From the air beside the stock, in from the side.
        return Some((cut(joined(on, run.off.as_ref()), Entry::Air, None, length(on)), false));
    }
    if let Some(from) = from_before(before, start, apart_most + LINK_SLACK, open, distance) {
        return Some((cut(joined(&[], run.off.as_ref()), Entry::Air, Some(from), 0.0), false));
    }
    if let Some(off) = &run.off {
        // Its end is in the air and its start is not: from its end, the other way.
        let back = backwards(&joined(&[], Some(off)));
        return Some((cut(back, Entry::Air, None, length(off)), true));
    }
    Some((cut(run.core.clone(), Entry::ZigZag, None, 0.0), false))
}

/// Whether the cuts `steps` reach every place on the floor that the centre of a tool keeping
/// `offset` from the walls may be over: places on a grid are asked, each must be within the
/// tool's `radius` of a cut.
fn reaches(open: &OpenFloor, steps: &[Step], radius: f64, offset: f64) -> bool {
    let around = loops_of(&open.floor).flat_map(|l| l.segs.iter()).fold(solidcraft_geom::Aabb2::EMPTY, |b, seg| b.union(&seg.bounds()));
    let size = (around.max.x - around.min.x).max(around.max.y - around.min.y);
    if !(size > 0.0 && size.is_finite()) {
        return true;
    }
    let grid = (0.5 * radius).max(size / REACH_PLACES);
    let (across, up) =
        (((around.max.x - around.min.x) / grid).ceil().max(1.0) as usize, ((around.max.y - around.min.y) / grid).ceil().max(1.0) as usize);
    for i in 0..across {
        for j in 0..up {
            let p = Vec2 { x: around.min.x + (i as f64 + 0.5) * grid, y: around.min.y + (j as f64 + 0.5) * grid };
            if !open.floor.iter().any(|f| f.contains(p)) || away(&open.walls, p) < offset {
                continue;
            }
            let cut = steps.iter().flat_map(|step| step.segs.iter()).any(|seg| apart(p, seg.point_at(seg.closest_param(p))) <= radius + 1e-6);
            if !cut {
                return false;
            }
        }
    }
    true
}

/// The cuts at a level that clear the open pocket `open` with a tool of the radius `radius`:
/// passes `stepover` apart, the last of them `wall` + `finish` from the walls, the furthest cut
/// first; and with `finish` above 0 the finishing pass, `wall` from them. `beside` is the
/// stock's box grown by the tool's radius and a contour's clearance beside the stock: outside
/// it the tool is in the air.
/// No cuts when the tool reaches nothing of the floor.
pub(crate) fn plan(
    open: &OpenFloor,
    beside: (Vec2, Vec2),
    radius: f64,
    (wall, finish): (f64, f64),
    stepover: f64,
    direction: CutDirection,
) -> Result<OpenPlan> {
    let sizes = [radius, wall, finish, stepover, beside.0.x, beside.0.y, beside.1.x, beside.1.y];
    if !(sizes.iter().all(|v| v.abs() <= MAX_COORDINATE) && radius > 0.0 && wall >= radius - 1e-9 && finish >= 0.0 && stepover > 0.0) {
        return bad("an open pocket's passes need a tool, a distance from the walls no less than its radius and a stepover above 0".into());
    }
    let nothing = || OpenPlan { steps: Vec::new(), finishing: Vec::new(), everywhere: true, against: 0 };
    if open.floor.is_empty() || open.walls.is_empty() {
        return Ok(nothing());
    }
    let offset = wall + finish;
    // A lengthening goes from where the cutter is over the stock's edge to beside the stock,
    // through no more stock than a skin.
    let through = radius + DEFAULT_CONTOUR_CLEAR + THROUGH * 2.0 * radius;
    // Past this no pass reaches the floor: the furthest a place on the floor's outline is from
    // the walls, and the cutter's reach.
    let bound =
        loops_of(&open.floor).flat_map(|l| l.segs.iter()).flat_map(|seg| [seg.start(), seg.mid()]).map(|p| away(&open.walls, p)).fold(0.0, f64::max)
            + radius;
    if !((bound - offset) / stepover <= MAX_POCKET_RINGS as f64) {
        return bad(format!("it takes more than {MAX_POCKET_RINGS} passes at a stepover of {stepover} mm"));
    }
    let mut levels: Vec<(f64, Vec<Run>)> = Vec::new();
    let mut count = 0;
    for k in 0..=MAX_POCKET_RINGS {
        let distance = offset + stepover * k as f64;
        let runs = runs_at(open, Keep::Floor, beside, (distance, radius), through, direction)?;
        if runs.is_empty() {
            if distance > bound {
                break;
            }
            continue;
        }
        count += runs.len();
        if count > MAX_POCKET_RINGS {
            return bad(format!("it takes more than {MAX_POCKET_RINGS} passes at a stepover of {stepover} mm"));
        }
        levels.push((distance, runs));
    }
    // A tool exactly as wide as a channel: a micron nearer the walls its pass down the middle
    // is there for certain. That pass on the walls is taken instead of the one at their own
    // distance when it has a stretch that one has nothing near.
    let none: Vec<Run> = Vec::new();
    let fitted = runs_at(open, Keep::Floor, beside, (offset - FIT, radius), through, direction)?;
    let on_walls = levels.first().filter(|(distance, _)| *distance == offset).map_or(none.as_slice(), |(_, runs)| runs.as_slice());
    // A place of the nearer pass that the pass on the walls has nothing near: where an arc meets
    // a wall almost along it, the two passes' ends slide some microns apart, which is no new cut.
    let away_from_walls_pass =
        |p: Vec2| !on_walls.iter().flat_map(|run| run.core.iter()).any(|seg| apart(p, seg.point_at(seg.closest_param(p))) <= SAME_PASS);
    let new_stretch = |run: &Run| -> f64 {
        run.core
            .iter()
            .map(|seg| {
                let places = (seg.length() / ASK_STEP).ceil().clamp(1.0, 400.0);
                let far = (0..places as usize).filter(|&j| away_from_walls_pass(seg.point_at((j as f64 + 0.5) / places))).count();
                seg.length() * far as f64 / places
            })
            .sum()
    };
    let more = fitted.iter().any(|run| new_stretch(run) > MIN_RUN);
    if more {
        match levels.first_mut().filter(|(distance, _)| *distance == offset) {
            Some(first) => *first = (offset - FIT, fitted),
            None => levels.insert(0, (offset - FIT, fitted)),
        }
    }
    if levels.is_empty() {
        return Ok(nothing());
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut against = 0;
    let mut before: &[Run] = none.as_slice();
    for (distance, runs) in levels.iter().rev() {
        for run in runs {
            if let Some((step, other_way)) = entered(run, before, stepover, open, *distance) {
                steps.push(step);
                against += usize::from(other_way);
            }
        }
        before = runs.as_slice();
    }
    let everywhere = reaches(open, &steps, radius, offset);
    // The finishing pass: along the walls, from the air or from the pass that left its stock.
    let mut finishing: Vec<Step> = Vec::new();
    if finish > 0.0 && !steps.is_empty() {
        let last = levels.first().map_or(none.as_slice(), |(_, runs)| runs.as_slice());
        for run in &runs_at(open, Keep::Floor, beside, (wall, radius), through, direction)? {
            if let Some((step, other_way)) = entered(run, last, finish, open, wall) {
                finishing.push(step);
                against += usize::from(other_way);
            }
        }
    }
    Ok(OpenPlan { steps, finishing, everywhere, against })
}

/// The cuts at a level along the open chain `chain` (its pieces end to end, X and Y in the WCS)
/// with a tool of the radius `radius`, beside `walls` (what stands above the cut's bottom, the
/// chain on their outline): `passes` passes `stepover` apart, the last of them `wall` from the
/// walls, the furthest first (decision 0043 of the CAM work). Each is the stretch of the pass
/// round the walls that runs along the chain, lengthened straight on past its ends into the air
/// beside the stock where it can be (`beside`, as [`plan`] has it), and entered as an open
/// pocket's passes are. No cuts when no stretch of the pass on the walls runs along the chain.
pub(crate) fn plan_along(
    walls: &[Region2],
    chain: &[Seg2],
    beside: (Vec2, Vec2),
    radius: f64,
    wall: f64,
    (passes, stepover): (usize, f64),
    direction: CutDirection,
) -> Result<OpenPlan> {
    let sizes = [radius, wall, stepover, beside.0.x, beside.0.y, beside.1.x, beside.1.y];
    if !(sizes.iter().all(|v| v.abs() <= MAX_COORDINATE) && radius > 0.0 && wall >= radius - 1e-9 && stepover >= 0.0) {
        return bad("a contour's passes need a tool, a distance from the walls no less than its radius and a stepover of 0 or more".into());
    }
    if !(1..=MAX_POCKET_RINGS).contains(&passes) || (passes > 1 && stepover <= 0.0) {
        return bad(format!("a contour takes 1 to {MAX_POCKET_RINGS} passes, more than one a stepover above 0 apart"));
    }
    let nothing = || OpenPlan { steps: Vec::new(), finishing: Vec::new(), everywhere: true, against: 0 };
    if walls.is_empty() || chain.is_empty() {
        return Ok(nothing());
    }
    let open = OpenFloor { floor: Vec::new(), walls: walls.to_vec() };
    let through = radius + DEFAULT_CONTOUR_CLEAR + THROUGH * 2.0 * radius;
    let mut levels: Vec<(f64, Vec<Run>)> = Vec::with_capacity(passes);
    for k in 0..passes {
        let distance = wall + stepover * k as f64;
        let runs = runs_at(&open, Keep::Along(chain), beside, (distance, radius), through, direction)?;
        if k == 0 && runs.is_empty() {
            return Ok(nothing());
        }
        levels.push((distance, runs));
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut against = 0;
    let none: Vec<Run> = Vec::new();
    let mut before: &[Run] = none.as_slice();
    for (distance, runs) in levels.iter().rev() {
        for run in runs {
            if let Some((step, other_way)) = entered(run, before, stepover, &open, *distance) {
                steps.push(step);
                against += usize::from(other_way);
            }
        }
        before = runs.as_slice();
    }
    Ok(OpenPlan { steps, finishing: Vec::new(), everywhere: true, against })
}

#[cfg(test)]
#[path = "open_pocket_tests.rs"]
mod tests;
