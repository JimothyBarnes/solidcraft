//! The part as heights over a simulation's grid (decision 0015 of the CAM work): the highest
//! point of the bodies over each cell's centre, read from their exact B-rep, never a mesh.
//!
//! A flat face that faces up fills the cells its region covers, exactly: a level one at its own
//! height, a sloping one (a chamfer) at its plane's height over each cell. Its rings are read as
//! Contour and Pocket read theirs. A face that stands upright tops nothing. Any other face (a
//! fillet, a drill point's cone, a dome) may be the top over the cells under it, and those are
//! asked one by one with an exact ray fired down, about a tenth of a millisecond each; which
//! cells those are is the box round the face (a sphere or a torus by its own size, otherwise
//! round its edges: a line by its ends, a circle by its centre and radius, a spline by its
//! control points), or the whole body's when an edge gives none.
use solidcore_record::brep::Curve;
use solidcore_record::{Loop2, Plane, Vec2, Vec3};
use solidcraft_cam::{HeightField, measure};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{Body, FaceSurface};

use super::cam_outline::face_rings;

/// A direction this near square to up is upright.
const UPRIGHT: f64 = 1e-9;
/// The most cells asked with a ray.
const MAX_RAYS: usize = 2_000_000;
/// How near a flat face's curved edges are followed when its cells are filled, mm.
const EDGE_TOLERANCE: f64 = 1e-3;

/// The highest point of `bodies` (each with its name, for errors) over each cell of `field`'s
/// grid, which lies in `plane`'s coordinates with heights along its normal: `below` where there
/// is nothing. `top` is a height nothing of the bodies passes.
pub(super) fn part_heights(bodies: &[(&str, &Body)], plane: &Plane, field: &HeightField, top: f64, below: f64) -> Result<Vec<f64>, String> {
    let up = plane.normal();
    let mut heights = vec![below; field.nx * field.ny];
    let mut asked = vec![false; field.nx * field.ny];
    let mut rays = 0;
    for &(name, body) in bodies {
        if body.is_mesh() {
            return Err(format!("body '{name}' is a mesh and has no exact faces: convert it to a solid first"));
        }
        let said = |e: solidcraft_kernel::KernelError| format!("body '{name}': {e}");
        let whole = body.exact_bounds().map_err(said)?;
        let whole = [whole.min, whole.max];
        // The boxes (in the body's own coordinates) round the faces that may top the cells
        // under them and are not flat.
        let mut boxes: Vec<[Vec3; 2]> = Vec::new();
        for face in 0..body.face_count() {
            let flat = match solidcraft_kernel::face_surface(body, face) {
                Some(FaceSurface::Plane { .. }) => body.planar_face(face),
                Some(FaceSurface::Cylinder { axis, .. }) if measure::runs_along(axis, up, measure::ALONG) => continue,
                Some(_) => None,
                None => body.planar_face(face),
            };
            let Some(on) = flat else {
                boxes.push(around(body, face).unwrap_or(whole));
                continue;
            };
            let rise = measure::slope(&on, up);
            // Upright, or facing down: never the top of anything.
            if rise <= UPRIGHT {
                continue;
            }
            // Facing up, level or sloping: its region's cells, each at the plane's height there.
            // Its rings are read in its own plane and looked at from above.
            let seen = face_rings(body, face, &on).map(|rings| {
                let from_above =
                    |ring: &Loop2| ring.polyline(EDGE_TOLERANCE).into_iter().map(|at| plane.to_local(on.to_world(at))).collect::<Vec<Vec2>>();
                rings.iter().map(from_above).collect::<Vec<_>>()
            });
            match seen {
                Ok(outlines) if rise >= measure::ALONG => {
                    let level = (plane.height(on.origin) * 1e9).round() / 1e9;
                    fill(&mut heights, field, &outlines, |_| level);
                }
                Ok(outlines) => fill(&mut heights, field, &outlines, |at| measure::rise_to(&on, plane.to_world(at), up)),
                Err(_) => boxes.push(around(body, face).unwrap_or(whole)),
            }
        }
        // The cells under those boxes, each asked once for this body.
        asked.fill(false);
        for [lo, hi] in boxes {
            let corners = (0..8).map(|k| Vec3 {
                x: if k & 1 == 0 { lo.x } else { hi.x },
                y: if k & 2 == 0 { lo.y } else { hi.y },
                z: if k & 4 == 0 { lo.z } else { hi.z },
            });
            let (mut least, mut most) = (Vec2 { x: f64::INFINITY, y: f64::INFINITY }, Vec2 { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY });
            for corner in corners {
                let at = plane.to_local(corner);
                (least, most) = (Vec2 { x: least.x.min(at.x), y: least.y.min(at.y) }, Vec2 { x: most.x.max(at.x), y: most.y.max(at.y) });
            }
            let Some((i0, i1)) = field.columns(least.x, most.x) else { continue };
            let Some((j0, j1)) = field.rows(least.y, most.y) else { continue };
            for j in j0..=j1 {
                for i in i0..=i1 {
                    let Some(done) = asked.get_mut(j * field.nx + i) else { continue };
                    if std::mem::replace(done, true) {
                        continue;
                    }
                    rays += 1;
                    if rays > MAX_RAYS {
                        return Err(format!("the part's curved faces cover more than {MAX_RAYS} cells: simulate with a larger cell"));
                    }
                    let from = plane.to_world(field.centre(i, j)) + up * (top + 1.0);
                    if let Some(hit) = body.fire_ray(from, -up).map_err(said)?
                        && let Some(height) = heights.get_mut(j * field.nx + i)
                    {
                        // Without the last bits of the ray's arithmetic.
                        *height = height.max(((top + 1.0 - hit.depth) * 1e9).round() / 1e9);
                    }
                }
            }
        }
    }
    Ok(heights)
}

/// Raises the cells of `heights` whose centres lie in the region `outlines` bound (the outer
/// ring and its holes: a point inside an odd number of them) to `height` there, a row at a time.
fn fill(heights: &mut [f64], field: &HeightField, outlines: &[Vec<Vec2>], height: impl Fn(Vec2) -> f64) {
    let (low, high) = outlines.iter().flatten().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y)));
    // The rows whose centres lie between the lowest and the highest point of the rings.
    let Some((first, last)) = field.rows(low, high) else { return };
    let mut crossings: Vec<f64> = Vec::new();
    for j in first..=last {
        let y = field.centre(0, j).y;
        crossings.clear();
        for outline in outlines {
            for (k, a) in outline.iter().enumerate() {
                let Some(b) = outline.get((k + 1) % outline.len()) else { continue };
                if (a.y > y) != (b.y > y) {
                    crossings.push(a.x + (y - a.y) / (b.y - a.y) * (b.x - a.x));
                }
            }
        }
        crossings.sort_by(f64::total_cmp);
        for &[from, to] in crossings.as_chunks::<2>().0 {
            let Some((i0, i1)) = field.columns(from, to) else { continue };
            for i in i0..=i1 {
                let at = field.centre(i, j);
                if at.x < from || at.x > to {
                    continue;
                }
                if let Some(cell) = heights.get_mut(j * field.nx + i) {
                    *cell = cell.max(height(at));
                }
            }
        }
    }
}

/// The box (in the body's coordinates) that holds face `face` of `body`, when its surface or
/// its edges say: a sphere or a torus by its own size, otherwise round its edges ([`hold_edge`]).
/// `None` when an edge gives none or the face has no edges.
fn around(body: &Body, face: usize) -> Option<[Vec3; 2]> {
    let mut held = Held::default();
    match solidcraft_kernel::face_surface(body, face) {
        Some(FaceSurface::Sphere { center, radius }) => held.hold(center, radius),
        Some(FaceSurface::Torus { center, major, minor, .. }) => held.hold(center, major + minor),
        _ => {
            let record = body.record();
            let edges = solidcraft_kernel::face_edge_indices(body, face);
            if edges.is_empty() {
                return None;
            }
            for edge in edges {
                if let Some(circle) = solidcraft_kernel::edge_circle(body, edge) {
                    held.hold(circle.center, circle.radius);
                    continue;
                }
                let e = record.edges.get(edge)?;
                for vertex in e.vertices {
                    held.hold(Vec3::from(record.vertices.get(vertex as usize)?.point), 0.0);
                }
                hold_curve(e.nurbs.as_ref().unwrap_or(&e.curve), 0.0, &mut held)?;
            }
        }
    }
    held.around()
}

/// A box grown to hold points, each with a reach every way.
struct Held {
    lo: Vec3,
    hi: Vec3,
}

impl Default for Held {
    fn default() -> Held {
        Held {
            lo: Vec3 { x: f64::INFINITY, y: f64::INFINITY, z: f64::INFINITY },
            hi: Vec3 { x: f64::NEG_INFINITY, y: f64::NEG_INFINITY, z: f64::NEG_INFINITY },
        }
    }
}

impl Held {
    fn hold(&mut self, centre: Vec3, reach: f64) {
        self.lo = Vec3 { x: self.lo.x.min(centre.x - reach), y: self.lo.y.min(centre.y - reach), z: self.lo.z.min(centre.z - reach) };
        self.hi = Vec3 { x: self.hi.x.max(centre.x + reach), y: self.hi.y.max(centre.y + reach), z: self.hi.z.max(centre.z + reach) };
    }

    fn around(&self) -> Option<[Vec3; 2]> {
        let finite = |v: Vec3| v.x.is_finite() && v.y.is_finite() && v.z.is_finite();
        (self.lo.x <= self.hi.x && finite(self.lo) && finite(self.hi)).then_some([self.lo, self.hi])
    }
}

/// Grows `held` to hold the whole of `curve`, each point with `reach` to spare: a line edge is
/// held by its ends (the caller's), a circle or an ellipse by its centre and its radius every
/// way, a polyline by its points, a spline by its control points (it lies within them), and the
/// meeting of two surfaces by the curve that leads it and its tolerance. `None` for a spline
/// with a weight that is not positive.
fn hold_curve(curve: &Curve, reach: f64, held: &mut Held) -> Option<()> {
    match curve {
        Curve::Line { .. } => {}
        Curve::Circle { frame, radius } => held.hold(Vec3::from(frame.origin), radius.abs() + reach),
        Curve::Ellipse { frame, radii } => held.hold(Vec3::from(frame.origin), radii.iter().fold(0.0_f64, |most, r| most.max(r.abs())) + reach),
        Curve::Polyline { points } | Curve::Spline { points, .. } => points.iter().for_each(|point| held.hold(Vec3::from(*point), reach)),
        Curve::RationalSpline { homogeneous, .. } => {
            for [x, y, z, w] in homogeneous {
                if !(*w > 0.0) {
                    return None;
                }
                held.hold(Vec3 { x: x / w, y: y / w, z: z / w }, reach);
            }
        }
        Curve::SurfaceIntersection { leader, tolerance, .. } => hold_curve(leader, reach + tolerance.abs(), held)?,
    }
    Some(())
}
