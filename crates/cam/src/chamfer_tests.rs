use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::sim::Simulation;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};
use crate::toolpath::{ArcDirection, Plane};
use solidcore_record::Seg2;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// Tool 7: a 10 mm, 4-flute chamfer mill, 90 degrees at its point, with 10 mm of flutes and data
/// for aluminium: 100 m/min and 0.05 mm a tooth, plunging at half.
fn chamfer10() -> Tool {
    Tool {
        number: 7,
        name: "10 mm 90 degree chamfer mill".into(),
        kind: ToolKind::ChamferMill,
        geometry: ToolGeometry {
            diameter: 10.0,
            corner_radius: 0.0,
            flute_length: 10.0,
            shoulder_length: 0.0,
            overall_length: 60.0,
            shank_diameter: 10.0,
            tip_angle: 0.5 * PI,
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 4,
        holder: Holder { stick_out: 30.0, segments: vec![] },
        length_offset: 7,
        diameter_offset: 7,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData { material: "aluminium".into(), surface_speed: 100.0, chip_load: 0.05, plunge_factor: 0.5, ramp_factor: 0.5 }],
        overrides: Overrides::default(),
    }
}

/// The 60 x 40 x 20 block faced already: the stock is 1 mm bigger all round and no higher. In
/// the WCS (the middle of the stock's top) the block is X -30 to 30, Y -20 to 20, its top at Z 0.
fn faced_block() -> Placement {
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
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 0.0 } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    };
    setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }).unwrap()
}

fn block_outline() -> Loop2 {
    let corners = [(-30.0, -20.0), (30.0, -20.0), (30.0, 20.0), (-30.0, 20.0)];
    Loop2 { segs: (0..4).map(|i| Seg2::Line { a: q(corners[i].0, corners[i].1), b: q(corners[(i + 1) % 4].0, corners[(i + 1) % 4].1) }).collect() }
}

fn chamfered(op: &ChamferOp, tool: &Tool) -> Result<Chamfered> {
    op.toolpath(None, tool, &faced_block(), Some("aluminium"), &[block_outline()])
}

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

#[test]
fn a_chamfer_goes_once_round_the_wall_below_the_edge_and_outside_it() {
    // Half a millimetre of chamfer with a 90 degree tool: it goes 0.5 down the wall, the tip is
    // 1 below that (Z -1.5 under the block's top at 0), and at 45 degrees the axis is then 1
    // outside the wall: the centre goes round at X ±31 and Y ±21, the corners arcs of 1 about
    // the block's own. The arcs on and off are 1 too (the chamfer and half a millimetre, which
    // here is also a tenth of the tool): down at (1, -22), on to the wall at the middle of the
    // front, clockwise round, and off to (-1, -22).
    let d = chamfered(&ChamferOp::default(), &chamfer10()).unwrap();
    assert_eq!((d.top, d.z, d.width, d.leg, d.offset), (0.0, -1.5, 0.5, 0.5, 1.0));
    assert_eq!((d.entries.clone(), d.leads.clone()), (vec![[1.0, -22.0]], vec![1.0]));
    assert!(close(d.round, 200.0 + 2.0 * PI), "{}", d.round);
    assert_eq!(ChamferOp::default().wall_height(&chamfer10(), &faced_block()).unwrap(), -0.5);
    // 100 m/min on its 10 mm: the chip thinned by the cut's half millimetre of width, so the
    // feed is 0.05 mm a tooth over 2 sqrt(0.05 - 0.05^2).
    let rpm = 100_000.0 / (10.0 * PI);
    let feed = 0.05 / (2.0 * (0.05_f64 - 0.0025).sqrt()) * 4.0 * rpm;
    assert!(close(d.rpm, rpm) && close(d.feed, feed) && close(d.plunge_feed, 0.5 * feed), "{} {} {}", d.rpm, d.feed, d.plunge_feed);
    let z = -1.5;
    let arc = |x: f64, y: f64, cx: f64, cy: f64, direction: ArcDirection| Move::Arc {
        to: p(x, y, z),
        center: p(cx, cy, z),
        plane: Plane::Xy,
        direction,
        feed: d.feed,
    };
    let line = |x: f64, y: f64| Move::Linear { to: p(x, y, z), feed: d.feed };
    let (on, round) = (ArcDirection::CounterClockwise, ArcDirection::Clockwise);
    let want = vec![
        Move::ToolChange { tool: 7 },
        Move::Spindle { rpm: d.rpm, direction: SpindleDirection::Clockwise },
        Move::Coolant { mode: Coolant::Flood },
        Move::Rapid { to: p(1.0, -22.0, 25.0) },
        Move::Rapid { to: p(1.0, -22.0, 5.0) },
        Move::Linear { to: p(1.0, -22.0, z), feed: d.plunge_feed },
        arc(0.0, -21.0, 0.0, -22.0, on),
        line(-30.0, -21.0),
        arc(-31.0, -20.0, -30.0, -20.0, round),
        line(-31.0, 20.0),
        arc(-30.0, 21.0, -30.0, 20.0, round),
        line(30.0, 21.0),
        arc(31.0, 20.0, 30.0, 20.0, round),
        line(31.0, -20.0),
        arc(30.0, -21.0, 30.0, -20.0, round),
        line(0.0, -21.0),
        arc(-1.0, -22.0, 0.0, -22.0, on),
        Move::Rapid { to: p(-1.0, -22.0, 5.0) },
        Move::Rapid { to: p(-1.0, -22.0, 25.0) },
        Move::Coolant { mode: Coolant::Off },
        Move::SpindleStop,
    ];
    assert!(same_moves(&d.toolpath.moves, &want), "{:#?}", d.toolpath.moves);
    // Conventional goes the other way round: down at (-1, -22).
    let d = chamfered(&ChamferOp { direction: CutDirection::Conventional, ..ChamferOp::default() }, &chamfer10()).unwrap();
    assert_eq!(d.entries, vec![[-1.0, -22.0]]);
}

#[test]
fn the_tools_angle_and_tip_and_the_numbers_given_place_the_cut() {
    // A 60 degree tool with a 2 mm flat at its tip, a chamfer 1 wide, the tip half a millimetre
    // below it: 30 degrees from the axis, so the chamfer goes sqrt(3) down the wall, and the
    // axis is the flat's 1 and 0.5 tan 30 outside it.
    let pointed = {
        let mut t = chamfer10();
        (t.geometry.tip_angle, t.geometry.tip_diameter) = (PI / 3.0, 2.0);
        t
    };
    let op = ChamferOp { width: Some(1.0), tip_below: Some(0.5), top: Some(-2.0), ..ChamferOp::default() };
    let d = chamfered(&op, &pointed).unwrap();
    let slope = (PI / 6.0).tan();
    assert!(close(d.leg, 3.0_f64.sqrt()) && close(d.offset, 1.0 + 0.5 * slope) && close(d.z, -2.0 - 3.0_f64.sqrt() - 0.5), "{d:?}");
    assert!(close(op.wall_height(&pointed, &faced_block()).unwrap(), -2.0 - 3.0_f64.sqrt()));
    // The arcs on and off: the chamfer's 1 and half a millimetre, more than a tenth of the tool.
    assert_eq!(d.leads, vec![1.5]);
    // A spot drill chamfers too; an end mill does not.
    let spot = {
        let mut t = chamfer10();
        t.kind = ToolKind::SpotDrill;
        t
    };
    assert_eq!(chamfered(&ChamferOp::default(), &spot).unwrap().offset, 1.0);
    // Speed and feed typed in are used as they are; no arcs when none are asked for.
    let d = chamfered(&ChamferOp { rpm: Some(4000.0), feed: Some(300.0), lead: Some(0.0), ..ChamferOp::default() }, &chamfer10()).unwrap();
    assert_eq!((d.rpm, d.feed, d.plunge_feed, d.leads.clone(), d.entries.clone()), (4000.0, 300.0, 150.0, vec![0.0], vec![[0.0, -21.0]]));
    assert!(d.notes.is_empty(), "{:?}", d.notes);
}

/// The faced block's stock in cells of 0.1 mm with the block as the part: its top at Z 0 where
/// the block is, nothing elsewhere.
fn block_in_stock() -> Simulation {
    let sim = Simulation::new(Bounds { min: p(-31.0, -21.0, -20.0), max: p(31.0, 21.0, 0.0) }, 0.1, 0.01).unwrap();
    let f = sim.stock.clone();
    let part = (0..f.nx * f.ny)
        .map(|k| {
            let c = f.centre(k % f.nx, k / f.nx);
            if c.x.abs() < 30.0 && c.y.abs() < 20.0 { 0.0 } else { -21.0 }
        })
        .collect();
    sim.with_part(part).unwrap()
}

#[test]
fn the_simulation_finds_the_chamfer_where_it_was_meant() {
    let d = chamfered(&ChamferOp::default(), &chamfer10()).unwrap();
    // Meant to cut the chamfer's half millimetre into the sharp edge the model has: no gouge.
    let mut sim = block_in_stock();
    let cut = sim.cut_into(0, &d.toolpath, &chamfer10(), d.leg).unwrap();
    assert_eq!((cut.gouges, cut.rapid_hits), (0, 0));
    // Along the front wall (Y -20) the stock rises at 45 degrees from 0.5 below the top at the
    // wall to the top 0.5 in: over each cell's own centre, exactly.
    let f = sim.stock.clone();
    let height = |x: f64, y: f64| f.at(((x - f.x) / f.dx - 0.5).round() as usize, ((y - f.y) / f.dy - 0.5).round() as usize).unwrap();
    for (k, inside) in [0.05, 0.15, 0.25, 0.35, 0.45].into_iter().enumerate() {
        let want = -0.5 + inside;
        assert!((height(10.05, -20.0 + inside) - want).abs() < 1e-9, "cell {k}: {} and {want}", height(10.05, -20.0 + inside));
        assert!((height(-30.0 + inside, 5.05) - want).abs() < 1e-9, "the left wall, cell {k}");
    }
    assert_eq!((height(10.05, -19.45), height(0.05, 0.05)), (0.0, 0.0), "past the chamfer the top is as it was");
    // The part is taken to be what the chamfer left: nothing stands above it, nothing is below.
    let part = sim.part().unwrap();
    assert!(f.heights.iter().zip(part).all(|(stock, part)| *part <= -20.0 || (stock - part).abs() < 1e-9));
    // Round a corner the chamfer follows the tool's arc about the block's corner: 0.5 down at
    // the corner itself, by a cell's width or so.
    assert!((height(29.95, -19.95) + 0.5).abs() < 0.1, "{}", height(29.95, -19.95));
    // Cut without saying it was meant, the same toolpath cuts into the part: every move along
    // the wall, 0.45 deep at the cell nearest it.
    let mut plain = block_in_stock();
    let cut = plain.cut(0, &d.toolpath, &chamfer10()).unwrap();
    assert!(cut.gouges >= 8, "{}", cut.gouges);
    assert!(
        plain.gouges.iter().all(|g| g.depth <= 0.5 + 1e-9) && plain.gouges.iter().any(|g| (g.depth - 0.45).abs() < 1e-9),
        "{:?}",
        plain.gouges.first()
    );
    // A chamfer cut deeper than it says it means to is still found: the same path 1 mm lower.
    let lower = |by: f64| Toolpath {
        start: d.toolpath.start,
        moves: d
            .toolpath
            .moves
            .iter()
            .map(|m| match *m {
                Move::Linear { to, feed } if to.z < 0.0 => Move::Linear { to: p(to.x, to.y, to.z - by), feed },
                Move::Arc { to, center, plane, direction, feed } => {
                    Move::Arc { to: p(to.x, to.y, to.z - by), center: p(center.x, center.y, center.z - by), plane, direction, feed }
                }
                other => other,
            })
            .collect(),
    };
    let mut deep = block_in_stock();
    assert!(deep.cut_into(0, &lower(1.0), &chamfer10(), d.leg).unwrap().gouges > 0);
    assert!(deep.cut_into(0, &lower(1.0), &chamfer10(), -1.0).is_err() && deep.cut_into(0, &lower(1.0), &chamfer10(), f64::NAN).is_err());
    // What is meant is measured from the model, not from what a pass before left: the chamfer
    // cut as meant and then again 0.3 lower is 0.8 into the edge where 0.5 was meant. The cells
    // within 0.29 of the wall are below what was meant, each 0.3 below what the first pass left.
    let mut twice = block_in_stock();
    assert_eq!(twice.cut_into(0, &d.toolpath, &chamfer10(), d.leg).unwrap().gouges, 0);
    let again = twice.cut_into(1, &lower(0.3), &chamfer10(), d.leg).unwrap();
    assert!(again.gouges >= 8, "{}", again.gouges);
    // (Along the wall exactly 0.3; on the arcs on and off it, where the tool is a hair further
    // out, a hair less.)
    assert!(twice.gouges.iter().all(|g| g.operation == 1 && g.depth > 0.25 && g.depth <= 0.3 + 1e-9), "{:?}", twice.gouges);
    assert!(twice.gouges.iter().filter(|g| (g.depth - 0.3).abs() < 1e-9).count() >= 4);
    let f = twice.stock.clone();
    let cell = |x: f64, y: f64| ((y - f.y) / f.dy - 0.5).round() as usize * f.nx + ((x - f.x) / f.dx - 0.5).round() as usize;
    let (model, part) = (twice.model().unwrap(), twice.part().unwrap());
    // The model is as it was given; the part as meant is what the first pass left where the
    // second went too far (0.45 down at the cell by the wall), and what the second left where
    // it did not (0.35 in: 0.15 down, then 0.45).
    assert_eq!((model[cell(10.05, -19.95)], model[cell(10.05, -19.65)]), (0.0, 0.0));
    assert!((part[cell(10.05, -19.95)] + 0.45).abs() < 1e-9 && (part[cell(10.05, -19.65)] + 0.45).abs() < 1e-9);
    assert!((f.heights[cell(10.05, -19.95)] + 0.75).abs() < 1e-9, "the stock is cut all the same");
    // What is cut afterwards is judged against the part as it was meant: a flat tool passing
    // 0.2 below the top over the chamfered edge alone (its side 0.3 inside the wall) cuts air,
    // where on the sharp edge it would have cut into the part.
    let pass = Toolpath { start: p(-10.0, -24.7, -0.2), moves: vec![Move::Linear { to: p(10.0, -24.7, -0.2), feed: 500.0 }] };
    let flat = Tool { kind: ToolKind::FlatEndMill, ..chamfer10() };
    let mut after = block_in_stock();
    after.cut_into(0, &d.toolpath, &chamfer10(), d.leg).unwrap();
    let cut = after.cut(1, &pass, &flat).unwrap();
    assert_eq!((cut.gouges, cut.removed), (0, 0.0));
    let sharp = block_in_stock().cut(0, &pass, &flat).unwrap();
    assert_eq!(sharp.gouges, 1);
}

#[test]
fn a_chamfer_round_an_arc_is_not_a_cut_into_the_part() {
    // A disc of 40: the tool goes round it in arcs, which the simulation cuts as chords a hair
    // inside them, so the cone is a hair nearer the disc and cuts as much lower. For a tool
    // with a sloping side the chords are fine enough to keep that within the tolerance, on a
    // coarse grid as on a fine one: no gouge, and the chamfer no deeper than its half
    // millimetre and a tolerance (0.01) anywhere round the disc.
    let disc = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let d = ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), Some("aluminium"), &[disc]).unwrap();
    for cell in [0.25, 0.1] {
        let sim = Simulation::new(Bounds { min: p(-31.0, -21.0, -20.0), max: p(31.0, 21.0, 0.0) }, cell, 0.01).unwrap();
        let f = sim.stock.clone();
        let part: Vec<f64> =
            (0..f.nx * f.ny).map(|k| if f.centre(k % f.nx, k / f.nx).x.hypot(f.centre(k % f.nx, k / f.nx).y) < 20.0 { 0.0 } else { -21.0 }).collect();
        let mut sim = sim.with_part(part.clone()).unwrap();
        let cut = sim.cut_into(0, &d.toolpath, &chamfer10(), d.leg).unwrap();
        assert_eq!(cut.gouges, 0, "cells of {cell}: {:?}", sim.gouges.first());
        let deepest = sim.stock.heights.iter().zip(&part).filter(|(_, part)| **part > -20.0).map(|(stock, part)| part - stock).fold(0.0, f64::max);
        assert!(deepest <= d.leg + 0.01 + 1e-9 && deepest > 0.3, "cells of {cell}: the chamfer goes {deepest} down");
        // Told nothing was meant, the same toolpath is a cut into the part at every arc.
        let mut plain =
            Simulation::new(Bounds { min: p(-31.0, -21.0, -20.0), max: p(31.0, 21.0, 0.0) }, cell, 0.01).unwrap().with_part(part).unwrap();
        assert!(plain.cut(0, &d.toolpath, &chamfer10()).unwrap().gouges >= 2);
    }
}

#[test]
fn an_island_standing_in_a_pocket_is_left_sharp_and_said() {
    // Just under the top of a plate with a pocket and an island in it, the section has two
    // pieces: the plate, and the island inside the plate's outline. The chamfer goes round the
    // outside exactly as it does without the island, and says what it left.
    let island = Loop2 { segs: vec![Seg2::Arc { center: q(8.0, 2.0), radius: 9.0, start: 0.0, sweep: 2.0 * PI }] };
    let alone = chamfered(&ChamferOp::default(), &chamfer10()).unwrap();
    for outline in [vec![block_outline(), island.clone()], vec![island.clone(), block_outline()]] {
        let d = ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), Some("aluminium"), &outline).unwrap();
        assert!(same_moves(&d.toolpath.moves, &alone.toolpath.moves));
        assert_eq!((d.entries.len(), d.round), (1, alone.round));
        assert_eq!(
            d.notes,
            ["1 edge(s) inside the part's outline (an island standing in a pocket) left sharp: a chamfer goes round the outside only"]
        );
    }
    assert!(alone.notes.is_empty());
    // Two parts side by side are each an outside, and each is gone round.
    let beside = Loop2 { segs: vec![Seg2::Arc { center: q(60.0, 0.0), radius: 9.0, start: 0.0, sweep: 2.0 * PI }] };
    let d = ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), Some("aluminium"), &[block_outline(), beside]).unwrap();
    assert!(d.entries.len() == 2 && d.notes.is_empty() && close(d.round, alone.round + 20.0 * PI), "{:?} {}", d.notes, d.round);
    // More loops than a part has are refused before any is looked at.
    let many = vec![island; MAX_CHAMFER_OUTLINES + 1];
    let refused = ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), None, &many).unwrap_err().to_string();
    assert!(refused.contains("1 to 1000 loops"), "{refused}");
}

#[test]
fn what_cannot_be_chamfered_is_refused_with_the_reason() {
    let mill = {
        let mut t = chamfer10();
        (t.kind, t.geometry.tip_angle) = (ToolKind::FlatEndMill, 0.0);
        t
    };
    let needle = {
        let mut t = chamfer10();
        t.geometry.tip_angle = 0.1;
        t
    };
    let short = {
        let mut t = chamfer10();
        t.geometry.flute_length = 1.0;
        t
    };
    let cases: Vec<(ChamferOp, Tool, &str)> = vec![
        (ChamferOp::default(), mill, "does not cut a chamfer"),
        (ChamferOp::default(), needle, "tip angle must be 20 to 160 degrees"),
        (ChamferOp::default(), short, "past tool 7's 1 mm of flutes"),
        // 4.5 wide with the tip 1 below: 2 x (1 + 4.5) = 11 mm across on a 10 mm tool.
        (ChamferOp { width: Some(4.5), ..ChamferOp::default() }, chamfer10(), "is cut 11 mm across on the tool, and tool 7 is 10 mm"),
        (ChamferOp { width: Some(0.0), ..ChamferOp::default() }, chamfer10(), "chamfer's width"),
        (ChamferOp { tip_below: Some(-0.1), ..ChamferOp::default() }, chamfer10(), "below the chamfer"),
        (ChamferOp { top: Some(f64::NAN), ..ChamferOp::default() }, chamfer10(), "finite"),
        (ChamferOp { top: Some(6.0), ..ChamferOp::default() }, chamfer10(), "retract height"),
        (ChamferOp { lead: Some(-1.0), ..ChamferOp::default() }, chamfer10(), "arcs in and out"),
        (ChamferOp { rpm: Some(0.0), ..ChamferOp::default() }, chamfer10(), "rpm"),
        (ChamferOp { feed: Some(-1.0), ..ChamferOp::default() }, chamfer10(), "feed"),
    ];
    for (op, tool, expect) in cases {
        let e = chamfered(&op, &tool).unwrap_err().to_string();
        assert!(e.contains(expect), "{op:?}: {e:?} does not mention {expect:?}");
    }
    // The widest chamfer this tool cuts with its tip 1 below: 4 mm.
    assert_eq!(chamfered(&ChamferOp { width: Some(4.0), ..ChamferOp::default() }, &chamfer10()).unwrap().offset, 1.0);
    let with =
        |outline: &[Loop2]| ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), Some("aluminium"), outline).unwrap_err().to_string();
    assert!(with(&[]).contains("no outline"));
    let e = ChamferOp::default().toolpath(None, &chamfer10(), &faced_block(), None, &[block_outline()]).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
}

#[test]
fn hostile_numbers_never_panic_and_a_chamfer_is_plain_json() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, 5e-324, f64::MAX, f64::MIN];
    for &v in &bad {
        for field in 0..6 {
            let mut op = ChamferOp::default();
            match field {
                0 => op.width = Some(v),
                1 => op.tip_below = Some(v),
                2 => op.top = Some(v),
                3 => op.lead = Some(v),
                4 => op.rpm = Some(v),
                _ => op.feed = Some(v),
            }
            let _ = chamfered(&op, &chamfer10());
        }
        for size in 0..4 {
            let mut odd = chamfer10();
            match size {
                0 => odd.geometry.diameter = v,
                1 => odd.geometry.tip_angle = v,
                2 => odd.geometry.tip_diameter = v,
                _ => odd.geometry.flute_length = v,
            }
            if let Ok(done) = chamfered(&ChamferOp::default(), &odd) {
                assert!(done.offset.is_finite() && done.offset >= 0.0 && done.z.is_finite(), "{v}: {done:?}");
            }
        }
    }
    let op: ChamferOp = serde_json::from_str("{}").unwrap();
    assert_eq!(op, ChamferOp::default());
    let op: ChamferOp = serde_json::from_str(r#"{"width": 1, "tip_below": 0.5, "direction": "conventional"}"#).unwrap();
    assert_eq!((op.width, op.tip_below, op.direction), (Some(1.0), Some(0.5), CutDirection::Conventional));
    assert!(serde_json::from_str::<ChamferOp>(r#"{"angle": 45}"#).is_err());
    assert_eq!(serde_json::from_value::<ChamferOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
