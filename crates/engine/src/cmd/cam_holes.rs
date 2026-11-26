//! Holes found on the model for the Manufacture commands (decision 0012 of the CAM work): the
//! cylinder walls of a body that run along a direction, measured exactly on its B-rep, never on
//! its mesh. Upstream's engine measures bore walls the same way for fastener seats
//! (`cmd/parts.rs`).
use solidcore_record::Vec3;
use solidcraft_cam::measure;
use solidcraft_geom::prelude::*;
use solidcraft_kernel::Body;

/// One hole of a body along a direction, in the body's coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Hole {
    /// The point of the axis at the upper end of the wall.
    pub top: Vec3,
    pub radius: f64,
    /// The wall's length down from its upper end.
    pub depth: f64,
    /// How far below the wall's lower end the nearest material is, looked for on the axis and
    /// near the wall: `None` where there is none (a through hole), 0 at a floor at the end of the
    /// wall (flat, or the step down to a smaller hole), more at a drill point.
    pub floor: Option<f64>,
}

/// Two directions this near parallel are parallel (the kernel's own measure for a wall's rims).
const PARALLEL: f64 = 1e-9;
/// The most cylinder faces read in one body; a body with more is refused rather than half read.
pub(super) const MAX_WALL_FACES: usize = 20_000;
/// Where the rays near a hole's wall run, as a share of its radius.
const NEAR_WALL: f64 = 0.9;

/// A cylinder wall along the direction, while its faces are gathered.
struct Wall {
    /// Where the axis crosses the plane through the origin square to the direction.
    at: Vec3,
    radius: f64,
    /// The wall's ends, measured along the direction.
    low: f64,
    high: f64,
    /// How much of a turn the rim edges at the upper end make, radians.
    turn: f64,
}

/// The holes of `b` that open along `up`. A hole is a cylinder wall with the material outside
/// it, whose axis runs along `up`, that goes all the way round at its upper end (a pocket's
/// corner fillet does not) and has nothing above it across its width (a counterbore cut from
/// the other side does). A mesh body has none: its triangles are not cylinders.
pub(super) fn holes_along(b: &Body, up: Vec3) -> Result<Vec<Hole>, String> {
    let Some(up) = up.unit() else { return Err("the direction to look for holes along has no length".into()) };
    if b.is_mesh() {
        return Ok(Vec::new());
    }
    // The kernel's own measure of "the same place" on this body.
    let tol = (b.size() * 1e-7).max(1e-9) * 100.0;
    let mut walls: Vec<Wall> = Vec::new();
    for face in 0..b.record().faces.len() {
        let Some(c) = solidcraft_kernel::cylinder_between_rims(b, face).filter(|c| c.internal) else { continue };
        if !measure::runs_along(c.axis, up.vec(), 1.0 - PARALLEL) {
            continue;
        }
        if walls.len() >= MAX_WALL_FACES {
            return Err(format!("the body has more than {MAX_WALL_FACES} hole faces"));
        }
        let ends = [c.axis_point + c.axis * c.start, c.axis_point + c.axis * c.end];
        let heights = [measure::height_along(ends[0], up.vec()), measure::height_along(ends[1], up.vec())];
        let (low, high) = (heights[0].min(heights[1]), heights[0].max(heights[1]));
        let turn: f64 = solidcraft_kernel::face_edge_indices(b, face)
            .into_iter()
            .filter_map(|edge| solidcraft_kernel::edge_circle(b, edge))
            .filter(|rim| {
                measure::runs_along(rim.axis, up.vec(), 1.0 - PARALLEL) && (measure::height_along(rim.center, up.vec()) - high).abs() <= tol
            })
            .map(|rim| rim.angle)
            .sum();
        walls.push(Wall { at: ends[0] - up * heights[0], radius: c.radius, low, high, turn });
    }
    // Two directions square to `up` and to each other, to look near a hole's wall all round.
    let Some((u, v)) = measure::square_to(up.vec()) else { return Err("no direction square to the holes' was found".into()) };
    let mut holes = Vec::new();
    for w in joined(walls, tol) {
        if w.turn < std::f64::consts::TAU - 1e-6 || w.high - w.low <= tol {
            continue;
        }
        // From the middle of the wall, on its axis and at four places near the wall: nothing
        // above (the hole is open at its full width, and nothing overhangs it), and the nearest
        // thing below says how it ends.
        let half = 0.5 * (w.high - w.low);
        let middle = w.at + up * (w.low + half);
        let near = NEAR_WALL * w.radius;
        let from = [middle, middle + u * near, middle - u * near, middle + v * near, middle - v * near];
        let mut open = true;
        let mut floor: Option<f64> = None;
        for origin in from {
            if b.fire_ray(origin, up.vec()).map_err(|e| e.to_string())?.is_some() {
                open = false;
                break;
            }
            if let Some(hit) = b.fire_ray(origin, (-up).vec()).map_err(|e| e.to_string())? {
                let below = (hit.depth - half).max(0.0);
                floor = Some(floor.map_or(below, |nearest| nearest.min(below)));
            }
        }
        if open {
            holes.push(Hole { top: w.at + up * w.high, radius: w.radius, depth: w.high - w.low, floor });
        }
    }
    Ok(holes)
}

/// The faces of one wall as one: the halves of a wall share both its ends, and pieces stacked
/// along the axis share one. Joined until nothing more joins, so the order of the faces does not
/// matter.
fn joined(mut walls: Vec<Wall>, tol: f64) -> Vec<Wall> {
    loop {
        let before = walls.len();
        let mut out: Vec<Wall> = Vec::with_capacity(before);
        for w in walls {
            let same =
                |m: &&mut Wall| (m.at - w.at).len() <= tol && (m.radius - w.radius).abs() <= tol && w.low <= m.high + tol && w.high >= m.low - tol;
            match out.iter_mut().find(same) {
                Some(m) => {
                    // The turn is the upper end's: both walls' when they end at the same height.
                    if (w.high - m.high).abs() <= tol {
                        m.turn += w.turn;
                    } else if w.high > m.high {
                        m.turn = w.turn;
                    }
                    m.low = m.low.min(w.low);
                    m.high = m.high.max(w.high);
                }
                None => out.push(w),
            }
        }
        if out.len() == before {
            return out;
        }
        walls = out;
    }
}
