//! The measures the Manufacture workspace takes of the model before it makes toolpaths: which way
//! a flat face looks against the tool's axis, whether a hole runs along it, how high a sloping
//! face is above a place, how far apart two places are. The engine reads faces, edges and points
//! from the kernel and asks these here, so the app layers hold state and compute no geometry
//! (upstream's rule, `cargo xtask app-geometry`): the CAM geometry is this crate's, as the B-rep's
//! is the kernel's.
//!
//! Directions are of length one unless said otherwise; the tool's axis points up, out of the part.
use solidcore_record::{Plane, Vec2, Vec3};
use solidcraft_geom::prelude::*;

/// The cosine within which two directions count as one: a face that looks up the tool's axis, a
/// hole that runs along it (about 45 microradians).
pub const ALONG: f64 = 1.0 - 1e-9;

/// Whether the flat face `face` looks up `axis` (its normal within [`ALONG`] of it): a floor or a
/// top, square to the tool.
pub fn looks_up(face: &Plane, axis: Vec3) -> bool {
    face.normal().dot(axis) >= ALONG
}

/// Whether the flat face `face` looks down `axis`: an underside, square to the tool.
pub fn looks_down(face: &Plane, axis: Vec3) -> bool {
    face.normal().dot(axis) <= -ALONG
}

/// How the flat face `face` slopes against `axis`: the cosine between its normal and the axis, 1
/// for a face square to the tool looking up, 0 for an upright wall, below 0 for one looking down.
pub fn slope(face: &Plane, axis: Vec3) -> f64 {
    face.normal().dot(axis)
}

/// How far up `axis` from `from` the plane of the flat face `face` is, for a face that looks up
/// the axis at all ([`slope`] above 0): where the line up through `from` meets it.
pub fn rise_to(face: &Plane, from: Vec3, axis: Vec3) -> f64 {
    (face.origin - from).dot(face.normal()) / face.normal().dot(axis)
}

/// Whether the direction `dir` runs along `axis`, either way, with a cosine of at least `cosine`.
pub fn runs_along(dir: impl Into<Vec3>, axis: impl Into<Vec3>, cosine: f64) -> bool {
    dir.into().dot(axis).abs() >= cosine
}

/// How high the place `p` is along `axis`, measured from the origin.
pub fn height_along(p: Vec3, axis: impl Into<Vec3>) -> f64 {
    p.dot(axis)
}

/// Two directions square to `axis` and to each other, to look all round a place on it; `None` when
/// `axis` has no length.
pub fn square_to(axis: Vec3) -> Option<(Vec3, Vec3)> {
    let across = if axis.x.abs() < 0.9 { Vec3::X } else { Vec3::Y };
    let u = axis.cross(across).unit()?.vec();
    Some((u, axis.cross(u)))
}

/// How far apart the places `a` and `b` of a plane are.
pub fn apart(a: Vec2, b: Vec2) -> f64 {
    (a.x - b.x).hypot(a.y - b.y)
}

/// How far apart the places `a` and `b` are, in space.
pub fn apart_in_space(a: Vec3, b: Vec3) -> f64 {
    (a.x - b.x).hypot(a.y - b.y).hypot(a.z - b.z)
}

/// How far apart two places are that are `across` apart in a plane and `up` apart above it.
pub fn slant(across: f64, up: f64) -> f64 {
    across.hypot(up)
}

/// Which way `to` lies from `from` in a plane: the angle from the plane's x axis, counter-clockwise,
/// in -π..=π; where an arc about `from` through `to` starts.
pub fn bearing(from: Vec2, to: Vec2) -> f64 {
    (to.y - from.y).atan2(to.x - from.x)
}

#[cfg(test)]
#[path = "measure_tests.rs"]
mod tests;
