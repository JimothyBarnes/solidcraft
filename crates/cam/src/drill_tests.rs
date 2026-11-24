use std::f64::consts::PI;

use super::*;
use crate::feeds::GENERIC_MAX_RPM;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
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

/// The vise setup on a 60 x 40 x 20 block with 1 mm of stock on top: the stock top is Z 0 in the
/// WCS and the part's top Z -1, clearance 25, retract 5.
fn placed() -> Placement {
    placed_with(1.0)
}

fn placed_with(on_top: f64) -> Placement {
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
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: on_top } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    };
    setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }).unwrap()
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
    o.toolpath(None, tool, &placed(), Some("aluminium"))
}

#[test]
fn three_holes_drill_nearest_first_from_the_parts_top_with_the_tools_feeds() {
    // From (0, 0) the nearest is (5, 0), then (20, 0). 12 deep to the tip from the part's top (-1):
    // bottom -13. R 5: the setup's retract above the stock's top (0). Back to R between holes.
    let d = drilled(&op(vec![[0.0, 0.0], [20.0, 0.0], [5.0, 0.0]], 12.0), &drill10()).unwrap();
    assert!(close(d.rpm, RPM) && close(d.feed, 0.2 * RPM), "{} {}", d.rpm, d.feed);
    assert_eq!((d.bottom, d.r, d.clearance), (-13.0, 5.0, 25.0));
    let hole = |x: f64| Move::Drill { to: p(x, 0.0, -13.0), r: 5.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 4.0 }, feed: d.feed };
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
fn a_holes_depth_is_the_parts_whatever_stock_is_on_it() {
    // 1 mm or 6 mm of stock on top: the WCS origin rides the stock top, but the hole ends 12 mm
    // below the part's top (Z 20 on the model, so Z 8) either way, and R stays 5 above the stock.
    for on_top in [1.0, 6.0] {
        let placement = placed_with(on_top);
        let d = op(vec![[0.0, 0.0]], 12.0).toolpath(None, &drill10(), &placement, Some("aluminium")).unwrap();
        assert_eq!(d.bottom + placement.origin.z, 8.0, "{on_top} mm on top");
        assert_eq!((d.r, d.r + placement.origin.z), (5.0, 25.0 + on_top), "{on_top} mm on top");
    }
    // With no stock on top the R plane is the retract height above the part itself.
    let d = op(vec![[0.0, 0.0]], 12.0).toolpath(None, &drill10(), &placed_with(0.0), Some("aluminium")).unwrap();
    assert_eq!((d.bottom, d.r), (-12.0, 5.0));
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
    // The same drill in a setup on the Haas VF-4SS (decision 0036): its spindle's 12 000, and
    // the feed keeps the chip.
    let mut haas = placed();
    haas.max_rpm = crate::machine::machine(Some("haas_vf4ss")).unwrap().max_rpm;
    let d = op(vec![[0.0, 0.0]], 5.0).toolpath(None, &small, &haas, Some("aluminium")).unwrap();
    assert_eq!(d.rpm, 12_000.0);
    assert!(close(d.feed, 0.04 * 12_000.0) && d.notes.iter().any(|n| n.contains("capped at 12000 rpm")), "{} {:?}", d.feed, d.notes);
    // A plunging end mill feeds at its plunge feed: 3 flutes at 0.05, half of it.
    let mut mill = drill10();
    mill.kind = ToolKind::FlatEndMill;
    mill.geometry.tip_angle = 0.0;
    mill.flutes = 3;
    mill.cutting_data[0].chip_load = 0.05;
    let d = drilled(&op(vec![[0.0, 0.0]], 5.0), &mill).unwrap();
    assert!(close(d.feed, 0.5 * 0.15 * RPM), "{}", d.feed);
    // No material on the setup and nothing typed in: the error says both ways out.
    let placement = placed();
    let e = op(vec![[0.0, 0.0]], 5.0).toolpath(None, &drill10(), &placement, None).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
    let e = op(vec![[0.0, 0.0]], 5.0).toolpath(None, &drill10(), &placement, Some("steel")).unwrap_err().to_string();
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
        (base.clone(), tap.clone(), "tool 3 is a tap: it cuts with the tap cycle, at its pitch of 1.5 mm"),
        (with(&|o| o.cycle = DrillCycle::Tap { pitch: 1.5 }), drill10(), "the tap cycle needs a tap, and tool 3 is a Drill"),
        (with(&|o| o.cycle = DrillCycle::Tap { pitch: 1.25 }), tap.clone(), "the cycle's pitch (1.25 mm) is not tool 3's (1.5 mm)"),
        (with(&|o| (o.cycle, o.feed) = (DrillCycle::Tap { pitch: 1.5 }, Some(100.0))), tap.clone(), "give the speed (rpm), not a feed"),
        (
            with(&|o| o.cycle = DrillCycle::Tap { pitch: 0.0 }),
            {
                let mut t = tap.clone();
                t.geometry.thread_pitch = 0.0;
                t
            },
            "a tap with no thread pitch",
        ),
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

#[test]
fn a_tap_turns_a_whole_number_of_times_a_minute_and_feeds_one_pitch_a_turn() {
    // An M10 x 1.5 tap with data for aluminium at 10 m/min: 10 000 / (10 pi) = 318.3 rev/min,
    // taken as 318, and 1.5 x 318 = 477 mm/min. 12 deep from the part's top, back to R.
    let tap = {
        let mut t = drill10();
        (t.kind, t.name) = (ToolKind::Tap, "M10 x 1.5 tap".into());
        (t.geometry.tip_angle, t.geometry.thread_pitch) = (0.0, 1.5);
        t.cutting_data[0].surface_speed = 10.0;
        t
    };
    let tapping = DrillOp { cycle: DrillCycle::Tap { pitch: 1.5 }, ..op(vec![[0.0, 0.0], [20.0, 0.0]], 12.0) };
    let d = drilled(&tapping, &tap).unwrap();
    assert_eq!((d.rpm, d.feed, d.bottom, d.r), (318.0, 477.0, -13.0, 5.0));
    let hole = |x: f64| Move::Drill { to: p(x, 0.0, -13.0), r: 5.0, retract: 5.0, cycle: DrillCycle::Tap { pitch: 1.5 }, feed: 477.0 };
    assert_eq!(
        d.toolpath.moves,
        vec![
            Move::ToolChange { tool: 3 },
            Move::Spindle { rpm: 318.0, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(0.0, 0.0, 25.0) },
            hole(0.0),
            hole(20.0),
            Move::Rapid { to: p(20.0, 0.0, 25.0) },
            Move::Coolant { mode: Coolant::Off },
            Move::SpindleStop,
        ]
    );
    // A speed typed in is used, to the whole turn, and the feed follows it.
    let d = drilled(&DrillOp { rpm: Some(250.4), ..tapping.clone() }, &tap).unwrap();
    assert_eq!((d.rpm, d.feed), (250.0, 375.0));
    // It posts as G84 and reads back as the tapped holes it is.
    let ctx = crate::ProgramContext { program_number: 7, program_name: "Tapped".into(), ..crate::ProgramContext::default() };
    let posted = crate::Post::builtin("haas_mill").unwrap().write(&drilled(&tapping, &tap).unwrap().toolpath, &ctx).unwrap();
    assert!(posted.text.contains("\nS318 M3\n") && posted.text.contains("\nG99 G84 X0. Y0. Z-13. R5. F477.\nX20. Y0.\nG80\n"), "{}", posted.text);
}
