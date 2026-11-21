use super::*;

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
        ("G81 X1 Z-1 R1", "canned cycles"),
        ("G28 Z0", "G91 G28 Z0"),
        ("G91 G28 X0 Z0", "G91 G28 Z0"),
        ("G91 G28 Z1", "G91 G28 Z0"),
        ("G91 G0 G28 Z0", "G28 and a motion code"),
        ("G53 G1 Z0 F1", "G53 G0 Z"),
        ("G91 G53 G0 Z0", "G53 G0 Z"),
        ("G30 Z0", "machine coordinates"),
        ("G53 G0 Z0\nG0 X1 Y1", "Z is unknown"),
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
    let alphabet: Vec<char> = "GMXYZIJKRFSTNOP0123456789.-+ ()#[]/%;\n\r\téA".chars().collect();
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
}
