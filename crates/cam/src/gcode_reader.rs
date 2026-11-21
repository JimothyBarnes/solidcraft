//! Reads G-code back into a [`Toolpath`], so every program a post writes can be checked against
//! the toolpath it came from (SPEC §6, "read-back").
//!
//! The language is the RS274/NGC subset our posts emit, read by the rules of NIST IR 6556 §3
//! (`research/gcode-rs274.md`): case-insensitive words, spaces anywhere outside comments,
//! `( … )` comments, block delete, line numbers, the modal groups and the order of execution of
//! Table 8. Fanuc-style `%` tape marks and `O` program numbers are accepted and ignored.
//!
//! Anything outside the subset (parameters, expressions, canned cycles, cutter compensation,
//! inverse-time feed, rotary axes, machine-coordinate moves) is refused with the line and the
//! reason, never guessed at. A program is hostile input: every line, word and number is checked.

use solidcore_record::Vec3;

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
}

/// Reads `program`. Errors name the 1-based line.
pub fn read(program: &str, settings: &ReadSettings) -> Result<ReadBack> {
    let mut m = Machine::new(settings.start);
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
    Ok(ReadBack { toolpath: m.toolpath, units: m.units, notes: m.notes, machine_retracts: m.machine_retracts })
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

fn g_name(code: i64) -> String {
    if code % 10 == 0 { format!("G{}", code / 10) } else { format!("G{}.{}", code / 10, code % 10) }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Motion {
    Rapid,
    Linear,
    Arc(ArcDirection),
    Cancelled,
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
}

impl Machine {
    fn new(start: Vec3) -> Self {
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
                    if group != 0 && gs.iter().any(|&g| g_group(g) == Some(group)) {
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
                730 | 810..=890 => Some("canned cycles"),
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
        // 19. Retract to machine Z home: exactly `G91 G28 Z0` or `G53 G0 Z…`, the forms tool-change
        // sequences use (decision 0005). Anything else in machine coordinates is refused.
        let retract = has_g(280) || has_g(530);
        if retract {
            let only_z = word('X').is_none() && word('Y').is_none() && word('Z').is_some();
            if has_g(280) {
                if !(only_z && self.incremental && word('Z') == Some(0.0)) {
                    return Err("G28: only `G91 G28 Z0` (retract Z to machine home) is supported".into());
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
            self.z_unknown = true;
            self.machine_retracts += 1;
        }
        // 20. Motion
        for &g in &gs {
            self.motion = match g {
                0 => Motion::Rapid,
                10 => Motion::Linear,
                20 => Motion::Arc(ArcDirection::Clockwise),
                30 => Motion::Arc(ArcDirection::CounterClockwise),
                800 => Motion::Cancelled,
                _ => continue,
            };
        }
        let axes = [word('X'), word('Y'), word('Z')];
        if retract {
            // The retract was this line's motion.
        } else if axes.iter().any(Option::is_some) {
            if self.z_unknown {
                if word('Z').is_none() || self.incremental {
                    return Err("Z is unknown after a machine retract: the next move must give an absolute Z".into());
                }
                if matches!(self.motion, Motion::Arc(_)) {
                    return Err("an arc may not start right after a machine retract".into());
                }
            }
            let target = self.target(axes, &len)?;
            self.z_unknown = false;
            match self.motion {
                Motion::Cancelled => return Err("axis words with no motion mode (after G80)".into()),
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
