//! Length units at the edges (UI, posts) and the tolerances CAM computations are made to.

use serde::{Deserialize, Serialize};

use crate::{Error, Result};

/// Millimetres in one inch (exact by definition).
pub const MM_PER_INCH: f64 = 25.4;

/// The largest coordinate magnitude a toolpath may use, in mm (100 m): far beyond any machine,
/// small enough that arithmetic on it stays exact to well under a micron.
pub const MAX_COORDINATE: f64 = 100_000.0;

/// The default chordal tolerance in mm: how far a computed path may stray from the exact one.
pub const DEFAULT_TOLERANCE: f64 = 0.01;

/// The length unit a program or a display uses. Internally everything is millimetres.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LengthUnit {
    #[default]
    Millimetre,
    Inch,
}

impl LengthUnit {
    /// `value` in this unit, as millimetres.
    pub fn to_mm(self, value: f64) -> f64 {
        match self {
            LengthUnit::Millimetre => value,
            LengthUnit::Inch => value * MM_PER_INCH,
        }
    }

    /// `mm` millimetres, in this unit.
    pub fn from_mm(self, mm: f64) -> f64 {
        match self {
            LengthUnit::Millimetre => mm,
            LengthUnit::Inch => mm / MM_PER_INCH,
        }
    }
}

/// A positive, finite tolerance in mm. The smallest allowed is a tenth of a micron, the
/// largest 10 mm: below that rounding dominates, above it the number is a mistake.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "f64", into = "f64")]
pub struct Tolerance(f64);

impl Tolerance {
    /// The smallest tolerance accepted, in mm.
    pub const MIN: f64 = 1e-4;
    /// The largest tolerance accepted, in mm.
    pub const MAX: f64 = 10.0;

    /// A tolerance of `mm` millimetres, refused outside [`Tolerance::MIN`]..=[`Tolerance::MAX`].
    pub fn new(mm: f64) -> Result<Self> {
        if !(Self::MIN..=Self::MAX).contains(&mm) {
            return Err(Error::BadValue(format!("tolerance must be between {} and {} mm, not {mm}", Self::MIN, Self::MAX)));
        }
        Ok(Self(mm))
    }

    /// The tolerance in mm.
    pub fn mm(self) -> f64 {
        self.0
    }
}

impl Default for Tolerance {
    fn default() -> Self {
        Self(DEFAULT_TOLERANCE)
    }
}

impl TryFrom<f64> for Tolerance {
    type Error = Error;
    fn try_from(mm: f64) -> Result<Self> {
        Self::new(mm)
    }
}

impl From<Tolerance> for f64 {
    fn from(t: Tolerance) -> f64 {
        t.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_inch_is_exactly_25_4_mm_both_ways() {
        assert_eq!(LengthUnit::Inch.to_mm(1.0), 25.4);
        assert_eq!(LengthUnit::Inch.from_mm(254.0), 10.0);
        assert_eq!(LengthUnit::Millimetre.to_mm(3.5), 3.5);
    }

    #[test]
    fn tolerances_outside_their_range_or_not_finite_are_refused() {
        for bad in [0.0, -1.0, 1e-5, 11.0, f64::NAN, f64::INFINITY] {
            assert!(Tolerance::new(bad).is_err(), "{bad}");
        }
        assert_eq!(Tolerance::new(0.005).map(Tolerance::mm), Ok(0.005));
        assert_eq!(Tolerance::default().mm(), DEFAULT_TOLERANCE);
    }

    #[test]
    fn a_tolerance_read_from_json_is_checked() {
        assert!(serde_json::from_str::<Tolerance>("-0.1").is_err());
        assert_eq!(serde_json::from_str::<Tolerance>("0.02").map(Tolerance::mm).ok(), Some(0.02));
    }
}
