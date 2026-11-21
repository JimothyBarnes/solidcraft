//! Cutting tools: their kind, geometry and holder, the cutting data a shop keeps per work material,
//! the spindle speed and feeds derived from it, and the JSON library tools are kept in.
//!
//! Speeds and feeds follow `research/cutting-data.md`: spindle speed from surface speed at the
//! diameter actually cutting (smaller than nominal on a ball or bull nose cutting shallower than
//! its corner radius), feed per tooth raised for radial chip thinning so the chip keeps its
//! intended thickness, feed from feed per tooth × teeth × speed, a tap's feed locked to its pitch.
//! They are starting points, never guarantees; the UI says so.
//!
//! Lengths are mm, angles radians, surface speed m/min, feeds mm/min, speeds rev/min.

use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use crate::toolpath::{Coolant, MAX_FEED, MAX_SPINDLE_RPM, MAX_TOOL_NUMBER};
use crate::{Error, Result};

/// The tool library file format this build reads.
pub const TOOL_LIBRARY_FORMAT: u32 = 1;
/// The most tools one library may hold.
pub const MAX_TOOLS: usize = 10_000;
/// The most holder segments one holder may have.
pub const MAX_HOLDER_SEGMENTS: usize = 64;
/// The most cutting data entries (work materials) per tool.
pub const MAX_CUTTING_DATA: usize = 256;
/// The largest tool dimension accepted, in mm.
const MAX_TOOL_SIZE: f64 = 2000.0;
/// The smallest cutting diameter accepted, in mm.
const MIN_DIAMETER: f64 = 0.01;
/// A chip this much thinner than intended rubs instead of cutting; this much thicker overloads.
const THIN_CHIP: f64 = 0.5;
const THICK_CHIP: f64 = 1.5;

/// What a tool is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolKind {
    FlatEndMill,
    BallEndMill,
    BullNoseEndMill,
    ChamferMill,
    Engraving,
    FaceMill,
    Drill,
    SpotDrill,
    Reamer,
    Tap,
    ThreadMill,
    SlotCutter,
    Dovetail,
    Lollipop,
}

impl ToolKind {
    /// Kinds that cut along their axis only: radial engagement does not apply.
    pub fn is_axial(self) -> bool {
        matches!(self, ToolKind::Drill | ToolKind::SpotDrill | ToolKind::Reamer | ToolKind::Tap)
    }
}

/// The tool's own shape. Unused fields are 0.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolGeometry {
    /// Cutting diameter (the largest, for a chamfer or engraving tool).
    pub diameter: f64,
    /// Corner radius: 0 on a flat end mill, half the diameter on a ball end mill.
    pub corner_radius: f64,
    /// Length of the cutting flutes.
    pub flute_length: f64,
    /// From the tip to where the shank or neck widens beyond the cutting diameter.
    pub shoulder_length: f64,
    /// Tip to the end of the shank.
    pub overall_length: f64,
    pub shank_diameter: f64,
    /// Included tip angle of a drill, spot drill, chamfer or engraving tool.
    pub tip_angle: f64,
    /// The flat at the tip of a chamfer or engraving tool.
    pub tip_diameter: f64,
    /// Thread pitch of a tap or thread mill.
    pub thread_pitch: f64,
}

/// One revolved piece of the holder, from its lower end up.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HolderSegment {
    pub length: f64,
    pub lower_diameter: f64,
    pub upper_diameter: f64,
}

/// The holder: how far the tool sticks out of it and its revolved shape above that.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Holder {
    /// From the tool tip to the holder's lower face.
    pub stick_out: f64,
    pub segments: Vec<HolderSegment>,
}

/// What a shop knows about cutting one work material with this tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CuttingData {
    /// The work material's id, e.g. `aluminium_6061`.
    pub material: String,
    /// Surface speed at the cutting edge, m/min.
    pub surface_speed: f64,
    /// The intended chip thickness per tooth, mm (the feed per tooth at full engagement).
    pub chip_load: f64,
    /// Plunge feed as a fraction of the cutting feed.
    pub plunge_factor: f64,
    /// Ramp feed as a fraction of the cutting feed.
    pub ramp_factor: f64,
}

/// Speed and feed typed in directly instead of derived.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Overrides {
    pub rpm: Option<f64>,
    pub feed: Option<f64>,
}

/// A cutting tool.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    /// The tool number (pocket) on the machine.
    pub number: u32,
    pub name: String,
    pub kind: ToolKind,
    pub geometry: ToolGeometry,
    pub flutes: u32,
    pub holder: Holder,
    /// Length and diameter offset registers (usually the tool number).
    pub length_offset: u32,
    pub diameter_offset: u32,
    pub coolant: Coolant,
    pub cutting_data: Vec<CuttingData>,
    pub overrides: Overrides,
}

/// How a tool engages the work for one operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Engagement {
    /// Axial depth of cut (ap), mm.
    pub axial: f64,
    /// Radial engagement (ae, the stepover), mm.
    pub radial: f64,
}

/// Something about derived speeds and feeds worth showing next to them.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FeedWarning {
    /// The derived speed was above the machine's; it was capped, so the surface speed is lower.
    SpindleCapped { wanted: f64, max: f64 },
    /// The chip is much thinner than intended: the edge rubs and wears.
    ChipTooThin { chip: f64, intended: f64 },
    /// The chip is much thicker than intended: risk of breaking the tool.
    ChipTooThick { chip: f64, intended: f64 },
}

/// Speed and feeds for one tool, material and engagement, with the numbers they came from.
#[derive(Clone, Debug, PartialEq)]
pub struct SpeedsFeeds {
    pub rpm: f64,
    /// Cutting feed, mm/min.
    pub feed: f64,
    pub plunge_feed: f64,
    pub ramp_feed: f64,
    /// The diameter actually cutting at this depth, mm.
    pub effective_diameter: f64,
    /// The surface speed at that diameter at `rpm`, m/min.
    pub surface_speed: f64,
    /// Feed per tooth, mm.
    pub feed_per_tooth: f64,
    /// The thickest chip at this engagement, mm.
    pub chip_thickness: f64,
    /// Whether rpm or feed were typed in rather than derived.
    pub overridden: bool,
    pub warnings: Vec<FeedWarning>,
}

fn in_range(v: f64, lo: f64, hi: f64) -> bool {
    v >= lo && v <= hi
}

impl Tool {
    /// Checks every number and the rules of the tool's kind.
    pub fn validate(&self) -> Result<()> {
        let bad = |m: String| Err(Error::BadValue(format!("tool {} ({}): {m}", self.number, self.name)));
        if !(1..=MAX_TOOL_NUMBER).contains(&self.number) {
            return bad(format!("tool number must be 1 to {MAX_TOOL_NUMBER}"));
        }
        if self.name.chars().count() > 200 {
            return bad("name longer than 200 characters".into());
        }
        let g = self.geometry;
        let lengths = [g.corner_radius, g.flute_length, g.shoulder_length, g.overall_length, g.shank_diameter, g.tip_diameter, g.thread_pitch];
        if !in_range(g.diameter, MIN_DIAMETER, MAX_TOOL_SIZE) || lengths.iter().any(|&v| !in_range(v, 0.0, MAX_TOOL_SIZE)) {
            return bad(format!("dimensions must be finite, at most {MAX_TOOL_SIZE} mm, and the diameter at least {MIN_DIAMETER} mm"));
        }
        if !(g.flute_length > 0.0 && g.overall_length >= g.flute_length && g.shank_diameter > 0.0) {
            return bad("needs a flute length, an overall length at least that long, and a shank diameter".into());
        }
        if g.shoulder_length != 0.0 && !(g.shoulder_length >= g.flute_length && g.shoulder_length <= g.overall_length) {
            return bad("the shoulder must be between the flute length and the overall length".into());
        }
        if !(1..=1000).contains(&self.flutes) {
            return bad("flutes must be 1 to 1000".into());
        }
        let r = g.corner_radius;
        let half = 0.5 * g.diameter;
        let angled = matches!(self.kind, ToolKind::Drill | ToolKind::SpotDrill | ToolKind::ChamferMill | ToolKind::Engraving | ToolKind::Dovetail);
        let kind_ok = match self.kind {
            ToolKind::FlatEndMill | ToolKind::SlotCutter | ToolKind::FaceMill => r <= 0.25 * g.diameter,
            ToolKind::BallEndMill | ToolKind::Lollipop => (r - half).abs() <= 1e-9,
            ToolKind::BullNoseEndMill => r > 0.0 && r < half,
            _ => r <= half,
        };
        if !kind_ok {
            return bad(format!("corner radius {r} does not suit a {:?}", self.kind));
        }
        if angled != (g.tip_angle > 0.0) || !in_range(g.tip_angle, 0.0, PI - 1e-6) {
            return bad("a drill, spot drill, chamfer, engraving or dovetail tool needs a tip angle below 180°; others none".into());
        }
        if matches!(self.kind, ToolKind::Tap | ToolKind::ThreadMill) != (g.thread_pitch > 0.0) {
            return bad("a tap or thread mill needs a thread pitch; others none".into());
        }
        if g.tip_diameter >= g.diameter {
            return bad("the tip flat must be smaller than the diameter".into());
        }
        let h = &self.holder;
        if !in_range(h.stick_out, 0.0, MAX_TOOL_SIZE) || h.stick_out > g.overall_length || h.segments.len() > MAX_HOLDER_SEGMENTS {
            return bad(format!("holder: stick-out within the tool's overall length, at most {MAX_HOLDER_SEGMENTS} segments"));
        }
        for s in &h.segments {
            if !(in_range(s.length, 1e-3, MAX_TOOL_SIZE)
                && in_range(s.lower_diameter, 1e-3, MAX_TOOL_SIZE)
                && in_range(s.upper_diameter, 1e-3, MAX_TOOL_SIZE))
            {
                return bad("holder segments need positive, finite sizes".into());
            }
        }
        if self.cutting_data.len() > MAX_CUTTING_DATA {
            return bad(format!("at most {MAX_CUTTING_DATA} cutting data entries"));
        }
        for (i, c) in self.cutting_data.iter().enumerate() {
            if c.material.is_empty() || c.material.len() > 64 || self.cutting_data.iter().take(i).any(|d| d.material == c.material) {
                return bad(format!("cutting data {i}: a material id of 1 to 64 characters, once per tool"));
            }
            if !(in_range(c.surface_speed, 0.1, 10_000.0) && in_range(c.chip_load, 1e-5, 10.0)) {
                return bad(format!("cutting data for {}: surface speed 0.1-10000 m/min, chip load 0.00001-10 mm", c.material));
            }
            if !(in_range(c.plunge_factor, 0.01, 1.0) && in_range(c.ramp_factor, 0.01, 1.0)) {
                return bad(format!("cutting data for {}: plunge and ramp factors 0.01 to 1", c.material));
            }
        }
        if let Some(rpm) = self.overrides.rpm
            && !(rpm > 0.0 && rpm <= MAX_SPINDLE_RPM)
        {
            return bad(format!("override speed must be above 0 and at most {MAX_SPINDLE_RPM} rpm"));
        }
        if let Some(feed) = self.overrides.feed
            && !(feed > 0.0 && feed <= MAX_FEED)
        {
            return bad(format!("override feed must be above 0 and at most {MAX_FEED} mm/min"));
        }
        Ok(())
    }

    /// The diameter cutting at axial depth `ap`: smaller than nominal on a corner radius cutting
    /// shallower than the radius (`research/cutting-data.md`).
    pub fn effective_diameter(&self, ap: f64) -> f64 {
        let g = self.geometry;
        let r = g.corner_radius;
        if r > 0.0 && ap < r && !self.kind.is_axial() {
            2.0 * (r * r - (r - ap) * (r - ap)).max(0.0).sqrt() + (g.diameter - 2.0 * r)
        } else {
            g.diameter
        }
    }

    /// Speed and feeds for `material` at `engagement`, capped at `max_rpm`.
    pub fn speeds_feeds(&self, material: &str, engagement: Engagement, max_rpm: f64) -> Result<SpeedsFeeds> {
        self.validate()?;
        let bad = |m: String| Err(Error::BadValue(format!("tool {}: {m}", self.number)));
        let Engagement { axial: ap, radial: ae } = engagement;
        if !(in_range(ap, 1e-4, MAX_TOOL_SIZE) && in_range(ae, 0.0, MAX_TOOL_SIZE) && max_rpm > 0.0 && max_rpm <= MAX_SPINDLE_RPM) {
            return bad("depth above 0, stepover 0 or more, machine speed above 0".into());
        }
        if !self.kind.is_axial() && !(ae > 0.0) {
            return bad("a milling tool needs a stepover above 0".into());
        }
        let data = self.cutting_data.iter().find(|c| c.material == material);
        let mut warnings = Vec::new();
        let dc = self.effective_diameter(ap).max(MIN_DIAMETER);
        // Spindle speed: typed in, or from surface speed at the cutting diameter.
        let (rpm, overridden_rpm) = match (self.overrides.rpm, data) {
            (Some(rpm), _) => (rpm, true),
            (None, Some(c)) => (c.surface_speed * 1000.0 / (PI * dc), false),
            (None, None) => return bad(format!("no cutting data for '{material}' and no typed-in speed")),
        };
        let rpm = if rpm > max_rpm {
            warnings.push(FeedWarning::SpindleCapped { wanted: rpm, max: max_rpm });
            max_rpm
        } else {
            rpm
        };
        // How thick the chip is for a feed per tooth fz at this engagement.
        let thinning = if self.kind.is_axial() || ae >= 0.5 * dc { 1.0 } else { 2.0 * ((ae / dc) - (ae / dc).powi(2)).sqrt() };
        let teeth = f64::from(self.flutes);
        let (feed, fz) = if self.kind == ToolKind::Tap {
            // A tap's feed is locked to its pitch: one pitch per revolution.
            (rpm * self.geometry.thread_pitch, self.geometry.thread_pitch / teeth)
        } else {
            match (self.overrides.feed, data) {
                (Some(feed), _) => (feed, feed / (rpm * teeth)),
                (None, Some(c)) => {
                    let fz = c.chip_load / thinning;
                    (fz * teeth * rpm, fz)
                }
                (None, None) => return bad(format!("no cutting data for '{material}' and no typed-in feed")),
            }
        };
        if !(feed > 0.0 && feed <= MAX_FEED) {
            return bad(format!("the derived feed {feed} mm/min is out of range"));
        }
        let chip = fz * thinning;
        if let Some(c) = data
            && self.kind != ToolKind::Tap
        {
            if chip < THIN_CHIP * c.chip_load {
                warnings.push(FeedWarning::ChipTooThin { chip, intended: c.chip_load });
            } else if chip > THICK_CHIP * c.chip_load {
                warnings.push(FeedWarning::ChipTooThick { chip, intended: c.chip_load });
            }
        }
        let (plunge, ramp) = data.map_or((0.5, 0.5), |c| (c.plunge_factor, c.ramp_factor));
        Ok(SpeedsFeeds {
            rpm,
            feed,
            plunge_feed: feed * plunge,
            ramp_feed: feed * ramp,
            effective_diameter: dc,
            surface_speed: PI * dc * rpm / 1000.0,
            feed_per_tooth: fz,
            chip_thickness: chip,
            overridden: overridden_rpm || self.overrides.feed.is_some(),
            warnings,
        })
    }
}

/// A tool library file: a named list of tools with unique numbers.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolLibrary {
    pub format: u32,
    pub name: String,
    pub tools: Vec<Tool>,
}

impl ToolLibrary {
    /// Reads and checks a library: the format, every tool, and that no two share a number.
    pub fn from_json(json: &str) -> Result<ToolLibrary> {
        let lib: ToolLibrary = serde_json::from_str(json).map_err(|e| Error::BadValue(format!("tool library: {e}")))?;
        lib.validate()?;
        Ok(lib)
    }

    /// The library as pretty JSON (diffable, one field per line).
    pub fn to_json(&self) -> Result<String> {
        self.validate()?;
        serde_json::to_string_pretty(self).map_err(|e| Error::BadValue(format!("tool library: {e}")))
    }

    pub fn validate(&self) -> Result<()> {
        if self.format != TOOL_LIBRARY_FORMAT {
            return Err(Error::BadValue(format!("tool library format {} (this build reads {TOOL_LIBRARY_FORMAT})", self.format)));
        }
        if self.tools.len() > MAX_TOOLS || self.name.chars().count() > 200 {
            return Err(Error::BadValue(format!("tool library: at most {MAX_TOOLS} tools and a name of at most 200 characters")));
        }
        for (i, t) in self.tools.iter().enumerate() {
            t.validate()?;
            if self.tools.iter().take(i).any(|u| u.number == t.number) {
                return Err(Error::BadValue(format!("tool library: tool number {} is used twice", t.number)));
            }
        }
        Ok(())
    }

    /// Tools whose name contains `text` (any case), of `kind` when given, with a diameter in
    /// `diameter` (mm, inclusive) when given; in library order.
    pub fn search(&self, text: &str, kind: Option<ToolKind>, diameter: Option<(f64, f64)>) -> Vec<&Tool> {
        let text = text.to_lowercase();
        self.tools
            .iter()
            .filter(|t| t.name.to_lowercase().contains(&text))
            .filter(|t| kind.is_none_or(|k| t.kind == k))
            .filter(|t| diameter.is_none_or(|(lo, hi)| t.geometry.diameter >= lo && t.geometry.diameter <= hi))
            .collect()
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;
