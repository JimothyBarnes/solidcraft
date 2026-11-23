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

/// A contour that goes straight in to the wall and straight back out: no arcs.
fn straight() -> ContourOp {
    ContourOp { lead: Some(0.0), ..ContourOp::default() }
}

/// Tool 6: the 20 mm mill as a 6 mm one with 3 flutes.
fn mill6() -> Tool {
    let mut tool = mill20();
    (tool.number, tool.name, tool.flutes) = (6, "6 mm flat end mill".into(), 3);
    (tool.geometry.diameter, tool.geometry.shank_diameter) = (6.0, 6.0);
    tool
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
    // stock's edge at -21, the tool's radius and 2 clear. Straight in and out, here.
    let d = contoured(&straight()).unwrap();
    assert_eq!((d.top, d.levels.clone()), (0.0, vec![-7.0, -14.0, -21.0]));
    assert_eq!((d.leads.clone(), d.tabs.len(), d.tab_top), (vec![0.0], 0, None));
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
    let d = contoured(&ContourOp { direction: CutDirection::Conventional, stepdown: Some(21.0), ..straight() }).unwrap();
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
    let d = contoured(&ContourOp { leave: Some(0.5), ..straight() }).unwrap();
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
    // More clearance moves where it goes down further out (2 to the side, where the arc on to
    // the wall begins).
    let d = contoured(&ContourOp { clear: Some(5.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.entries, vec![[2.0, -36.0]]);
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
    // the front), from beside the stock, 2 to the side of the middle for its arc on to the wall.
    assert_eq!((d.entries.clone(), d.leads.clone()), (vec![[2.0, -43.0]], vec![2.0]));
}

#[test]
fn a_round_part_is_cut_in_arcs_about_its_centre() {
    // A disc of radius 20 as one full arc; the tool's centre runs at 30.
    let disc = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(40.0, 40.0, 10.0) }, 1.0);
    let d = straight().toolpath(None, &mill20(), &placement, Some("aluminium"), &[disc]).unwrap();
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
    // Each goes down 2 before the middle of its side, where its arc on to the wall begins:
    // clockwise is up the left bar's left side and down the right bar's right side.
    let mut entries = d.entries.clone();
    entries.sort_by(|a, b| a[0].total_cmp(&b[0]));
    assert_eq!(entries, vec![[-58.0, -2.0], [58.0, 2.0]]);
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
    assert_eq!(d.entries, vec![[2.0, -132.0]]);
    assert!(d.notes.iter().any(|n| n.contains("100.0 mm of cutting as wide as the tool")), "{:?}", d.notes);
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
        (ContourOp { lead: Some(-1.0), ..ContourOp::default() }, mill20(), "radius of the arcs in and out"),
        (ContourOp { tabs: Some(Tabs { count: 0, width: None, height: None }), ..ContourOp::default() }, mill20(), "number of tabs"),
        (ContourOp { tabs: Some(Tabs { count: 2, width: Some(0.0), height: None }), ..ContourOp::default() }, mill20(), "width and height"),
        // The round is 200 + 20 pi, about 263 mm: six tabs of 6 with the 20 mm tool take 46 each.
        (ContourOp { tabs: Some(Tabs { count: 6, width: None, height: None }), ..ContourOp::default() }, mill20(), "6 tab(s) 6 mm wide do not fit"),
        (ContourOp { tabs: Some(Tabs { count: 2, width: None, height: Some(21.0) }), ..ContourOp::default() }, mill20(), "as high as the cut"),
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
        for field in 0..11 {
            let mut op = ContourOp::default();
            match field {
                0 => op.top = Some(v),
                1 => op.bottom = Some(v),
                2 => op.stepdown = Some(v),
                3 => op.leave = Some(v),
                4 => op.clear = Some(v),
                5 => op.rpm = Some(v),
                6 => op.feed = Some(v),
                7 => op.lead = Some(v),
                8 => op.tabs = Some(Tabs { count: 3, width: Some(v), height: None }),
                9 => op.tabs = Some(Tabs { count: 3, width: None, height: Some(v) }),
                _ => op.tabs = Some(Tabs { count: if v > 0.0 { usize::MAX } else { 0 }, width: Some(v), height: Some(v) }),
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
    assert!(serde_json::from_str::<ContourOp>(r#"{"spiral": true}"#).is_err());
    assert_eq!(serde_json::from_value::<ContourOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
    // Tabs are a number of them at least; the arcs in and out a radius.
    let op: ContourOp = serde_json::from_str(r#"{"lead": 1.5, "tabs": {"count": 4}}"#).unwrap();
    assert_eq!((op.lead, op.tabs), (Some(1.5), Some(Tabs { count: 4, width: None, height: None })));
    let op: ContourOp = serde_json::from_str(r#"{"tabs": {"count": 3, "width": 8, "height": 1.5}}"#).unwrap();
    assert_eq!(op.tabs, Some(Tabs { count: 3, width: Some(8.0), height: Some(1.5) }));
    assert!(serde_json::from_str::<ContourOp>(r#"{"tabs": 4}"#).is_err() && serde_json::from_str::<ContourOp>(r#"{"tabs": {"width": 4}}"#).is_err());
    assert_eq!(serde_json::from_value::<ContourOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}

#[test]
fn the_tool_meets_the_wall_and_leaves_it_by_quarter_turns() {
    // The 20 mm tool's arcs are 2 mm in radius (a tenth of its diameter) about (0, -32), 2 out
    // from where the round starts at (0, -30). Clockwise round the part is towards -X along the
    // front, so the tool comes on from (2, -32), counter-clockwise, and leaves to (-2, -32). It
    // goes down beside the stock at (2, -33), straight out from where the arc begins, and comes
    // up where the arc off the wall ends, in the cut it has just made.
    let d = contoured(&ContourOp::default()).unwrap();
    assert_eq!((d.entries.clone(), d.leads.clone()), (vec![[2.0, -33.0]], vec![2.0]));
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    let level = |z: f64| {
        let arc = |x: f64, y: f64, cx: f64, cy: f64, direction: ArcDirection| Move::Arc {
            to: p(x, y, z),
            center: p(cx, cy, z),
            plane: Plane::Xy,
            direction,
            feed: d.feed,
        };
        let line = |x: f64, y: f64| Move::Linear { to: p(x, y, z), feed: d.feed };
        vec![
            Move::Linear { to: p(2.0, -33.0, z), feed: d.plunge_feed },
            line(2.0, -32.0),
            arc(0.0, -30.0, 0.0, -32.0, ArcDirection::CounterClockwise),
            line(-30.0, -30.0),
            arc(-40.0, -20.0, -30.0, -20.0, ArcDirection::Clockwise),
            line(-40.0, 20.0),
            arc(-30.0, 30.0, -30.0, 20.0, ArcDirection::Clockwise),
            line(30.0, 30.0),
            arc(40.0, 20.0, 30.0, 20.0, ArcDirection::Clockwise),
            line(40.0, -20.0),
            arc(30.0, -30.0, 30.0, -20.0, ArcDirection::Clockwise),
            line(0.0, -30.0),
            arc(-2.0, -32.0, 0.0, -32.0, ArcDirection::CounterClockwise),
            Move::Rapid { to: p(-2.0, -32.0, 5.0) },
        ]
    };
    let mut want = vec![
        Move::ToolChange { tool: 4 },
        Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
        Move::Coolant { mode: Coolant::Flood },
        Move::Rapid { to: p(2.0, -33.0, 25.0) },
        Move::Rapid { to: p(2.0, -33.0, 5.0) },
    ];
    want.extend(level(-7.0));
    for z in [-14.0, -21.0] {
        // Back over to where it goes down.
        want.push(Move::Rapid { to: p(2.0, -33.0, 5.0) });
        want.extend(level(z));
    }
    want.extend([Move::Rapid { to: p(-2.0, -32.0, 25.0) }, Move::Coolant { mode: Coolant::Off }, Move::SpindleStop]);
    assert!(same_moves(&d.toolpath.moves, &want), "{:#?}", d.toolpath.moves);
    // Measured: plunges of 12, 19 and 26, and at each level 1 in, a quarter turn of radius 2
    // (pi), the round, and a quarter turn off. Rapids: 20 down, 12, 19 and 26 up, two hops of
    // sqrt(17) back to where it goes down, and 20 up at the end.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 57.0 + 3.0 * (201.0 + 22.0 * PI)), "{}", s.feed_length);
    assert!(close(s.rapid_length, 97.0 + 2.0 * 17.0_f64.sqrt()), "{}", s.rapid_length);
    // Conventional goes the other way round, so the arcs are mirrored: on from (-2, -32),
    // clockwise.
    let d = contoured(&ContourOp { direction: CutDirection::Conventional, stepdown: Some(21.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.entries, vec![[-2.0, -33.0]]);
    let on = Move::Arc { to: p(0.0, -30.0, -21.0), center: p(0.0, -32.0, -21.0), plane: Plane::Xy, direction: ArcDirection::Clockwise, feed: d.feed };
    let off =
        Move::Arc { to: p(2.0, -32.0, -21.0), center: p(0.0, -32.0, -21.0), plane: Plane::Xy, direction: ArcDirection::Clockwise, feed: d.feed };
    assert!(same_moves(&d.toolpath.moves[7..8], &[on]) && same_moves(&d.toolpath.moves[17..18], &[off]), "{:#?}", d.toolpath.moves);
    // A radius given is the radius used; a disc is met by an arc too, on the outside of its own.
    let d = contoured(&ContourOp { lead: Some(5.0), ..ContourOp::default() }).unwrap();
    assert_eq!((d.entries.clone(), d.leads.clone()), (vec![[5.0, -35.0]], vec![5.0]));
    let disc = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(40.0, 40.0, 10.0) }, 1.0);
    let d = ContourOp::default().toolpath(None, &mill20(), &placement, Some("aluminium"), &[disc]).unwrap();
    assert_eq!(d.leads, vec![2.0]);
    // Every point of the cut, the arcs on and off included, keeps the tool's radius from the disc.
    let nearest = cutting_points(&d.toolpath).iter().map(|at| at.x.hypot(at.y)).fold(f64::INFINITY, f64::min);
    assert!((nearest - 30.0).abs() <= 1e-6, "{nearest}");
}

/// A square loop from (0, 0) to (100, 100), counter-clockwise, as a tool's centre path.
fn square_path() -> Region2 {
    Region2 { outer: polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)]), holes: Vec::new() }
}

/// A small box as something in the way.
fn in_the_way(x: (f64, f64), y: (f64, f64)) -> Region2 {
    Region2 { outer: polygon(&[(x.0, y.0), (x.1, y.0), (x.1, y.1), (x.0, y.1)]), holes: Vec::new() }
}

#[test]
fn the_arcs_are_made_smaller_or_left_out_where_there_is_no_room() {
    let beside = (q(-20.0, -20.0), q(120.0, 120.0));
    let led = |all: &[Region2]| round(&all[0].outer, all, CutDirection::Climb, beside, 8.0).unwrap();
    let radius = |r: &Round| r.leads.map(|(on, _)| if let Seg2::Arc { radius, .. } = on { radius } else { 0.0 });
    // Nothing in the way: arcs of 8 at the middle of the front side. Clockwise is towards -X
    // there, so the tool comes on from (58, -8) and goes down straight out from that.
    let free = led(&[square_path()]);
    assert_eq!((free.start, free.entry, radius(&free)), (q(50.0, 0.0), q(58.0, -20.0), Some(8.0)));
    let (on, off) = free.leads.unwrap();
    assert!(apart(on.start(), q(58.0, -8.0)) < 1e-12 && apart(on.end(), q(50.0, 0.0)) < 1e-12, "{on:?}");
    assert!(apart(off.start(), q(50.0, 0.0)) < 1e-12 && apart(off.end(), q(42.0, -8.0)) < 1e-12, "{off:?}");
    // Something where the arc off the front side would pass: the next side is taken, with its
    // full arcs (the left one, of the two as near the front).
    let blocked_front = in_the_way((40.0, 45.0), (-6.0, -3.0));
    let next = led(&[square_path(), blocked_front.clone()]);
    assert_eq!((next.start, radius(&next)), (q(0.0, 50.0), Some(8.0)));
    // The same in the way at every side: the front again, with arcs of 4, which pass inside it.
    let all_round = [
        square_path(),
        blocked_front,
        in_the_way((-6.0, -3.0), (55.0, 60.0)),
        in_the_way((103.0, 106.0), (40.0, 45.0)),
        in_the_way((55.0, 60.0), (103.0, 106.0)),
    ];
    let smaller = led(&all_round);
    assert_eq!((smaller.start, smaller.entry, radius(&smaller)), (q(50.0, 0.0), q(54.0, -20.0), Some(4.0)));
    // Walls close on both sides of the way in to each side, 1.5 from it: no arc of 8, 4 or 2
    // passes, and the tool goes straight in at the front.
    let mut tight = vec![square_path()];
    for (x, y) in [((0.0, 48.5), (-30.0, -0.5)), ((51.5, 100.0), (-30.0, -0.5)), ((0.0, 48.5), (100.5, 130.0)), ((51.5, 100.0), (100.5, 130.0))] {
        tight.push(in_the_way(x, y));
        tight.push(in_the_way(y, x));
    }
    let none = led(&tight);
    assert_eq!((none.start, none.entry, radius(&none)), (q(50.0, 0.0), q(50.0, -20.0), None));
    // No arcs asked for: straight in, whatever the room.
    let plain = round(&square_path().outer, &[square_path()], CutDirection::Climb, beside, 0.0).unwrap();
    assert_eq!((plain.entry, radius(&plain)), (q(50.0, -20.0), None));
}

#[test]
fn an_arc_on_to_a_side_that_curves_away_stays_well_inside_its_curve() {
    // A crescent: a disc of 30 about the origin with a bite of radius 50 about (60, 0) out of
    // it. Its outside is the long arc; the bite is an arc the other way round, its centre on
    // the outside. A wall stands in the way in to the long arc, arcs and all, so the round
    // starts in the bite, at (10, 0), and an arc on to the wall there must be smaller than the
    // bite: at most half its radius, 25.
    let (x, y) = (50.0 / 3.0, (900.0_f64 - 2500.0 / 9.0).sqrt());
    let horn = y.atan2(x);
    let bite = (y).atan2(x - 60.0);
    let crescent = Region2 {
        outer: Loop2 {
            segs: vec![
                Seg2::Arc { center: q(0.0, 0.0), radius: 30.0, start: horn, sweep: 2.0 * PI - 2.0 * horn },
                Seg2::Arc { center: q(60.0, 0.0), radius: 50.0, start: 2.0 * PI - bite, sweep: -(2.0 * PI - 2.0 * bite) },
            ],
        },
        holes: Vec::new(),
    };
    assert!(crescent.outer.is_closed(1e-9) && crescent.outer.is_ccw());
    let all = [crescent, in_the_way((-50.0, -40.0), (-60.0, 60.0))];
    let beside = (q(-60.0, -60.0), q(60.0, 60.0));
    let radius = |lead: f64| {
        let r = round(&all[0].outer, &all, CutDirection::Climb, beside, lead).unwrap();
        assert!(apart(r.start, q(10.0, 0.0)) < 1e-9, "{:?}", r.start);
        r.leads.map(|(on, _)| if let Seg2::Arc { radius, .. } = on { radius } else { 0.0 })
    };
    assert_eq!(radius(20.0), Some(20.0));
    // 30 is too big for the bite; half of it is taken.
    assert_eq!(radius(30.0), Some(15.0));
    assert_eq!(radius(60.0), Some(15.0));
    // No part of 120 fits: straight in.
    assert_eq!(radius(120.0), None);
}

#[test]
fn an_arc_is_clear_only_when_it_meets_no_other_side_or_arc() {
    let region = |segs: Vec<Seg2>| vec![Region2 { outer: Loop2 { segs }, holes: Vec::new() }];
    let nothing = Loop2 { segs: Vec::new() };
    let clear = |arc: &Seg2, regions: &[Region2]| arc_is_clear(arc, regions, (&nothing, 0));
    // A quarter turn of radius 5 about the origin, from (5, 0) to (0, 5).
    let quarter = Seg2::Arc { center: q(0.0, 0.0), radius: 5.0, start: 0.0, sweep: 0.5 * PI };
    // A side across it, one beside it, one that stops short of it, and one that only touches it.
    assert!(!clear(&quarter, &region(vec![Seg2::Line { a: q(0.0, 0.0), b: q(6.0, 6.0) }])));
    assert!(clear(&quarter, &region(vec![Seg2::Line { a: q(0.0, 0.0), b: q(-6.0, -6.0) }])), "it would cross the other half of the circle");
    assert!(clear(&quarter, &region(vec![Seg2::Line { a: q(0.0, 0.0), b: q(3.0, 3.0) }])));
    assert!(!clear(&quarter, &region(vec![Seg2::Line { a: q(5.0, -1.0), b: q(5.0, 1.0) }])), "touching counts");
    assert!(clear(&quarter, &region(vec![Seg2::Line { a: q(5.5, -1.0), b: q(5.5, 1.0) }])));
    // Another arc: crossing it, on the far side of its circle, inside it, outside it.
    let other = |cx: f64, radius: f64, start: f64, sweep: f64| region(vec![Seg2::Arc { center: q(cx, 4.0), radius, start, sweep }]);
    assert!(!clear(&quarter, &other(4.0, 2.0, 0.0, 2.0 * PI)));
    assert!(clear(&quarter, &other(4.0, 2.0, -0.25 * PI, PI)), "only the half of that circle away from the arc");
    assert!(!clear(&quarter, &other(4.0, 2.0, 0.75 * PI, PI)), "the half towards it");
    assert!(clear(&quarter, &other(1.0, 0.5, 0.0, 2.0 * PI)));
    assert!(clear(&quarter, &other(9.0, 2.0, 0.0, 2.0 * PI)));
    // The side it is drawn to touch is not counted; the same side of another loop is.
    let wall = region(vec![Seg2::Line { a: q(5.0, -1.0), b: q(5.0, 1.0) }]);
    assert!(arc_is_clear(&quarter, &wall, (&wall[0].outer, 0)));
    assert!(!arc_is_clear(&quarter, &wall, (&wall[0].outer, 1)));
    // A straight line is not an arc.
    assert!(!clear(&Seg2::Line { a: q(0.0, 0.0), b: q(1.0, 0.0) }, &[]));
}

/// A simulation of `stock` in cells of 0.1 mm, with `d` cut from it by `tool`.
fn cut_from(stock: Bounds, d: &Contoured, tool: &Tool) -> crate::sim::Simulation {
    let mut sim = crate::sim::Simulation::new(stock, 0.1, 0.01).unwrap();
    sim.cut(0, &d.toolpath, tool).unwrap();
    sim
}

#[test]
fn tabs_are_left_at_the_bottom_one_to_a_side_where_they_fit() {
    // The block with the 6 mm tool: its centre goes round at X ±33 and Y ±23, the corners arcs
    // of 3, 200 + 6 pi round. Four tabs of 6 are 54.71 apart, and the tool is up for 12 at each
    // (the tab and its own diameter). The first may be 9 to 45.71 from the start; all four lie
    // on straight sides when it is 9 to 14 or 40.71 to 45.71, and the middle of the first of
    // those, 11.5, is taken: one tab to a side, each 11.5 past the middle of its side.
    let op = ContourOp { tabs: Some(Tabs { count: 4, width: None, height: None }), ..ContourOp::default() };
    let d = op.toolpath(None, &mill6(), &block(), Some("aluminium"), &[polygon(&BLOCK)]).unwrap();
    assert_eq!(d.levels, vec![-3.0, -6.0, -9.0, -12.0, -15.0, -18.0, -21.0]);
    assert_eq!(d.tab_top, Some(-19.0));
    let want = [(-11.5, -23.0), (-33.0, 11.5), (11.5, 23.0), (33.0, -11.5)];
    assert_eq!(d.tabs.len(), 4);
    for (got, want) in d.tabs.iter().zip(want) {
        assert!((got[0] - want.0).abs() < 1e-9 && (got[1] - want.1).abs() < 1e-9, "{:?}", d.tabs);
    }
    // Only the last level is below the tabs' tops. There the tool goes up 2 at the cutting feed
    // 6 before each tab's middle, across at Z -19, and down again 6 past it at the plunge feed.
    let mut at = d.toolpath.start;
    let (mut ups, mut downs, mut over) = (Vec::new(), Vec::new(), Vec::new());
    for m in &d.toolpath.moves {
        let to = m.end(at);
        if let Move::Linear { feed, .. } = *m {
            let level = (at.x - to.x).abs() > 1e-9 || (at.y - to.y).abs() > 1e-9;
            match (level, at.z, to.z) {
                (false, a, b) if a == -21.0 && b == -19.0 => ups.push((to.x, to.y, feed)),
                (false, a, b) if a == -19.0 && b == -21.0 => downs.push((to.x, to.y, feed)),
                (true, a, b) if a == -19.0 && b == -19.0 => over.push((at.x - to.x).hypot(at.y - to.y)),
                _ => {}
            }
        }
        at = to;
    }
    let near = |got: &[(f64, f64, f64)], want: &[(f64, f64)], feed: f64| {
        got.len() == want.len() && got.iter().zip(want).all(|(g, w)| (g.0 - w.0).abs() < 1e-9 && (g.1 - w.1).abs() < 1e-9 && close(g.2, feed))
    };
    assert!(near(&ups, &[(-5.5, -23.0), (-33.0, 5.5), (5.5, 23.0), (33.0, -5.5)], d.feed), "{ups:?}");
    assert!(near(&downs, &[(-17.5, -23.0), (-33.0, 17.5), (17.5, 23.0), (33.0, -17.5)], d.plunge_feed), "{downs:?}");
    assert!(over.len() == 4 && over.iter().all(|&length| close(length, 12.0)), "{over:?}");
    // Measured in stock with 10 mm round the block, so that the whole of the slot is in it: each
    // tab stands 2 high where the tool passed over it, the slot beside it is cut to the bottom,
    // and the four hold back what a 6 mm slot 12 long less the tool's two half discs is, 2
    // high: 4 x (72 - 9 pi) x 2 mm3. The tabs are where they were: they go by the round.
    let wide = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }, 10.0);
    let stock = Bounds { min: p(-40.0, -30.0, -21.0), max: p(40.0, 30.0, 0.0) };
    let in_wide = |op: &ContourOp| op.toolpath(None, &mill6(), &wide, Some("aluminium"), &[polygon(&BLOCK)]).unwrap();
    let (whole, tabbed) = (cut_from(stock, &in_wide(&ContourOp::default()), &mill6()), cut_from(stock, &in_wide(&op), &mill6()));
    assert_eq!(in_wide(&op).tabs, d.tabs);
    let held = whole.removed - tabbed.removed;
    assert!((held - 8.0 * (72.0 - 9.0 * PI)).abs() < 0.02 * held, "{held}");
    let height = |sim: &crate::sim::Simulation, x: f64, y: f64| {
        let f = &sim.stock;
        f.at(((x - f.x) / f.dx - 0.5).round() as usize, ((y - f.y) / f.dy - 0.5).round() as usize).unwrap()
    };
    assert_eq!((height(&tabbed, -11.5, -23.0), height(&tabbed, -25.0, -23.0), height(&whole, -11.5, -23.0)), (-19.0, -21.0, -21.0));
    assert_eq!((height(&tabbed, 33.0, -11.5), height(&tabbed, 33.0, 5.0)), (-19.0, -21.0));
    // Tabs 4 high stand above the last two levels: the tool lifts over each on both.
    let op = ContourOp { tabs: Some(Tabs { count: 4, width: Some(6.0), height: Some(4.0) }), ..ContourOp::default() };
    let d = op.toolpath(None, &mill6(), &block(), Some("aluminium"), &[polygon(&BLOCK)]).unwrap();
    let mut at = d.toolpath.start;
    let mut lifts = Vec::new();
    for m in &d.toolpath.moves {
        let to = m.end(at);
        if matches!(m, Move::Linear { .. }) && at.x == to.x && at.y == to.y && to.z == -17.0 {
            lifts.push(at.z);
        }
        at = to;
    }
    assert_eq!(lifts, vec![-18.0, -18.0, -18.0, -18.0, -21.0, -21.0, -21.0, -21.0]);
    assert_eq!(d.tab_top, Some(-17.0));
    // Two tabs: the middles of the left and right sides. Three are a third of the round apart:
    // all are on straight sides with the first from 76 + 3 pi less that third (where the second
    // comes on to the back side) to 24 (where the first leaves the front one). The middle of
    // that is 50/3 + pi/2: tabs at the front and at the back, each that far left of the middle,
    // and one at the middle of the right side.
    let middles = |count: usize| {
        let op = ContourOp { tabs: Some(Tabs { count, width: None, height: None }), ..ContourOp::default() };
        op.toolpath(None, &mill6(), &block(), Some("aluminium"), &[polygon(&BLOCK)]).unwrap().tabs
    };
    let two = middles(2);
    assert!(two.len() == 2 && apart(q(two[0][0], two[0][1]), q(-33.0, 0.0)) < 1e-9 && apart(q(two[1][0], two[1][1]), q(33.0, 0.0)) < 1e-9, "{two:?}");
    let three = middles(3);
    let first = 50.0 / 3.0 + 0.5 * PI;
    let want = [q(-first, -23.0), q(-first, 23.0), q(33.0, 0.0)];
    assert!(three.len() == 3 && three.iter().zip(want).all(|(got, want)| apart(q(got[0], got[1]), want) < 1e-9), "{three:?}");
}

#[test]
fn tabs_keep_to_straight_sides_where_they_can_and_are_evenly_spaced_where_they_cannot() {
    let along = |from: f64, to: f64| Seg2::Line { a: q(from, 0.0), b: q(to, 0.0) };
    // A corner as an arc of the given length (its radius 10) from (x, 0).
    let corner = |x: f64, length: f64| Seg2::Arc { center: q(x, 10.0), radius: 10.0, start: -0.5 * PI, sweep: length / 10.0 };
    // One tab on a round of two sides of 40 with a corner of 10 between them: the tool is up
    // for 12, which may begin 3 from the start and end 3 before the end. A whole side holds it
    // when its middle is 9 to 34 or 56 to 81: the middle of the first, 21.5.
    let spans = tab_spans(&[along(0.0, 40.0), corner(40.0, 10.0), along(50.0, 90.0)], 1, 6.0, &mill6()).unwrap();
    assert_eq!(spans, vec![(15.5, 27.5)]);
    // Two tabs on three sides of 30 with corners of 12 between: 57 apart. No place puts both on
    // a side: with the first at 9 to 24 it is on the first side, and with it at 33 to 48 the
    // second tab is on the third. The first stretch is taken, its middle 16.5; the second tab,
    // at 73.5, would lie on the second corner (72 to 84) and is moved back 7.5, on to the end
    // of the middle side.
    let sides = [along(0.0, 30.0), corner(30.0, 12.0), along(42.0, 72.0), corner(72.0, 12.0), along(84.0, 114.0)];
    assert_eq!(tab_spans(&sides, 2, 6.0, &mill6()).unwrap(), vec![(10.5, 22.5), (60.0, 72.0)]);
    // No straight side long enough: evenly spaced, the first half a space from the start.
    let arcs = [corner(0.0, 15.0), corner(15.0, 15.0), corner(30.0, 15.0), corner(45.0, 15.0)];
    assert_eq!(tab_spans(&arcs, 2, 6.0, &mill6()).unwrap(), vec![(9.0, 21.0), (39.0, 51.0)]);
    // Too many to fit: each tab takes its width and twice the tool's diameter.
    let e = tab_spans(&sides, 7, 6.0, &mill6()).unwrap_err().to_string();
    assert!(e.contains("7 tab(s) 6 mm wide do not fit") && e.contains("each takes 18 mm of the 114.000 mm"), "{e}");
    // Six just fit (19 apart for the 18 each takes): half a millimetre of choice, and none of
    // it on straight sides for all, but every lift is 12 long with at least 6 down between.
    let spans = tab_spans(&sides, 6, 6.0, &mill6()).unwrap();
    assert_eq!(spans.len(), 6);
    assert!(spans.iter().all(|&(up, down)| close(down - up, 12.0)) && spans.windows(2).all(|w| w[1].0 - w[0].1 >= 6.0 - 1e-9), "{spans:?}");
    assert!(spans[0].0 >= 3.0 - 1e-9 && spans[5].1 <= 111.0 + 1e-9, "{spans:?}");
}

#[test]
fn a_round_part_has_its_tabs_evenly_round_it() {
    // A disc of 20: the 6 mm tool's centre goes round at 23, clockwise from the front, (0, -23).
    // Three tabs, a third of a turn apart, the first a sixth of a turn from the start.
    let disc = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let placement = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(40.0, 40.0, 10.0) }, 1.0);
    let op = ContourOp { tabs: Some(Tabs { count: 3, width: None, height: None }), ..ContourOp::default() };
    let d = op.toolpath(None, &mill6(), &placement, Some("aluminium"), &[disc]).unwrap();
    let want = [(-23.0 * 0.75_f64.sqrt(), -11.5), (0.0, 23.0), (23.0 * 0.75_f64.sqrt(), -11.5)];
    assert_eq!(d.tabs.len(), 3);
    for (got, want) in d.tabs.iter().zip(want) {
        assert!((got[0] - want.0).abs() < 1e-9 && (got[1] - want.1).abs() < 1e-9, "{:?}", d.tabs);
    }
    // Over each tab the tool is 2 above the bottom (Z -11) for 12 mm of arc, and never nearer
    // the disc than its radius.
    let points = cutting_points(&d.toolpath);
    let nearest = points.iter().map(|at| at.x.hypot(at.y)).fold(f64::INFINITY, f64::min);
    assert!((nearest - 23.0).abs() <= 1e-6, "{nearest}");
    let mut at = d.toolpath.start;
    let mut over = 0.0;
    for m in &d.toolpath.moves {
        let to = m.end(at);
        if let Move::Arc { center, .. } = *m
            && at.z == -9.0
            && to.z == -9.0
        {
            let turned = ((at.y - center.y).atan2(at.x - center.x) - (to.y - center.y).atan2(to.x - center.x)).rem_euclid(2.0 * PI);
            over += 23.0 * turned;
        }
        at = to;
    }
    assert!(close(over, 36.0), "{over}");
}

/// A plate 120 x 100 x 10 with a window 80 x 60 through its middle: in the WCS the plate is X -60
/// to 60, Y -50 to 50, Z -11 to -1 under 1 mm of stock, and the window X -40 to 40, Y -30 to 30.
fn plate() -> Placement {
    placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(120.0, 100.0, 10.0) }, 1.0)
}

const WINDOW: [(f64, f64); 4] = [(-40.0, -30.0), (40.0, -30.0), (40.0, 30.0), (-40.0, 30.0)];

fn inside(op: &ContourOp) -> Result<Contoured> {
    ContourOp { side: ContourSide::Inside, openings: vec![[0.0, 0.0]], ..op.clone() }.toolpath(
        None,
        &mill20(),
        &plate(),
        Some("aluminium"),
        &[polygon(&WINDOW)],
    )
}

#[test]
fn an_inside_contour_goes_down_a_ramp_along_the_wall_and_once_round_at_each_level() {
    // The 20 mm tool's centre is 10 inside the window's wall: a rectangle X ±30, Y ±20, 200 round.
    // The stock's top is Z 0 and the plate's bottom Z -11: two levels of 5.5. Climb, with a
    // clockwise spindle, is counter-clockwise round the inside. The tool comes down over the
    // middle of the front side, (0, -20), and from the top goes down along the wall: at 3
    // degrees 5.5 mm takes 105 mm, so one turn of the 200, falling evenly, then the level once
    // round. At the bottom it leaves the wall by a quarter turn of 2 (a tenth of the tool) into
    // the window.
    let d = inside(&ContourOp::default()).unwrap();
    assert_eq!((d.top, d.levels.clone(), d.stepdown, d.offset), (0.0, vec![-5.5, -11.0], 10.0, 10.0));
    assert_eq!((d.entries.clone(), d.leads.clone(), d.tabs.len(), d.tab_top), (vec![[0.0, -20.0]], vec![2.0], 0, None));
    assert!(close(d.round, 200.0), "{}", d.round);
    // A cut as wide as the tool, 0.2 mm a revolution; down the ramp at half that.
    let (cut, ramp) = (0.2 * RPM, 0.1 * RPM);
    assert!(close(d.rpm, RPM) && close(d.feed, cut) && close(d.plunge_feed, ramp), "{} {} {}", d.rpm, d.feed, d.plunge_feed);
    // What the tool does not pass over: the middle 40 x 20, about 20 across.
    assert_eq!(
        d.notes,
        ["opening 1: a core about 20.0 mm across is left in its middle, loose once the cut is through; where it must be cut away, use Pocket"]
    );
    let corners = [(30.0, -20.0), (30.0, 20.0), (-30.0, 20.0), (-30.0, -20.0), (0.0, -20.0)];
    // How far round each corner is from the start, of the 200.
    let along = [30.0, 70.0, 130.0, 170.0, 200.0];
    let mut want = vec![
        Move::ToolChange { tool: 4 },
        Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
        Move::Coolant { mode: Coolant::Flood },
        Move::Rapid { to: p(0.0, -20.0, 25.0) },
        Move::Rapid { to: p(0.0, -20.0, 5.0) },
        Move::Linear { to: p(0.0, -20.0, 0.0), feed: ramp },
    ];
    for (above, z) in [(0.0, -5.5), (-5.5, -11.0)] {
        want.extend(corners.iter().zip(along).map(|(&(x, y), s)| Move::Linear { to: p(x, y, above + (z - above) * s / 200.0), feed: ramp }));
        want.extend(corners.iter().map(|&(x, y)| Move::Linear { to: p(x, y, z), feed: cut }));
    }
    want.extend([
        Move::Arc { to: p(2.0, -18.0, -11.0), center: p(0.0, -18.0, -11.0), plane: Plane::Xy, direction: ArcDirection::CounterClockwise, feed: cut },
        Move::Rapid { to: p(2.0, -18.0, 5.0) },
        Move::Rapid { to: p(2.0, -18.0, 25.0) },
        Move::Coolant { mode: Coolant::Off },
        Move::SpindleStop,
    ]);
    assert!(same_moves(&d.toolpath.moves, &want), "{:#?}", d.toolpath.moves);
    assert_eq!(d.toolpath.start, p(0.0, -20.0, 25.0));
    // Measured: 5 down to the top, two ramps of 200 falling 5.5 (each side longer by the same
    // share), two rounds of 200, and the quarter turn of radius 2.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 5.0 + 2.0 * 200.0 * (1.0 + (5.5f64 / 200.0).powi(2)).sqrt() + 400.0 + PI), "{}", s.feed_length);
    assert_eq!((s.min, s.max), (p(-30.0, -20.0, -11.0), p(30.0, 20.0, 25.0)));

    // The simulation: the band the tool passes over is the window with its corners left round
    // (4800 less 4 (100 - 25 pi)) less the middle 40 x 20, through the 11 mm of stock; nothing
    // of the plate is cut into, and no rapid passes through material.
    let placement = plate();
    let sim = crate::sim::Simulation::new(placement.stock, 0.25, 0.01).unwrap();
    let f = sim.stock.clone();
    let part: Vec<f64> = (0..f.nx * f.ny)
        .map(|k| {
            let c = f.centre(k % f.nx, k / f.nx);
            if c.x.abs() < 40.0 && c.y.abs() < 30.0 { -12.0 } else { -1.0 }
        })
        .collect();
    let mut sim = sim.with_part(part).unwrap();
    let report = sim.cut(0, &d.toolpath, &mill20()).unwrap();
    assert_eq!((report.gouges, report.rapid_hits, report.body_hits), (0, 0, 0), "{:?}", sim.gouges.first());
    let band = (4800.0 - 4.0 * (100.0 - 25.0 * PI) - 800.0) * 11.0;
    assert!((report.removed - band).abs() < 0.01 * band, "{} mm3, not {band}", report.removed);
}

#[test]
fn conventional_inside_goes_the_other_way_and_stock_to_leave_moves_the_path_in() {
    // Clockwise round the inside: from the middle of the front side towards -X, and off the
    // wall by a quarter turn the other way.
    let d = inside(&ContourOp { direction: CutDirection::Conventional, stepdown: Some(11.0), ..ContourOp::default() }).unwrap();
    assert_eq!(d.levels, vec![-11.0]);
    let cuts: Vec<Vec3> = d.toolpath.moves.iter().filter_map(|m| if let Move::Linear { to, .. } = m { Some(*to) } else { None }).collect();
    // Down to the top, then the ramp's first side, to the front left corner. One level of 11 mm
    // at 3 degrees is 210 mm of path, more than the 200 round: two turns, so the corner, 30
    // round, is 30 of 400 of the way down.
    assert_eq!(cuts[0], p(0.0, -20.0, 0.0));
    assert!((cuts[1].x + 30.0).abs() < 1e-9 && (cuts[1].y + 20.0).abs() < 1e-9 && (cuts[1].z + 11.0 * 30.0 / 400.0).abs() < 1e-9, "{:?}", cuts[1]);
    let Some(Move::Arc { to, center, direction, .. }) = d.toolpath.moves.iter().find(|m| matches!(m, Move::Arc { .. })) else {
        panic!("no arc off the wall")
    };
    assert_eq!((*to, *center, *direction), (p(-2.0, -18.0, -11.0), p(0.0, -18.0, -11.0), ArcDirection::Clockwise));
    // Half a millimetre left on the wall: the path is 10.5 inside it, 59 x 39, 196 round.
    let d = inside(&ContourOp { leave: Some(0.5), lead: Some(0.0), ..ContourOp::default() }).unwrap();
    assert!(close(d.round, 196.0) && d.offset == 10.5 && d.entries == vec![[0.0, -19.5]] && d.leads == vec![0.0], "{} {:?}", d.round, d.entries);
    // No arc was asked for: straight up from the wall, with nothing to note about it.
    assert!(!d.toolpath.moves.iter().any(|m| matches!(m, Move::Arc { .. })) && d.notes.len() == 1, "{:?}", d.notes);
}

#[test]
fn a_round_hole_is_bored_down_a_helix() {
    // A hole of 24 through the plate's middle, with the 20 mm tool: its centre goes round a
    // circle of radius 2, 12.57 round. Down 5.5 mm at 3 degrees is 105 mm of path: nine turns to
    // each level, every one an arc about the hole's middle, counter-clockwise, falling as it
    // goes; then the level once round. The tool passes over the whole hole: nothing is left in
    // its middle, and the hole is cut to its wall all round and no further.
    let hole = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 12.0, start: 0.0, sweep: 2.0 * PI }] };
    let op = ContourOp { side: ContourSide::Inside, openings: vec![[0.0, 0.0]], ..ContourOp::default() };
    let d = op.toolpath(None, &mill20(), &plate(), Some("aluminium"), &[hole]).unwrap();
    assert!(close(d.round, 4.0 * PI) && d.levels == vec![-5.5, -11.0] && d.notes.is_empty(), "{} {:?}", d.round, d.notes);
    let arcs: Vec<(Vec3, Vec3, ArcDirection)> = d
        .toolpath
        .moves
        .iter()
        .filter_map(|m| if let Move::Arc { to, center, direction, .. } = m { Some((*to, *center, *direction)) } else { None })
        .collect();
    let about_the_middle = |a: &&(Vec3, Vec3, ArcDirection)| a.1.x.abs() < 1e-9 && a.1.y.abs() < 1e-9;
    let turns: Vec<&(Vec3, Vec3, ArcDirection)> = arcs.iter().filter(about_the_middle).collect();
    assert!(turns.iter().all(|a| a.2 == ArcDirection::CounterClockwise && (a.0.x.hypot(a.0.y) - 2.0).abs() < 1e-9), "{arcs:?}");
    // Never upward. The path is two half circles and the tool starts at the middle of one: three
    // arcs a turn, and each level's nine turns and its round are 3 (9 + 1), twice. At each
    // level's own depth: the last arc of its ramp and the three of its round.
    assert_eq!(turns.len(), 60);
    assert!(turns.windows(2).all(|pair| pair[1].0.z <= pair[0].0.z + 1e-12));
    assert_eq!((turns.iter().filter(|a| a.0.z == -5.5).count(), turns.iter().filter(|a| a.0.z == -11.0).count()), (4, 4));
    // Off the wall by a quarter turn of 1: a half of the 2 asked, which is as wide as the path.
    assert_eq!(d.leads, vec![1.0]);

    let placement = plate();
    let sim = crate::sim::Simulation::new(placement.stock, 0.25, 0.01).unwrap();
    let f = sim.stock.clone();
    let out = |k: usize| f.centre(k % f.nx, k / f.nx).x.hypot(f.centre(k % f.nx, k / f.nx).y);
    let part: Vec<f64> = (0..f.nx * f.ny).map(|k| if out(k) < 12.0 { -12.0 } else { -1.0 }).collect();
    let mut sim = sim.with_part(part).unwrap();
    let report = sim.cut(0, &d.toolpath, &mill20()).unwrap();
    assert_eq!((report.gouges, report.rapid_hits), (0, 0), "{:?}", sim.gouges.first());
    assert!(
        (0..f.nx * f.ny).all(|k| sim.stock.heights[k] == if out(k) < 12.0 { -11.0 } else { 0.0 }),
        "the hole and nothing else, to the stock's bottom"
    );
}

#[test]
fn what_cannot_be_contoured_inside_is_refused_with_the_reason() {
    // A slot 15 wide does not take a 20 mm tool; its width is said.
    let slot = polygon(&[(-40.0, -7.5), (40.0, -7.5), (40.0, 7.5), (-40.0, 7.5)]);
    let op = ContourOp { side: ContourSide::Inside, openings: vec![[0.0, 0.0]], ..ContourOp::default() };
    let refused = op.toolpath(None, &mill20(), &plate(), Some("aluminium"), std::slice::from_ref(&slot)).unwrap_err().to_string();
    assert!(refused.contains("tool 4 (20 mm across) does not fit in opening 1, which is about 15.00 mm across"), "{refused}");
    // The 6 mm tool does; given clockwise, the wall is gone round all the same.
    let d = op.toolpath(None, &mill6(), &plate(), Some("aluminium"), &[slot.reversed()]).unwrap();
    assert!(close(d.round, 2.0 * (74.0 + 9.0)) && d.entries == vec![[0.0, -4.5]], "{} {:?}", d.round, d.entries);
    // Tabs are for a part cut round its outside, a ramp for the inside, and the ramp has its limits.
    let tabs = Some(Tabs { count: 2, width: None, height: None });
    assert!(inside(&ContourOp { tabs, ..ContourOp::default() }).unwrap_err().to_string().contains("an inside contour takes none"));
    let ramp = ContourOp { ramp_angle: Some(0.1), ..ContourOp::default() };
    assert!(contoured(&ramp).unwrap_err().to_string().contains("a ramp angle is for an inside contour"));
    assert!(inside(&ContourOp { ramp_angle: Some(1.0), ..ContourOp::default() }).unwrap_err().to_string().contains("0.5 to 30 degrees"));
    assert!(inside(&ContourOp { ramp_angle: Some(f64::NAN), ..ContourOp::default() }).is_err());
    // A hole hardly wider than the tool: the path is so short that the ramp takes too many turns.
    let tight = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 10.001, start: 0.0, sweep: 2.0 * PI }] };
    let refused = op.toolpath(None, &mill20(), &plate(), Some("aluminium"), &[tight]).unwrap_err().to_string();
    assert!(refused.contains("would take") && refused.contains("turns"), "{refused}");
    // Kept as plain data, and an operation saved before there was a side is an outside one.
    // An inside contour is given its openings, one for each of its points; an outside one has none.
    let none = ContourOp { side: ContourSide::Inside, ..ContourOp::default() };
    assert!(
        none.toolpath(None, &mill20(), &plate(), Some("aluminium"), &[polygon(&WINDOW)])
            .unwrap_err()
            .to_string()
            .contains("1 to 1000 openings, not 0")
    );
    let two = ContourOp { openings: vec![[0.0, 0.0], [1.0, 1.0]], ..op.clone() };
    let refused = two.toolpath(None, &mill20(), &plate(), Some("aluminium"), &[polygon(&WINDOW)]).unwrap_err().to_string();
    assert!(refused.contains("1 openings were read from the model for the operation's 2 points"), "{refused}");
    assert!(
        contoured(&ContourOp { openings: vec![[0.0, 0.0]], ..ContourOp::default() })
            .unwrap_err()
            .to_string()
            .contains("openings are for an inside contour")
    );
    let region = Region2 { outer: polygon(&WINDOW), holes: Vec::new() };
    let outside = ContourOp::default().toolpath_inside(None, &mill20(), &plate(), Some("aluminium"), std::slice::from_ref(&region));
    assert!(outside.unwrap_err().to_string().contains("an outside contour goes round the part's outline"));
    assert!(ContourOp { openings: vec![[f64::NAN, 0.0]], ..op.clone() }.toolpath_inside(None, &mill20(), &plate(), None, &[region]).is_err());
    // Kept as plain data, and an operation saved before there was a side is an outside one.
    let kept =
        serde_json::to_value(ContourOp { side: ContourSide::Inside, openings: vec![[1.0, 2.0]], ramp_angle: Some(0.1), ..ContourOp::default() })
            .unwrap();
    assert_eq!(
        (kept["side"].as_str(), kept["ramp_angle"].as_f64(), kept["openings"].clone()),
        (Some("inside"), Some(0.1), serde_json::json!([[1.0, 2.0]]))
    );
    let old: ContourOp = serde_json::from_str(r#"{"top": null, "bottom": -5.0}"#).unwrap();
    assert_eq!((old.side, old.ramp_angle, old.bottom, old.openings.len()), (ContourSide::Outside, None, Some(-5.0), 0));
}

#[test]
fn a_post_near_the_wall_is_gone_round_at_the_tools_radius() {
    // A post 10 across stands in the window at (25, 0), 10 from the wall on its right: the 20 mm
    // tool does not pass between them. Where the tool's centre may be is the rectangle X ±30,
    // Y ±20 less a circle of 15 about the post's middle (its radius and the tool's), which takes
    // the right side from Y -14.14 to 14.14 (15² - 5² = 200). The tool leaves the wall there,
    // goes round the far side of the post on that circle (360 less twice atan(14.14 / 5), 218.9
    // degrees of it) and comes back to the wall: 200 - 28.28 + 57.32 round.
    let post = Loop2 { segs: vec![Seg2::Arc { center: q(25.0, 0.0), radius: 5.0, start: 0.0, sweep: 2.0 * PI }] };
    let opening = Region2 { outer: polygon(&WINDOW), holes: vec![post] };
    let op = ContourOp { side: ContourSide::Inside, openings: vec![[0.0, 0.0]], stepdown: Some(11.0), ..ContourOp::default() };
    let d = op.toolpath_inside(None, &mill20(), &plate(), Some("aluminium"), &[opening]).unwrap();
    let chord = 200f64.sqrt();
    let round = 200.0 - 2.0 * chord + 15.0 * (2.0 * PI - 2.0 * (chord / 5.0).atan());
    assert!(close(d.round, round) && (round - 229.03).abs() < 0.01, "{} and {round}", d.round);
    assert_eq!((d.entries.clone(), d.leads.clone()), (vec![[0.0, -20.0]], vec![2.0]));
    // Never nearer the wall or the post than its radius, wherever it cuts.
    let cuts = cutting_points(&d.toolpath);
    assert!(cuts.len() > 50);
    for at in cuts {
        assert!(at.x.abs() <= 30.0 + 1e-6 && at.y.abs() <= 20.0 + 1e-6 && (at.x - 25.0).hypot(at.y) >= 15.0 - 1e-6, "{at:?}");
    }
    // The simulation, with the post standing as high as the plate: nothing of either is cut into.
    let sim = crate::sim::Simulation::new(plate().stock, 0.25, 0.01).unwrap();
    let f = sim.stock.clone();
    let part: Vec<f64> = (0..f.nx * f.ny)
        .map(|k| {
            let c = f.centre(k % f.nx, k / f.nx);
            if c.x.abs() < 40.0 && c.y.abs() < 30.0 && (c.x - 25.0).hypot(c.y) >= 5.0 { -12.0 } else { -1.0 }
        })
        .collect();
    let mut sim = sim.with_part(part).unwrap();
    let report = sim.cut(0, &d.toolpath, &mill20()).unwrap();
    assert_eq!((report.gouges, report.rapid_hits), (0, 0), "{:?}", sim.gouges.first());
}

/// One level of one pass round the block, straight in and out: the tool's centre `out` from the
/// block's walls, down at Y `entry` and in to the middle of the front side from there.
fn pass_level(out: f64, entry: f64, z: f64, cut: f64, plunge: f64) -> Vec<Move> {
    let arc = |x: f64, y: f64, cx: f64, cy: f64| Move::Arc {
        to: p(x, y, z),
        center: p(cx, cy, z),
        plane: Plane::Xy,
        direction: ArcDirection::Clockwise,
        feed: cut,
    };
    let line = |x: f64, y: f64| Move::Linear { to: p(x, y, z), feed: cut };
    let (x, y) = (30.0 + out, 20.0 + out);
    let mut moves = vec![Move::Linear { to: p(0.0, entry, z), feed: plunge }];
    if entry != -y {
        moves.push(line(0.0, -y));
    }
    moves.extend([
        line(-30.0, -y),
        arc(-x, -20.0, -30.0, -20.0),
        line(-x, 20.0),
        arc(-30.0, y, -30.0, 20.0),
        line(30.0, y),
        arc(x, 20.0, 30.0, 20.0),
        line(x, -20.0),
        arc(30.0, -y, 30.0, -20.0),
        line(0.0, -y),
    ]);
    if entry != -y {
        moves.push(line(0.0, entry));
    }
    moves.push(Move::Rapid { to: p(0.0, entry, 5.0) });
    moves
}

#[test]
fn a_contour_takes_its_passes_sideways_from_the_outside_in_at_every_level() {
    // Three passes with the 20 mm tool, four tenths of it apart (8): its centre 26, 18 and 10
    // outside the block, the furthest first, at each of the three levels in turn. The two
    // outer rounds begin outside where the tool may go down (33 from the middle, front), so it
    // goes down where they begin; the last comes in from Y -33, as one pass alone does.
    let op = ContourOp { passes: Some(3), ..straight() };
    let d = contoured(&op).unwrap();
    assert_eq!((d.passes, d.stepover, d.offset, d.levels.clone()), (3, 8.0, 10.0, vec![-7.0, -14.0, -21.0]));
    assert_eq!(d.entries, vec![[0.0, -46.0], [0.0, -38.0], [0.0, -33.0]]);
    assert!(close(d.round, 600.0 + 108.0 * PI), "{}", d.round);
    let mut want = vec![
        Move::ToolChange { tool: 4 },
        Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
        Move::Coolant { mode: Coolant::Flood },
        Move::Rapid { to: p(0.0, -46.0, 25.0) },
        Move::Rapid { to: p(0.0, -46.0, 5.0) },
    ];
    for (k, z) in [-7.0, -14.0, -21.0].into_iter().enumerate() {
        for (out, entry) in [(26.0, -46.0), (18.0, -38.0), (10.0, -33.0)] {
            // Across above the stock to where this pass goes down (the first of all is there).
            if k > 0 || out < 26.0 {
                want.push(Move::Rapid { to: p(0.0, entry, 5.0) });
            }
            want.extend(pass_level(out, entry, z, d.feed, d.plunge_feed));
        }
    }
    want.extend([Move::Rapid { to: p(0.0, -33.0, 25.0) }, Move::Coolant { mode: Coolant::Off }, Move::SpindleStop]);
    assert!(same_moves(&d.toolpath.moves, &want), "{:#?}", d.toolpath.moves);
    // One pass is the contour as it was, to the last move; a stepover given is the one used.
    let one = contoured(&ContourOp { passes: Some(1), ..straight() }).unwrap();
    assert!(same_moves(&one.toolpath.moves, &contoured(&straight()).unwrap().toolpath.moves) && (one.passes, one.stepover) == (1, 0.0));
    let near = contoured(&ContourOp { passes: Some(2), stepover: Some(3.0), ..straight() }).unwrap();
    assert_eq!((near.passes, near.stepover, near.entries.clone()), (2, 3.0, vec![[0.0, -33.0], [0.0, -33.0]]));
    assert!(close(near.round, 400.0 + 46.0 * PI), "{}", near.round);
    // With stock to leave the last pass is that far off the wall, and the others beyond it.
    let left = contoured(&ContourOp { passes: Some(2), leave: Some(0.5), ..straight() }).unwrap();
    assert!(close(left.offset, 10.5) && close(left.round, 400.0 + (37.0 + 21.0) * PI), "{} {}", left.offset, left.round);
}

#[test]
fn passes_cut_away_all_the_stock_beside_a_part_and_a_shoulder_down_to_its_floor() {
    // The block in stock 24 bigger all round (108 x 88, its top at 0 and its bottom at -21).
    // Three passes with the 20 mm tool reach 36 from the block's walls, past the stock's
    // corners (33.9 from the block's): nothing is left beside the block, and what is removed
    // is the stock less the block's footprint, 21 deep: (108 x 88 - 60 x 40) x 21.
    let wide = placed_on(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }, 24.0);
    let cut_with = |op: &ContourOp, tool: &Tool, placement: &Placement, outline: &[(f64, f64)], part: &dyn Fn(f64, f64) -> f64| {
        let d = op.toolpath(None, tool, placement, Some("aluminium"), &[polygon(outline)]).unwrap();
        let sim = crate::sim::Simulation::new(placement.stock, 0.25, 0.01).unwrap();
        let f = sim.stock.clone();
        let heights = (0..f.nx * f.ny)
            .map(|k| {
                let c = f.centre(k % f.nx, k / f.nx);
                part(c.x, c.y)
            })
            .collect();
        let mut sim = sim.with_part(heights).unwrap();
        let report = sim.cut(0, &d.toolpath, tool).unwrap();
        (sim, report)
    };
    let block_part = |x: f64, y: f64| if x.abs() < 30.0 && y.abs() < 20.0 { -1.0 } else { -22.0 };
    let (sim, report) = cut_with(&ContourOp { passes: Some(3), ..ContourOp::default() }, &mill20(), &wide, &BLOCK, &block_part);
    assert_eq!((report.gouges, report.rapid_hits), (0, 0));
    let (over, beside) = sim.leftover();
    assert!(beside == 0.0 && close(over, 2400.0), "{over} over the block, {beside} beside it");
    assert!((report.removed - (108.0 * 88.0 - 2400.0) * 21.0).abs() < 1e-6, "{}", report.removed);
    // One pass cuts a slot as wide as the tool and leaves a frame of the stock standing round
    // it, 4 wide and wider at its corners, but for where the tool came in across it.
    let (sim, _) = cut_with(&ContourOp::default(), &mill20(), &wide, &BLOCK, &block_part);
    let frame = (108.0 * 88.0 - (100.0 * 80.0 - (4.0 - PI) * 400.0)) * 21.0;
    let beside = sim.leftover().1;
    assert!(beside < frame && beside > frame - 20.0 * 4.5 * 21.0, "{beside} of a frame of {frame}");

    // A shoulder: on the block's footprint a boss 40 x 20 stands 8 above a step all round it
    // (the step's floor at Z -9, 10 wide, with the stock's millimetre beyond it). Round the
    // boss down to the floor with the 6 mm tool: one pass reaches 6 from the boss and leaves
    // most of the step standing.
    let stepped = |x: f64, y: f64| {
        if x.abs() < 20.0 && y.abs() < 10.0 {
            -1.0
        } else if x.abs() < 30.0 && y.abs() < 20.0 {
            -9.0
        } else {
            -22.0
        }
    };
    let boss = [(-20.0, -10.0), (20.0, -10.0), (20.0, 10.0), (-20.0, 10.0)];
    let (sim, report) = cut_with(&ContourOp { bottom: Some(-9.0), ..ContourOp::default() }, &mill6(), &block(), &boss, &stepped);
    assert_eq!(report.gouges, 0);
    assert!(sim.leftover().0 > 800.0 + 5_000.0, "{}", sim.leftover().0);
    // Four passes, 2.4 apart (four tenths of the tool), reach 13.2: past the step's sides (11
    // with the stock) and all but the tips of its corners (14.1 from the boss's). Nothing is
    // cut into; the step is at its floor; what stands above the part is the boss's own stock,
    // 40 x 20 and 1 thick, and those four tips, under a square millimetre each and 9 high.
    let op = ContourOp { bottom: Some(-9.0), passes: Some(4), ..ContourOp::default() };
    let (sim, report) = cut_with(&op, &mill6(), &block(), &boss, &stepped);
    assert_eq!((report.gouges, report.rapid_hits), (0, 0));
    let over = sim.leftover().0;
    assert!(over > 800.0 && over < 800.0 + 4.0 * 1.0 * 9.0, "{over}");
    let f = sim.stock.clone();
    let height = |x: f64, y: f64| f.at(((x - f.x) / f.dx - 0.5).round() as usize, ((y - f.y) / f.dy - 0.5).round() as usize).unwrap();
    assert_eq!((height(25.125, 0.125), height(-29.875, 0.125), height(0.125, 14.875), height(0.125, 0.125)), (-9.0, -9.0, -9.0, 0.0));
    assert_eq!(height(29.875, 19.875), 0.0, "the tip of a corner");
    // Six passes reach 18: the corners too.
    let op = ContourOp { bottom: Some(-9.0), passes: Some(6), ..ContourOp::default() };
    let (sim, report) = cut_with(&op, &mill6(), &block(), &boss, &stepped);
    assert!(report.gouges == 0 && close(sim.leftover().0, 800.0), "{}", sim.leftover().0);
}

#[test]
fn passes_that_cannot_be_are_refused_with_the_reason() {
    let refused = |op: ContourOp| contoured(&op).unwrap_err().to_string();
    assert!(refused(ContourOp { passes: Some(0), ..ContourOp::default() }).contains("the number of passes must be 1 to 100"));
    assert!(refused(ContourOp { passes: Some(101), ..ContourOp::default() }).contains("the number of passes must be 1 to 100"));
    // Passes further apart than the tool is wide would leave a wall between them.
    let apart = refused(ContourOp { passes: Some(2), stepover: Some(20.5), ..ContourOp::default() });
    assert!(apart.contains("the stepover must be 0.001 mm to the tool's diameter (20 mm)"), "{apart}");
    assert!(contoured(&ContourOp { passes: Some(2), stepover: Some(20.0), ..ContourOp::default() }).is_ok());
    assert!(refused(ContourOp { stepover: Some(4.0), ..ContourOp::default() }).contains("give the number of passes, two or more, with it"));
    assert!(refused(ContourOp { passes: Some(1), stepover: Some(4.0), ..ContourOp::default() }).contains("give the number of passes"));
    // Tabs go with one pass; inside, passes are Pocket's work.
    let tabs = Some(Tabs { count: 4, width: None, height: None });
    assert!(refused(ContourOp { passes: Some(2), tabs, ..ContourOp::default() }).contains("tabs go with one pass"));
    assert!(contoured(&ContourOp { passes: Some(1), tabs, ..ContourOp::default() }).is_ok());
    let inside = ContourOp { side: ContourSide::Inside, openings: vec![[0.0, 0.0]], passes: Some(2), ..ContourOp::default() };
    assert!(refused(inside).contains("passes sideways are for the outside"));
    // Kept by name, and none unless given.
    let op: ContourOp = serde_json::from_str(r#"{"passes": 3, "stepover": 5}"#).unwrap();
    assert_eq!((op.passes, op.stepover, ContourOp::default().passes, ContourOp::default().stepover), (Some(3), Some(5.0), None, None));
    assert_eq!(serde_json::from_value::<ContourOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
    // No number makes it panic.
    for v in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1.0, 0.0, 1e300, 5e-324] {
        let _ = contoured(&ContourOp { passes: Some(3), stepover: Some(v), ..ContourOp::default() });
    }
    for n in [0, 1, 2, 100, 101, usize::MAX] {
        let _ = contoured(&ContourOp { passes: Some(n), stepover: Some(1e-3), ..ContourOp::default() });
    }
}
