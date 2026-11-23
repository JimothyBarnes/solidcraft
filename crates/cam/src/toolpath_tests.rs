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
    // NIST's limits are 0.002 mm in a millimetre program and 0.0002 inch, 0.00508 mm, in an inch
    // one. A toolpath is held to the wider, so that an inch program reads back as one (a post
    // writing millimetres holds its own arcs to the narrower): 0.0050 passes, 0.0052 does not.
    let ok = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0050, 0.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise);
    assert!(ok.validate().is_ok());
    let bad = arc(p(10.0, 0.0, 0.0), p(0.0, 10.0052, 0.0), Vec3::default(), Plane::Xy, ArcDirection::CounterClockwise);
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

#[test]
fn a_toolpath_is_drawn_as_straight_pieces_that_say_how_they_move() {
    // A rapid down, a plunge, a cut, a quarter arc of radius 10 and a hole.
    let path = Toolpath {
        start: p(0.0, 0.0, 20.0),
        moves: vec![
            Move::Spindle { rpm: 1000.0, direction: SpindleDirection::Clockwise },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::Linear { to: p(0.0, 0.0, -1.0), feed: 100.0 },
            Move::Linear { to: p(10.0, 0.0, -1.0), feed: 200.0 },
            Move::Arc { to: p(0.0, 10.0, -1.0), center: p(0.0, 0.0, -1.0), plane: Plane::Xy, direction: ArcDirection::CounterClockwise, feed: 200.0 },
            Move::Rapid { to: p(0.0, 10.0, 5.0) },
            Move::Drill { to: p(20.0, 10.0, -4.0), r: 2.0, retract: 5.0, cycle: crate::DrillCycle::Drill { dwell: 0.5 }, feed: 50.0 },
        ],
    };
    let pieces = path.drawn(0.1).unwrap();
    // The arc: a chord may stray 0.1 from radius 10, so at most 2 acos(0.99) = 16.2° each: six of 15°.
    let kinds: Vec<Drawn> = pieces.iter().map(|x| x.0).collect();
    let mut expect = vec![Drawn::Rapid, Drawn::Plunge, Drawn::Cut];
    expect.extend([Drawn::Cut; 6]);
    // Up; then the hole: across at 5, down to R (2), the cut straight down, back up to 5.
    expect.extend([Drawn::Rapid, Drawn::Rapid, Drawn::Rapid, Drawn::Plunge, Drawn::Rapid]);
    assert_eq!(kinds, expect);
    // The pieces join end to end, the repeated rapid and the dwell left out.
    assert!(pieces.windows(2).all(|w| w[0].2 == w[1].1));
    assert_eq!((pieces[0].1, pieces[pieces.len() - 1].2), (p(0.0, 0.0, 20.0), p(20.0, 10.0, 5.0)));
    // Every corner of the drawn arc is on the circle, and the middle of every chord within 0.1 of it.
    for (_, a, b) in &pieces[3..9] {
        assert!((a.x.hypot(a.y) - 10.0).abs() < 1e-9 && (b.x.hypot(b.y) - 10.0).abs() < 1e-9);
        let stray = 10.0 - (0.5 * (a.x + b.x)).hypot(0.5 * (a.y + b.y));
        assert!((0.0..=0.1).contains(&stray), "{stray}");
    }
    assert_eq!(pieces[8].2, p(0.0, 10.0, -1.0));
    // A finer tolerance draws more pieces; a useless one is refused.
    assert!(path.drawn(0.001).unwrap().len() > pieces.len());
    assert!(path.drawn(0.0).is_err() && path.drawn(f64::NAN).is_err());
}

/// One operation as the strategies write them: the tool, the spindle, the coolant, down from
/// the clearance height at `x`, a cut, up again, and everything off.
fn operation(tool: u32, rpm: f64, coolant: Coolant, x: f64) -> Vec<Move> {
    let mut moves = vec![Move::ToolChange { tool }, Move::Spindle { rpm, direction: SpindleDirection::Clockwise }];
    if coolant != Coolant::Off {
        moves.push(Move::Coolant { mode: coolant });
    }
    moves.extend([Move::Rapid { to: p(x, 0.0, 25.0) }, Move::Linear { to: p(x, 0.0, -1.0), feed: 300.0 }, Move::Rapid { to: p(x, 0.0, 25.0) }]);
    if coolant != Coolant::Off {
        moves.push(Move::Coolant { mode: Coolant::Off });
    }
    moves.push(Move::SpindleStop);
    moves
}

#[test]
fn operations_one_after_another_with_the_same_tool_keep_it() {
    let program = |operations: &[Vec<Move>]| Toolpath { start: p(0.0, 0.0, 25.0), moves: operations.concat() };
    let (a, b, c) =
        (operation(2, 6000.0, Coolant::Flood, 10.0), operation(2, 6000.0, Coolant::Flood, 20.0), operation(3, 4000.0, Coolant::Flood, 30.0));
    // Tool 2 twice, then tool 3: between the first two nothing stops and nothing is changed or
    // started again, only the moves are left; tool 3 is a change like any other.
    let kept = program(&[a.clone(), b.clone(), c.clone()]).without_needless_changes();
    let mut want = a[..a.len() - 2].to_vec();
    want.extend_from_slice(&b[3..]);
    want.extend_from_slice(&c);
    assert_eq!(kept.moves, want);
    assert_eq!(kept.moves.iter().filter(|m| matches!(m, Move::ToolChange { .. })).count(), 2);
    kept.validate().unwrap();
    // The moves themselves are all there, in their order, and it ends where it ended.
    let motion = |t: &Toolpath| t.moves.iter().filter(|m| matches!(m, Move::Rapid { .. } | Move::Linear { .. })).copied().collect::<Vec<_>>();
    assert_eq!(motion(&kept), motion(&program(&[a.clone(), b.clone(), c.clone()])));
    // Done twice it is as done once; a program with nothing needless in it is as it was.
    assert_eq!(kept.without_needless_changes(), kept);
    let plain = program(&[a.clone(), c.clone(), a.clone()]);
    assert_eq!(plain.without_needless_changes(), plain);
    // The same tool at another speed: the spindle is told the new speed, turning; the coolant
    // stays as it is. Without coolant in the second, it is turned off where the second begins.
    let faster = program(&[a.clone(), operation(2, 9000.0, Coolant::Flood, 20.0)]).without_needless_changes();
    let between: Vec<Move> = faster.moves[a.len() - 2..a.len()].to_vec();
    assert_eq!(between, [Move::Spindle { rpm: 9000.0, direction: SpindleDirection::Clockwise }, Move::Rapid { to: p(20.0, 0.0, 25.0) }]);
    // The second without coolant: the first's coolant is turned off as it was, since nothing
    // turns it on again; the spindle goes on turning.
    let dry = program(&[a.clone(), operation(2, 6000.0, Coolant::Off, 20.0)]).without_needless_changes();
    assert_eq!(
        dry.moves[a.len() - 2..a.len()],
        [Move::Coolant { mode: Coolant::Off }, Move::Rapid { to: p(20.0, 0.0, 25.0) }],
        "{:?}",
        &dry.moves[a.len() - 3..]
    );
    assert_eq!(dry.moves.iter().filter(|m| matches!(m, Move::SpindleStop)).count(), 1);
    // A tool change to the same tool with nothing started after it (a program that ends there)
    // loses only the change: the spindle and the coolant stay stopped.
    let mut ends = a.clone();
    ends.push(Move::ToolChange { tool: 2 });
    assert_eq!(program(&[ends]).without_needless_changes().moves, a);
    // Three in a row, and a tool that comes back after another: only what is in a row is kept.
    let thrice = program(&[a.clone(), b.clone(), a.clone(), c.clone(), a.clone()]).without_needless_changes();
    let changes: Vec<u32> = thrice.moves.iter().filter_map(|m| if let Move::ToolChange { tool } = m { Some(*tool) } else { None }).collect();
    assert_eq!(changes, [2, 3, 2]);
    assert_eq!(thrice.moves.iter().filter(|m| matches!(m, Move::SpindleStop)).count(), 3);
    // Nothing at all, and changes alone, are as they were.
    assert_eq!(Toolpath::default().without_needless_changes(), Toolpath::default());
}
