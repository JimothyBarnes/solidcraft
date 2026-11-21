use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// Tool 4: a 20 mm, 4-flute flat end mill with 30 mm of flutes and data for aluminium (200 m/min,
/// 0.05 mm a tooth, plunging at half): 200 000 / (20 π) rev/min, 0.2 mm a revolution.
fn mill20() -> Tool {
    Tool {
        number: 4,
        name: "20 mm flat end mill".into(),
        kind: ToolKind::FlatEndMill,
        geometry: ToolGeometry {
            diameter: 20.0,
            corner_radius: 0.0,
            flute_length: 30.0,
            shoulder_length: 0.0,
            overall_length: 100.0,
            shank_diameter: 20.0,
            tip_angle: 0.0,
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 4,
        holder: Holder { stick_out: 50.0, segments: vec![] },
        length_offset: 4,
        diameter_offset: 4,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 200.0, chip_load: 0.05, plunge_factor: 0.5, ramp_factor: 0.5 }],
        overrides: Overrides::default(),
    }
}

const RPM: f64 = 200_000.0 / (20.0 * PI);

/// The vise setup on a 60 x 40 x 20 block with `on_top` mm of stock over it. With 1 mm: the stock
/// is X -31 to 31, Y -21 to 21 in the WCS, its top Z 0, the model's top Z -1; clearance 25, retract 5.
fn placed(on_top: f64) -> Placement {
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

fn faced(op: &FaceOp, tool: &Tool) -> Result<Faced> {
    op.toolpath(None, tool, &placed(1.0), Some("aluminium"))
}

/// The cutting passes: every feed move along X at a level, as (from x, to x, y, z).
fn cuts(path: &Toolpath) -> Vec<(f64, f64, f64, f64)> {
    let mut at = path.start;
    let mut out = Vec::new();
    for m in &path.moves {
        if let Move::Linear { to, .. } = m
            && to.x != at.x
        {
            out.push((at.x, to.x, to.y, to.z));
        }
        at = m.end(at);
    }
    out
}

#[test]
fn facing_takes_the_stock_top_down_to_the_part_in_zig_zag_passes() {
    // 1 mm to take off, in one level at Z -1. Stepover 14 (70% of 20): the first pass at
    // Y 21 + 10 - 14 = 17 and the last at -17, 34 apart, so three gaps of 34/3. Each pass runs
    // from X -43 to 43: the stock's 31, the tool's radius 10 and 2 clear.
    let d = faced(&FaceOp::default(), &mill20()).unwrap();
    let (y1, y2) = (17.0 - 34.0 * 1.0 / 3.0, 17.0 - 34.0 * 2.0 / 3.0);
    assert_eq!(d.levels, vec![-1.0]);
    assert_eq!(d.passes, vec![17.0, y1, y2, -17.0]);
    assert!(close(d.rpm, RPM) && close(d.feed, 0.2 * RPM) && close(d.plunge_feed, 0.1 * RPM), "{} {} {}", d.rpm, d.feed, d.plunge_feed);
    let (cut, plunge) = (d.feed, d.plunge_feed);
    assert_eq!(d.toolpath.start, p(-43.0, 17.0, 25.0));
    assert_eq!(
        d.toolpath.moves,
        vec![
            Move::ToolChange { tool: 4 },
            Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(-43.0, 17.0, 25.0) },
            Move::Rapid { to: p(-43.0, 17.0, 5.0) },
            Move::Linear { to: p(-43.0, 17.0, -1.0), feed: plunge },
            Move::Linear { to: p(43.0, 17.0, -1.0), feed: cut },
            Move::Linear { to: p(43.0, y1, -1.0), feed: cut },
            Move::Linear { to: p(-43.0, y1, -1.0), feed: cut },
            Move::Linear { to: p(-43.0, y2, -1.0), feed: cut },
            Move::Linear { to: p(43.0, y2, -1.0), feed: cut },
            Move::Linear { to: p(43.0, -17.0, -1.0), feed: cut },
            Move::Linear { to: p(-43.0, -17.0, -1.0), feed: cut },
            Move::Rapid { to: p(-43.0, -17.0, 5.0) },
            Move::Rapid { to: p(-43.0, -17.0, 25.0) },
            Move::Coolant { mode: Coolant::Off },
            Move::SpindleStop,
        ]
    );
    // Measured: a 6 mm plunge, four passes of 86 and three steps of 34/3 cut 384 mm; rapids are
    // 20 down, then 6 and 20 back up.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 384.0) && close(s.rapid_length, 46.0), "{} {}", s.feed_length, s.rapid_length);
    // The whole top is covered: the passes reach past both Y edges, are never more than the
    // stepover apart, and the first and last bite exactly the stepover into the stock.
    assert!(d.passes[0] + 10.0 >= 21.0 && d.passes[3] - 10.0 <= -21.0);
    assert!(d.passes.windows(2).all(|w| w[0] - w[1] <= d.stepover + 1e-9));
    assert!(close(21.0 - (d.passes[0] - 10.0), 14.0) && close((d.passes[3] + 10.0) + 21.0, 14.0));
}

#[test]
fn deeper_facing_goes_down_in_equal_levels() {
    // 3 mm in steps of at most 1.2: three levels of 1.
    let d = faced(&FaceOp { bottom: Some(-3.0), stepdown: Some(1.2), ..FaceOp::default() }, &mill20()).unwrap();
    assert_eq!(d.levels, vec![-1.0, -2.0, -3.0]);
    // Each level plunges beside the stock from the retract height (6, 7 and 8 mm) and cuts 378.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 21.0 + 3.0 * 378.0), "{}", s.feed_length);
    let zs: Vec<f64> = cuts(&d.toolpath).iter().map(|c| c.3).collect();
    assert_eq!(zs, vec![-1.0; 4].into_iter().chain(vec![-2.0; 4]).chain(vec![-3.0; 4]).collect::<Vec<_>>());
    // A stepdown that divides the depth is used as it is.
    let d = faced(&FaceOp { bottom: Some(-3.0), stepdown: Some(1.5), ..FaceOp::default() }, &mill20()).unwrap();
    assert_eq!(d.levels, vec![-1.5, -3.0]);
}

#[test]
fn one_way_passes_all_cut_the_same_way() {
    let d = faced(&FaceOp { pattern: FacePattern::OneWay, ..FaceOp::default() }, &mill20()).unwrap();
    let passes = cuts(&d.toolpath);
    assert_eq!(passes.len(), 4);
    assert!(passes.iter().all(|&(from, to, _, z)| from == -43.0 && to == 43.0 && z == -1.0), "{passes:?}");
    // Four passes of 86 and a 6 mm plunge before each; between passes the tool is at the retract height.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 4.0 * 86.0 + 4.0 * 6.0), "{}", s.feed_length);
    let mut at = d.toolpath.start;
    for m in &d.toolpath.moves {
        if let Move::Rapid { to } = m
            && to.x != at.x
        {
            assert_eq!((at.z, to.z), (5.0, 5.0), "a move back across the stock below the retract height");
        }
        at = m.end(at);
    }
}

#[test]
fn stepover_and_clearance_given_are_used_and_a_wide_tool_takes_one_pass() {
    // Stepover 10, clear 5: first pass at 21, last at -21, 42 apart in five gaps of 8.4; X from -46.
    let d = faced(&FaceOp { stepover: Some(10.0), clear: Some(5.0), ..FaceOp::default() }, &mill20()).unwrap();
    assert_eq!(d.passes.len(), 6);
    assert!(d.passes.windows(2).all(|w| close(w[0] - w[1], 8.4)), "{:?}", d.passes);
    assert_eq!((d.passes[0], d.passes[5]), (21.0, -21.0));
    assert!(cuts(&d.toolpath).iter().all(|c| c.0.abs() == 46.0 && c.1.abs() == 46.0));
    // A 100 mm face mill is wider than the 42 mm stock: one pass down the middle, X -83 to 83.
    let mut face = mill20();
    face.kind = ToolKind::FaceMill;
    face.geometry.diameter = 100.0;
    face.geometry.shank_diameter = 32.0;
    face.geometry.flute_length = 10.0;
    face.flutes = 6;
    let d = faced(&FaceOp::default(), &face).unwrap();
    assert_eq!(d.passes, vec![0.0]);
    assert_eq!(cuts(&d.toolpath), vec![(-83.0, 83.0, 0.0, -1.0)]);
    // Speed and feed typed in: used as they are, plunging at half the feed.
    let d = faced(&FaceOp { rpm: Some(2000.0), feed: Some(500.0), ..FaceOp::default() }, &mill20()).unwrap();
    assert_eq!((d.rpm, d.feed, d.plunge_feed), (2000.0, 500.0, 250.0));
}

#[test]
fn what_cannot_be_faced_is_refused_with_the_reason() {
    let drill = {
        let mut t = mill20();
        t.kind = ToolKind::Drill;
        t.geometry.tip_angle = 2.0;
        t
    };
    let cases: Vec<(FaceOp, Tool, &str)> = vec![
        (FaceOp::default(), drill, "does not face"),
        (FaceOp { stepover: Some(25.0), ..FaceOp::default() }, mill20(), "at most the tool's diameter"),
        (FaceOp { stepover: Some(0.0), ..FaceOp::default() }, mill20(), "stepover"),
        (FaceOp { stepover: Some(0.001), ..FaceOp::default() }, mill20(), "more than 10000 passes"),
        (FaceOp { clear: Some(-1.0), ..FaceOp::default() }, mill20(), "clearance past the stock"),
        (FaceOp { bottom: Some(-40.0), ..FaceOp::default() }, mill20(), "past tool 4's 30 mm of flutes"),
        (FaceOp { bottom: Some(-3.0), stepdown: Some(0.001), ..FaceOp::default() }, mill20(), "more than 1000 levels"),
        (FaceOp { stepdown: Some(0.0), ..FaceOp::default() }, mill20(), "stepdown"),
        (FaceOp { top: Some(10.0), ..FaceOp::default() }, mill20(), "retract height"),
        (FaceOp { top: Some(-2.0), bottom: Some(-1.0), ..FaceOp::default() }, mill20(), "must be below the top"),
        (FaceOp { bottom: Some(f64::NAN), ..FaceOp::default() }, mill20(), "finite"),
        (FaceOp { rpm: Some(0.0), ..FaceOp::default() }, mill20(), "rpm"),
        (FaceOp { feed: Some(-1.0), ..FaceOp::default() }, mill20(), "feed"),
    ];
    for (op, tool, expect) in cases {
        let e = faced(&op, &tool).unwrap_err().to_string();
        assert!(e.contains(expect), "{op:?}: {e:?} does not mention {expect:?}");
    }
    // A deep face is fine once it has a stepdown its flutes can take.
    assert_eq!(faced(&FaceOp { bottom: Some(-40.0), stepdown: Some(10.0), ..FaceOp::default() }, &mill20()).unwrap().levels.len(), 4);
    // No stock on top of the part: there is nothing to face, said so rather than an empty program.
    let e = FaceOp::default().toolpath(None, &mill20(), &placed(0.0), Some("aluminium")).unwrap_err().to_string();
    assert!(e.contains("nothing to face"), "{e}");
    // No material and nothing typed in: the error names both ways out.
    let e = FaceOp::default().toolpath(None, &mill20(), &placed(1.0), None).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
}

#[test]
fn a_facing_operation_is_plain_json_and_every_field_has_a_default() {
    let op: FaceOp = serde_json::from_str("{}").unwrap();
    assert_eq!(op, FaceOp::default());
    let op: FaceOp = serde_json::from_str(r#"{"pattern": "one_way", "stepover": 12, "bottom": -2}"#).unwrap();
    assert_eq!((op.pattern, op.stepover, op.bottom), (FacePattern::OneWay, Some(12.0), Some(-2.0)));
    assert!(serde_json::from_str::<FaceOp>(r#"{"angle": 45}"#).is_err());
    assert_eq!(serde_json::from_value::<FaceOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
