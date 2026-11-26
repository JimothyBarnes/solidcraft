use std::f64::consts::PI;

use solidcore_record::Loop2;

use super::*;
use crate::contour::{ContourSide, CutDirection, Tabs};
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry, ToolKind};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

/// Tool 4: a 20 mm, 4-flute flat end mill with 30 mm of flutes and data for aluminium.
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

/// The 60 x 40 x 20 block with 1 mm of stock round it and on top, the WCS at the middle of the
/// stock's top: the model is X -30 to 30, Y -20 to 20, Z -21 to -1, the stock X -31 to 31, Y -21
/// to 21, Z -21 to 0; clearance 25, retract 5.
fn block() -> Placement {
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
    setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(60.0, 40.0, 20.0) }).unwrap()
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Region2 {
    let c = [q(x0, y0), q(x1, y0), q(x1, y1), q(x0, y1)];
    Region2 { outer: Loop2 { segs: (0..4).map(|i| Seg2::Line { a: c[i], b: c[(i + 1) % 4] }).collect() }, holes: Vec::new() }
}

/// The step's back half and a side wall X 20 to 30 down its front, as one L.
fn l_walls() -> Vec<Region2> {
    let c = [q(-30.0, 0.0), q(20.0, 0.0), q(20.0, -20.0), q(30.0, -20.0), q(30.0, 20.0), q(-30.0, 20.0)];
    vec![Region2 { outer: Loop2 { segs: (0..6).map(|i| Seg2::Line { a: c[i], b: c[(i + 1) % 6] }).collect() }, holes: Vec::new() }]
}

fn line(a: (f64, f64), b: (f64, f64)) -> Seg2 {
    Seg2::Line { a: q(a.0, a.1), b: q(b.0, b.1) }
}

/// A step along the block's front: its back half (Y 0 to 20) stands to the top, its front half is
/// a ledge 10 mm below the model's top, at Z -11. What stands above the ledge is the back half.
fn step_walls() -> Vec<Region2> {
    vec![rect(-30.0, 0.0, 30.0, 20.0)]
}

/// The contour down to the ledge.
fn to_ledge() -> ContourOp {
    ContourOp { bottom: Some(-11.0), ..ContourOp::default() }
}

/// The cutting moves (fed, not rapid) at the height `z`: their ends.
fn cuts_at(path: &Toolpath, z: f64) -> Vec<Vec3> {
    path.moves
        .iter()
        .filter_map(|m| match *m {
            Move::Linear { to, .. } | Move::Arc { to, .. } if near(to.z, z) => Some(to),
            _ => None,
        })
        .collect()
}

#[test]
fn along_a_steps_wall_the_tool_runs_its_radius_off_it_from_the_air_past_one_end_to_the_air_past_the_other() {
    let chain = vec![line((-30.0, 0.0), (30.0, 0.0))];
    let done = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain.clone()]).unwrap();
    // 11 mm from the stock's top to the ledge, at most 10 a level: two of 5.5.
    assert_eq!(done.levels, vec![-5.5, -11.0]);
    assert!(near(done.offset, 10.0) && done.passes == 1);
    // Climb with a clockwise spindle keeps the wall on the tool's right: going west along it.
    // The pass is the line Y -10, lengthened to the air: the stock is X -31 to 31 and the
    // cutter's edge clears it by 2, so its centre is at X 43 and X -43 there.
    assert_eq!(done.entries, vec![[43.0, -10.0]]);
    assert!(near(done.round, 86.0), "{}", done.round);
    for z in [-5.5, -11.0] {
        let cuts = cuts_at(&done.toolpath, z);
        assert!(!cuts.is_empty());
        assert!(cuts.iter().all(|c| near(c.y, -10.0)), "{cuts:?}");
        let (west, east) = cuts.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(w, e), c| (w.min(c.x), e.max(c.x)));
        assert!(near(west, -43.0) && near(east, 43.0), "{west} {east}");
    }
    // Down in the air beside the stock at each level, the way in from it at half the feed.
    let downs: Vec<Vec3> = done
        .toolpath
        .moves
        .iter()
        .filter_map(|m| match *m {
            Move::Linear { to, feed } if feed == done.plunge_feed && near(to.x, 43.0) => Some(to),
            _ => None,
        })
        .collect();
    assert_eq!(downs.len(), 2, "{:?}", done.toolpath.moves);
    let slow: Vec<Vec3> = done
        .toolpath
        .moves
        .iter()
        .filter_map(|m| match *m {
            Move::Linear { to, feed } if near(feed, 0.5 * done.feed) && !near(to.x, 43.0) => Some(to),
            _ => None,
        })
        .collect();
    assert!(slow.len() == 2 && slow.iter().all(|to| near(to.x, 30.0)), "{slow:?}");
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    // Conventional goes the other way along it.
    let other = ContourOp { direction: CutDirection::Conventional, ..to_ledge() };
    let back = other.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain]).unwrap();
    assert_eq!(back.entries, vec![[-43.0, -10.0]]);
}

#[test]
fn a_chain_picked_either_way_round_gives_the_same_toolpath() {
    let forward = vec![line((-30.0, 0.0), (30.0, 0.0))];
    let backward = vec![line((30.0, 0.0), (-30.0, 0.0))];
    let a = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[forward]).unwrap();
    let b = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[backward]).unwrap();
    assert_eq!(a.toolpath, b.toolpath);
}

#[test]
fn where_a_wall_stands_across_the_chain_the_pass_stops_at_it_and_is_cut_from_its_other_end() {
    // An L: the back half stands, and a side wall X 20 to 30 down the front. The chain is the
    // back wall's foot from X -30 to the side wall, X 20.
    let walls = l_walls();
    let chain = vec![line((-30.0, 0.0), (20.0, 0.0))];
    let done = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &walls, &[chain]).unwrap();
    // The pass at Y -10 meets the side wall's pass at X 10, the tool touching both walls: it
    // stops there. Climbing it would start there with no air, so it is cut from its west end, in
    // the air, the other way, and the operation says so.
    assert_eq!(done.entries, vec![[-43.0, -10.0]]);
    assert!(near(done.round, 53.0), "{}", done.round);
    for z in [-5.5, -11.0] {
        let cuts = cuts_at(&done.toolpath, z);
        let east = cuts.iter().map(|c| c.x).fold(f64::NEG_INFINITY, f64::max);
        assert!(near(east, 10.0) && cuts.iter().all(|c| near(c.y, -10.0)), "{cuts:?}");
    }
    assert!(done.notes.iter().any(|n| n.contains("the other way than asked")), "{:?}", done.notes);
}

#[test]
fn an_l_shaped_chain_is_followed_round_its_corner_and_out_past_both_ends() {
    // The same L, its chain the back wall's foot and the side wall's: one chain round the inside
    // corner at (20, 0), out to the front at (20, -20).
    let walls = l_walls();
    let chain = vec![line((-30.0, 0.0), (20.0, 0.0)), line((20.0, 0.0), (20.0, -20.0))];
    let done = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &walls, &[chain]).unwrap();
    // Climbing, the walls on the tool's right: in from the front at X 10, Y -33 (the stock's
    // front at -21, the radius and 2 mm of air), up to the corner at (10, -10), and west along
    // the back wall to X -43: 23 + 53 mm.
    assert_eq!(done.entries, vec![[10.0, -33.0]]);
    assert!(near(done.round, 76.0), "{}", done.round);
    let cuts = cuts_at(&done.toolpath, -11.0);
    assert!(cuts.iter().all(|c| near(c.y, -10.0) || near(c.x, 10.0)), "{cuts:?}");
    assert!(done.notes.is_empty(), "{:?}", done.notes);
}

#[test]
fn passes_sideways_are_cut_the_furthest_first_each_along_the_chain() {
    let chain = vec![line((-30.0, 0.0), (30.0, 0.0))];
    let op = ContourOp { passes: Some(2), ..to_ledge() };
    let done = op.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain]).unwrap();
    // Four tenths of the tool apart: Y -18 first, then Y -10, at each level.
    assert!(near(done.stepover, 8.0) && done.passes == 2);
    let ys: Vec<f64> = cuts_at(&done.toolpath, -5.5).iter().map(|c| c.y).collect();
    let first_on_wall = ys.iter().position(|&y| near(y, -10.0)).unwrap();
    assert!(ys[..first_on_wall].iter().all(|&y| near(y, -18.0)) && first_on_wall > 0, "{ys:?}");
    assert_eq!(done.entries.len(), 2);
}

#[test]
fn a_contour_along_an_open_chain_takes_no_tabs_and_needs_something_beside_it() {
    let chain = vec![line((-30.0, 0.0), (30.0, 0.0))];
    let tabs = ContourOp { tabs: Some(Tabs { count: 2, width: None, height: None }), ..to_ledge() };
    let e = tabs.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain.clone()]).unwrap_err();
    assert!(e.to_string().contains("no tabs"), "{e}");
    let e = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &[], &[chain.clone()]).unwrap_err();
    assert!(e.to_string().contains("nothing stands above the bottom"), "{e}");
    // A chain that is no part of the walls' outline: nothing of the pass runs along it.
    let away = vec![line((-30.0, -15.0), (30.0, -15.0))];
    let e = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[away]).unwrap_err();
    assert!(e.to_string().contains("can't go along the open chain"), "{e}");
    // The side a closed chain is cut on does not matter to an open one (its edges picked, as an
    // operation along chains has them).
    let inside = ContourOp { side: ContourSide::Inside, edges: vec![[30.0, 20.0, 9.0]], ..to_ledge() };
    assert!(inside.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain]).is_ok());
}

#[test]
fn closed_chains_and_open_ones_are_cut_under_one_tool_change() {
    // The step's back half picked round its top edge as a closed chain (cut round its outside
    // from the top down to the ledge), and the wall's foot as an open one.
    let chain = vec![line((-30.0, 0.0), (30.0, 0.0))];
    let op = to_ledge();
    let round = op.toolpath(None, &mill20(), &block(), Some("aluminium"), &[step_walls()[0].outer.clone()]).unwrap();
    let along = op.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[chain]).unwrap();
    let (rounds, alongs) = (round.toolpath.moves.len(), along.toolpath.moves.len());
    let (round_length, along_length) = (round.round, along.round);
    let both = round.and_then(along).unwrap();
    let count = |want: fn(&Move) -> bool| both.toolpath.moves.iter().filter(|m| want(m)).count();
    assert_eq!(count(|m| matches!(m, Move::ToolChange { .. })), 1);
    assert_eq!(count(|m| matches!(m, Move::SpindleStop)), 1);
    // Less the first's coolant off and spindle stop, and the second's tool change, spindle and
    // coolant on.
    assert_eq!(both.toolpath.moves.len(), rounds - 2 + alongs - 3);
    assert!(near(both.round, round_length + along_length));
    assert_eq!(both.entries.len(), 2);
}

#[test]
fn hostile_chains_and_walls_never_panic() {
    let bad = f64::NAN;
    let chains = [vec![line((bad, 0.0), (30.0, f64::INFINITY))], vec![], vec![line((0.0, 0.0), (0.0, 0.0))]];
    let walls = [vec![rect(bad, 0.0, 30.0, 20.0)], vec![rect(0.0, 0.0, 0.0, 0.0)], step_walls()];
    for chain in &chains {
        for walls in &walls {
            let _ = to_ledge().toolpath_along(None, &mill20(), &block(), Some("aluminium"), walls, std::slice::from_ref(chain));
        }
    }
    let angle = ContourOp { ramp_angle: Some(PI), ..to_ledge() };
    let _ = angle.toolpath_along(None, &mill20(), &block(), Some("aluminium"), &step_walls(), &[vec![line((-30.0, 0.0), (30.0, 0.0))]]);
}
