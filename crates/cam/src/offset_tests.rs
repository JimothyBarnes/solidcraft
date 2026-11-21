//! Every expected number here is worked out by hand from the shapes; areas and lengths are measured
//! with upstream's own `Region2Geom` and `Seg2Geom`, not with sums of ours (decision 0007).
use std::f64::consts::{FRAC_PI_2, PI, TAU};

use solidcraft_geom::Region2Geom;

use super::*;

fn v(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

/// A counter-clockwise loop of straight sides through `points`.
fn polygon(points: &[(f64, f64)]) -> Loop2 {
    let n = points.len();
    Loop2 { segs: (0..n).map(|i| Seg2::Line { a: v(points[i].0, points[i].1), b: v(points[(i + 1) % n].0, points[(i + 1) % n].1) }).collect() }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Loop2 {
    polygon(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)])
}

fn solid(outer: Loop2) -> Region2 {
    Region2 { outer, holes: vec![] }
}

fn area(regions: &[Region2]) -> f64 {
    regions.iter().map(Region2Geom::area).sum()
}

fn perimeter(regions: &[Region2]) -> f64 {
    regions.iter().flat_map(Region2::loops).flat_map(|l| &l.segs).map(Seg2Geom::length).sum()
}

fn arcs(l: &Loop2) -> Vec<(Vec2, f64, f64)> {
    l.segs.iter().filter_map(|s| if let Seg2::Arc { center, radius, sweep, .. } = *s { Some((center, radius, sweep)) } else { None }).collect()
}

fn lines(l: &Loop2) -> usize {
    l.segs.iter().filter(|s| matches!(s, Seg2::Line { .. })).count()
}

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// The distance from `p` to a line or arc.
fn to_seg(p: Vec2, s: &Seg2) -> f64 {
    let d = |a: Vec2, b: Vec2| (a.x - b.x).hypot(a.y - b.y);
    match *s {
        Seg2::Line { a, b } => {
            let (ux, uy) = (b.x - a.x, b.y - a.y);
            let t = (((p.x - a.x) * ux + (p.y - a.y) * uy) / (ux * ux + uy * uy)).clamp(0.0, 1.0);
            d(p, v(a.x + t * ux, a.y + t * uy))
        }
        Seg2::Arc { center, radius, start, sweep } => {
            let angle = (p.y - center.y).atan2(p.x - center.x);
            let along = if sweep >= 0.0 { (angle - start).rem_euclid(TAU) } else { (start - angle).rem_euclid(TAU) };
            if along <= sweep.abs() { (d(p, center) - radius).abs() } else { d(p, s.start()).min(d(p, s.end())) }
        }
        _ => f64::NAN,
    }
}

fn to_boundary(p: Vec2, regions: &[Region2]) -> f64 {
    regions.iter().flat_map(Region2::loops).flat_map(|l| &l.segs).map(|s| to_seg(p, s)).fold(f64::INFINITY, f64::min)
}

/// What an offset is: every point of the result's boundary lies `distance` from the source's
/// boundary (within `tol`), inside the source when it shrank and outside when it grew.
fn assert_is_the_offset(source: &[Region2], result: &[Region2], distance: f64, tol: f64) {
    for seg in result.iter().flat_map(Region2::loops).flat_map(|l| &l.segs) {
        for t in [0.0, 0.3, 0.5, 0.8] {
            let p = seg.point_at(t);
            let far = to_boundary(p, source);
            assert!(close(far, distance.abs(), tol), "{p:?} of {seg:?} is {far} mm from the source, not {}", distance.abs());
            let inside = source.iter().any(|r| r.contains(p));
            assert_eq!(inside, distance > 0.0, "{p:?} of {seg:?} is on the wrong side of the source");
        }
    }
}

#[test]
fn a_square_grown_gains_a_round_corner_of_the_offsets_radius_at_each_corner() {
    // 40 square grown by 5: four sides of 40 and four quarter circles of radius 5 about the corners.
    // Area 40² + 4 x 40 x 5 + π 5², outline 4 x 40 + 2 π 5.
    let source = [solid(rect(0.0, 0.0, 40.0, 40.0))];
    let grown = offset_regions(&source, -5.0).unwrap();
    assert_eq!((grown.len(), grown[0].holes.len()), (1, 0));
    assert!(close(area(&grown), 1600.0 + 800.0 + 25.0 * PI, 1e-9), "{}", area(&grown));
    assert!(close(perimeter(&grown), 160.0 + 10.0 * PI, 1e-9), "{}", perimeter(&grown));
    let corners = arcs(&grown[0].outer);
    assert_eq!((corners.len(), lines(&grown[0].outer)), (4, 4));
    for (center, radius, sweep) in corners {
        assert!(close(radius, 5.0, 1e-9) && close(sweep, FRAC_PI_2, 1e-9), "{radius} {sweep}");
        assert!([0.0, 40.0].iter().any(|c| close(center.x, *c, 1e-9)) && [0.0, 40.0].iter().any(|c| close(center.y, *c, 1e-9)), "{center:?}");
    }
    assert_is_the_offset(&source, &grown, -5.0, 1e-9);
}

#[test]
fn a_square_shrunk_keeps_sharp_corners_and_vanishes_past_its_middle() {
    let source = [solid(rect(0.0, 0.0, 40.0, 40.0))];
    // By 5: a 30 square, four straight sides.
    let shrunk = offset_regions(&source, 5.0).unwrap();
    assert_eq!((shrunk.len(), lines(&shrunk[0].outer), arcs(&shrunk[0].outer).len()), (1, 4, 0));
    assert!(close(area(&shrunk), 900.0, 1e-9) && close(perimeter(&shrunk), 120.0, 1e-9));
    assert_is_the_offset(&source, &shrunk, 5.0, 1e-9);
    // By 19.9: a 0.2 square is left. Past the middle (20): nothing.
    assert!(close(area(&offset_regions(&source, 19.9).unwrap()), 0.04, 1e-9));
    assert!(offset_regions(&source, 20.5).unwrap().is_empty());
    assert!(offset_regions(&source, 500.0).unwrap().is_empty());
    // By nothing: the same square.
    assert!(close(area(&offset_regions(&source, 0.0).unwrap()), 1600.0, 1e-9));
}

#[test]
fn a_circle_stays_a_circle() {
    let source = [solid(Loop2::circle(v(3.0, 4.0), 10.0))];
    for (distance, radius) in [(4.0, 6.0), (-4.0, 14.0), (9.5, 0.5)] {
        let moved = offset_regions(&source, distance).unwrap();
        assert_eq!(moved.len(), 1, "{distance}");
        assert!(close(area(&moved), PI * radius * radius, 1e-9), "{distance}: {}", area(&moved));
        assert!(close(perimeter(&moved), TAU * radius, 1e-9));
        assert_eq!(lines(&moved[0].outer), 0);
        for (center, r, _) in arcs(&moved[0].outer) {
            assert!(close(r, radius, 1e-9) && close(center.x, 3.0, 1e-9) && close(center.y, 4.0, 1e-9), "{distance}: {center:?} {r}");
        }
        assert_is_the_offset(&source, &moved, distance, 1e-9);
    }
    assert!(offset_regions(&source, 10.5).unwrap().is_empty());
}

/// A 100 square with a round island (a hole in the region) of radius 10 at its middle.
fn pocket_with_island() -> Region2 {
    Region2 { outer: rect(0.0, 0.0, 100.0, 100.0), holes: vec![Loop2::circle(v(50.0, 50.0), 10.0).reversed()] }
}

#[test]
fn an_island_grows_as_its_pocket_shrinks() {
    let source = [pocket_with_island()];
    // The path of a 10 mm tool's centre: walls in by 5 (a 90 square), the island out by 5 (radius 15).
    let path = offset_regions(&source, 5.0).unwrap();
    assert_eq!((path.len(), path[0].holes.len()), (1, 1));
    assert!(close(area(&path), 8100.0 - 225.0 * PI, 1e-9), "{}", area(&path));
    for (center, radius, sweep) in arcs(&path[0].holes[0]) {
        assert!(
            close(radius, 15.0, 1e-9) && close(center.x, 50.0, 1e-9) && close(center.y, 50.0, 1e-9) && sweep < 0.0,
            "{center:?} {radius} {sweep}"
        );
    }
    assert_is_the_offset(&source, &path, 5.0, 1e-9);
    // Grown by 5 the island shrinks to radius 5: 100² + 4 x 100 x 5 + 25 π - 25 π = 12 000 exactly.
    let grown = offset_regions(&source, -5.0).unwrap();
    assert!(close(area(&grown), 12_000.0, 1e-9), "{}", area(&grown));
    assert_eq!(grown[0].holes.len(), 1);
    // Grown past the island's radius, the island is gone.
    let grown = offset_regions(&source, -10.5).unwrap();
    assert_eq!((grown.len(), grown[0].holes.len()), (1, 0));
    assert!(close(area(&grown), 10_000.0 + 400.0 * 10.5 + PI * 10.5 * 10.5, 1e-9));
    // A hole given counter-clockwise is the same hole.
    let mut wrong_way = pocket_with_island();
    wrong_way.holes[0] = wrong_way.holes[0].reversed();
    assert!(close(area(&offset_regions(&[wrong_way], 5.0).unwrap()), 8100.0 - 225.0 * PI, 1e-9));
}

#[test]
fn islands_that_grow_into_each_other_become_one() {
    // Two islands of radius 10, centres 24 apart (4 between them). Shrinking the pocket by 3 grows
    // them to 13 each: 26 > 24, so they join. Two discs of radius R whose centres are D apart
    // overlap in a lens of 2 R² acos(D / 2R) - (D / 2) √(4R² - D²).
    let source = [Region2 {
        outer: rect(0.0, 0.0, 100.0, 100.0),
        holes: vec![Loop2::circle(v(40.0, 50.0), 10.0).reversed(), Loop2::circle(v(64.0, 50.0), 10.0).reversed()],
    }];
    let path = offset_regions(&source, 3.0).unwrap();
    assert_eq!((path.len(), path[0].holes.len()), (1, 1));
    let (r, d) = (13.0_f64, 24.0_f64);
    let lens = 2.0 * r * r * (d / (2.0 * r)).acos() - 0.5 * d * (4.0 * r * r - d * d).sqrt();
    assert!(close(area(&path), 94.0 * 94.0 - (2.0 * PI * r * r - lens), 1e-7), "{}", area(&path));
    assert_is_the_offset(&source, &path, 3.0, 1e-7);
    // By 1.9 they are still two (11.9 each, 23.8 < 24).
    assert_eq!(offset_regions(&source, 1.9).unwrap()[0].holes.len(), 2);
}

#[test]
fn an_island_near_a_wall_joins_the_wall() {
    // An island of radius 10 whose edge is 5 from the top wall. Shrunk by 4, the wall is at Y 96
    // and the island's radius 14 reaches Y 99: it breaks into the wall, leaving one outline with a
    // bite out of it and no hole. The bite is the disc less its cap above Y 96, of height 3:
    // R² acos((R - h) / R) - (R - h) √(2 R h - h²).
    let source = [Region2 { outer: rect(0.0, 0.0, 100.0, 100.0), holes: vec![Loop2::circle(v(50.0, 85.0), 10.0).reversed()] }];
    let path = offset_regions(&source, 4.0).unwrap();
    assert_eq!((path.len(), path[0].holes.len()), (1, 0));
    let (r, h) = (14.0_f64, 3.0_f64);
    let cap = r * r * ((r - h) / r).acos() - (r - h) * (2.0 * r * h - h * h).sqrt();
    assert!(close(area(&path), 92.0 * 92.0 - (PI * r * r - cap), 1e-7), "{}", area(&path));
    assert_is_the_offset(&source, &path, 4.0, 1e-7);
}

#[test]
fn a_region_with_a_thin_neck_splits_in_two() {
    // Two 30 squares joined by a neck 10 long and 4 wide. Shrunk by 3 the neck (half-width 2)
    // closes and two pieces are left. Each is a 24 square plus a small cusp toward the neck, between
    // the wall's offset (X 27) and the arcs of radius 3 about the neck's two corners, which meet at
    // Y 15: twice ∫₀² (3 - √(9 - u²)) du = 2 (6 - √5 - 4.5 asin(2/3)).
    let source = [solid(polygon(&[
        (0.0, 0.0),
        (30.0, 0.0),
        (30.0, 13.0),
        (40.0, 13.0),
        (40.0, 0.0),
        (70.0, 0.0),
        (70.0, 30.0),
        (40.0, 30.0),
        (40.0, 17.0),
        (30.0, 17.0),
        (30.0, 30.0),
        (0.0, 30.0),
    ]))];
    let pieces = offset_regions(&source, 3.0).unwrap();
    assert_eq!(pieces.len(), 2);
    let cusp = 2.0 * (6.0 - 5.0_f64.sqrt() - 4.5 * (2.0_f64 / 3.0).asin());
    assert!(close(area(&pieces), 2.0 * (576.0 + cusp), 1e-7), "{} vs {}", area(&pieces), 2.0 * (576.0 + cusp));
    assert!(close(pieces[0].area(), pieces[1].area(), 1e-7));
    assert_is_the_offset(&source, &pieces, 3.0, 1e-7);
    // By 1.9 the neck is still open: one piece.
    assert_eq!(offset_regions(&source, 1.9).unwrap().len(), 1);
}

#[test]
fn regions_grown_into_each_other_become_one() {
    // Two 10 squares 2 apart, each grown by 2 to 100 + 80 + 4π. They then share the 2 x 10 strip
    // between them and, above and below it, half the lens of two radius-2 corner arcs whose centres
    // are 2 apart: 4π/3 - √3 each.
    let source = [solid(rect(0.0, 0.0, 10.0, 10.0)), solid(rect(12.0, 0.0, 22.0, 10.0))];
    let grown = offset_regions(&source, -2.0).unwrap();
    assert_eq!((grown.len(), grown[0].holes.len()), (1, 0));
    let shared = 20.0 + 2.0 * (4.0 * PI / 3.0 - 3.0_f64.sqrt());
    assert!(close(area(&grown), 2.0 * (180.0 + 4.0 * PI) - shared, 1e-7), "{}", area(&grown));
    assert_is_the_offset(&source, &grown, -2.0, 1e-7);
    // Grown by 0.9 they are still two.
    assert_eq!(offset_regions(&source, -0.9).unwrap().len(), 2);
}

#[test]
fn arcs_of_any_sweep_and_slots_offset_exactly() {
    // A slot: a 20 x 10 middle with half-circle ends of radius 5. Grown by 2 it is a slot of radius
    // 7; shrunk by 2, radius 3; shrunk by 5.5, gone.
    let slot = Loop2 {
        segs: vec![
            Seg2::Line { a: v(0.0, -5.0), b: v(20.0, -5.0) },
            Seg2::Arc { center: v(20.0, 0.0), radius: 5.0, start: -FRAC_PI_2, sweep: PI },
            Seg2::Line { a: v(20.0, 5.0), b: v(0.0, 5.0) },
            Seg2::Arc { center: v(0.0, 0.0), radius: 5.0, start: FRAC_PI_2, sweep: PI },
        ],
    };
    let source = [solid(slot)];
    for (distance, radius) in [(-2.0, 7.0), (2.0, 3.0)] {
        let moved = offset_regions(&source, distance).unwrap();
        assert!(close(area(&moved), 20.0 * 2.0 * radius + PI * radius * radius, 1e-9), "{distance}: {}", area(&moved));
        assert!(close(perimeter(&moved), 40.0 + TAU * radius, 1e-9));
        assert_is_the_offset(&source, &moved, distance, 1e-9);
    }
    assert!(offset_regions(&source, 5.5).unwrap().is_empty());
    // A keyhole: three quarters of a circle of radius 10 closed by two sides to its centre. The
    // arc sweeps more than half a turn, so it goes in as two pieces and must come back the same curve.
    let keyhole = Loop2 {
        segs: vec![
            Seg2::Line { a: v(0.0, 0.0), b: v(10.0, 0.0) },
            Seg2::Arc { center: v(0.0, 0.0), radius: 10.0, start: 0.0, sweep: 1.5 * PI },
            Seg2::Line { a: v(0.0, -10.0), b: v(0.0, 0.0) },
        ],
    };
    assert!(close(solid(keyhole.clone()).area(), 75.0 * PI, 1e-9));
    let moved = offset_regions(&[solid(keyhole.clone())], 0.0).unwrap();
    assert!(close(area(&moved), 75.0 * PI, 1e-9) && close(perimeter(&moved), 20.0 + 15.0 * PI, 1e-9));
    // Shrunk by 1: radius 9 over three quarters, less the two strips of width 1 along the sides,
    // which overlap in the unit square at the centre and each lose the sliver past the arc.
    let shrunk = offset_regions(&[solid(keyhole.clone())], 1.0).unwrap();
    assert_is_the_offset(&[solid(keyhole)], &shrunk, 1.0, 1e-9);
}

#[test]
fn what_cannot_be_offset_is_refused_not_guessed_at() {
    let square = rect(0.0, 0.0, 10.0, 10.0);
    let open = Loop2 { segs: square.segs[..3].to_vec() };
    let gap = Loop2 { segs: vec![square.segs[0], square.segs[1], square.segs[2], Seg2::Line { a: v(0.0, 10.0), b: v(0.0, 0.5) }] };
    let flat = polygon(&[(0.0, 0.0), (10.0, 0.0)]);
    let curve = Loop2 {
        segs: vec![
            Seg2::Cubic { p0: v(0.0, 0.0), p1: v(5.0, 5.0), p2: v(10.0, 5.0), p3: v(10.0, 0.0) },
            Seg2::Line { a: v(10.0, 0.0), b: v(0.0, 0.0) },
        ],
    };
    let nan = polygon(&[(0.0, 0.0), (10.0, f64::NAN), (10.0, 10.0)]);
    let far = polygon(&[(0.0, 0.0), (1e9, 0.0), (10.0, 10.0)]);
    let no_radius = Loop2 { segs: vec![Seg2::Arc { center: v(0.0, 0.0), radius: 0.0, start: 0.0, sweep: TAU }] };
    let cases: Vec<(Loop2, &str)> = vec![
        (Loop2 { segs: vec![] }, "segments"),
        (open, "not closed"),
        (gap, "not closed"),
        (flat, "encloses no area"),
        (curve, "only lines and arcs"),
        (nan, "region 0's outer loop"),
        (far, "region 0's outer loop"),
        (no_radius, "radius"),
    ];
    for (outer, expect) in cases {
        let e = offset_region(&solid(outer.clone()), 1.0).unwrap_err().to_string();
        assert!(e.contains(expect), "{outer:?}: {e:?} does not mention {expect:?}");
        // The same loop as a hole is refused too, by its own name.
        let e = offset_region(&Region2 { outer: square.clone(), holes: vec![outer] }, 1.0).unwrap_err().to_string();
        assert!(e.contains("hole 0") || e.contains(expect), "{e:?}");
    }
    for distance in [f64::NAN, f64::INFINITY, -f64::INFINITY, 1e9] {
        assert!(offset_region(&solid(square.clone()), distance).is_err(), "{distance}");
    }
    let many = Loop2 { segs: (0..=MAX_OFFSET_SEGMENTS).map(|k| Seg2::Line { a: v(k as f64, 0.0), b: v(k as f64 + 1.0, 0.0) }).collect() };
    assert!(offset_region(&solid(many), 1.0).unwrap_err().to_string().contains("segments"));
    // No regions: nothing.
    assert!(offset_regions(&[], 1.0).unwrap().is_empty());
}

/// A fixed linear congruential generator: deterministic, no dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }

    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.next()
    }
}

/// A star-shaped loop about the origin: five to twelve corners at growing angles (so no two
/// neighbours are half a turn apart) and radii of 20 to 30, some sides gently bulged into arcs. It
/// never crosses itself.
fn star(rng: &mut Rng) -> Loop2 {
    let n = 5 + (rng.next() * 8.0) as usize;
    let corners: Vec<Vec2> = (0..n)
        .map(|k| {
            let angle = TAU * (k as f64 + rng.between(0.1, 0.9)) / n as f64;
            let radius = rng.between(20.0, 30.0);
            v(radius * angle.cos(), radius * angle.sin())
        })
        .collect();
    let segs = (0..n)
        .map(|k| {
            let (a, b) = (corners[k], corners[(k + 1) % n]);
            let bulge = if rng.next() < 0.5 { 0.0 } else { rng.between(-0.1, 0.1) };
            if bulge.abs() < 0.01 {
                return Seg2::Line { a, b };
            }
            let (dx, dy) = (b.x - a.x, b.y - a.y);
            let off = (1.0 - bulge * bulge) / (4.0 * bulge);
            let center = v(a.x + 0.5 * dx - dy * off, a.y + 0.5 * dy + dx * off);
            Seg2::Arc {
                center,
                radius: dx.hypot(dy) * (1.0 + bulge * bulge) / (4.0 * bulge.abs()),
                start: (a.y - center.y).atan2(a.x - center.x),
                sweep: 4.0 * bulge.atan(),
            }
        })
        .collect();
    Loop2 { segs }
}

#[test]
fn random_shapes_offset_without_a_panic_and_every_result_is_the_offset() {
    let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
    let mut checked = 0;
    for case in 0..3000 {
        let source = [solid(star(&mut rng))];
        let distance = rng.between(-8.0, 8.0);
        let result = std::panic::catch_unwind(|| offset_regions(&source, distance));
        let Ok(result) = result else { panic!("case {case}: offsetting {source:?} by {distance} panicked") };
        let Ok(moved) = result else { panic!("case {case}: offsetting {source:?} by {distance} was refused: {result:?}") };
        if distance < 0.0 {
            assert_eq!(moved.len(), 1, "case {case}: a shape grown stays one piece");
        }
        assert_is_the_offset(&source, &moved, distance, 1e-6);
        checked += moved.len();
    }
    assert!(checked > 2500, "{checked}");
}

/// A 100 x 80 pocket with one to four round islands that neither touch each other nor the walls.
fn pocket(rng: &mut Rng) -> Region2 {
    let mut islands: Vec<(Vec2, f64)> = Vec::new();
    let wanted = 1 + (rng.next() * 4.0) as usize;
    for _ in 0..40 {
        let radius = rng.between(3.0, 10.0);
        let at = v(rng.between(radius + 1.0, 99.0 - radius), rng.between(radius + 1.0, 79.0 - radius));
        if islands.len() < wanted && islands.iter().all(|(c, r)| (c.x - at.x).hypot(c.y - at.y) > r + radius + 0.5) {
            islands.push((at, radius));
        }
    }
    Region2 { outer: rect(0.0, 0.0, 100.0, 80.0), holes: islands.into_iter().map(|(c, r)| Loop2::circle(c, r).reversed()).collect() }
}

#[test]
fn random_pockets_with_islands_offset_without_a_panic_and_every_result_is_the_offset() {
    // Islands join each other and the walls, pockets split and vanish: whatever is left must
    // still be the offset.
    let mut rng = Rng(0x2545_f491_4f6c_dd1d);
    let (mut pieces, mut gone) = (0, 0);
    for case in 0..1500 {
        let source = [pocket(&mut rng)];
        let distance = rng.between(-6.0, 14.0);
        let result = std::panic::catch_unwind(|| offset_regions(&source, distance));
        let Ok(result) = result else { panic!("case {case}: offsetting {source:?} by {distance} panicked") };
        let Ok(moved) = result else { panic!("case {case}: offsetting {source:?} by {distance} was refused: {result:?}") };
        assert_is_the_offset(&source, &moved, distance, 1e-6);
        // Shrinking never adds area, growing never takes it away.
        let (before, after) = (area(&source), area(&moved));
        assert!(if distance > 0.0 { after < before } else { after >= before }, "case {case}: {before} mm² became {after} mm² at {distance}");
        pieces += moved.len();
        gone += usize::from(moved.is_empty());
    }
    assert!(pieces > 1200 && gone < 300, "{pieces} pieces, {gone} gone");
}

#[test]
fn a_loop_of_twenty_thousand_sides_offsets() {
    // A regular polygon of 20 000 sides on a circle of radius 100, shrunk by 10: the same polygon
    // on a circle of radius 100 - 10 / cos(π / n), whose area is (n / 2) r² sin(2π / n).
    let n = 20_000;
    let corner = |k: usize, r: f64| (r * (TAU * k as f64 / n as f64).cos(), r * (TAU * k as f64 / n as f64).sin());
    let source = [solid(polygon(&(0..n).map(|k| corner(k, 100.0)).collect::<Vec<_>>()))];
    let shrunk = offset_regions(&source, 10.0).unwrap();
    let r = 100.0 - 10.0 / (PI / n as f64).cos();
    // The crate treats sides this short as parallel, so its corners are a hair off: the result is
    // held to the tolerance every offset is verified to, which over the whole outline is at most
    // its length times that tolerance in area.
    let area_tolerance = perimeter(&source) * OFFSET_TOLERANCE;
    assert!(close(area(&shrunk), 0.5 * n as f64 * r * r * (TAU / n as f64).sin(), area_tolerance), "{}", area(&shrunk));
    // Grown, every corner gains a sliver of an arc: the outline is the polygon's plus a full turn
    // of radius 10.
    let grown = offset_regions(&source, -10.0).unwrap();
    assert!(close(perimeter(&grown), perimeter(&source) + TAU * 10.0, 1e-6), "{}", perimeter(&grown));
}

#[test]
fn hostile_loops_never_panic() {
    // Loops that cross themselves, touch, repeat points or overlap: refused or offset to something,
    // never a panic.
    let bow_tie = polygon(&[(0.0, 0.0), (10.0, 10.0), (10.0, 0.0), (0.0, 10.0)]);
    let repeated = polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0), (0.0, 10.0)]);
    let spike = polygon(&[(0.0, 0.0), (10.0, 0.0), (20.0, 0.0), (10.0, 0.0), (10.0, 10.0), (0.0, 10.0)]);
    let tiny = rect(0.0, 0.0, 1e-4, 1e-4);
    let square = rect(0.0, 0.0, 10.0, 10.0);
    let overlapping = Region2 { outer: square.clone(), holes: vec![rect(5.0, 5.0, 15.0, 15.0).reversed()] };
    let outside = Region2 { outer: square.clone(), holes: vec![rect(20.0, 20.0, 25.0, 25.0).reversed()] };
    let twice = [solid(square.clone()), solid(square.clone())];
    let mut sets: Vec<Vec<Region2>> = [bow_tie, repeated, spike, tiny].into_iter().map(|l| vec![solid(l)]).collect();
    sets.extend([vec![overlapping], vec![outside], twice.to_vec()]);
    for regions in &sets {
        for distance in [-3.0, -0.001, 0.0, 0.001, 0.5, 3.0, 50.0] {
            let result = std::panic::catch_unwind(|| offset_regions(regions, distance));
            assert!(result.is_ok(), "offsetting {regions:?} by {distance} panicked");
        }
    }
}

#[test]
fn finely_tessellated_circles_offset_within_a_tenth_of_a_micron() {
    // A circle that arrives as thousands of tiny sides (a mesh outline, a curve chopped fine). The
    // exact answer is known: the polygon's inner radius shrinks by the distance. The result's area
    // may differ from it by at most its outline's length times 0.0001 mm.
    //
    // Found 2026-11-21: on these, `cavalier_contours` 0.9.0 treats neighbouring tiny sides as
    // parallel (its test is on the cross product of the sides, under 1e-5 mm²) and joins them
    // without intersecting them. With its default settings the answers here are within 2e-5 mm, but
    // a debug build trips one of its own `debug_assert!`s on the first case, and other settings or
    // scales gave a vanished loop or took seconds. So every result is to be verified at run time.
    for (r, side, d) in [(20.0, 0.03, 10.0), (20.0, 0.01, 10.0), (5.0, 0.01, 2.0), (1.0, 0.01, 0.5), (50.0, 0.01, 49.0)] {
        let n = (TAU * r / side).round();
        let corners: Vec<(f64, f64)> = (0..n as usize).map(|k| (r * (TAU * k as f64 / n).cos(), r * (TAU * k as f64 / n).sin())).collect();
        let source = [solid(polygon(&corners))];
        let shrunk = std::panic::catch_unwind(|| offset_regions(&source, d));
        let Ok(Ok(shrunk)) = shrunk else { panic!("a circle of radius {r} in sides of {side} mm, shrunk by {d}, panicked or was refused") };
        let inner: f64 = r - d / (PI / n).cos();
        let exact = 0.5 * n * inner * inner * (TAU / n).sin();
        assert!(close(area(&shrunk), exact, TAU * inner * 1e-4), "radius {r}, sides {side}, by {d}: {} mm², exactly {exact}", area(&shrunk));
    }
}

/// The source loops in order and their boundary, as `verify` takes them.
fn sourced(regions: &[Region2]) -> (Vec<Vec<Loop2>>, Boundary) {
    let mut count = 0;
    let source: Vec<Vec<Loop2>> = regions
        .iter()
        .map(|r| {
            let mut loops = vec![checked(&r.outer, true, "outer", &mut count).unwrap()];
            loops.extend(r.holes.iter().map(|h| checked(h, false, "hole", &mut count).unwrap()));
            loops
        })
        .collect();
    let boundary = Boundary::new(source.iter().flatten().flat_map(|l| l.segs.iter().copied()).collect()).unwrap();
    (source, boundary)
}

#[test]
fn the_check_refuses_every_kind_of_wrong_offset() {
    // A 40 square shrunk by 5 is the 30 square from 5 to 35. Each wrong answer below is one a
    // faulty offsetter could give; none may pass.
    let square = [solid(rect(0.0, 0.0, 40.0, 40.0))];
    let (source, boundary) = sourced(&square);
    let check = |result: &[Region2], distance: f64| verify(&source, &boundary, result, distance);
    assert_eq!(check(&[solid(rect(5.0, 5.0, 35.0, 35.0))], 5.0), Ok(()));
    // Within the tolerance is still right: half of it off.
    assert_eq!(check(&[solid(rect(5.000_05, 5.0, 35.000_05, 35.0))], 5.0), Ok(()));
    let wrong: Vec<(&str, Vec<Region2>, f64, &str)> = vec![
        ("nothing at all", vec![], 5.0, "missing"),
        ("moved over by a micron", vec![solid(rect(5.001, 5.0, 35.001, 35.0))], 5.0, "from the outline"),
        ("a micron too small all round", vec![solid(rect(5.001, 5.001, 34.999, 34.999))], 5.0, "from the outline"),
        ("a micron too big all round", vec![solid(rect(4.999, 4.999, 35.001, 35.001))], 5.0, "from the outline"),
        ("with a loop that does not belong", vec![solid(rect(5.0, 5.0, 35.0, 35.0)), solid(rect(18.0, 18.0, 22.0, 22.0))], 5.0, "from the outline"),
        ("the offset for another distance", vec![solid(rect(6.0, 6.0, 34.0, 34.0))], 5.0, "from the outline"),
        ("a corner cut off", vec![solid(polygon(&[(5.0, 5.0), (35.0, 5.0), (35.0, 30.0), (30.0, 35.0), (5.0, 35.0)]))], 5.0, "from the outline"),
    ];
    for (what, result, distance, expect) in &wrong {
        let e = check(result, *distance).expect_err(what);
        assert!(e.contains(expect), "{what}: {e:?} does not mention {expect:?}");
    }
    // The square grown by 5 is at the right distance, but on the wrong side for a shrink, and the
    // other way round.
    let grown = offset_regions(&square, -5.0).unwrap();
    assert_eq!(check(&grown, -5.0), Ok(()));
    assert!(check(&grown, 5.0).unwrap_err().contains("wrong side"));
    assert!(check(&[solid(rect(5.0, 5.0, 35.0, 35.0))], -5.0).unwrap_err().contains("wrong side"));
    // A square grown with sharp corners instead of round ones: its corners are too far out.
    assert!(check(&[solid(rect(-5.0, -5.0, 45.0, 45.0))], -5.0).unwrap_err().contains("from the outline"));
}

#[test]
fn the_check_notices_a_piece_or_an_island_gone_missing() {
    // The neck that closes leaves two pieces: one alone is not the offset.
    let dumbbell = [solid(polygon(&[
        (0.0, 0.0),
        (30.0, 0.0),
        (30.0, 13.0),
        (40.0, 13.0),
        (40.0, 0.0),
        (70.0, 0.0),
        (70.0, 30.0),
        (40.0, 30.0),
        (40.0, 17.0),
        (30.0, 17.0),
        (30.0, 30.0),
        (0.0, 30.0),
    ]))];
    let (source, boundary) = sourced(&dumbbell);
    let pieces = offset_regions(&dumbbell, 3.0).unwrap();
    assert_eq!(verify(&source, &boundary, &pieces, 3.0), Ok(()));
    assert!(verify(&source, &boundary, &pieces[..1], 3.0).unwrap_err().contains("missing"));
    assert!(verify(&source, &boundary, &pieces[1..], 3.0).unwrap_err().contains("missing"));
    // A pocket with an island: the path without the island's loop would plough through the island.
    let pocket = [pocket_with_island()];
    let (source, boundary) = sourced(&pocket);
    let path = offset_regions(&pocket, 5.0).unwrap();
    assert_eq!(verify(&source, &boundary, &path, 5.0), Ok(()));
    let no_island = [solid(path[0].outer.clone())];
    assert!(verify(&source, &boundary, &no_island, 5.0).unwrap_err().contains("missing"));
    // And the island's loop alone, without the walls'.
    let only_island = [solid(path[0].holes[0].reversed())];
    assert!(verify(&source, &boundary, &only_island, 5.0).is_err());
    // Where the pocket is only just as wide as the offset is deep, nothing has to be there: a slot
    // 10 wide shrunk by 5 is a line, and no result at all is right.
    let slot = [solid(rect(0.0, 0.0, 50.0, 10.0))];
    let (source, boundary) = sourced(&slot);
    assert_eq!(verify(&source, &boundary, &[], 5.0), Ok(()));
    assert_eq!(offset_regions(&slot, 5.0).map(|r| r.len()), Ok(0));
    // A hair narrower offset leaves a sliver thinner than the check insists on: either answer passes.
    assert!(offset_regions(&slot, 4.9995).is_ok());
}

#[test]
fn the_nearest_distance_agrees_with_checking_every_segment() {
    let mut rng = Rng(0x0123_4567_89ab_cdef);
    for _ in 0..40 {
        let mut segs: Vec<Seg2> = star(&mut rng).segs;
        segs.extend(Loop2::circle(v(rng.between(-40.0, 40.0), rng.between(-40.0, 40.0)), rng.between(1.0, 15.0)).segs);
        segs.push(Seg2::Arc {
            center: v(rng.between(-40.0, 40.0), rng.between(-40.0, 40.0)),
            radius: rng.between(1.0, 30.0),
            start: rng.between(-7.0, 7.0),
            sweep: rng.between(-6.0, 6.0),
        });
        let boundary = Boundary::new(segs.clone()).unwrap();
        for _ in 0..200 {
            let p = v(rng.between(-60.0, 60.0), rng.between(-60.0, 60.0));
            let every = segs.iter().map(|s| distance_to(p, s)).fold(f64::INFINITY, f64::min);
            assert!((boundary.distance(p) - every).abs() < 1e-12, "{p:?}: {} by the index, {every} by every segment", boundary.distance(p));
            // And ours agrees with the test's own, written separately.
            assert!((every - segs.iter().map(|s| to_seg(p, s)).fold(f64::INFINITY, f64::min)).abs() < 1e-12);
        }
    }
    assert!(Boundary::new(vec![]).is_none());
}

#[test]
fn winding_counts_the_turns_of_lines_and_arcs() {
    let square = rect(0.0, 0.0, 10.0, 10.0);
    let circle = Loop2::circle(v(0.0, 0.0), 10.0);
    // Three quarters of a circle of radius 10 and two sides to its centre: the quarter where X is
    // above 0 and Y below is outside it, though inside the arc's circle.
    let keyhole = Loop2 {
        segs: vec![
            Seg2::Line { a: v(0.0, 0.0), b: v(10.0, 0.0) },
            Seg2::Arc { center: v(0.0, 0.0), radius: 10.0, start: 0.0, sweep: 1.5 * PI },
            Seg2::Line { a: v(0.0, -10.0), b: v(0.0, 0.0) },
        ],
    };
    let cases: Vec<(&Loop2, (f64, f64), i64)> = vec![
        (&square, (5.0, 5.0), 1),
        (&square, (0.001, 9.999), 1),
        (&square, (-1.0, 5.0), 0),
        (&square, (5.0, 10.001), 0),
        (&square, (1e4, 1e4), 0),
        (&circle, (0.0, 0.0), 1),
        (&circle, (9.9, 0.0), 1),
        (&circle, (-7.0, 7.0), 1),
        (&circle, (10.1, 0.0), 0),
        (&circle, (-8.0, 8.0), 0),
        (&keyhole, (-5.0, 5.0), 1),
        (&keyhole, (5.0, 5.0), 1),
        (&keyhole, (-5.0, -5.0), 1),
        (&keyhole, (5.0, -5.0), 0),
        (&keyhole, (0.1, -0.1), 0),
        (&keyhole, (-0.1, -0.1), 1),
        (&keyhole, (12.0, 0.0), 0),
    ];
    for (l, (x, y), turns) in cases {
        assert_eq!(winding(v(x, y), l), turns, "({x}, {y})");
        // The same loop run the other way winds the other way.
        assert_eq!(winding(v(x, y), &l.reversed()), -turns, "({x}, {y}) reversed");
    }
    // A hole takes its turn away again: in the island of the pocket nothing is left.
    let pocket = pocket_with_island();
    let turns = |x: f64, y: f64| pocket.loops().map(|l| winding(v(x, y), l)).sum::<i64>();
    assert_eq!((turns(20.0, 20.0), turns(50.0, 50.0), turns(120.0, 50.0)), (1, 0, 0));
}
