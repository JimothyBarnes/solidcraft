//! Drilling cycles (decision 0008): a hole is one [`Move::Drill`], and this module says how it
//! runs as plain moves (NIST IR 6556 §3.5.16) with the distances a control keeps in its own
//! settings rather than in the program ([`CycleMotion`]).
//!
//! A hole starts at or above its R plane. From there it rapids across at the height it is at to
//! above the hole, rapids down to R, cuts to the bottom as its [`DrillCycle`] says, and rapids up
//! to its retract height (R or above). Where controls disagree (what G98 returns to, where G83
//! retracts between pecks) the choice is the post's data, never a guess here.

use serde::{Deserialize, Serialize};
use solidcore_record::Vec3;

use crate::toolpath::{MAX_DWELL_SECONDS, Move, SpindleDirection};
use crate::units::MAX_COORDINATE;

/// The most pecks one hole may take.
pub const MAX_PECKS: usize = 10_000;
/// The smallest peck accepted, in mm.
pub const MIN_PECK: f64 = 0.001;
/// The least a hole's bottom may lie below its R plane, in mm.
pub const MIN_HOLE_DEPTH: f64 = 0.001;
/// The largest of a control's cycle distances ([`CycleMotion`]), in mm.
pub const MAX_CYCLE_DISTANCE: f64 = 25.0;
/// How far below its R plane a hole may start and still count as starting at R, in mm.
const START_SLOP: f64 = 1e-9;
/// The finest and the coarsest thread a tap may cut, in mm a turn.
pub const MIN_PITCH: f64 = 0.01;
pub const MAX_PITCH: f64 = 100.0;
/// How far a tapped hole's feed may be from its pitch times the spindle speed, as a share of it:
/// a tap fed out of step with its spindle breaks, or cuts another thread.
pub const TAP_FEED_SLOP: f64 = 1e-6;

/// How a hole is cut from its R plane to its bottom.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum DrillCycle {
    /// Feed to the bottom, stay there `dwell` seconds (0 for none), rapid out: G81, or G82 with a
    /// dwell.
    Drill { dwell: f64 },
    /// Feed down `peck` at a time, rapid out after each peck to clear the chips and back down to
    /// just above the depth reached: G83.
    Peck { peck: f64 },
    /// Feed down `peck` at a time, backing off a little after each peck to break the chip: G73.
    ChipBreak { peck: f64 },
    /// Feed down in step with the spindle, `pitch` mm a turn, reverse the spindle at the bottom
    /// and feed back out to R in step with it: G84, a right-hand thread. The hole's feed is the
    /// pitch times the spindle speed ([`check_tap`]).
    Tap { pitch: f64 },
    /// Feed to the bottom, stay there `dwell` seconds (0 for none), and feed back out to R: G85,
    /// or G89 with a dwell (NIST IR 6556 3.5.16.6 and 3.5.16.10). For a reamer or a boring
    /// tool: a rapid out would drag a line down the finished wall.
    Ream { dwell: f64 },
}

/// What G98 returns a hole to (NIST IR 6556 §3.5.20 and Haas's workbook differ, decision 0008).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum G98Rule {
    /// The height the tool was at when the line that drills the hole started (NIST).
    #[default]
    BlockStart,
    /// The height the tool was at when the cycle was first programmed (Haas).
    CycleStart,
}

/// Where a peck cycle (G83) rapids out to between pecks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PeckRetract {
    /// The hole's retract height (NIST: "clear Z").
    #[default]
    ClearZ,
    /// The R plane, plus [`CycleMotion::above_r`] (Haas).
    RPlane,
}

/// The distances a control uses to run drilling cycles that are not in the program, in mm. They
/// set how the pecks are drawn and simulated, and are written out when a hole is written as plain
/// moves. On a Haas they are Setting 22 (`peck_clearance` and `chip_break_back_off`) and Setting
/// 52 (`above_r`), changed at the control.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CycleMotion {
    /// How far above the depth reached a peck cycle comes back down to at rapid.
    pub peck_clearance: f64,
    /// How far a chip-break cycle backs off after each peck.
    pub chip_break_back_off: f64,
    /// Where a peck cycle rapids out to between pecks.
    pub peck_retract: PeckRetract,
    /// How far above R a peck cycle rapids out to, when `peck_retract` is the R plane.
    pub above_r: f64,
}

impl Default for CycleMotion {
    fn default() -> Self {
        Self { peck_clearance: 0.5, chip_break_back_off: 0.5, peck_retract: PeckRetract::ClearZ, above_r: 0.0 }
    }
}

impl CycleMotion {
    /// Checks every distance: clearance and back-off above 0, `above_r` 0 or more, all at most
    /// [`MAX_CYCLE_DISTANCE`].
    pub fn check(&self) -> std::result::Result<(), String> {
        let positive = |v: f64| v > 0.0 && v <= MAX_CYCLE_DISTANCE;
        if !positive(self.peck_clearance) || !positive(self.chip_break_back_off) {
            return Err(format!("peck_clearance and chip_break_back_off must be above 0 and at most {MAX_CYCLE_DISTANCE} mm"));
        }
        if !(0.0..=MAX_CYCLE_DISTANCE).contains(&self.above_r) {
            return Err(format!("above_r must be 0 to {MAX_CYCLE_DISTANCE} mm"));
        }
        Ok(())
    }
}

/// Checks one hole that starts at `from`: the bottom `to`, the R plane `r` and the retract height
/// in range and in order, the cycle's numbers in range.
pub(crate) fn check(from: Vec3, to: Vec3, r: f64, retract: f64, cycle: DrillCycle) -> std::result::Result<(), String> {
    if !(r.abs() <= MAX_COORDINATE) || !(retract.abs() <= MAX_COORDINATE) {
        return Err(format!("R and retract heights must be finite and within {MAX_COORDINATE} mm"));
    }
    if !(r - to.z >= MIN_HOLE_DEPTH) {
        return Err(format!("the R plane ({r} mm) must be at least {MIN_HOLE_DEPTH} mm above the hole's bottom ({} mm)", to.z));
    }
    if !(retract >= r) {
        return Err(format!("a hole retracts to its R plane or above, not to {retract} mm (R is {r} mm)"));
    }
    if !(from.z >= r - START_SLOP) {
        return Err(format!("a hole starts at or above its R plane ({r} mm), not at {} mm", from.z));
    }
    match cycle {
        DrillCycle::Drill { dwell } if (0.0..=MAX_DWELL_SECONDS).contains(&dwell) => Ok(()),
        DrillCycle::Drill { dwell } => Err(format!("dwell must be 0 to {MAX_DWELL_SECONDS} s, not {dwell}")),
        DrillCycle::Peck { peck } | DrillCycle::ChipBreak { peck } => {
            if !(MIN_PECK..=MAX_COORDINATE).contains(&peck) {
                return Err(format!("a peck must be at least {MIN_PECK} mm, not {peck}"));
            }
            if !(pecks(r, to.z, peck) <= MAX_PECKS as f64) {
                return Err(format!("a hole of {} mm in pecks of {peck} mm takes more than {MAX_PECKS} pecks", r - to.z));
            }
            Ok(())
        }
        DrillCycle::Tap { pitch } if (MIN_PITCH..=MAX_PITCH).contains(&pitch) => Ok(()),
        DrillCycle::Tap { pitch } => Err(format!("a tap's pitch must be {MIN_PITCH} to {MAX_PITCH} mm, not {pitch}")),
        DrillCycle::Ream { dwell } if (0.0..=MAX_DWELL_SECONDS).contains(&dwell) => Ok(()),
        DrillCycle::Ream { dwell } => Err(format!("dwell must be 0 to {MAX_DWELL_SECONDS} s, not {dwell}")),
    }
}

/// Checks that a tapped hole of `pitch` mm a turn fed at `feed` mm/min is in step with the
/// spindle: turning clockwise (`spindle` is its speed and direction, `None` when it is stopped)
/// at the speed that makes the feed one pitch a turn.
pub(crate) fn check_tap(pitch: f64, feed: f64, spindle: Option<(f64, SpindleDirection)>) -> std::result::Result<(), String> {
    let Some((rpm, direction)) = spindle else { return Err("a tapped hole with the spindle stopped: start it before the hole".into()) };
    if direction != SpindleDirection::Clockwise {
        return Err("a tapped hole (G84) is cut with the spindle turning clockwise: left-hand tapping is not supported yet".into());
    }
    let in_step = pitch * rpm;
    if !((feed - in_step).abs() <= TAP_FEED_SLOP * in_step) {
        return Err(format!(
            "a tapped hole's feed must be its pitch times the spindle speed: {pitch} mm a turn at {rpm} rev/min is {in_step} mm/min, not {feed}"
        ));
    }
    Ok(())
}

/// How many pecks of `peck` cut from `r` down to `bottom`: at least one, and a last peck shorter
/// than a millionth of a peck is not counted (it would only be rounding).
fn pecks(r: f64, bottom: f64, peck: f64) -> f64 {
    ((r - bottom) / peck - 1e-6).ceil().max(1.0)
}

/// The depth after peck `k` of `n`: each measured from R, so rounding does not build up, and the
/// last one exactly at the bottom.
fn depth_after(k: usize, n: usize, r: f64, bottom: f64, peck: f64) -> f64 {
    if k >= n { bottom } else { (r - k as f64 * peck).max(bottom) }
}

/// The plain moves one checked hole makes from `from`, appended to `out`.
pub(crate) fn expand(from: Vec3, to: Vec3, r: f64, retract: f64, cycle: DrillCycle, feed: f64, motion: &CycleMotion, out: &mut Vec<Move>) {
    let at = |z: f64| Vec3 { x: to.x, y: to.y, z };
    if from.x != to.x || from.y != to.y {
        out.push(Move::Rapid { to: at(from.z) });
    }
    if from.z > r {
        out.push(Move::Rapid { to: at(r) });
    }
    match cycle {
        DrillCycle::Drill { dwell } => {
            out.push(Move::Linear { to: at(to.z), feed });
            if dwell > 0.0 {
                out.push(Move::Dwell { seconds: dwell });
            }
        }
        DrillCycle::Peck { peck } => {
            let out_to = match motion.peck_retract {
                PeckRetract::ClearZ => retract,
                PeckRetract::RPlane => r + motion.above_r,
            };
            let n = pecks(r, to.z, peck) as usize;
            for k in 1..=n {
                let depth = depth_after(k, n, r, to.z, peck);
                out.push(Move::Linear { to: at(depth), feed });
                if k == n {
                    break;
                }
                out.push(Move::Rapid { to: at(out_to) });
                let back = depth + motion.peck_clearance;
                if back < out_to {
                    out.push(Move::Rapid { to: at(back) });
                }
            }
        }
        DrillCycle::ChipBreak { peck } => {
            let n = pecks(r, to.z, peck) as usize;
            for k in 1..=n {
                let depth = depth_after(k, n, r, to.z, peck);
                out.push(Move::Linear { to: at(depth), feed });
                if k == n {
                    break;
                }
                out.push(Move::Rapid { to: at((depth + motion.chip_break_back_off).min(r)) });
            }
        }
        DrillCycle::Tap { .. } => {
            // In, and back out to R at the same feed: the control reverses the spindle at the
            // bottom and turns it forward again at R. Above R the tool is clear of the thread.
            out.push(Move::Linear { to: at(to.z), feed });
            out.push(Move::Linear { to: at(r), feed });
            if retract > r {
                out.push(Move::Rapid { to: at(retract) });
            }
            return;
        }
        DrillCycle::Ream { dwell } => {
            // In, a dwell when it has one, and back out to R at the same feed. Above R the
            // tool is clear of the hole.
            out.push(Move::Linear { to: at(to.z), feed });
            if dwell > 0.0 {
                out.push(Move::Dwell { seconds: dwell });
            }
            out.push(Move::Linear { to: at(r), feed });
            if retract > r {
                out.push(Move::Rapid { to: at(retract) });
            }
            return;
        }
    }
    out.push(Move::Rapid { to: at(retract) });
}

#[cfg(test)]
#[path = "cycle_tests.rs"]
mod tests;
