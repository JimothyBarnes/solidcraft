//! Post-processing: a [`Toolpath`] to G-code for one controller, by one fixed writer driven by a
//! [`Post`] file (decision 0005). A post only picks options, formats numbers and fills named
//! slots in fixed lines; there is no scripting. Every program is read back with our reader and
//! compared with what was meant before it is returned, so a post can never hand out text that
//! does something else.
//!
//! Holes go out as canned cycles where the post's [`Cycles`] says the control has them and every
//! control we know reads them alike, and as plain moves otherwise (decision 0008).
//!
//! The output is never "safe to run": every program starts by saying it is unverified and must
//! be proven on the machine first (air cut, single block).

use std::f64::consts::TAU;

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::cycle::{self, CycleMotion, DrillCycle, G98Rule};
use crate::gcode_reader::{ReadBack, ReadSettings, read};
use crate::toolpath::{ArcDirection, ArcGeom, Coolant, Move, Plane, SpindleDirection, Toolpath};
use crate::units::LengthUnit;
use crate::{Error, Result};

/// The post file format this build reads.
pub const POST_FORMAT: u32 = 1;
/// The posts built into the app, by id.
pub const BUILTIN_POSTS: &[(&str, &str)] = &[
    ("fanuc_mill", include_str!("../posts/fanuc_mill.json")),
    ("haas_mill", include_str!("../posts/haas_mill.json")),
    ("rs274", include_str!("../posts/rs274.json")),
];
/// The most short lines one arc may become when it cannot be written as an arc.
const MAX_ARC_SEGMENTS: usize = 10_000;
/// The safety text every program opens with (our own wording; each line fits 48 characters).
const SAFETY_LINES: [&str; 2] = ["UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM", "PROVE OUT FIRST: AIR CUT, SINGLE BLOCK"];

/// The kind of machine a post writes for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MachineKind {
    Mill,
}

/// Decimal places per kind of number.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Numbers {
    pub length_mm: u8,
    pub length_inch: u8,
    pub feed_mm: u8,
    pub feed_inch: u8,
    pub speed: u8,
    pub dwell: u8,
    /// Write `10.` rather than `10.000`.
    pub strip_trailing_zeros: bool,
}

/// Line numbers (`N…`) on every line but tape marks and the program number.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LineNumbers {
    pub start: u32,
    pub step: u32,
}

/// How comments are written. Comments are always in parentheses.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Comments {
    pub upper_case: bool,
    pub max_length: usize,
}

/// Whether modal words are written again when unchanged.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Modal {
    pub repeat_motion: bool,
    pub repeat_feed: bool,
}

/// What arcs the controller takes. Arcs are always written in centre format (I, J, K).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Arcs {
    /// The planes arcs may be written in; arcs in other planes become lines.
    pub planes: Vec<Plane>,
    /// Whether an arc may also move along its plane's normal.
    pub helical: bool,
    /// Arcs sweeping more are split into equal pieces.
    pub max_sweep_degrees: f64,
    /// Arcs with a smaller radius (mm) become lines.
    pub min_radius: f64,
    /// How far (mm) lines replacing an arc may stray from it.
    pub linearize_tolerance: f64,
}

/// Which drilling cycles the control takes as canned cycles, and how it runs them (decision
/// 0008). A hole whose cycle is off here, or that a canned cycle can't say exactly, is written as
/// plain moves with `motion`'s distances.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Cycles {
    /// G81: a hole with no dwell.
    pub drill: bool,
    /// G82: a hole with a dwell at the bottom (needs the post's `dwell`).
    pub dwell: bool,
    /// G83: pecks with a full retract.
    pub peck: bool,
    /// G73: pecks with a chip-breaking back-off.
    pub chip_break: bool,
    /// G84: a hole tapped in step with the spindle. A post without it refuses a tapped hole: it
    /// can't be written as plain moves.
    #[serde(default)]
    pub tap: bool,
    /// What the control's G98 returns to.
    pub g98: G98Rule,
    /// The control's own cycle distances, used to draw and simulate holes and to write them as
    /// plain moves.
    pub motion: CycleMotion,
}

/// The fixed lines of a program. Each may use `{placeholders}`; see [`Post::from_json`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Sequences {
    pub program_start: Vec<String>,
    pub safe_start: Vec<String>,
    pub tool_change_before: Vec<String>,
    pub tool_change_after: Vec<String>,
    pub operation_start: Vec<String>,
    pub program_end: Vec<String>,
}

/// A post: everything controller-specific about writing G-code, as data.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Post {
    pub format: u32,
    pub id: String,
    pub name: String,
    pub description: String,
    pub machine_kind: MachineKind,
    pub numbers: Numbers,
    pub line_numbers: Option<LineNumbers>,
    pub comments: Comments,
    pub modal: Modal,
    pub arcs: Arcs,
    /// Digits of the `{program_number}`, zero-padded; `None` when the post has no program number.
    pub program_number_digits: Option<u8>,
    /// Whether dwells are written (`G4 P<seconds>`, always with all `numbers.dwell` decimals, so a
    /// control that reads a P without a fraction as milliseconds still reads seconds); otherwise a
    /// dwell is refused.
    pub dwell: bool,
    /// Drilling cycles. A post file without them writes every hole as plain moves.
    #[serde(default)]
    pub cycles: Cycles,
    /// Words put before the first move after a tool change (e.g. `G43 H{tool}`).
    pub tool_length_offset: Option<String>,
    pub sequences: Sequences,
}

/// What a program is for, beyond its moves.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramContext {
    pub program_number: u32,
    pub program_name: String,
    /// The units the program is written in.
    pub units: LengthUnit,
    /// The work offset, 1 to 6 for G54 to G59.
    pub work_offset: u8,
}

impl Default for ProgramContext {
    fn default() -> Self {
        Self { program_number: 1, program_name: "PROGRAM".into(), units: LengthUnit::Millimetre, work_offset: 1 }
    }
}

/// A written program and what reading it back found.
#[derive(Clone, Debug, PartialEq)]
pub struct Posted {
    pub text: String,
    /// The moves the text was written to make, after arcs were split or turned into lines and
    /// holes the post can't write as canned cycles into plain moves.
    pub written: Toolpath,
    /// The program read back with our reader; it matched `written` within [`Posted::tolerance`].
    pub read_back: ReadBack,
    /// The largest position difference accepted, in mm (half the output resolution).
    pub tolerance: f64,
    /// Things worth knowing (an arc written as lines, a move too short to write).
    pub notes: Vec<String>,
}

/// How many more decimals a tap's feed is written with than other feeds.
const TAP_FEED_DECIMALS: u8 = 2;
/// How far the pitch a tap's feed and spindle speed say, as written, may be from its own, as a
/// share of it.
const TAP_PITCH_SLOP: f64 = 1e-4;

const PLACEHOLDERS: &[&str] = &["program_number", "program_name", "units_code", "work_offset", "tool"];

impl Post {
    /// The built-in post `id`.
    pub fn builtin(id: &str) -> Result<Post> {
        let (_, json) = BUILTIN_POSTS.iter().find(|(name, _)| *name == id).ok_or_else(|| Error::BadPost(format!("no built-in post '{id}'")))?;
        Post::from_json(json)
    }

    /// Reads and checks a post file. Placeholders are `{program_number}`, `{program_name}`,
    /// `{units_code}` (G20/G21), `{work_offset}` (G54…G59) and `{tool}` (only in the tool-change
    /// lines and the tool length offset). Every fixed line must read without making a move.
    pub fn from_json(json: &str) -> Result<Post> {
        let post: Post = serde_json::from_str(json).map_err(|e| Error::BadPost(e.to_string()))?;
        post.check()?;
        Ok(post)
    }

    fn check(&self) -> Result<()> {
        let bad = |m: String| Err(Error::BadPost(format!("{}: {m}", self.id)));
        if self.format != POST_FORMAT {
            return bad(format!("format {} (this build reads {POST_FORMAT})", self.format));
        }
        if self.id.is_empty() || self.id.len() > 64 || !self.id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_') {
            return bad("id must be 1 to 64 of a-z, 0-9 and _".into());
        }
        if self.name.chars().count() > 100 || self.description.chars().count() > 2000 {
            return bad("name or description too long".into());
        }
        let n = self.numbers;
        if !(1..=6).contains(&n.length_mm) || !(1..=6).contains(&n.length_inch) || n.feed_mm > 4 || n.feed_inch > 4 || n.speed > 3 || n.dwell > 3 {
            return bad("decimals out of range (lengths 1-6, feeds 0-4, speed and dwell 0-3)".into());
        }
        if let Some(l) = self.line_numbers
            && (l.start > 99_999 || !(1..=1000).contains(&l.step))
        {
            return bad("line numbers start at 0-99999 with a step of 1-1000".into());
        }
        if !(48..=256).contains(&self.comments.max_length) {
            return bad("comments.max_length must be 48 to 256".into());
        }
        let a = &self.arcs;
        if !(1.0..=360.0).contains(&a.max_sweep_degrees) || !(0.0..=1000.0).contains(&a.min_radius) || !(1e-4..=1.0).contains(&a.linearize_tolerance)
        {
            return bad("arcs: max_sweep_degrees 1-360, min_radius 0-1000 mm, linearize_tolerance 0.0001-1 mm".into());
        }
        if self.dwell && n.dwell == 0 {
            return bad("a post that writes dwells needs numbers.dwell of 1 to 3 (a P without a fraction is milliseconds on some controls)".into());
        }
        if let Some(d) = self.program_number_digits
            && !(1..=8).contains(&d)
        {
            return bad("program_number_digits must be 1 to 8".into());
        }
        if let Err(e) = self.cycles.motion.check() {
            return bad(format!("cycles.motion: {e}"));
        }
        if self.cycles.dwell && !self.dwell {
            return bad("cycles.dwell needs dwell: G82's P is written as a dwell's is".into());
        }
        let s = &self.sequences;
        let sample =
            Fill { program_number: Some("1".into()), program_name: "TEST".into(), units_code: "G21", work_offset: "G54".into(), tool: Some(1) };
        let no_tool = Fill { tool: None, ..sample.clone() };
        let groups: [(&str, &[String], &Fill); 6] = [
            ("program_start", &s.program_start, &no_tool),
            ("safe_start", &s.safe_start, &no_tool),
            ("tool_change_before", &s.tool_change_before, &sample),
            ("tool_change_after", &s.tool_change_after, &sample),
            ("operation_start", &s.operation_start, &no_tool),
            ("program_end", &s.program_end, &no_tool),
        ];
        let tlo = self.tool_length_offset.iter().cloned().collect::<Vec<_>>();
        for (name, lines, fill) in groups.into_iter().chain([("tool_length_offset", tlo.as_slice(), &sample)]) {
            if lines.len() > 50 {
                return bad(format!("{name}: at most 50 lines"));
            }
            let fill = if self.program_number_digits.is_none() { &Fill { program_number: None, ..fill.clone() } } else { fill };
            let mut text = String::new();
            for line in lines {
                if line.chars().count() > 200 {
                    return bad(format!("{name}: a line longer than 200 characters"));
                }
                text.push_str(&render(line, fill).map_err(|e| Error::BadPost(format!("{}: {name}: {e}", self.id)))?);
                text.push('\n');
            }
            let r = read(&text, &ReadSettings::default()).map_err(|e| Error::BadPost(format!("{}: {name} does not read: {e}", self.id)))?;
            if !r.toolpath.moves.is_empty() {
                return bad(format!("{name} makes moves; fixed lines may only set modes"));
            }
        }
        Ok(())
    }

    /// Writes `toolpath` as a program, reads it back and checks it against what was meant.
    pub fn write(&self, toolpath: &Toolpath, ctx: &ProgramContext) -> Result<Posted> {
        self.check()?;
        toolpath.validate()?;
        let mut w = Writer::new(self, ctx)?;
        w.program(toolpath)?;
        let text = w.lines.join("\n") + "\n";
        let written = Toolpath { start: toolpath.start, moves: std::mem::take(&mut w.written) };
        let read_back = read(&text, &ReadSettings { start: toolpath.start, block_delete: false, g98: self.cycles.g98 })?;
        let tolerance = 0.5 * ctx.units.to_mm(10f64.powi(-i32::from(w.length_decimals)));
        compare(&written, &read_back.toolpath, tolerance, &w)?;
        Ok(Posted { text, written, read_back, tolerance, notes: std::mem::take(&mut w.notes) })
    }
}

/// Values for the placeholders.
#[derive(Clone)]
struct Fill {
    program_number: Option<String>,
    program_name: String,
    units_code: &'static str,
    work_offset: String,
    tool: Option<u32>,
}

/// `line` with its placeholders filled; an unknown or unavailable placeholder is an error.
fn render(line: &str, fill: &Fill) -> std::result::Result<String, String> {
    let mut out = String::new();
    let mut rest = line;
    while let Some(open) = rest.find(['{', '}']) {
        let (before, after) = rest.split_at(open);
        out.push_str(before);
        if after.starts_with('}') {
            return Err("a '}' with no '{'".into());
        }
        let close = after.find('}').ok_or("a '{' with no '}'")?;
        let name = after.get(1..close).unwrap_or("");
        let value = match name {
            "program_number" => fill.program_number.clone().ok_or("{program_number} needs program_number_digits")?,
            "program_name" => fill.program_name.clone(),
            "units_code" => fill.units_code.to_string(),
            "work_offset" => fill.work_offset.clone(),
            "tool" => fill.tool.map(|t| t.to_string()).ok_or("{tool} only in tool-change lines and the tool length offset")?,
            other => return Err(format!("unknown placeholder {{{other}}} (known: {})", PLACEHOLDERS.join(", "))),
        };
        out.push_str(&value);
        rest = after.get(close + 1..).unwrap_or("");
    }
    out.push_str(rest);
    Ok(out)
}

/// `value` with `decimals` places, always with a decimal point (on Fanuc-style controls `X1`
/// may mean 0.001), `-0` written as `0`.
fn format_decimal(value: f64, decimals: u8, strip: bool) -> String {
    let mut s = format!("{:.*}", usize::from(decimals), value);
    if s.starts_with('-') && s.chars().all(|c| matches!(c, '-' | '0' | '.')) {
        s.remove(0);
    }
    if strip && s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
    }
    if !s.contains('.') {
        s.push('.');
    }
    s
}

/// A canned cycle being written: the words its holes share. A hole with the same words is one more
/// `X… Y…` line of it.
#[derive(Clone, Debug, PartialEq)]
struct CycleKey {
    /// The return mode and the cycle code (`G99 G81`).
    head: [String; 2],
    /// Z, R, Q or P, and F.
    tail: Vec<String>,
    /// For G98, the height every hole returns to.
    height: Option<String>,
}

/// The program as it is being written.
struct Writer<'a> {
    post: &'a Post,
    ctx: &'a ProgramContext,
    lines: Vec<String>,
    next_line_number: Option<u32>,
    length_decimals: u8,
    feed_decimals: u8,
    /// The last X, Y, Z written (as text and as the value a reader gets back, in mm); `None`
    /// when unknown, so the next move writes it.
    out: [Option<(String, f64)>; 3],
    /// The exact position the toolpath is at.
    at: Vec3,
    motion: Option<&'static str>,
    feed: Option<String>,
    plane: Option<Plane>,
    spindle_on: bool,
    /// The spindle speed as a reader gets it back from the last S written, rev/min.
    spindle_speed: Option<f64>,
    /// The tool whose length offset goes on the next move.
    pending_offset: Option<u32>,
    /// Whether the machine's position is known: not at the program start (the machine is
    /// wherever it was) nor after a tool change. Until it is, the next move must be a rapid,
    /// and it writes all three axes.
    position_known: bool,
    /// The canned cycle in force, if any.
    cycle: Option<CycleKey>,
    written: Vec<Move>,
    notes: Vec<String>,
}

impl<'a> Writer<'a> {
    fn new(post: &'a Post, ctx: &'a ProgramContext) -> Result<Self> {
        if !(1..=6).contains(&ctx.work_offset) {
            return Err(Error::BadValue(format!("work offset {} (1 to 6 for G54 to G59)", ctx.work_offset)));
        }
        if let Some(d) = post.program_number_digits
            && ctx.program_number.to_string().len() > usize::from(d)
        {
            return Err(Error::BadValue(format!("program number {} has more than {d} digits", ctx.program_number)));
        }
        let (length_decimals, feed_decimals) = match ctx.units {
            LengthUnit::Millimetre => (post.numbers.length_mm, post.numbers.feed_mm),
            LengthUnit::Inch => (post.numbers.length_inch, post.numbers.feed_inch),
        };
        Ok(Self {
            post,
            ctx,
            lines: Vec::new(),
            next_line_number: post.line_numbers.map(|l| l.start),
            length_decimals,
            feed_decimals,
            out: [None, None, None],
            at: Vec3::default(),
            motion: None,
            feed: None,
            plane: None,
            spindle_on: false,
            spindle_speed: None,
            pending_offset: None,
            position_known: false,
            cycle: None,
            written: Vec::new(),
            notes: Vec::new(),
        })
    }

    fn fill(&self, tool: Option<u32>) -> Fill {
        Fill {
            program_number: self.post.program_number_digits.map(|d| format!("{:0width$}", self.ctx.program_number, width = usize::from(d))),
            program_name: self.comment_text(&self.ctx.program_name),
            units_code: if self.ctx.units == LengthUnit::Inch { "G20" } else { "G21" },
            work_offset: format!("G{}", 53 + u32::from(self.ctx.work_offset)),
            tool,
        }
    }

    /// Text safe inside a comment: printable ASCII, no parentheses, cut to length.
    fn comment_text(&self, text: &str) -> String {
        let cleaned: String = text.chars().map(|c| if (c.is_ascii_graphic() && c != '(' && c != ')') || c == ' ' { c } else { ' ' }).collect();
        let cleaned = if self.post.comments.upper_case { cleaned.to_ascii_uppercase() } else { cleaned };
        cleaned.chars().take(self.post.comments.max_length.saturating_sub(2)).collect()
    }

    fn emit(&mut self, line: String) {
        let unnumbered = line.starts_with('%') || line.starts_with('O') || line.starts_with('o');
        match (self.next_line_number, self.post.line_numbers) {
            (Some(n), Some(l)) if !unnumbered => {
                self.lines.push(format!("N{n} {line}"));
                let next = n.saturating_add(l.step);
                self.next_line_number = Some(if next > 99_999 { l.start } else { next });
            }
            _ => self.lines.push(line),
        }
    }

    fn sequence(&mut self, lines: &[String], tool: Option<u32>) -> Result<()> {
        let fill = self.fill(tool);
        for line in lines {
            let text = render(line, &fill).map_err(|e| Error::BadPost(format!("{}: {e}", self.post.id)))?;
            self.emit(text);
        }
        Ok(())
    }

    fn program(&mut self, toolpath: &Toolpath) -> Result<()> {
        self.at = toolpath.start;
        let s = &self.post.sequences;
        self.sequence(&s.program_start.clone(), None)?;
        for text in SAFETY_LINES {
            let c = self.comment_text(text);
            self.emit(format!("({c})"));
        }
        self.sequence(&s.safe_start.clone(), None)?;
        self.sequence(&s.operation_start.clone(), None)?;
        for (index, m) in toolpath.moves.iter().enumerate() {
            self.one(index, m)?;
        }
        self.end_cycle();
        self.sequence(&self.post.sequences.program_end.clone(), None)?;
        Ok(())
    }

    /// A length as written and as read back (mm).
    fn length(&self, mm: f64) -> (String, f64) {
        let text = format_decimal(self.ctx.units.from_mm(mm), self.length_decimals, self.post.numbers.strip_trailing_zeros);
        let back = text.parse::<f64>().map(|v| self.ctx.units.to_mm(v)).unwrap_or(f64::NAN);
        (text, back)
    }

    fn feed_word(&mut self, feed: f64) -> Option<String> {
        let text = format_decimal(self.ctx.units.from_mm(feed), self.feed_decimals, self.post.numbers.strip_trailing_zeros);
        if self.post.modal.repeat_feed || self.feed.as_deref() != Some(text.as_str()) {
            self.feed = Some(text.clone());
            Some(format!("F{text}"))
        } else {
            None
        }
    }

    fn motion_word(&mut self, code: &'static str) -> Option<String> {
        if self.post.modal.repeat_motion || self.motion != Some(code) {
            self.motion = Some(code);
            Some(code.to_string())
        } else {
            None
        }
    }

    /// Axis words for `to`: changed axes only, or all of them (`all`). Returns the words and
    /// whether any was needed.
    fn axis_words(&mut self, to: Vec3, all: bool) -> Vec<String> {
        let mut words = Vec::new();
        for (k, (letter, v)) in [('X', to.x), ('Y', to.y), ('Z', to.z)].into_iter().enumerate() {
            let (text, back) = self.length(v);
            let Some(slot) = self.out.get_mut(k) else { continue };
            if all || slot.as_ref().map(|(t, _)| t != &text).unwrap_or(true) {
                words.push(format!("{letter}{text}"));
                *slot = Some((text, back));
            }
        }
        words
    }

    fn one(&mut self, index: usize, m: &Move) -> Result<()> {
        let bad = |reason: String| Error::BadMove { index, reason };
        let first_after_change = !self.position_known;
        if !matches!(m, Move::Drill { .. }) {
            self.end_cycle();
        }
        match *m {
            Move::Rapid { to } => {
                let mut words: Vec<String> = Vec::new();
                if let Some(tool) = self.pending_offset.take()
                    && let Some(tlo) = self.post.tool_length_offset.clone()
                {
                    words.push(render(&tlo, &self.fill(Some(tool))).map_err(|e| Error::BadPost(format!("{}: {e}", self.post.id)))?);
                }
                let axes = self.axis_words(to, first_after_change);
                if axes.is_empty() {
                    self.notes.push(format!("move {index}: a rapid too short to write was left out"));
                    self.at = to;
                    return Ok(());
                }
                if first_after_change {
                    self.motion = None;
                }
                words.extend(self.motion_word("G0"));
                words.extend(axes);
                self.emit(words.join(" "));
                self.written.push(Move::Rapid { to });
                self.at = to;
                self.position_known = true;
            }
            _ if first_after_change && matches!(m, Move::Linear { .. } | Move::Arc { .. } | Move::Drill { .. }) => {
                return Err(bad("the first move of a program or after a tool change must be a rapid".into()));
            }
            Move::Linear { to, feed } => self.linear(index, to, feed),
            Move::Arc { to, center, plane, direction, feed } => self.arc(index, to, center, plane, direction, feed)?,
            Move::Drill { to, r, retract, cycle, feed } => self.drill(index, to, r, retract, cycle, feed)?,
            Move::Dwell { seconds } => {
                if !self.post.dwell {
                    return Err(bad(format!("the {} post has no verified dwell format yet", self.post.id)));
                }
                // Never stripped: `P1.` could read as milliseconds on some controls; `P1.000` cannot.
                self.emit(format!("G4 P{}", format_decimal(seconds, self.post.numbers.dwell, false)));
                self.written.push(*m);
            }
            Move::ToolChange { tool } => {
                if self.spindle_on {
                    return Err(bad("stop the spindle before a tool change".into()));
                }
                self.sequence(&self.post.sequences.tool_change_before.clone(), Some(tool))?;
                self.emit(format!("T{tool} M6"));
                self.sequence(&self.post.sequences.tool_change_after.clone(), Some(tool))?;
                self.out = [None, None, None];
                self.pending_offset = Some(tool);
                self.position_known = false;
                self.written.push(*m);
            }
            Move::Spindle { rpm, direction } => {
                let s = format_decimal(rpm, self.post.numbers.speed, true);
                let s = s.strip_suffix('.').unwrap_or(&s).to_string();
                let code = if direction == SpindleDirection::Clockwise { "M3" } else { "M4" };
                self.emit(format!("S{s} {code}"));
                self.spindle_on = true;
                self.spindle_speed = s.parse().ok();
                self.written.push(*m);
            }
            Move::SpindleStop => {
                self.emit("M5".into());
                self.spindle_on = false;
                self.spindle_speed = None;
                self.written.push(*m);
            }
            Move::Coolant { mode } => {
                self.emit(
                    match mode {
                        Coolant::Off => "M9",
                        Coolant::Flood => "M8",
                        Coolant::Mist => "M7",
                    }
                    .into(),
                );
                self.written.push(*m);
            }
        }
        Ok(())
    }

    /// Ends the canned cycle in force, if any.
    fn end_cycle(&mut self) {
        if self.cycle.take().is_some() {
            self.emit("G80".into());
            self.motion = None;
        }
    }

    /// The canned cycle words for a hole from `from`, or why it can't be one. G98 is used only when
    /// the hole returns to the height it started at: every control we know reads that alike.
    fn cycle_key(&self, from: Vec3, to: Vec3, r: f64, retract: f64, cycle: DrillCycle, feed: f64) -> std::result::Result<CycleKey, String> {
        let c = &self.post.cycles;
        let (code, on) = match cycle {
            DrillCycle::Drill { dwell } if dwell > 0.0 => ("G82", c.dwell),
            DrillCycle::Drill { .. } => ("G81", c.drill),
            DrillCycle::Peck { .. } => ("G83", c.peck),
            DrillCycle::ChipBreak { .. } => ("G73", c.chip_break),
            DrillCycle::Tap { .. } => ("G84", c.tap),
        };
        if !on {
            return Err(format!("the {} post does not write {code}", self.post.id));
        }
        let (z_text, z_back) = self.length(to.z);
        let (r_text, r_back) = self.length(r);
        let (retract_text, _) = self.length(retract);
        if !(r_back > z_back) {
            return Err("its R plane and bottom round to the same height".into());
        }
        let (mode, height) = if retract_text == r_text {
            ("G99", None)
        } else if self.length(from.z).0 == retract_text {
            ("G98", Some(retract_text))
        } else {
            return Err("it returns to a height that is neither its R plane nor where it started".into());
        };
        let mut tail = vec![format!("Z{z_text}"), format!("R{r_text}")];
        match cycle {
            DrillCycle::Peck { peck } | DrillCycle::ChipBreak { peck } => {
                let (q_text, q_back) = self.length(peck);
                if !(q_back > 0.0) {
                    return Err("its peck rounds to 0".into());
                }
                tail.push(format!("Q{q_text}"));
            }
            // Never stripped, as a dwell's P (see `Post::dwell`).
            DrillCycle::Drill { dwell } if dwell > 0.0 => tail.push(format!("P{}", format_decimal(dwell, self.post.numbers.dwell, false))),
            DrillCycle::Drill { .. } | DrillCycle::Tap { .. } => {}
        }
        // A tap's feed says its pitch, with the spindle speed: two more decimals than other
        // feeds (Haas's workbook writes F32.1429 for 14 threads an inch at 450 rev/min), and
        // the two as written must still be the pitch.
        let decimals =
            if matches!(cycle, DrillCycle::Tap { .. }) { self.feed_decimals.saturating_add(TAP_FEED_DECIMALS).min(6) } else { self.feed_decimals };
        let feed_text = format_decimal(self.ctx.units.from_mm(feed), decimals, self.post.numbers.strip_trailing_zeros);
        if let DrillCycle::Tap { pitch } = cycle {
            let written = feed_text.parse::<f64>().ok().map(|f| self.ctx.units.to_mm(f)).zip(self.spindle_speed).map(|(feed, speed)| feed / speed);
            if !written.is_some_and(|written| (written - pitch).abs() <= TAP_PITCH_SLOP * pitch) {
                return Err(format!(
                    "its feed and the spindle speed as written say a pitch of {} mm, not {pitch}: give the tap a spindle speed the post writes exactly",
                    written.unwrap_or(f64::NAN)
                ));
            }
        }
        tail.push(format!("F{feed_text}"));
        Ok(CycleKey { head: [mode.to_string(), code.to_string()], tail, height })
    }

    /// One hole: one line of a canned cycle (starting it when its words differ from the one in
    /// force), or plain moves.
    fn drill(&mut self, index: usize, to: Vec3, r: f64, retract: f64, cycle: DrillCycle, feed: f64) -> Result<()> {
        let from = self.at;
        let key = match self.cycle_key(from, to, r, retract, cycle, feed) {
            Ok(key) => key,
            // A tapped hole is cut in step with the spindle, which plain moves can't say.
            Err(why) if matches!(cycle, DrillCycle::Tap { .. }) => {
                return Err(Error::BadMove {
                    index,
                    reason: format!("a tapped hole can't be written as plain moves, and not as a cycle because {why}"),
                });
            }
            Err(why) => {
                self.end_cycle();
                self.notes.push(format!("move {index}: hole written as plain moves because {why}"));
                let mut moves = Vec::new();
                cycle::expand(from, to, r, retract, cycle, feed, &self.post.cycles.motion, &mut moves);
                for m in &moves {
                    self.one(index, m)?;
                }
                return Ok(());
            }
        };
        // X and Y on every line, so a hole right above the last still drills.
        let (x_text, x_back) = self.length(to.x);
        let (y_text, y_back) = self.length(to.y);
        let xy = [format!("X{x_text}"), format!("Y{y_text}")];
        if self.cycle.as_ref() == Some(&key) {
            self.emit(xy.join(" "));
        } else {
            self.end_cycle();
            let line: Vec<String> = key.head.iter().cloned().chain(xy).chain(key.tail.iter().cloned()).collect();
            self.emit(line.join(" "));
            self.feed = key.tail.last().and_then(|f| f.strip_prefix('F')).map(str::to_string);
            self.motion = None;
            self.cycle = Some(key);
        }
        // The control's Z after a hole is its retract height, not the Z word.
        self.out = [Some((x_text, x_back)), Some((y_text, y_back)), None];
        self.written.push(Move::Drill { to, r, retract, cycle, feed });
        self.at = Vec3 { x: to.x, y: to.y, z: retract };
        Ok(())
    }

    fn linear(&mut self, index: usize, to: Vec3, feed: f64) {
        let axes = self.axis_words(to, false);
        if axes.is_empty() {
            self.notes.push(format!("move {index}: a cut too short to write was left out"));
            self.at = to;
            return;
        }
        let mut words: Vec<String> = self.motion_word("G1").into_iter().collect();
        words.extend(axes);
        words.extend(self.feed_word(feed));
        self.emit(words.join(" "));
        self.written.push(Move::Linear { to, feed });
        self.at = to;
    }

    fn arc(&mut self, index: usize, to: Vec3, center: Vec3, plane: Plane, direction: ArcDirection, feed: f64) -> Result<()> {
        let geom = ArcGeom::new(self.at, to, center, plane, direction).map_err(|reason| Error::BadMove { index, reason })?;
        let (_, _, w0) = plane.split(self.at);
        let (_, _, w1) = plane.split(to);
        let a = &self.post.arcs;
        let why_lines = if !a.planes.contains(&plane) {
            Some("its plane is not in the post's arc planes")
        } else if !a.helical && (w1 - w0).abs() > 1e-9 {
            Some("the post takes no helical arcs")
        } else if geom.radius < a.min_radius {
            Some("its radius is below the post's smallest")
        } else {
            None
        };
        if let Some(why) = why_lines {
            return self.arc_as_lines(index, &geom, feed, why);
        }
        let pieces = (geom.sweep.abs() / a.max_sweep_degrees.to_radians() - 1e-9).ceil().max(1.0);
        let pieces = pieces.min(MAX_ARC_SEGMENTS as f64) as usize;
        // Check every piece writes and reads back as the arc meant before writing any of it.
        let mut planned = Vec::with_capacity(pieces);
        let saved = self.out.clone();
        let mut start = self.at;
        for k in 1..=pieces {
            let end = if k == pieces { to } else { geom.point(k as f64 / pieces as f64) };
            let piece_sweep = geom.sweep / pieces as f64;
            match self.arc_words(start, end, center, plane, direction, piece_sweep) {
                Some(words) => planned.push((end, words)),
                None => {
                    self.out = saved;
                    return self.arc_as_lines(index, &geom, feed, "its rounded numbers would not read back as this arc");
                }
            }
            start = end;
        }
        if self.plane != Some(plane) {
            self.emit(
                match plane {
                    Plane::Xy => "G17",
                    Plane::Zx => "G18",
                    Plane::Yz => "G19",
                }
                .into(),
            );
            self.plane = Some(plane);
        }
        let code = if direction == ArcDirection::Clockwise { "G2" } else { "G3" };
        for (end, (axis_words, offsets)) in planned {
            let mut words: Vec<String> = self.motion_word(code).into_iter().collect();
            words.extend(axis_words);
            words.extend(offsets);
            words.extend(self.feed_word(feed));
            self.emit(words.join(" "));
            self.written.push(Move::Arc { to: end, center, plane, direction, feed });
            self.at = end;
        }
        Ok(())
    }

    /// The words of one arc piece from `start` to `end`, or `None` when its rounded numbers would
    /// break the reader's radius rule or read as a different sweep (a tiny arc whose ends round
    /// together reads as a full circle). Updates the written position only when it returns words.
    fn arc_words(
        &mut self,
        start: Vec3,
        end: Vec3,
        center: Vec3,
        plane: Plane,
        direction: ArcDirection,
        sweep: f64,
    ) -> Option<(Vec<String>, Vec<String>)> {
        // Where the reader thinks the tool is: the last value written, or (before any) the exact start.
        let back = |k: usize, exact: f64, me: &Self| me.out.get(k).and_then(|o| o.as_ref().map(|(_, b)| *b)).unwrap_or(exact);
        let start_back = Vec3 { x: back(0, start.x, self), y: back(1, start.y, self), z: back(2, start.z, self) };
        let end_back = Vec3 { x: self.length(end.x).1, y: self.length(end.y).1, z: self.length(end.z).1 };
        let (su, sv, _) = plane.split(start_back);
        let (cu, cv, _) = plane.split(center);
        let (ou_text, ou_back) = self.length(cu - su);
        let (ov_text, ov_back) = self.length(cv - sv);
        let center_back = plane.join(su + ou_back, sv + ov_back, 0.0);
        let read = ArcGeom::new(start_back, end_back, center_back, plane, direction).ok()?;
        if !((read.sweep - sweep).abs() <= 1e-3 * TAU) {
            return None;
        }
        // In-plane axes are always written (a full circle has no changed axis); the normal axis when it changes.
        let (in_plane, normal): ([usize; 2], usize) = match plane {
            Plane::Xy => ([0, 1], 2),
            Plane::Zx => ([2, 0], 1),
            Plane::Yz => ([1, 2], 0),
        };
        let letters = ['X', 'Y', 'Z'];
        let ends = [end.x, end.y, end.z];
        let mut axis_words = Vec::new();
        for k in [0usize, 1, 2] {
            let (text, back) = self.length(ends.get(k).copied().unwrap_or(0.0));
            let changed = self.out.get(k).map(|o| o.as_ref().map(|(t, _)| t != &text).unwrap_or(true)).unwrap_or(true);
            if in_plane.contains(&k) || (k == normal && changed) {
                axis_words.push(format!("{}{text}", letters.get(k).copied().unwrap_or('X')));
                if let Some(slot) = self.out.get_mut(k) {
                    *slot = Some((text, back));
                }
            }
        }
        let offset_letters = match plane {
            Plane::Xy => ['I', 'J'],
            Plane::Zx => ['K', 'I'],
            Plane::Yz => ['J', 'K'],
        };
        let offsets = vec![format!("{}{ou_text}", offset_letters[0]), format!("{}{ov_text}", offset_letters[1])];
        Some((axis_words, offsets))
    }

    fn arc_as_lines(&mut self, index: usize, geom: &ArcGeom, feed: f64, why: &str) -> Result<()> {
        let tol = self.post.arcs.linearize_tolerance;
        let per = if tol >= geom.radius { std::f64::consts::FRAC_PI_2 } else { 2.0 * (1.0 - tol / geom.radius).acos() };
        let n = (geom.sweep.abs() / per).ceil().max(1.0);
        if !(n <= MAX_ARC_SEGMENTS as f64) {
            return Err(Error::BadMove { index, reason: format!("an arc that needs more than {MAX_ARC_SEGMENTS} lines ({why})") });
        }
        let n = n as usize;
        self.notes.push(format!("move {index}: arc written as {n} lines because {why}"));
        for k in 1..=n {
            self.linear(index, geom.point(k as f64 / n as f64), feed);
        }
        Ok(())
    }
}

/// Checks the read-back moves against the written ones: positions within `tol`, centres within
/// twice that, feeds and speeds within half their last digit, arcs of the same length.
fn compare(written: &Toolpath, read: &Toolpath, tol: f64, w: &Writer<'_>) -> Result<()> {
    let mismatch = |index: usize, message: String| Err(Error::PostMismatch { index, message });
    if written.moves.len() != read.moves.len() {
        return mismatch(written.moves.len().min(read.moves.len()), format!("{} moves written, {} read back", written.moves.len(), read.moves.len()));
    }
    let feed_tol = 0.5 * w.ctx.units.to_mm(10f64.powi(-i32::from(w.feed_decimals))) + 1e-9;
    let speed_tol = 0.5 * 10f64.powi(-i32::from(w.post.numbers.speed)) + 1e-9;
    let dwell_tol = 0.5 * 10f64.powi(-i32::from(w.post.numbers.dwell)) + 1e-9;
    let near = |a: Vec3, b: Vec3, t: f64| (a.x - b.x).abs() <= t && (a.y - b.y).abs() <= t && (a.z - b.z).abs() <= t;
    let tol = tol + 1e-9;
    let (mut at_w, mut at_r) = (written.start, read.start);
    for (index, (a, b)) in written.moves.iter().zip(&read.moves).enumerate() {
        let ok = match (*a, *b) {
            (Move::Rapid { to: p }, Move::Rapid { to: q }) => near(p, q, tol),
            (Move::Linear { to: p, feed: f }, Move::Linear { to: q, feed: g }) => near(p, q, tol) && (f - g).abs() <= feed_tol,
            (
                Move::Arc { to: p, center: c, plane, direction, feed: f },
                Move::Arc { to: q, center: d, plane: plane2, direction: direction2, feed: g },
            ) => {
                let (cu, cv, _) = plane.split(c);
                let (du, dv, _) = plane.split(d);
                let lengths = ArcGeom::new(at_w, p, c, plane, direction).ok().zip(ArcGeom::new(at_r, q, d, plane2, direction2).ok());
                plane == plane2
                    && direction == direction2
                    && near(p, q, tol)
                    && (cu - du).abs() <= 2.0 * tol
                    && (cv - dv).abs() <= 2.0 * tol
                    && (f - g).abs() <= feed_tol
                    && lengths.is_some_and(|(x, y)| (x.length() - y.length()).abs() <= 8.0 * tol)
            }
            (Move::Dwell { seconds: s }, Move::Dwell { seconds: t }) => (s - t).abs() <= dwell_tol,
            (Move::Drill { to: p, r: r1, retract: t1, cycle: c1, feed: f }, Move::Drill { to: q, r: r2, retract: t2, cycle: c2, feed: g }) => {
                let same_cycle = match (c1, c2) {
                    (DrillCycle::Drill { dwell: a }, DrillCycle::Drill { dwell: b }) => (a - b).abs() <= dwell_tol,
                    (DrillCycle::Peck { peck: a }, DrillCycle::Peck { peck: b })
                    | (DrillCycle::ChipBreak { peck: a }, DrillCycle::ChipBreak { peck: b }) => (a - b).abs() <= tol,
                    (DrillCycle::Tap { pitch: a }, DrillCycle::Tap { pitch: b }) => (a - b).abs() <= TAP_PITCH_SLOP * a,
                    _ => false,
                };
                near(p, q, tol) && (r1 - r2).abs() <= tol && (t1 - t2).abs() <= tol && (f - g).abs() <= feed_tol && same_cycle
            }
            (Move::Spindle { rpm: r, direction: d }, Move::Spindle { rpm: s, direction: e }) => d == e && (r - s).abs() <= speed_tol,
            (x, y) => x == y,
        };
        if !ok {
            return mismatch(index, format!("wrote {a:?} but it reads back as {b:?}"));
        }
        at_w = a.end(at_w);
        at_r = b.end(at_r);
    }
    Ok(())
}

#[cfg(test)]
#[path = "post_tests.rs"]
mod tests;
