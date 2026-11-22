use std::f64::consts::PI;

use super::*;
use crate::cycle::DrillCycle;
use crate::tool::{Holder, HolderSegment, Overrides, ToolGeometry};
use crate::toolpath::{ArcDirection, Coolant, Plane};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

/// A tool of `kind` and `diameter` (mm) with a corner radius and a tip angle (degrees) where its
/// kind has them.
fn tool(kind: ToolKind, diameter: f64, corner_radius: f64, tip_degrees: f64) -> Tool {
    Tool {
        number: 1,
        name: "test tool".into(),
        kind,
        geometry: ToolGeometry {
            diameter,
            corner_radius,
            flute_length: 30.0,
            shoulder_length: 0.0,
            overall_length: 80.0,
            shank_diameter: diameter,
            tip_angle: tip_degrees.to_radians(),
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 2,
        holder: Holder { stick_out: 40.0, segments: vec![] },
        length_offset: 1,
        diameter_offset: 1,
        coolant: Coolant::Off,
        cutting_data: vec![],
        overrides: Overrides::default(),
    }
}

fn flat10() -> Tool {
    tool(ToolKind::FlatEndMill, 10.0, 0.0, 0.0)
}

/// A stock 100 x 60 x 20 with its top at Z 0, in cells of a quarter of a millimetre, checked to
/// 0.01 mm.
fn stock() -> Simulation {
    Simulation::new(Bounds { min: p(0.0, 0.0, -20.0), max: p(100.0, 60.0, 0.0) }, 0.25, 0.01).unwrap()
}

fn path(start: Vec3, moves: Vec<Move>) -> Toolpath {
    Toolpath { start, moves }
}

fn feed(x: f64, y: f64, z: f64) -> Move {
    Move::Linear { to: p(x, y, z), feed: 500.0 }
}

fn rapid(x: f64, y: f64, z: f64) -> Move {
    Move::Rapid { to: p(x, y, z) }
}

/// The height of the cell whose centre is nearest (x, y).
fn height(sim: &Simulation, x: f64, y: f64) -> f64 {
    let f = &sim.stock;
    f.at(((x - f.x) / f.dx - 0.5).round() as usize, ((y - f.y) / f.dy - 0.5).round() as usize).unwrap()
}

/// Whether a measured volume is the true one to within what the grid can tell: half a cell
/// along the cut's outline (`round` mm long), over its depth.
fn about(measured: f64, exact: f64, round: f64, depth: f64, cell: f64) -> bool {
    (measured - exact).abs() <= 0.5 * round * cell * depth
}

#[test]
fn a_slot_removes_its_own_volume_and_no_more() {
    // A 10 mm flat end mill down 5 mm at (20, 30) and 60 mm along X: a slot 10 wide with round
    // ends, (60 x 10 + 25 pi) x 5 = 3392.7 mm3.
    let mut sim = stock();
    let before = sim.stock.volume_above(sim.floor);
    assert_eq!(before, 100.0 * 60.0 * 20.0);
    let cut = sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -5.0), feed(80.0, 30.0, -5.0), rapid(80.0, 30.0, 5.0)]), &flat10()).unwrap();
    let exact = (600.0 + 25.0 * PI) * 5.0;
    assert!(about(cut.removed, exact, 120.0 + 10.0 * PI, 5.0, 0.25), "{} mm3, not {exact}", cut.removed);
    assert!((cut.removed - exact).abs() < 0.005 * exact, "within half a percent on this grid: {} and {exact}", cut.removed);
    // What it says it removed is what is gone.
    assert!((before - sim.stock.volume_above(sim.floor) - cut.removed).abs() < 1e-6 && cut.removed == sim.removed);
    // In the slot the stock is at -5; beside it, untouched.
    assert_eq!((height(&sim, 50.0, 30.0), height(&sim, 50.0, 34.8), height(&sim, 50.0, 35.2), height(&sim, 14.0, 30.0)), (-5.0, -5.0, 0.0, 0.0));
    assert_eq!((cut.gouges, cut.rapid_hits, sim.below_stock), (0, 0, 0.0));
    // Cutting it again removes nothing.
    let again = sim.cut(2, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -5.0), feed(80.0, 30.0, -5.0)]), &flat10()).unwrap();
    assert_eq!(again.removed, 0.0);
}

#[test]
fn a_ramp_leaves_the_slope_the_tools_trailing_edge_cut() {
    // Down 6 mm over 60 mm along X. A flat tool's bottom is level, so over any place the lowest
    // it gets is when its trailing edge leaves: 5 mm further on. Over a cell at X x on the
    // centre line that is -6 (x + 5 - 20) / 60, exactly.
    let mut sim = stock();
    sim.cut(1, &path(p(20.0, 30.0, 0.0), vec![feed(80.0, 30.0, -6.0)]), &flat10()).unwrap();
    for x in [30.125, 50.125, 70.125] {
        let want = -6.0 * (x + 5.0 - 20.0) / 60.0;
        assert!((height(&sim, x, 30.125) - want).abs() < 2e-3, "at X {x}: {} and {want}", height(&sim, x, 30.125));
    }
    // Off the centre line the edge leaves sooner: at 3 mm to the side, 4 mm on.
    let want = -6.0 * (50.125 + (25.0_f64 - 3.0 * 3.0).sqrt() - 20.0) / 60.0;
    assert!((height(&sim, 50.125, 33.125) - want).abs() < 0.02, "{} and {want}", height(&sim, 50.125, 33.125));
    // Past the end the tool's full depth, under its last position.
    assert_eq!(height(&sim, 83.0, 30.0), -6.0);
}

#[test]
fn a_ball_and_a_drill_cut_their_own_shapes() {
    // A 10 mm ball 3 mm deep, 60 mm along X: a groove whose section is the segment of the
    // ball's circle 3 deep, 25 acos(0.4) - 2 sqrt(21) = 19.82 mm2, and at the two ends half a
    // spherical cap each: pi 3^2 (15 - 3) / 3 = 113.1 mm3 together.
    let mut sim = stock();
    let ball = tool(ToolKind::BallEndMill, 10.0, 5.0, 0.0);
    let cut = sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -3.0), feed(80.0, 30.0, -3.0)]), &ball).unwrap();
    let section = 25.0 * 0.4_f64.acos() - 2.0 * 21.0_f64.sqrt();
    let exact = 60.0 * section + PI * 9.0 * 12.0 / 3.0;
    assert!((cut.removed - exact).abs() < 0.01 * exact, "{} mm3, not {exact}", cut.removed);
    // Under the ball at a distance d from its path the groove is 5 - sqrt(25 - d^2) shallower
    // than 3: exactly, over each cell's own centre.
    for d in [0.125_f64, 3.125, 4.375] {
        let want = -3.0 + 5.0 - (25.0 - d * d).sqrt();
        assert!((height(&sim, 50.125, 30.0 + d) - want).abs() < 1e-9, "{d} mm aside: {} and {want}", height(&sim, 50.125, 30.0 + d));
    }
    assert_eq!(height(&sim, 50.125, 34.875), 0.0, "past where the ball reaches down to the top");
    // A 10 mm drill with a 118 degree point, pecked to Z -12 at its tip: a cylinder down to
    // where the point begins and the cone of the point, 5 / tan 59 = 3.004 high.
    let mut sim = stock();
    let drill = tool(ToolKind::Drill, 10.0, 0.0, 118.0);
    let hole = Move::Drill { to: p(50.0, 30.0, -12.0), r: 2.0, retract: 2.0, cycle: DrillCycle::Peck { peck: 3.0 }, feed: 200.0 };
    let cut = sim.cut(1, &path(p(50.0, 30.0, 10.0), vec![hole]), &drill).unwrap();
    let point = 5.0 / 59.0_f64.to_radians().tan();
    let exact = PI * 25.0 * (12.0 - point) + PI * 25.0 * point / 3.0;
    assert!((cut.removed - exact).abs() < 0.01 * exact, "{} mm3, not {exact}", cut.removed);
    assert!((height(&sim, 50.125, 30.125) + 12.0).abs() < 0.2 && (height(&sim, 54.6, 30.125) + 12.0 - point).abs() < 0.35);
    // Its pecks rapid back down the hole it has drilled: no rapid passes through material.
    assert_eq!((cut.rapid_hits, cut.gouges), (0, 0));
    // A tap cuts a thread, which heights can't show: refused by name, as an undercutting tool is.
    let tap = tool(ToolKind::Tap, 6.0, 0.0, 0.0);
    assert!(sim.cut(2, &path(p(50.0, 30.0, 10.0), vec![feed(50.0, 30.0, 5.0)]), &tap).unwrap_err().to_string().contains("cuts a thread"));
    // A bull-nose with a 2 mm corner is flat for 3 mm out and rounds up to 2 at its edge.
    let bull = Cutter::of(&tool(ToolKind::BullNoseEndMill, 10.0, 2.0, 0.0)).unwrap();
    assert_eq!((bull.rise(0.0), bull.rise(3.0), bull.rise(5.0)), (0.0, 0.0, 2.0));
    assert!((bull.rise(4.0) - (2.0 - 3.0_f64.sqrt())).abs() < 1e-12);
}

#[test]
fn an_arc_is_cut_as_the_ring_it_sweeps() {
    // Once round a circle of radius 20 about (50, 30), 4 deep: the ring between radii 15 and
    // 25, pi (625 - 225) x 4 = 5026.5 mm3.
    let mut sim = stock();
    let round =
        Move::Arc { to: p(70.0, 30.0, -4.0), center: p(50.0, 30.0, 0.0), plane: Plane::Xy, direction: ArcDirection::CounterClockwise, feed: 500.0 };
    let cut = sim.cut(1, &path(p(70.0, 30.0, 5.0), vec![feed(70.0, 30.0, -4.0), round]), &flat10()).unwrap();
    let exact = PI * 400.0 * 4.0;
    assert!((cut.removed - exact).abs() < 0.005 * exact, "{} mm3, not {exact}", cut.removed);
    assert_eq!((height(&sim, 50.0, 30.0), height(&sim, 50.0, 50.0), height(&sim, 50.0, 44.0)), (0.0, -4.0, 0.0));
}

/// A part under the stock: a slab whose top is at Z -5 from X 10 to 90 and Y 10 to 50, with a
/// boss up to Z -1 from X 40 to 60 and Y 20 to 40. Nothing elsewhere.
fn with_boss(sim: Simulation) -> Simulation {
    let f = sim.stock.clone();
    let mut part = vec![-100.0; f.nx * f.ny];
    for j in 0..f.ny {
        for i in 0..f.nx {
            let c = f.centre(i, j);
            if c.x > 10.0 && c.x < 90.0 && c.y > 10.0 && c.y < 50.0 {
                part[j * f.nx + i] = if c.x > 40.0 && c.x < 60.0 && c.y > 20.0 && c.y < 40.0 { -1.0 } else { -5.0 };
            }
        }
    }
    sim.with_part(part).unwrap()
}

#[test]
fn a_cut_into_the_part_is_a_gouge_and_one_along_its_wall_is_not() {
    // Facing the stock down to the boss's top touches the part and no more.
    let mut sim = with_boss(stock());
    let mut passes = vec![feed(-6.0, 4.0, -1.0)];
    for (k, y) in [4.0, 12.0, 20.0, 28.0, 36.0, 44.0, 52.0, 60.0].into_iter().enumerate() {
        passes.push(feed(if k % 2 == 0 { -6.0 } else { 106.0 }, y, -1.0));
        passes.push(feed(if k % 2 == 0 { 106.0 } else { -6.0 }, y, -1.0));
    }
    let faced = sim.cut(1, &path(p(-6.0, 4.0, 5.0), passes), &flat10()).unwrap();
    assert_eq!((faced.gouges, sim.gouge_count), (0, 0));
    assert!((faced.removed - 6000.0).abs() < 1e-6, "the whole top, 1 mm deep: {}", faced.removed);
    // Along the slab at Z -5 beside the boss, the tool's edge on the boss's wall: clean.
    let beside = sim.cut(2, &path(p(20.0, 45.0, 5.0), vec![feed(20.0, 45.0, -5.0), feed(80.0, 45.0, -5.0)]), &flat10()).unwrap();
    assert_eq!(beside.gouges, 0);
    // The same cut 1 mm nearer goes 1 mm into the boss, 4 mm below its top: one gouge, in the
    // second move, 4 deep, at a place inside the boss.
    let into = sim.cut(3, &path(p(20.0, 44.0, 5.0), vec![feed(20.0, 44.0, -5.0), feed(80.0, 44.0, -5.0)]), &flat10()).unwrap();
    assert_eq!((into.gouges, sim.gouge_count, sim.gouges.len()), (1, 1, 1));
    let g = sim.gouges[0];
    assert_eq!((g.operation, g.move_index, g.depth, g.at.z), (3, 1, 4.0, -5.0));
    assert!(g.at.x > 40.0 && g.at.x < 60.0 && g.at.y > 39.0 && g.at.y < 40.0, "{g:?}");
    // Straight through the boss: one more, and deeper below the slab there is nothing to cut into.
    let through = sim.cut(4, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -5.0), feed(80.0, 30.0, -5.0)]), &flat10()).unwrap();
    assert_eq!((through.gouges, sim.gouge_count), (1, 2));
    // The stock is below the part only where a cut went into it: in the two gouges' slots, and
    // nowhere along the wall the clean cut ran beside.
    let (field, part) = (sim.stock.clone(), sim.part().unwrap().to_vec());
    let below: Vec<Vec2> =
        (0..field.nx * field.ny).filter(|&k| field.heights[k] < part[k] - 1e-9).map(|k| field.centre(k % field.nx, k / field.nx)).collect();
    assert!(
        !below.is_empty() && below.iter().all(|at| at.x > 40.0 && at.x < 60.0 && at.y < 40.0 && (at.y > 39.0 || (at.y > 25.0 && at.y < 35.0))),
        "{:?}",
        below.first()
    );
    // What stands above the part, and beside it: over the slab (but for the boss and the two
    // slots) 4 mm of stock is left.
    let (over, beside_part) = sim.leftover();
    assert!(over > 4.0 * (80.0 * 40.0 - 400.0) - 2.0 * 4.0 * 700.0 && over < 4.0 * (80.0 * 40.0 - 400.0), "{over}");
    assert!((beside_part - 19.0 * (6000.0 - 3200.0)).abs() < 19.0 * 240.0 * 0.25, "{beside_part}");
}

#[test]
fn a_rapid_through_material_is_a_hit_and_a_cut_below_the_stock_is_measured() {
    let mut sim = stock();
    // Above the stock: fine. Two millimetres into it: a hit, 2 deep, in that move.
    let across =
        sim.cut(1, &path(p(-10.0, 30.0, 5.0), vec![rapid(110.0, 30.0, 5.0), rapid(110.0, 30.0, -2.0), rapid(-10.0, 30.0, -2.0)]), &flat10()).unwrap();
    assert_eq!((across.rapid_hits, sim.rapid_hit_count, across.removed), (1, 1, 0.0));
    let hit = sim.rapid_hits[0];
    assert_eq!((hit.operation, hit.move_index, hit.depth, hit.at.z), (1, 2, 2.0, -2.0));
    // Along a slot already cut to -5, a rapid at -3 passes through air: the slot's walls beside
    // the tool are not in its way.
    let slot = vec![feed(20.0, 10.0, -5.0), feed(80.0, 10.0, -5.0), rapid(80.0, 10.0, -3.0), rapid(20.0, 10.0, -3.0), rapid(20.0, 10.0, 5.0)];
    assert_eq!(sim.cut(2, &path(p(20.0, 10.0, 5.0), slot), &flat10()).unwrap().rapid_hits, 0);
    // Through the stock's bottom (Z -20) to -22: 2 mm below it, and only the stock is removed.
    // Back up at rapid from below the stock, the tool is in the hole it made: no hit.
    let through = sim.cut(3, &path(p(50.0, 50.0, 5.0), vec![feed(50.0, 50.0, -22.0), rapid(50.0, 50.0, 5.0)]), &flat10()).unwrap();
    assert_eq!((sim.below_stock, height(&sim, 50.0, 50.0), through.rapid_hits), (2.0, -20.0, 0));
    assert!((through.removed - 25.0 * PI * 20.0).abs() < 0.01 * 25.0 * PI * 20.0, "{}", through.removed);
    // A rapid below the stock's bottom under material that is still there is in it all the same.
    let under = sim.cut(4, &path(p(50.0, 50.0, -22.0), vec![rapid(70.0, 50.0, -22.0)]), &flat10()).unwrap();
    assert_eq!((under.rapid_hits, sim.rapid_hits.last().map(|hit| hit.depth)), (1, Some(20.0)));
}

#[test]
fn a_drill_through_a_hole_of_the_part_cuts_into_nothing() {
    // A plate as thick as the stock less 1 mm on top, with a hole of 10 through it at (50, 30):
    // the part has no height there, and is marked as below the stock's bottom, as the engine
    // marks it.
    let sim = stock();
    let f = sim.stock.clone();
    let part: Vec<f64> = (0..f.nx * f.ny)
        .map(|k| {
            let c = f.centre(k % f.nx, k / f.nx);
            if (c.x - 50.0).hypot(c.y - 30.0) < 5.0 { -21.0 } else { -1.0 }
        })
        .collect();
    let mut sim = sim.with_part(part).unwrap();
    // A 10 mm drill to Z -24 at its tip: its full diameter a millimetre past the bottom of the
    // stock (the point is 5 / tan 59 = 3.004 long). It takes the hole and nothing of the part,
    // and comes back up the hole at rapid: no gouge, no rapid through material, and the tip
    // 4 mm below the stock, exactly.
    let drill = tool(ToolKind::Drill, 10.0, 0.0, 118.0);
    let hole = |x: f64| Move::Drill { to: p(x, 30.0, -24.0), r: 2.0, retract: 5.0, cycle: DrillCycle::Drill { dwell: 0.0 }, feed: 200.0 };
    let cut = sim.cut(1, &path(p(50.0, 30.0, 10.0), vec![hole(50.0)]), &drill).unwrap();
    assert_eq!((cut.gouges, cut.rapid_hits, sim.below_stock), (0, 0, 4.0));
    assert!((cut.removed - PI * 25.0 * 20.0).abs() < 0.01 * PI * 25.0 * 20.0, "{}", cut.removed);
    assert_eq!(height(&sim, 50.125, 30.125), -20.0);
    // Pecked, it rapids back down its own hole, below the stock's bottom too: still nothing.
    let pecked = Move::Drill { to: p(50.0, 30.0, -24.0), r: 2.0, retract: 5.0, cycle: DrillCycle::Peck { peck: 3.0 }, feed: 200.0 };
    let again = sim.cut(2, &path(p(50.0, 30.0, 10.0), vec![pecked]), &drill).unwrap();
    assert_eq!((again.gouges, again.rapid_hits, again.removed), (0, 0, 0.0));
    // The same hole drilled where the part has none: a cut into the part, from its top at Z -1
    // down to the bottom of the stock, which is as far as there is part to cut into.
    let wrong = sim.cut(3, &path(p(70.0, 30.0, 10.0), vec![hole(70.0)]), &drill).unwrap();
    assert_eq!((wrong.gouges, wrong.rapid_hits), (1, 0));
    assert_eq!(sim.gouges.last().map(|gouge| (gouge.operation, gouge.depth)), Some((3, 19.0)));
}

#[test]
fn what_cannot_be_simulated_is_refused_and_hostile_numbers_never_panic() {
    let block = Bounds { min: p(0.0, 0.0, -20.0), max: p(100.0, 60.0, 0.0) };
    assert!(Simulation::new(block, 0.001, 0.01).unwrap_err().to_string().contains("at least 0.01 mm"));
    assert!(Simulation::new(block, 0.02, 0.01).unwrap_err().to_string().contains("more than 4000000"));
    assert!(Simulation::new(block, 1.0, -1.0).is_err());
    assert!(stock().with_part(vec![0.0; 3]).unwrap_err().to_string().contains("one a cell"));
    let undercut = tool(ToolKind::Dovetail, 10.0, 0.0, 60.0);
    assert!(stock().cut(1, &path(p(0.0, 0.0, 5.0), vec![feed(10.0, 10.0, -1.0)]), &undercut).unwrap_err().to_string().contains("cuts under itself"));
    // A grid covers its box and no more: 100.1 mm in cells of at most 0.25 is 401 of them, each
    // 100.1 / 401 wide; 60 mm is 240 of 0.25.
    let field = HeightField::flat(Vec2 { x: 0.0, y: 0.0 }, Vec2 { x: 100.1, y: 60.0 }, 0.25, 2.0).unwrap();
    assert_eq!((field.nx, field.ny, field.dx, field.dy), (401, 240, 100.1 / 401.0, 0.25));
    assert!((field.volume_above(0.0) - 100.1 * 60.0 * 2.0).abs() < 1e-6);
    assert_eq!(
        (field.columns(0.0, 100.1), field.columns(50.0, 50.01), field.rows(-5.0, 0.2), field.rows(70.0, 80.0)),
        (Some((0, 400)), None, Some((0, 0)), None)
    );
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, f64::MAX];
    for &v in &bad {
        let _ = Simulation::new(Bounds { min: p(v, 0.0, -20.0), max: p(100.0, v, 0.0) }, 0.5, 0.01);
        let _ = Simulation::new(block, v, v);
        let mut sim = Simulation::new(block, 1.0, 0.01).unwrap();
        // Moves and tools of hostile numbers are refused or cut; far outside the stock they cut nothing.
        let _ = sim.cut(1, &path(p(v, v, v), vec![feed(v, 0.0, -1.0), rapid(0.0, v, v)]), &flat10());
        let _ = sim.cut(1, &path(p(0.0, 0.0, 5.0), vec![feed(50.0, 30.0, -1.0)]), &tool(ToolKind::FlatEndMill, v, v, v));
        let _ = sim.cut(1, &path(p(0.0, 0.0, 5.0), vec![feed(50.0, 30.0, -1.0)]), &tool(ToolKind::Drill, 10.0, v, v));
        let far = sim.cut(1, &path(p(5000.0, 5000.0, 5.0), vec![feed(6000.0, 5000.0, -1.0)]), &flat10()).unwrap();
        assert_eq!(far.removed, 0.0);
        assert!(sim.stock.heights.iter().all(|h| h.is_finite() && *h >= -20.0 && *h <= 0.0));
    }
}

#[test]
fn a_part_faced_pocketed_and_contoured_loses_the_stock_that_is_not_part_and_nothing_else() {
    use crate::contour::ContourOp;
    use crate::face::FaceOp;
    use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
    use crate::pocket::{PocketOp, PocketShape};
    use solidcore_record::{Loop2, Region2, Seg2};
    // A plate 100 x 80 x 20 with a pocket 60 x 40, 6 deep, in the middle of its top, in stock
    // 1 mm bigger all round and on top; the WCS over the middle at the stock's top. In the WCS
    // the plate's top is Z -1, the pocket's floor -7, the plate's bottom (and the stock's) -21.
    let setup = Setup {
        id: "setup-1".into(),
        name: "Op 1".into(),
        kind: SetupKind::Milling,
        machine: None,
        material: None,
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
    let placement = setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(100.0, 80.0, 20.0) }).unwrap();
    let rect = |x: f64, y: f64| {
        let q = |x: f64, y: f64| Vec2 { x, y };
        Loop2 {
            segs: vec![
                Seg2::Line { a: q(-x, -y), b: q(x, -y) },
                Seg2::Line { a: q(x, -y), b: q(x, y) },
                Seg2::Line { a: q(x, y), b: q(-x, y) },
                Seg2::Line { a: q(-x, y), b: q(-x, -y) },
            ],
        }
    };
    let (mill20, mut mill10) = (tool(ToolKind::FlatEndMill, 20.0, 0.0, 0.0), flat10());
    mill10.number = 2;
    // The three operations, with a speed and feed given (the tools here have no cutting data).
    let face = FaceOp { rpm: Some(3000.0), feed: Some(600.0), ..FaceOp::default() }.toolpath(None, &mill20, &placement, None).unwrap();
    let pocket = PocketOp { pockets: vec![[0.0, 0.0]], rpm: Some(4000.0), feed: Some(500.0), ..PocketOp::default() };
    let shape = PocketShape { floor: -7.0, region: Region2 { outer: rect(30.0, 20.0), holes: Vec::new() } };
    let pocket = pocket.toolpath(None, &mill10, &placement, None, &[shape]).unwrap();
    let contour = ContourOp { rpm: Some(4000.0), feed: Some(500.0), ..ContourOp::default() }
        .toolpath(None, &mill10, &placement, None, &[rect(50.0, 40.0)])
        .unwrap();

    // The part over the stock's grid: its top, the pocket's floor, nothing beside it.
    let sim = Simulation::new(placement.stock, 0.25, 0.01).unwrap();
    let field = sim.stock.clone();
    let mut part = vec![-100.0; field.nx * field.ny];
    for j in 0..field.ny {
        for i in 0..field.nx {
            let c = field.centre(i, j);
            if c.x.abs() < 50.0 && c.y.abs() < 40.0 {
                part[j * field.nx + i] = if c.x.abs() < 30.0 && c.y.abs() < 20.0 { -7.0 } else { -1.0 };
            }
        }
    }
    let mut sim = sim.with_part(part).unwrap();
    let before = sim.stock.volume_above(sim.floor);
    assert_eq!(before, 102.0 * 82.0 * 21.0);
    let faced = sim.cut(1, &face.toolpath, &mill20).unwrap();
    let pocketed = sim.cut(2, &pocket.toolpath, &mill10).unwrap();
    let contoured = sim.cut(3, &contour.toolpath, &mill10).unwrap();
    // Nothing cut into the part, no rapid through material, nothing below the stock.
    assert_eq!((sim.gouge_count, sim.rapid_hit_count, sim.below_stock), (0, 0, 0.0), "{:?} {:?}", sim.gouges.first(), sim.rapid_hits.first());
    // Facing takes the 1 mm on top; the pocket its 60 x 40 x 6 but for the four corners a
    // 10 mm tool can't reach (each 25 - 25 pi / 4 in area); the contour the 1 mm of stock
    // round the plate, over the 20 mm facing left of it.
    let corners = 4.0 * (25.0 - 25.0 * PI / 4.0);
    assert!((faced.removed - 102.0 * 82.0).abs() < 1e-6, "{}", faced.removed);
    assert!((pocketed.removed - (2400.0 - corners) * 6.0).abs() < 0.5 * 200.0 * 0.25 * 6.0, "{} and {}", pocketed.removed, (2400.0 - corners) * 6.0);
    assert!((contoured.removed - (102.0 * 82.0 - 8000.0) * 20.0).abs() < 0.5 * 360.0 * 0.25 * 20.0, "{}", contoured.removed);
    // The oracle: what was removed is the stock less the part, but for those corners; what
    // still stands above the part is the corners; beside the part nothing is left.
    let (stock_volume, part_volume) = (102.0 * 82.0 * 21.0, 100.0 * 80.0 * 20.0 - 2400.0 * 6.0);
    let removed = faced.removed + pocketed.removed + contoured.removed;
    assert!((removed - sim.removed).abs() < 1e-6);
    assert!(
        (removed - (stock_volume - part_volume - corners * 6.0)).abs() < 0.002 * (stock_volume - part_volume),
        "{removed} and {}",
        stock_volume - part_volume - corners * 6.0
    );
    let (over, beside) = sim.leftover();
    assert!(
        (over - corners * 6.0).abs() < 0.25 * corners * 6.0 && beside < 0.5 * 360.0 * 0.25 * 20.0,
        "{over} over the part (the corners are {}), {beside} beside it",
        corners * 6.0
    );
}

/// The 10 mm flat tool with only 10 mm of flutes.
fn short10() -> Tool {
    let mut t = flat10();
    t.geometry.flute_length = 10.0;
    t
}

#[test]
fn a_cut_deeper_than_the_flutes_puts_the_shank_in_material() {
    // A slot 15 deep in one pass with 10 mm of flutes: 5 mm of the stock above the flutes is in
    // the way of what does not cut, in the plunge and in the slot after it.
    let mut sim = stock();
    let deep = sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -15.0), feed(80.0, 30.0, -15.0)]), &short10()).unwrap();
    assert_eq!((deep.body_hits, sim.body_hit_count, deep.gouges), (2, 2, 0));
    let hit = sim.body_hits[1];
    assert_eq!((hit.operation, hit.move_index, hit.part, hit.depth, hit.at.z), (1, 1, BodyPart::Shank, 5.0, -5.0));
    // The same slot in two passes of 7.5: at the second the stock above the flutes is gone, and
    // the wall beside the tool is the wall, not something in the shank's way.
    let mut sim = stock();
    let passes = vec![feed(20.0, 30.0, -7.5), feed(80.0, 30.0, -7.5), feed(80.0, 30.0, -15.0), feed(20.0, 30.0, -15.0)];
    assert_eq!(sim.cut(1, &path(p(20.0, 30.0, 5.0), passes), &short10()).unwrap().body_hits, 0);
    // With flutes to spare nothing is looked at, and nothing found: the 30 mm tool, 15 deep.
    let mut sim = stock();
    assert_eq!(sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -15.0), feed(80.0, 30.0, -15.0)]), &flat10()).unwrap().body_hits, 0);
}

#[test]
fn a_shank_wider_than_the_cutter_and_a_holder_are_checked_to_their_width() {
    // A 4 mm cutter with 8 mm of flutes on a 10 mm shank that begins 12 from the tip. A slot 14
    // deep in two passes: the cutter's own depth is fine, but at the second pass the shank's
    // lower end is at Z -2, and beside the 4 mm slot the stock is still at 0: 2 mm in.
    let necked = {
        let mut t = tool(ToolKind::FlatEndMill, 4.0, 0.0, 0.0);
        (t.geometry.flute_length, t.geometry.shoulder_length, t.geometry.shank_diameter) = (8.0, 12.0, 10.0);
        t
    };
    let mut sim = stock();
    let passes = vec![feed(20.0, 30.0, -7.0), feed(80.0, 30.0, -7.0), feed(80.0, 30.0, -14.0), feed(20.0, 30.0, -14.0)];
    let cut = sim.cut(1, &path(p(20.0, 30.0, 5.0), passes), &necked).unwrap();
    assert_eq!((cut.body_hits, sim.body_hits[0].move_index, sim.body_hits[0].part, sim.body_hits[0].depth), (2, 2, BodyPart::Shank, 2.0));
    let beside = sim.body_hits[1].at;
    assert!((beside.y - 30.0).abs() > 2.0 && (beside.y - 30.0).abs() < 5.0 && beside.z == -2.0, "{beside:?}");
    // One pass 10 deep is within the shank's reach (it begins 12 up), though past the flutes:
    // the stock above the flutes is in the neck's way under the cutter, and only there.
    let mut sim = stock();
    let cut = sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -10.0), feed(80.0, 30.0, -10.0)]), &necked).unwrap();
    assert_eq!((cut.body_hits, sim.body_hits[0].depth), (2, 2.0));
    assert!(sim.body_hits.iter().all(|hit| (hit.at.y - 30.0).abs() < 2.0), "{:?}", sim.body_hits);
    // A holder 30 mm across that the 10 mm tool sticks 12 out of, widening to 40 over 20 mm. A
    // pocket 16 x 16 cut 15 deep in two passes round a middle already cleared: the holder's face
    // is at Z -3 at the bottom, 3 mm into the stock round the pocket.
    let held = {
        let mut t = flat10();
        t.holder = Holder { stick_out: 12.0, segments: vec![HolderSegment { length: 20.0, lower_diameter: 30.0, upper_diameter: 40.0 }] };
        t
    };
    let square = |z: f64| vec![feed(47.0, 27.0, z), feed(53.0, 27.0, z), feed(53.0, 33.0, z), feed(47.0, 33.0, z), feed(47.0, 27.0, z)];
    let mut sim = stock();
    let down: Vec<Move> = square(-7.5).into_iter().chain(square(-15.0)).collect();
    let cut = sim.cut(1, &path(p(47.0, 27.0, 5.0), down), &held).unwrap();
    assert_eq!(cut.body_hits, 5, "every move of the lower pass: {:?}", sim.body_hits);
    assert!(
        sim.body_hits.iter().all(|hit| hit.part == BodyPart::Holder && hit.move_index >= 5 && (hit.depth - 3.0).abs() < 1e-9 && hit.at.z == -3.0)
    );
    // 12 deep, the holder's face is at the stock's top: nothing touches.
    let mut sim = stock();
    let shallow: Vec<Move> = square(-6.0).into_iter().chain(square(-12.0)).collect();
    assert_eq!(sim.cut(1, &path(p(47.0, 27.0, 5.0), shallow), &held).unwrap().body_hits, 0);
    // At rapid too. A slot 10 wide cut 15 deep by a tool that reaches; then this one run along
    // it at rapid, its tip 14 down: the tip is in the slot and passes through nothing, but the
    // holder's face is 2 below the top of the stock on both sides of the slot.
    let mut sim = stock();
    let slot = vec![feed(20.0, 30.0, -7.5), feed(80.0, 30.0, -7.5), feed(80.0, 30.0, -15.0), feed(20.0, 30.0, -15.0)];
    assert_eq!(sim.cut(1, &path(p(20.0, 30.0, 5.0), slot), &flat10()).unwrap().body_hits, 0);
    let along =
        sim.cut(2, &path(p(20.0, 30.0, 5.0), vec![rapid(20.0, 30.0, -14.0), rapid(80.0, 30.0, -14.0), rapid(80.0, 30.0, 5.0)]), &held).unwrap();
    assert_eq!((along.rapid_hits, along.body_hits), (0, 3), "{:?}", sim.body_hits);
    let worst = sim.body_hits[1];
    assert_eq!((worst.operation, worst.move_index, worst.part, worst.depth, worst.at.z), (2, 1, BodyPart::Holder, 2.0, -2.0));
    // Well above the stock a holder is not looked at: a rapid across with the tip 8 up.
    let over = sim.cut(3, &path(p(-30.0, 30.0, 8.0), vec![rapid(130.0, 30.0, 8.0)]), &held).unwrap();
    assert_eq!((over.rapid_hits, over.body_hits), (0, 0));
    // A tool with no holder described has none to check, however deep it goes; and hostile
    // sizes make no holder rather than a panic.
    let mut bare = flat10();
    bare.holder = Holder { stick_out: 5.0, segments: vec![] };
    let mut sim = stock();
    assert_eq!(sim.cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -15.0)]), &bare).unwrap().body_hits, 0);
    for v in [f64::NAN, f64::INFINITY, -1.0, 0.0, 1e300] {
        let mut odd = held.clone();
        odd.holder = Holder { stick_out: v, segments: vec![HolderSegment { length: v, lower_diameter: v, upper_diameter: 30.0 }; 300] };
        (odd.geometry.flute_length, odd.geometry.shank_diameter, odd.geometry.shoulder_length) = (v, v, v);
        let _ = stock().cut(1, &path(p(20.0, 30.0, 5.0), vec![feed(20.0, 30.0, -15.0), rapid(20.0, 30.0, 5.0)]), &odd);
    }
}
