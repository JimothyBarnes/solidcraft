use std::f64::consts::PI;

use super::*;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-9
}

/// A 10 mm square at Z -1 after a plunge from Z 5, fed at 600 mm/min.
fn square() -> Toolpath {
    let f = 600.0;
    Toolpath {
        start: p(0.0, 0.0, 5.0),
        moves: vec![
            Move::ToolChange { tool: 1 },
            Move::Spindle { rpm: 10_000.0, direction: SpindleDirection::Clockwise },
            Move::Linear { to: p(0.0, 0.0, -1.0), feed: 300.0 },
            Move::Linear { to: p(10.0, 0.0, -1.0), feed: f },
            Move::Linear { to: p(10.0, 10.0, -1.0), feed: f },
            Move::Linear { to: p(0.0, 10.0, -1.0), feed: f },
            Move::Linear { to: p(0.0, 0.0, -1.0), feed: f },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::SpindleStop,
        ],
    }
}

#[test]
fn a_square_measures_its_perimeter_plunge_and_time() {
    let s = square().stats().unwrap();
    assert!(close(s.feed_length, 46.0)); // 6 mm plunge + 40 mm perimeter
    assert!(close(s.rapid_length, 6.0));
    assert!(close(s.feed_minutes, 6.0 / 300.0 + 40.0 / 600.0));
    assert!(close(s.minutes(6_000.0), s.feed_minutes + 0.001));
    assert_eq!(s.tool_changes, 1);
    assert_eq!((s.min, s.max), (p(0.0, 0.0, -1.0), p(10.0, 10.0, 5.0)));
}

fn arc(from: Vec3, to: Vec3, center: Vec3, plane: Plane, direction: ArcDirection) -> Toolpath {
    Toolpath { start: from, moves: vec![Move::Arc { to, center, plane, direction, feed: 100.0 }] }
}

#[test]
fn a_quarter_arc_is_five_pi_long_either_way_round_the_short_side() {
    // Radius 10 about the origin, from +X to +Y: a quarter counter-clockwise.
    let ccw = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0, 0.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise).stats().unwrap();
    assert!(close(ccw.feed_length, 5.0 * PI));
    assert_eq!((ccw.min.x, ccw.max.x, ccw.min.y, ccw.max.y), (0.0, 10.0, 0.0, 10.0));
    // Clockwise between the same points is the other three quarters, reaching -X and -Y.
    let cw = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0, 0.0), Vec3::default(), Plane::Xy, ArcDirection::Clockwise).stats().unwrap();
    assert!(close(cw.feed_length, 15.0 * PI));
    assert!(close(cw.min.x, -10.0) && close(cw.min.y, -10.0));
}

#[test]
fn start_equal_to_end_is_a_full_circle_and_a_rise_makes_a_helix() {
    let full = arc(p(5.0, 0.0, 0.0), p(5.0, 0.0, 0.0), Vec3::default(), Plane::Xy, ArcDirection::Clockwise).stats().unwrap();
    assert!(close(full.feed_length, 10.0 * PI));
    // One turn of radius 5 dropping 2 mm: the unrolled helix is a right triangle.
    let helix = arc(p(5.0, 0.0, 0.0), p(5.0, 0.0, -2.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise).stats().unwrap();
    assert!(close(helix.feed_length, (10.0 * PI).hypot(2.0)));
    assert!(close(helix.min.z, -2.0));
}

#[test]
fn arcs_in_the_side_planes_follow_g18_and_g19_handedness() {
    // ZX plane (normal +Y): counter-clockwise goes from +Z towards +X.
    let zx = arc(p(0.0, 0.0, 10.0), p(10.0, 0.0, 0.0), Vec3::default(), Plane::Zx, ArcDirection::CounterClockwise).stats().unwrap();
    assert!(close(zx.feed_length, 5.0 * PI));
    assert!(close(zx.min.z, 0.0) && close(zx.max.x, 10.0) && zx.min.x >= -1e-9);
    // YZ plane (normal +X): counter-clockwise goes from +Y towards +Z.
    let yz = arc(p(0.0, 10.0, 0.0), p(0.0, 0.0, 10.0), Vec3::default(), Plane::Yz, ArcDirection::CounterClockwise).stats().unwrap();
    assert!(close(yz.feed_length, 5.0 * PI));
    assert!(yz.min.z >= -1e-9);
}

#[test]
fn an_arc_whose_ends_are_not_equally_far_from_its_centre_is_refused() {
    // NIST's limit is 0.002 mm: 0.0019 passes, 0.0021 does not.
    let ok = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0019, 0.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise);
    assert!(ok.validate().is_ok());
    let bad = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0021, 0.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise);
    assert!(matches!(bad.validate(), Err(Error::BadMove { index: 0, .. })));
    let degenerate = arc(Vec3::default(), Vec3::default(), Vec3::default(), Plane::Xy, ArcDirection::Clockwise);
    assert!(degenerate.validate().is_err());
}

#[test]
fn hostile_numbers_are_refused_never_panic() {
    let nan = f64::NAN;
    let inf = f64::INFINITY;
    let bad_moves = [
        Move::Rapid { to: p(nan, 0.0, 0.0) },
        Move::Rapid { to: p(0.0, 0.0, 1e9) },
        Move::Linear { to: p(1.0, 0.0, 0.0), feed: 0.0 },
        Move::Linear { to: p(1.0, 0.0, 0.0), feed: -5.0 },
        Move::Linear { to: p(1.0, 0.0, 0.0), feed: nan },
        Move::Linear { to: p(1.0, 0.0, 0.0), feed: inf },
        Move::Arc { to: p(0.0, 1.0, 0.0), center: p(nan, 0.0, 0.0), plane: Plane::Xy, direction: ArcDirection::Clockwise, feed: 1.0 },
        Move::Dwell { seconds: -1.0 },
        Move::Dwell { seconds: nan },
        Move::ToolChange { tool: 0 },
        Move::ToolChange { tool: u32::MAX },
        Move::Spindle { rpm: 0.0, direction: SpindleDirection::Clockwise },
        Move::Spindle { rpm: inf, direction: SpindleDirection::CounterClockwise },
    ];
    for m in bad_moves {
        let tp = Toolpath { start: p(1.0, 0.0, 0.0), moves: vec![m] };
        assert!(tp.validate().is_err(), "{m:?} was accepted");
        assert!(tp.stats().is_err(), "{m:?} was measured");
    }
    let bad_start = Toolpath { start: p(0.0, inf, 0.0), moves: vec![] };
    assert!(bad_start.validate().is_err());
}

#[test]
fn moves_round_trip_through_json_with_their_kind_named() {
    let tp = square();
    let json = serde_json::to_string(&tp).unwrap();
    assert!(json.contains(r#""kind":"tool_change""#) && json.contains(r#""kind":"linear""#));
    assert_eq!(serde_json::from_str::<Toolpath>(&json).unwrap(), tp);
    assert!(serde_json::from_str::<Toolpath>(r#"{"start":{"x":0,"y":0,"z":0},"moves":[{"kind":"teleport"}]}"#).is_err());
}
