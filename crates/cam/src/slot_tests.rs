use std::f64::consts::{FRAC_PI_2, PI};

use solidcore_record::Loop2;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};
use crate::toolpath::ArcDirection;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// Tool 2: a 3-flute flat end mill `diameter` across with 22 mm of flutes and data for aluminium
/// (150 m/min, 0.05 mm a tooth, plunging and ramping at half): 0.15 mm a revolution.
fn mill(diameter: f64) -> Tool {
    Tool {
        number: 2,
        name: format!("{diameter} mm flat end mill"),
        kind: ToolKind::FlatEndMill,
        geometry: ToolGeometry {
            diameter,
            corner_radius: 0.0,
            flute_length: 22.0,
            shoulder_length: 0.0,
            overall_length: 72.0,
            shank_diameter: diameter,
            tip_angle: 0.0,
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 3,
        holder: Holder { stick_out: 32.0, segments: vec![] },
        length_offset: 2,
        diameter_offset: 2,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 150.0, chip_load: 0.05, plunge_factor: 0.5, ramp_factor: 0.5 }],
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

/// The outline of a straight slot `width` wide between the centres of its ends, `a` and `b`,
/// counter-clockwise: the side on the right of the way from `a` to `b`, the end at `b`, the
/// other side, the end at `a`.
fn straight(a: (f64, f64), b: (f64, f64), width: f64) -> Loop2 {
    let r = 0.5 * width;
    let length = (b.0 - a.0).hypot(b.1 - a.1);
    let (ux, uy) = ((b.0 - a.0) / length, (b.1 - a.1) / length);
    let (nx, ny) = (-uy * r, ux * r);
    let way = uy.atan2(ux);
    Loop2 {
        segs: vec![
            Seg2::Line { a: q(a.0 - nx, a.1 - ny), b: q(b.0 - nx, b.1 - ny) },
            Seg2::Arc { center: q(b.0, b.1), radius: r, start: way - FRAC_PI_2, sweep: PI },
            Seg2::Line { a: q(b.0 + nx, b.1 + ny), b: q(a.0 + nx, a.1 + ny) },
            Seg2::Arc { center: q(a.0, a.1), radius: r, start: way + FRAC_PI_2, sweep: PI },
        ],
    }
}

/// The outline of a slot `width` wide round `centre`: its middle line `radius` from it, from the
/// angle `from` on by `turn`, counter-clockwise: the outer side, the far end, the inner side,
/// the near end.
fn curved(centre: (f64, f64), radius: f64, from: f64, turn: f64, width: f64) -> Loop2 {
    let r = 0.5 * width;
    let c = q(centre.0, centre.1);
    let end = |angle: f64| q(centre.0 + radius * angle.cos(), centre.1 + radius * angle.sin());
    Loop2 {
        segs: vec![
            Seg2::Arc { center: c, radius: radius + r, start: from, sweep: turn },
            Seg2::Arc { center: end(from + turn), radius: r, start: from + turn, sweep: PI },
            Seg2::Arc { center: c, radius: radius - r, start: from + turn, sweep: -turn },
            Seg2::Arc { center: end(from), radius: r, start: from + PI, sweep: PI },
        ],
    }
}

/// A slot whose floor is at Z -7 (6 mm into the block) with the outline `outline`.
fn slot(outline: Loop2) -> SlotShape {
    SlotShape { bottom: -7.0, region: Region2 { outer: outline, holes: vec![] } }
}

fn one(at: [f64; 2]) -> SlotOp {
    SlotOp { slots: vec![at], ..SlotOp::default() }
}

fn slotted(op: &SlotOp, tool: &Tool, shapes: &[SlotShape]) -> Result<Slotted> {
    op.toolpath(None, tool, &placed(), Some("aluminium"), shapes)
}

/// Each half of every line and of every arc of `outline` as a piece of its own, the loop begun
/// at its piece `first`: what a model's section may give for the same outline.
fn in_pieces(outline: &Loop2, first: usize) -> Loop2 {
    let mut segs: Vec<Seg2> = Vec::new();
    for seg in &outline.segs {
        match *seg {
            Seg2::Line { a, b } => {
                let mid = q(0.5 * (a.x + b.x), 0.5 * (a.y + b.y));
                segs.extend([Seg2::Line { a, b: mid }, Seg2::Line { a: mid, b }]);
            }
            Seg2::Arc { center, radius, start, sweep } => {
                segs.extend([
                    Seg2::Arc { center, radius, start, sweep: 0.5 * sweep },
                    Seg2::Arc { center, radius, start: start + 0.5 * sweep, sweep: 0.5 * sweep },
                ]);
            }
            other => segs.push(other),
        }
    }
    segs.rotate_left(first);
    Loop2 { segs }
}

/// The two ends of a middle line, the lower-left first.
fn ends(m: &SlotLine) -> [(f64, f64); 2] {
    let (a, b) = (m.line.start(), m.line.end());
    let mut ends = [(a.x, a.y), (b.x, b.y)];
    ends.sort_by(|a, b| a.partial_cmp(b).unwrap());
    ends
}

#[test]
fn a_slot_is_ramped_down_to_and_fro_along_its_middle_and_cleared_at_each_level() {
    // A slot 10 wide from (0, 0) to (40, 0), 7 below the stock's top, with the 10 mm tool: two
    // levels of 3.5 (half the tool is the most a level takes). At 3 degrees the tool falls
    // 40 x tan 3 = 2.096 in one length of the slot, so each level takes two lengths, 1.75 down
    // in each, and then one more at the level to cut away what the last one left.
    let d = slotted(&one([20.0, 0.0]), &mill(10.0), &[slot(straight((0.0, 0.0), (40.0, 0.0), 10.0))]).unwrap();
    let rpm = 15_000.0 / PI;
    assert!(close(d.rpm, rpm) && close(d.feed, rpm * 0.15) && close(d.ramp_feed, rpm * 0.075) && close(d.plunge_feed, rpm * 0.075), "{d:?}");
    assert_eq!((d.top, d.levels.clone(), d.stepdown, d.legs.clone()), (0.0, vec![vec![-3.5, -7.0]], 5.0, vec![2]));
    assert_eq!((d.widths.clone(), d.lengths.clone(), d.left.clone(), d.notes.clone()), (vec![10.0], vec![40.0], vec![0.0], vec![]));
    let (ramp, feed) = (d.ramp_feed, d.feed);
    let down = |x: f64, z: f64| Move::Linear { to: p(x, 0.0, z), feed: ramp };
    assert_eq!(d.toolpath.start, p(0.0, 0.0, 25.0));
    assert_eq!(
        d.toolpath.moves,
        vec![
            Move::ToolChange { tool: 2 },
            Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            // From the end nearer the front, then the left: above it, down to the stock's top.
            Move::Rapid { to: p(0.0, 0.0, 25.0) },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::Linear { to: p(0.0, 0.0, 0.0), feed: d.plunge_feed },
            // The first level: there and back down the ramp, and once along it at the level.
            down(40.0, -1.75),
            down(0.0, -3.5),
            Move::Linear { to: p(40.0, 0.0, -3.5), feed },
            // The second, from the end the tool is at.
            down(0.0, -5.25),
            down(40.0, -7.0),
            Move::Linear { to: p(0.0, 0.0, -7.0), feed },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::Rapid { to: p(0.0, 0.0, 25.0) },
            Move::Coolant { mode: Coolant::Off },
            Move::SpindleStop,
        ]
    );
    // 5 down to the top, four lengths of the ramp and two at the levels.
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 5.0 + 4.0 * 40.0_f64.hypot(1.75) + 80.0), "{s:?}");

    // A steeper ramp takes one length a level, and a level then ends at the slot's other end:
    // 10 degrees falls 7.05 in 40. With one level (a stepdown of 7) the tool goes there down
    // the ramp, comes back at the bottom, and leaves from where it began.
    let steep = SlotOp { ramp_angle: Some(10.0_f64.to_radians()), stepdown: Some(7.0), ..one([20.0, 0.0]) };
    let d = slotted(&steep, &mill(10.0), &[slot(straight((0.0, 0.0), (40.0, 0.0), 10.0))]).unwrap();
    assert_eq!((d.levels.clone(), d.legs.clone()), (vec![vec![-7.0]], vec![1]));
    assert_eq!(
        d.toolpath.moves[5..9],
        [
            Move::Linear { to: p(0.0, 0.0, 0.0), feed: d.plunge_feed },
            Move::Linear { to: p(40.0, 0.0, -7.0), feed: d.ramp_feed },
            Move::Linear { to: p(0.0, 0.0, -7.0), feed: d.feed },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
        ]
    );
    // A bottom given is cut to, whatever the model's: 2 below the top is one level, and at 3
    // degrees one length of the slot gets there.
    let shallow = SlotOp { bottom: Some(-2.0), ..one([20.0, 0.0]) };
    let d = slotted(&shallow, &mill(10.0), &[slot(straight((0.0, 0.0), (40.0, 0.0), 10.0))]).unwrap();
    assert_eq!((d.levels.clone(), d.legs.clone()), (vec![vec![-2.0]], vec![1]));
}

#[test]
fn a_slot_round_a_centre_is_cut_along_its_arc() {
    // A slot 8 wide whose middle is a quarter of a circle of radius 30 about the origin, from
    // (30, 0) round to (0, 30): 15 pi long. The 8 mm tool: two levels of 3.5, each two lengths
    // of the arc down (15 pi x tan 3 = 2.47 a length) and one at the level.
    let tool = mill(8.0);
    let d = slotted(&one([21.0, 21.0]), &tool, &[slot(curved((0.0, 0.0), 30.0, 0.0, FRAC_PI_2, 8.0))]).unwrap();
    assert!(close(d.lengths[0], 15.0 * PI) && close(d.widths[0], 8.0) && d.left[0].abs() < 1e-9, "{d:?}");
    assert_eq!((d.levels.clone(), d.stepdown, d.legs.clone()), (vec![vec![-3.5, -7.0]], 4.0, vec![2]));
    // Each length is one arc about the origin, falling as it goes: to (0, 30) counter-clockwise
    // and back to (30, 0) clockwise.
    let arcs: Vec<(f64, f64, f64, ArcDirection, f64)> = d
        .toolpath
        .moves
        .iter()
        .filter_map(|m| match *m {
            Move::Arc { to, center, direction, feed, .. } => {
                assert!(center.x.abs() < 1e-12 && center.y.abs() < 1e-12 && center.z == to.z, "{m:?}");
                Some((to.x, to.y, to.z, direction, feed))
            }
            _ => None,
        })
        .collect();
    let (ccw, cw) = (ArcDirection::CounterClockwise, ArcDirection::Clockwise);
    let expected = [
        (0.0, 30.0, -1.75, ccw, d.ramp_feed),
        (30.0, 0.0, -3.5, cw, d.ramp_feed),
        (0.0, 30.0, -3.5, ccw, d.feed),
        (30.0, 0.0, -5.25, cw, d.ramp_feed),
        (0.0, 30.0, -7.0, ccw, d.ramp_feed),
        (30.0, 0.0, -7.0, cw, d.feed),
    ];
    assert_eq!(arcs.len(), expected.len(), "{arcs:?}");
    for (got, want) in arcs.iter().zip(&expected) {
        assert!(
            (got.0 - want.0).abs() < 1e-9 && (got.1 - want.1).abs() < 1e-9 && close(got.2, want.2) && got.3 == want.3 && got.4 == want.4,
            "{got:?} for {want:?}"
        );
    }
    // It starts above the end nearer the front, and nothing but those arcs cuts below the top.
    assert_eq!(d.toolpath.moves[3], Move::Rapid { to: p(30.0, 0.0, 25.0) });
    let s = d.toolpath.stats().unwrap();
    assert!(close(s.feed_length, 5.0 + 4.0 * (15.0 * PI).hypot(1.75) + 2.0 * 15.0 * PI), "{s:?}");
}

#[test]
fn a_slot_cut_with_a_tool_as_wide_is_cut_to_size_and_a_narrower_tool_leaves_its_walls() {
    // The block with the slot of the first test in it, simulated on cells of a quarter: the
    // part's height is the slot's floor where a cell's centre is within 5 of the middle line.
    let outline = straight((0.0, 0.0), (40.0, 0.0), 10.0);
    let placement = placed();
    let in_slot = |x: f64, y: f64| (x - x.clamp(0.0, 40.0)).hypot(y) <= 5.0;
    let cut_with = |tool: &Tool| {
        let d = slotted(&one([20.0, 0.0]), tool, &[slot(outline.clone())]).unwrap();
        let sim = crate::sim::Simulation::new(placement.stock, 0.25, 0.01).unwrap();
        let f = sim.stock.clone();
        let centres: Vec<Vec2> = (0..f.nx * f.ny).map(|k| f.centre(k % f.nx, k / f.nx)).collect();
        let cells_in = centres.iter().filter(|c| in_slot(c.x, c.y)).count();
        let heights = centres.iter().map(|c| if in_slot(c.x, c.y) { -7.0 } else { -1.0 }).collect();
        let mut sim = sim.with_part(heights).unwrap();
        let report = sim.cut(0, &d.toolpath, tool).unwrap();
        (sim, report, d, cells_in, f.nx * f.ny)
    };
    // The 10 mm tool: nothing of the part is cut into, and all that stands above the part
    // afterwards is the stock's millimetre on the block's top outside the slot. What was
    // removed is the slot, 7 deep: (40 x 10 + 25 pi) x 7 = 3349.8, as near as the cells are
    // (3353.0 on these).
    let (sim, report, d, cells_in, cells) = cut_with(&mill(10.0));
    assert_eq!((report.gouges, report.rapid_hits), (0, 0));
    let (over, beside) = sim.leftover();
    assert!(close(over, (cells - cells_in) as f64 * 0.0625) && beside == 0.0, "{over} over the part, {beside} beside it");
    assert!(close(report.removed, cells_in as f64 * 0.0625 * 7.0), "{}", report.removed);
    assert!((report.removed - (400.0 + 25.0 * PI) * 7.0).abs() < 4.0, "{}", report.removed);
    assert!(d.notes.is_empty(), "{:?}", d.notes);
    // The 8 mm tool leaves a millimetre on each wall and round each end, 7 high, and says so:
    // the slot less one 8 wide, (40 x 2 + 9 pi) x 7 = 757.9 (757.75 on these cells).
    let (sim, report, d, _, _) = cut_with(&mill(8.0));
    assert_eq!((report.gouges, report.rapid_hits), (0, 0));
    let walls = sim.leftover().0 - (cells - cells_in) as f64 * 0.0625;
    assert!((walls - (80.0 + 9.0 * PI) * 7.0).abs() < 1.0, "{walls}");
    assert_eq!(d.left, vec![1.0]);
    assert_eq!(
        d.notes,
        vec!["tool 2 (8 mm across) is narrower than the slot at X 20 Y 0 (10.000 mm wide): 1.000 mm stays on each wall, for an inside contour to finish".to_string()]
    );
}

#[test]
fn a_slots_middle_is_read_from_its_outline_however_the_outline_is_given() {
    // Straight, at an angle: from (10, 10) to (40, 50), 50 long, 12 wide.
    let outline = straight((10.0, 10.0), (40.0, 50.0), 12.0);
    let region = |outer: Loop2| Region2 { outer, holes: vec![] };
    let m = slot_line(&region(outline.clone())).unwrap();
    assert!(matches!(m.line, Seg2::Line { .. }) && close(m.width, 12.0) && close(m.line.length(), 50.0), "{m:?}");
    assert_eq!(ends(&m), [(10.0, 10.0), (40.0, 50.0)]);
    // The same outline in halves of every side, begun anywhere, and the other way round.
    for first in 0..8 {
        let pieces = in_pieces(&outline, first);
        for given in [pieces.clone(), pieces.reversed()] {
            let again = slot_line(&region(given)).unwrap();
            assert!(close(again.width, 12.0) && close(again.line.length(), 50.0), "{first}: {again:?}");
            let [a, b] = ends(&again);
            assert!(
                (a.0 - 10.0).abs() < 1e-9 && (a.1 - 10.0).abs() < 1e-9 && (b.0 - 40.0).abs() < 1e-9 && (b.1 - 50.0).abs() < 1e-9,
                "{first}: {again:?}"
            );
        }
    }
    // Round a centre: three quarters of a turn about (5, -5) at radius 20, 6 wide.
    let round = curved((5.0, -5.0), 20.0, 0.25, 1.5 * PI, 6.0);
    for given in [round.clone(), in_pieces(&round, 3), in_pieces(&round, 5).reversed()] {
        let m = slot_line(&region(given)).unwrap();
        let Seg2::Arc { center, radius, start, sweep } = m.line else { panic!("{m:?}") };
        assert!(center.x == 5.0 && center.y == -5.0 && close(radius, 20.0) && close(m.width, 6.0), "{m:?}");
        assert!((start - 0.25).abs() < 1e-9 && (sweep - 1.5 * PI).abs() < 1e-9, "{m:?}");
    }

    // What is not a slot is refused with what it is.
    let why = |outer: Loop2, holes: Vec<Loop2>| slot_line(&Region2 { outer, holes }).unwrap_err();
    assert_eq!(why(Loop2::circle(q(0.0, 0.0), 5.0), vec![]), "it is round: drill it or bore it");
    let half = |start: f64| Seg2::Arc { center: q(0.0, 0.0), radius: 5.0, start, sweep: PI };
    assert_eq!(why(Loop2 { segs: vec![half(0.0), half(PI)] }, vec![]), "it is round: drill it or bore it");
    assert_eq!(why(outline.clone(), vec![Loop2::circle(q(25.0, 30.0), 1.0).reversed()]), "something stands in it: clear it with Pocket");
    let square = |x0: f64, y0: f64, x1: f64, y1: f64| {
        let c = [q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)];
        Loop2 { segs: (0..4).map(|i| Seg2::Line { a: c[i], b: c[(i + 1) % 4] }).collect() }
    };
    assert!(why(square(0.0, 0.0, 40.0, 10.0), vec![]).starts_with("it is not a slot"));
    // A pocket with rounded corners: its ends are not half circles as wide as it is.
    let corner = |x: f64, y: f64, start: f64| Seg2::Arc { center: q(x, y), radius: 2.0, start, sweep: FRAC_PI_2 };
    let rounded = Loop2 {
        segs: vec![
            Seg2::Line { a: q(2.0, 0.0), b: q(38.0, 0.0) },
            corner(38.0, 2.0, -FRAC_PI_2),
            Seg2::Line { a: q(40.0, 2.0), b: q(40.0, 8.0) },
            corner(38.0, 8.0, 0.0),
            Seg2::Line { a: q(38.0, 10.0), b: q(2.0, 10.0) },
            corner(2.0, 8.0, FRAC_PI_2),
            Seg2::Line { a: q(0.0, 8.0), b: q(0.0, 2.0) },
            corner(2.0, 2.0, PI),
        ],
    };
    assert!(why(rounded, vec![]).starts_with("it is not a slot"));
    // A keyhole: ends of two sizes, their sides not parallel.
    let mut keyhole = straight((0.0, 0.0), (40.0, 0.0), 10.0);
    keyhole.segs[1] = Seg2::Arc { center: q(40.0, 0.0), radius: 5.5, start: -FRAC_PI_2, sweep: PI };
    assert!(why(keyhole, vec![]).starts_with("it is not a slot"));
    // Half a disc: one half-round end and a straight side.
    let d_shape = Loop2 { segs: vec![half(-FRAC_PI_2), Seg2::Line { a: q(0.0, 5.0), b: q(0.0, -5.0) }] };
    assert!(why(d_shape, vec![]).starts_with("it is not a slot"));
    // Sides that are arcs about two centres.
    let mut bent = curved((0.0, 0.0), 30.0, 0.0, FRAC_PI_2, 8.0);
    bent.segs[2] = Seg2::Arc { center: q(0.5, 0.0), radius: 26.0, start: FRAC_PI_2, sweep: -FRAC_PI_2 };
    assert!(why(bent, vec![]).starts_with("it is not a slot"));
}

#[test]
fn slots_are_cut_one_after_another_each_from_its_end_nearer_the_tool() {
    // Two slots 10 wide and 7 deep: the first as before; the second 20 behind it, its outline
    // given from its far end. Two levels of three lengths each leave the tool where it began,
    // at (0, 0): the second slot is begun from (0, 20), whichever way its outline goes.
    let shapes = [slot(straight((0.0, 0.0), (40.0, 0.0), 10.0)), slot(straight((40.0, 20.0), (0.0, 20.0), 10.0))];
    let op = SlotOp { slots: vec![[20.0, 0.0], [20.0, 20.0]], ..SlotOp::default() };
    let d = slotted(&op, &mill(10.0), &shapes).unwrap();
    assert_eq!((d.levels.len(), d.legs.clone(), d.lengths.clone()), (2, vec![2, 2], vec![40.0, 40.0]));
    let rapids: Vec<Vec3> = d.toolpath.moves.iter().filter_map(|m| if let Move::Rapid { to } = *m { Some(to) } else { None }).collect();
    assert_eq!(rapids, vec![p(0.0, 0.0, 25.0), p(0.0, 0.0, 5.0), p(0.0, 0.0, 5.0), p(0.0, 20.0, 5.0), p(0.0, 20.0, 5.0), p(0.0, 20.0, 25.0)]);
    // One level of one length down and one back... is two lengths: still where it began. With
    // one level of two lengths down and one at the level the tool ends at the far end, (40, 0),
    // and the second slot is begun from (40, 20).
    let once = SlotOp { bottom: Some(-3.5), ..op.clone() };
    let d = slotted(&once, &mill(10.0), &shapes).unwrap();
    assert_eq!((d.levels.clone(), d.legs.clone()), (vec![vec![-3.5], vec![-3.5]], vec![2, 2]));
    let rapids: Vec<Vec3> = d.toolpath.moves.iter().filter_map(|m| if let Move::Rapid { to } = *m { Some(to) } else { None }).collect();
    assert_eq!(rapids, vec![p(0.0, 0.0, 25.0), p(0.0, 0.0, 5.0), p(40.0, 0.0, 5.0), p(40.0, 20.0, 5.0), p(0.0, 20.0, 5.0), p(0.0, 20.0, 25.0)]);
    // In a program it starts from where the tool is.
    let from = op.toolpath(Some(p(7.0, 8.0, 25.0)), &mill(10.0), &placed(), Some("aluminium"), &shapes).unwrap();
    assert_eq!((from.toolpath.start, from.toolpath.moves), (p(7.0, 8.0, 25.0), slotted(&op, &mill(10.0), &shapes).unwrap().toolpath.moves));
}

#[test]
fn slots_that_cannot_be_cut_are_refused_with_the_reason() {
    let shape = || slot(straight((0.0, 0.0), (40.0, 0.0), 10.0));
    let refused = |op: &SlotOp, tool: &Tool, shapes: &[SlotShape]| slotted(op, tool, shapes).unwrap_err().to_string();
    let at = one([20.0, 0.0]);
    // A tool wider than the slot, by anything a machine could show.
    assert_eq!(
        refused(&at, &mill(10.01), &[shape()]),
        "tool 2 (10.01 mm across) is wider than the slot at X 20 Y 0, which is 10.000 mm wide: use a narrower tool"
    );
    // A point given with a zero that carries a minus sign is said without it.
    assert!(refused(&one([-0.0, -0.0]), &mill(10.01), &[shape()]).contains("the slot at X 0 Y 0, which"));
    // What is not a slot, by its point.
    let pocket = SlotShape { bottom: -7.0, region: Region2 { outer: Loop2::circle(q(20.0, 0.0), 8.0), holes: vec![] } };
    assert_eq!(refused(&at, &mill(10.0), &[pocket]), "the slot at X 20 Y 0: it is round: drill it or bore it");
    // Heights: a bottom that is not below the top, a slot deeper than the tool reaches, levels
    // deeper than its flutes.
    assert!(
        refused(&SlotOp { bottom: Some(0.0), ..at.clone() }, &mill(10.0), &[shape()]).contains("its bottom (Z 0 mm) is not below the top (Z 0 mm)")
    );
    assert!(
        refused(&SlotOp { bottom: Some(-30.0), ..at.clone() }, &mill(10.0), &[shape()])
            .contains("is 30 mm deep, more than the 22 mm tool 2 reaches down a wall")
    );
    let mut short = mill(10.0);
    (short.geometry.flute_length, short.geometry.shoulder_length) = (3.0, 22.0);
    assert!(refused(&at, &short, &[shape()]).contains("each level is 3.5 mm deep, past tool 2's 3 mm of flutes"));
    assert!(
        refused(&SlotOp { top: Some(6.0), ..at.clone() }, &mill(10.0), &[shape()])
            .contains("the retract height (5 mm) must be above the slot's top (6 mm)")
    );
    // A slot hardly longer than it is wide: the ramp would go along it more than 500 times.
    let stub = slot(straight((0.0, 0.0), (0.1, 0.0), 10.0));
    let e = refused(&one([0.0, 0.0]), &mill(10.0), &[stub]);
    assert!(
        e.contains("is 0.100 mm from the centre of one end to the other's, so the ramp down 3.5 mm would go along it 668 times: use a steeper ramp"),
        "{e}"
    );
    // The points and the slots read for them must agree in number.
    assert!(refused(&at, &mill(10.0), &[shape(), shape()]).contains("2 slots were read from the model for the operation's 1 points"));
    // The parameters, each by name, and the tools that do not cut a slot.
    for (op, why) in [
        (SlotOp::default(), "a slot operation cuts 1 to 1000 slots, not 0"),
        (SlotOp { slots: vec![[0.0, 0.0]; 1001], ..SlotOp::default() }, "a slot operation cuts 1 to 1000 slots, not 1001"),
        (one([f64::NAN, 0.0]), "the points in the slots must be finite"),
        (one([0.0, f64::INFINITY]), "the points in the slots must be finite"),
        (SlotOp { top: Some(f64::NAN), ..at.clone() }, "the top must be finite"),
        (SlotOp { bottom: Some(f64::NEG_INFINITY), ..at.clone() }, "the bottom must be finite"),
        (SlotOp { stepdown: Some(0.0), ..at.clone() }, "the stepdown must be at least 0.001 mm"),
        (SlotOp { stepdown: Some(f64::NAN), ..at.clone() }, "the stepdown must be at least 0.001 mm"),
        (SlotOp { ramp_angle: Some(0.0), ..at.clone() }, "the ramp angle must be 0.5 to 30 degrees"),
        (SlotOp { ramp_angle: Some(1.0), ..at.clone() }, "the ramp angle must be 0.5 to 30 degrees"),
        (SlotOp { rpm: Some(0.0), ..at.clone() }, "rpm must be above 0"),
        (SlotOp { feed: Some(-1.0), ..at.clone() }, "feed must be above 0"),
    ] {
        let e = refused(&op, &mill(10.0), &[shape()]);
        assert!(e.contains(why), "{op:?}: {e:?} does not mention {why:?}");
    }
    let mut drill = mill(10.0);
    drill.kind = ToolKind::Drill;
    assert_eq!(refused(&at, &drill, &[shape()]), "tool 2 (Drill) does not cut a slot: use a flat or bull-nose end mill");
    // Outlines no model gives never panic: no sides, sides of no length, numbers that are none.
    let nan = f64::NAN;
    for outer in [
        Loop2 { segs: vec![] },
        Loop2 { segs: vec![Seg2::Line { a: q(0.0, 0.0), b: q(0.0, 0.0) }; 4] },
        Loop2 { segs: vec![Seg2::Arc { center: q(nan, nan), radius: nan, start: nan, sweep: nan }; 4] },
        straight((0.0, 0.0), (0.0, 0.0), 10.0),
        straight((0.0, 0.0), (40.0, 0.0), 0.0),
        curved((0.0, 0.0), 30.0, 0.0, 0.0, 8.0),
        curved((0.0, 0.0), 3.0, 0.0, FRAC_PI_2, 8.0),
        curved((0.0, 0.0), 1e300, 0.0, FRAC_PI_2, 8.0),
    ] {
        let hostile = SlotShape { bottom: -7.0, region: Region2 { outer, holes: vec![] } };
        assert!(slotted(&at, &mill(10.0), &[hostile]).is_err());
    }
}

#[test]
fn a_slot_operation_is_plain_json() {
    let op = SlotOp { slots: vec![[20.0, 0.0]], bottom: Some(-7.5), ramp_angle: Some(0.1), ..SlotOp::default() };
    let json = serde_json::to_value(&op).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"slots": [[20.0, 0.0]], "top": null, "bottom": -7.5, "stepdown": null, "ramp_angle": 0.1, "rpm": null, "feed": null})
    );
    assert_eq!(serde_json::from_value::<SlotOp>(json).unwrap(), op);
    // Only the points are needed, and a word it does not know is refused.
    assert_eq!(serde_json::from_str::<SlotOp>(r#"{"slots": [[1, 2]]}"#).unwrap(), one([1.0, 2.0]));
    assert!(serde_json::from_str::<SlotOp>(r#"{"slots": [[1, 2]], "stepover": 3}"#).is_err());
}
