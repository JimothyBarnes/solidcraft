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
    // first, a machine-home retract before the tool change, the first move across in X and Y
    // and then down in Z with the length offset, the plane before the first arc, modal words
    // left out when unchanged.
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
N100 G0 X10. Y0.
N110 G43 H1 Z5.
N120 G1 Z-1. F200.
N130 G17
N140 G3 X0. Y10. I-10. J0. F400.
N150 G0 Z5.
N160 M5
N170 M9
N180 G91 G28 Z0.
N190 G90
N200 M30
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
    // 10 mm and 5 mm in inches to 4 places: across, then down with the length offset.
    assert!(posted.text.contains(" G0 X0.3937 Y0.\n") && posted.text.contains(" G43 H1 Z0.1969\n"), "{}", posted.text);
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
    // numbers, G28 G91 Z0. before the tool change and at the end, G90 back after each, and from
    // machine home the tool across to X and Y before it comes down with G43 (the workbook's own
    // tool change, p. 23).
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
G0 X10. Y0.
G43 H1 Z5.
G1 Z-1. F200.
G17
G3 X0. Y10. I-10. J0. F400.
G0 Z5.
M5
M9
G28 G91 Z0.
G28 Y0.
G90
M30
%
";
    let posted = Post::builtin("haas_mill").unwrap().write(&bracket(), &ctx()).unwrap();
    assert_eq!(posted.text, expected);
    // The last retract takes Z home and then the table: read back as that, and as no move.
    assert!(posted.read_back.table_home && posted.read_back.machine_retracts == 2);
    assert!(!Post::builtin("fanuc_mill").unwrap().write(&bracket(), &ctx()).unwrap().read_back.table_home);
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
    assert!(inch.text.contains("G20") && inch.text.contains("\nG0 X0.3937 Y0.\nG43 H1 Z0.1969\n"), "{}", inch.text);
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
G0 X10. Y10.
G43 H3 Z25.
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
G28 Y0.
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

/// One hole tapped M6 x 1 at 500 rev/min (so 500 mm/min), 12 deep from R 2, back to R.
fn tapped(rpm: f64) -> Toolpath {
    Toolpath {
        start: p(0.0, 0.0, 50.0),
        moves: vec![
            Move::ToolChange { tool: 5 },
            Move::Spindle { rpm, direction: SpindleDirection::Clockwise },
            Move::Rapid { to: p(10.0, 10.0, 25.0) },
            Move::Rapid { to: p(10.0, 10.0, 2.0) },
            Move::Drill { to: p(10.0, 10.0, -10.0), r: 2.0, retract: 2.0, cycle: DrillCycle::Tap { pitch: 1.0 }, feed: rpm },
            Move::Rapid { to: p(10.0, 10.0, 50.0) },
            Move::SpindleStop,
        ],
    }
}

#[test]
fn a_tapped_hole_is_g84_where_the_post_has_it_and_refused_where_it_has_not() {
    for id in ["haas_mill", "rs274"] {
        let posted = Post::builtin(id).unwrap().write(&tapped(500.0), &ctx()).unwrap();
        assert!(posted.text.contains("S500 M3"), "{id}:\n{}", posted.text);
        assert!(posted.text.contains("G99 G84 X10. Y10. Z-10. R2. F500."), "{id}:\n{}", posted.text);
        assert_eq!(posted.read_back.toolpath, tapped(500.0), "{id}");
        assert!(posted.notes.is_empty(), "{:?}", posted.notes);
        // In inches the feed has four decimals, so that with the speed it is still the pitch:
        // 500 / 25.4 = 19.6850 inches a minute.
        let inch = ProgramContext { units: LengthUnit::Inch, ..ctx() };
        let posted = Post::builtin(id).unwrap().write(&tapped(500.0), &inch).unwrap();
        assert!(
            posted.text.lines().any(|line| line.starts_with("G99 G84 X0.3937 Y0.3937 Z-0.3937 R0.0787") && line.ends_with(" F19.685")),
            "{id}:\n{}",
            posted.text
        );
        let Move::Drill { cycle: DrillCycle::Tap { pitch }, .. } = posted.read_back.toolpath.moves[4] else { panic!("{id}") };
        assert!((pitch - 1.0).abs() < 1e-5, "{id}: {pitch}");
    }
    // 0.7 mm a turn at 357 rev/min is 249.9 mm/min: three decimals in millimetres.
    let fine = Toolpath {
        moves: tapped(357.0)
            .moves
            .into_iter()
            .map(|m| match m {
                Move::Drill { to, r, retract, .. } => Move::Drill { to, r, retract, cycle: DrillCycle::Tap { pitch: 0.7 }, feed: 0.7 * 357.0 },
                other => other,
            })
            .collect(),
        ..tapped(357.0)
    };
    let posted = Post::builtin("haas_mill").unwrap().write(&fine, &ctx()).unwrap();
    assert!(posted.text.contains("G99 G84 X10. Y10. Z-10. R2. F249.9\n"), "{}", posted.text);
    // The Fanuc-style post has no tapping set up: refused, never written as plain moves.
    let e = Post::builtin("fanuc_mill").unwrap().write(&tapped(500.0), &ctx()).unwrap_err().to_string();
    assert!(e.contains("a tapped hole can't be written as plain moves") && e.contains("fanuc_mill post does not write G84"), "{e}");
    // A speed the program can't say exactly (S has no decimals) would make the feed and the
    // speed as written another pitch: refused.
    let e = Post::builtin("haas_mill").unwrap().write(&tapped(500.4), &ctx()).unwrap_err().to_string();
    assert!(e.contains("give the tap a spindle speed the post writes exactly"), "{e}");
    // A hole that returns to neither R nor where it started can't be a cycle, and so can't be tapped.
    let mut odd = tapped(500.0);
    if let Some(Move::Drill { retract, .. }) = odd.moves.get_mut(4) {
        *retract = 10.0;
    }
    assert!(Post::builtin("haas_mill").unwrap().write(&odd, &ctx()).unwrap_err().to_string().contains("neither its R plane nor where it started"));
    // A post file from before tapping has none.
    let old = include_str!("../posts/haas_mill.json").replace(r#""tap": true, "#, "");
    assert!(!old.contains("\"tap\"") && !Post::from_json(&old).unwrap().cycles.tap);
}

#[test]
fn from_machine_home_the_tool_goes_across_before_it_comes_down() {
    // A tool change takes the tool to machine home, far above the work. From there the first
    // move is written on two lines: X and Y, then Z with the length offset. It is still the one
    // move, and reads back as one.
    for (id, across, down) in [("haas_mill", "\nG0 X10. Y0.\n", "\nG43 H1 Z5.\n"), ("fanuc_mill", " G0 X10. Y0.\n", " G43 H1 Z5.\n")] {
        let posted = Post::builtin(id).unwrap().write(&bracket(), &ctx()).unwrap();
        let (a, b) = (posted.text.find(across).unwrap(), posted.text.find(down).unwrap());
        assert!(a < b && !posted.text.contains("X10. Y0. Z5."), "{id}: {}", posted.text);
        assert_eq!(posted.written.moves, bracket().moves, "{id}");
        assert_eq!(posted.read_back.toolpath.moves.iter().filter(|m| matches!(m, Move::Rapid { .. })).count(), 2, "{id}");
    }
    // Where the tool change does not go to machine home, the move is one line, as the reference
    // post writes it; and a post may not ask for two lines without going home.
    let posted = Post::builtin("rs274").unwrap().write(&bracket(), &ctx()).unwrap();
    assert!(posted.text.contains("\nG43 H1 G0 X10. Y0. Z5.\n"), "{}", posted.text);
    let mut file: serde_json::Value = serde_json::from_str(BUILTIN_POSTS.iter().find(|(id, _)| *id == "rs274").unwrap().1).unwrap();
    file["xy_before_z"] = serde_json::json!(true);
    assert!(Post::from_json(&file.to_string()).unwrap_err().to_string().contains("xy_before_z needs tool_change_before"));
    // With no tool change before it, a program's first move is one line on every post: nothing
    // says the tool is at machine home.
    let plain = Toolpath { start: p(0.0, 0.0, 50.0), moves: vec![Move::Rapid { to: p(10.0, 0.0, 5.0) }, Move::Rapid { to: p(10.0, 0.0, 50.0) }] };
    let posted = Post::builtin("haas_mill").unwrap().write(&plain, &ctx()).unwrap();
    assert!(posted.text.contains("\nG0 X10. Y0. Z5.\n"), "{}", posted.text);
}

#[test]
fn an_inch_program_keeps_its_small_arcs_as_arcs() {
    // Four hundred quarter turns of 0.5 to 2 mm radius about centres off any grid. Rounded to
    // four decimals of an inch (0.00254 mm a step), the start's and the end's distances from the
    // centre come out more than 0.002 mm apart for 161 of them, which were written as lines
    // while every program was held to the millimetre limit. None comes out more than 0.0002
    // inch (0.00508 mm, two steps) apart: an inch program is held to the inch limit (NIST IR
    // 6556 section 3.5.3.2), so every one is written as the arc it is. A millimetre program, on
    // its finer grid, keeps them all too.
    let mut moves = vec![
        Move::ToolChange { tool: 1 },
        Move::Spindle { rpm: 1200.0, direction: SpindleDirection::Clockwise },
        Move::Rapid { to: p(0.0, 0.0, 5.0) },
        Move::Linear { to: p(0.0, 0.0, -1.0), feed: 200.0 },
    ];
    for k in 0..400 {
        let (radius, i) = ([0.5, 1.0, 1.5, 2.0][k % 4], k as f64);
        let (cx, cy) = (0.37 * i + 0.0123, 53.0 * (0.61 * i).sin() + 0.0456);
        moves.push(Move::Linear { to: p(cx + radius, cy, -1.0), feed: 400.0 });
        moves.push(Move::Arc {
            to: p(cx, cy + radius, -1.0),
            center: p(cx, cy, -1.0),
            plane: Plane::Xy,
            direction: ArcDirection::CounterClockwise,
            feed: 400.0,
        });
    }
    moves.extend([Move::Rapid { to: p(0.0, 0.0, 5.0) }, Move::SpindleStop]);
    let tp = Toolpath { start: p(0.0, 0.0, 50.0), moves };
    let as_lines = |units: LengthUnit| {
        let posted = Post::builtin("haas_mill").unwrap().write(&tp, &ProgramContext { units, ..ctx() }).unwrap();
        // What was written as an arc reads back within the limit of the program's units.
        let mut at = posted.read_back.toolpath.start;
        for m in &posted.read_back.toolpath.moves {
            if let Move::Arc { to, center, .. } = m {
                let apart = ((at.x - center.x).hypot(at.y - center.y) - (to.x - center.x).hypot(to.y - center.y)).abs();
                assert!(apart <= crate::toolpath::arc_radius_tolerance(units) + 1e-9, "{apart} mm apart in {units:?}");
            }
            at = m.end(at);
        }
        posted.notes.iter().filter(|note| note.contains("arc written as")).count()
    };
    assert_eq!((as_lines(LengthUnit::Millimetre), as_lines(LengthUnit::Inch)), (0, 0), "arcs written as lines in millimetres and in inches");
}

#[test]
fn two_operations_with_one_tool_post_with_one_tool_change() {
    let operation = |x: f64| {
        vec![
            Move::ToolChange { tool: 2 },
            Move::Spindle { rpm: 6000.0, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(x, 0.0, 25.0) },
            Move::Rapid { to: p(x, 0.0, 5.0) },
            Move::Linear { to: p(x, 0.0, -1.0), feed: 300.0 },
            Move::Linear { to: p(x + 10.0, 0.0, -1.0), feed: 600.0 },
            Move::Rapid { to: p(x + 10.0, 0.0, 25.0) },
            Move::Coolant { mode: Coolant::Off },
            Move::SpindleStop,
        ]
    };
    let both = Toolpath { start: p(0.0, 0.0, 50.0), moves: [operation(10.0), operation(40.0)].concat() };
    let post = Post::builtin("haas_mill").unwrap();
    let as_written = post.write(&both, &ctx()).unwrap();
    let kept = post.write(&both.without_needless_changes(), &ctx()).unwrap();
    let count = |text: &str, word: &str| text.lines().filter(|line| line.split_whitespace().any(|w| w == word)).count();
    // As the operations were written, the tool is changed for itself, from machine home, with
    // the spindle and the coolant stopped and started again; kept, once of each.
    assert_eq!((count(&as_written.text, "M6"), count(&as_written.text, "M5"), count(&as_written.text, "M9")), (2, 2, 2));
    assert_eq!(
        (count(&kept.text, "M6"), count(&kept.text, "M3"), count(&kept.text, "M8"), count(&kept.text, "M5"), count(&kept.text, "M9")),
        (1, 1, 1, 1, 1)
    );
    // Between the two the tool goes across at the clearance height and down again, turning.
    assert!(kept.text.contains("X20. F600.\nG0 Z25.\nX40.\nZ5.\nG1 Z-1. F300.\nX50. F600.\nG0 Z25.\nM9\nM5\n"), "{}", kept.text);
    assert!(kept.text.lines().count() < as_written.text.lines().count());
    // Both read back as the moves they were given.
    assert_eq!(kept.read_back.toolpath.moves.iter().filter(|m| matches!(m, Move::ToolChange { .. })).count(), 1);
}

#[test]
fn a_reamed_hole_is_g85_or_g89_where_the_post_has_them_and_plain_moves_where_not() {
    let ream = DrillCycle::Ream { dwell: 0.0 };
    for id in ["haas_mill", "rs274"] {
        let posted = Post::builtin(id).unwrap().write(&one_hole(ream, 1.0), &ctx()).unwrap();
        assert!(posted.text.contains("\nG99 G85 X0. Y0. Z-5. R1. F100.\nG80\n"), "{id}:\n{}", posted.text);
        assert_eq!(posted.written.moves.last(), Some(&Move::Drill { to: p(0.0, 0.0, -5.0), r: 1.0, retract: 1.0, cycle: ream, feed: 100.0 }));
        // With a dwell at the bottom it is G89.
        let posted = Post::builtin(id).unwrap().write(&one_hole(DrillCycle::Ream { dwell: 0.5 }, 1.0), &ctx()).unwrap();
        assert!(posted.text.contains("\nG99 G89 X0. Y0. Z-5. R1. P0.5"), "{id}:\n{}", posted.text);
        assert!(posted.text.lines().any(|line| line.starts_with("G99 G89") && line.ends_with(" F100.")), "{id}:\n{}", posted.text);
    }
    // The dwell keeps all its decimals on the Haas, as G82's does.
    let posted = Post::builtin("haas_mill").unwrap().write(&one_hole(DrillCycle::Ream { dwell: 0.5 }, 1.0), &ctx()).unwrap();
    assert!(posted.text.contains("\nG99 G89 X0. Y0. Z-5. R1. P0.500 F100.\n"), "{}", posted.text);
    // The Fanuc-style post has no cycle for it read from a public source yet: it writes the
    // moves themselves, fed in and fed out, which say the same.
    let posted = Post::builtin("fanuc_mill").unwrap().write(&one_hole(ream, 10.0), &ctx()).unwrap();
    assert!(
        !posted.text.contains("G85") && posted.notes.iter().any(|n| n.contains("plain moves") && n.contains("G85")),
        "{:?}\n{}",
        posted.notes,
        posted.text
    );
    assert_eq!(
        posted.written.moves[2..],
        [
            Move::Rapid { to: p(0.0, 0.0, 1.0) },
            Move::Linear { to: p(0.0, 0.0, -5.0), feed: 100.0 },
            Move::Linear { to: p(0.0, 0.0, 1.0), feed: 100.0 },
            Move::Rapid { to: p(0.0, 0.0, 10.0) },
        ]
    );
    // With a dwell it is refused there, as a drilled hole's dwell is: that post's dwell is not verified.
    let e = Post::builtin("fanuc_mill").unwrap().write(&one_hole(DrillCycle::Ream { dwell: 0.5 }, 1.0), &ctx()).unwrap_err();
    assert!(e.to_string().contains("dwell"), "{e}");
}
