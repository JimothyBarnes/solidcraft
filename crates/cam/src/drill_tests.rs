use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// A 10 mm, 2-flute, 118° drill with 40 mm of flutes and data for aluminium: 100 m/min, 0.1 mm a
/// tooth. Its speed is 100 000 / (10 π) rev/min, its feed 0.1 x 2 a revolution.
fn drill10() -> Tool {
    Tool {
        number: 3,
        name: "10 mm drill".into(),
        kind: ToolKind::Drill,
        geometry: ToolGeometry {
            diameter: 10.0,
            corner_radius: 0.0,
            flute_length: 40.0,
            shoulder_length: 0.0,
            overall_length: 90.0,
            shank_diameter: 10.0,
            tip_angle: 118f64.to_radians(),
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 2,
        holder: Holder { stick_out: 50.0, segments: vec![] },
        length_offset: 3,
        diameter_offset: 3,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 100.0, chip_load: 0.1, plunge_factor: 0.5, ramp_factor: 0.5 }],
        overrides: Overrides::default(),
    }
}

const RPM: f64 = 100_000.0 / (10.0 * PI);

/// The vise setup on a 60 x 40 x 20 block: the stock top is Z 0 in the WCS, clearance 25, retract 5.
fn placed() -> (Placement, SafeHeights) {
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
    let placement = setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }).unwrap();
    (placement, setup.safe_heights)
}

fn op(holes: Vec<[f64; 2]>, depth: f64) -> DrillOp {
    DrillOp {
        holes,
        order: HoleOrder::Nearest,
        top: None,
        depth,
        depth_to: DepthTo::Tip,
        r_above: None,
        between: Between::RPlane,
        cycle: DrillCycle::Peck { peck: 4.0 },
        rpm: None,
        feed: None,
    }
}

fn drilled(o: &DrillOp, tool: &Tool) -> Result<Drilled> {
    let (placement, safe) = placed();
    o.toolpath(None, tool, &placement, safe, Some("aluminium"))
}

#[test]
fn three_holes_drill_nearest_first_from_the_stock_top_with_the_tools_feeds() {
    // From (0, 0) the nearest is (5, 0), then (20, 0). Top 0 (the stock's), R 5 (the setup's
    // retract above it), 12 deep to the tip, back to R between holes.
    let d = drilled(&op(vec![[0.0, 0.0], [20.0, 0.0], [5.0, 0.0]], 12.0), &drill10()).unwrap();
    assert!(close(d.rpm, RPM) && close(d.feed, 0.2 * RPM), "{} {}", d.rpm, d.feed);
    assert_eq!((d.bottom, d.r, d.clearance), (-12.0, 5.0, 25.0));
    let hole = |x: f64| Move::Drill { to: p(x, 0.0, -12.0), r: 5.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 4.0 }, feed: d.feed };
    assert_eq!(d.toolpath.start, p(0.0, 0.0, 25.0));
    assert_eq!(
        d.toolpath.moves,
        vec![
            Move::ToolChange { tool: 3 },
            Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(0.0, 0.0, 25.0) },
            hole(0.0),
            hole(5.0),
            hole(20.0),
            Move::Rapid { to: p(20.0, 0.0, 25.0) },
            Move::Coolant { mode: Coolant::Off },
            Move::SpindleStop,
        ]
    );
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    // As given keeps the list's order.
    let mut given = op(vec![[0.0, 0.0], [20.0, 0.0], [5.0, 0.0]], 12.0);
    given.order = HoleOrder::AsGiven;
    let d = drilled(&given, &drill10()).unwrap();
    let xs: Vec<f64> = d.toolpath.moves.iter().filter_map(|m| if let Move::Drill { to, .. } = m { Some(to.x) } else { None }).collect();
    assert_eq!(xs, vec![0.0, 20.0, 5.0]);
}

#[test]
fn heights_follow_the_operation_when_it_sets_them() {
    let mut o = op(vec![[0.0, 0.0], [10.0, 0.0]], 10.0);
    // To full diameter: the 118° point is 5 / tan(59°) longer.
    o.depth_to = DepthTo::FullDiameter;
    o.top = Some(-3.0);
    o.r_above = Some(2.0);
    o.between = Between::Clearance;
    let d = drilled(&o, &drill10()).unwrap();
    let point = 5.0 / 59f64.to_radians().tan();
    assert!(close(d.bottom, -13.0 - point), "{}", d.bottom);
    assert_eq!(d.r, -1.0);
    // Clamps in the way: up to clearance (25) between holes, so there is no rapid up after the last.
    let drills: Vec<&Move> = d.toolpath.moves.iter().filter(|m| matches!(m, Move::Drill { .. })).collect();
    assert!(drills.iter().all(|m| matches!(m, Move::Drill { retract, .. } if *retract == 25.0)));
    assert!(matches!(d.toolpath.moves.get(d.toolpath.moves.len() - 3), Some(Move::Drill { .. })));
    // A flat end mill has no point: full diameter is its tip.
    let mut mill = drill10();
    mill.kind = ToolKind::FlatEndMill;
    mill.geometry.tip_angle = 0.0;
    assert_eq!(drilled(&o, &mill).unwrap().bottom, -13.0);
}

#[test]
fn feeds_given_or_derived_and_capped_are_said_so() {
    let mut o = op(vec![[0.0, 0.0]], 5.0);
    // Both given: used as they are.
    o.rpm = Some(1200.0);
    o.feed = Some(90.0);
    let d = drilled(&o, &drill10()).unwrap();
    assert_eq!((d.rpm, d.feed), (1200.0, 90.0));
    // Only the speed given: the feed keeps the tool's chip, 0.2 mm a revolution.
    o.feed = None;
    let d = drilled(&o, &drill10()).unwrap();
    assert!(close(d.feed, 240.0), "{}", d.feed);
    // A 1 mm drill would spin at 31 831 rev/min: capped at the generic 10 000, and noted.
    let mut small = drill10();
    small.geometry.diameter = 1.0;
    small.geometry.shank_diameter = 1.0;
    small.cutting_data[0].chip_load = 0.02;
    let d = drilled(&op(vec![[0.0, 0.0]], 5.0), &small).unwrap();
    assert_eq!(d.rpm, GENERIC_MAX_RPM);
    assert!(close(d.feed, 0.04 * GENERIC_MAX_RPM), "{}", d.feed);
    assert!(d.notes.iter().any(|n| n.contains("capped at 10000 rpm")), "{:?}", d.notes);
    // A plunging end mill feeds at its plunge feed: 3 flutes at 0.05, half of it.
    let mut mill = drill10();
    mill.kind = ToolKind::FlatEndMill;
    mill.geometry.tip_angle = 0.0;
    mill.flutes = 3;
    mill.cutting_data[0].chip_load = 0.05;
    let d = drilled(&op(vec![[0.0, 0.0]], 5.0), &mill).unwrap();
    assert!(close(d.feed, 0.5 * 0.15 * RPM), "{}", d.feed);
    // No material on the setup and nothing typed in: the error says both ways out.
    let (placement, safe) = placed();
    let e = op(vec![[0.0, 0.0]], 5.0).toolpath(None, &drill10(), &placement, safe, None).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
    let e = op(vec![[0.0, 0.0]], 5.0).toolpath(None, &drill10(), &placement, safe, Some("steel")).unwrap_err().to_string();
    assert!(e.contains("no cutting data for 'steel'"), "{e}");
}

#[test]
fn what_a_tool_or_the_heights_cannot_do_is_refused() {
    let tap = {
        let mut t = drill10();
        t.kind = ToolKind::Tap;
        t.geometry.tip_angle = 0.0;
        t.geometry.thread_pitch = 1.5;
        t
    };
    let ball = {
        let mut t = drill10();
        t.kind = ToolKind::BallEndMill;
        t.geometry.tip_angle = 0.0;
        t.geometry.corner_radius = 5.0;
        t
    };
    let base = op(vec![[0.0, 0.0]], 5.0);
    let with = |f: &dyn Fn(&mut DrillOp)| {
        let mut o = base.clone();
        f(&mut o);
        o
    };
    let cases: Vec<(DrillOp, Tool, &str)> = vec![
        (base.clone(), tap, "tapping (G84)"),
        (base.clone(), ball, "does not drill"),
        (with(&|o| o.depth = 41.0), drill10(), "past tool 3's 40 mm of flutes"),
        (with(&|o| o.holes.clear()), drill10(), "1 to 10000 holes"),
        (with(&|o| o.holes = vec![[f64::NAN, 0.0]]), drill10(), "finite"),
        (with(&|o| o.top = Some(f64::INFINITY)), drill10(), "top"),
        (with(&|o| o.depth = 0.0), drill10(), "depth"),
        (with(&|o| o.depth = f64::NAN), drill10(), "depth"),
        (with(&|o| o.r_above = Some(0.0)), drill10(), "R plane"),
        (with(&|o| o.r_above = Some(30.0)), drill10(), "above the clearance height"),
        (with(&|o| o.rpm = Some(-1.0)), drill10(), "rpm"),
        (with(&|o| o.feed = Some(f64::NAN)), drill10(), "feed"),
        (with(&|o| o.cycle = DrillCycle::Peck { peck: 0.0 }), drill10(), "peck"),
    ];
    for (o, tool, expect) in cases {
        let e = drilled(&o, &tool).unwrap_err().to_string();
        assert!(e.contains(expect), "{o:?}: {e:?} does not mention {expect:?}");
    }
}

#[test]
fn a_drill_operation_is_plain_json_with_defaults() {
    let o: DrillOp = serde_json::from_str(r#"{"holes": [[1, 2]], "depth": 8, "cycle": {"type": "drill", "dwell": 0}}"#).unwrap();
    assert_eq!((o.order, o.depth_to, o.between, o.top, o.r_above), (HoleOrder::Nearest, DepthTo::Tip, Between::RPlane, None, None));
    assert!(serde_json::from_str::<DrillOp>(r#"{"holes": [], "depth": 8, "cycle": {"type": "drill", "dwell": 0}, "spin": 1}"#).is_err());
    let back: DrillOp = serde_json::from_value(serde_json::to_value(&o).unwrap()).unwrap();
    assert_eq!(back, o);
}

#[test]
fn many_holes_are_ordered_without_trouble() {
    // A 100 x 100 grid, listed column by column: nearest-first visits every hole once.
    let holes: Vec<[f64; 2]> = (0..100).flat_map(|i| (0..100).map(move |j| [f64::from(i) * 5.0, f64::from(j) * 5.0])).collect();
    let d = drilled(&op(holes, 5.0), &drill10()).unwrap();
    let drilled_holes = d.toolpath.moves.iter().filter(|m| matches!(m, Move::Drill { .. })).count();
    assert_eq!(drilled_holes, MAX_HOLES);
    // Each step is to a neighbour 5 mm away: 9 999 steps, 49 995 mm across at R.
    let s = d.toolpath.stats().unwrap();
    assert!(s.rapid_length > 49_995.0, "{}", s.rapid_length);
}
