use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};
use crate::toolpath::Drawn;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
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

/// The vise setup on a model filling `model` (model coordinates), with `side` mm of stock round
/// it and 1 mm on top; the WCS at the middle of the stock's top, clearance 25, retract 5.
fn placed_on(model: Bounds, side: f64) -> Placement {
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
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: side, x_max: side, y_min: side, y_max: side, z_min: 0.0, z_max: 1.0 } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    };
    setup.place(model).unwrap()
}

/// The 60 x 40 x 20 block: in the WCS the model is X -30 to 30, Y -20 to 20, Z -21 to -1, and
/// the stock X -31 to 31, Y -21 to 21, Z -21 to 0.
fn block() -> Placement {
    placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }, 1.0)
}

/// A closed loop of straight sides through `points`, in order.
fn polygon(points: &[(f64, f64)]) -> Loop2 {
    let n = points.len();
    Loop2 { segs: (0..n).map(|i| Seg2::Line { a: q(points[i].0, points[i].1), b: q(points[(i + 1) % n].0, points[(i + 1) % n].1) }).collect() }
}

const BLOCK: [(f64, f64); 4] = [(-30.0, -20.0), (30.0, -20.0), (30.0, 20.0), (-30.0, 20.0)];

fn contoured(op: &ContourOp) -> Result<Contoured> {
    op.toolpath(None, &mill20(), &block(), Some("aluminium"), &[polygon(&BLOCK)])
}

/// Whether two toolpaths' moves are the same to a nanometre (the offsetter's last bits are its own).
fn same_moves(got: &[Move], want: &[Move]) -> bool {
    let near = |a: Vec3, b: Vec3| (a.x - b.x).abs() <= 1e-9 && (a.y - b.y).abs() <= 1e-9 && (a.z - b.z).abs() <= 1e-9;
    got.len() == want.len()
        && got.iter().zip(want).all(|(g, w)| match (*g, *w) {
            (Move::Rapid { to: a }, Move::Rapid { to: b }) => near(a, b),
            (Move::Linear { to: a, feed: f }, Move::Linear { to: b, feed: h }) => near(a, b) && close(f, h),
            (Move::Arc { to: a, center: c, plane: m, direction: d, feed: f }, Move::Arc { to: b, center: e, plane: n, direction: o, feed: h }) => {
                near(a, b) && near(c, e) && m == n && d == o && close(f, h)
            }
            (a, b) => a == b,
        })
}

/// One level of the block's contour, clockwise from the front: down beside the stock at Y -33,
/// in to Y -30, round with the corners rolled about the block's own corners, and out again.
fn block_level(z: f64, cut: f64, plunge: f64) -> Vec<Move> {
    let arc = |x: f64, y: f64, cx: f64, cy: f64| Move::Arc {
        to: p(x, y, z),
        center: p(cx, cy, z),
        plane: Plane::Xy,
        direction: ArcDirection::Clockwise,
        feed: cut,
    };
    let line = |x: f64, y: f64| Move::Linear { to: p(x, y, z), feed: cut };
    vec![
        Move::Linear { to: p(0.0, -33.0, z), feed: plunge },
        line(0.0, -30.0),
        line(-30.0, -30.0),
        arc(-40.0, -20.0, -30.0, -20.0),
        line(-40.0, 20.0),
        arc(-30.0, 30.0, -30.0, 20.0),
        line(30.0, 30.0),
        arc(40.0, 20.0, 30.0, 20.0),
        line(40.0, -20.0),
        arc(30.0, -30.0, 30.0, -20.0),
        line(0.0, -30.0),
        line(0.0, -33.0),
        Move::Rapid { to: p(0.0, -33.0, 5.0) },
    ]
}

#[test]
fn a_contour_goes_clockwise_round_a_block_in_equal_levels() {
    // The stock's top is Z 0 and the block's bottom Z -21: 21 mm in steps of at most 10 (half the
    // 20 mm tool) is three levels of 7. The tool's centre is its radius, 10, outside the block:
    // sides at X ±40 and Y ±30, each corner an arc of 10 about the block's corner. The longest
    // sides are the two along X; the front one (Y -30) is where it comes in, from Y -33: the
    // stock's edge at -21, the tool's radius and 2 clear.
    let d = contoured(&ContourOp::default()).unwrap();
    assert_eq!((d.top, d.levels.clone()), (0.0, vec![-7.0, -14.0, -21.0]));
    assert_eq!((d.stepdown, d.offset), (10.0, 10.0));
    assert_eq!(d.entries, vec![[0.0, -33.0]]);
    assert!(close(d.round, 200.0 + 20.0 * PI), "{}", d.round);
    // A cut as wide as the tool: the chip is not thinned, so the feed is 0.2 mm a revolution.
    assert!(close(d.rpm, RPM) && close(d.feed, 0.2 * RPM) && close(d.plunge_feed, 0.1 * RPM), "{} {} {}", d.rpm, d.feed, d.plunge_feed);
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    assert_eq!(d.toolpath.start, p(0.0, -33.0, 25.0));
    let mut want = vec![
        Move::ToolChange { tool: 4 },
        Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
        Move::Coolant { mode: Coolant::Flood },
        Move::Rapid { to: p(0.0, -33.0, 25.0) },
        Move::Rapid { to: p(0.0, -33.0, 5.0) },
    ];
    for z in [-7.0, -14.0, -21.0] {
        want.extend(block_level(z, d.feed, d.plunge_feed));
    }
    want.extend([Move::Rapid { to: p(0.0, -33.0, 25.0) }, Move::Coolant { mode: Coolant::Off }, Move::SpindleStop]);
    assert!(same_moves(&d.toolpath.moves, &want), "{:#?}", d.toolpath.moves);
    // Measured: plunges of 12, 19 and 26 from the retract height, and at each level 3 in, the
    // round of 200 + 20 pi, and 3 out. Rapids: 20 down to the retract height, 12, 19 and 26 back
    // up to it, and 20 up at the end.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 57.0 + 3.0 * (206.0 + 20.0 * PI)), "{}", s.feed_length);
    assert!(close(s.rapid_length, 97.0), "{}", s.rapid_length);
    assert_eq!((s.min, s.max), (p(-40.0, -33.0, -21.0), p(40.0, 30.0, 25.0)));
}

#[test]
fn conventional_goes_the_other_way_and_stock_to_leave_moves_the_path_out() {
    let d = contoured(&ContourOp { direction: CutDirection::Conventional, stepdown: Some(21.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.levels, vec![-21.0]);
    let arc = |x: f64, y: f64, cx: f64, cy: f64| Move::Arc {
        to: p(x, y, -21.0),
        center: p(cx, cy, -21.0),
        plane: Plane::Xy,
        direction: ArcDirection::CounterClockwise,
        feed: d.feed,
    };
    let line = |x: f64, y: f64| Move::Linear { to: p(x, y, -21.0), feed: d.feed };
    let want = vec![
        Move::Linear { to: p(0.0, -33.0, -21.0), feed: d.plunge_feed },
        line(0.0, -30.0),
        line(30.0, -30.0),
        arc(40.0, -20.0, 30.0, -20.0),
        line(40.0, 20.0),
        arc(30.0, 30.0, 30.0, 20.0),
        line(-30.0, 30.0),
        arc(-40.0, 20.0, -30.0, 20.0),
        line(-40.0, -20.0),
        arc(-30.0, -30.0, -30.0, -20.0),
        line(0.0, -30.0),
        line(0.0, -33.0),
        Move::Rapid { to: p(0.0, -33.0, 5.0) },
    ];
    assert!(same_moves(&d.toolpath.moves[5..18], &want), "{:#?}", d.toolpath.moves);
    // Half a millimetre left on the wall: the centre is 10.5 out, so it comes in to Y -30.5 from
    // the same place beside the stock, and the round is 200 + 21 pi.
    let d = contoured(&ContourOp { leave: Some(0.5), ..ContourOp::default() }).unwrap();
    assert_eq!((d.offset, d.entries.clone()), (10.5, vec![[0.0, -33.0]]));
    assert!(close(d.round, 200.0 + 21.0 * PI), "{}", d.round);
    assert!(same_moves(
        &d.toolpath.moves[5..7],
        &[Move::Linear { to: p(0.0, -33.0, -7.0), feed: d.plunge_feed }, Move::Linear { to: p(0.0, -30.5, -7.0), feed: d.feed }]
    ));
}

#[test]
fn heights_and_the_stepdown_given_set_the_levels() {
    // From the faced top (Z -1) down 10: one level, the default stepdown being 10.
    let d = contoured(&ContourOp { top: Some(-1.0), bottom: Some(-11.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.levels, vec![-11.0]);
    // 21 mm in steps of at most 5 is five levels of 4.2.
    let d = contoured(&ContourOp { stepdown: Some(5.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.levels.len(), 5);
    assert!(d.levels.iter().enumerate().all(|(k, &z)| close(z, -4.2 * (k + 1) as f64)), "{:?}", d.levels);
    assert_eq!(d.levels[4], -21.0);
    // A stepdown that divides the depth is used as it is; past the bottom of the part is allowed
    // (through into a spoil board), down to what the flutes reach.
    let d = contoured(&ContourOp { bottom: Some(-22.0), stepdown: Some(11.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.levels, vec![-11.0, -22.0]);
    // Speed and feed typed in: used as they are, plunging at half the feed.
    let d = contoured(&ContourOp { rpm: Some(2000.0), feed: Some(500.0), ..ContourOp::default() }).unwrap();
    assert_eq!((d.rpm, d.feed, d.plunge_feed), (2000.0, 500.0, 250.0));
    // More clearance moves where it goes down further out.
    let d = contoured(&ContourOp { clear: Some(5.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.entries, vec![[0.0, -36.0]]);
}

/// The distance from `at` to the closed polygon through `points`.
fn distance(at: Vec2, points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (q(points[i].0, points[i].1), q(points[(i + 1) % n].0, points[(i + 1) % n].1));
            let (side, to) = (q(b.x - a.x, b.y - a.y), q(at.x - a.x, at.y - a.y));
            let t = ((to.x * side.x + to.y * side.y) / (side.x * side.x + side.y * side.y)).clamp(0.0, 1.0);
            (to.x - t * side.x).hypot(to.y - t * side.y)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Every point of the drawn cutting moves, a quarter of a millimetre apart at most.
fn cutting_points(path: &Toolpath) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (kind, a, b) in path.drawn(1e-5).unwrap() {
        if kind != Drawn::Cut {
            continue;
        }
        let steps = ((a.x - b.x).hypot(a.y - b.y) / 0.25).ceil().max(1.0) as usize;
        out.extend((0..=steps).map(|k| {
            let t = k as f64 / steps as f64;
            p(a.x + t * (b.x - a.x), a.y + t * (b.y - a.y), a.z + t * (b.z - a.z))
        }));
    }
    out
}

#[test]
fn the_tool_never_comes_nearer_the_part_than_its_radius_and_reaches_every_wall() {
    // An L: 60 x 60 with the upper right 40 x 40 taken out. Its inside corner at (20, 20) leaves
    // a sharp corner in the path at (30, 30); its five outside corners are rolled round.
    let l = [(0.0, 0.0), (60.0, 0.0), (60.0, 20.0), (20.0, 20.0), (20.0, 60.0), (0.0, 60.0)];
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 60.0, 20.0) }, 1.0);
    // The WCS is at the middle of the stock: the L's corners move by (-30, -30).
    let outline: Vec<(f64, f64)> = l.iter().map(|&(x, y)| (x - 30.0, y - 30.0)).collect();
    let d = ContourOp::default().toolpath(None, &mill20(), &placement, Some("aluminium"), &[polygon(&outline)]).unwrap();
    // The round: sides of 60, 20, 30, 30, 20 and 60 (the two at the inside corner end where they
    // meet, 10 short of the part's corner each) and five quarter turns of radius 10.
    assert!(close(d.round, 220.0 + 25.0 * PI), "{}", d.round);
    let points = cutting_points(&d.toolpath);
    assert!(points.len() > 3000);
    let nearest = points.iter().map(|at| distance(q(at.x, at.y), &outline)).fold(f64::INFINITY, f64::min);
    assert!((nearest - 10.0).abs() <= 1e-4, "the tool's centre comes within {nearest} mm of the part, not 10");
    // Every wall is reached: each point of the outline has the tool's centre 10 from it somewhere
    // along the round (11 and a bit at the inside corner, which a round tool can't reach into).
    let n = outline.len();
    for i in 0..n {
        let (a, b) = (outline[i], outline[(i + 1) % n]);
        for k in 0..=40 {
            let t = f64::from(k) / 40.0;
            let on = q(a.0 + t * (b.0 - a.0), a.1 + t * (b.1 - a.1));
            let reach = points.iter().map(|at| (at.x - on.x).hypot(at.y - on.y)).fold(f64::INFINITY, f64::min);
            let corner = (on.x + 10.0).hypot(on.y + 10.0);
            let most = if corner < 10.0 { 10.0 + (2.0_f64.sqrt() - 1.0) * 10.0 } else { 10.0 };
            assert!(reach <= most + 0.02, "the wall at ({}, {}) is {reach} mm from the tool's centre", on.x, on.y);
        }
    }
    // It comes in at the front (the longest side, Y -40 in the WCS, 60 long as the left one is, and nearer
    // the front), from beside the stock.
    assert_eq!(d.entries, vec![[0.0, -43.0]]);
}

#[test]
fn a_round_part_is_cut_in_arcs_about_its_centre() {
    // A disc of radius 20 as one full arc; the tool's centre runs at 30.
    let disc = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(40.0, 40.0, 10.0) }, 1.0);
    let d = ContourOp::default().toolpath(None, &mill20(), &placement, Some("aluminium"), &[disc]).unwrap();
    assert!(close(d.round, 60.0 * PI), "{}", d.round);
    assert_eq!(d.levels, vec![-5.5, -11.0]);
    // It comes in straight out from the circle, from beside the stock (21 + 10 + 2 from the middle).
    let [x, y] = d.entries[0];
    assert!(close(x.abs().max(y.abs()), 33.0), "{x} {y}");
    let mut arcs = 0;
    let mut at = d.toolpath.start;
    for m in &d.toolpath.moves {
        if let Move::Arc { to, center, direction, .. } = *m {
            arcs += 1;
            assert_eq!(direction, ArcDirection::Clockwise);
            assert!(center.x.abs() <= 1e-9 && center.y.abs() <= 1e-9, "{center:?}");
            assert!(close(to.x.hypot(to.y), 30.0) && close(at.x.hypot(at.y), 30.0), "{at:?} {to:?}");
        }
        at = m.end(at);
    }
    assert!(arcs >= 4, "{arcs}");
    let s = d.toolpath.stats().unwrap();
    // Two levels: plunges of 10.5 and 16, and at each 3 in, the circle, and 3 out.
    assert!(close(s.feed_length, 26.5 + 2.0 * (6.0 + 60.0 * PI)), "{}", s.feed_length);
}

#[test]
fn each_part_is_gone_round_and_the_way_in_never_crosses_another() {
    // Two bars, 10 x 100, 70 apart. Each one's longest sides are its two long ones; the left
    // side is tried first. For the right bar that way in runs across the left bar, so it comes
    // in from its right.
    let left = [(-45.0, -50.0), (-35.0, -50.0), (-35.0, 50.0), (-45.0, 50.0)];
    let right = [(35.0, -50.0), (45.0, -50.0), (45.0, 50.0), (35.0, 50.0)];
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(90.0, 100.0, 10.0) }, 1.0);
    let d = ContourOp::default().toolpath(None, &mill20(), &placement, Some("aluminium"), &[polygon(&left), polygon(&right)]).unwrap();
    let mut entries = d.entries.clone();
    entries.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(entries, vec![[-58.0, 0.0], [58.0, 0.0]]);
    assert!(close(d.round, 2.0 * (220.0 + 20.0 * PI)), "{}", d.round);
    // Between the two the tool is up at the retract height.
    let mut at = d.toolpath.start;
    for m in &d.toolpath.moves {
        if let Move::Rapid { to } = m
            && (to.x != at.x || to.y != at.y)
        {
            assert!(at.z >= 5.0 && to.z >= 5.0, "a move across the part below the retract height: {at:?} to {to:?}");
        }
        at = m.end(at);
    }
    // No point of the cut comes nearer either bar than the tool's radius.
    let nearest = cutting_points(&d.toolpath)
        .iter()
        .map(|at| distance(q(at.x, at.y), &left).min(distance(q(at.x, at.y), &right)))
        .fold(f64::INFINITY, f64::min);
    assert!((nearest - 10.0).abs() <= 1e-4, "{nearest}");
}

#[test]
fn a_way_is_clear_only_when_it_meets_no_side_or_arc() {
    let region = |segs: Vec<Seg2>| vec![Region2 { outer: Loop2 { segs }, holes: Vec::new() }];
    let (from, way) = (q(0.0, 0.0), q(1.0, 0.0));
    // A side across the way at X 5.
    let wall = region(vec![Seg2::Line { a: q(5.0, -1.0), b: q(5.0, 1.0) }]);
    assert!(!way_is_clear(from, way, 10.0, &wall));
    assert!(way_is_clear(from, way, 4.0, &wall), "the way stops short of it");
    assert!(way_is_clear(from, way, 10.0, &region(vec![Seg2::Line { a: q(5.0, 1.0), b: q(5.0, 2.0) }])), "the side is beside the way");
    // A side lying along the way is in it; one alongside, off it, is not.
    assert!(!way_is_clear(from, way, 10.0, &region(vec![Seg2::Line { a: q(3.0, 0.0), b: q(4.0, 0.0) }])));
    assert!(way_is_clear(from, way, 10.0, &region(vec![Seg2::Line { a: q(3.0, 0.5), b: q(4.0, 0.5) }])));
    // The side the way starts on does not count.
    assert!(way_is_clear(from, way, 10.0, &region(vec![Seg2::Line { a: q(0.0, -1.0), b: q(0.0, 1.0) }])));
    // The right half of a circle about (5, 0) is met at X 6 and not at X 4; the left half at X 4.
    let half = |start: f64| region(vec![Seg2::Arc { center: q(5.0, 0.0), radius: 1.0, start, sweep: PI }]);
    assert!(way_is_clear(from, way, 5.5, &half(-0.5 * PI)));
    assert!(!way_is_clear(from, way, 6.5, &half(-0.5 * PI)));
    assert!(!way_is_clear(from, way, 5.5, &half(0.5 * PI)));
    // The same halves turned the other way.
    let back = |start: f64| region(vec![Seg2::Arc { center: q(5.0, 0.0), radius: 1.0, start, sweep: -PI }]);
    assert!(way_is_clear(from, way, 5.5, &back(0.5 * PI)));
    assert!(!way_is_clear(from, way, 5.5, &back(-0.5 * PI)));
    // A circle off to the side is missed.
    assert!(way_is_clear(from, way, 10.0, &region(vec![Seg2::Arc { center: q(5.0, 3.0), radius: 1.0, start: 0.0, sweep: 2.0 * PI }])));
}

#[test]
fn what_is_worth_knowing_is_noted() {
    // A square ring, 100 across with a 60 mm opening and a 10 mm slit through its right side.
    // The 20 mm tool can't pass the slit, so the opening is left and said to be.
    let ring = [
        (-50.0, -50.0),
        (50.0, -50.0),
        (50.0, -5.0),
        (30.0, -5.0),
        (30.0, -30.0),
        (-30.0, -30.0),
        (-30.0, 30.0),
        (30.0, 30.0),
        (30.0, 5.0),
        (50.0, 5.0),
        (50.0, 50.0),
        (-50.0, 50.0),
    ];
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(100.0, 100.0, 10.0) }, 1.0);
    let d = ContourOp::default().toolpath(None, &mill20(), &placement, Some("aluminium"), &[polygon(&ring)]).unwrap();
    assert_eq!(d.notes.len(), 1);
    assert!(d.notes[0].contains("1 enclosed place(s)"), "{:?}", d.notes);
    // The round is the outside only: four sides of 100 and a full turn of radius 10, but for the
    // slit's mouth. There the tool's centre is 10 from each of the mouth's corners, (50, ±5): in
    // place of 10 mm of straight side it turns 30 degrees about one corner and 30 about the
    // other, meeting at X 50 + sqrt(75).
    assert!(close(d.round, 390.0 + 20.0 * PI + 2.0 * 10.0 * PI / 6.0), "{}", d.round);
    // A block in stock 100 mm bigger all round: the way in is 100 mm of full-width cutting, at every level.
    let wide = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }, 100.0);
    let d = ContourOp::default().toolpath(None, &mill20(), &wide, Some("aluminium"), &[polygon(&BLOCK)]).unwrap();
    assert_eq!(d.entries, vec![[0.0, -132.0]]);
    assert!(d.notes.iter().any(|n| n.contains("102.0 mm of cutting as wide as the tool")), "{:?}", d.notes);
}

#[test]
fn a_slot_as_wide_as_the_tool_is_cut_or_passed_by_and_never_cut_into() {
    // A U, 60 x 60, with a slot 40 deep in its top, about the 20 mm tool's width. A hair wider
    // and the tool goes down one side and back up the other; a hair narrower and the round
    // passes its mouth. Exactly as wide is a question of the last bit of a number: the path may
    // run down the middle or pass by (when the offsetter can't verify the first, the outline is
    // taken a micron bigger), and either way the tool keeps its radius from every wall.
    let u: [(f64, f64); 8] = [(-30.0, -30.0), (30.0, -30.0), (30.0, 30.0), (10.0, 30.0), (10.0, -10.0), (-10.0, -10.0), (-10.0, 30.0), (-30.0, 30.0)];
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 60.0, 20.0) }, 1.0);
    for width in [20.0, 19.999, 20.001, 12.0] {
        let slot: Vec<(f64, f64)> = u.iter().map(|&(x, y)| (if x.abs() == 10.0 { 0.5 * width * x.signum() } else { x }, y)).collect();
        let d = ContourOp::default()
            .toolpath(None, &mill20(), &placement, Some("aluminium"), &[polygon(&slot)])
            .unwrap_or_else(|e| panic!("a slot {width} wide: {e}"));
        let nearest = cutting_points(&d.toolpath).iter().map(|at| distance(q(at.x, at.y), &slot)).fold(f64::INFINITY, f64::min);
        assert!((10.0 - 1e-4..=10.0 + 2e-3).contains(&nearest), "a slot {width} wide: the tool's centre comes within {nearest} mm of the part");
        // Into a slot a hair wider than the tool it does go, down one side and back up the other.
        let deep = cutting_points(&d.toolpath).iter().filter(|at| at.x.abs() < 5.0 && at.y < 20.0 && at.y > -20.0).count();
        if width != 20.0 {
            assert_eq!(deep > 0, width > 20.0, "a slot {width} wide: {deep} points of the path are inside it");
        }
    }
}

#[test]
fn what_cannot_be_contoured_is_refused_with_the_reason() {
    let drill = {
        let mut t = mill20();
        t.kind = ToolKind::Drill;
        t.geometry.tip_angle = 2.0;
        t
    };
    let short = {
        let mut t = mill20();
        t.geometry.flute_length = 5.0;
        t.geometry.shoulder_length = 30.0;
        t
    };
    let cases: Vec<(ContourOp, Tool, &str)> = vec![
        (ContourOp::default(), drill, "does not cut a contour"),
        (ContourOp { bottom: Some(-31.0), ..ContourOp::default() }, mill20(), "more than the 30 mm tool 4 reaches down a wall"),
        (ContourOp { stepdown: Some(8.0), ..ContourOp::default() }, short.clone(), "past tool 4's 5 mm of flutes"),
        (ContourOp { stepdown: Some(0.001), ..ContourOp::default() }, mill20(), "more than 1000 levels"),
        (ContourOp { stepdown: Some(0.0), ..ContourOp::default() }, mill20(), "stepdown"),
        (ContourOp { leave: Some(-0.1), ..ContourOp::default() }, mill20(), "stock to leave"),
        (ContourOp { clear: Some(-1.0), ..ContourOp::default() }, mill20(), "clearance past the stock"),
        (ContourOp { top: Some(10.0), ..ContourOp::default() }, mill20(), "retract height"),
        (ContourOp { top: Some(-2.0), bottom: Some(-1.0), ..ContourOp::default() }, mill20(), "must be below the top"),
        (ContourOp { bottom: Some(1.0), ..ContourOp::default() }, mill20(), "nothing to cut"),
        (ContourOp { bottom: Some(f64::NAN), ..ContourOp::default() }, mill20(), "finite"),
        (ContourOp { rpm: Some(0.0), ..ContourOp::default() }, mill20(), "rpm"),
        (ContourOp { feed: Some(-1.0), ..ContourOp::default() }, mill20(), "feed"),
    ];
    for (op, tool, expect) in cases {
        let e = op.toolpath(None, &tool, &block(), Some("aluminium"), &[polygon(&BLOCK)]).unwrap_err().to_string();
        assert!(e.contains(expect), "{op:?}: {e:?} does not mention {expect:?}");
    }
    // The same tool with its 30 mm reach is fine in steps its 5 mm of flutes can take.
    assert_eq!(
        ContourOp { stepdown: Some(5.0), ..ContourOp::default() }
            .toolpath(None, &short, &block(), Some("aluminium"), &[polygon(&BLOCK)])
            .unwrap()
            .levels
            .len(),
        5
    );
    // No outline, an open one, one with a curve that is not a line or an arc.
    let with = |outline: &[Loop2]| ContourOp::default().toolpath(None, &mill20(), &block(), Some("aluminium"), outline).unwrap_err().to_string();
    assert!(with(&[]).contains("no outline"));
    let open = Loop2 { segs: polygon(&BLOCK).segs.into_iter().take(3).collect() };
    assert!(with(&[open]).contains("not closed"));
    let curved = Loop2 {
        segs: vec![
            Seg2::Line { a: q(-30.0, -20.0), b: q(30.0, -20.0) },
            Seg2::Cubic { p0: q(30.0, -20.0), p1: q(40.0, 0.0), p2: q(0.0, 40.0), p3: q(-30.0, -20.0) },
        ],
    };
    assert!(with(&[curved]).contains("only lines and arcs"));
    // No material and nothing typed in: the error names both ways out.
    let e = ContourOp::default().toolpath(None, &mill20(), &block(), None, &[polygon(&BLOCK)]).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
}

#[test]
fn hostile_numbers_never_panic() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, 5e-324, f64::MAX, f64::MIN];
    let tool = mill20();
    let outline = [polygon(&BLOCK)];
    for &v in &bad {
        for field in 0..7 {
            let mut op = ContourOp::default();
            match field {
                0 => op.top = Some(v),
                1 => op.bottom = Some(v),
                2 => op.stepdown = Some(v),
                3 => op.leave = Some(v),
                4 => op.clear = Some(v),
                5 => op.rpm = Some(v),
                _ => op.feed = Some(v),
            }
            let _ = op.toolpath(None, &tool, &block(), Some("aluminium"), &outline);
        }
        // An outline of hostile numbers: a point, a spike, a huge or broken loop.
        let loops = [
            polygon(&[(v, v), (v, v), (v, v)]),
            polygon(&[(0.0, 0.0), (v, 0.0), (0.0, v)]),
            Loop2 { segs: vec![Seg2::Arc { center: q(v, 0.0), radius: v, start: v, sweep: v }] },
            Loop2 { segs: vec![] },
        ];
        for l in loops {
            let _ = ContourOp::default().toolpath(None, &tool, &block(), Some("aluminium"), &[l]);
        }
        // A tool of hostile sizes, each alone and all together: never a path inside the part.
        for size in 0..4 {
            let mut odd = mill20();
            if size == 0 || size == 3 {
                odd.geometry.diameter = v;
            }
            if size == 1 || size == 3 {
                odd.geometry.flute_length = v;
            }
            if size == 2 || size == 3 {
                odd.geometry.shoulder_length = v;
            }
            if let Ok(done) = ContourOp::default().toolpath(None, &odd, &block(), Some("aluminium"), &outline) {
                assert!(done.offset > 0.0 && done.offset.is_finite(), "{v}: a path {} mm from the outline", done.offset);
            }
        }
    }
}

#[test]
fn a_contour_operation_is_plain_json_and_every_field_has_a_default() {
    let op: ContourOp = serde_json::from_str("{}").unwrap();
    assert_eq!(op, ContourOp::default());
    assert_eq!(op.direction, CutDirection::Climb);
    let op: ContourOp = serde_json::from_str(r#"{"direction": "conventional", "stepdown": 3, "bottom": -12, "leave": 0.2}"#).unwrap();
    assert_eq!((op.direction, op.stepdown, op.bottom, op.leave), (CutDirection::Conventional, Some(3.0), Some(-12.0), Some(0.2)));
    assert!(serde_json::from_str::<ContourOp>(r#"{"tabs": 4}"#).is_err());
    assert_eq!(serde_json::from_value::<ContourOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
