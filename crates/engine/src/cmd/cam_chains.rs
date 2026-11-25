//! Chains of edges picked on the model, for a contour round them (decision 0038 of the CAM work),
//! read exactly from the bodies' B-rep, never a mesh.
//!
//! An edge is named by a point on it, as upstream's own commands name edges: the edge nearest the
//! point that lies level (in a plane parallel to the setup's XY) is the one. Its chain is the edges
//! joined to it end to end at its height, carried on through every vertex where exactly one other
//! level edge of that height goes on (a boss's outline is one chain, whatever its corners; at a
//! vertex where two could go on, the chain stops: which one is meant can't be told). A chain that
//! comes back to where it began is closed.
use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::prelude::*;
use solidcraft_geom::{Loop2Geom, PlaneGeom, Seg2Geom};
use solidcraft_kernel::{Body, BoolOp, PlanarPiece};

use super::cam_outline::{cut_faces, fitted, hair_of, spline_arcs};

/// The most edges a body may have for its edges to be picked.
const MAX_BODY_EDGES: usize = 20_000;
/// The most edges one chain may have.
const MAX_CHAIN_EDGES: usize = 10_000;
/// How far a picked point may be from the edge it names, mm: a click lands on the edge as the
/// screen draws it, a little off the exact curve.
const REACH: f64 = 2.0;

/// A chain of level edges, in a plane's coordinates.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Chain {
    /// Its height above the plane.
    pub z: f64,
    /// Its pieces, end to end from its start.
    pub segs: Vec<Seg2>,
    pub closed: bool,
    /// The body it is on (its place in the list) and the edges it is made of.
    pub body: usize,
    pub edges: Vec<usize>,
}

impl Chain {
    pub(super) fn length(&self) -> f64 {
        self.segs.iter().map(Seg2Geom::length).sum()
    }
}

/// A level edge of a body.
#[derive(Clone)]
struct Level {
    edge: usize,
    z: f64,
    /// Its pieces in the plane, from its first vertex to its second.
    segs: Vec<Seg2>,
    ends: [u32; 2],
}

/// `p` in `plane`'s coordinates: its place in the plane, and its height above it.
fn local(plane: &Plane, p: Vec3) -> (Vec2, f64) {
    let d = p - plane.origin;
    (Vec2 { x: d.dot(plane.x), y: d.dot(plane.y) }, plane.height(p))
}

fn apart(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// The level edges of `body` in `plane`: each that lies in a plane parallel to it, read as lines
/// and arcs. An edge that can't be read so is not offered.
fn levels(body: &Body, plane: &Plane) -> Result<Vec<Level>, String> {
    if body.is_mesh() {
        return Ok(Vec::new());
    }
    let record = body.record();
    if record.edges.len() > MAX_BODY_EDGES {
        return Err(format!("it has more than {MAX_BODY_EDGES} edges"));
    }
    let tol = body.planar_edges_tolerance().max(1e-6);
    let vertex = |v: u32| record.vertices.get(v as usize).map(|v| local(plane, Vec3 { x: v.point[0], y: v.point[1], z: v.point[2] }));
    let level: Vec<usize> = (0..record.edges.len())
        .filter(|&i| {
            let Some(edge) = record.edges.get(i) else { return false };
            matches!((vertex(edge.vertices[0]), vertex(edge.vertices[1])), (Some(a), Some(b)) if (a.1 - b.1).abs() <= tol)
        })
        .collect();
    let read = body.planar_edges(plane, &level);
    let mut out = Vec::new();
    for (&i, pieces) in level.iter().zip(read) {
        let (Some(pieces), Some(edge)) = (pieces, record.edges.get(i)) else { continue };
        let mut segs = Vec::new();
        let mut readable = true;
        for piece in &pieces {
            match piece {
                PlanarPiece::Seg(s) => match fitted(s) {
                    Ok(more) => segs.extend(more),
                    Err(_) => readable = false,
                },
                PlanarPiece::Spline(s) => match spline_arcs(s) {
                    Ok(more) => segs.extend(more),
                    Err(_) => readable = false,
                },
            }
        }
        let (Some(start), Some((_, z))) = (vertex(edge.vertices[0]).map(|v| v.0), vertex(edge.vertices[0])) else { continue };
        if !readable || segs.is_empty() {
            continue;
        }
        // From its first vertex to its second.
        let (first, last) = (segs.first().map(Seg2Geom::start), segs.last().map(Seg2Geom::end));
        if let (Some(first), Some(last)) = (first, last)
            && apart(last, start) < apart(first, start)
        {
            segs = segs.iter().rev().map(Seg2Geom::reversed).collect();
        }
        // Without the last bits of the arithmetic, so that an edge at Z -13 reads -13.
        out.push(Level { edge: i, z: (z * 1e9).round() / 1e9, segs, ends: edge.vertices });
    }
    Ok(out)
}

/// How far the point `p` (in the plane, at the height `h`) is from `level`.
fn distance(level: &Level, p: Vec2, h: f64) -> f64 {
    let across = level.segs.iter().map(|seg| apart(p, seg.point_at(seg.closest_param(p)))).fold(f64::INFINITY, f64::min);
    across.hypot(h - level.z)
}

/// The chain of `levels` that `start` is in: carried on from each of its ends through every vertex
/// where exactly one other level edge of its height goes on, until it comes back or can't go on.
fn chain_from(levels: &[Level], start: usize, tol: f64) -> Result<(Vec<(usize, bool)>, bool), String> {
    let Some(first) = levels.get(start) else { return Ok((Vec::new(), false)) };
    let z = first.z;
    // The level edges of the chain's height at the vertex `v` other than `arriving`, the edge the
    // chain comes to it by: each with whether it is gone along backwards (to its first vertex).
    let at = |v: u32, arriving: usize| -> Vec<(usize, bool)> {
        levels
            .iter()
            .enumerate()
            .filter(|(k, l)| *k != arriving && (l.z - z).abs() <= tol && l.ends.contains(&v))
            .map(|(k, l)| (k, l.ends[0] != v))
            .collect()
    };
    let mut order = vec![(start, false)];
    let (begin, mut end) = (first.ends[0], first.ends[1]);
    if begin == end {
        return Ok((order, true));
    }
    // On from its end, while exactly one edge goes on and it is not in the chain yet.
    let mut arriving = start;
    loop {
        let next = at(end, arriving);
        let [(k, backwards)] = next.as_slice() else { break };
        if order.iter().any(|(used, _)| used == k) {
            break;
        }
        let Some(l) = levels.get(*k) else { break };
        end = if *backwards { l.ends[0] } else { l.ends[1] };
        order.push((*k, *backwards));
        arriving = *k;
        if order.len() > MAX_CHAIN_EDGES {
            return Err(format!("a chain of more than {MAX_CHAIN_EDGES} edges"));
        }
        if end == begin {
            // Closed, unless more than the chain's own two edges meet where it began.
            let closes = at(begin, arriving).len() == 1;
            return Ok((order, closes));
        }
    }
    // Back from its beginning, likewise.
    let (mut begin, mut arriving) = (begin, start);
    loop {
        let next = at(begin, arriving);
        let [(k, away)] = next.as_slice() else { break };
        if order.iter().any(|(used, _)| used == k) {
            break;
        }
        let Some(l) = levels.get(*k) else { break };
        // Gone along so that it ends at `begin`: backwards when `begin` is its first vertex.
        let backwards = !*away;
        begin = if backwards { l.ends[1] } else { l.ends[0] };
        order.insert(0, (*k, backwards));
        arriving = *k;
        if order.len() > MAX_CHAIN_EDGES {
            return Err(format!("a chain of more than {MAX_CHAIN_EDGES} edges"));
        }
    }
    Ok((order, false))
}

/// The chains `picks` (points on edges, in model coordinates) name in `bodies` (each with its
/// name), in `plane`'s coordinates: one for each pick, but for picks on a chain already named.
pub(super) fn chains(bodies: &[(&str, &Body)], plane: &Plane, picks: &[Vec3]) -> Result<Vec<Chain>, String> {
    let size = bodies.iter().map(|(_, body)| body.size()).fold(0.0, f64::max);
    let tol = hair_of(size) * 0.1;
    let mut read: Vec<Option<Vec<Level>>> = (0..bodies.len()).map(|_| None).collect();
    let mut out: Vec<Chain> = Vec::new();
    for pick in picks {
        let (p, h) = local(plane, *pick);
        let here = format!("X {:.3} Y {:.3} Z {:.3}", pick.x, pick.y, pick.z);
        // The nearest level edge of any body.
        let mut best: Option<(usize, usize, f64)> = None;
        for (b, &(name, body)) in bodies.iter().enumerate() {
            let Some(slot) = read.get_mut(b) else { continue };
            if slot.is_none() {
                *slot = Some(levels(body, plane).map_err(|e| format!("body '{name}': {e}"))?);
            }
            for (k, level) in slot.iter().flatten().enumerate() {
                let far = distance(level, p, h);
                if far <= REACH && best.is_none_or(|(.., nearest)| far < nearest) {
                    best = Some((b, k, far));
                }
            }
        }
        let Some((b, k, _)) = best else {
            return Err(format!(
                "no edge lying level is near the point picked at {here}: a contour goes round edges that lie flat in the setup's XY, such as a wall's top or bottom edge"
            ));
        };
        let Some(levels) = read.get(b).and_then(Option::as_ref) else { continue };
        let Some(edge) = levels.get(k).map(|l| l.edge) else { continue };
        if out.iter().any(|chain| chain.body == b && chain.edges.contains(&edge)) {
            continue;
        }
        let (order, closed) = chain_from(levels, k, tol)?;
        let mut segs = Vec::new();
        let mut edges = Vec::new();
        for (k, backwards) in order {
            let Some(l) = levels.get(k) else { continue };
            edges.push(l.edge);
            if backwards {
                segs.extend(l.segs.iter().rev().map(Seg2Geom::reversed));
            } else {
                segs.extend(l.segs.iter().copied());
            }
        }
        let z = levels.get(k).map_or(h, |l| l.z);
        out.push(Chain { z, segs, closed, body: b, edges });
    }
    Ok(out)
}

/// Checks that the inside of the closed chain `ring` (in `plane`'s coordinates) is open above the
/// height `bottom`: the region it bounds, shrunk by a hair, as a prism from just above `bottom` to
/// above the bodies, meets none of them. The name of a body in the way, when one is.
pub(super) fn in_the_way<'a>(bodies: &[(&'a str, &Body)], plane: &Plane, ring: &Loop2, bottom: f64) -> Result<Option<&'a str>, String> {
    let size = bodies.iter().map(|(_, body)| body.size()).fold(0.0, f64::max);
    let hair = hair_of(size);
    let cut = plane.offset(bottom + hair);
    let mut top = 0.0_f64;
    for &(name, body) in bodies {
        if let Some(above) = cut_faces(body, &cut).map_err(|e| format!("body '{name}': {e}"))? {
            top = top.max(above.reach);
        }
    }
    if top <= hair {
        return Ok(None);
    }
    let region = Region2 { outer: ring.ccw(), holes: Vec::new() };
    let within = solidcraft_cam::offset_region(&region, hair).map_err(|e| format!("the chain: {e}"))?;
    if within.is_empty() {
        return Ok(None);
    }
    let prisms = solidcraft_kernel::extrude(&cut, &within, 0.0, top + hair).map_err(|e| format!("checking the chain: {e}"))?;
    for &(name, body) in bodies {
        for prism in &prisms {
            if solidcraft_kernel::boolean(prism, body, BoolOp::Intersect).map_err(|e| format!("checking body '{name}': {e}"))?.is_some() {
                return Ok(Some(name));
            }
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(a: (f64, f64), b: (f64, f64)) -> Seg2 {
        Seg2::Line { a: Vec2 { x: a.0, y: a.1 }, b: Vec2 { x: b.0, y: b.1 } }
    }

    fn level(edge: usize, a: (f64, f64), b: (f64, f64), ends: [u32; 2]) -> Level {
        Level { edge, z: 0.0, segs: vec![line(a, b)], ends }
    }

    #[test]
    fn a_chain_goes_on_through_corners_where_one_edge_goes_on_and_stops_where_two_do() {
        // A square 0..10 (vertices 0 1 2 3), its edges given out of order and one backwards, and a
        // line from its corner 2 out to (20, 10) (vertex 4).
        let square = vec![
            level(10, (10.0, 0.0), (10.0, 10.0), [1, 2]),
            level(11, (0.0, 0.0), (10.0, 0.0), [0, 1]),
            level(12, (0.0, 10.0), (0.0, 0.0), [3, 0]),
            level(13, (0.0, 10.0), (10.0, 10.0), [3, 2]),
        ];
        let (order, closed) = chain_from(&square, 1, 1e-6).unwrap();
        assert!(closed);
        assert_eq!(order.iter().map(|(k, _)| square[*k].edge).collect::<Vec<_>>(), vec![11, 10, 13, 12]);
        // The top edge, given from 3 to 2, is gone along backwards.
        assert_eq!(order.iter().find(|(k, _)| square[*k].edge == 13).map(|(_, b)| *b), Some(true));
        // With a spur at corner 2, two edges go on there: the chain stops at it from both sides.
        let mut spur = square;
        spur.push(level(14, (10.0, 10.0), (20.0, 10.0), [2, 4]));
        let (order, closed) = chain_from(&spur, 1, 1e-6).unwrap();
        assert!(!closed);
        assert_eq!(order.iter().map(|(k, _)| spur[*k].edge).collect::<Vec<_>>(), vec![13, 12, 11, 10]);
        // An edge of another height does not join: lifted, the right side stops the chain.
        let mut high = spur.clone();
        if let Some(l) = high.get_mut(0) {
            l.z = 5.0;
        }
        let (order, closed) = chain_from(&high, 1, 1e-6).unwrap();
        assert!(!closed && !order.iter().any(|(k, _)| high[*k].edge == 10));
    }
}
