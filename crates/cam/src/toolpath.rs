//! A toolpath: the moves a machine makes for one operation, in the work coordinate system,
//! with the tool tip as the controlled point. Toolpaths are derived from the design and the
//! operation's parameters, never saved in the design.

use std::f64::consts::{FRAC_PI_2, TAU};

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// The most moves one toolpath may hold.
pub const MAX_MOVES: usize = 10_000_000;
/// The highest feed accepted, in mm/min (1 km/min).
pub const MAX_FEED: f64 = 1_000_000.0;
/// The highest spindle speed accepted, in rev/min.
pub const MAX_SPINDLE_RPM: f64 = 200_000.0;
/// The longest dwell accepted, in seconds.
pub const MAX_DWELL_SECONDS: f64 = 3_600.0;
/// The highest tool number accepted.
pub const MAX_TOOL_NUMBER: u32 = 99_999;
/// How far the start and end of an arc may differ in distance from its centre, in mm. This is
/// RS274/NGC's limit for centre-format arcs (NIST IR 6556 §3.5.3.2), so every arc we accept a
/// controller following it accepts too.
pub const ARC_RADIUS_TOLERANCE: f64 = 0.002;
/// The smallest arc radius accepted, in mm.
const MIN_ARC_RADIUS: f64 = 1e-3;
/// Start and end closer than this in the arc's plane, in mm, make a full circle.
const FULL_CIRCLE_CHORD: f64 = 1e-6;

/// The plane an arc turns in, named by its two axes (G17, G18, G19). An arc is counter-clockwise
/// when seen from the positive end of the plane's normal axis.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Plane {
    /// The XY plane, normal Z.
    #[default]
    Xy,
    /// The ZX plane, normal Y.
    Zx,
    /// The YZ plane, normal X.
    Yz,
}

impl Plane {
    /// `p` as (u, v, w): two in-plane coordinates, ordered so that u × v points along the
    /// normal, then the coordinate along the normal.
    pub(crate) fn split(self, p: Vec3) -> (f64, f64, f64) {
        match self {
            Plane::Xy => (p.x, p.y, p.z),
            Plane::Zx => (p.z, p.x, p.y),
            Plane::Yz => (p.y, p.z, p.x),
        }
    }

    /// The point with in-plane coordinates (u, v) and normal coordinate w.
    pub(crate) fn join(self, u: f64, v: f64, w: f64) -> Vec3 {
        match self {
            Plane::Xy => Vec3 { x: u, y: v, z: w },
            Plane::Zx => Vec3 { x: v, y: w, z: u },
            Plane::Yz => Vec3 { x: w, y: u, z: v },
        }
    }
}

/// Which way an arc turns, seen from the positive end of its plane's normal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArcDirection {
    Clockwise,
    CounterClockwise,
}

/// Which way the spindle turns, seen from above the tool looking down its axis at the work.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpindleDirection {
    Clockwise,
    CounterClockwise,
}

/// The coolant mode.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Coolant {
    Off,
    Flood,
    Mist,
}

/// One step of a toolpath. Positions are in mm in the work coordinate system and feeds in mm/min.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Move {
    /// Move at the machine's rapid rate to `to`. Not necessarily a straight line on the machine.
    Rapid { to: Vec3 },
    /// Cut in a straight line to `to` at `feed`.
    Linear { to: Vec3, feed: f64 },
    /// Cut along a circular arc (or a helix, when `to` differs along the plane's normal) to `to`
    /// at `feed`, around `center`. The centre's coordinate along the normal is ignored. When
    /// start and end coincide in the plane the arc is a full circle.
    Arc { to: Vec3, center: Vec3, plane: Plane, direction: ArcDirection, feed: f64 },
    /// Stay still for `seconds`.
    Dwell { seconds: f64 },
    /// Change to tool number `tool`.
    ToolChange { tool: u32 },
    /// Start or change the spindle.
    Spindle { rpm: f64, direction: SpindleDirection },
    /// Stop the spindle.
    SpindleStop,
    /// Set the coolant.
    Coolant { mode: Coolant },
}

impl Move {
    /// Where the tool is after this move, from `from`.
    pub fn end(&self, from: Vec3) -> Vec3 {
        match *self {
            Move::Rapid { to } | Move::Linear { to, .. } | Move::Arc { to, .. } => to,
            _ => from,
        }
    }
}

/// A toolpath: where the tool starts and the moves it makes from there.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Toolpath {
    pub start: Vec3,
    pub moves: Vec<Move>,
}

/// Measures of a toolpath. Lengths in mm, times in minutes, bounds over every point the tool
/// tip passes through (arcs included).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Stats {
    /// Length cut at feed (lines and arcs).
    pub feed_length: f64,
    /// Length moved at rapid, measured as straight lines.
    pub rapid_length: f64,
    /// Time spent cutting: each feed move's length over its feed.
    pub feed_minutes: f64,
    /// Time spent dwelling.
    pub dwell_minutes: f64,
    /// Number of tool changes.
    pub tool_changes: usize,
    /// The lowest corner of the box around the path.
    pub min: Vec3,
    /// The highest corner of the box around the path.
    pub max: Vec3,
}

impl Stats {
    /// The estimated run time in minutes when rapids move at `rapid_rate` mm/min. Acceleration
    /// is ignored, so real machines take longer.
    pub fn minutes(&self, rapid_rate: f64) -> f64 {
        let rapid = if rapid_rate > 0.0 && rapid_rate.is_finite() { self.rapid_length / rapid_rate } else { 0.0 };
        self.feed_minutes + self.dwell_minutes + rapid
    }
}

/// An arc's geometry, worked out from its start.
struct ArcGeom {
    plane: Plane,
    /// Centre in the plane.
    cu: f64,
    cv: f64,
    radius: f64,
    /// Angle of the start around the centre.
    start_angle: f64,
    /// Signed sweep: positive counter-clockwise.
    sweep: f64,
    /// Start and end along the normal.
    w0: f64,
    w1: f64,
}

impl ArcGeom {
    fn new(from: Vec3, to: Vec3, center: Vec3, plane: Plane, direction: ArcDirection) -> std::result::Result<Self, String> {
        let (su, sv, w0) = plane.split(from);
        let (eu, ev, w1) = plane.split(to);
        let (cu, cv, _) = plane.split(center);
        let r0 = (su - cu).hypot(sv - cv);
        let r1 = (eu - cu).hypot(ev - cv);
        if !(r0 >= MIN_ARC_RADIUS) {
            return Err(format!("arc radius {r0} mm is below {MIN_ARC_RADIUS} mm"));
        }
        if !((r0 - r1).abs() <= ARC_RADIUS_TOLERANCE) {
            return Err(format!("arc start is {r0} mm from the centre but its end is {r1} mm (at most {ARC_RADIUS_TOLERANCE} mm apart)"));
        }
        let start_angle = (sv - cv).atan2(su - cu);
        let end_angle = (ev - cv).atan2(eu - cu);
        let full = (eu - su).hypot(ev - sv) <= FULL_CIRCLE_CHORD;
        let ccw = if full { TAU } else { (end_angle - start_angle).rem_euclid(TAU) };
        let sweep = match direction {
            ArcDirection::CounterClockwise => ccw,
            ArcDirection::Clockwise if full => -TAU,
            ArcDirection::Clockwise => -(TAU - ccw).rem_euclid(TAU),
        };
        Ok(Self { plane, cu, cv, radius: 0.5 * (r0 + r1), start_angle, sweep, w0, w1 })
    }

    fn length(&self) -> f64 {
        (self.sweep.abs() * self.radius).hypot(self.w1 - self.w0)
    }

    /// The points where the arc reaches its furthest along the in-plane axes (at most four).
    fn extremes(&self) -> impl Iterator<Item = Vec3> + '_ {
        (0..4).filter_map(move |k| {
            let theta = f64::from(k) * FRAC_PI_2;
            // How far (in the arc's own direction) theta is past the start.
            let past = if self.sweep >= 0.0 { (theta - self.start_angle).rem_euclid(TAU) } else { (self.start_angle - theta).rem_euclid(TAU) };
            (past <= self.sweep.abs()).then(|| {
                let t = if self.sweep == 0.0 { 0.0 } else { past / self.sweep.abs() };
                let w = self.w0 + t * (self.w1 - self.w0);
                self.plane.join(self.cu + self.radius * theta.cos(), self.cv + self.radius * theta.sin(), w)
            })
        })
    }
}

fn finite_in_range(p: Vec3) -> bool {
    [p.x, p.y, p.z].iter().all(|c| c.abs() <= MAX_COORDINATE)
}

fn check_feed(feed: f64) -> std::result::Result<(), String> {
    if feed > 0.0 && feed <= MAX_FEED { Ok(()) } else { Err(format!("feed must be above 0 and at most {MAX_FEED} mm/min, not {feed}")) }
}

impl Toolpath {
    /// Every move with the point it starts from.
    fn walk(&self) -> impl Iterator<Item = (Vec3, &Move)> {
        self.moves.iter().scan(self.start, |at, m| {
            let from = *at;
            *at = m.end(from);
            Some((from, m))
        })
    }

    /// Checks every number in the toolpath: coordinates finite and within [`MAX_COORDINATE`],
    /// feeds, speeds, dwells and tool numbers in range, arcs with matching start and end radii.
    pub fn validate(&self) -> Result<()> {
        if self.moves.len() > MAX_MOVES {
            return Err(Error::TooManyMoves { count: self.moves.len(), max: MAX_MOVES });
        }
        if !finite_in_range(self.start) {
            return Err(Error::BadValue(format!("the start point must be finite and within {MAX_COORDINATE} mm")));
        }
        for (index, (from, m)) in self.walk().enumerate() {
            Self::check_move(from, m).map_err(|reason| Error::BadMove { index, reason })?;
        }
        Ok(())
    }

    fn check_move(from: Vec3, m: &Move) -> std::result::Result<(), String> {
        let check_point = |p: Vec3| {
            if finite_in_range(p) { Ok(()) } else { Err(format!("point must be finite and within {MAX_COORDINATE} mm")) }
        };
        match *m {
            Move::Rapid { to } => check_point(to),
            Move::Linear { to, feed } => {
                check_point(to)?;
                check_feed(feed)
            }
            Move::Arc { to, center, plane, direction, feed } => {
                check_point(to)?;
                check_point(center)?;
                check_feed(feed)?;
                ArcGeom::new(from, to, center, plane, direction).map(|_| ())
            }
            Move::Dwell { seconds } if (0.0..=MAX_DWELL_SECONDS).contains(&seconds) => Ok(()),
            Move::Dwell { seconds } => Err(format!("dwell must be 0 to {MAX_DWELL_SECONDS} s, not {seconds}")),
            Move::ToolChange { tool } if (1..=MAX_TOOL_NUMBER).contains(&tool) => Ok(()),
            Move::ToolChange { tool } => Err(format!("tool number must be 1 to {MAX_TOOL_NUMBER}, not {tool}")),
            Move::Spindle { rpm, .. } if rpm > 0.0 && rpm <= MAX_SPINDLE_RPM => Ok(()),
            Move::Spindle { rpm, .. } => Err(format!("spindle speed must be above 0 and at most {MAX_SPINDLE_RPM} rpm, not {rpm}")),
            Move::SpindleStop | Move::Coolant { .. } => Ok(()),
        }
    }

    /// Lengths, times and bounds of a valid toolpath ([`Toolpath::validate`] first).
    pub fn stats(&self) -> Result<Stats> {
        self.validate()?;
        let mut s =
            Stats { feed_length: 0.0, rapid_length: 0.0, feed_minutes: 0.0, dwell_minutes: 0.0, tool_changes: 0, min: self.start, max: self.start };
        for (index, (from, m)) in self.walk().enumerate() {
            match *m {
                Move::Rapid { to } => {
                    s.rapid_length += distance(from, to);
                    include(to, &mut s);
                }
                Move::Linear { to, feed } => {
                    let len = distance(from, to);
                    s.feed_length += len;
                    s.feed_minutes += len / feed;
                    include(to, &mut s);
                }
                Move::Arc { to, center, plane, direction, feed } => {
                    let arc = ArcGeom::new(from, to, center, plane, direction).map_err(|reason| Error::BadMove { index, reason })?;
                    let len = arc.length();
                    s.feed_length += len;
                    s.feed_minutes += len / feed;
                    for p in arc.extremes() {
                        include(p, &mut s);
                    }
                    include(to, &mut s);
                }
                Move::Dwell { seconds } => s.dwell_minutes += seconds / 60.0,
                Move::ToolChange { .. } => s.tool_changes += 1,
                Move::Spindle { .. } | Move::SpindleStop | Move::Coolant { .. } => {}
            }
        }
        Ok(s)
    }
}

fn include(p: Vec3, s: &mut Stats) {
    s.min = Vec3 { x: s.min.x.min(p.x), y: s.min.y.min(p.y), z: s.min.z.min(p.z) };
    s.max = Vec3 { x: s.max.x.max(p.x), y: s.max.y.max(p.y), z: s.max.z.max(p.z) };
}

fn distance(a: Vec3, b: Vec3) -> f64 {
    ((b.x - a.x).powi(2) + (b.y - a.y).powi(2) + (b.z - a.z).powi(2)).sqrt()
}

#[cfg(test)]
#[path = "toolpath_tests.rs"]
mod tests;
