use std::f64::consts::PI;

use super::*;

fn q(x: f64, y: f64) -> Vec2 {
    Vec2 { x, y }
}

/// A closed loop of straight sides through `points`, in order.
fn polygon(points: &[(f64, f64)]) -> Loop2 {
    let n = points.len();
    Loop2 { segs: (0..n).map(|i| Seg2::Line { a: q(points[i].0, points[i].1), b: q(points[(i + 1) % n].0, points[(i + 1) % n].1) }).collect() }
}

fn region(points: &[(f64, f64)]) -> Region2 {
    Region2 { outer: polygon(points), holes: Vec::new() }
}

fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<(f64, f64)> {
    vec![(x0, y0), (x1, y0), (x1, y1), (x0, y1)]
}

/// The tests' part is a block 100 x 80 about the origin with 1 mm of stock round it, and their
/// tool is 10 mm across: its centre 7 mm beside the stock (2 mm of air to its edge) is in the
/// air, which is outside this box.
fn beside() -> (Vec2, Vec2) {
    (q(-58.0, -48.0), q(58.0, 48.0))
}

/// The plan for the 10 mm tool at a stepover of 4 mm, nothing left on the walls but `finish`.
fn planned(open: &OpenFloor, finish: f64, direction: CutDirection) -> OpenPlan {
    plan(open, beside(), 5.0, (5.0, finish), 4.0, direction).unwrap()
}

/// The places a cut goes through, in order: its start and the end of each of its pieces, to a
/// micron.
fn corners(step: &Step) -> Vec<(f64, f64)> {
    let micron = |v: f64| (v * 1e3).round() / 1e3 + 0.0;
    std::iter::once(step.start).chain(step.segs.iter().map(Seg2Geom::end)).map(|p| (micron(p.x), micron(p.y))).collect()
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6
}

/// A step 10 wide along the front of the block, from its front edge (Y -40) to a wall at Y -30.
fn step() -> OpenFloor {
    OpenFloor { floor: vec![region(&rect(-50.0, -40.0, 50.0, -30.0))], walls: vec![region(&rect(-50.0, -30.0, 50.0, 40.0))] }
}

#[test]
fn a_step_is_cut_in_passes_along_its_wall_the_furthest_first_each_from_beside_the_stock() {
    let plan = planned(&step(), 0.0, CutDirection::Climb);
    // Three passes, 13, 9 and 5 mm from the wall: at 13 the cutter still reaches 2 mm over the
    // floor's open edge (and takes the stock beside it), at 17 it would be past it. Climb has
    // the wall on the tool's right: from +X to -X. Each goes from 7 mm beside the stock (X 58)
    // straight through to 7 mm beside it at the other end, and does not roll round the wall's
    // ends.
    assert_eq!(
        plan.steps.iter().map(corners).collect::<Vec<_>>(),
        vec![
            vec![(58.0, -43.0), (50.0, -43.0), (-50.0, -43.0), (-58.0, -43.0)],
            vec![(58.0, -39.0), (50.0, -39.0), (-50.0, -39.0), (-58.0, -39.0)],
            vec![(58.0, -35.0), (50.0, -35.0), (-50.0, -35.0), (-58.0, -35.0)],
        ]
    );
    // Down in the air, the first 8 mm (to the wall's end) fed slower, up after each.
    for step in &plan.steps {
        assert!(step.entry == Entry::Air && step.from.is_none() && !step.closed && step.lift && near(step.slow, 8.0) && near(step.length(), 116.0));
    }
    assert!(plan.everywhere && plan.against == 0 && plan.finishing.is_empty());
    // Conventional is the other way along each.
    let other = planned(&step(), 0.0, CutDirection::Conventional);
    assert_eq!(corners(&other.steps[0]), vec![(-58.0, -43.0), (-50.0, -43.0), (50.0, -43.0), (58.0, -43.0)]);
    assert_eq!(other.steps.len(), 3);
    // With 0.5 mm left for a finishing pass the passes keep that much further off, and the
    // finishing pass goes along the wall itself, from the air as they do.
    let finished = planned(&step(), 0.5, CutDirection::Climb);
    assert_eq!(finished.steps.iter().map(|step| corners(step)[1].1).collect::<Vec<_>>(), vec![-43.5, -39.5, -35.5]);
    assert_eq!(finished.finishing.iter().map(corners).collect::<Vec<_>>(), vec![vec![(58.0, -35.0), (50.0, -35.0), (-50.0, -35.0), (-58.0, -35.0)]]);
    assert!(finished.finishing[0].entry == Entry::Air && finished.finishing[0].from.is_none());
}

#[test]
fn a_shoulder_is_followed_round_its_corner_and_a_pass_with_no_way_in_is_cut_from_its_far_end() {
    // The step stops at X 20, where the block is whole again: a wall along Y -30 and one along
    // X 20, which ends at the block's front (Y -40).
    let wall = [(-50.0, -30.0), (20.0, -30.0), (20.0, -40.0), (50.0, -40.0), (50.0, 40.0), (-50.0, 40.0)];
    let open = OpenFloor { floor: vec![region(&rect(-50.0, -40.0, 20.0, -30.0))], walls: vec![region(&wall)] };
    let plan = planned(&open, 0.0, CutDirection::Climb);
    let cut: Vec<Vec<(f64, f64)>> = plan.steps.iter().map(corners).collect();
    // 13 mm from the walls the pass ends where it comes within 13 of the wall's end at
    // (20, -40): at X 20 - sqrt(13^2 - 3^2) = 7.351. There is no air there to start in (the block
    // is in the way), nothing was cut before it, so it is cut from its other end, against climb.
    assert_eq!(cut[0], vec![(-58.0, -43.0), (-50.0, -43.0), (7.351, -43.0)]);
    // 9 and 5 mm from them the passes come in from beside the front of the stock (Y -48), along
    // the wall at X 20, round the corner and out at the left end.
    assert_eq!(cut[1], vec![(11.0, -48.0), (11.0, -40.0), (11.0, -39.0), (-50.0, -39.0), (-58.0, -39.0)]);
    assert_eq!(cut[2], vec![(15.0, -48.0), (15.0, -40.0), (15.0, -35.0), (-50.0, -35.0), (-58.0, -35.0)]);
    assert_eq!((plan.steps.len(), plan.against, plan.everywhere), (3, 1, true));
    assert!(plan.steps.iter().all(|step| step.entry == Entry::Air && step.from.is_none() && near(step.slow, 8.0)));
}

#[test]
fn a_channel_open_at_one_end_is_cut_from_its_mouth_the_middle_first() {
    // A channel 20 wide from the block's front 40 mm in: walls at X -10 and 10 and across Y 0.
    let wall = [(-50.0, -40.0), (-10.0, -40.0), (-10.0, 0.0), (10.0, 0.0), (10.0, -40.0), (50.0, -40.0), (50.0, 40.0), (-50.0, 40.0)];
    let open = OpenFloor { floor: vec![region(&rect(-10.0, -40.0, 10.0, 0.0))], walls: vec![region(&wall)] };
    let plan = planned(&open, 0.0, CutDirection::Climb);
    // 9 mm from the walls first, a pass up the middle and back 2 mm apart; then 5 mm from
    // them. Each from beside the stock in front of the mouth, up along the wall on its right,
    // across the end and out again; at 13 mm there is no room in a channel 20 wide.
    assert_eq!(
        plan.steps.iter().map(corners).collect::<Vec<_>>(),
        vec![
            vec![(1.0, -48.0), (1.0, -40.0), (1.0, -9.0), (-1.0, -9.0), (-1.0, -40.0), (-1.0, -48.0)],
            vec![(5.0, -48.0), (5.0, -40.0), (5.0, -5.0), (-5.0, -5.0), (-5.0, -40.0), (-5.0, -48.0)],
        ]
    );
    assert!(plan.everywhere && plan.against == 0);
    assert!(plan.steps.iter().all(|step| step.entry == Entry::Air && step.from.is_none() && near(step.slow, 8.0)));
    // A tool exactly as wide as the channel goes up its middle and back, from the mouth: one
    // cut, within the micron of fit of the middle all the way, and no further in than its own
    // radius from the end.
    let fits = plan_for(&open, 10.0);
    assert_eq!(fits.steps.len(), 1, "{:?}", fits.steps.iter().map(corners).collect::<Vec<_>>());
    let way: Vec<Vec2> = std::iter::once(fits.steps[0].start).chain(fits.steps[0].segs.iter().map(Seg2Geom::end)).collect();
    assert!(way.iter().all(|p| p.x.abs() <= 1.5e-3 && p.y <= -10.0 + 1.5e-3), "{way:?}");
    let (first, last, deepest) = (way[0], way[way.len() - 1], way.iter().map(|p| p.y).fold(f64::NEG_INFINITY, f64::max));
    assert!(near(first.y, -53.0) && near(last.y, -53.0) && (deepest + 10.0).abs() <= 1.5e-3, "{way:?}");
    assert!(fits.steps[0].entry == Entry::Air && fits.steps[0].from.is_none());
    // A wider one has nowhere to be in it.
    let wide = plan_for(&open, 10.5);
    assert!(wide.steps.is_empty(), "{:?}", wide.steps.iter().map(corners).collect::<Vec<_>>());
}

/// The plan for a tool of the radius `radius`, at a stepover of 80% of it.
fn plan_for(open: &OpenFloor, radius: f64) -> OpenPlan {
    plan(open, (q(-53.0 - radius, -43.0 - radius), q(53.0 + radius, 43.0 + radius)), radius, (radius, 0.0), 0.8 * radius, CutDirection::Climb)
        .unwrap()
}

#[test]
fn the_floor_round_a_boss_is_cut_from_the_corners_in_and_then_round_and_round() {
    // A boss 40 x 30 in the middle of the block, the block's top round it the floor.
    let open = OpenFloor { floor: vec![region(&rect(-50.0, -40.0, 50.0, 40.0))], walls: vec![region(&rect(-20.0, -15.0, 20.0, 15.0))] };
    let plan = planned(&open, 0.0, CutDirection::Climb);
    // 41 and 37 mm from the boss the cutter is over the floor only at the block's four
    // corners; 33 mm from it also along the block's two ends; from 29 mm in the pass goes all
    // the way round. 45 mm from it the cutter is past the corners.
    let closed: Vec<bool> = plan.steps.iter().map(|step| step.closed).collect();
    assert_eq!(closed, [vec![false; 10], vec![true; 7]].concat());
    // The stretches start and end in the air beside the stock.
    for step in &plan.steps[..10] {
        let (first, last) = (step.start, step.segs.last().unwrap().end());
        let out = |p: Vec2| p.x.abs() >= 58.0 - 1e-6 || p.y.abs() >= 48.0 - 1e-6;
        assert!(step.entry == Entry::Air && step.from.is_none() && step.slow > 0.0 && out(first) && out(last), "{:?}", corners(step));
    }
    // A pass that goes all the way round has no end to come in at: the tool goes down on the
    // pass before, 4 mm further out, in the air that one cut, and across. Its length is the
    // boss's own way round and a circle of its distance from it.
    for (k, step) in plan.steps[10..].iter().enumerate() {
        let distance = 29.0 - 4.0 * k as f64;
        let from = step.from.unwrap();
        assert!(step.entry == Entry::Air && (apart(from, step.start) - 4.0).abs() < 1e-3, "{k}: {from:?} {:?}", step.start);
        assert!((step.length() - (140.0 + 2.0 * PI * distance)).abs() < 1e-3, "{k}: {}", step.length());
        assert!((away(&open.walls, step.start) - distance).abs() < 1e-3 && (away(&open.walls, from) - distance - 4.0).abs() < 1e-3);
    }
    assert!(plan.everywhere && plan.against == 0);
}

#[test]
fn a_pass_rolls_round_the_end_of_a_wall_that_the_floor_goes_on_past() {
    // The step along the whole front, but the wall behind it only on the left half: right of
    // X 0 the floor's back edge is open too.
    let open = OpenFloor { floor: vec![region(&rect(-50.0, -40.0, 50.0, -30.0))], walls: vec![region(&rect(-50.0, -30.0, 0.0, 40.0))] };
    let plan = planned(&open, 0.0, CutDirection::Conventional);
    // The pass on the wall, last: along it from the left end, a quarter turn round its end at
    // (0, -30), which is over the floor, and up its other side for as long as the cutter still
    // reaches over the floor's edge (to Y -25.01).
    let on_the_wall = plan.steps.last().unwrap();
    assert_eq!(corners(on_the_wall), vec![(-58.0, -35.0), (-50.0, -35.0), (0.0, -35.0), (5.0, -30.0), (5.0, -25.01)]);
    assert!(matches!(on_the_wall.segs[2], Seg2::Arc { center, radius, .. } if apart(center, q(0.0, -30.0)) < 1e-9 && near(radius, 5.0)));
    // The floor's far end is 51 mm from the wall's end: the passes go on out to there, each
    // an arc about the wall's end that comes in over the floor's front edge from beside the
    // stock, and every place on the floor is reached.
    assert!(plan.everywhere);
    let furthest = plan.steps.iter().flat_map(|step| step.segs.iter().map(Seg2Geom::end)).map(|p| away(&open.walls, p)).fold(0.0, f64::max);
    assert!((49.0..=56.0).contains(&furthest), "{furthest}");
    for step in &plan.steps {
        for seg in &step.segs {
            for k in 0..=20 {
                let p = seg.point_at(f64::from(k) / 20.0);
                assert!(away(&open.walls, p) >= 5.0 - 1e-3, "{p:?}");
            }
        }
    }
}

/// A sequence of numbers from 0 to 1 that is the same every run.
struct Dice(u64);

impl Dice {
    fn roll(&mut self) -> f64 {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 11) as f64) / ((1u64 << 53) as f64)
    }

    fn between(&mut self, lo: f64, hi: f64) -> f64 {
        lo + (hi - lo) * self.roll()
    }
}

#[test]
fn passes_round_walls_anywhere_keep_off_them_start_in_the_air_and_reach_the_whole_floor() {
    let mut dice = Dice(20_261_124);
    let floor = rect(-50.0, -40.0, 50.0, 40.0);
    let mut cuts = 0;
    for case in 0..40 {
        // One to three blocks standing on the block's top, on a grid so that none overlap; one
        // of them may reach the block's edge.
        let mut walls: Vec<Region2> = Vec::new();
        for cell in 0..3 {
            if cell > 0 && dice.roll() < 0.4 {
                continue;
            }
            let (x0, x1) = (-48.0 + 33.0 * f64::from(cell), -18.0 + 33.0 * f64::from(cell));
            let (a, b) = (dice.between(x0, x0 + 12.0), dice.between(x1 - 12.0, x1));
            let (c, d) = (if dice.roll() < 0.3 { -40.0 } else { dice.between(-36.0, -10.0) }, dice.between(0.0, 36.0));
            walls.push(region(&rect(a, c, b, d)));
        }
        let (radius, leave) = (dice.between(1.5, 8.0), if dice.roll() < 0.5 { 0.0 } else { dice.between(0.0, 0.5) });
        let direction = if dice.roll() < 0.5 { CutDirection::Climb } else { CutDirection::Conventional };
        let open = OpenFloor { floor: vec![region(&floor)], walls };
        let lo_hi = (q(-53.0 - radius, -43.0 - radius), q(53.0 + radius, 43.0 + radius));
        let offset = radius + leave;
        let plan =
            plan(&open, lo_hi, radius, (offset, 0.0), dice.between(0.3, 1.0) * radius, direction).unwrap_or_else(|e| panic!("case {case}: {e}"));
        assert!(!plan.steps.is_empty() && plan.everywhere, "case {case}: {} cuts, everywhere {}", plan.steps.len(), plan.everywhere);
        let in_the_air = |p: Vec2| p.x <= lo_hi.0.x + 1e-6 || p.x >= lo_hi.1.x - 1e-6 || p.y <= lo_hi.0.y + 1e-6 || p.y >= lo_hi.1.y - 1e-6;
        for (k, step) in plan.steps.iter().enumerate() {
            // No place on a cut, or on the way to it, is nearer a wall than the tool may be.
            let way: Vec<Seg2> = step.from.map(|from| Seg2::Line { a: from, b: step.start }).into_iter().chain(step.segs.iter().copied()).collect();
            for seg in &way {
                for j in 0..=40 {
                    let p = seg.point_at(f64::from(j) / 40.0);
                    assert!(away(&open.walls, p) >= offset - 1e-3, "case {case} cut {k}: {p:?} is {} from a wall", away(&open.walls, p));
                }
            }
            // The tool goes down in the air beside the stock, or on a pass cut before, or down
            // a ramp along the cut itself.
            let down = step.from.unwrap_or(step.start);
            let cut_before = plan.steps[..k].iter().flat_map(|s| s.segs.iter()).any(|seg| apart(down, seg.point_at(seg.closest_param(down))) <= 1e-6);
            match step.entry {
                Entry::Air => assert!(in_the_air(down) || (step.from.is_some() && cut_before), "case {case} cut {k}: down at {down:?}"),
                Entry::Ramp => assert!(step.closed, "case {case} cut {k}"),
                Entry::ZigZag => assert!(!step.closed, "case {case} cut {k}"),
                Entry::Link => panic!("case {case} cut {k}: a link"),
            }
        }
        cuts += plan.steps.len();
    }
    assert!(cuts > 200, "{cuts}");
}

#[test]
fn hostile_floors_never_panic() {
    let wall = region(&rect(-50.0, -30.0, 50.0, 40.0));
    let floor = region(&rect(-50.0, -40.0, 50.0, -30.0));
    let empty = Region2 { outer: Loop2 { segs: Vec::new() }, holes: Vec::new() };
    for open in [
        OpenFloor { floor: Vec::new(), walls: Vec::new() },
        OpenFloor { floor: vec![floor.clone()], walls: Vec::new() },
        OpenFloor { floor: Vec::new(), walls: vec![wall.clone()] },
        OpenFloor { floor: vec![empty.clone()], walls: vec![wall.clone()] },
        OpenFloor { floor: vec![floor.clone()], walls: vec![empty] },
        OpenFloor { floor: vec![floor.clone()], walls: vec![region(&[(f64::NAN, 0.0), (1.0, 0.0), (1.0, 1.0)])] },
    ] {
        for (radius, stepover) in [(5.0, 4.0), (0.0, 4.0), (5.0, 0.0), (f64::NAN, 4.0), (5.0, f64::INFINITY), (1e300, 1e-300), (-5.0, -4.0)] {
            let _ = plan(&open, beside(), radius, (radius, 0.0), stepover, CutDirection::Climb);
            let _ = plan(&open, (q(f64::NAN, 0.0), q(f64::INFINITY, -1.0)), radius, (radius, f64::NAN), stepover, CutDirection::Conventional);
        }
    }
}

#[test]
fn how_wide_a_tool_may_be_and_what_stands_on_the_floor() {
    // Nothing stands across a step from its wall: a tool of any width goes along it.
    assert_eq!((step().across(), step().islands()), (MAX_COORDINATE, 0));
    // A channel 20 wide takes a tool no wider than that (and the fit, a micron a side).
    let wall = [(-50.0, -40.0), (-10.0, -40.0), (-10.0, 0.0), (10.0, 0.0), (10.0, -40.0), (50.0, -40.0), (50.0, 40.0), (-50.0, 40.0)];
    let channel = OpenFloor { floor: vec![region(&rect(-10.0, -40.0, 10.0, 0.0))], walls: vec![region(&wall)] };
    assert!((channel.across() - 20.002).abs() < 1e-9 && channel.islands() == 0, "{}", channel.across());
    // A boss stands on the floor round it, and a tool of any width goes round it.
    let boss = OpenFloor { floor: vec![region(&rect(-50.0, -40.0, 50.0, 40.0))], walls: vec![region(&rect(-20.0, -15.0, 20.0, 15.0))] };
    assert_eq!((boss.across(), boss.islands()), (MAX_COORDINATE, 1));
    // The same floor in four faces round the boss, as a model may have it.
    let four = OpenFloor {
        floor: vec![
            region(&rect(-50.0, -40.0, 50.0, -15.0)),
            region(&rect(-50.0, 15.0, 50.0, 40.0)),
            region(&rect(-50.0, -15.0, -20.0, 15.0)),
            region(&rect(20.0, -15.0, 50.0, 15.0)),
        ],
        walls: vec![region(&rect(-20.0, -15.0, 20.0, 15.0))],
    };
    assert_eq!((four.across(), four.islands()), (MAX_COORDINATE, 1));
    assert_eq!(planned(&four, 0.0, CutDirection::Climb).steps.len(), 17);
    // No floor, no walls: nothing, and no panic.
    assert_eq!(OpenFloor { floor: Vec::new(), walls: Vec::new() }.across(), 0.0);
}
