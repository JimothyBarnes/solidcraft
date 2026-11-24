use std::f64::consts::{FRAC_PI_2, PI};

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

fn line(a: (f64, f64), b: (f64, f64)) -> Seg2 {
    Seg2::Line { a: q(a.0, a.1), b: q(b.0, b.1) }
}

/// Tool 7: a 2-flute, 90 degree engraving tool 6 mm across with a tip of 0.2 and 6 mm of
/// flutes, and data for aluminium (150 m/min, 0.02 mm a tooth, plunging at half).
fn engraver() -> Tool {
    Tool {
        number: 7,
        name: "6 mm 90 degree engraver".into(),
        kind: ToolKind::Engraving,
        geometry: ToolGeometry {
            diameter: 6.0,
            corner_radius: 0.0,
            flute_length: 6.0,
            shoulder_length: 0.0,
            overall_length: 50.0,
            shank_diameter: 6.0,
            tip_angle: FRAC_PI_2,
            tip_diameter: 0.2,
            thread_pitch: 0.0,
        },
        flutes: 2,
        holder: Holder { stick_out: 25.0, segments: vec![] },
        length_offset: 7,
        diameter_offset: 7,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 150.0, chip_load: 0.02, plunge_factor: 0.5, ramp_factor: 0.5 }],
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

fn mark() -> EngraveOp {
    EngraveOp { sketch: "Mark".into(), ..EngraveOp::default() }
}

/// An engraving of `pieces` drawn on the block's top (Z -1), from nowhere.
fn engraved(op: &EngraveOp, pieces: &[Seg2]) -> Result<Engraved> {
    op.toolpath(None, &engraver(), &placed(), Some("aluminium"), -1.0, pieces)
}

/// Where the moves that are not about the spindle or the coolant go, and whether each is a
/// rapid.
fn places(path: &Toolpath) -> Vec<(bool, Vec3)> {
    path.moves
        .iter()
        .filter_map(|m| match *m {
            Move::Rapid { to } => Some((true, to)),
            Move::Linear { to, .. } | Move::Arc { to, .. } => Some((false, to)),
            _ => None,
        })
        .collect()
}

fn goes(path: &Toolpath, want: &[(bool, (f64, f64, f64))]) -> bool {
    let got = places(path);
    got.len() == want.len()
        && got.iter().zip(want).all(|((rapid, at), (fast, to))| rapid == fast && close(at.x, to.0) && close(at.y, to.1) && close(at.z, to.2))
}

#[test]
fn an_engraving_follows_the_curves_joined_end_to_end_at_its_depth() {
    // An L drawn on the block's top: 10 along X and 5 up, given as two lines in the other
    // order, one of them drawn backwards. They meet at (10, 0), so they are one run of 15,
    // cut from the free end it finds first: (10, 5).
    let l = [line((10.0, 5.0), (10.0, 0.0)), line((0.0, 0.0), (10.0, 0.0))];
    let d = engraved(&mark(), &l).unwrap();
    // 0.2 deep unless told: the tip at Z -1.2. The 90 degree cone with its tip of 0.2 leaves a
    // groove 0.2 + 2 x 0.2 = 0.6 wide at the surface.
    assert!(close(d.top, -1.0) && close(d.z, -1.2) && close(d.depth, 0.2) && close(d.width, 0.6), "{d:?}");
    assert!(d.runs.len() == 1 && close(d.runs[0], 15.0), "{:?}", d.runs);
    // Above the run's start at the clearance height, down to the retract height at rapid and
    // from there at the plunge feed, along the run, and up where it ends.
    let want = [
        (true, (10.0, 5.0, 25.0)),
        (true, (10.0, 5.0, 5.0)),
        (false, (10.0, 5.0, -1.2)),
        (false, (10.0, 0.0, -1.2)),
        (false, (0.0, 0.0, -1.2)),
        (true, (0.0, 0.0, 5.0)),
        (true, (0.0, 0.0, 25.0)),
    ];
    assert!(goes(&d.toolpath, &want), "{:#?}", d.toolpath.moves);
    // The speed is the tool's own at its full 6 mm (150 m/min), the feed 0.02 mm a tooth on two
    // teeth, and the tool goes down at half of it.
    let rpm = 150_000.0 / (PI * 6.0);
    assert!(close(d.rpm, rpm) && close(d.feed, 0.04 * rpm) && close(d.plunge_feed, 0.02 * rpm), "{} {} {}", d.rpm, d.feed, d.plunge_feed);
    let feeds: Vec<f64> = d.toolpath.moves.iter().filter_map(|m| if let Move::Linear { feed, .. } = *m { Some(feed) } else { None }).collect();
    assert!(feeds.len() == 3 && close(feeds[0], d.plunge_feed) && close(feeds[1], d.feed) && close(feeds[2], d.feed), "{feeds:?}");
    assert!(matches!(d.toolpath.moves[0], Move::ToolChange { tool: 7 }) && matches!(d.toolpath.moves.last(), Some(Move::SpindleStop)));
    // From where the tool is, the run is cut from its nearer end: turned round for a tool
    // above (0, 0).
    let d = mark().toolpath(Some(p(0.0, 0.0, 25.0)), &engraver(), &placed(), Some("aluminium"), -1.0, &l).unwrap();
    let want = [
        (true, (0.0, 0.0, 25.0)),
        (true, (0.0, 0.0, 5.0)),
        (false, (0.0, 0.0, -1.2)),
        (false, (10.0, 0.0, -1.2)),
        (false, (10.0, 5.0, -1.2)),
        (true, (10.0, 5.0, 5.0)),
        (true, (10.0, 5.0, 25.0)),
    ];
    assert!(goes(&d.toolpath, &want), "{:#?}", d.toolpath.moves);
    // A depth and a surface of its own: 0.5 below Z -3.
    let deeper = engraved(&EngraveOp { depth: Some(0.5), top: Some(-3.0), ..mark() }, &l).unwrap();
    assert!(close(deeper.z, -3.5) && close(deeper.width, 1.2), "{} {}", deeper.z, deeper.width);
}

#[test]
fn curves_that_do_not_meet_are_runs_of_their_own_each_after_the_nearest() {
    // A line from (0, 0) to (10, 0), a circle of radius 5 about (30, 0) (it starts and ends at
    // (35, 0)), and a line from (60, 20) to (40, 0), given circle first.
    let circle = Seg2::Arc { center: q(30.0, 0.0), radius: 5.0, start: 0.0, sweep: 2.0 * PI };
    let pieces = [circle, line((0.0, 0.0), (10.0, 0.0)), line((60.0, 20.0), (40.0, 0.0))];
    // From above the origin: the first line, then of what is left the end nearest (10, 0),
    // which is the circle's start, 25 away (the other line's nearer end is 30 away), then that
    // line from (40, 0), turned round.
    let d = mark().toolpath(Some(p(0.0, 0.0, 25.0)), &engraver(), &placed(), Some("aluminium"), -1.0, &pieces).unwrap();
    assert!(d.runs.len() == 3 && close(d.runs[0], 10.0) && close(d.runs[1], 10.0 * PI) && close(d.runs[2], 800.0_f64.sqrt()), "{:?}", d.runs);
    let z = -1.2;
    let want = [
        (true, (0.0, 0.0, 25.0)),
        (true, (0.0, 0.0, 5.0)),
        (false, (0.0, 0.0, z)),
        (false, (10.0, 0.0, z)),
        (true, (10.0, 0.0, 5.0)),
        (true, (35.0, 0.0, 5.0)),
        (false, (35.0, 0.0, z)),
        (false, (25.0, 0.0, z)),
        (false, (35.0, 0.0, z)),
        (true, (35.0, 0.0, 5.0)),
        (true, (40.0, 0.0, 5.0)),
        (false, (40.0, 0.0, z)),
        (false, (60.0, 20.0, z)),
        (true, (60.0, 20.0, 5.0)),
        (true, (60.0, 20.0, 25.0)),
    ];
    assert!(goes(&d.toolpath, &want), "{:#?}", d.toolpath.moves);
    // The circle is cut as two half turns, counter-clockwise as it was drawn.
    let arcs = d.toolpath.moves.iter().filter(|m| matches!(m, Move::Arc { direction: crate::toolpath::ArcDirection::CounterClockwise, .. })).count();
    assert_eq!(arcs, 2);
    // Where three ends meet no run goes on: which way is not the tool's to choose. Three lines
    // from one point are three runs.
    let star = [line((0.0, 0.0), (10.0, 0.0)), line((0.0, 0.0), (0.0, 10.0)), line((-10.0, 0.0), (0.0, 0.0))];
    let d = engraved(&mark(), &star).unwrap();
    assert!(d.runs.len() == 3 && d.runs.iter().all(|&run| close(run, 10.0)), "{:?}", d.runs);
    // Two squares' sides, all sixteen pieces mixed up and some drawn backwards (as a letter's
    // outline comes): two closed runs, 40 and 16 round, and the tool goes down twice.
    let square = |x: f64, side: f64| {
        let c = [(x, 0.0), (x + side, 0.0), (x + side, side), (x, side)];
        (0..4).flat_map(move |k| {
            let (a, b) = (c[k], c[(k + 1) % 4]);
            let mid = (0.5 * (a.0 + b.0), 0.5 * (a.1 + b.1));
            [line(a, mid), if k % 2 == 0 { line(b, mid) } else { line(mid, b) }]
        })
    };
    let mut pieces: Vec<Seg2> = square(0.0, 10.0).chain(square(20.0, 4.0)).collect();
    pieces.swap(1, 9);
    pieces.swap(3, 14);
    pieces.reverse();
    let d = engraved(&mark(), &pieces).unwrap();
    let mut runs = d.runs.clone();
    runs.sort_by(f64::total_cmp);
    assert!(runs.len() == 2 && close(runs[0], 16.0) && close(runs[1], 40.0), "{:?}", d.runs);
    let plunges = d.toolpath.moves.iter().filter(|m| matches!(m, Move::Linear { feed, .. } if close(*feed, d.plunge_feed))).count();
    assert_eq!(plunges, 2);
    assert!(close(d.toolpath.stats().unwrap().feed_length, 56.0 + 2.0 * 6.2), "{}", d.toolpath.stats().unwrap().feed_length);
}

#[test]
fn what_cannot_be_engraved_is_refused_with_the_reason() {
    let l = [line((0.0, 0.0), (10.0, 0.0))];
    let drill = {
        let mut t = engraver();
        t.kind = ToolKind::Drill;
        t
    };
    let cases: Vec<(EngraveOp, Tool, &str)> = vec![
        (EngraveOp::default(), engraver(), "follows a sketch"),
        (EngraveOp { sketch: "  ".into(), ..EngraveOp::default() }, engraver(), "follows a sketch"),
        (EngraveOp { depth: Some(0.0), ..mark() }, engraver(), "the depth must be"),
        (EngraveOp { depth: Some(-0.1), ..mark() }, engraver(), "the depth must be"),
        (EngraveOp { depth: Some(f64::NAN), ..mark() }, engraver(), "the depth must be"),
        (EngraveOp { depth: Some(6.5), ..mark() }, engraver(), "past tool 7's 6 mm of flutes"),
        (EngraveOp { top: Some(f64::INFINITY), ..mark() }, engraver(), "the surface's height"),
        (EngraveOp { top: Some(5.0), ..mark() }, engraver(), "the retract height (5 mm) must be above the engraved surface (5 mm)"),
        (EngraveOp { rpm: Some(0.0), ..mark() }, engraver(), "rpm must be above 0"),
        (EngraveOp { feed: Some(-1.0), ..mark() }, engraver(), "feed must be above 0"),
        (mark(), drill, "does not engrave"),
    ];
    for (op, tool, why) in cases {
        let e = op.toolpath(None, &tool, &placed(), Some("aluminium"), -1.0, &l).unwrap_err().to_string();
        assert!(e.contains(why), "{op:?}: {e}");
    }
    // A sketch with nothing drawn, and one whose curves were not made lines and arcs.
    assert!(engraved(&mark(), &[]).unwrap_err().to_string().contains("sketch 'Mark' has no curves to follow"));
    let bent = Seg2::Cubic { p0: q(0.0, 0.0), p1: q(1.0, 2.0), p2: q(2.0, 2.0), p3: q(3.0, 0.0) };
    assert!(engraved(&mark(), &[bent]).unwrap_err().to_string().contains("must be lines and arcs"));
    // Pieces of no length are not curves: with nothing else there is nothing to follow.
    assert!(engraved(&mark(), &[line((1.0, 1.0), (1.0, 1.0))]).unwrap_err().to_string().contains("has no curves to follow"));
    // An end mill engraves too: its groove is as wide as it is; a ball's is its chord.
    let mut flat = engraver();
    (flat.kind, flat.geometry.tip_angle, flat.geometry.tip_diameter) = (ToolKind::FlatEndMill, 0.0, 0.0);
    assert_eq!(mark().width(&flat), 6.0);
    let mut ball = flat.clone();
    (ball.kind, ball.geometry.corner_radius) = (ToolKind::BallEndMill, 3.0);
    assert!(close(mark().width(&ball), 2.0 * (0.2_f64 * 5.8).sqrt()), "{}", mark().width(&ball));
    assert!(mark().toolpath(None, &flat, &placed(), Some("aluminium"), -1.0, &l).is_ok());
}

#[test]
fn hostile_numbers_never_panic_and_an_engraving_is_plain_json() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, 5e-324, f64::MAX, f64::MIN];
    let l = [line((0.0, 0.0), (10.0, 0.0)), line((10.0, 0.0), (10.0, 5.0))];
    for &v in &bad {
        for field in 0..4 {
            let mut op = mark();
            match field {
                0 => op.depth = Some(v),
                1 => op.top = Some(v),
                2 => op.rpm = Some(v),
                _ => op.feed = Some(v),
            }
            let _ = engraved(&op, &l);
        }
        // Curves of hostile numbers, a surface of one, and a tool of hostile sizes.
        let odd = [
            line((v, 0.0), (10.0, v)),
            line((v, v), (v, v)),
            Seg2::Arc { center: q(v, 0.0), radius: v, start: v, sweep: v },
            Seg2::Arc { center: q(0.0, 0.0), radius: 5.0, start: 0.0, sweep: v },
            line((0.0, 0.0), (10.0, 0.0)),
        ];
        let _ = engraved(&mark(), &odd);
        let _ = mark().toolpath(None, &engraver(), &placed(), Some("aluminium"), v, &l);
        let _ = mark().toolpath(Some(p(v, v, v)), &engraver(), &placed(), Some("aluminium"), -1.0, &l);
        for size in 0..4 {
            let mut tool = engraver();
            match size {
                0 => tool.geometry.diameter = v,
                1 => tool.geometry.tip_angle = v,
                2 => tool.geometry.tip_diameter = v,
                _ => tool.geometry.flute_length = v,
            }
            if let Ok(d) = mark().toolpath(None, &tool, &placed(), Some("aluminium"), -1.0, &l) {
                assert!(d.z.is_finite() && d.runs.len() == 1, "{v}: {d:?}");
            }
        }
    }
    let op: EngraveOp = serde_json::from_str(r#"{"sketch": "Part number"}"#).unwrap();
    assert_eq!(op, EngraveOp { sketch: "Part number".into(), ..EngraveOp::default() });
    let op: EngraveOp = serde_json::from_str(r#"{"sketch": "Logo", "depth": 0.1, "top": -2, "rpm": 12000, "feed": 300}"#).unwrap();
    assert_eq!((op.depth, op.top, op.rpm, op.feed), (Some(0.1), Some(-2.0), Some(12_000.0), Some(300.0)));
    assert!(serde_json::from_str::<EngraveOp>(r#"{"sketch": "Logo", "width": 1}"#).is_err());
    assert!(serde_json::from_str::<EngraveOp>("{}").is_err());
    assert_eq!(serde_json::from_value::<EngraveOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
