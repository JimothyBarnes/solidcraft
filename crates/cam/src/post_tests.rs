use std::f64::consts::PI;

use super::*;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

/// A small program on a 0.001 mm grid: a tool change, a plunge, a quarter arc, a retract.
fn bracket() -> Toolpath {
    Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![
            Move::ToolChange { tool: 1 },
            Move::Spindle { rpm: 1200.0, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(10.0, 0.0, 5.0) },
            Move::Linear { to: p(10.0, 0.0, -1.0), feed: 200.0 },
            Move::Arc { to: p(0.0, 10.0, -1.0), center: p(0.0, 0.0, -1.0), plane: Plane::Xy, direction: ArcDirection::CounterClockwise, feed: 400.0 },
            Move::Rapid { to: p(0.0, 10.0, 5.0) },
            Move::SpindleStop,
            Move::Coolant { mode: Coolant::Off },
        ],
    }
}

fn ctx() -> ProgramContext {
    ProgramContext { program_number: 1, program_name: "Bracket op1".into(), units: LengthUnit::Millimetre, work_offset: 1 }
}

/// The largest difference between two toolpaths' positions and arc centres, in mm.
fn max_difference(a: &Toolpath, b: &Toolpath) -> f64 {
    let d = |p: Vec3, q: Vec3| (p.x - q.x).abs().max((p.y - q.y).abs()).max((p.z - q.z).abs());
    a.moves
        .iter()
        .zip(&b.moves)
        .map(|(m, n)| match (m, n) {
            (Move::Rapid { to: p }, Move::Rapid { to: q }) | (Move::Linear { to: p, .. }, Move::Linear { to: q, .. }) => d(*p, *q),
            (Move::Arc { to: p, center: c, plane, .. }, Move::Arc { to: q, center: e, .. }) => {
                let (cu, cv, _) = plane.split(*c);
                let (eu, ev, _) = plane.split(*e);
                d(*p, *q).max((cu - eu).abs()).max((cv - ev).abs())
            }
            _ => 0.0,
        })
        .fold(0.0, f64::max)
}

#[test]
fn every_built_in_post_loads() {
    for (id, _) in BUILTIN_POSTS {
        let post = Post::builtin(id).unwrap();
        assert_eq!(post.id, *id);
    }
    assert!(Post::builtin("nope").is_err());
}

#[test]
fn the_fanuc_style_post_writes_this_program_exactly() {
    // Derived by hand from the post file: tape marks and the O line unnumbered, the safety lines
    // first, a machine-home retract before the tool change, the length offset on the first rapid,
    // the plane before the first arc, modal words left out when unchanged.
    let expected = "\
%
O0001 (BRACKET OP1)
N10 (UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)
N20 (PROVE OUT FIRST: AIR CUT, SINGLE BLOCK)
N30 G17 G40 G49 G80 G90 G94 G21
N40 G54
N50 G91 G28 Z0.
N60 G90
N70 T1 M6
N80 S1200 M3
N90 M8
N100 G43 H1 G0 X10. Y0. Z5.
N110 G1 Z-1. F200.
N120 G17
N130 G3 X0. Y10. I-10. J0. F400.
N140 G0 Z5.
N150 M5
N160 M9
N170 G91 G28 Z0.
N180 G90
N190 M30
%
";
    let posted = Post::builtin("fanuc_mill").unwrap().write(&bracket(), &ctx()).unwrap();
    assert_eq!(posted.text, expected);
    assert_eq!(posted.read_back.machine_retracts, 2);
}

#[test]
fn a_hand_made_toolpath_reads_back_within_a_micron_from_both_posts() {
    // P0's acceptance (SPEC §5): post, read back, the same moves within 1e-6 mm.
    let mut tp = bracket();
    // Add a full circle, a helix and a side-plane arc before the retract.
    let extra = [
        Move::Arc { to: p(0.0, 10.0, -1.0), center: p(0.0, 5.0, -1.0), plane: Plane::Xy, direction: ArcDirection::Clockwise, feed: 400.0 },
        Move::Arc { to: p(0.0, 10.0, -3.5), center: p(0.0, 5.0, 0.0), plane: Plane::Xy, direction: ArcDirection::CounterClockwise, feed: 300.0 },
        Move::Linear { to: p(0.0, 10.0, 0.0), feed: 300.0 },
        Move::Arc { to: p(10.0, 10.0, 10.0), center: p(10.0, 10.0, 0.0), plane: Plane::Zx, direction: ArcDirection::CounterClockwise, feed: 250.0 },
    ];
    tp.moves.splice(6..6, extra);
    tp.validate().unwrap();
    for id in ["rs274", "fanuc_mill", "haas_mill"] {
        let posted = Post::builtin(id).unwrap().write(&tp, &ctx()).unwrap();
        assert_eq!(posted.written.moves.len(), posted.read_back.toolpath.moves.len(), "{id}");
        let diff = max_difference(&posted.written, &posted.read_back.toolpath);
        assert!(diff <= 1e-6, "{id}: read back {diff} mm off");
        // What was written is the toolpath meant: splitting arcs keeps their length.
        let (meant, wrote) = (tp.stats().unwrap(), posted.written.stats().unwrap());
        assert!((meant.feed_length - wrote.feed_length).abs() < 1e-9, "{id}");
    }
    // The Fanuc-style post takes at most half a turn per block: the full circle and the helix
    // are two blocks each (their midpoints are on the grid, so the micron check above holds).
    let fanuc = Post::builtin("fanuc_mill").unwrap().write(&tp, &ctx()).unwrap();
    assert_eq!(fanuc.written.moves.len(), tp.moves.len() + 2);
}

#[test]
fn a_split_point_off_the_grid_reads_back_within_the_posts_tolerance() {
    // Three quarters of a turn, split at 135° for the Fanuc-style post's half-turn limit: the
    // split point 10·sin 135° = 7.0710678… rounds to 3 places, so it reads back within 0.0005 mm.
    let tp = Toolpath {
        start: p(0.0, 10.0, 0.0),
        moves: vec![
            Move::Rapid { to: p(0.0, 10.0, 0.0) },
            Move::Arc {
                to: p(10.0, 10.0, 10.0),
                center: p(0.0, 10.0, 10.0),
                plane: Plane::Zx,
                direction: ArcDirection::CounterClockwise,
                feed: 250.0,
            },
        ],
    };
    let posted = Post::builtin("fanuc_mill").unwrap().write(&tp, &ctx()).unwrap();
    assert_eq!(posted.written.moves.iter().filter(|m| matches!(m, Move::Arc { .. })).count(), 2);
    let diff = max_difference(&posted.written, &posted.read_back.toolpath);
    assert!(diff > 1e-6 && diff <= posted.tolerance + 1e-9, "{diff}");
}

#[test]
fn inch_output_reads_back_within_half_its_last_digit() {
    let ctx = ProgramContext { units: LengthUnit::Inch, ..ctx() };
    let posted = Post::builtin("fanuc_mill").unwrap().write(&bracket(), &ctx).unwrap();
    assert!(posted.text.contains("G20"));
    assert!(posted.text.contains("X0.3937 Y0. Z0.1969")); // 10 mm, 5 mm in inches to 4 places
    assert!((posted.tolerance - 0.00127).abs() < 1e-12);
    assert!(max_difference(&posted.written, &posted.read_back.toolpath) <= posted.tolerance + 1e-9);
}

#[test]
fn numbers_always_carry_a_decimal_point_and_never_a_minus_zero() {
    assert_eq!(format_decimal(10.0, 3, true), "10.");
    assert_eq!(format_decimal(10.0, 3, false), "10.000");
    assert_eq!(format_decimal(-0.0001, 3, true), "0.");
    assert_eq!(format_decimal(-1.23456, 3, true), "-1.235");
    assert_eq!(format_decimal(0.5, 1, true), "0.5");
}

#[test]
fn an_arc_too_small_to_round_is_written_as_a_line_not_a_full_circle() {
    // A 0.0002 mm long arc: its ends round to the same 0.001 mm, which a reader would take as a
    // full circle of radius 1. It must become a line (or nothing), never a circle.
    let a = 0.0002_f64; // radians at radius 1
    let tp = Toolpath {
        start: p(1.0, 0.0, 0.0),
        moves: vec![
            Move::Rapid { to: p(1.0, 0.0, 0.0) },
            Move::Arc {
                to: p(a.cos(), a.sin(), 0.0),
                center: p(0.0, 0.0, 0.0),
                plane: Plane::Xy,
                direction: ArcDirection::CounterClockwise,
                feed: 100.0,
            },
            Move::Linear { to: p(2.0, 0.0, 0.0), feed: 100.0 },
        ],
    };
    let posted = Post::builtin("fanuc_mill").unwrap().write(&tp, &ctx()).unwrap();
    assert!(!posted.text.contains("G3"), "{}", posted.text);
    let read = posted.read_back.toolpath.stats().unwrap();
    assert!(read.feed_length < 1.01, "read back {} mm of cutting", read.feed_length);
}

#[test]
fn an_arc_in_a_plane_the_post_lacks_becomes_lines_within_tolerance() {
    let mut post = Post::builtin("rs274").unwrap();
    post.arcs.planes = vec![Plane::Xy];
    let tp = Toolpath {
        start: p(0.0, 0.0, 10.0),
        moves: vec![
            Move::Rapid { to: p(0.0, 0.0, 10.0) },
            Move::Arc { to: p(10.0, 0.0, 0.0), center: p(0.0, 0.0, 0.0), plane: Plane::Zx, direction: ArcDirection::CounterClockwise, feed: 100.0 },
        ],
    };
    let posted = post.write(&tp, &ctx()).unwrap();
    // Radius 10, a quarter turn, 0.001 mm: each line spans 2·acos(1 − 0.0001) rad, so 56 lines.
    assert_eq!(posted.written.moves.iter().filter(|m| matches!(m, Move::Linear { .. })).count(), 56);
    assert!(posted.notes.iter().any(|n| n.contains("56 lines")));
    // Every line's midpoint stays within the tolerance of the circle.
    let mut at = tp.start;
    for m in &posted.written.moves {
        let to = m.end(at);
        let mid = p(0.5 * (at.x + to.x), 0.0, 0.5 * (at.z + to.z));
        assert!(10.0 - mid.x.hypot(mid.z) <= 0.001 + 1e-12);
        at = to;
    }
    let length = posted.written.stats().unwrap().feed_length;
    assert!((length - 5.0 * PI).abs() < 0.01);
}

#[test]
fn unsafe_or_unwritable_toolpaths_are_refused() {
    let post = Post::builtin("fanuc_mill").unwrap();
    let spindle_on = Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![Move::Spindle { rpm: 1000.0, direction: SpindleDirection::Clockwise }, Move::ToolChange { tool: 2 }],
    };
    assert!(post.write(&spindle_on, &ctx()).unwrap_err().to_string().contains("stop the spindle"));
    let cut_first =
        Toolpath { start: p(0.0, 0.0, 50.0), moves: vec![Move::ToolChange { tool: 2 }, Move::Linear { to: p(1.0, 0.0, 0.0), feed: 10.0 }] };
    assert!(post.write(&cut_first, &ctx()).unwrap_err().to_string().contains("must be a rapid"));
    // The machine is wherever it was when a program starts: the first move must be a rapid too.
    let opens_with_a_cut = Toolpath { start: p(0.0, 0.0, 50.0), moves: vec![Move::Linear { to: p(1.0, 0.0, 0.0), feed: 10.0 }] };
    assert!(post.write(&opens_with_a_cut, &ctx()).unwrap_err().to_string().contains("first move of a program"));
    let dwell = Toolpath { start: p(0.0, 0.0, 0.0), moves: vec![Move::Dwell { seconds: 1.0 }] };
    assert!(post.write(&dwell, &ctx()).unwrap_err().to_string().contains("dwell"));
    assert!(Post::builtin("rs274").unwrap().write(&dwell, &ctx()).unwrap().text.contains("G4 P1.000\n"));
    assert!(post.write(&bracket(), &ProgramContext { program_number: 12345, ..ctx() }).is_err());
    assert!(post.write(&bracket(), &ProgramContext { work_offset: 7, ..ctx() }).is_err());
}

#[test]
fn bad_post_files_are_refused_when_read() {
    let good: serde_json::Value = serde_json::from_str(BUILTIN_POSTS.iter().find(|(id, _)| *id == "fanuc_mill").unwrap().1).unwrap();
    let with = |pointer: &str, value: serde_json::Value| {
        let mut v = good.clone();
        *v.pointer_mut(pointer).unwrap() = value;
        Post::from_json(&v.to_string())
    };
    use serde_json::json;
    assert!(with("/format", json!(2)).is_err());
    assert!(with("/id", json!("Bad Id")).is_err());
    assert!(with("/numbers/length_mm", json!(9)).is_err());
    assert!(with("/comments/max_length", json!(10)).is_err());
    assert!(with("/arcs/max_sweep_degrees", json!(0)).is_err());
    assert!(with("/arcs/linearize_tolerance", json!(f64::MAX)).is_err());
    assert!(with("/sequences/safe_start", json!(["G21 {nope}"])).unwrap_err().to_string().contains("unknown placeholder"));
    assert!(with("/sequences/safe_start", json!(["T{tool} M6"])).unwrap_err().to_string().contains("{tool} only"));
    assert!(with("/sequences/safe_start", json!(["G0 X1"])).unwrap_err().to_string().contains("makes moves"));
    assert!(with("/sequences/safe_start", json!(["G21 }"])).is_err());
    assert!(with("/sequences/program_end", json!(["#1=2"])).unwrap_err().to_string().contains("does not read"));
    assert!(with("/program_number_digits", json!(null)).unwrap_err().to_string().contains("program_number_digits"));
    let mut unknown = good.clone();
    unknown.as_object_mut().unwrap().insert("script".into(), json!("rm -rf"));
    assert!(Post::from_json(&unknown.to_string()).is_err());
    assert!(Post::from_json("not json").is_err());
}

#[test]
fn the_haas_post_writes_this_program_exactly() {
    // Derived by hand from haas_mill.json and research/gcode-haas.md: five-digit O number, no line
    // numbers, G28 G91 Z0. before the tool change and at the end, G90 back after each.
    let expected = "\
%
O00001 (BRACKET OP1)
(UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)
(PROVE OUT FIRST: AIR CUT, SINGLE BLOCK)
G17 G40 G49 G80 G90 G94 G21
G54
G28 G91 Z0.
G90
T1 M6
S1200 M3
M8
G43 H1 G0 X10. Y0. Z5.
G1 Z-1. F200.
G17
G3 X0. Y10. I-10. J0. F400.
G0 Z5.
M5
M9
G28 G91 Z0.
G90
M30
%
";
    let posted = Post::builtin("haas_mill").unwrap().write(&bracket(), &ctx()).unwrap();
    assert_eq!(posted.text, expected);
}

#[test]
fn haas_output_keeps_one_m_code_per_line_and_explicit_dwell_seconds() {
    let post = Post::builtin("haas_mill").unwrap();
    let mut tp = bracket();
    tp.moves.insert(5, Move::Dwell { seconds: 0.5 });
    tp.moves.insert(5, Move::Dwell { seconds: 2.0 });
    let posted = post.write(&tp, &ctx()).unwrap();
    // A Haas G04 P without a fraction is milliseconds: every dwell carries its decimals.
    assert!(posted.text.contains("\nG4 P2.000\n") && posted.text.contains("\nG4 P0.500\n"), "{}", posted.text);
    for line in posted.text.lines() {
        let m_words = line.split_whitespace().filter(|w| w.starts_with('M')).count();
        assert!(m_words <= 1, "two M codes on one line: {line}");
    }
    // Inch: 4 places, G20 to check Setting 9.
    let inch = post.write(&bracket(), &ProgramContext { units: LengthUnit::Inch, ..ctx() }).unwrap();
    assert!(inch.text.contains("G20") && inch.text.contains("X0.3937 Y0. Z0.1969"));
}

#[test]
fn a_post_that_writes_dwells_must_give_them_decimals() {
    let good: serde_json::Value = serde_json::from_str(BUILTIN_POSTS.iter().find(|(id, _)| *id == "haas_mill").unwrap().1).unwrap();
    let mut v = good.clone();
    *v.pointer_mut("/numbers/dwell").unwrap() = serde_json::json!(0);
    assert!(Post::from_json(&v.to_string()).unwrap_err().to_string().contains("milliseconds"));
}

/// Two peck holes that go back to R between them (G99), up over a clamp, then two drilled holes that
/// come back to the height they start at (G98).
fn holes() -> Toolpath {
    let peck = DrillCycle::Peck { peck: 4.0 };
    let drill = DrillCycle::Drill { dwell: 0.0 };
    Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![
            Move::ToolChange { tool: 3 },
            Move::Spindle { rpm: 1500.0, direction: SpindleDirection::Clockwise },
            Move::Rapid { to: p(10.0, 10.0, 25.0) },
            Move::Drill { to: p(10.0, 10.0, -12.0), r: 2.0, retract: 2.0, cycle: peck, feed: 200.0 },
            Move::Drill { to: p(20.0, 10.0, -12.0), r: 2.0, retract: 2.0, cycle: peck, feed: 200.0 },
            Move::Rapid { to: p(20.0, 10.0, 25.0) },
            Move::Drill { to: p(30.0, 10.0, -5.0), r: 2.0, retract: 25.0, cycle: drill, feed: 150.0 },
            Move::Drill { to: p(40.0, 10.0, -5.0), r: 2.0, retract: 25.0, cycle: drill, feed: 150.0 },
            Move::Rapid { to: p(40.0, 10.0, 50.0) },
            Move::SpindleStop,
        ],
    }
}

#[test]
fn the_haas_post_writes_holes_as_canned_cycles_exactly() {
    // Derived by hand (decision 0008): X and Y on every hole, G80 before the next other move, and
    // that move writes Z again (after a hole the control is at the retract height, not the Z word).
    let expected = "\
%
O00001 (BRACKET OP1)
(UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)
(PROVE OUT FIRST: AIR CUT, SINGLE BLOCK)
G17 G40 G49 G80 G90 G94 G21
G54
G28 G91 Z0.
G90
T3 M6
S1500 M3
G43 H3 G0 X10. Y10. Z25.
G99 G83 X10. Y10. Z-12. R2. Q4. F200.
X20. Y10.
G80
G0 Z25.
G98 G81 X30. Y10. Z-5. R2. F150.
X40. Y10.
G80
G0 Z50.
M5
G28 G91 Z0.
G90
M30
%
";
    let posted = Post::builtin("haas_mill").unwrap().write(&holes(), &ctx()).unwrap();
    assert_eq!(posted.text, expected);
    assert_eq!(posted.written, holes());
    assert_eq!(posted.read_back.toolpath, holes());
    assert!(posted.notes.is_empty(), "{:?}", posted.notes);
}

#[test]
fn every_built_in_post_writes_g81_and_g83_and_reads_them_back() {
    for id in ["rs274", "fanuc_mill", "haas_mill"] {
        let posted = Post::builtin(id).unwrap().write(&holes(), &ctx()).unwrap();
        assert!(posted.text.contains("G99 G83 X10. Y10. Z-12. R2. Q4. F200."), "{id}:\n{}", posted.text);
        assert!(posted.text.contains("G98 G81 X30. Y10. Z-5. R2. F150."), "{id}:\n{}", posted.text);
        assert_eq!(posted.read_back.toolpath, holes(), "{id}");
        // In inches too, within half the last digit.
        let inch = ProgramContext { units: LengthUnit::Inch, ..ctx() };
        let posted = Post::builtin(id).unwrap().write(&holes(), &inch).unwrap();
        assert!(posted.text.contains("G83"), "{id}");
        assert!(posted.read_back.toolpath.moves.len() == holes().moves.len(), "{id}");
    }
}

fn one_hole(cycle: DrillCycle, retract: f64) -> Toolpath {
    Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![
            Move::ToolChange { tool: 1 },
            Move::Rapid { to: p(0.0, 0.0, 10.0) },
            Move::Drill { to: p(0.0, 0.0, -5.0), r: 1.0, retract, cycle, feed: 100.0 },
        ],
    }
}

#[test]
fn a_hole_a_post_cannot_say_as_a_cycle_is_written_as_plain_moves() {
    let chip = DrillCycle::ChipBreak { peck: 3.0 };
    // G73 is not in RS274/NGC: 6 mm from R in two pecks, backing off the post's 0.5 mm between.
    let posted = Post::builtin("rs274").unwrap().write(&one_hole(chip, 10.0), &ctx()).unwrap();
    assert!(!posted.text.contains("G73"), "{}", posted.text);
    assert!(posted.notes.iter().any(|n| n.contains("plain moves") && n.contains("G73")), "{:?}", posted.notes);
    assert_eq!(
        posted.written.moves[2..],
        [
            Move::Rapid { to: p(0.0, 0.0, 1.0) },
            Move::Linear { to: p(0.0, 0.0, -2.0), feed: 100.0 },
            Move::Rapid { to: p(0.0, 0.0, -1.5) },
            Move::Linear { to: p(0.0, 0.0, -5.0), feed: 100.0 },
            Move::Rapid { to: p(0.0, 0.0, 10.0) },
        ]
    );
    // Haas has G73, and the hole returns to where it started: G98.
    let posted = Post::builtin("haas_mill").unwrap().write(&one_hole(chip, 10.0), &ctx()).unwrap();
    assert!(posted.text.contains("\nG98 G73 X0. Y0. Z-5. R1. Q3. F100.\nG80\n"), "{}", posted.text);
    // A dwell keeps all its decimals, as G4's P does.
    let dwell = DrillCycle::Drill { dwell: 0.5 };
    let posted = Post::builtin("haas_mill").unwrap().write(&one_hole(dwell, 1.0), &ctx()).unwrap();
    assert!(posted.text.contains("\nG99 G82 X0. Y0. Z-5. R1. P0.500 F100.\n"), "{}", posted.text);
    // The Fanuc-style post has no verified dwell yet, so a dwell hole is refused, not guessed at.
    let e = Post::builtin("fanuc_mill").unwrap().write(&one_hole(dwell, 1.0), &ctx()).unwrap_err();
    assert!(e.to_string().contains("dwell"), "{e}");
    // A hole that returns neither to R nor to where it started is no G98 or G99.
    let posted = Post::builtin("haas_mill").unwrap().write(&one_hole(DrillCycle::Drill { dwell: 0.0 }, 5.0), &ctx()).unwrap();
    assert!(!posted.text.contains("G81"), "{}", posted.text);
    assert!(posted.notes.iter().any(|n| n.contains("neither")), "{:?}", posted.notes);
    assert_eq!(posted.written.moves.last(), Some(&Move::Rapid { to: p(0.0, 0.0, 5.0) }));
}

#[test]
fn a_hole_is_never_the_first_move_after_a_tool_change() {
    let tp = Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![
            Move::ToolChange { tool: 1 },
            Move::Drill { to: p(0.0, 0.0, -5.0), r: 1.0, retract: 50.0, cycle: DrillCycle::Drill { dwell: 0.0 }, feed: 100.0 },
        ],
    };
    for (id, _) in BUILTIN_POSTS {
        let e = Post::builtin(id).unwrap().write(&tp, &ctx()).unwrap_err();
        assert!(e.to_string().contains("must be a rapid"), "{id}: {e}");
    }
}

#[test]
fn post_files_without_cycles_still_read_and_bad_cycle_data_is_refused() {
    let mut post: serde_json::Value = serde_json::from_str(include_str!("../posts/rs274.json")).unwrap();
    let cycles = post.as_object_mut().unwrap().remove("cycles").unwrap();
    // An older post file: no canned cycles, every hole as plain moves.
    let old = Post::from_json(&post.to_string()).unwrap();
    let posted = old.write(&holes(), &ctx()).unwrap();
    assert!(!["G81", "G83", "G98", "G99"].iter().any(|code| posted.text.contains(code)), "{}", posted.text);
    assert_eq!(posted.notes.iter().filter(|n| n.contains("plain moves")).count(), 4);
    // Repeating motion codes never repeats a cycle's code on its later holes (that line would start a new cycle).
    let mut repeat = post.clone();
    repeat["cycles"] = cycles.clone();
    repeat["modal"]["repeat_motion"] = serde_json::json!(true);
    let posted = Post::from_json(&repeat.to_string()).unwrap().write(&holes(), &ctx()).unwrap();
    assert!(posted.text.contains("\nX20. Y10.\n"), "{}", posted.text);
    let with = |path: &str, value: serde_json::Value| {
        let mut p = post.clone();
        p["cycles"] = cycles.clone();
        *p.pointer_mut(path).unwrap() = value;
        Post::from_json(&p.to_string())
    };
    assert!(with("/cycles/motion/peck_clearance", serde_json::json!(0)).unwrap_err().to_string().contains("cycles.motion"));
    assert!(with("/cycles/motion/above_r", serde_json::json!(-1)).unwrap_err().to_string().contains("cycles.motion"));
    assert!(with("/dwell", serde_json::json!(false)).unwrap_err().to_string().contains("cycles.dwell"));
    assert!(with("/cycles/g98", serde_json::json!("sometimes")).is_err());
    let mut unknown = post.clone();
    unknown["cycles"] = cycles;
    unknown["cycles"]["tapping"] = serde_json::json!(true);
    assert!(Post::from_json(&unknown.to_string()).is_err());
}
