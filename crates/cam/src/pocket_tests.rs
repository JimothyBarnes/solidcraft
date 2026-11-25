use std::f64::consts::PI;

use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use crate::tool::{CuttingData, Holder, Overrides, ToolGeometry};
use crate::toolpath::{ArcDirection, Drawn};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * b.abs().max(1.0)
}

/// Tool 2: a 10 mm, 3-flute flat end mill with 22 mm of flutes and data for aluminium (150 m/min,
/// 0.05 mm a tooth, plunging and ramping at half): 15 000 / pi rev/min, 0.15 mm a revolution.
fn mill10() -> Tool {
    Tool {
        number: 2,
        name: "10 mm flat end mill".into(),
        kind: ToolKind::FlatEndMill,
        geometry: ToolGeometry {
            diameter: 10.0,
            corner_radius: 0.0,
            flute_length: 22.0,
            shoulder_length: 0.0,
            overall_length: 72.0,
            shank_diameter: 10.0,
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

const RPM: f64 = 15_000.0 / PI;

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

/// A closed loop of straight sides through `points`, in order.
fn polygon(points: &[(f64, f64)]) -> Loop2 {
    let n = points.len();
    Loop2 { segs: (0..n).map(|i| Seg2::Line { a: q(points[i].0, points[i].1), b: q(points[(i + 1) % n].0, points[(i + 1) % n].1) }).collect() }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// A pocket whose floor is at Z -7 (6 mm into the block): the wall through `wall`, islands
/// through each of `islands`.
fn pocket(wall: &[(f64, f64)], islands: &[Vec<(f64, f64)>]) -> PocketShape {
    PocketShape { floor: -7.0, region: Region2 { outer: polygon(wall), holes: islands.iter().map(|i| polygon(i).reversed()).collect() }, open: None }
}

fn one(at: [f64; 2]) -> PocketOp {
    PocketOp { pockets: vec![at], ..PocketOp::default() }
}

fn pocketed(op: &PocketOp, shapes: &[PocketShape]) -> Result<Pocketed> {
    op.toolpath(None, &mill10(), &placed(), Some("aluminium"), shapes)
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

/// Whether `at` is inside the closed polygon through `points` (crossings of a ray to the right).
fn inside(at: Vec2, points: &[(f64, f64)]) -> bool {
    let n = points.len();
    (0..n)
        .filter(|&i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            (a.1 > at.y) != (b.1 > at.y) && at.x < a.0 + (at.y - a.1) / (b.1 - a.1) * (b.0 - a.0)
        })
        .count()
        % 2
        == 1
}

/// How far the tool's centre at `at` is from the nearest wall or island of a pocket; negative
/// when it is outside the pocket or in an island.
fn room(at: Vec2, wall: &[(f64, f64)], islands: &[Vec<(f64, f64)>]) -> f64 {
    let nearest = islands.iter().map(|i| distance(at, i)).fold(distance(at, wall), f64::min);
    if inside(at, wall) && !islands.iter().any(|i| inside(at, i)) { nearest } else { -nearest }
}

/// Every point the tool's centre feeds through (ramps, rings and the ways between them, and the
/// ends of its descents), a quarter of a millimetre apart at most.
fn fed_points(path: &Toolpath) -> Vec<Vec3> {
    let mut out = Vec::new();
    for (kind, a, b) in path.drawn(1e-5).unwrap() {
        if kind == Drawn::Rapid {
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

/// Checks the toolpath of a pocket cut with the 10 mm tool ([`check_with`]).
fn check(done: &Pocketed, wall: &[(f64, f64)], islands: &[Vec<(f64, f64)>], floor: f64) {
    let looked_at = check_with(done, wall, islands, floor, 5.0);
    assert!(looked_at > 100, "the check looked at too little: {looked_at} places");
}

/// Checks a pocket's toolpath against its shape, by measuring: the tool's centre is never nearer
/// a wall or an island than the tool's `radius`, and at the floor every place the tool can reach
/// is cut. Places are cells of half a millimetre: a cell is reachable when a cell where the
/// centre may be lies within the radius less a cell of it, and cut when a fed point at the floor
/// lies within the radius. The centre may be where it has a hundredth of a millimetre to spare:
/// a gap exactly as wide as the tool is not one it is asked to go through. Returns how many
/// places were reachable.
fn check_with(done: &Pocketed, wall: &[(f64, f64)], islands: &[Vec<(f64, f64)>], floor: f64, radius: f64) -> usize {
    let points = fed_points(&done.toolpath);
    let tightest = points.iter().map(|at| room(q(at.x, at.y), wall, islands)).fold(f64::INFINITY, f64::min);
    assert!(tightest >= radius - 1e-4, "the tool's centre comes within {tightest} mm of a wall, not {radius}");
    let lowest = points.iter().map(|at| at.z).fold(f64::INFINITY, f64::min);
    assert!((lowest - floor).abs() <= 1e-9, "the tool goes down to Z {lowest}, not the floor at {floor}");
    let (x0, x1) = wall.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), w| (lo.min(w.0), hi.max(w.0)));
    let (y0, y1) = wall.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), w| (lo.min(w.1), hi.max(w.1)));
    let h = 0.5;
    let (nx, ny) = (((x1 - x0) / h).ceil() as usize + 1, ((y1 - y0) / h).ceil() as usize + 1);
    let cell = |i: usize, j: usize| q(x0 + i as f64 * h, y0 + j as f64 * h);
    let stamp = |grid: &mut Vec<bool>, at: Vec2, radius: f64| {
        let reach = (radius / h).ceil() as isize;
        let (ci, cj) = (((at.x - x0) / h).round() as isize, ((at.y - y0) / h).round() as isize);
        for i in (ci - reach).max(0)..=(ci + reach).min(nx as isize - 1) {
            for j in (cj - reach).max(0)..=(cj + reach).min(ny as isize - 1) {
                let c = cell(i as usize, j as usize);
                if (c.x - at.x).hypot(c.y - at.y) <= radius {
                    grid[i as usize * ny + j as usize] = true;
                }
            }
        }
    };
    let (mut reachable, mut cut) = (vec![false; nx * ny], vec![false; nx * ny]);
    for i in 0..nx {
        for j in 0..ny {
            if room(cell(i, j), wall, islands) >= radius + 0.01 {
                stamp(&mut reachable, cell(i, j), radius - h);
            }
        }
    }
    for at in points.iter().filter(|at| (at.z - floor).abs() <= 1e-9) {
        stamp(&mut cut, q(at.x, at.y), radius);
    }
    let left: Vec<Vec2> = (0..nx * ny).filter(|&k| reachable[k] && !cut[k]).map(|k| cell(k / ny, k % ny)).collect();
    assert!(left.is_empty(), "{} places the tool can reach are left uncut at the floor, the first at {:?}", left.len(), left.first());
    reachable.iter().filter(|&&r| r).count()
}

#[test]
fn a_rectangular_pocket_is_cleared_in_rings_from_the_inside_out() {
    // A pocket 60 x 40 about the origin, its floor at Z -7 under the stock's top at 0. The 10 mm
    // tool's centre stays 5 from the walls: a 50 x 30 rectangle. Rings 4 apart (40% of the tool):
    // 50 x 30, 42 x 22, 34 x 14 and 26 x 6, the next would be 18 x -2. Two levels of 3.5 (at
    // most half the tool).
    let wall = rect(-30.0, -20.0, 30.0, 20.0);
    let done = pocketed(&one([0.0, 0.0]), &[pocket(&wall, &[])]).unwrap();
    // Innermost first.
    assert_eq!(done.levels, vec![vec![-3.5, -7.0]]);
    assert!(
        done.rings.len() == 1 && done.rings[0].len() == 4 && done.rings[0].iter().zip([64.0, 96.0, 128.0, 160.0]).all(|(&a, b)| close(a, b)),
        "{:?}",
        done.rings
    );
    assert_eq!((done.top, done.stepover, done.stepdown, done.offset), (0.0, 4.0, 5.0, 5.0));
    // A cut as wide as the tool: 0.15 mm a revolution, half of it down the ramp and straight down.
    assert!(close(done.rpm, RPM) && close(done.feed, 0.15 * RPM) && close(done.ramp_feed, 0.075 * RPM) && close(done.plunge_feed, 0.075 * RPM));
    check(&done, &wall, &[], -7.0);
    // At each level: down the innermost ring (64 round) in two turns, since 3.5 mm at 3 degrees
    // takes 66.8 mm; once round it level; then out to each ring in turn, 4 mm at a time.
    // Measured: the ramp is the hypotenuse of 128 along and 3.5 down. Before it the tool comes
    // down in the air at the plunge feed from the retract height: 5 mm, then 8.5.
    let s = done.toolpath.stats().unwrap();
    let level = 128.0_f64.hypot(3.5) + 64.0 + 3.0 * 4.0 + 96.0 + 128.0 + 160.0;
    assert!(close(s.feed_length, 2.0 * level + 5.0 + 8.5), "{} {}", s.feed_length, 2.0 * level + 13.5);
    // Rapids: 20 down to the retract height, 8.5 back up to it after the first level, 12 across
    // to the innermost ring again (three steps of 4, in a line), 12 up after the second, 20 up.
    assert!(close(s.rapid_length, 72.5), "{}", s.rapid_length);
    // Climb: every ring counter-clockwise, the wall on the tool's right.
    let mut at = done.toolpath.start;
    let mut turned = 0.0;
    for m in &done.toolpath.moves {
        let to = m.end(at);
        if matches!(m, Move::Linear { .. }) && to.z == at.z {
            turned += at.x * to.y - to.x * at.y;
        }
        at = to;
    }
    assert!(turned > 0.0, "{turned}");
    // The other way round on request, and nothing else changes.
    let other = pocketed(&PocketOp { direction: CutDirection::Conventional, ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    check(&other, &wall, &[], -7.0);
    assert!(close(other.toolpath.stats().unwrap().feed_length, s.feed_length));
}

#[test]
fn an_island_is_gone_round_and_never_cut_into() {
    // A pocket 80 x 60 with a 20 x 20 island in its middle. The tool's centre keeps 5 from the
    // wall and 5 from the island. Two stepovers in, the room above and below the island is
    // gone and the region is two strips, left and right: each is entered on its own.
    let wall = rect(-40.0, -30.0, 40.0, 30.0);
    let island = vec![rect(-10.0, -10.0, 10.0, 10.0)];
    let done = pocketed(&one([-30.0, 0.0]), &[pocket(&wall, &island)]).unwrap();
    check(&done, &wall, &island, -7.0);
    // Six rings: the wall's and the island's, one stepover in from each, and the two strips.
    // The ring round the island is 80 of straight sides and a full turn of radius 5, the next
    // one out the same sides and a turn of radius 9; along the wall a 70 x 50 rectangle and
    // inside it one of 62 x 42.
    let passes = &done.rings[0];
    assert_eq!(passes.len(), 6, "{passes:?}");
    for length in [80.0 + 10.0 * PI, 80.0 + 18.0 * PI, 240.0, 208.0] {
        assert!(passes.iter().any(|&l| (l - length).abs() < 1e-6), "no ring {length} mm round among {passes:?}");
    }
    // The rings that end at the wall and at the island are cut last of theirs.
    assert!(close(passes[5], 240.0) || close(passes[5], 80.0 + 10.0 * PI), "{passes:?}");
    // Round the island the tool goes clockwise (climb: the island on its right).
    let mut at = done.toolpath.start;
    let mut corners = 0;
    for m in &done.toolpath.moves {
        if let Move::Arc { center, direction, .. } = *m
            && center.x.abs() <= 10.0 + 1e-9
            && center.y.abs() <= 10.0 + 1e-9
        {
            corners += 1;
            assert_eq!(direction, ArcDirection::Clockwise, "round the island's corner at {center:?}");
        }
        at = m.end(at);
    }
    assert!(corners >= 8 && at.z == 25.0, "{corners}");
    // Three ways in at each level (the two strips and the island's own rings), each from above.
    let entries = done
        .toolpath
        .moves
        .windows(2)
        .filter(|w| matches!(w, [Move::Rapid { .. }, Move::Linear { feed, .. }] if close(*feed, done.plunge_feed)))
        .count();
    assert_eq!(entries, 2 * 3);
}

#[test]
fn an_island_near_a_wall_leaves_one_ring_that_runs_along_both() {
    // The same pocket with a 10 x 10 island 12 from its right wall. For the tool's centre the
    // gap between the island and that wall is 2 wide; one stepover in it is shut, and the ring
    // there runs along the wall's ring on one side and the island's on the other. The rings
    // inside it lead to it wherever they end, and it leads on to whichever is nearest.
    let wall = rect(-40.0, -30.0, 40.0, 30.0);
    for island in [rect(18.0, -5.0, 28.0, 5.0), rect(-28.0, 14.0, -18.0, 24.0), rect(-5.0, -29.0, 5.0, -19.0)] {
        let islands = vec![island.clone()];
        for direction in [CutDirection::Climb, CutDirection::Conventional] {
            let done =
                pocketed(&PocketOp { direction, ..one([0.0, 20.0]) }, &[pocket(&wall, &islands)]).unwrap_or_else(|e| panic!("{island:?}: {e}"));
            check(&done, &wall, &islands, -7.0);
            // Every way between rings is one stepover long: the feed moves at the floor that
            // are not part of a ring.
            assert!(done.rings[0].len() >= 6, "{:?}", done.rings);
        }
    }
    // Two islands and a stepover that divides nothing evenly: a narrow one exactly a tool's
    // width from the left wall (a gap the tool is not sent through), and one in a corner, too
    // near both walls to go round.
    let islands = vec![rect(-30.0, -4.0, -8.0, 4.0), rect(20.0, 10.0, 34.0, 24.0)];
    let done = pocketed(&PocketOp { stepover: Some(3.3), ..one([0.0, -20.0]) }, &[pocket(&wall, &islands)]).unwrap();
    check(&done, &wall, &islands, -7.0);
    // The tool can't go round either island, and that is said: what it can't reach stays.
    assert!(done.notes.len() == 1 && done.notes[0].contains("tool 2 does not pass everywhere in the pocket at X 0 Y -20"), "{:?}", done.notes);
    // Where it does pass everywhere, nothing is noted: one island with 12 mm on its tightest side.
    let roomy = vec![rect(18.0, -5.0, 28.0, 5.0)];
    assert!(pocketed(&one([0.0, 20.0]), &[pocket(&wall, &roomy)]).unwrap().notes.is_empty());
}

#[test]
fn random_pockets_with_islands_are_cleared_without_ever_touching_a_wall() {
    // Rectangular pockets with up to three islands placed at random (never touching one another
    // or the wall), cut with tools and stepovers picked at random, both ways round. Each one
    // either comes back refused for a reason of its own (the tool does not fit) or is measured:
    // never nearer a wall or an island than the tool's radius, and nothing reachable left uncut.
    let mut seed = 0x2545_f491_4f6c_dd1d_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut cut, mut refused, mut looked_at) = (0, 0, 0);
    for case in 0..120 {
        let (half_x, half_y) = (12.0 + (next() * 30.0).round(), 10.0 + (next() * 22.0).round());
        let wall = rect(-half_x, -half_y, half_x, half_y);
        let mut islands: Vec<Vec<(f64, f64)>> = Vec::new();
        for _ in 0..(next() * 4.0) as usize {
            let (w, h) = (2.0 + (next() * 14.0).round(), 2.0 + (next() * 12.0).round());
            let (x, y) = (((next() * 2.0 - 1.0) * (half_x - w - 1.0)).round(), ((next() * 2.0 - 1.0) * (half_y - h - 1.0)).round());
            let island = rect(x - 0.5 * w, y - 0.5 * h, x + 0.5 * w, y + 0.5 * h);
            let apart = |a: &Vec<(f64, f64)>, b: &Vec<(f64, f64)>| {
                a[0].0 > b[2].0 + 0.5 || b[0].0 > a[2].0 + 0.5 || a[0].1 > b[2].1 + 0.5 || b[0].1 > a[2].1 + 0.5
            };
            let inside_wall = island[0].0 > -half_x + 0.5 && island[2].0 < half_x - 0.5 && island[0].1 > -half_y + 0.5 && island[2].1 < half_y - 0.5;
            if inside_wall && islands.iter().all(|other| apart(other, &island)) {
                islands.push(island);
            }
        }
        let diameter = [4.0, 6.0, 8.0, 10.0, 12.0][(next() * 5.0) as usize % 5];
        let mut tool = mill10();
        tool.geometry.diameter = diameter;
        tool.geometry.shank_diameter = diameter;
        let op = PocketOp {
            stepover: Some(((0.15 + 0.35 * next()) * diameter * 100.0).round() / 100.0),
            direction: if next() < 0.5 { CutDirection::Climb } else { CutDirection::Conventional },
            stepdown: Some(7.0),
            ..one([0.0, 0.0])
        };
        match op.toolpath(None, &tool, &placed(), Some("aluminium"), &[pocket(&wall, &islands)]) {
            Ok(done) => {
                cut += 1;
                looked_at += check_with(&done, &wall, &islands, -7.0, 0.5 * diameter);
            }
            Err(e) => {
                refused += 1;
                let why = e.to_string();
                assert!(
                    why.contains("does not fit") || why.contains("turns"),
                    "case {case} ({wall:?}, {islands:?}, a {diameter} mm tool, {op:?}): {why}"
                );
            }
        }
    }
    // (1500 of these were run once when this was written: 1466 cut, 34 too narrow for their tool.)
    assert!(cut >= 100 && looked_at > 500_000, "{cut} cut, {refused} refused, {looked_at} places looked at");
}

#[test]
fn a_pocket_that_pinches_is_entered_on_each_side_of_the_neck() {
    // Two 30 x 30 squares joined by a neck 12 wide. The tool's centre keeps 5 from every wall:
    // two 20 x 20 squares and a neck 2 wide, its four corners rolled round the neck's own corners
    // at (±10, ±6). One stepover in (9 from the walls) the neck is shut and there are two parts;
    // two in (13), two smaller ones; three in, nothing. Five rings, and the tool comes down
    // twice at each level.
    let wall = vec![
        (-40.0, -15.0),
        (-10.0, -15.0),
        (-10.0, -6.0),
        (10.0, -6.0),
        (10.0, -15.0),
        (40.0, -15.0),
        (40.0, 15.0),
        (10.0, 15.0),
        (10.0, 6.0),
        (-10.0, 6.0),
        (-10.0, 15.0),
        (-40.0, 15.0),
    ];
    let done = pocketed(&one([-25.0, 0.0]), &[pocket(&wall, &[])]).unwrap();
    check(&done, &wall, &[], -7.0);
    // Each side's rings are not plain squares: towards the neck the centre may go on until it is
    // the ring's distance from the neck's two corners, so the side there is two arcs about them
    // meeting on the middle line.
    //  - 13 from the walls: a 4 mm side at X ∓27, the top and bottom (Y ±2) from there to the
    //    arcs at X ∓(10 + sqrt(169 - 16)), and the arcs, each from the direction (sqrt(153), 4)
    //    to (sqrt(133), 6) seen from its corner.
    //  - 9 from the walls: three 12 mm sides, and arcs from the square's own corners (straight
    //    out from the neck's corners) to the direction (sqrt(45), 6).
    //  - 5 from the walls, the outline: the squares' 80 each less the 2 mm of the neck, the
    //    neck's two sides of 30, and at each of its corners a quarter turn of 5 for 10 of sides.
    let inner = 4.0 + 2.0 * (17.0 - 153.0_f64.sqrt()) + 2.0 * 13.0 * (6.0_f64.atan2(133.0_f64.sqrt()) - 4.0_f64.atan2(153.0_f64.sqrt()));
    let middle = 36.0 + 2.0 * 9.0 * 6.0_f64.atan2(45.0_f64.sqrt());
    let outline = 2.0 * 78.0 + 60.0 - 4.0 * (10.0 - 2.5 * PI);
    let passes = &done.rings[0];
    assert!(
        passes.len() == 5 && passes.iter().zip([inner, middle, inner, middle, outline]).all(|(&got, want)| (got - want).abs() < 1e-6),
        "{passes:?}, not {:?}",
        [inner, middle, outline]
    );
    let entries = done
        .toolpath
        .moves
        .windows(2)
        .filter(|w| matches!(w, [Move::Rapid { .. }, Move::Linear { feed, .. }] if close(*feed, done.plunge_feed)))
        .count();
    assert_eq!(entries, 2 * 2);
    // The 10 mm tool passes the 12 mm neck. A 14 mm tool fits each square and not the neck: the
    // squares are cleared, the neck keeps its material, and that is said.
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    let mut wide = mill10();
    wide.geometry.diameter = 14.0;
    wide.geometry.shank_diameter = 14.0;
    let left = one([-25.0, 0.0]).toolpath(None, &wide, &placed(), Some("aluminium"), &[pocket(&wall, &[])]).unwrap();
    assert!(left.notes.iter().any(|note| note.contains("does not pass everywhere")), "{:?}", left.notes);
    assert!(check_with(&left, &wall, &[], -7.0, 7.0) > 100);
}

#[test]
fn a_round_pocket_is_cut_in_circles_and_entered_down_a_helix() {
    // A round pocket of radius 20: the centre's circles are 15, 11, 7 and 3. The innermost is
    // 6 pi round, so 3.5 mm at 3 degrees takes four turns of it.
    let circle = Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 20.0, start: 0.0, sweep: 2.0 * PI }] };
    let shape = PocketShape { floor: -7.0, region: Region2 { outer: circle, holes: Vec::new() }, open: None };
    let done = pocketed(&one([0.0, 0.0]), &[shape]).unwrap();
    assert!(done.rings[0].iter().zip([3.0, 7.0, 11.0, 15.0]).all(|(&l, r)| close(l, 2.0 * PI * r)) && done.rings[0].len() == 4, "{:?}", done.rings);
    let s = done.toolpath.stats().unwrap();
    let level = (4.0 * 6.0 * PI).hypot(3.5) + 6.0 * PI + 3.0 * 4.0 + 2.0 * PI * (7.0 + 11.0 + 15.0);
    assert!(close(s.feed_length, 2.0 * level + 13.5), "{} {}", s.feed_length, 2.0 * level + 13.5);
    // Every cutting move but the ways between rings is a counter-clockwise arc about the middle,
    // and the helix's arcs each end lower than they start.
    let mut at = done.toolpath.start;
    let (mut arcs, mut falling) = (0, 0);
    for m in &done.toolpath.moves {
        let to = m.end(at);
        if let Move::Arc { center, direction, .. } = *m {
            arcs += 1;
            assert!(direction == ArcDirection::CounterClockwise && center.x.abs() <= 1e-9 && center.y.abs() <= 1e-9, "{m:?}");
            assert!(close(at.x.hypot(at.y), to.x.hypot(to.y)), "{at:?} {to:?}");
            falling += usize::from(to.z < at.z);
        }
        at = to;
    }
    assert!(arcs >= 2 * (8 + 4 * 2) && falling >= 2 * 8, "{arcs} arcs, {falling} of them falling");
    let lowest = fed_points(&done.toolpath).iter().map(|at| at.z).fold(f64::INFINITY, f64::min);
    assert_eq!(lowest, -7.0);
}

#[test]
fn several_pockets_are_cut_one_after_another_each_to_its_own_floor() {
    let left = rect(-45.0, -15.0, -15.0, 15.0);
    let right = rect(15.0, -15.0, 45.0, 15.0);
    let shapes = [pocket(&left, &[]), PocketShape { floor: -3.0, ..pocket(&right, &[]) }];
    let op = PocketOp { pockets: vec![[-30.0, 0.0], [30.0, 0.0]], stepdown: Some(2.0), ..PocketOp::default() };
    let done = pocketed(&op, &shapes).unwrap();
    // 7 mm in steps of at most 2 is four levels of 1.75; 3 mm is two of 1.5.
    assert_eq!(done.levels, vec![vec![-1.75, -3.5, -5.25, -7.0], vec![-1.5, -3.0]]);
    // Each 30 x 30 pocket: the centre's 20 x 20, then 12 x 12 and 4 x 4.
    assert!(
        done.rings.len() == 2 && done.rings.iter().all(|r| r.len() == 3 && close(r[0], 16.0) && close(r[1], 48.0) && close(r[2], 80.0)),
        "{:?}",
        done.rings
    );
    let points = fed_points(&done.toolpath);
    assert!(points.iter().all(|at| if at.x < 0.0 { at.z >= -7.0 - 1e-9 } else { at.z >= -3.0 - 1e-9 }));
    assert!(points.iter().all(|at| room(q(at.x, at.y), &left, &[]).max(room(q(at.x, at.y), &right, &[])) >= 5.0 - 1e-4));
    // Between the two the tool is up at the retract height.
    let mut at = done.toolpath.start;
    for m in &done.toolpath.moves {
        if let Move::Rapid { to } = m
            && (to.x != at.x || to.y != at.y)
        {
            assert!(at.z >= 5.0 && to.z >= 5.0, "a move across the part below the retract height: {at:?} to {to:?}");
        }
        at = m.end(at);
    }
    // Stock left on the walls moves every ring in; a stepover and a ramp angle given are used.
    let op = PocketOp { leave: Some(0.5), stepover: Some(5.0), ramp_angle: Some(10.0_f64.to_radians()), ..one([-30.0, 0.0]) };
    let done = pocketed(&op, &[pocket(&left, &[])]).unwrap();
    // The centre's square is 19 x 19; 5 in, 9 x 9; 10 in, nothing.
    assert_eq!((done.offset, done.stepover), (5.5, 5.0));
    assert!(done.rings[0].len() == 2 && close(done.rings[0][0], 36.0) && close(done.rings[0][1], 76.0), "{:?}", done.rings);
    let tightest = fed_points(&done.toolpath).iter().map(|at| room(q(at.x, at.y), &left, &[])).fold(f64::INFINITY, f64::min);
    assert!((tightest - 5.5).abs() <= 1e-4, "{tightest}");
}

#[test]
fn what_cannot_be_pocketed_is_refused_with_the_reason() {
    let wall = rect(-30.0, -20.0, 30.0, 20.0);
    let shape = pocket(&wall, &[]);
    let drill = {
        let mut t = mill10();
        t.kind = ToolKind::Drill;
        t.geometry.tip_angle = 2.0;
        t
    };
    let cases: Vec<(PocketOp, Tool, &str)> = vec![
        (one([0.0, 0.0]), drill, "does not cut a pocket"),
        (PocketOp::default(), mill10(), "cuts 1 to 1000 pockets"),
        (PocketOp { stepover: Some(5.1), ..one([0.0, 0.0]) }, mill10(), "half of tool 2's diameter"),
        (PocketOp { stepover: Some(0.0), ..one([0.0, 0.0]) }, mill10(), "stepover"),
        (PocketOp { stepdown: Some(0.0), ..one([0.0, 0.0]) }, mill10(), "stepdown"),
        (PocketOp { stepdown: Some(0.001), ..one([0.0, 0.0]) }, mill10(), "more than 1000 levels"),
        (PocketOp { leave: Some(-0.1), ..one([0.0, 0.0]) }, mill10(), "stock to leave"),
        (PocketOp { finish: Some(-0.1), ..one([0.0, 0.0]) }, mill10(), "finishing pass must take"),
        (PocketOp { finish: Some(f64::NAN), ..one([0.0, 0.0]) }, mill10(), "finishing pass must take"),
        (PocketOp { finish: Some(f64::INFINITY), ..one([0.0, 0.0]) }, mill10(), "finishing pass must take"),
        (PocketOp { ramp_angle: Some(1.0), ..one([0.0, 0.0]) }, mill10(), "ramp angle"),
        (PocketOp { top: Some(-7.0), ..one([0.0, 0.0]) }, mill10(), "is not below the top"),
        (PocketOp { top: Some(10.0), ..one([0.0, 0.0]) }, mill10(), "retract height"),
        (PocketOp { top: Some(f64::NAN), ..one([0.0, 0.0]) }, mill10(), "finite"),
        (PocketOp { rpm: Some(0.0), ..one([0.0, 0.0]) }, mill10(), "rpm"),
        (PocketOp { feed: Some(-1.0), ..one([0.0, 0.0]) }, mill10(), "feed"),
        (
            PocketOp { pockets: vec![[0.0, 0.0], [1.0, 1.0]], ..PocketOp::default() },
            mill10(),
            "1 pockets were read from the model for the operation's 2 points",
        ),
    ];
    for (op, tool, expect) in cases {
        let e = op.toolpath(None, &tool, &placed(), Some("aluminium"), std::slice::from_ref(&shape)).unwrap_err().to_string();
        assert!(e.contains(expect), "{op:?}: {e:?} does not mention {expect:?}");
    }
    // A pocket too narrow for the tool says how wide it is: 8 mm here.
    let narrow = pocket(&rect(-30.0, -4.0, 30.0, 4.0), &[]);
    let e = pocketed(&one([0.0, 0.0]), &[narrow]).unwrap_err().to_string();
    assert!(e.contains("tool 2 (10 mm across) does not fit") && e.contains("about 8.0") && e.contains("X 0 Y 0"), "{e}");
    // A pocket deeper than the flutes reach.
    let deep = PocketShape { floor: -23.0, ..pocket(&wall, &[]) };
    let e = pocketed(&one([0.0, 0.0]), &[deep]).unwrap_err().to_string();
    assert!(e.contains("more than the 22 mm tool 2 reaches"), "{e}");
    // A round pocket a hair wider than the tool: the ring to ramp down is too short to be one.
    let tight = PocketShape {
        floor: -7.0,
        region: Region2 {
            outer: Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 5.01, start: 0.0, sweep: 2.0 * PI }] },
            holes: Vec::new(),
        },
        open: None,
    };
    let e = pocketed(&one([0.0, 0.0]), &[tight]).unwrap_err().to_string();
    assert!(e.contains("turns") && e.contains("narrower tool or a steeper ramp"), "{e}");
    // No material and nothing typed in: the error names both ways out.
    let e = one([0.0, 0.0]).toolpath(None, &mill10(), &placed(), None, &[shape]).unwrap_err().to_string();
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
}

#[test]
fn hostile_numbers_never_panic() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, -1e300, 1e300, -1.0, 0.0, 1e-320, 5e-324, f64::MAX, f64::MIN];
    let wall = rect(-30.0, -20.0, 30.0, 20.0);
    let shape = pocket(&wall, &[]);
    for &v in &bad {
        for field in 0..10 {
            let mut op = one([0.0, 0.0]);
            match field {
                0 => op.top = Some(v),
                1 => op.stepdown = Some(v),
                2 => op.stepover = Some(v),
                3 => op.leave = Some(v),
                4 => op.ramp_angle = Some(v),
                5 => op.rpm = Some(v),
                6 => op.feed = Some(v),
                7 => op.finish = Some(v),
                8 => op.rest = Some(v),
                _ => op.pockets = vec![[v, v]],
            }
            let _ = pocketed(&op, std::slice::from_ref(&shape));
            // And each with a finishing pass asked for, and after a wider tool.
            let _ = pocketed(&PocketOp { finish: op.finish.or(Some(0.3)), ..op.clone() }, std::slice::from_ref(&shape));
            let _ = pocketed(&PocketOp { rest: op.rest.or(Some(16.0)), ..op }, std::slice::from_ref(&shape));
        }
        // Shapes of hostile numbers: a floor, a point, a spike, a broken loop, a hostile island.
        let shapes = [
            PocketShape { floor: v, ..shape.clone() },
            pocket(&[(v, v), (v, v), (v, v)], &[]),
            pocket(&[(0.0, 0.0), (v, 0.0), (0.0, v)], &[]),
            PocketShape { floor: -7.0, region: Region2 { outer: Loop2 { segs: vec![] }, holes: Vec::new() }, open: None },
            pocket(&wall, &[vec![(v, v), (1.0, v), (v, 1.0)]]),
            PocketShape {
                floor: -7.0,
                region: Region2 { outer: Loop2 { segs: vec![Seg2::Arc { center: q(v, 0.0), radius: v, start: v, sweep: v }] }, holes: Vec::new() },
                open: None,
            },
        ];
        for odd in shapes {
            let _ = pocketed(&one([0.0, 0.0]), std::slice::from_ref(&odd));
            let _ = pocketed(&PocketOp { rest: Some(16.0), ..one([0.0, 0.0]) }, std::slice::from_ref(&odd));
            let _ = pocketed(&PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) }, &[odd]);
        }
        // A tool of hostile sizes: never a path outside the region.
        for size in 0..3 {
            let mut odd = mill10();
            match size {
                0 => odd.geometry.diameter = v,
                1 => odd.geometry.flute_length = v,
                _ => odd.geometry.shoulder_length = v,
            }
            for finish in [None, Some(0.3)] {
                if let Ok(done) =
                    (PocketOp { finish, ..one([0.0, 0.0]) }).toolpath(None, &odd, &placed(), Some("aluminium"), std::slice::from_ref(&shape))
                {
                    assert!(done.offset > 0.0 && done.offset.is_finite(), "{v}: {}", done.offset);
                }
            }
        }
    }
}

#[test]
fn a_pocket_operation_is_plain_json() {
    let op: PocketOp = serde_json::from_str(r#"{"pockets": [[1, 2]]}"#).unwrap();
    assert_eq!(op, one([1.0, 2.0]));
    let op: PocketOp =
        serde_json::from_str(r#"{"pockets": [[1, 2], [3, 4]], "direction": "conventional", "stepover": 3, "leave": 0.2, "ramp_angle": 0.1}"#)
            .unwrap();
    assert_eq!(
        (op.pockets.len(), op.direction, op.stepover, op.leave, op.ramp_angle, op.finish),
        (2, CutDirection::Conventional, Some(3.0), Some(0.2), Some(0.1), None)
    );
    let finished: PocketOp = serde_json::from_str(r#"{"pockets": [[1, 2]], "finish": 0.3}"#).unwrap();
    assert_eq!(finished, PocketOp { finish: Some(0.3), ..one([1.0, 2.0]) });
    assert!(serde_json::from_str::<PocketOp>(r#"{"pockets": [[1, 2]], "helix": 4}"#).is_err());
    assert!(serde_json::from_str::<PocketOp>("{}").is_err(), "a pocket operation names its pockets");
    assert_eq!(serde_json::from_value::<PocketOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}

/// The distance from `at` to the side of the polygon through `points` that starts at its
/// corner `side`.
fn off_side(at: Vec2, points: &[(f64, f64)], side: usize) -> f64 {
    let n = points.len();
    distance(at, &[points[side % n], points[(side + 1) % n], points[side % n]])
}

/// Where the tool goes at rapid at the retract height (Z 5), in order.
fn rapids_at_retract(path: &Toolpath) -> Vec<Vec2> {
    path.moves
        .iter()
        .filter_map(|m| if let Move::Rapid { to } = *m { Some(to) } else { None })
        .filter(|to| to.z == 5.0)
        .map(|to| q(to.x, to.y))
        .collect()
}

#[test]
fn a_finishing_pass_takes_what_the_rings_left_on_the_walls() {
    // The pocket 60 x 40 about the origin, its floor at Z -7. Without a finishing pass nothing
    // is other than it was.
    let wall = rect(-30.0, -20.0, 30.0, 20.0);
    let plain = pocketed(&one([0.0, 0.0]), &[pocket(&wall, &[])]).unwrap();
    assert_eq!((plain.finish, plain.offset, plain.finishing.clone(), plain.finish_levels.clone()), (0.0, 5.0, vec![vec![]], vec![vec![]]));
    assert_eq!(plain.finish_leads, vec![Vec::<f64>::new()]);
    // With 0.3 left for one: the rings keep 5.3 from the walls, 49.4 x 29.4 and, 4 apart,
    // 41.4 x 21.4, 33.4 x 13.4 and 25.4 x 5.4; the pass runs 5 from the walls, the 50 x 30
    // rectangle a pocket without one ends on, once, at the floor, met and left by quarter
    // turns of 1 mm (a tenth of the tool's diameter, as a contour's).
    let done = pocketed(&PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    assert_eq!((done.finish, done.offset, done.finish_levels.clone(), done.finish_leads.clone()), (0.3, 5.3, vec![vec![-7.0]], vec![vec![1.0]]));
    assert!(done.rings.len() == 1 && done.rings[0].iter().zip([61.6, 93.6, 125.6, 157.6]).all(|(&a, b)| close(a, b)), "{:?}", done.rings);
    assert!(done.finishing.len() == 1 && done.finishing[0].len() == 1 && close(done.finishing[0][0], 160.0), "{:?}", done.finishing);
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    // Measured: everything the tool can reach is cut at the floor, and its centre is never
    // nearer a wall than its radius.
    check(&done, &wall, &[], -7.0);
    // At the first level only the rings cut, and they keep the finishing stock more from every
    // wall; at the floor the pass comes to 5 from each of the four.
    let points = fed_points(&done.toolpath);
    let at_level = |z: f64| points.iter().filter(move |at| (at.z - z).abs() <= 1e-9).map(|at| q(at.x, at.y));
    let tightest = at_level(-3.5).map(|at| room(at, &wall, &[])).fold(f64::INFINITY, f64::min);
    assert!((tightest - 5.3).abs() <= 1e-4, "the rings come within {tightest} mm of a wall");
    for side in 0..4 {
        let nearest = at_level(-7.0).map(|at| off_side(at, &wall, side)).fold(f64::INFINITY, f64::min);
        assert!((nearest - 5.0).abs() <= 1e-6, "side {side}: the pass comes within {nearest} mm");
    }
    // The pass starts at the middle of the front side, going left to right (climb, the wall
    // on the tool's right). The tool comes down 1 mm behind and 1 mm in from there, in air the
    // rings cut (0.7 inside where the outermost ring ran), and goes up 1 mm ahead and 1 mm in.
    let rapids = rapids_at_retract(&done.toolpath);
    let (down, up) = (rapids[rapids.len() - 2], rapids[rapids.len() - 1]);
    assert!(close(down.x, -1.0) && close(down.y, -14.0) && close(up.x, 1.0) && close(up.y, -14.0), "{down:?} {up:?}");
    // What is fed: at each level down the innermost ring (61.6 round) in two turns, once round
    // it, and out to each ring in turn, 4 mm at a time; the plunges of 5 and 8.5 before the two
    // ramps; and the pass: 12 down from the retract height in air the rings cut, a quarter
    // turn of 1 mm on to the wall, 160 round, and a quarter turn off it.
    let s = done.toolpath.stats().unwrap();
    let level = 123.2_f64.hypot(3.5) + 61.6 + 3.0 * 4.0 + 93.6 + 125.6 + 157.6;
    assert!(close(s.feed_length, 2.0 * level + 13.5 + 12.0 + PI + 160.0), "{} {}", s.feed_length, 2.0 * level + 185.5 + PI);
    // The pass is fed as the rings are: along a wall it is a far lighter cut, but in a corner
    // of the pocket it is not, and there it has half the feed. Every move at the floor has the
    // one cutting feed, but for the last 0.3 mm into each of the pass's four corners (where
    // the tool is in the stock of the wall ahead too); the 160 mm along the walls are among
    // them.
    let corners = [(25.0, -15.0), (25.0, 15.0), (-25.0, 15.0), (-25.0, -15.0)];
    let mut at = done.toolpath.start;
    let (mut along_wall, mut slowed) = (0.0, 0.0);
    for m in &done.toolpath.moves {
        let to = m.end(at);
        if let Move::Linear { feed, .. } | Move::Arc { feed, .. } = *m
            && to.z == -7.0
            && at.z == -7.0
        {
            let length = (to.x - at.x).hypot(to.y - at.y);
            if feed != done.feed {
                assert!(
                    close(feed, 0.5 * done.feed) && close(length, 0.3) && corners.iter().any(|&(x, y)| close(to.x, x) && close(to.y, y)),
                    "{m:?} from {at:?}"
                );
                slowed += length;
            }
            if (room(q(at.x, at.y), &wall, &[]) - 5.0).abs() <= 1e-6 && (room(q(to.x, to.y), &wall, &[]) - 5.0).abs() <= 1e-6 {
                along_wall += length;
            }
        }
        at = to;
    }
    assert!(close(along_wall, 160.0) && close(slowed, 1.2), "{along_wall} {slowed}");
    // The other way round on request: the same lengths, down where the other went up.
    let other = pocketed(&PocketOp { finish: Some(0.3), direction: CutDirection::Conventional, ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    check(&other, &wall, &[], -7.0);
    assert!(close(other.toolpath.stats().unwrap().feed_length, s.feed_length));
    let rapids = rapids_at_retract(&other.toolpath);
    let (down, up) = (rapids[rapids.len() - 2], rapids[rapids.len() - 1]);
    assert!(close(down.x, 1.0) && close(down.y, -14.0) && close(up.x, -1.0) && close(up.y, -14.0), "{down:?} {up:?}");
    // With stock left on the walls for another operation too, the pass stops that much short.
    let left = pocketed(&PocketOp { finish: Some(0.3), leave: Some(0.5), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    assert!(close(left.offset, 5.8) && close(left.finishing[0][0], 2.0 * (49.0 + 29.0)), "{} {:?}", left.offset, left.finishing);
    let tightest = fed_points(&left.toolpath).iter().map(|at| room(q(at.x, at.y), &wall, &[])).fold(f64::INFINITY, f64::min);
    assert!((tightest - 5.5).abs() <= 1e-4, "{tightest}");
    // A pass that takes more than the quarter turns are wide has wider ones, so that they
    // still begin in air: 1 mm of stock, turns of 2.
    let heavy = pocketed(&PocketOp { finish: Some(1.0), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    assert_eq!((heavy.offset, heavy.finish_leads.clone(), heavy.finish_levels.clone()), (6.0, vec![vec![2.0]], vec![vec![-7.0]]));
    check(&heavy, &wall, &[], -7.0);
}

#[test]
fn a_finishing_pass_goes_round_islands_and_down_in_steps_where_the_flutes_are_short() {
    // The pocket 80 x 60 with an island 20 x 20 in its middle: the pass goes along the wall and
    // round the island, each once, and neither is cut into.
    let wall = rect(-40.0, -30.0, 40.0, 30.0);
    let island = rect(-10.0, -10.0, 10.0, 10.0);
    let done = pocketed(&PocketOp { finish: Some(0.25), ..one([-25.0, 0.0]) }, &[pocket(&wall, std::slice::from_ref(&island))]).unwrap();
    check(&done, &wall, std::slice::from_ref(&island), -7.0);
    // Along the wall: 70 x 50. Round the island the centre is 5 off its sides and turns round
    // its corners: 80 and a circle of radius 5. Both by quarter turns of 1 mm.
    assert!(close(done.finishing[0][0], 240.0) && close(done.finishing[0][1], 80.0 + 10.0 * PI), "{:?}", done.finishing);
    assert_eq!(done.finish_leads, vec![vec![1.0, 1.0]]);
    let at_floor: Vec<Vec2> = fed_points(&done.toolpath).iter().filter(|at| (at.z + 7.0).abs() <= 1e-9).map(|at| q(at.x, at.y)).collect();
    let (to_wall, to_island) =
        at_floor.iter().fold((f64::INFINITY, f64::INFINITY), |(w, i), at| (w.min(distance(*at, &wall)), i.min(distance(*at, &island))));
    assert!((to_wall - 5.0).abs() <= 1e-6 && (to_island - 5.0).abs() <= 1e-6, "{to_wall} {to_island}");
    // Along the wall as before: down at X -1 Y -24, up at X 1 Y -24. Round the island the
    // other way (the island on the tool's right): it starts under the island's front side,
    // going right to left, down at X 1 Y -16 and up at X -1 Y -16, 1 mm further from the
    // island than the pass.
    let rapids = rapids_at_retract(&done.toolpath);
    let n = rapids.len();
    let expected = [(-1.0, -24.0), (1.0, -24.0), (1.0, -16.0), (-1.0, -16.0)];
    assert!(rapids[n - 4..].iter().zip(expected).all(|(at, (x, y))| close(at.x, x) && close(at.y, y)), "{:?}", &rapids[n - 4..]);
    // A tool that reaches 30 down a wall on 12 mm of flutes, in a pocket 20 deep: the pass is
    // cut in two steps of 10, top down, each as high as the flutes can cut.
    let mut long = mill10();
    (long.geometry.flute_length, long.geometry.shoulder_length, long.geometry.overall_length) = (12.0, 30.0, 80.0);
    long.holder.stick_out = 40.0;
    let deep = PocketShape { floor: -20.0, ..pocket(&rect(-30.0, -20.0, 30.0, 20.0), &[]) };
    let op = PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) };
    let done = op.toolpath(None, &long, &placed(), Some("aluminium"), std::slice::from_ref(&deep)).unwrap();
    assert_eq!((done.levels.clone(), done.finish_levels.clone()), (vec![vec![-5.0, -10.0, -15.0, -20.0]], vec![vec![-10.0, -20.0]]));
    let s = done.toolpath.stats().unwrap();
    let plain =
        PocketOp { leave: Some(0.3), ..one([0.0, 0.0]) }.toolpath(None, &long, &placed(), Some("aluminium"), std::slice::from_ref(&deep)).unwrap();
    // The same rings as a pocket that leaves 0.3 for another operation, and the two steps of
    // the pass more: down 15 and 25 from the retract height, and twice the two quarter turns
    // and the 160 round.
    assert!(close(s.feed_length - plain.toolpath.stats().unwrap().feed_length, 15.0 + 25.0 + 2.0 * (PI + 160.0)), "{}", s.feed_length);
    // No moment of the pass is a bigger cut than a ring's, which is 10 wide and 5 deep here
    // (50 mm2). In each corner the tool's edge is in the stock over 90 degrees more than along
    // a wall (where it is 19.9 degrees, for 0.3 of stock): the tool is buried
    // 5 x (1 - cos 109.9) = 6.71 mm wide for a moment, at half the feed, which counts as 3.35 mm
    // at the whole feed. With flutes for the whole depth one step of 20 would be 67 mm2, so
    // the pass takes two steps here too; a pocket 10 deep goes in one (33.5 mm2).
    let mut whole = long.clone();
    whole.geometry.flute_length = 30.0;
    let stepped = op.toolpath(None, &whole, &placed(), Some("aluminium"), std::slice::from_ref(&deep)).unwrap();
    assert_eq!(stepped.finish_levels, vec![vec![-10.0, -20.0]]);
    let ten = PocketShape { floor: -10.0, ..deep.clone() };
    let one_step = op.toolpath(None, &whole, &placed(), Some("aluminium"), &[ten]).unwrap();
    assert_eq!((one_step.levels.clone(), one_step.finish_levels.clone()), (vec![vec![-5.0, -10.0]], vec![vec![-10.0]]));
    // 3 mm of stock (4.24 from where the rings went in a corner): the tool is buried
    // 5 x (1 - cos 156.4) = 9.58 mm there, as 4.79: two steps of 10 (47.9 mm2 each).
    let thick =
        PocketOp { finish: Some(3.0), ..one([0.0, 0.0]) }.toolpath(None, &whole, &placed(), Some("aluminium"), std::slice::from_ref(&deep)).unwrap();
    assert_eq!((thick.finish_levels.clone(), thick.finish_leads.clone()), (vec![vec![-10.0, -20.0]], vec![vec![6.0]]));
    // A round pocket has no corners. Along its wall (radius 25, the tool's centre on a circle
    // of 20) the cut is 0.37 mm wide, hardly more than the 0.3 along a straight wall, and 20
    // deep goes in one step (7.5 mm2), the tool never further out than that circle.
    let circle = |floor: f64| PocketShape {
        floor,
        region: Region2 {
            outer: Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 25.0, start: 0.0, sweep: 2.0 * PI }] },
            holes: Vec::new(),
        },
        open: None,
    };
    let round = op.toolpath(None, &whole, &placed(), Some("aluminium"), &[circle(-20.0)]).unwrap();
    assert_eq!((round.finish_levels.clone(), round.finish_leads.clone()), (vec![vec![-20.0]], vec![vec![1.0]]));
    let furthest = fed_points(&round.toolpath).iter().map(|at| at.x.hypot(at.y)).fold(0.0, f64::max);
    assert!(close(round.finishing[0][0], 40.0 * PI) && (furthest - 20.0).abs() <= 1e-6, "{:?} {furthest}", round.finishing);
    // 35 deep, with flutes for it, it takes two steps: none is deeper than three diameters.
    let mut longer = whole.clone();
    (longer.geometry.flute_length, longer.geometry.shoulder_length, longer.geometry.overall_length) = (40.0, 0.0, 100.0);
    longer.holder.stick_out = 50.0;
    let deeper = op.toolpath(None, &longer, &placed(), Some("aluminium"), &[circle(-35.0)]).unwrap();
    assert_eq!(deeper.finish_levels, vec![vec![-17.5, -35.0]]);
}

/// The length fed at the floor (Z -7) at half of `done`'s cutting feed, and how many such moves
/// end at none of `corners`.
fn slowed_into(done: &Pocketed, corners: &[(f64, f64)]) -> (f64, usize) {
    let mut at = done.toolpath.start;
    let (mut slowed, mut elsewhere) = (0.0, 0);
    for m in &done.toolpath.moves {
        let to = m.end(at);
        if let Move::Linear { feed, .. } | Move::Arc { feed, .. } = *m
            && to.z == -7.0
            && at.z == -7.0
            && close(feed, 0.5 * done.feed)
        {
            slowed += (to.x - at.x).hypot(to.y - at.y);
            elsewhere += usize::from(!corners.iter().any(|&(x, y)| (to.x - x).abs() <= 1e-6 && (to.y - y).abs() <= 1e-6));
        }
        at = to;
    }
    (slowed, elsewhere)
}

#[test]
fn a_finishing_pass_slows_sooner_into_a_sharper_corner() {
    // A pocket that is half a square of 60: a right angle and two corners of 45 degrees. The
    // tool's centre goes round the triangle 5 inside it, whose sharp corners are 5 / tan 22.5
    // = 12.07 short of the pocket's.
    let half = vec![(-30.0, -20.0), (30.0, -20.0), (-30.0, 40.0)];
    let sharp = 30.0 - 5.0 / (PI / 8.0).tan();
    let corners = [(-25.0, -15.0), (sharp, -15.0), (-25.0, sharp + 10.0)];
    let done = pocketed(&PocketOp { finish: Some(0.3), ..one([-10.0, 0.0]) }, &[pocket(&half, &[])]).unwrap();
    check(&done, &half, &[], -7.0);
    // In a corner of 45 degrees the rings turned 0.3 / sin 22.5 = 0.78 mm short of where the
    // pass turns (2.6 times the stock, of the four a corner may account for), and the pass is
    // beyond their corner for its last 0.3 x tan 67.5 = 0.72 mm: that far it has half the
    // feed. Into the right angle, the last 0.3. One step all the same: buried
    // 5 x (1 - cos 154.9) = 9.53 mm in a sharp corner, as 4.76 at the whole feed, and 7 deep
    // that is 33.3 mm2 of the 35 a ring is.
    assert!(
        done.finish_levels == vec![vec![-7.0]] && done.finish_leads == vec![vec![1.0]] && done.notes.is_empty(),
        "{:?} {:?}",
        done.finish_levels,
        done.notes
    );
    let (slowed, elsewhere) = slowed_into(&done, &corners);
    assert!(close(slowed, 0.3 + 2.0 * 0.3 * (3.0 * PI / 8.0).tan()) && elsewhere == 0, "{slowed} {elsewhere}");
    // A corner of 20 degrees is sharper than a corner may be: the rings turned 0.3 / sin 10 =
    // 1.73 mm short of the pass's corner, 5.8 times the stock. The pass is taken to be a cut as
    // wide as the tool there and goes down in the rings' levels, and the operation says so.
    let sliver = vec![(-30.0, -10.0), (30.0, -10.0), (-30.0, -10.0 + 60.0 * (PI / 9.0).tan())];
    let done = pocketed(&PocketOp { finish: Some(0.3), ..one([-20.0, -3.0]) }, &[pocket(&sliver, &[])]).unwrap();
    check(&done, &sliver, &[], -7.0);
    assert!(done.finish_levels == done.levels && done.levels == vec![vec![-3.5, -7.0]], "{:?} {:?}", done.finish_levels, done.levels);
    assert!(done.notes.iter().any(|n| n.contains("level by level")), "{:?}", done.notes);
}

#[test]
fn a_finishing_pass_is_cut_level_by_level_where_the_rings_did_not_go() {
    // A pocket 60 x 40 with a finger 10.4 wide and 20 long off one side. The 10 mm tool goes
    // into the finger with 0.4 to spare, and a pocket without a finishing pass is cut there.
    let fingered = vec![(-30.0, -20.0), (30.0, -20.0), (30.0, -5.2), (50.0, -5.2), (50.0, 5.2), (30.0, 5.2), (30.0, 20.0), (-30.0, 20.0)];
    let plain = pocketed(&one([0.0, 0.0]), &[pocket(&fingered, &[])]).unwrap();
    let furthest = |done: &Pocketed, z: f64| {
        fed_points(&done.toolpath).iter().filter(|at| (at.z - z).abs() <= 1e-9).map(|at| at.x).fold(f64::NEG_INFINITY, f64::max)
    };
    assert!((furthest(&plain, -3.5) - 45.0).abs() <= 1e-4 && (furthest(&plain, -7.0) - 45.0).abs() <= 1e-4);
    // With 0.3 for a finishing pass the rings stay out of the finger, which they would need
    // 10.6 for: they turn back at its mouth. The pass goes into it, and down it that is a cut
    // as wide as the tool, as a ring is: so the pass is cut at the rings' two levels, not in
    // one step 7 deep, and the operation says so.
    let done = pocketed(&PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&fingered, &[])]).unwrap();
    assert_eq!((done.levels.clone(), done.finish_levels.clone()), (vec![vec![-3.5, -7.0]], vec![vec![-3.5, -7.0]]));
    assert!(
        done.notes.len() == 1
            && done.notes[0].contains("the pocket at X 0 Y 0 with 0.3 mm left on its walls")
            && done.notes[0].contains("a cut as wide as tool 2"),
        "{:?}",
        done.notes
    );
    check(&done, &fingered, &[], -7.0);
    assert!((furthest(&done, -3.5) - 45.0).abs() <= 1e-4 && (furthest(&done, -7.0) - 45.0).abs() <= 1e-4);
    // (The rings alone, 5.3 from the two corners of its mouth, get as far as between them.)
    let rough = pocketed(&PocketOp { leave: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&fingered, &[])]).unwrap();
    let mouth = 30.0 - (5.3_f64.powi(2) - 5.2_f64.powi(2)).sqrt();
    assert!((furthest(&rough, -7.0) - mouth).abs() <= 1e-4, "the rings alone go to X {}, not {mouth}", furthest(&rough, -7.0));
    // A lighter pass that the finger has room for is cut in one step: 0.1 a side leaves the
    // rings 0.2, and they go down the finger first.
    let light = pocketed(&PocketOp { finish: Some(0.1), ..one([0.0, 0.0]) }, &[pocket(&fingered, &[])]).unwrap();
    check(&light, &fingered, &[], -7.0);
    assert!(light.finish_levels == vec![vec![-7.0]] && light.notes.is_empty(), "{:?} {:?}", light.finish_levels, light.notes);
    assert!((furthest(&light, -3.5) - 44.9).abs() <= 1e-4, "{}", furthest(&light, -3.5));
    // A pocket the tool fits only without the finishing stock says so.
    let tight = rect(-30.0, -5.2, 30.0, 5.2);
    assert!(pocketed(&one([0.0, 0.0]), &[pocket(&tight, &[])]).is_ok());
    let e = pocketed(&PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&tight, &[])]).unwrap_err().to_string();
    assert!(e.contains("does not fit") && e.contains("with 0.3 mm left on its walls for the finishing pass"), "{e}");
    // No finishing pass of nothing: 0 is none.
    let none = pocketed(&PocketOp { finish: Some(0.0), ..one([0.0, 0.0]) }, &[pocket(&tight, &[])]).unwrap();
    assert_eq!((none.finish, none.finishing.clone(), none.finish_leads.clone()), (0.0, vec![vec![]], vec![vec![]]));
    // A room off the pocket that the tool cannot get to from it (through a neck 4 wide), and
    // fits in only without the finishing stock: without a finishing pass it is cut on its own,
    // entered down a ramp. With one the rings do not go into it and the pass, which is not
    // made to start in uncut stock, has nowhere to begin there: refused, with why.
    let roomed = vec![
        (-30.0, -20.0),
        (30.0, -20.0),
        (30.0, -2.0),
        (34.0, -2.0),
        (34.0, -5.2),
        (64.0, -5.2),
        (64.0, 5.2),
        (34.0, 5.2),
        (34.0, 2.0),
        (30.0, 2.0),
        (30.0, 20.0),
        (-30.0, 20.0),
    ];
    let plain = pocketed(&one([0.0, 0.0]), &[pocket(&roomed, &[])]).unwrap();
    assert!((furthest(&plain, -7.0) - 59.0).abs() <= 1e-4 && plain.notes.iter().any(|n| n.contains("does not pass everywhere")), "{:?}", plain.notes);
    let e = pocketed(&PocketOp { finish: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&roomed, &[])]).unwrap_err().to_string();
    assert!(e.contains("the pocket at X 0 Y 0") && e.contains("has nowhere to start there") && e.contains("0.3 mm"), "{e}");
}

#[test]
fn random_pockets_are_finished_without_touching_a_wall_or_cutting_wide_at_full_depth() {
    // The random pockets again (a rectangle with up to three islands, a tool and a stepover
    // picked at random, both ways round), two levels deep, with a finishing pass of 0.1 to
    // 0.6 mm. Each is refused for a reason of its own or measured: never nearer a wall or an
    // island than the tool's radius, nothing reachable left uncut at the floor, and one of
    // two things about how the pass goes down. Either it is cut level by level, and the
    // operation says that the rings did not go everywhere. Or it is cut in one step, and then
    // no place of it is further from where the rings ran than four times its stock, which is
    // what a corner may account for (and the quarter of a millimetre at most between two places
    // the operation measures at): the rings here are those of the same pocket cut with that
    // stock left on and no finishing pass.
    let mut seed = 0x9e37_79b9_7f4a_7c15_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut cut, mut refused, mut levelled, mut straight, mut looked_at) = (0, 0, 0, 0, 0);
    for case in 0..120 {
        let (half_x, half_y) = (12.0 + (next() * 30.0).round(), 10.0 + (next() * 22.0).round());
        let wall = rect(-half_x, -half_y, half_x, half_y);
        let mut islands: Vec<Vec<(f64, f64)>> = Vec::new();
        for _ in 0..(next() * 4.0) as usize {
            let (w, h) = (2.0 + (next() * 14.0).round(), 2.0 + (next() * 12.0).round());
            let (x, y) = (((next() * 2.0 - 1.0) * (half_x - w - 1.0)).round(), ((next() * 2.0 - 1.0) * (half_y - h - 1.0)).round());
            let island = rect(x - 0.5 * w, y - 0.5 * h, x + 0.5 * w, y + 0.5 * h);
            let apart = |a: &Vec<(f64, f64)>, b: &Vec<(f64, f64)>| {
                a[0].0 > b[2].0 + 0.5 || b[0].0 > a[2].0 + 0.5 || a[0].1 > b[2].1 + 0.5 || b[0].1 > a[2].1 + 0.5
            };
            let inside_wall = island[0].0 > -half_x + 0.5 && island[2].0 < half_x - 0.5 && island[0].1 > -half_y + 0.5 && island[2].1 < half_y - 0.5;
            if inside_wall && islands.iter().all(|other| apart(other, &island)) {
                islands.push(island);
            }
        }
        let diameter = [4.0, 6.0, 8.0, 10.0, 12.0][(next() * 5.0) as usize % 5];
        let mut tool = mill10();
        tool.geometry.diameter = diameter;
        tool.geometry.shank_diameter = diameter;
        let finish = (1.0 + (next() * 5.0).round()) / 10.0;
        let op = PocketOp {
            stepover: Some(((0.15 + 0.35 * next()) * diameter * 100.0).round() / 100.0),
            direction: if next() < 0.5 { CutDirection::Climb } else { CutDirection::Conventional },
            stepdown: Some(3.5),
            finish: Some(finish),
            ..one([0.0, 0.0])
        };
        let said = format!("case {case} ({wall:?}, {islands:?}, a {diameter} mm tool, {op:?})");
        let shape = pocket(&wall, &islands);
        let done = match op.toolpath(None, &tool, &placed(), Some("aluminium"), std::slice::from_ref(&shape)) {
            Ok(done) => done,
            Err(e) => {
                refused += 1;
                let why = e.to_string();
                assert!(why.contains("does not fit") || why.contains("turns") || why.contains("has nowhere to start there"), "{said}: {why}");
                continue;
            }
        };
        cut += 1;
        looked_at += check_with(&done, &wall, &islands, -7.0, 0.5 * diameter);
        straight += done.finish_leads[0].iter().filter(|&&lead| lead == 0.0).count();
        if done.notes.iter().any(|n| n.contains("level by level")) {
            levelled += 1;
            assert_eq!(done.finish_levels, vec![vec![-3.5, -7.0]], "{said}");
            continue;
        }
        assert_eq!(done.finish_levels, vec![vec![-7.0]], "{said}");
        // Where the rings ran at the floor, in cells of a millimetre.
        let rough =
            PocketOp { finish: None, leave: Some(finish), ..op.clone() }.toolpath(None, &tool, &placed(), Some("aluminium"), &[shape]).unwrap();
        let (nx, ny) = (2 * half_x as usize + 1, 2 * half_y as usize + 1);
        let cell = |at: Vec2| (((at.x + half_x).floor() as usize).min(nx - 1), ((at.y + half_y).floor() as usize).min(ny - 1));
        let mut ran: Vec<Vec<Vec2>> = vec![Vec::new(); nx * ny];
        for at in fed_points(&rough.toolpath).iter().filter(|at| (at.z + 7.0).abs() <= 1e-9) {
            let (i, j) = cell(q(at.x, at.y));
            ran[i * ny + j].push(q(at.x, at.y));
        }
        // Each place of the pass that is nearer a wall than the rings go.
        let reach = 4.0 * finish + 0.25 + 0.15;
        let span = reach.ceil() as usize;
        for at in fed_points(&done.toolpath).iter().filter(|at| (at.z + 7.0).abs() <= 1e-9).map(|at| q(at.x, at.y)) {
            if room(at, &wall, &islands) >= 0.5 * diameter + finish - 1e-3 {
                continue;
            }
            let (i, j) = cell(at);
            let near = (i.saturating_sub(span)..=(i + span).min(nx - 1))
                .flat_map(|i| (j.saturating_sub(span)..=(j + span).min(ny - 1)).map(move |j| i * ny + j))
                .flat_map(|k| ran[k].iter())
                .map(|ring| (ring.x - at.x).hypot(ring.y - at.y))
                .fold(f64::INFINITY, f64::min);
            assert!(near <= reach, "{said}: at {at:?} the pass is {near} mm from where the rings ran, in one step");
        }
    }
    eprintln!("{cut} cut, {refused} refused, {levelled} level by level, {straight} loops met straight on, {looked_at} places looked at");
    assert!(
        cut >= 90 && levelled >= 3 && looked_at > 400_000,
        "{cut} cut, {refused} refused, {levelled} level by level, {straight} straight, {looked_at} places"
    );
}

/// A flat end mill like tool 2, `diameter` across.
fn mill(diameter: f64) -> Tool {
    let mut tool = mill10();
    (tool.geometry.diameter, tool.geometry.shank_diameter) = (diameter, diameter);
    tool
}

/// Checks a pocket cut by two tools against its shape, by measuring, as [`check_with`] does for
/// one: `first` (of radius `wide`, when it cut at all) and then `rest` (of `radius`). The
/// second tool's centre is never nearer a wall or an island than its radius, and at the floor
/// every place it can reach is cut by one of them. Returns how many places were reachable.
fn check_after(
    first: Option<&Pocketed>,
    wide: f64,
    rest: &Pocketed,
    wall: &[(f64, f64)],
    islands: &[Vec<(f64, f64)>],
    floor: f64,
    radius: f64,
) -> usize {
    let points = fed_points(&rest.toolpath);
    let tightest = points.iter().map(|at| room(q(at.x, at.y), wall, islands)).fold(f64::INFINITY, f64::min);
    assert!(tightest >= radius - 1e-4, "the tool's centre comes within {tightest} mm of a wall, not {radius}");
    let (x0, x1) = wall.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), w| (lo.min(w.0), hi.max(w.0)));
    let (y0, y1) = wall.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), w| (lo.min(w.1), hi.max(w.1)));
    let h = 0.5;
    let (nx, ny) = (((x1 - x0) / h).ceil() as usize + 1, ((y1 - y0) / h).ceil() as usize + 1);
    let cell = |i: usize, j: usize| q(x0 + i as f64 * h, y0 + j as f64 * h);
    let stamp = |grid: &mut Vec<bool>, at: Vec2, radius: f64| {
        let reach = (radius / h).ceil() as isize;
        let (ci, cj) = (((at.x - x0) / h).round() as isize, ((at.y - y0) / h).round() as isize);
        for i in (ci - reach).max(0)..=(ci + reach).min(nx as isize - 1) {
            for j in (cj - reach).max(0)..=(cj + reach).min(ny as isize - 1) {
                let c = cell(i as usize, j as usize);
                if (c.x - at.x).hypot(c.y - at.y) <= radius {
                    grid[i as usize * ny + j as usize] = true;
                }
            }
        }
    };
    let (mut reachable, mut cut) = (vec![false; nx * ny], vec![false; nx * ny]);
    for i in 0..nx {
        for j in 0..ny {
            if room(cell(i, j), wall, islands) >= radius + 0.01 {
                stamp(&mut reachable, cell(i, j), radius - h);
            }
        }
    }
    for at in points.iter().filter(|at| (at.z - floor).abs() <= 1e-9) {
        stamp(&mut cut, q(at.x, at.y), radius);
    }
    for at in first.map(|first| fed_points(&first.toolpath)).unwrap_or_default().iter().filter(|at| (at.z - floor).abs() <= 1e-9) {
        stamp(&mut cut, q(at.x, at.y), wide);
    }
    let left: Vec<Vec2> = (0..nx * ny).filter(|&k| reachable[k] && !cut[k]).map(|k| cell(k / ny, k % ny)).collect();
    assert!(left.is_empty(), "{} places the tool can reach are left uncut at the floor, the first at {:?}", left.len(), left.first());
    reachable.iter().filter(|&&r| r).count()
}

#[test]
fn rest_machining_cuts_only_what_the_wider_tool_left() {
    // The pocket 60 x 40 about the origin, cut first with a 20 mm tool: its centre went where
    // it had 10 to every wall, the rectangle X -20 to 20, Y -10 to 10, and it left the four
    // corners round to a radius of 10.
    let wall = rect(-30.0, -20.0, 30.0, 20.0);
    let first = one([0.0, 0.0]).toolpath(None, &mill(20.0), &placed(), Some("aluminium"), &[pocket(&wall, &[])]).unwrap();
    // After it the 10 mm tool. Of its four rings (5, 9, 13 and 17 from the walls) only the
    // outermost comes where there is anything left: within 5 of the wider tool's rectangle the
    // 10 mm tool is wholly inside what was cleared. Along the front, 5 from the wall, that is
    // up to X 20; what is thinner than 0.02 stays, so the tool cuts from where it would take
    // that much, sqrt(5.02^2 - 5^2) = 0.4477 further on, to the corner at X 25 and as far up
    // the side: 9.105 in each corner, and a millimetre more at each end, back in what was
    // cleared, where it goes down and comes up.
    let whole = pocketed(&one([0.0, 0.0]), &[pocket(&wall, &[])]).unwrap();
    let done = pocketed(&PocketOp { rest: Some(20.0), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    let corner = 2.0 * (5.0 - (5.02_f64.powi(2) - 25.0).sqrt()) + 2.0;
    assert!(
        done.rest.len() == 1 && done.rest[0].len() == 4 && done.rest[0].iter().all(|&cut| (cut - corner).abs() <= 5e-3),
        "{:?} {corner}",
        done.rest
    );
    assert!(done.rings == whole.rings && done.levels == whole.levels && whole.rest.is_empty() && done.notes.is_empty(), "{:?}", done.notes);
    // What is fed: at each of the two levels the four corners, and down to each from the
    // retract height in air (8.5 and 12): nothing along the walls between the corners, and a
    // small part of what the whole pocket takes.
    let s = done.toolpath.stats().unwrap();
    assert!((s.feed_length - (8.0 * corner + 4.0 * (8.5 + 12.0))).abs() <= 0.05, "{}", s.feed_length);
    assert!(s.feed_length < 0.2 * whole.toolpath.stats().unwrap().feed_length);
    for at in fed_points(&done.toolpath) {
        assert!(at.x.abs() >= 19.44 && at.y.abs() >= 9.44, "{at:?} is not in a corner");
    }
    // The tool goes down only where it is wholly inside what the wider tool cleared: no
    // further than 5 from that tool's rectangle.
    let mut plunges = 0;
    let mut at = done.toolpath.start;
    for m in &done.toolpath.moves {
        let to = m.end(at);
        if let Move::Linear { feed, .. } = *m
            && to.z < at.z
        {
            assert!(close(feed, done.plunge_feed), "{m:?}");
            let far = (to.x.abs() - 20.0).max(0.0).hypot((to.y.abs() - 10.0).max(0.0));
            assert!(far <= 5.0 + 1e-6, "the tool goes down at {to:?}, {far} mm from where the wider tool's centre went");
            plunges += 1;
        }
        at = to;
    }
    assert_eq!(plunges, 8);
    // Measured: the centre is never nearer a wall than its radius, and with what the wider
    // tool cut, everything the 10 mm tool can reach is cut at the floor.
    let looked_at = check_after(Some(&first), 10.0, &done, &wall, &[], -7.0, 5.0);
    assert!(looked_at > 100);
    // A finishing pass after it is the whole pocket's: the walls all round, once.
    let finished = pocketed(&PocketOp { rest: Some(20.0), finish: Some(0.3), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap();
    assert!(
        finished.rest[0].len() == 4 && close(finished.finishing[0][0], 160.0) && finished.finish_levels == vec![vec![-7.0]],
        "{:?}",
        finished.rest
    );
    // The wider tool is given by its diameter, and has to be the wider one.
    for narrow in [10.0, 6.0, 0.0, -20.0, f64::NAN, f64::INFINITY] {
        let e = pocketed(&PocketOp { rest: Some(narrow), ..one([0.0, 0.0]) }, &[pocket(&wall, &[])]).unwrap_err().to_string();
        assert!(e.contains("rest machining is after a wider tool"), "{narrow}: {e}");
    }
    let op: PocketOp = serde_json::from_str(r#"{"pockets": [[0, 0]], "rest": 20}"#).unwrap();
    assert_eq!(op, PocketOp { rest: Some(20.0), ..one([0.0, 0.0]) });
}

#[test]
fn rest_machining_cuts_whole_what_the_wider_tool_never_came_to() {
    // The pocket with a finger 10.4 wide and 20 long off one side. A 20 mm tool cuts the
    // pocket and stays out of the finger. The 10 mm tool after it cuts the four corners and
    // the finger: from the side of its mouth (where the pocket's own corners are, round the
    // finger's first corner), down it, across its end and back, in one go, started and left in
    // what was cleared.
    let fingered = vec![(-30.0, -20.0), (30.0, -20.0), (30.0, -5.2), (50.0, -5.2), (50.0, 5.2), (30.0, 5.2), (30.0, 20.0), (-30.0, 20.0)];
    let first = one([0.0, 0.0]).toolpath(None, &mill(20.0), &placed(), Some("aluminium"), &[pocket(&fingered, &[])]).unwrap();
    let done = pocketed(&PocketOp { rest: Some(20.0), ..one([0.0, 0.0]) }, &[pocket(&fingered, &[])]).unwrap();
    assert_eq!(done.rest[0].len(), 5, "{:?}", done.rest);
    // Round each corner of the mouth the centre turns a quarter circle of 5. The wider tool's
    // centre went towards the mouth too, round the same corners 10 from them, as far as where
    // those two arcs meet in the mouth's middle: 31.3 degrees round from the wall. Up to there
    // the 10 mm tool, 5 from the corner, is 5 from where the wider one went: nothing to cut. Past
    // it the nearest place the wider tool went is that meeting point, and the tool takes 0.02
    // when it is 5.02 from it: 3.63 degrees further, by the triangle with sides 5 and 10. So the
    // finger's cut is two arcs of 90 less 34.96 degrees, twice 15, 0.4 across, and its two
    // millimetres more: 42.006.
    let met = (5.2_f64 / (100.0_f64 - 5.2 * 5.2).sqrt()).atan();
    let takes = ((125.0 - 5.02_f64.powi(2)) / 100.0).acos();
    let arc = 5.0 * (0.5 * PI - met - takes);
    let finger = 2.0 * arc + 30.4 + 2.0;
    assert!((finger - 42.006).abs() <= 1e-3, "{finger}");
    let longest = done.rest[0].iter().copied().fold(0.0, f64::max);
    assert!((longest - finger).abs() <= 0.01, "{longest} {finger}");
    let furthest = |z: f64| fed_points(&done.toolpath).iter().filter(|at| (at.z - z).abs() <= 1e-9).map(|at| at.x).fold(f64::NEG_INFINITY, f64::max);
    assert!((furthest(-3.5) - 45.0).abs() <= 1e-4 && (furthest(-7.0) - 45.0).abs() <= 1e-4);
    check_after(Some(&first), 10.0, &done, &fingered, &[], -7.0, 5.0);
    // A pocket 60 x 16 the 20 mm tool does not go into at all: after it the 10 mm tool cuts
    // the whole pocket, move for move as with no word about another tool.
    let narrow = rect(-30.0, -8.0, 30.0, 8.0);
    assert!(one([0.0, 0.0]).toolpath(None, &mill(20.0), &placed(), Some("aluminium"), &[pocket(&narrow, &[])]).is_err());
    let whole = pocketed(&one([0.0, 0.0]), &[pocket(&narrow, &[])]).unwrap();
    let after = pocketed(&PocketOp { rest: Some(20.0), ..one([0.0, 0.0]) }, &[pocket(&narrow, &[])]).unwrap();
    assert_eq!(after.toolpath, whole.toolpath);
    assert_eq!(after.rest, whole.rings);
    // A round pocket of radius 30: the 20 mm tool leaves nothing the 10 mm one can reach, and
    // an operation with nothing to cut is refused, with why. With a finishing pass asked for
    // there is that to cut, and the operation says the rest is nothing.
    let circle = PocketShape {
        floor: -7.0,
        region: Region2 {
            outer: Loop2 { segs: vec![Seg2::Arc { center: q(0.0, 0.0), radius: 30.0, start: 0.0, sweep: 2.0 * PI }] },
            holes: Vec::new(),
        },
        open: None,
    };
    let e = pocketed(&PocketOp { rest: Some(20.0), ..one([0.0, 0.0]) }, std::slice::from_ref(&circle)).unwrap_err().to_string();
    assert!(e.contains("a 20 mm tool leaves nothing for tool 2 to cut here"), "{e}");
    let finished = pocketed(&PocketOp { rest: Some(20.0), finish: Some(0.3), ..one([0.0, 0.0]) }, &[circle]).unwrap();
    assert!(finished.rest == vec![Vec::<f64>::new()] && close(finished.finishing[0][0], 50.0 * PI), "{:?} {:?}", finished.rest, finished.finishing);
    assert!(
        finished.notes.iter().any(|n| n.contains("a 20 mm tool leaves nothing in the pocket at X 0 Y 0 for tool 2 to cut")),
        "{:?}",
        finished.notes
    );
}

#[test]
fn random_pockets_are_rest_machined_in_air_where_cleared_and_wholly_where_not() {
    // The random pockets once more (a rectangle with up to three islands), each cut first with
    // a tool and then with one of half to two thirds its width. Measured: the second tool's
    // centre is never nearer a wall or an island than its radius; with what the first tool
    // cut, everything the second can reach is cut at the floor; and wherever the second tool
    // goes straight down, it is wholly inside what the first one cut (every place on its edge
    // is within the first tool's radius of a place that tool's centre fed through).
    let mut seed = 0x1234_5678_9abc_def1_u64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed >> 11) as f64 / (1u64 << 53) as f64
    };
    let (mut cut, mut refused, mut nothing, mut whole, mut parts, mut looked_at, mut unmeasured) = (0, 0, 0, 0, 0, 0, 0);
    for case in 0..80 {
        let (half_x, half_y) = (14.0 + (next() * 30.0).round(), 12.0 + (next() * 22.0).round());
        let wall = rect(-half_x, -half_y, half_x, half_y);
        let mut islands: Vec<Vec<(f64, f64)>> = Vec::new();
        for _ in 0..(next() * 4.0) as usize {
            let (w, h) = (2.0 + (next() * 14.0).round(), 2.0 + (next() * 12.0).round());
            let (x, y) = (((next() * 2.0 - 1.0) * (half_x - w - 1.0)).round(), ((next() * 2.0 - 1.0) * (half_y - h - 1.0)).round());
            let island = rect(x - 0.5 * w, y - 0.5 * h, x + 0.5 * w, y + 0.5 * h);
            let apart = |a: &Vec<(f64, f64)>, b: &Vec<(f64, f64)>| {
                a[0].0 > b[2].0 + 0.5 || b[0].0 > a[2].0 + 0.5 || a[0].1 > b[2].1 + 0.5 || b[0].1 > a[2].1 + 0.5
            };
            let inside_wall = island[0].0 > -half_x + 0.5 && island[2].0 < half_x - 0.5 && island[0].1 > -half_y + 0.5 && island[2].1 < half_y - 0.5;
            if inside_wall && islands.iter().all(|other| apart(other, &island)) {
                islands.push(island);
            }
        }
        let before: f64 = [8.0, 10.0, 12.0, 16.0][(next() * 4.0) as usize % 4];
        let diameter = if next() < 0.5 { 0.5 * before } else { (before * 2.0 / 3.0).round() };
        let shape = pocket(&wall, &islands);
        let op = PocketOp {
            stepdown: Some(7.0),
            direction: if next() < 0.5 { CutDirection::Climb } else { CutDirection::Conventional },
            ..one([0.0, 0.0])
        };
        let said = format!("case {case} ({wall:?}, {islands:?}, a {diameter} mm tool after a {before} mm one, {op:?})");
        let first = op.toolpath(None, &mill(before), &placed(), Some("aluminium"), std::slice::from_ref(&shape)).ok();
        let done = match (PocketOp { rest: Some(before), ..op.clone() }).toolpath(None, &mill(diameter), &placed(), Some("aluminium"), &[shape]) {
            Ok(done) => done,
            Err(e) => {
                let why = e.to_string();
                refused += usize::from(!why.contains("leaves nothing"));
                nothing += usize::from(why.contains("leaves nothing"));
                assert!(why.contains("does not fit") || why.contains("turns") || why.contains("leaves nothing"), "{said}: {why}");
                continue;
            }
        };
        cut += 1;
        // The first tool's own pocket may have been refused though the tool fits somewhere (its
        // ramp has no room): what it would have cleared cannot be measured then. The second
        // tool's centre still keeps its radius from every wall.
        if first.is_none() {
            let tightest = fed_points(&done.toolpath).iter().map(|at| room(q(at.x, at.y), &wall, &islands)).fold(f64::INFINITY, f64::min);
            assert!(tightest >= 0.5 * diameter - 1e-4, "{said}: the tool's centre comes within {tightest} mm of a wall");
            unmeasured += 1;
            continue;
        }
        looked_at += check_after(first.as_ref(), 0.5 * before, &done, &wall, &islands, -7.0, 0.5 * diameter);
        // Where the first tool's centre fed through at the floor, in cells of a millimetre.
        let (nx, ny) = (2 * half_x as usize + 1, 2 * half_y as usize + 1);
        let cell = |at: Vec2| (((at.x + half_x).floor() as usize).min(nx - 1), ((at.y + half_y).floor() as usize).min(ny - 1));
        let mut ran: Vec<Vec<Vec2>> = vec![Vec::new(); nx * ny];
        for at in first.iter().flat_map(|first| fed_points(&first.toolpath)).filter(|at| (at.z + 7.0).abs() <= 1e-9) {
            let (i, j) = cell(q(at.x, at.y));
            ran[i * ny + j].push(q(at.x, at.y));
        }
        let span = (0.5 * before + 1.0).ceil() as usize;
        let from_first = |at: Vec2| {
            let (i, j) = cell(at);
            (i.saturating_sub(span)..=(i + span).min(nx - 1))
                .flat_map(|i| (j.saturating_sub(span)..=(j + span).min(ny - 1)).map(move |j| i * ny + j))
                .flat_map(|k| ran[k].iter())
                .map(|ring| (ring.x - at.x).hypot(ring.y - at.y))
                .fold(f64::INFINITY, f64::min)
        };
        // Each cut of the level: a whole ring (down a ramp or from the ring before), or a part
        // of one, which is where the tool goes straight down from the retract height.
        let mut at = done.toolpath.start;
        let mut down = 0;
        for m in &done.toolpath.moves {
            let to = m.end(at);
            if let Move::Linear { .. } = *m
                && at.z == 5.0
                && to.z == -7.0
            {
                down += 1;
                for k in 0..16 {
                    let a = f64::from(k) * PI / 8.0;
                    let edge = q(to.x + (0.5 * diameter - 0.05) * a.cos(), to.y + (0.5 * diameter - 0.05) * a.sin());
                    let far = from_first(edge);
                    assert!(
                        far <= 0.5 * before + 0.2,
                        "{said}: down at {to:?}, where its edge at {edge:?} is {far} mm from where the first tool's centre went"
                    );
                }
            }
            at = to;
        }
        parts += down;
        whole += usize::from(down == 0);
    }
    let counts = format!(
        "{cut} cut ({unmeasured} after a first tool whose own pocket was refused), {refused} refused, {nothing} with nothing left, {whole} cut with whole rings alone, {parts} parts gone down to in air, {looked_at} places"
    );
    eprintln!("{counts}");
    assert!(cut >= 40 && parts >= 100 && looked_at > 200_000, "{counts}");
}

/// A step as the engine reads a pocket open at a side: 10 wide along the block's front, from
/// its front edge (Y -40) to a wall at Y -30, its floor at Z -7.
fn open_step() -> PocketShape {
    let floor = Region2 { outer: polygon(&rect(-50.0, -40.0, 50.0, -30.0)), holes: Vec::new() };
    let walls = vec![Region2 { outer: polygon(&rect(-50.0, -30.0, 50.0, 40.0)), holes: Vec::new() }];
    PocketShape { floor: -7.0, region: floor.clone(), open: Some(crate::OpenFloor { floor: vec![floor], walls }) }
}

fn near3(a: Vec3, b: Vec3) -> bool {
    (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6 && (a.z - b.z).abs() < 1e-6
}

/// Where each straight cut of `moves` ends, and its feed.
fn lines(moves: &[Move]) -> Vec<(Vec3, f64)> {
    moves
        .iter()
        .filter_map(|m| match m {
            Move::Linear { to, feed } => Some((*to, *feed)),
            _ => None,
        })
        .collect()
}

#[test]
fn a_pocket_open_at_a_side_is_cut_in_passes_from_beside_the_stock_at_every_level() {
    let done = pocketed(&one([0.0, -35.0]), &[open_step()]).unwrap();
    // 7 mm down from the stock's top in two levels, three passes at each: 13, 9 and 5 mm from
    // the wall, 116 mm long with their ways in and out.
    assert_eq!((done.levels.clone(), done.open.clone(), done.offset), (vec![vec![-3.5, -7.0]], vec![true], 5.0));
    assert!(done.rings.len() == 1 && done.rings[0].len() == 3 && done.rings[0].iter().all(|long| (long - 116.0).abs() < 1e-6), "{:?}", done.rings);
    assert!(done.notes.is_empty(), "{:?}", done.notes);
    let (feed, m) = (RPM * 0.15, &done.toolpath.moves);
    let rapid = |k: usize| match m[k] {
        Move::Rapid { to } => to,
        ref other => panic!("{k}: {other:?}"),
    };
    let line = |k: usize| match m[k] {
        Move::Linear { to, feed } => (to, feed),
        ref other => panic!("{k}: {other:?}"),
    };
    // The first pass: above its start 7 mm beside the stock, down there in the air, in to the
    // wall's end at half the feed, along the floor, out at the other end, up.
    assert!(near3(rapid(3), p(58.0, -43.0, 25.0)) && near3(rapid(4), p(58.0, -43.0, 5.0)));
    assert!(near3(line(5).0, p(58.0, -43.0, -3.5)) && close(line(5).1, 0.5 * feed));
    assert!(near3(line(6).0, p(50.0, -43.0, -3.5)) && close(line(6).1, 0.5 * feed));
    assert!(near3(line(7).0, p(-50.0, -43.0, -3.5)) && close(line(7).1, feed));
    assert!(near3(line(8).0, p(-58.0, -43.0, -3.5)) && close(line(8).1, feed));
    assert!(near3(rapid(9), p(-58.0, -43.0, 5.0)));
    // Six passes in all, four straight moves each; the last is along the wall, at the floor.
    let cuts = lines(m);
    assert_eq!(cuts.len(), 24);
    assert!(near3(cuts[23].0, p(-58.0, -35.0, -7.0)) && near3(cuts[22].0, p(-50.0, -35.0, -7.0)));
    // No cut is nearer the wall than the tool's radius, and the tool goes down only beside the
    // stock (which ends at X 51).
    let mut at = done.toolpath.start;
    for mv in m {
        match mv {
            Move::Linear { to, .. } => {
                assert!(to.y <= -35.0 + 1e-6, "{to:?}");
                if to.z < at.z - 1e-9 {
                    assert!(to.x.abs() >= 58.0 - 1e-6 && (to.x - at.x).abs() < 1e-9 && (to.y - at.y).abs() < 1e-9, "{at:?} to {to:?}");
                }
                at = *to;
            }
            Move::Rapid { to } => at = *to,
            _ => {}
        }
    }
}

#[test]
fn an_open_pocket_takes_a_finishing_pass_along_its_wall_and_refuses_rest_machining() {
    let done = pocketed(&PocketOp { finish: Some(0.5), ..one([0.0, -35.0]) }, &[open_step()]).unwrap();
    // The passes keep 5.5 mm from the wall; the finishing pass goes along it once, at the floor,
    // from beside the stock as they do.
    assert_eq!((done.offset, done.finish, done.finish_levels.clone()), (5.5, 0.5, vec![vec![-7.0]]));
    assert!(done.finishing.len() == 1 && done.finishing[0].len() == 1 && (done.finishing[0][0] - 116.0).abs() < 1e-6, "{:?}", done.finishing);
    assert_eq!(done.finish_leads, vec![Vec::<f64>::new()]);
    let cuts = lines(&done.toolpath.moves);
    assert_eq!(cuts.len(), 6 * 4 + 4);
    let ys: Vec<f64> = cuts.chunks(4).map(|pass| (pass[2].0.y * 1e6).round() / 1e6).collect();
    assert_eq!(ys, vec![-43.5, -39.5, -35.5, -43.5, -39.5, -35.5, -35.0]);
    assert!(near3(cuts[27].0, p(-58.0, -35.0, -7.0)) && near3(cuts[24].0, p(58.0, -35.0, -7.0)));
    // Rest machining is not built for one: it says so.
    let said = pocketed(&PocketOp { rest: Some(16.0), ..one([0.0, -35.0]) }, &[open_step()]).unwrap_err().to_string();
    assert!(said.contains("rest machining of an open pocket"), "{said}");
}

#[test]
fn an_open_pocket_far_from_the_stocks_edge_is_entered_down_a_ramp_along_its_first_pass() {
    // A floor 40 x 10 in the middle of the block before a wall along Y 0, the stock's edge 30 mm
    // and more from its ends: no pass can be lengthened to beside the stock.
    let floor = Region2 { outer: polygon(&rect(-20.0, -10.0, 20.0, 0.0)), holes: Vec::new() };
    let walls = vec![Region2 { outer: polygon(&rect(-20.0, 0.0, 20.0, 30.0)), holes: Vec::new() }];
    let shape = PocketShape { floor: -7.0, region: floor.clone(), open: Some(crate::OpenFloor { floor: vec![floor], walls }) };
    let done = pocketed(&one([0.0, -5.0]), &[shape]).unwrap();
    assert!(done.rings[0].len() == 3 && done.rings[0].iter().all(|long| (long - 40.0).abs() < 1e-6), "{:?}", done.rings);
    let (feed, m) = (RPM * 0.15, &done.toolpath.moves);
    let cuts = lines(m);
    // The first pass, 13 mm from the wall: down to the stock's top, then there and back along
    // itself falling 3.5 mm (80 mm of ramp is 2.5 degrees, under the 3 asked), at the ramp's
    // feed; then the pass itself.
    assert!(near3(cuts[0].0, p(20.0, -13.0, 0.0)) && close(cuts[0].1, 0.5 * feed));
    assert!(near3(cuts[1].0, p(-20.0, -13.0, -1.75)) && close(cuts[1].1, 0.5 * feed));
    assert!(near3(cuts[2].0, p(20.0, -13.0, -3.5)) && close(cuts[2].1, 0.5 * feed));
    assert!(near3(cuts[3].0, p(-20.0, -13.0, -3.5)) && close(cuts[3].1, feed));
    // The next, 9 mm from it: down where the first one started, in the air that one cut, 4 mm
    // across to its own start, and along.
    assert!(near3(cuts[4].0, p(20.0, -13.0, -3.5)) && close(cuts[4].1, 0.5 * feed));
    assert!(near3(cuts[5].0, p(20.0, -9.0, -3.5)) && close(cuts[5].1, feed));
    assert!(near3(cuts[6].0, p(-20.0, -9.0, -3.5)) && close(cuts[6].1, feed));
    // At the second level the ramp starts from the first level's height.
    let second = cuts.iter().position(|(to, _)| to.z < -3.5 - 1e-9).unwrap();
    assert!(near3(cuts[second - 1].0, p(20.0, -13.0, -3.5)) && near3(cuts[second].0, p(-20.0, -13.0, -5.25)), "{:?}", &cuts[second - 1..=second]);
    assert!(done.notes.is_empty(), "{:?}", done.notes);
}

#[test]
fn an_open_pocket_simulated_takes_the_stock_over_its_floor_and_beside_it_and_cuts_into_nothing() {
    use crate::sim::Simulation;
    let done = pocketed(&one([0.0, -35.0]), &[open_step()]).unwrap();
    // The part over the stock's grid: the block's top at Z -1, the step's floor at Z -7.
    let sim = Simulation::new(placed().stock, 0.25, 0.01).unwrap();
    let field = sim.stock.clone();
    let mut part = vec![-100.0; field.nx * field.ny];
    for j in 0..field.ny {
        for i in 0..field.nx {
            let c = field.centre(i, j);
            if c.x.abs() < 50.0 && c.y.abs() < 40.0 {
                part[j * field.nx + i] = if c.y < -30.0 { -7.0 } else { -1.0 };
            }
        }
    }
    let mut sim = sim.with_part(part).unwrap();
    let cut = sim.cut(1, &done.toolpath, &mill10()).unwrap();
    assert_eq!((sim.gouge_count, sim.rapid_hit_count, sim.below_stock), (0, 0, 0.0), "{:?} {:?}", sim.gouges.first(), sim.rapid_hits.first());
    // What is taken: the stock over the step (100 x 10), the millimetre of it along the step's
    // front (102 x 1) and at its two ends (1 x 10 each), all 7 mm down from the stock's top.
    let expected = (1000.0 + 102.0 + 20.0) * 7.0;
    assert!((cut.removed - expected).abs() < 0.5 * 250.0 * 0.25 * 7.0, "{} and {expected}", cut.removed);
    // Nothing is left over the step; what still stands over the part is the millimetre on the
    // block's top, which is the facing's.
    let (over, _) = sim.leftover();
    assert!((over - 100.0 * 70.0).abs() < 0.5 * 100.0 * 0.25, "{over}");
}
