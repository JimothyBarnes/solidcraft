use super::*;
use crate::units::MM_PER_INCH;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn moves(program: &str) -> Vec<Move> {
    read(program, &ReadSettings::default()).unwrap().toolpath.moves
}

fn moves_from(start: Vec3, program: &str) -> Vec<Move> {
    read(program, &ReadSettings { start, ..ReadSettings::default() }).unwrap().toolpath.moves
}

fn error(program: &str) -> String {
    match read(program, &ReadSettings::default()) {
        Ok(r) => panic!("{program:?} was read: {:?}", r.toolpath.moves),
        Err(e) => e.to_string(),
    }
}

#[test]
fn a_whole_program_reads_to_its_moves_in_order() {
    let program = "%\nO1234 (square corner)\nN10 G21 G90 G17\nN20 T1 M6\nS1000 M3 M8\nG0 X0 Y0 Z5\nG1 Z-1 F100\nX10\nG0 Z5\nM5 M9\nM30\n%\n";
    assert_eq!(
        moves(program),
        vec![
            Move::ToolChange { tool: 1 },
            Move::Spindle { rpm: 1000.0, direction: SpindleDirection::Clockwise },
            Move::Coolant { mode: Coolant::Flood },
            Move::Rapid { to: p(0.0, 0.0, 5.0) },
            Move::Linear { to: p(0.0, 0.0, -1.0), feed: 100.0 },
            Move::Linear { to: p(10.0, 0.0, -1.0), feed: 100.0 },
            Move::Rapid { to: p(10.0, 0.0, 5.0) },
            Move::SpindleStop,
            Move::Coolant { mode: Coolant::Off },
        ]
    );
}

#[test]
fn spaces_case_and_line_endings_do_not_change_a_line() {
    // NIST IR 6556 §3.3's own example: "g0x +0. 12 34y 7" means G0 X0.1234 Y7.
    assert_eq!(moves("g0x +0. 12 34y 7\r\n"), vec![Move::Rapid { to: p(0.1234, 7.0, 0.0) }]);
}

#[test]
fn the_nist_centre_format_example_has_its_centre_at_10_11() {
    // NIST IR 6556 §3.5.3.2: from (7, 7), "G17 G2 x 10 y 16 i 3 j 4 z 9" has centre (10, 11), radius 5.
    let m = moves_from(p(7.0, 7.0, 9.0), "F50\nG17 G2 x 10 y 16 i 3 j 4 z 9");
    assert_eq!(
        m,
        vec![Move::Arc { to: p(10.0, 16.0, 9.0), center: p(10.0, 11.0, 9.0), plane: Plane::Xy, direction: ArcDirection::Clockwise, feed: 50.0 }]
    );
}

#[test]
fn radius_format_picks_the_short_or_long_way_by_the_sign_of_r() {
    let centre = |program: &str| match moves_from(p(10.0, 0.0, 0.0), program).first() {
        Some(Move::Arc { center, .. }) => *center,
        other => panic!("{other:?}"),
    };
    let near = |a: Vec3, b: Vec3| (a.x - b.x).abs() < 1e-9 && (a.y - b.y).abs() < 1e-9;
    // (10, 0) to (0, 10), radius 10: a quarter about the origin, or three quarters about (10, 10).
    assert!(near(centre("F1 G3 X0 Y10 R10"), p(0.0, 0.0, 0.0)));
    assert!(near(centre("F1 G3 X0 Y10 R-10"), p(10.0, 10.0, 0.0)));
    assert!(near(centre("F1 G2 X0 Y10 R10"), p(10.0, 10.0, 0.0)));
    assert!(near(centre("F1 G2 X0 Y10 R-10"), p(0.0, 0.0, 0.0)));
    // The lengths confirm the sweep: 5π for the quarter, 15π for three quarters.
    let length = |program: &str| {
        read(program, &ReadSettings { start: p(10.0, 0.0, 0.0), ..Default::default() }).unwrap().toolpath.stats().unwrap().feed_length
    };
    assert!((length("F1 G3 X0 Y10 R10") - 5.0 * std::f64::consts::PI).abs() < 1e-9);
    assert!((length("F1 G3 X0 Y10 R-10") - 15.0 * std::f64::consts::PI).abs() < 1e-9);
}

#[test]
fn a_full_circle_and_a_side_plane_arc() {
    let full = moves_from(p(5.0, 0.0, 0.0), "F1 G2 X5 Y0 I-5 J0");
    assert!(matches!(full.first(), Some(Move::Arc { center, .. }) if *center == p(0.0, 0.0, 0.0)));
    // G18 (ZX): I is X's offset, K is Z's. From Z10 to X10 counter-clockwise about the origin.
    let side = moves_from(p(0.0, 0.0, 10.0), "F1 G18 G3 X10 Z0 I0 K-10");
    assert!(matches!(side.first(), Some(Move::Arc { center, plane: Plane::Zx, .. }) if *center == p(0.0, 0.0, 0.0)));
}

#[test]
fn inches_and_incremental_moves_come_back_in_millimetres() {
    // G20 on the same line applies before the move (Table 8 item 12 before item 20).
    assert_eq!(moves("G20 G1 X1 F10"), vec![Move::Linear { to: p(25.4, 0.0, 0.0), feed: 254.0 }]);
    assert_eq!(
        moves("G91 G0 X1 Y2\nX1\nG90 X0"),
        vec![Move::Rapid { to: p(1.0, 2.0, 0.0) }, Move::Rapid { to: p(2.0, 2.0, 0.0) }, Move::Rapid { to: p(0.0, 2.0, 0.0) }]
    );
    let r = read("G20\nG0 X1", &ReadSettings::default()).unwrap();
    assert_eq!(r.units, LengthUnit::Inch);
}

#[test]
fn items_on_one_line_run_in_the_standard_order() {
    // Tool change (items 5-6) before the spindle (7); the spindle stop (7) before the move (20).
    assert_eq!(moves("S500 M3 T2 M6"), vec![Move::ToolChange { tool: 2 }, Move::Spindle { rpm: 500.0, direction: SpindleDirection::Clockwise }]);
    assert_eq!(
        moves("S500 M3\nG1 X1 F100 M5"),
        vec![
            Move::Spindle { rpm: 500.0, direction: SpindleDirection::Clockwise },
            Move::SpindleStop,
            Move::Linear { to: p(1.0, 0.0, 0.0), feed: 100.0 },
        ]
    );
    // A new S while the spindle turns changes its speed.
    assert_eq!(moves("S500 M4\nS800")[1], Move::Spindle { rpm: 800.0, direction: SpindleDirection::CounterClockwise });
}

#[test]
fn block_delete_skips_a_line_only_when_switched_on() {
    let on = read("/G0 X5\nG0 Y1", &ReadSettings { block_delete: true, ..Default::default() }).unwrap();
    assert_eq!(on.toolpath.moves, vec![Move::Rapid { to: p(0.0, 1.0, 0.0) }]);
    assert_eq!(moves("/G0 X5").len(), 1);
}

#[test]
fn lines_after_the_program_end_are_noted_not_run() {
    let r = read("G0 X1\nM30\nG0 X9\n%", &ReadSettings::default()).unwrap();
    assert_eq!(r.toolpath.moves, vec![Move::Rapid { to: p(1.0, 0.0, 0.0) }]);
    assert_eq!(r.notes.len(), 1);
}

#[test]
fn hostile_programs_are_refused_with_their_line_and_reason() {
    let long = format!("G0 X{}", "1".repeat(300));
    let cases: &[(&str, &str)] = &[
        ("G1 X1", "before any feed"),
        ("#1=2", "parameters"),
        ("G0 X[1+2]", "expressions"),
        ("G0 X#1", "parameters"),
        ("F1\nG41 X1 D1", "line 2: G41"),
        ("G85 X1 Z-1 R1", "boring cycles"),
        ("F1\nG0 Z5\nG99 G84 X1 Y1 Z-1 R1", "G84 with the spindle stopped"),
        ("F1 S100 M4\nG0 Z5\nG99 G84 X1 Y1 Z-1 R1", "G84 with the spindle turning counter-clockwise"),
        ("S100 M3\nG0 Z5\nG99 G84 X1 Y1 Z-1 R1", "before any feed"),
        ("F1 S100 M3\nG0 Z5\nG99 G84 X1 Y1 Z-1 R1 Q0.5", "G84 takes no Q"),
        ("G28 Z0", "G91 G28 Z0"),
        ("G91 G28 X0 Z0", "G91 G28 Z0"),
        ("G91 G28 Z1", "G91 G28 Z0"),
        ("G91 G0 G28 Z0", "G28 and a motion code"),
        ("G91 G28 Y0", "G91 G28 Z0"),
        ("G91 G28 Z0\nG28 X0", "G91 G28 Z0"),
        ("G91 G28 Z0\nG28 Y5", "G91 G28 Z0"),
        ("G91 G28 Z0\nG28 Y0\nG90\nG0 X1 Y1 Z1", "the table was sent home"),
        ("F1\nG91 G28 Z0\nG28 Y0\nG90\nG81 X1 Y1 Z-1 R1", "the table was sent home"),
        ("G53 G1 Z0 F1", "G53 G0 Z"),
        ("G91 G53 G0 Z0", "G53 G0 Z"),
        ("G30 Z0", "machine coordinates"),
        ("F1\nG53 G0 Z0\nG1 X1 Y1", "Z is unknown"),
        ("G91 G28 Z0\nG0 X1 Y1", "Z is unknown"),
        ("F1\nG53 G0 Z0\nG0 X1 Y1\nG1 X2", "Z is unknown"),
        ("G91 G28 Z0\nG0 Z-5", "Z is unknown"),
        ("F1\nG53 G0 Z0\nG2 X1 Y0 Z0 I0.5", "right after a machine retract"),
        ("G93 G1 X1 F1", "inverse-time"),
        ("G0 G1 X1", "modal group 1"),
        ("G20 G21", "modal group 6"),
        ("X1 X2", "two X words"),
        ("S1 M3 M4", "modal group 7"),
        ("M3 M8 M48 M1 M7", "modal group"),
        ("G0 X", "needs a number"),
        ("G0 X1.2.3", "two decimal points"),
        ("G0 X1 (unclosed", "not closed"),
        ("G0 (a ( b) X1", "may not contain"),
        ("N123456 G0 X1", "line number"),
        ("G0 N1 X1", "must come first"),
        ("E5", "does not start a word"),
        ("G0 A10", "rotary"),
        ("F1 G2 X1 Y1", "needs I and J"),
        ("F1 G2 X1.1 Y0 I0.5 J0", "at most 0.002"),
        ("F1 G2 X1 Y0 I0.5 R1", "not both"),
        ("F1 G2 X0 Y0 R1", "end away"),
        ("F1 G2 X10 Y0 R1", "too small"),
        ("G1 X1e5 F1", "does not start a word"),
        ("S-1 M3", "negative"),
        ("M3", "no speed"),
        ("M6", "no tool selected"),
        ("G4", "needs P"),
        ("G4 P-1", "negative"),
        ("G0 X1000000", "beyond"),
        ("G999", "not a known G code"),
        ("G1.5", "not a known G code"),
        ("M99", "not a known M code"),
        ("G80\nX1", "no motion mode"),
        ("G1", "at least one axis"),
        ("G0 X1 é", "unexpected character"),
        ("G0 X1;", "unexpected character"),
        ("F0", "above 0"),
        ("T-3", "not a tool number"),
        ("G43 Z1", "needs H"),
        (&long, "characters long"),
    ];
    for (program, expect) in cases {
        let e = error(program);
        assert!(e.contains(expect), "{program:?}: {e:?} does not mention {expect:?}");
    }
}

#[test]
fn random_bytes_never_panic() {
    // A fixed linear congruential generator over a G-code-flavoured alphabet: deterministic, no
    // dependency, and dense in the characters the parser treats specially.
    let alphabet: Vec<char> = "GMXYZIJKRFSTNOPQL0123456789.-+ ()#[]/%;\n\r\téA".chars().collect();
    let mut state: u64 = 0x2545_f491_4f6c_dd1d;
    for _ in 0..3000 {
        let mut program = String::new();
        for _ in 0..(state % 120) {
            state = state.wrapping_mul(6_364_136_223_846_793_005).wrapping_add(1_442_695_040_888_963_407);
            let i = usize::try_from(state >> 33).unwrap_or(0) % alphabet.len();
            program.push(alphabet[i]);
        }
        let _ = read(&program, &ReadSettings::default());
    }
}

#[test]
fn machine_z_retracts_are_counted_not_moved_and_need_a_full_next_move() {
    // The Fanuc and Haas forms of "Z up to machine home" before a tool change.
    let r = read("G0 X1 Y2 Z3\nG91 G28 Z0\nG90\nT1 M6\nG53 G0 Z0\nG0 G43 H1 X4 Y5 Z50", &ReadSettings::default()).unwrap();
    assert_eq!(r.machine_retracts, 2);
    assert_eq!(r.toolpath.moves, vec![Move::Rapid { to: p(1.0, 2.0, 3.0) }, Move::ToolChange { tool: 1 }, Move::Rapid { to: p(4.0, 5.0, 50.0) }]);
    // From machine home the tool may go across first, in X and Y alone, and then come down: the
    // one move, on two lines (or more: it comes down where the last of them left it).
    let two = read("G0 X1 Y2 Z3\nG91 G28 Z0\nG90\nT1 M6\nG0 X4 Y5\nG43 H1 Z50", &ReadSettings::default()).unwrap();
    assert_eq!(two.toolpath.moves, r.toolpath.moves);
    let more = read("G0 X1 Y2 Z3\nG91 G28 Z0\nG90\nT1 M6\nG0 X9\nX4 Y5\nG43 H1 Z50", &ReadSettings::default()).unwrap();
    assert_eq!(more.toolpath.moves, r.toolpath.moves);
    // A program may end with the table sent home after Z: no move, and not another Z retract.
    let ended = read("G0 X1 Y2 Z3\nM5\nG91 G28 Z0\nG28 Y0\nG90\nM30", &ReadSettings::default()).unwrap();
    assert_eq!((ended.toolpath.moves.len(), ended.machine_retracts, ended.table_home, r.table_home), (2, 1, true, false));
}

fn drill(x: f64, y: f64, bottom: f64, r: f64, retract: f64, cycle: DrillCycle, feed: f64) -> Move {
    Move::Drill { to: p(x, y, bottom), r, retract, cycle, feed }
}

fn read_with(g98: G98Rule, program: &str) -> std::result::Result<Vec<Move>, String> {
    read(program, &ReadSettings { g98, ..ReadSettings::default() }).map(|r| r.toolpath.moves).map_err(|e| e.to_string())
}

#[test]
fn nists_g81_example_reads_as_one_hole() {
    // NIST IR 6556 §3.5.16.2, example 1, from (1, 2, 3): G98 returns to the old Z, 3.
    let moves = moves_from(p(1.0, 2.0, 3.0), "F100\nG90 G81 G98 X4 Y5 Z1.5 R2.8");
    assert_eq!(moves, vec![drill(4.0, 5.0, 1.5, 2.8, 3.0, DrillCycle::Drill { dwell: 0.0 }, 100.0)]);
}

#[test]
fn g98_returns_where_the_controls_rule_says() {
    // G99 holes, then G98 after them, then G99 with a new depth and R plane on a hole's own line.
    let program = "G21 G90 G17 F200\nG0 X0 Y0 Z25\nG99 G83 X10 Y10 Z-12 R2 Q4\nX20\nG98 X30\nG99 X40 Z-6 R-1\nG80 G0 Z25";
    let peck = DrillCycle::Peck { peck: 4.0 };
    let first = [rapid_to(0.0, 0.0, 25.0), drill(10.0, 10.0, -12.0, 2.0, 2.0, peck, 200.0), drill(20.0, 10.0, -12.0, 2.0, 2.0, peck, 200.0)];
    // Haas: G98 goes back up to the 25 the cycle started at.
    let haas = read_with(G98Rule::CycleStart, program).unwrap();
    let expect: Vec<Move> = first
        .iter()
        .copied()
        .chain([drill(30.0, 10.0, -12.0, 2.0, 25.0, peck, 200.0), drill(40.0, 10.0, -6.0, -1.0, -1.0, peck, 200.0), rapid_to(40.0, 10.0, 25.0)])
        .collect();
    assert_eq!(haas, expect);
    // NIST: G98 goes back to where its own line started, the R plane of the hole before.
    let nist = read_with(G98Rule::BlockStart, program).unwrap();
    let expect: Vec<Move> = first
        .iter()
        .copied()
        .chain([drill(30.0, 10.0, -12.0, 2.0, 2.0, peck, 200.0), drill(40.0, 10.0, -6.0, -1.0, -1.0, peck, 200.0), rapid_to(40.0, 10.0, 25.0)])
        .collect();
    assert_eq!(nist, expect);
}

fn rapid_to(x: f64, y: f64, z: f64) -> Move {
    Move::Rapid { to: p(x, y, z) }
}

#[test]
fn every_drilling_cycle_reads_with_its_own_words() {
    let holes = moves("F50\nG0 Z5\nG99 G82 X1 Y1 Z-2 R1 P0.25\nG73 X2 Y2 Z-3 R1 Q0.5\nG81 X3 Y3 Z-1 R1\nG80");
    assert_eq!(
        holes[1..],
        [
            drill(1.0, 1.0, -2.0, 1.0, 1.0, DrillCycle::Drill { dwell: 0.25 }, 50.0),
            drill(2.0, 2.0, -3.0, 1.0, 1.0, DrillCycle::ChipBreak { peck: 0.5 }, 50.0),
            drill(3.0, 3.0, -1.0, 1.0, 1.0, DrillCycle::Drill { dwell: 0.0 }, 50.0),
        ]
    );
    // In inches every length is converted, the dwell (seconds) is not.
    let inch = moves("G20 F10\nG0 X0 Y0 Z1\nG98 G82 X1 Y1 Z-0.5 R0.1 P1.5");
    let mm = |v: f64| v * MM_PER_INCH;
    assert_eq!(inch[1], drill(mm(1.0), mm(1.0), mm(-0.5), mm(0.1), mm(1.0), DrillCycle::Drill { dwell: 1.5 }, mm(10.0)));
}

#[test]
fn g80_and_a_move_on_one_line_end_the_cycle_then_move() {
    // Haas writes `G80 G00 Z1.`; NIST would call two group 1 codes an error, but the meaning is plain.
    for line in ["G80 G0 X5", "G0 G80 X5"] {
        let read = moves(&format!("F10\nG0 Z5\nG98 G81 X1 Y1 Z-1 R1\n{line}"));
        assert_eq!(read.last(), Some(&rapid_to(5.0, 1.0, 5.0)), "{line}");
    }
    assert!(error("G80 G1 G0 X1").contains("modal group 1"));
    assert!(error("G81 G0 X1").contains("modal group 1"));
    // Any other move ends the cycle too: a later X is a rapid, not another hole.
    let read = moves("F10\nG0 Z5\nG98 G81 X1 Y1 Z-1 R1\nG0 X2\nX3");
    assert_eq!(read.last(), Some(&rapid_to(3.0, 1.0, 5.0)));
}

#[test]
fn drilling_cycles_controls_disagree_on_are_refused() {
    let cases: &[(&str, &str)] = &[
        ("G81 X1 Y1 Z-1 R1", "before G98 or G99"),
        ("G98 G81 Z-1 R1", "without X or Y"),
        ("G98 G81 X1 Y1 Z-1 R1\nZ-2", "without X or Y"),
        ("G98 G81 X1 Y1 Z-1 R1\nR3", "without X or Y"),
        ("G98 G81 X1 Y1 Z-1 R20", "below the R plane"),
        ("G98 G81 X1 Y1 Z-1", "needs R"),
        ("G98 G81 X1 Y1 R1", "needs Z"),
        ("G98 G81 X1 Y1 Z2 R1", "R above Z"),
        ("G98 G81 X1 Y1 Z1 R1", "R above Z"),
        ("G98 G81 X1 Y1 Z-2000000 R1", "beyond"),
        ("G91 G98 G81 X1 Y1 Z-1 R1", "incremental"),
        ("G98 G81 X1 Y1 Z-1 R1 L2", "repeats (L)"),
        ("G18 G98 G81 X1 Y1 Z-1 R1", "XY plane"),
        ("G98 G81 X1 Y1 Z-1 R1 P1", "P (a dwell) on G81"),
        ("G98 G81 X1 Y1 Z-1 R1 Q1", "G81 takes no Q"),
        ("G98 G82 X1 Y1 Z-1 R1", "G82 needs P"),
        ("G98 G82 X1 Y1 Z-1 R1 Q1 P1", "G82 takes no Q"),
        ("G98 G82 X1 Y1 Z-1 R1 P-1", "negative"),
        ("G98 G83 X1 Y1 Z-1 R1", "G83 needs Q"),
        ("G98 G83 X1 Y1 Z-1 R1 Q0", "above 0"),
        ("G98 G73 X1 Y1 Z-1 R1 Q-1", "above 0"),
        ("G98 G83 X1 Y1 Z-1 R1 Q1 P1", "P (a dwell) on G83"),
        ("G98 G81 X1 Y1 Z-1 R1\nX2 P1", "P (a dwell) on G81"),
        ("G98 G81 X1 Y1 Z-1 R1 G4 P1", "G4 and a drilling cycle"),
        ("G91 G28 Z0\nG90\nG98 G81 X1 Y1 Z-1 R1", "Z is unknown"),
    ];
    for (body, expect) in cases {
        let program = format!("F100\nG0 Z10\n{body}");
        let e = error(&program);
        assert!(e.contains(expect), "{body:?}: {e:?} does not mention {expect:?}");
    }
    assert!(error("G0 Z10\nG98 G81 X1 Y1 Z-1 R1").contains("before any feed"));
    // Starting another cycle without G80 is fine by NIST's G98, not by Haas's (which counts from the first).
    let again = "F100\nG0 Z10\nG98 G81 X1 Y1 Z-1 R1\nG83 X2 Y2 Z-1 R1 Q1";
    assert_eq!(read_with(G98Rule::BlockStart, again).map(|m| m.len()), Ok(3));
    assert!(read_with(G98Rule::CycleStart, again).unwrap_err().contains("G80 first"));
}

#[test]
fn g84_reads_as_a_tapped_hole_whose_pitch_is_its_feed_over_the_speed() {
    // 500 mm/min at 400 rev/min is 1.25 mm a turn; the next line taps another hole the same.
    let read = moves("S400 M3\nG0 X0 Y0 Z5\nG99 G84 X10 Y0 Z-10 R2 F500\nX20\nG80\nM5");
    let tapped = |x: f64| drill(x, 0.0, -10.0, 2.0, 2.0, DrillCycle::Tap { pitch: 1.25 }, 500.0);
    assert_eq!(read[2..], [tapped(10.0), tapped(20.0), Move::SpindleStop]);
    // In inches: 14 threads an inch at 450 rev/min, as Haas's workbook writes it.
    let inch = moves("G20 S450 M3\nG0 X0 Y0 Z1\nG99 G84 X0.75 Y0.75 Z-0.65 R0.1 F32.1429");
    let Move::Drill { cycle: DrillCycle::Tap { pitch }, feed, .. } = inch[2] else { panic!("{:?}", inch[2]) };
    assert!((pitch - MM_PER_INCH / 14.0).abs() < 1e-5 && (feed - 32.1429 * MM_PER_INCH).abs() < 1e-9, "{pitch} {feed}");
}
