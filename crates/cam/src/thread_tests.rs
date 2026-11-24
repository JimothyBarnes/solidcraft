use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};
use crate::toolpath::ArcDirection;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// Tool 8: a 3-flute thread mill 7 mm across with one row of teeth for a pitch of 1.5, on a neck
/// that reaches 20 mm down a hole, and data for aluminium (120 m/min, 0.03 mm a tooth, going
/// down at half).
fn thread_mill() -> Tool {
    Tool {
        number: 8,
        name: "7 mm thread mill, pitch 1.5".into(),
        kind: ToolKind::ThreadMill,
        geometry: ToolGeometry {
            diameter: 7.0,
            corner_radius: 0.0,
            flute_length: 1.5,
            shoulder_length: 20.0,
            overall_length: 60.0,
            shank_diameter: 8.0,
            tip_angle: 0.0,
            tip_diameter: 0.0,
            thread_pitch: 1.5,
        },
        flutes: 3,
        holder: Holder { stick_out: 30.0, segments: vec![] },
        length_offset: 8,
        diameter_offset: 8,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 120.0, chip_load: 0.03, plunge_factor: 0.5, ramp_factor: 0.5 }],
        overrides: Overrides::default(),
    }
}

/// The vise setup on a 100 x 80 x 20 block with 1 mm of stock all round and on top: the WCS at
/// the middle of the stock's top, so the block's top is Z -1; clearance 25, retract 5.
fn placed() -> Placement {
    let setup = Setup {
        id: "setup-1".into(),
        name: "Op 1".into(),
        kind: SetupKind::Milling,
        machine: None,
        material: Some("aluminium".into()),
        wcs: Wcs {
            origin: WcsOrigin::StockBox { anchor: BoxAnchor { x: BoxSide::Center, y: BoxSide::Center, z: BoxSide::Max } },
            orientation: WcsOrientation::Model,
        },
        work_offset: 1,
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 1.0 } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    };
    setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(100.0, 80.0, 20.0) }).unwrap()
}

/// M10 x 1.5, 6 deep, in one hole at (10, 5).
fn m10() -> ThreadOp {
    ThreadOp { holes: vec![[10.0, 5.0]], diameter: 10.0, depth: 6.0, ..ThreadOp::default() }
}

fn threaded(op: &ThreadOp, tool: &Tool) -> Result<Threaded> {
    op.toolpath(None, tool, &placed(), Some("aluminium"))
}

/// Each move that cuts or goes down the hole, in order: a line as (None, its end, its feed), an
/// arc as (its centre, its end, its feed); every arc must be counter-clockwise.
fn cuts(path: &Toolpath) -> Vec<(Option<Vec3>, Vec3, f64)> {
    path.moves
        .iter()
        .filter_map(|m| match *m {
            Move::Linear { to, feed } => Some((None, to, feed)),
            Move::Arc { to, center, direction, feed, .. } => {
                assert_eq!(direction, ArcDirection::CounterClockwise, "{m:?}");
                Some((Some(center), to, feed))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_right_hand_thread_is_milled_from_the_bottom_up_round_the_hole() {
    // M10 x 1.5 with the 7 mm tool: the thread is 0.5413 x 1.5 = 0.812 deep in the wall of a
    // hole drilled to 10 - 1.0825 x 1.5 = 8.376. The tool's centre goes round 1.5 from the
    // hole's middle at full depth, and 0.812 x (1 - sqrt 0.5) = 0.238 less on the first of two
    // passes. One row of teeth: a turn for each of the four pitches in 6 mm.
    let d = threaded(&m10(), &thread_mill()).unwrap();
    assert!(close(d.minor, 10.0 - 2.0 * THREAD_DEPTH * 1.5) && (d.minor - 8.376).abs() < 1e-3, "{}", d.minor);
    let first = 1.5 - (1.0 - 0.5_f64.sqrt()) * THREAD_DEPTH * 1.5;
    assert!(d.passes.len() == 2 && close(d.passes[0], first) && close(d.passes[1], 1.5) && (first - 1.2622).abs() < 1e-4, "{:?}", d.passes);
    assert_eq!((d.turns, d.pitch, d.top, d.bottom), (4, 1.5, -1.0, -7.0));
    // The speed is the tool's at its 7 mm (120 m/min), the feed 0.03 mm a tooth on three teeth
    // at the edge, and the centre's 3/10 of that: its circle is 3 across where the edge's is 10.
    let rpm = 120_000.0 / (PI * 7.0);
    assert!(close(d.rpm, rpm) && close(d.feed, 0.09 * rpm) && close(d.centre_feed, 0.027 * rpm) && close(d.plunge_feed, 0.045 * rpm), "{d:?}");
    // The moves, by hand, for the first turn of the first pass: down the hole's middle to a
    // quarter of a pitch below the thread's bottom (Z -7.375); on to the circle by a half turn
    // about the point half way out, rising to Z -7; once round the hole's middle in two halves,
    // rising 1.5; off by a half turn back to the middle, rising 0.375 more.
    let c = cuts(&d.toolpath);
    assert_eq!(c.len(), 2 * 4 * 5, "{c:#?}");
    let (a, half) = (first, 0.5 * first);
    let want = [
        (None, p(10.0, 5.0, -7.375), d.plunge_feed),
        (Some(p(10.0 + half, 5.0, -7.0)), p(10.0 + a, 5.0, -7.0), d.centre_feed),
        (Some(p(10.0, 5.0, -6.25)), p(10.0 - a, 5.0, -6.25), d.centre_feed),
        (Some(p(10.0, 5.0, -5.5)), p(10.0 + a, 5.0, -5.5), d.centre_feed),
        (Some(p(10.0 + half, 5.0, -5.125)), p(10.0, 5.0, -5.125), d.centre_feed),
    ];
    for (got, want) in c.iter().zip(&want) {
        let same = |a: Vec3, b: Vec3| close(a.x, b.x) && close(a.y, b.y) && close(a.z, b.z);
        assert!(
            got.0.is_some() == want.0.is_some() && got.0.zip(want.0).is_none_or(|(g, w)| same(g, w)) && same(got.1, want.1) && close(got.2, want.2),
            "{got:?} {want:?}"
        );
    }
    // Every line is in the hole's middle, at the feed for going down; the turns of a pass begin
    // 1.5 apart, from the bottom up, and the last one ends at the hole's top; the second pass
    // goes back down the middle and does the same 0.238 further out.
    let lines: Vec<&(Option<Vec3>, Vec3, f64)> = c.iter().filter(|m| m.0.is_none()).collect();
    assert!(lines.iter().all(|m| close(m.1.x, 10.0) && close(m.1.y, 5.0) && close(m.2, d.plunge_feed)));
    let starts: Vec<f64> = lines.iter().map(|m| m.1.z).collect();
    assert!(starts.iter().zip([-7.375, -5.875, -4.375, -2.875, -7.375, -5.875, -4.375, -2.875]).all(|(z, want)| close(*z, want)), "{starts:?}");
    assert!(close(c[19].1.z, -0.625) && close(c[18].1.z, -1.0) && close(c[23].1.x, 11.5) && close(c[39].1.z, -0.625), "{:?} {:?}", c[19], c[23]);
    // What is fed: down the middle (12.375 from the retract height, 0.75 between turns, 6.75
    // back down for the second pass), and for each turn two half turns of half the radius and
    // one whole turn, each a helix.
    let s = d.toolpath.stats().unwrap();
    let turn = |a: f64| 2.0 * (0.5 * PI * a).hypot(0.375) + (2.0 * PI * a).hypot(1.5);
    assert!(close(s.feed_length, 12.375 + 6.0 * 0.75 + 6.75 + 4.0 * (turn(first) + turn(1.5))), "{}", s.feed_length);
    assert!(d.notes.iter().any(|n| n.contains("drilled to 8.376 mm and 0.375 mm deeper than the thread")), "{:?}", d.notes);
    // A left-hand thread goes the same way round, from the top down.
    let left = threaded(&ThreadOp { hand: ThreadHand::Left, ..m10() }, &thread_mill()).unwrap();
    let c = cuts(&left.toolpath);
    let starts: Vec<f64> = c.iter().filter(|m| m.0.is_none()).map(|m| m.1.z).collect();
    assert!(starts.iter().zip([-0.625, -2.125, -3.625, -5.125, -0.625, -2.125, -3.625, -5.125]).all(|(z, want)| close(*z, want)), "{starts:?}");
    assert!(
        close(c[1].1.z, -1.0) && close(c[3].1.z, -2.5) && close(c[4].1.z, -2.875) && close(c[18].1.z, -7.0) && close(c[19].1.z, -7.375),
        "{:?}",
        &c[..5]
    );
    assert!(close(left.toolpath.stats().unwrap().feed_length, 5.625 + 6.0 * 0.75 + 6.75 + 4.0 * (turn(first) + turn(1.5))));
}

#[test]
fn teeth_for_the_whole_thread_cut_it_in_one_turn_and_several_holes_one_after_another() {
    // The same thread with a tool whose teeth run 6 mm (four pitches of 1.5): one turn makes
    // the whole thread. One pass asked: four arcs a hole, after one move down its middle.
    let mut full = thread_mill();
    full.geometry.flute_length = 6.0;
    let op = ThreadOp { holes: vec![[10.0, 5.0], [-20.0, 5.0], [-20.0, -15.0]], passes: Some(1), ..m10() };
    let d = threaded(&op, &full).unwrap();
    assert_eq!((d.turns, d.passes.clone()), (1, vec![1.5]));
    let c = cuts(&d.toolpath);
    assert_eq!(c.len(), 3 * 5);
    for (k, hole) in op.holes.iter().enumerate() {
        let m = &c[5 * k..5 * k + 5];
        assert!(m[0].0.is_none() && close(m[0].1.x, hole[0]) && close(m[0].1.y, hole[1]) && close(m[0].1.z, -7.375), "{m:?}");
        assert!(close(m[1].1.x, hole[0] + 1.5) && close(m[3].1.z, -5.5) && close(m[4].1.x, hole[0]) && close(m[4].1.z, -5.125), "{m:?}");
    }
    // Between holes the tool is at the retract height: up the hole's middle, across, and down.
    let rapids: Vec<Vec3> = d.toolpath.moves.iter().filter_map(|m| if let Move::Rapid { to } = *m { Some(to) } else { None }).collect();
    let want = [
        (10.0, 5.0, 25.0),
        (10.0, 5.0, 5.0),
        (10.0, 5.0, 5.0),
        (-20.0, 5.0, 5.0),
        (-20.0, 5.0, 5.0),
        (-20.0, -15.0, 5.0),
        (-20.0, -15.0, 5.0),
        (-20.0, -15.0, 25.0),
    ];
    assert!(
        rapids.len() == want.len() && rapids.iter().zip(want).all(|(at, w)| close(at.x, w.0) && close(at.y, w.1) && close(at.z, w.2)),
        "{rapids:?}"
    );
    // Teeth 1.5 apart cut that pitch and no other; one row of teeth cuts any.
    let e = threaded(&ThreadOp { pitch: Some(1.0), ..m10() }, &full).unwrap_err().to_string();
    assert!(e.contains("tool 8 has teeth 1.5 mm apart along 6 mm: it cuts that pitch, not 1 mm"), "{e}");
    let fine = threaded(&ThreadOp { pitch: Some(1.0), ..m10() }, &thread_mill()).unwrap();
    assert!(close(fine.minor, 10.0 - 2.0 * THREAD_DEPTH) && fine.turns == 6 && close(fine.pitch, 1.0), "{} {}", fine.minor, fine.turns);
    // Three passes: 57.7%, 81.6% and all of the depth.
    let three = threaded(&ThreadOp { passes: Some(3), ..m10() }, &full).unwrap();
    let cut = THREAD_DEPTH * 1.5;
    let want = [1.5 - (1.0 - (1.0_f64 / 3.0).sqrt()) * cut, 1.5 - (1.0 - (2.0_f64 / 3.0).sqrt()) * cut, 1.5];
    assert!(three.passes.iter().zip(want).all(|(a, w)| close(*a, w)), "{:?}", three.passes);
}

#[test]
fn what_cannot_be_thread_milled_is_refused_with_the_reason() {
    let mill = {
        let mut t = thread_mill();
        (t.kind, t.geometry.thread_pitch) = (ToolKind::FlatEndMill, 0.0);
        t
    };
    let cases: Vec<(ThreadOp, Tool, &str)> = vec![
        (m10(), mill, "does not mill a thread"),
        (ThreadOp { holes: vec![], ..m10() }, thread_mill(), "cuts 1 to 1000 holes"),
        (ThreadOp { holes: vec![[f64::NAN, 0.0]], ..m10() }, thread_mill(), "the holes' places"),
        // M8 x 1.25 is drilled to 6.647: the 7 mm tool does not go in.
        (ThreadOp { diameter: 8.0, pitch: Some(1.25), ..m10() }, thread_mill(), "does not go into a hole drilled for this thread (6.647 mm)"),
        (ThreadOp { diameter: 0.0, ..m10() }, thread_mill(), "major diameter"),
        (ThreadOp { pitch: Some(0.0), ..m10() }, thread_mill(), "the thread's pitch must be"),
        (ThreadOp { pitch: Some(f64::NAN), ..m10() }, thread_mill(), "the thread's pitch must be"),
        (ThreadOp { depth: 0.0, ..m10() }, thread_mill(), "the thread's depth must be above 0"),
        (ThreadOp { depth: 19.8, ..m10() }, thread_mill(), "more than the 20 mm tool 8 reaches down a hole"),
        (ThreadOp { passes: Some(0), ..m10() }, thread_mill(), "1 to 20 passes"),
        (ThreadOp { passes: Some(21), ..m10() }, thread_mill(), "1 to 20 passes"),
        (ThreadOp { top: Some(5.0), ..m10() }, thread_mill(), "the retract height (5 mm) must be above the holes' top (5 mm)"),
        (ThreadOp { rpm: Some(0.0), ..m10() }, thread_mill(), "rpm must be above 0"),
        (ThreadOp { feed: Some(-1.0), ..m10() }, thread_mill(), "feed must be above 0"),
    ];
    for (op, tool, why) in cases {
        let e = threaded(&op, &tool).unwrap_err().to_string();
        assert!(e.contains(why), "{op:?}: {e}");
    }
    // The thread's bottom a quarter of a pitch short of the tool's reach is the deepest it goes.
    assert!(threaded(&ThreadOp { depth: 19.625, ..m10() }, &thread_mill()).is_ok());
}

#[test]
fn hostile_numbers_never_panic_and_a_thread_is_plain_json() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, 5e-324, f64::MAX, f64::MIN];
    for &v in &bad {
        for field in 0..7 {
            let mut op = m10();
            match field {
                0 => op.diameter = v,
                1 => op.pitch = Some(v),
                2 => op.top = Some(v),
                3 => op.depth = v,
                4 => op.rpm = Some(v),
                5 => op.feed = Some(v),
                _ => op.holes = vec![[v, v]],
            }
            let _ = threaded(&op, &thread_mill());
        }
        let _ = m10().toolpath(Some(p(v, v, v)), &thread_mill(), &placed(), Some("aluminium"));
        for size in 0..4 {
            let mut tool = thread_mill();
            match size {
                0 => tool.geometry.diameter = v,
                1 => tool.geometry.thread_pitch = v,
                2 => tool.geometry.flute_length = v,
                _ => tool.geometry.shoulder_length = v,
            }
            if let Ok(d) = threaded(&m10(), &tool) {
                assert!(d.passes.iter().all(|a| a.is_finite() && *a > 0.0) && d.turns >= 1, "{v}: {d:?}");
            }
        }
    }
    for passes in [0, 1, 20, u32::MAX] {
        let _ = threaded(&ThreadOp { passes: Some(passes), ..m10() }, &thread_mill());
    }
    let op: ThreadOp = serde_json::from_str(r#"{"holes": [[10, 5]], "diameter": 10, "depth": 6}"#).unwrap();
    assert_eq!(op, m10());
    let op: ThreadOp = serde_json::from_str(r#"{"holes": [[0, 0]], "diameter": 8, "depth": 5, "pitch": 1.25, "hand": "left", "passes": 3}"#).unwrap();
    assert_eq!((op.pitch, op.hand, op.passes), (Some(1.25), ThreadHand::Left, Some(3)));
    assert!(serde_json::from_str::<ThreadOp>(r#"{"holes": [[0, 0]], "diameter": 8}"#).is_err());
    assert!(serde_json::from_str::<ThreadOp>(r#"{"holes": [[0, 0]], "diameter": 8, "depth": 5, "taper": 1}"#).is_err());
    assert_eq!(serde_json::from_value::<ThreadOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
