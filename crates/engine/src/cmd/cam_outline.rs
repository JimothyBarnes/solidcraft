//! The part's outline for the contour operation (decision 0013 of the CAM work): the closed loops
//! round the part where a level plane cuts it, as lines and arcs, read exactly from its B-rep
//! (never its mesh) the way section views read their cut faces (`drawing_view_hatch.rs`): the part
//! of each body above the plane is kept with a half space, and its faces lying in the plane are
//! the section. A free-form edge (an ellipse, a spline) is cut into its Bézier pieces and each is
//! fitted with arcs to within [`FIT`], with the geometry crate's own spline and arc-fit code.
//!
//! An outline taken at one height is only the part's outline if nothing above it sticks out past
//! it. That is checked exactly too: each loop, grown by a hair and extruded up past the bodies, is
//! cut from what the bodies have above the plane, and nothing may be left.
use solidcore_record::brep::{Loop, OrientedEdge};
use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{Body, BoolOp, PlanarPiece, PlanarSpline};

/// How near to opposite a face's normal must be to the plane's for the face to be a cut face.
const FACING: f64 = 1.0 - 1e-9;
/// How much a loop is grown before it is checked to hold what is above it, mm at least and as a
/// share of the bodies' size: a wall standing on the outline is inside the grown loop by this
/// much, well clear of the kernel's own tolerances, so the check never meets faces lying on one
/// another. An overhang smaller than this is not seen.
const HAIR: f64 = 0.01;
const HAIR_OF_SIZE: f64 = 1e-4;
/// How near the arcs fitted to a free-form edge stay to it, mm: well inside [`HAIR`], so the
/// fitted outline grown by a hair still holds the part.
const FIT: f64 = 0.002;
/// The most control points one free-form edge may have.
const MAX_SPLINE_POINTS: usize = 10_000;
/// The most loops an outline may have.
pub(super) const MAX_OUTLINE_LOOPS: usize = 1_000;

/// The outline of `bodies` (each with its name, for errors) in `plane`: the outer loop of every
/// piece of their sections there, counter-clockwise, in the plane's own coordinates
/// (`plane.origin`, `plane.x`, `plane.y`), the plane's normal pointing up through the part. None
/// when no body has a face of its section there. An error when an edge of a section can't be read
/// as lines and arcs, or when something of a body above the plane reaches out past the outline.
pub(super) fn outline(bodies: &[(&str, &Body)], plane: &Plane) -> Result<Vec<Loop2>, String> {
    let mut loops = Vec::new();
    // What each body has above the plane, and how high the highest reaches above it.
    let mut above: Vec<(&str, Body)> = Vec::new();
    let (mut top, mut size) = (0.0_f64, 0.0_f64);
    for &(name, body) in bodies {
        if body.is_mesh() {
            return Err(format!("body '{name}' is a mesh and has no exact outline: convert it to a solid first"));
        }
        let said = |e: solidcraft_kernel::KernelError| format!("body '{name}': {e}");
        // How far the body reaches below and above the plane, by its exact box.
        let tol = body.planar_edges_tolerance();
        let bounds = body.exact_bounds().map_err(said)?;
        let (n, half) = (plane.normal(), bounds.size() * 0.5);
        let (middle, spread) = (plane.height(bounds.center()), n.x.abs() * half.x + n.y.abs() * half.y + n.z.abs() * half.z);
        if middle + spread <= tol {
            continue;
        }
        // A body standing on the plane is its own upper part, and its faces there are the
        // section: nothing is cut (a cut along a face it already has is not one to ask for).
        let kept = if middle - spread >= -tol {
            body.clone()
        } else {
            let Some(kept) = solidcraft_kernel::half_space(body, plane, true).map_err(said)?.0 else { continue };
            kept
        };
        for outer in section(&kept, plane).map_err(|e| format!("body '{name}': {e}"))? {
            if loops.len() >= MAX_OUTLINE_LOOPS {
                return Err(format!("the outline has more than {MAX_OUTLINE_LOOPS} loops"));
            }
            loops.push(outer);
        }
        top = top.max(middle + spread);
        size = size.max(body.size());
        above.push((name, kept));
    }
    if loops.is_empty() {
        return Ok(loops);
    }
    // Nothing above the plane may stick out past the outline: each loop, grown by a hair, as a
    // prism from just below the plane to just above the highest body.
    let hair = HAIR.max(HAIR_OF_SIZE * size);
    let mut grown = Vec::with_capacity(loops.len());
    for l in &loops {
        let region = Region2 { outer: l.clone(), holes: Vec::new() };
        let wider = solidcraft_cam::offset_region(&region, -hair).map_err(|e| format!("the part's outline: {e}"))?;
        grown.extend(wider.into_iter().map(|r| Region2 { outer: r.outer, holes: Vec::new() }));
    }
    let prisms = solidcraft_kernel::extrude(plane, &grown, -hair, top + hair).map_err(|e| format!("checking for overhangs: {e}"))?;
    for (name, kept) in above {
        let mut left = Some(kept);
        for prism in &prisms {
            let Some(rest) = left else { break };
            left = solidcraft_kernel::boolean(&rest, prism, BoolOp::Cut).map_err(|e| format!("checking body '{name}' for overhangs: {e}"))?;
        }
        if left.is_some() {
            return Err(format!(
                "body '{name}' is wider above the contour's bottom than at it, so a cut round its outline there would cut into it: raise the bottom to where the part is widest"
            ));
        }
    }
    Ok(loops)
}

/// The outer loops of `kept`'s faces that lie in `plane` and face down out of it: its section
/// there, each loop counter-clockwise in the plane's coordinates.
fn section(kept: &Body, plane: &Plane) -> Result<Vec<Loop2>, String> {
    let tol = kept.planar_edges_tolerance();
    let mut out = Vec::new();
    for (index, face) in kept.record().faces.iter().enumerate() {
        let cut = kept.planar_face(index).is_some_and(|on| on.normal().dot(plane.normal()) <= -FACING && plane.height(on.origin).abs() <= tol);
        if !cut {
            continue;
        }
        let rings: Vec<&[OrientedEdge]> = face
            .loops
            .iter()
            .filter_map(|l| match l {
                Loop::Edges { edges } => Some(edges.as_slice()),
                Loop::Vertex { .. } => None,
            })
            .collect();
        // The ring round the others is the face's outline; the rest are openings through the
        // part, which a cut round the outside does not go into.
        let mut outer: Option<Loop2> = None;
        for ring in rings {
            let edges: Vec<usize> = ring.iter().map(|used| used.edge as usize).collect();
            let mut segs = Vec::new();
            for (used, read) in ring.iter().zip(kept.planar_edges(plane, &edges)) {
                let Some(pieces) = read else { return Err("an edge of its outline could not be read in the cut's plane".into()) };
                let mut along = Vec::with_capacity(pieces.len());
                for piece in &pieces {
                    match piece {
                        PlanarPiece::Seg(s) => along.extend(fitted(s)?),
                        PlanarPiece::Spline(s) => along.extend(spline_arcs(s)?),
                    }
                }
                if !used.forward {
                    along = along.iter().rev().map(|s| s.reversed()).collect();
                }
                segs.extend(along);
            }
            let ring = Loop2 { segs };
            if outer.as_ref().is_none_or(|kept| ring.signed_area().abs() > kept.signed_area().abs()) {
                outer = Some(ring);
            }
        }
        if let Some(outer) = outer {
            out.push(outer.ccw());
        }
    }
    Ok(out)
}

/// A piece of an edge as the lines and arcs a machine cuts: itself, or a free-form one fitted
/// with arcs to within [`FIT`].
fn fitted(seg: &Seg2) -> Result<Vec<Seg2>, String> {
    seg.fit_arcs(FIT).ok_or_else(|| format!("a curve of its outline could not be fitted with arcs to within {FIT} mm"))
}

/// A spline edge as lines and arcs within [`FIT`] of it: each span between two of its knots is
/// one Bézier piece, exactly (the geometry crate's knot insertion), and each piece is fitted. A
/// rational piece of degree 2 is a conic (an ellipse's arc); a rational piece of a higher degree
/// has no segment of its own and is refused.
fn spline_arcs(s: &PlanarSpline) -> Result<Vec<Seg2>, String> {
    use solidcraft_geom::bspline::{check_knots, segment, weighted_points};
    let refused = |why: &str| format!("a curve of its outline can't be followed: {why}");
    let (p, n) = (s.degree, s.points.len());
    if p == 0 || n <= p || n > MAX_SPLINE_POINTS || s.knots.len() != n + p + 1 {
        return Err(refused("its degree, knots and control points don't go together"));
    }
    check_knots(&s.knots).map_err(|e| refused(&e.to_string()))?;
    let weights = if s.weights.is_empty() { vec![1.0; n] } else { s.weights.clone() };
    let points: Vec<Vec3> = s.points.iter().map(|q| Vec3 { x: q.x, y: q.y, z: 0.0 }).collect();
    let ctrl = weighted_points(&points, &weights).map_err(|e| refused(&e.to_string()))?;
    // The knots its spans lie between: those of its domain, each once.
    let mut breaks: Vec<f64> = s.knots.get(p..=n).map(<[f64]>::to_vec).unwrap_or_default();
    breaks.dedup();
    let mut out = Vec::new();
    for span in breaks.windows(2) {
        let [a, b] = *span else { continue };
        let (_, piece) = segment(p, &s.knots, &ctrl, a, b).map_err(|e| refused(&e.to_string()))?;
        let (Some(first), true) = (piece.first(), piece.len() == p + 1) else { return Err(refused("a span of it is not one piece")) };
        let flat: Vec<Vec2> = piece.iter().map(|h| Vec2 { x: h.x / h.w, y: h.y / h.w }).collect();
        let rational = piece.iter().any(|h| (h.w / first.w - 1.0).abs() > 1e-12);
        let seg = match (rational, piece.as_slice()) {
            (false, _) => solidcraft_geom::bezier(&flat),
            // Its end weights brought to one: the middle weight over the root of their product.
            (true, [h0, h1, h2]) => match *flat.as_slice() {
                [a, apex, b] => Some(Seg2::Conic { a, apex, b, w: h1.w / (h0.w * h2.w).sqrt() }),
                _ => None,
            },
            _ => None,
        };
        let seg = seg.ok_or_else(|| refused("it is a rational curve of degree 3 or more, or of too high a degree"))?;
        seg.check(1e9).map_err(refused)?;
        out.extend(fitted(&seg)?);
    }
    if out.is_empty() {
        return Err(refused("it has no length"));
    }
    Ok(out)
}
