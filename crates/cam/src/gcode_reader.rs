//! Reads G-code back into a [`Toolpath`], so every program a post writes can be checked against
//! the toolpath it came from (SPEC §6, "read-back").
//!
//! The language is the RS274/NGC subset our posts emit, read by the rules of NIST IR 6556 §3
//! (`research/gcode-rs274.md`): case-insensitive words, spaces anywhere outside comments,
//! `( … )` comments, block delete, line numbers, the modal groups and the order of execution of
//! Table 8. Fanuc-style `%` tape marks and `O` program numbers are accepted and ignored.
//!
//! Drilling cycles G81, G82, G83 and G73 read into one [`Move::Drill`] per hole (§3.5.16,
//! decision 0008), in absolute mode in the XY plane, with G98's height by
//! [`ReadSettings::g98`]. What controls run differently is refused by name: a cycle before G98
//! or G99, a cycle line without X or Y, a hole started below its R plane, repeats (L). G84 reads
//! into a tapped hole whose pitch is its feed over the spindle speed in force (decision 0017),
//! with the spindle turning clockwise as NIST asks.
//!
//! Anything outside the subset (parameters, expressions, left-hand tapping, boring cycles, cutter
//! compensation, inverse-time feed, rotary axes, machine-coordinate moves) is refused with the
//! line and the reason, never guessed at. A program is hostile input: every line, word and number
//! is checked.

use solidcore_record::Vec3;

use crate::cycle::{DrillCycle, G98Rule};
use crate::toolpath::{ARC_RADIUS_TOLERANCE, ArcDirection, Coolant, Move, Plane, SpindleDirection, Toolpath};
use crate::units::{LengthUnit, MAX_COORDINATE};
use crate::{Error, MAX_MOVES, Result};

/// The longest line accepted, in characters (NIST IR 6556 §3.3).
pub const MAX_LINE_CHARS: usize = 256;
/// The most lines a program may have.
pub const MAX_LINES: usize = 20_000_000;
/// The most M words on one line (§3.3.5).
const MAX_M_WORDS: usize = 4;
/// How close to an integer a code number must be (§3.3.2.1).
const INTEGER_SLOP: f64 = 1e-4;

/// How to read a program.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct ReadSettings {
    /// Where the tool is before the first line, in mm in the work coordinate system.
    pub start: Vec3,
    /// Skip lines that start with `/` (the operator's block-delete switch is on).
    pub block_delete: bool,
    /// What G98 returns a drilling cycle to: the control's rule (decision 0008).
    pub g98: G98Rule,
}

/// What a program read back to.
#[derive(Clone, Debug, PartialEq)]
pub struct ReadBack {
    /// The moves, in mm and mm/min whatever units the program used.
    pub toolpath: Toolpath,
    /// The length unit in force at the end.
    pub units: LengthUnit,
    /// Things accepted but worth a look (an optional stop, text after the program end).
    pub notes: Vec<String>,
    /// How many retracts to machine Z home (`G91 G28 Z0`, `G53 G0 Z…`) the program made. They
    /// are not moves of the toolpath: where the tool ends up depends on the machine.
    pub machine_retracts: usize,
    /// Whether the program ended by sending the table home in Y (`G91 G28 Y0`, after Z).
    pub table_home: bool,
}

/// Reads `program`. Errors name the 1-based line.
pub fn read(program: &str, settings: &ReadSettings) -> Result<ReadBack> {
    let mut m = Machine::new(settings.start, settings.g98);
    for (index, raw) in program.split('\n').enumerate() {
        let line = index + 1;
        if line > MAX_LINES {
            return Err(Error::Gcode { line, message: format!("more than {MAX_LINES} lines") });
        }
        if m.ended {
            if !raw.trim().is_empty() && !is_tape_mark(raw) {
                m.notes.push(format!("line {line}: ignored, after the program end"));
            }
            continue;
        }
        let block = parse_line(raw.strip_suffix('\r').unwrap_or(raw)).map_err(|message| Error::Gcode { line, message })?;
        let Some(block) = block else { continue };
        if block.block_delete && settings.block_delete {
            continue;
        }
        m.execute(&block, line).map_err(|message| Error::Gcode { line, message })?;
        if m.toolpath.moves.len() > MAX_MOVES {
            return Err(Error::TooManyMoves { count: m.toolpath.moves.len(), max: MAX_MOVES });
        }
    }
    m.toolpath.validate()?;
    Ok(ReadBack { toolpath: m.toolpath, units: m.units, notes: m.notes, machine_retracts: m.machine_retracts, table_home: m.table_home })
}

fn is_tape_mark(raw: &str) -> bool {
    raw.trim() == "%"
}

/// One word: a letter and its value.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Word {
    letter: char,
    value: f64,
}

/// A parsed line: its words, after the line number and comments are taken out.
#[derive(Debug, Default)]
struct Block {
    block_delete: bool,
    words: Vec<Word>,
}

/// Splits a line into words (NIST IR 6556 §3.3). `Ok(None)` for a line with nothing to do.
fn parse_line(raw: &str) -> std::result::Result<Option<Block>, String> {
    let chars: Vec<char> = raw.chars().collect();
    if chars.len() > MAX_LINE_CHARS {
        return Err(format!("line is {} characters long (at most {MAX_LINE_CHARS})", chars.len()));
    }
    if is_tape_mark(raw) {
        return Ok(None);
    }
    let mut block = Block::default();
    let mut i = 0;
    let mut first_item = true;
    while i < chars.len() {
        let c = chars.get(i).copied().unwrap_or(' ');
        let col = i + 1;
        match c {
            ' ' | '\t' => i += 1,
            '(' => {
                let close = chars.iter().skip(i + 1).position(|&ch| ch == ')' || ch == '(');
                match close.and_then(|k| chars.get(i + 1 + k).map(|ch| (k, *ch))) {
                    Some((k, ')')) => i += k + 2,
                    Some(_) => return Err(format!("column {col}: a comment may not contain '('")),
                    None => return Err(format!("column {col}: comment is not closed on this line")),
                }
            }
            '/' if first_item && block.words.is_empty() && !block.block_delete => {
                block.block_delete = true;
                i += 1;
            }
            '#' => return Err(format!("column {col}: parameters (#) are not supported")),
            '[' => return Err(format!("column {col}: expressions ([ ]) are not supported")),
            c if c.is_ascii_alphabetic() => {
                let letter = c.to_ascii_uppercase();
                let (value, next, digits) = parse_number(&chars, i + 1).map_err(|e| format!("column {col}: {letter}: {e}"))?;
                match letter {
                    'N' => {
                        if !first_item {
                            return Err(format!("column {col}: a line number must come first"));
                        }
                        if digits.contains(['.', '-', '+']) || digits.len() > 5 {
                            return Err(format!("column {col}: a line number is N and at most 5 digits, not N{digits}"));
                        }
                    }
                    'O' => {
                        if !first_item {
                            return Err(format!("column {col}: a program number must come first"));
                        }
                    }
                    'A'..='D' | 'F' | 'G' | 'H' | 'I' | 'J' | 'K' | 'L' | 'M' | 'P' | 'Q' | 'R' | 'S' | 'T' | 'X' | 'Y' | 'Z' => {
                        block.words.push(Word { letter, value });
                    }
                    other => return Err(format!("column {col}: '{other}' does not start a word")),
                }
                first_item = false;
                i = next;
                continue;
            }
            other => return Err(format!("column {col}: unexpected character {other:?}")),
        }
        if c != ' ' && c != '\t' {
            first_item = false;
        }
    }
    Ok(Some(block))
}

/// A number from `start`: optional sign, digits, at most one point, at least one digit; spaces
/// and tabs between its characters are allowed (§3.3, §3.3.2.1). Returns the value, the index
/// after it and the characters it was written with.
fn parse_number(chars: &[char], start: usize) -> std::result::Result<(f64, usize, String), String> {
    let mut text = String::new();
    let mut i = start;
    while let Some(&c) = chars.get(i) {
        match c {
            ' ' | '\t' => {}
            '+' | '-' if text.is_empty() => text.push(c),
            '0'..='9' => text.push(c),
            '.' if !text.contains('.') => text.push(c),
            '.' => return Err("a number has two decimal points".into()),
            _ => break,
        }
        i += 1;
    }
    if !text.chars().any(|c| c.is_ascii_digit()) {
        return Err(match chars.get(i) {
            Some('#') => "parameters (#) are not supported".into(),
            Some('[') => "expressions ([ ]) are not supported".into(),
            _ => "a word needs a number".into(),
        });
    }
    let value: f64 = text.parse().map_err(|_| format!("{text} is not a number"))?;
    if !value.is_finite() {
        return Err(format!("{text} is out of range"));
    }
    Ok((value, i, text.trim_start_matches(['+', '-']).to_string()))
}

/// The modal group (NIST IR 6556 Table 4) of a G code, given as ten times its number.
fn g_group(code: i64) -> Option<u8> {
    Some(match code {
        0 | 10 | 20 | 30 | 382 | 730 | 800 | 810..=890 => 1,
        170 | 180 | 190 => 2,
        900 | 910 => 3,
        930 | 940 => 5,
        200 | 210 => 6,
        400 | 410 | 420 => 7,
        430 | 490 => 8,
        980 | 990 => 10,
        540 | 550 | 560 | 570 | 580 | 590 | 591 | 592 | 593 => 12,
        610 | 611 | 640 => 13,
        40 | 100 | 280 | 300 | 530 | 920..=923 => 0,
        _ => return None,
    })
}

/// The modal group of an M code.
fn m_group(code: i64) -> Option<u8> {
    Some(match code {
        0 | 1 | 2 | 30 | 60 => 4,
        6 => 6,
        3..=5 => 7,
        7..=9 => 8,
        48 | 49 => 9,
        _ => return None,
    })
}

/// `value` as an integer, when within [`INTEGER_SLOP`] of one.
fn as_integer(value: f64) -> Option<i64> {
    let r = value.round();
    ((value - r).abs() <= INTEGER_SLOP && r.abs() < 1e9).then_some(r as i64)
}

/// Whether two motion codes may share a line: G80 and a move (`G80 G0 Z1.`, which Haas writes),
/// read as the cycle ending and then the move.
fn cancel_then_move(a: i64, b: i64) -> bool {
    let is_move = |g: i64| matches!(g, 0 | 10 | 20 | 30);
    (a == 800 && is_move(b)) || (b == 800 && is_move(a))
}

fn g_name(code: i64) -> String {
    if code % 10 == 0 { format!("G{}", code / 10) } else { format!("G{}.{}", code / 10, code % 10) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Rapid,
    Linear,
    Arc(ArcDirection),
    /// A drilling cycle ([`Machine::cycle`]).
    Cycle,
    Cancelled,
}

/// Where a drilling cycle retracts to after each hole (group 10).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RetractMode {
    /// G98: the height before the cycle ([`G98Rule`] says which).
    Initial,
    /// G99: the R plane.
    RPlane,
}

/// A drilling cycle in force: its code (ten times its number) and the values its holes share, in
/// mm and seconds.
#[derive(Clone, Copy, Debug, PartialEq)]
struct CycleState {
    code: i64,
    bottom: f64,
    r: f64,
    q: Option<f64>,
    p: Option<f64>,
    /// The tool's Z when the cycle was programmed.
    start_z: f64,
}

/// The machine state the program drives.
struct Machine {
    toolpath: Toolpath,
    at: Vec3,
    motion: Motion,
    plane: Plane,
    units: LengthUnit,
    incremental: bool,
    /// Feed in mm/min; 0 until an F word sets it.
    feed: f64,
    speed: f64,
    spindle: Option<SpindleDirection>,
    selected_tool: Option<u32>,
    ended: bool,
    notes: Vec<String>,
    /// After a machine retract the tool's Z in the work coordinates is unknown until an
    /// absolute Z word sets it.
    z_unknown: bool,
    machine_retracts: usize,
    /// The table was sent home in Y: only the program's end may follow.
    table_home: bool,
    retract_mode: Option<RetractMode>,
    cycle: Option<CycleState>,
    g98: G98Rule,
}

impl Machine {
    fn new(start: Vec3, g98: G98Rule) -> Self {
        Self {
            toolpath: Toolpath { start, moves: Vec::new() },
            at: start,
            motion: Motion::Cancelled,
            plane: Plane::Xy,
            units: LengthUnit::Millimetre,
            incremental: false,
            feed: 0.0,
            speed: 0.0,
            spindle: None,
            selected_tool: None,
            ended: false,
            notes: Vec::new(),
            z_unknown: false,
            machine_retracts: 0,
            table_home: false,
            retract_mode: None,
            cycle: None,
            g98,
        }
    }

    fn push(&mut self, m: Move) {
        self.at = m.end(self.at);
        self.toolpath.moves.push(m);
    }

    fn execute(&mut self, block: &Block, line: usize) -> std::result::Result<(), String> {
        // Sort the words: G and M codes (checked for groups), the rest at most once each.
        let mut gs: Vec<i64> = Vec::new();
        let mut ms: Vec<i64> = Vec::new();
        let mut other: [Option<f64>; 26] = [None; 26];
        for w in &block.words {
            match w.letter {
                'G' => {
                    let code = as_integer(w.value * 10.0).ok_or_else(|| format!("G{} is not a G code", w.value))?;
                    let group = g_group(code).ok_or_else(|| format!("{} is not a known G code", g_name(code)))?;
                    let clash = gs.iter().filter(|&&g| g_group(g) == Some(group)).any(|&g| !(group == 1 && cancel_then_move(g, code)));
                    if group != 0 && clash {
                        return Err(format!("two G codes from modal group {group} on one line"));
                    }
                    gs.push(code);
                }
                'M' => {
                    let code = as_integer(w.value).ok_or_else(|| format!("M{} is not an M code", w.value))?;
                    let group = m_group(code).ok_or_else(|| format!("M{code} is not a known M code"))?;
                    if ms.iter().any(|&m| m_group(m) == Some(group)) {
                        return Err(format!("two M codes from modal group {group} on one line"));
                    }
                    ms.push(code);
                    if ms.len() > MAX_M_WORDS {
                        return Err(format!("more than {MAX_M_WORDS} M words on one line"));
                    }
                }
                letter => {
                    let slot =
                        other.get_mut((letter as u8).wrapping_sub(b'A') as usize).ok_or_else(|| format!("'{letter}' does not start a word"))?;
                    if slot.is_some() {
                        return Err(format!("two {letter} words on one line"));
                    }
                    *slot = Some(w.value);
                }
            }
        }
        let word = |c: char| other.get((c as u8).wrapping_sub(b'A') as usize).copied().flatten();
        let has_g = |code: i64| gs.contains(&code);
        let has_m = |code: i64| ms.contains(&code);

        // Refuse what this reader does not model, by name.
        for &g in &gs {
            let unsupported = match g {
                382 => Some("probing (G38.2)"),
                850..=890 => Some("boring cycles (G85-G89)"),
                930 => Some("inverse-time feed (G93)"),
                410 | 420 => Some("cutter radius compensation (G41/G42)"),
                100 | 300 | 920..=923 => Some("moves and offsets in machine coordinates (G10, G30, G92)"),
                _ => None,
            };
            if let Some(what) = unsupported {
                return Err(format!("{}: {what} not supported yet", g_name(g)));
            }
        }
        for axis in ['A', 'B', 'C'] {
            if word(axis).is_some() {
                return Err(format!("rotary axis {axis} not supported yet"));
            }
        }
        if has_m(60) {
            return Err("M60 (pallet change) not supported".into());
        }

        // Table 8 order. Lengths are converted with the units in force *after* this line's
        // G20/G21 (item 12), as every length word is used by later items.
        if has_g(200) {
            self.units = LengthUnit::Inch;
        }
        if has_g(210) {
            self.units = LengthUnit::Millimetre;
        }
        let units = self.units;
        let len = |v: f64| units.to_mm(v);
        // 3. F
        if let Some(f) = word('F') {
            if !(f > 0.0) {
                return Err(format!("feed F{f} must be above 0"));
            }
            self.feed = len(f);
        }
        // 4. S
        let mut speed_changed = false;
        if let Some(s) = word('S') {
            if !(s >= 0.0) {
                return Err(format!("spindle speed S{s} may not be negative"));
            }
            speed_changed = s != self.speed;
            self.speed = s;
        }
        // 5. T
        if let Some(t) = word('T') {
            let t = as_integer(t).filter(|t| (0..=i64::from(u32::MAX)).contains(t)).ok_or_else(|| format!("T{t} is not a tool number"))?;
            self.selected_tool = u32::try_from(t).ok();
        }
        // 6. M6
        if has_m(6) {
            let tool = self.selected_tool.ok_or("M6 with no tool selected (T)")?;
            self.push(Move::ToolChange { tool });
        }
        // 7. Spindle
        let new_direction = if has_m(3) {
            Some(Some(SpindleDirection::Clockwise))
        } else if has_m(4) {
            Some(Some(SpindleDirection::CounterClockwise))
        } else if has_m(5) {
            Some(None)
        } else {
            None
        };
        match (new_direction, self.spindle) {
            (Some(Some(direction)), _) => {
                if !(self.speed > 0.0) {
                    return Err("spindle started with no speed (S)".into());
                }
                self.spindle = Some(direction);
                self.push(Move::Spindle { rpm: self.speed, direction });
            }
            (Some(None), _) => {
                self.spindle = None;
                self.push(Move::SpindleStop);
            }
            (None, Some(direction)) if speed_changed => {
                if !(self.speed > 0.0) {
                    return Err("S0 while the spindle turns: stop it with M5".into());
                }
                self.push(Move::Spindle { rpm: self.speed, direction });
            }
            (None, _) => {}
        }
        // 8. Coolant (M7 and M8 together are both on; we keep the flood).
        if has_m(9) {
            self.push(Move::Coolant { mode: Coolant::Off });
        } else if has_m(8) {
            self.push(Move::Coolant { mode: Coolant::Flood });
        } else if has_m(7) {
            self.push(Move::Coolant { mode: Coolant::Mist });
        }
        // 10. Dwell
        if has_g(40) {
            let p = word('P').ok_or("G4 needs P (seconds)")?;
            if !(p >= 0.0) {
                return Err(format!("dwell P{p} may not be negative"));
            }
            self.push(Move::Dwell { seconds: p });
        }
        // 11. Plane
        if has_g(170) {
            self.plane = Plane::Xy;
        } else if has_g(180) {
            self.plane = Plane::Zx;
        } else if has_g(190) {
            self.plane = Plane::Yz;
        }
        // 14. Tool length offset: positions are already of the tool tip.
        if has_g(430) && word('H').is_none() {
            return Err("G43 needs H (the offset number)".into());
        }
        // 17. Distance mode
        if has_g(900) {
            self.incremental = false;
        } else if has_g(910) {
            self.incremental = true;
        }
        // 18. Retract mode of drilling cycles
        if has_g(980) {
            self.retract_mode = Some(RetractMode::Initial);
        } else if has_g(990) {
            self.retract_mode = Some(RetractMode::RPlane);
        }
        // 19. Retract to machine Z home: exactly `G91 G28 Z0` or `G53 G0 Z…`, the forms tool-change
        // sequences use (decision 0005); and after it `G91 G28 Y0`, the table home, as Haas's
        // examples end a program (decision 0020). Anything else in machine coordinates is refused.
        let retract = has_g(280) || has_g(530);
        if retract {
            let only_z = word('X').is_none() && word('Y').is_none() && word('Z').is_some();
            // With Z at machine home already, the table may be sent home too, to end a program.
            let table = has_g(280) && self.z_unknown && self.incremental && word('X').is_none() && word('Z').is_none() && word('Y') == Some(0.0);
            if has_g(280) {
                if !(table || only_z && self.incremental && word('Z') == Some(0.0)) {
                    return Err(
                        "G28: only `G91 G28 Z0` (retract Z to machine home) is supported, and `G91 G28 Y0` after it (the table home, to end a program)"
                            .into(),
                    );
                }
                if gs.iter().any(|&g| g_group(g) == Some(1)) {
                    return Err("G28 and a motion code on one line".into());
                }
            } else {
                if !(only_z && has_g(0) && !self.incremental) {
                    return Err("G53: only `G53 G0 Z…` in absolute mode (a machine Z position) is supported".into());
                }
                self.motion = Motion::Rapid;
            }
            if table {
                self.table_home = true;
            } else {
                self.z_unknown = true;
                self.machine_retracts += 1;
            }
        }
        // 20. Motion. G80 first: on a line with G80 and a move (as Haas writes `G80 G0 Z1.`) the
        // cycle ends, then the tool moves.
        if has_g(800) {
            self.motion = Motion::Cancelled;
            self.cycle = None;
        }
        let cycle_code = gs.iter().copied().find(|g| matches!(g, 730 | 810 | 820 | 830 | 840));
        for &g in &gs {
            self.motion = match g {
                0 => Motion::Rapid,
                10 => Motion::Linear,
                20 => Motion::Arc(ArcDirection::Clockwise),
                30 => Motion::Arc(ArcDirection::CounterClockwise),
                730 | 810 | 820 | 830 | 840 => Motion::Cycle,
                _ => continue,
            };
            if self.motion != Motion::Cycle {
                self.cycle = None;
            }
        }
        let axes = [word('X'), word('Y'), word('Z')];
        if self.table_home && !retract && (cycle_code.is_some() || axes.iter().any(Option::is_some)) {
            return Err("the table was sent home (G28 Y0): only the program's end may follow".into());
        }
        if retract {
            // The retract was this line's motion.
        } else if self.motion == Motion::Cycle && (cycle_code.is_some() || axes.iter().any(Option::is_some) || word('R').is_some()) {
            if has_g(40) {
                return Err("G4 and a drilling cycle on one line".into());
            }
            self.cycle_line(cycle_code, axes, [word('R'), word('Q'), word('P'), word('L')], &len)?;
        } else if self.z_unknown && word('Z').is_none() && !self.incremental && self.motion == Motion::Rapid && axes.iter().any(Option::is_some) {
            // At machine home the tool goes across to where it will come down: a rapid in X and
            // Y alone. It is the first part of the move that then gives Z, not a move of its
            // own, and Z stays unknown until that one.
            let target = self.target(axes, &len)?;
            self.at = Vec3 { x: target.x, y: target.y, z: self.at.z };
        } else if axes.iter().any(Option::is_some) {
            if self.z_unknown {
                if word('Z').is_none() || self.incremental {
                    return Err(
                        "Z is unknown after a machine retract: the next move must give an absolute Z (a rapid in X and Y alone may come first)"
                            .into(),
                    );
                }
                if matches!(self.motion, Motion::Arc(_)) {
                    return Err("an arc may not start right after a machine retract".into());
                }
            }
            let target = self.target(axes, &len)?;
            self.z_unknown = false;
            match self.motion {
                Motion::Cancelled => return Err("axis words with no motion mode (after G80)".into()),
                Motion::Cycle => return Err("a drilling cycle with no cycle in force".into()),
                Motion::Rapid => self.push(Move::Rapid { to: target }),
                Motion::Linear => {
                    let feed = self.cutting_feed()?;
                    self.push(Move::Linear { to: target, feed });
                }
                Motion::Arc(direction) => {
                    let feed = self.cutting_feed()?;
                    let center = self.arc_center(direction, target, [word('I'), word('J'), word('K')], word('R'), &len)?;
                    self.push(Move::Arc { to: target, center, plane: self.plane, direction, feed });
                }
            }
        } else if gs.iter().any(|&g| matches!(g, 10 | 20 | 30)) {
            return Err("a motion code needs at least one axis word".into());
        }
        // 21. Stop
        if has_m(0) || has_m(1) {
            self.notes.push(format!("line {line}: program stop (M0/M1) ignored for read-back"));
        }
        if has_m(2) || has_m(30) {
            self.ended = true;
        }
        Ok(())
    }

    /// One hole of a drilling cycle: a line with G73, G81, G82, G83 or G84 (`code`), or a line
    /// with X or Y while one is in force. `words` are R, Q, P and L.
    fn cycle_line(
        &mut self,
        code: Option<i64>,
        axes: [Option<f64>; 3],
        words: [Option<f64>; 4],
        len: &impl Fn(f64) -> f64,
    ) -> std::result::Result<(), String> {
        let [r, q, p, l] = words;
        if self.plane != Plane::Xy {
            return Err("drilling cycles only in the XY plane (G17)".into());
        }
        if self.incremental {
            return Err("drilling cycles in incremental mode (G91) are not supported yet".into());
        }
        if l.is_some() {
            return Err("cycle repeats (L) are not supported".into());
        }
        if self.z_unknown {
            return Err("Z is unknown after a machine retract: give an absolute Z before a drilling cycle".into());
        }
        let mode = self.retract_mode.ok_or("a drilling cycle before G98 or G99: controls differ in which they start in")?;
        let [x, y, z] = axes;
        if x.is_none() && y.is_none() {
            return Err("a drilling cycle line without X or Y: controls differ on whether it drills (Haas Setting 28)".into());
        }
        let state = match (code, self.cycle) {
            (Some(_), Some(_)) if self.g98 == G98Rule::CycleStart => {
                return Err(
                    "a new drilling cycle while one is in force: end it with G80 first (this control's G98 height comes from the first)".into()
                );
            }
            (Some(code), _) => CycleState {
                code,
                bottom: len(z.ok_or("a drilling cycle needs Z (the hole's bottom)")?),
                r: len(r.ok_or("a drilling cycle needs R (the R plane)")?),
                q: q.map(len),
                p,
                start_z: self.at.z,
            },
            (None, Some(mut state)) => {
                state.bottom = z.map_or(state.bottom, len);
                state.r = r.map_or(state.r, len);
                state.q = q.map(len).or(state.q);
                state.p = p.or(state.p);
                state
            }
            (None, None) => return Err("a drilling cycle with no cycle in force".into()),
        };
        let name = g_name(state.code);
        let mut cycle = match state.code {
            810 | 820 | 840 if state.q.is_some() => return Err(format!("{name} takes no Q")),
            730 | 810 | 830 | 840 if state.p.is_some() => return Err(format!("P (a dwell) on {name} is not supported")),
            810 => DrillCycle::Drill { dwell: 0.0 },
            // Its pitch is its feed over the spindle speed: worked out below, with the feed.
            840 => match self.spindle {
                Some(SpindleDirection::Clockwise) if self.speed > 0.0 => DrillCycle::Tap { pitch: 0.0 },
                Some(SpindleDirection::CounterClockwise) => {
                    return Err("G84 with the spindle turning counter-clockwise: it taps a right-hand thread, clockwise".into());
                }
                _ => return Err("G84 with the spindle stopped: it must be turning clockwise already (NIST IR 6556 3.5.16)".into()),
            },
            820 => match state.p {
                Some(p) if p >= 0.0 => DrillCycle::Drill { dwell: p },
                Some(p) => return Err(format!("dwell P{p} may not be negative")),
                None => return Err("G82 needs P (the dwell in seconds)".into()),
            },
            730 | 830 => match state.q {
                Some(q) if q > 0.0 && state.code == 830 => DrillCycle::Peck { peck: q },
                Some(q) if q > 0.0 => DrillCycle::ChipBreak { peck: q },
                Some(_) => return Err(format!("{name}: Q (the peck) must be above 0")),
                None => return Err(format!("{name} needs Q (the peck)")),
            },
            _ => return Err(format!("{name} is not a drilling cycle")),
        };
        if [state.bottom, state.r].iter().any(|v| !(v.abs() <= MAX_COORDINATE)) {
            return Err(format!("a position beyond {MAX_COORDINATE} mm"));
        }
        if !(state.r > state.bottom) {
            return Err("the R plane must be above the hole's bottom (R above Z)".into());
        }
        if self.at.z < state.r {
            return Err("the tool is below the R plane when the hole starts: controls differ on what happens".into());
        }
        let hole = self.target([x, y, None], len)?;
        let retract = match (mode, self.g98) {
            (RetractMode::RPlane, _) => state.r,
            (RetractMode::Initial, G98Rule::BlockStart) => self.at.z.max(state.r),
            (RetractMode::Initial, G98Rule::CycleStart) => state.start_z.max(state.r),
        };
        let feed = self.cutting_feed()?;
        if let DrillCycle::Tap { pitch } = &mut cycle {
            *pitch = feed / self.speed;
        }
        self.push(Move::Drill { to: Vec3 { x: hole.x, y: hole.y, z: state.bottom }, r: state.r, retract, cycle, feed });
        self.cycle = Some(state);
        Ok(())
    }

    fn cutting_feed(&self) -> std::result::Result<f64, String> {
        if self.feed > 0.0 { Ok(self.feed) } else { Err("a cutting move before any feed (F)".into()) }
    }

    /// The end point from axis words (in program units), absolute or incremental.
    fn target(&self, axes: [Option<f64>; 3], len: &impl Fn(f64) -> f64) -> std::result::Result<Vec3, String> {
        let current = [self.at.x, self.at.y, self.at.z];
        let mut out = current;
        for ((o, cur), w) in out.iter_mut().zip(current).zip(axes) {
            if let Some(v) = w {
                *o = if self.incremental { cur + len(v) } else { len(v) };
            }
        }
        if out.iter().any(|c| !(c.abs() <= MAX_COORDINATE)) {
            return Err(format!("a position beyond {MAX_COORDINATE} mm"));
        }
        Ok(Vec3 { x: out[0], y: out[1], z: out[2] })
    }

    /// The centre of an arc from the current point to `to`: centre format (I, J, K: offsets from
    /// the start, always incremental) or radius format (R; negative for more than half a turn).
    fn arc_center(
        &self,
        direction: ArcDirection,
        to: Vec3,
        ijk: [Option<f64>; 3],
        r: Option<f64>,
        len: &impl Fn(f64) -> f64,
    ) -> std::result::Result<Vec3, String> {
        let plane = self.plane;
        let (su, sv, sw) = plane.split(self.at);
        let (eu, ev, _) = plane.split(to);
        // The in-plane offsets in (u, v) order: G17 (I, J), G18 (K, I), G19 (J, K).
        let [i, j, k] = ijk;
        let (ou, ov, names) = match plane {
            Plane::Xy => (i, j, "I and J"),
            Plane::Zx => (k, i, "I and K"),
            Plane::Yz => (j, k, "J and K"),
        };
        if let Some(r) = r {
            if ou.is_some() || ov.is_some() {
                return Err("an arc has R or centre offsets, not both".into());
            }
            let r = len(r);
            let (du, dv) = (eu - su, ev - sv);
            let chord = du.hypot(dv);
            if !(chord > 1e-9) {
                return Err("a radius-format arc must end away from where it starts".into());
            }
            let half = 0.5 * chord;
            if !(r.abs() >= half - ARC_RADIUS_TOLERANCE) {
                return Err(format!("radius {} mm is too small for an arc spanning {chord} mm", r.abs()));
            }
            let h = (r * r - half * half).max(0.0).sqrt();
            // Left of the chord, seen from the plane's normal, for a short counter-clockwise arc.
            let (nu, nv) = (-dv / chord, du / chord);
            let short = r > 0.0;
            let left = matches!(direction, ArcDirection::CounterClockwise) == short;
            let s = if left { h } else { -h };
            return Ok(plane.join(su + 0.5 * du + s * nu, sv + 0.5 * dv + s * nv, sw));
        }
        if ou.is_none() && ov.is_none() {
            return Err(format!("an arc in this plane needs {names}, or R"));
        }
        let center = plane.join(su + ou.map_or(0.0, len), sv + ov.map_or(0.0, len), sw);
        let (cu, cv, _) = plane.split(center);
        let (r0, r1) = ((su - cu).hypot(sv - cv), (eu - cu).hypot(ev - cv));
        if !((r0 - r1).abs() <= ARC_RADIUS_TOLERANCE) {
            return Err(format!("arc start is {r0} mm from its centre and its end {r1} mm (at most {ARC_RADIUS_TOLERANCE} mm apart)"));
        }
        Ok(center)
    }
}

#[cfg(test)]
#[path = "gcode_reader_tests.rs"]
mod tests;
