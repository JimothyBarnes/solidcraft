use super::*;
use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI};

const Z: Vec3 = Vec3 { x: 0.0, y: 0.0, z: 1.0 };

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-12
}

/// A face through `origin`, its x along world X and its y turned `tilt` up from world Y about X:
/// its normal is (0, -sin tilt, cos tilt).
fn face(origin: Vec3, tilt: f64) -> Plane {
    Plane { origin, x: Vec3::X, y: Vec3 { x: 0.0, y: tilt.cos(), z: tilt.sin() } }
}

#[test]
fn a_level_face_looks_up_or_down_and_a_tilt_past_45_microradians_does_neither() {
    let level = face(Vec3::ZERO, 0.0);
    assert!(looks_up(&level, Z) && !looks_down(&level, Z));
    let under = Plane { y: Vec3 { x: 0.0, y: -1.0, z: 0.0 }, ..level };
    assert!(looks_down(&under, Z) && !looks_up(&under, Z));
    // cos 1e-5 = 1 - 5e-11 is within ALONG; cos 1e-4 = 1 - 5e-9 is not.
    assert!(looks_up(&face(Vec3::ZERO, 1e-5), Z));
    assert!(!looks_up(&face(Vec3::ZERO, 1e-4), Z));
    let wall = face(Vec3::ZERO, FRAC_PI_2);
    assert!(!looks_up(&wall, Z) && !looks_down(&wall, Z));
    assert!(slope(&wall, Z).abs() <= 1e-15);
}

#[test]
fn a_sloping_face_rises_by_the_tangent_of_its_slope() {
    // Tilted 30 degrees, through Z 10: 4 mm along Y (down its slope's side) it is 4 tan 30 higher.
    let slanted = face(Vec3 { x: 0.0, y: 0.0, z: 10.0 }, PI / 6.0);
    assert!(near(slope(&slanted, Z), (PI / 6.0).cos()));
    let up = rise_to(&slanted, Vec3 { x: 7.0, y: 4.0, z: 0.0 }, Z);
    assert!(near(up, 10.0 + 4.0 * (PI / 6.0).tan()), "{up}");
    // From a place above it, the plane is below: the rise is negative.
    assert!(near(rise_to(&slanted, Vec3 { x: 0.0, y: 0.0, z: 15.0 }, Z), -5.0));
}

#[test]
fn a_direction_runs_along_the_axis_either_way() {
    assert!(runs_along(Z, Z, ALONG));
    assert!(runs_along(Vec3 { x: 0.0, y: 0.0, z: -1.0 }, Z, ALONG));
    // A milliradian off: cos = 1 - 5e-7.
    let off = Vec3 { x: 0.0, y: 1e-3_f64.sin(), z: 1e-3_f64.cos() };
    assert!(!runs_along(off, Z, ALONG));
    assert!(runs_along(off, Z, 1.0 - 1e-6));
}

#[test]
fn heights_are_measured_along_the_axis() {
    let p = Vec3 { x: 3.0, y: 4.0, z: 12.0 };
    assert!(near(height_along(p, Z), 12.0));
    assert!(near(height_along(p, Vec3 { x: 0.0, y: 0.6, z: 0.8 }), 12.0));
}

#[test]
fn two_directions_square_to_the_axis_and_to_each_other() {
    let (u, v) = square_to(Z).unwrap();
    assert_eq!((u, v), (Vec3 { x: 0.0, y: 1.0, z: 0.0 }, Vec3 { x: -1.0, y: 0.0, z: 0.0 }));
    // Along X, Y is the one to cross with: Z, then X × Z = -Y.
    let (u, v) = square_to(Vec3::X).unwrap();
    assert_eq!((u, v), (Z, Vec3 { x: 0.0, y: -1.0, z: 0.0 }));
    let tilted = Vec3 { x: 0.0, y: 0.6, z: 0.8 };
    let (u, v) = square_to(tilted).unwrap();
    for (a, b) in [(u, tilted), (v, tilted), (u, v)] {
        assert!(height_along(a, b).abs() <= 1e-15);
    }
    assert!(near(apart_in_space(u, Vec3::ZERO), 1.0) && near(apart_in_space(v, Vec3::ZERO), 1.0));
    assert!(square_to(Vec3::ZERO).is_none());
}

#[test]
fn distances_and_bearings_are_hand_checked() {
    let o = Vec2 { x: 0.0, y: 0.0 };
    assert!(near(apart(o, Vec2 { x: 3.0, y: 4.0 }), 5.0));
    assert!(near(apart_in_space(Vec3 { x: 1.0, y: 2.0, z: 3.0 }, Vec3 { x: 3.0, y: 5.0, z: 9.0 }), 7.0));
    assert!(near(slant(3.0, -4.0), 5.0));
    assert!(near(bearing(Vec2 { x: 1.0, y: 1.0 }, Vec2 { x: 1.0, y: 3.0 }), FRAC_PI_2));
    assert!(near(bearing(o, Vec2 { x: -1.0, y: 0.0 }), PI));
    assert!(near(bearing(o, Vec2 { x: 1.0, y: -1.0 }), -FRAC_PI_4));
}

#[test]
fn hostile_inputs_never_panic() {
    let bad = Vec3 { x: f64::NAN, y: f64::INFINITY, z: -f64::INFINITY };
    let p = Plane { origin: bad, x: bad, y: bad };
    let _ = (looks_up(&p, bad), looks_down(&p, Z), slope(&p, bad), rise_to(&p, bad, Z), runs_along(bad, Z, ALONG));
    let _ = (height_along(bad, bad), square_to(bad), apart_in_space(bad, Z), slant(f64::NAN, f64::INFINITY));
    let v = Vec2 { x: f64::NAN, y: f64::INFINITY };
    let _ = (apart(v, v), bearing(v, v));
    // An upright face (its y along Z): its rise is infinite or not a number, never a panic.
    let upright = Plane { origin: Vec3::ZERO, x: Vec3::X, y: Z };
    assert!(!rise_to(&upright, Vec3 { x: 0.0, y: 1.0, z: 0.0 }, Z).is_finite());
}
