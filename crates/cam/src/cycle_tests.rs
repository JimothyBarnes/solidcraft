use super::*;
use crate::{Error, Toolpath};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn rapid(x: f64, y: f64, z: f64) -> Move {
    Move::Rapid { to: p(x, y, z) }
}

fn feed(x: f64, y: f64, z: f64, f: f64) -> Move {
    Move::Linear { to: p(x, y, z), feed: f }
}

fn hole(from: Vec3, m: Move, motion: &CycleMotion) -> Vec<Move> {
    Toolpath { start: from, moves: vec![m] }.expanded(motion).unwrap().moves
}

#[test]
fn a_drilled_hole_runs_as_nists_g81_example() {
    // NIST IR 6556 §3.5.16.2, example 1: from (1, 2, 3), `G90 G81 G98 X4 Y5 Z1.5 R2.8`. G98 returns
    // to the old Z, 3.
    let m = Move::Drill { to: p(4.0, 5.0, 1.5), r: 2.8, retract: 3.0, cycle: DrillCycle::Drill { dwell: 0.0 }, feed: 100.0 };
    assert_eq!(
        hole(p(1.0, 2.0, 3.0), m, &CycleMotion::default()),
        vec![rapid(4.0, 5.0, 3.0), rapid(4.0, 5.0, 2.8), feed(4.0, 5.0, 1.5, 100.0), rapid(4.0, 5.0, 3.0)]
    );
}

#[test]
fn a_dwell_hole_waits_at_the_bottom() {
    let m = Move::Drill { to: p(0.0, 0.0, -3.0), r: 1.0, retract: 1.0, cycle: DrillCycle::Drill { dwell: 0.5 }, feed: 50.0 };
    assert_eq!(
        hole(p(0.0, 0.0, 1.0), m, &CycleMotion::default()),
        vec![feed(0.0, 0.0, -3.0, 50.0), Move::Dwell { seconds: 0.5 }, rapid(0.0, 0.0, 1.0)]
    );
}

#[test]
fn a_peck_hole_clears_its_chips_after_every_peck() {
    // 12 mm from R (2) to the bottom (-10) in pecks of 5: down to -3, -8, then the last 2 to -10.
    // NIST rapids out to the clear height (10) between pecks and back down to 0.5 above the depth.
    let m = Move::Drill { to: p(0.0, 0.0, -10.0), r: 2.0, retract: 10.0, cycle: DrillCycle::Peck { peck: 5.0 }, feed: 80.0 };
    let nist = CycleMotion { peck_clearance: 0.5, ..CycleMotion::default() };
    assert_eq!(
        hole(p(0.0, 0.0, 10.0), m, &nist),
        vec![
            rapid(0.0, 0.0, 2.0),
            feed(0.0, 0.0, -3.0, 80.0),
            rapid(0.0, 0.0, 10.0),
            rapid(0.0, 0.0, -2.5),
            feed(0.0, 0.0, -8.0, 80.0),
            rapid(0.0, 0.0, 10.0),
            rapid(0.0, 0.0, -7.5),
            feed(0.0, 0.0, -10.0, 80.0),
            rapid(0.0, 0.0, 10.0),
        ]
    );
    // Haas rapids out to R plus Setting 52 (1 mm here) instead.
    let haas = CycleMotion { peck_clearance: 0.5, chip_break_back_off: 0.5, peck_retract: PeckRetract::RPlane, above_r: 1.0 };
    let moves = hole(p(0.0, 0.0, 10.0), m, &haas);
    assert_eq!(moves[2], rapid(0.0, 0.0, 3.0));
    assert_eq!(moves[5], rapid(0.0, 0.0, 3.0));
    assert_eq!(moves.last(), Some(&rapid(0.0, 0.0, 10.0)));
    // Cut length: the depth plus one clearance for each peck after the first, 12 + 2 x 0.5.
    let path = Toolpath { start: p(0.0, 0.0, 10.0), moves: vec![m] };
    let cut = path.expanded(&nist).unwrap().stats().unwrap().feed_length;
    assert!((cut - 13.0).abs() < 1e-12, "{cut}");
}

#[test]
fn a_chip_break_hole_backs_off_without_leaving_it() {
    let m = Move::Drill { to: p(5.0, 5.0, -10.0), r: 2.0, retract: 2.0, cycle: DrillCycle::ChipBreak { peck: 5.0 }, feed: 80.0 };
    let motion = CycleMotion { chip_break_back_off: 0.5, ..CycleMotion::default() };
    assert_eq!(
        hole(p(0.0, 0.0, 2.0), m, &motion),
        vec![
            rapid(5.0, 5.0, 2.0),
            feed(5.0, 5.0, -3.0, 80.0),
            rapid(5.0, 5.0, -2.5),
            feed(5.0, 5.0, -8.0, 80.0),
            rapid(5.0, 5.0, -7.5),
            feed(5.0, 5.0, -10.0, 80.0),
            rapid(5.0, 5.0, 2.0),
        ]
    );
}

#[test]
fn pecks_are_measured_from_r_so_rounding_never_adds_one() {
    // 0.1 does not add up exactly in binary: ten pecks of it must still be ten, ending on the bottom.
    let m = Move::Drill { to: p(0.0, 0.0, -1.0), r: 0.0, retract: 0.0, cycle: DrillCycle::ChipBreak { peck: 0.1 }, feed: 10.0 };
    let moves = hole(p(0.0, 0.0, 0.0), m, &CycleMotion::default());
    let cuts: Vec<f64> = moves.iter().filter_map(|m| if let Move::Linear { to, .. } = m { Some(to.z) } else { None }).collect();
    assert_eq!(cuts.len(), 10);
    assert_eq!(cuts.last(), Some(&-1.0));
    // The most pecks a hole may take still finishes.
    let deepest = Move::Drill { to: p(0.0, 0.0, -10.0), r: 0.0, retract: 0.0, cycle: DrillCycle::Peck { peck: 0.001 }, feed: 10.0 };
    let moves = hole(p(0.0, 0.0, 0.0), deepest, &CycleMotion::default());
    assert_eq!(moves.iter().filter(|m| matches!(m, Move::Linear { .. })).count(), MAX_PECKS);
    assert_eq!(moves.iter().rev().nth(1), Some(&feed(0.0, 0.0, -10.0, 10.0)));
}

#[test]
fn a_hole_ends_at_its_retract_height_and_counts_in_the_stats() {
    let holes = Toolpath {
        start: p(0.0, 0.0, 10.0),
        moves: vec![
            Move::Drill { to: p(10.0, 0.0, -5.0), r: 1.0, retract: 1.0, cycle: DrillCycle::Drill { dwell: 0.0 }, feed: 60.0 },
            Move::Drill { to: p(20.0, 0.0, -5.0), r: 1.0, retract: 10.0, cycle: DrillCycle::Drill { dwell: 0.0 }, feed: 60.0 },
        ],
    };
    assert_eq!(holes.moves[0].end(holes.start), p(10.0, 0.0, 1.0));
    let s = holes.stats().unwrap();
    // Two holes of 6 mm at 60 mm/min. Rapids: across 10, down 9 to R, up 6 from the bottom to R;
    // across 10 at R, up 15 from the bottom to 10.
    assert!((s.feed_length - 12.0).abs() < 1e-12);
    assert!((s.feed_minutes - 0.2).abs() < 1e-12);
    assert!((s.rapid_length - 50.0).abs() < 1e-12, "{}", s.rapid_length);
    assert_eq!((s.min.z, s.max.z, s.max.x), (-5.0, 10.0, 20.0));
}

#[test]
fn bad_holes_are_refused_with_the_move_and_reason() {
    let ok = DrillCycle::Drill { dwell: 0.0 };
    let cases: &[(Vec3, Move, &str)] = &[
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 1.0), r: 1.0, retract: 5.0, cycle: ok, feed: 10.0 }, "above the hole's bottom"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 0.5, cycle: ok, feed: 10.0 }, "R plane or above"),
        (p(0.0, 0.0, 0.5), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 1.0, cycle: ok, feed: 10.0 }, "starts at or above"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: f64::NAN, retract: 5.0, cycle: ok, feed: 10.0 }, "finite"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: f64::INFINITY, cycle: ok, feed: 10.0 }, "finite"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, f64::NAN, 0.0), r: 1.0, retract: 5.0, cycle: ok, feed: 10.0 }, "finite"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 5.0, cycle: ok, feed: 0.0 }, "feed"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 5.0, cycle: DrillCycle::Drill { dwell: -1.0 }, feed: 10.0 }, "dwell"),
        (p(0.0, 0.0, 5.0), Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 0.0 }, feed: 10.0 }, "a peck"),
        (
            p(0.0, 0.0, 5.0),
            Move::Drill { to: p(0.0, 0.0, 0.0), r: 1.0, retract: 5.0, cycle: DrillCycle::ChipBreak { peck: f64::NAN }, feed: 10.0 },
            "a peck",
        ),
        (
            p(0.0, 0.0, 5.0),
            Move::Drill { to: p(0.0, 0.0, -100.0), r: 1.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 0.001 }, feed: 10.0 },
            "more than 10000 pecks",
        ),
    ];
    for (start, m, expect) in cases {
        let path = Toolpath { start: *start, moves: vec![Move::Rapid { to: *start }, *m] };
        match path.validate() {
            Err(Error::BadMove { index: 1, reason }) => assert!(reason.contains(expect), "{m:?}: {reason:?} does not mention {expect:?}"),
            other => panic!("{m:?}: {other:?}"),
        }
        assert!(path.expanded(&CycleMotion::default()).is_err());
        assert!(path.stats().is_err());
    }
}

#[test]
fn bad_cycle_distances_are_refused() {
    let base = CycleMotion::default();
    for motion in [
        CycleMotion { peck_clearance: 0.0, ..base },
        CycleMotion { peck_clearance: f64::NAN, ..base },
        CycleMotion { chip_break_back_off: -1.0, ..base },
        CycleMotion { chip_break_back_off: 26.0, ..base },
        CycleMotion { above_r: -0.1, ..base },
        CycleMotion { above_r: f64::INFINITY, ..base },
    ] {
        assert!(motion.check().is_err(), "{motion:?}");
        let path = Toolpath::default();
        assert!(path.expanded(&motion).is_err(), "{motion:?}");
    }
    assert!(base.check().is_ok());
}

#[test]
fn a_hole_is_plain_json() {
    let m = Move::Drill { to: p(1.0, 2.0, -3.0), r: 1.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 2.0 }, feed: 90.0 };
    let json = serde_json::to_value(m).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"kind": "drill", "to": [1.0, 2.0, -3.0], "r": 1.0, "retract": 5.0, "cycle": {"type": "peck", "peck": 2.0}, "feed": 90.0})
    );
    assert_eq!(serde_json::from_value::<Move>(json).unwrap(), m);
    let motion: CycleMotion =
        serde_json::from_str(r#"{"peck_clearance": 1, "chip_break_back_off": 1, "peck_retract": "r_plane", "above_r": 0}"#).unwrap();
    assert_eq!(motion.peck_retract, PeckRetract::RPlane);
    assert!(
        serde_json::from_str::<CycleMotion>(r#"{"peck_clearance": 1, "chip_break_back_off": 1, "peck_retract": "r_plane", "above_r": 0, "x": 1}"#)
            .is_err()
    );
}

#[test]
fn a_tapped_hole_feeds_in_and_back_out_to_r() {
    // In step with the spindle down to the bottom, and (the spindle reversed, which is the
    // control's doing) back out to R at the same feed; then up to where it started at rapid.
    let tap = |retract: f64| Move::Drill { to: p(10.0, 0.0, -10.0), r: 2.0, retract, cycle: DrillCycle::Tap { pitch: 1.0 }, feed: 500.0 };
    let start = Toolpath { start: p(0.0, 0.0, 5.0), moves: vec![Move::Spindle { rpm: 500.0, direction: SpindleDirection::Clockwise }, tap(5.0)] };
    assert_eq!(
        start.expanded(&CycleMotion::default()).unwrap().moves[1..],
        [rapid(10.0, 0.0, 5.0), rapid(10.0, 0.0, 2.0), feed(10.0, 0.0, -10.0, 500.0), feed(10.0, 0.0, 2.0, 500.0), rapid(10.0, 0.0, 5.0)]
    );
    // Back to R only: it is there already when it has fed out.
    let low = Toolpath { start: p(10.0, 0.0, 2.0), moves: vec![Move::Spindle { rpm: 500.0, direction: SpindleDirection::Clockwise }, tap(2.0)] };
    assert_eq!(low.expanded(&CycleMotion::default()).unwrap().moves[1..], [feed(10.0, 0.0, -10.0, 500.0), feed(10.0, 0.0, 2.0, 500.0)]);
    // 12 mm in and 12 out at 500 mm/min: 2.88 s of cutting, and the hole ends at its retract height.
    let s = start.stats().unwrap();
    assert!((s.feed_length - 24.0).abs() < 1e-12 && (s.feed_minutes - 24.0 / 500.0).abs() < 1e-12, "{s:?}");
    assert_eq!(start.moves[1].end(p(0.0, 0.0, 5.0)), p(10.0, 0.0, 5.0));
}

#[test]
fn a_tap_must_be_in_step_with_the_spindle() {
    let tapped = |spindle: Option<Move>, pitch: f64, feed: f64| {
        let hole = Move::Drill { to: p(0.0, 0.0, -10.0), r: 2.0, retract: 2.0, cycle: DrillCycle::Tap { pitch }, feed };
        Toolpath { start: p(0.0, 0.0, 5.0), moves: spindle.into_iter().chain([hole]).collect() }.validate()
    };
    let turning = |rpm: f64, direction: SpindleDirection| Some(Move::Spindle { rpm, direction });
    let clockwise = SpindleDirection::Clockwise;
    // 1.25 mm a turn at 400 rev/min is 500 mm/min, and nothing else.
    assert!(tapped(turning(400.0, clockwise), 1.25, 500.0).is_ok());
    let refused = |made: crate::Result<()>| match made {
        Err(Error::BadMove { index, reason }) => (index, reason),
        other => panic!("{other:?}"),
    };
    let (index, why) = refused(tapped(turning(400.0, clockwise), 1.25, 501.0));
    assert!(index == 1 && why.contains("1.25 mm a turn at 400 rev/min is 500 mm/min, not 501"), "{why}");
    assert!(refused(tapped(turning(400.0, clockwise), 1.0, 500.0)).1.contains("pitch times the spindle speed"));
    // The spindle stopped, or turning the other way.
    assert!(refused(tapped(None, 1.25, 500.0)).1.contains("spindle stopped"));
    assert!(refused(tapped(turning(400.0, SpindleDirection::CounterClockwise), 1.25, 500.0)).1.contains("clockwise"));
    let stopped = Toolpath {
        start: p(0.0, 0.0, 5.0),
        moves: vec![
            Move::Spindle { rpm: 400.0, direction: clockwise },
            Move::SpindleStop,
            Move::Drill { to: p(0.0, 0.0, -10.0), r: 2.0, retract: 2.0, cycle: DrillCycle::Tap { pitch: 1.25 }, feed: 500.0 },
        ],
    };
    assert!(refused(stopped.validate()).1.contains("spindle stopped"));
    // A pitch no tap has, and hostile ones.
    for pitch in [0.0, -1.0, 0.001, 1e9, f64::NAN, f64::INFINITY] {
        assert!(refused(tapped(turning(400.0, clockwise), pitch, 500.0)).1.contains("pitch"), "{pitch}");
    }
}
