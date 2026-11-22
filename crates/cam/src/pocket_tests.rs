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
    PocketShape { floor: -7.0, region: Region2 { outer: polygon(wall), holes: islands.iter().map(|i| polygon(i).reversed()).collect() } }
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
    let shape = PocketShape { floor: -7.0, region: Region2 { outer: circle, holes: Vec::new() } };
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
        for field in 0..8 {
            let mut op = one([0.0, 0.0]);
            match field {
                0 => op.top = Some(v),
                1 => op.stepdown = Some(v),
                2 => op.stepover = Some(v),
                3 => op.leave = Some(v),
                4 => op.ramp_angle = Some(v),
                5 => op.rpm = Some(v),
                6 => op.feed = Some(v),
                _ => op.pockets = vec![[v, v]],
            }
            let _ = pocketed(&op, std::slice::from_ref(&shape));
        }
        // Shapes of hostile numbers: a floor, a point, a spike, a broken loop, a hostile island.
        let shapes = [
            PocketShape { floor: v, ..shape.clone() },
            pocket(&[(v, v), (v, v), (v, v)], &[]),
            pocket(&[(0.0, 0.0), (v, 0.0), (0.0, v)], &[]),
            PocketShape { floor: -7.0, region: Region2 { outer: Loop2 { segs: vec![] }, holes: Vec::new() } },
            pocket(&wall, &[vec![(v, v), (1.0, v), (v, 1.0)]]),
            PocketShape {
                floor: -7.0,
                region: Region2 { outer: Loop2 { segs: vec![Seg2::Arc { center: q(v, 0.0), radius: v, start: v, sweep: v }] }, holes: Vec::new() },
            },
        ];
        for odd in shapes {
            let _ = pocketed(&one([0.0, 0.0]), &[odd]);
        }
        // A tool of hostile sizes: never a path outside the region.
        for size in 0..3 {
            let mut odd = mill10();
            match size {
                0 => odd.geometry.diameter = v,
                1 => odd.geometry.flute_length = v,
                _ => odd.geometry.shoulder_length = v,
            }
            if let Ok(done) = one([0.0, 0.0]).toolpath(None, &odd, &placed(), Some("aluminium"), std::slice::from_ref(&shape)) {
                assert!(done.offset > 0.0 && done.offset.is_finite(), "{v}: {}", done.offset);
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
        (op.pockets.len(), op.direction, op.stepover, op.leave, op.ramp_angle),
        (2, CutDirection::Conventional, Some(3.0), Some(0.2), Some(0.1))
    );
    assert!(serde_json::from_str::<PocketOp>(r#"{"pockets": [[1, 2]], "helix": 4}"#).is_err());
    assert!(serde_json::from_str::<PocketOp>("{}").is_err(), "a pocket operation names its pockets");
    assert_eq!(serde_json::from_value::<PocketOp>(serde_json::to_value(&op).unwrap()).unwrap(), op);
}
