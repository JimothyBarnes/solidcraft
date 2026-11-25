//! An operation's heights (decision 0037 of the CAM work): clearance, retract, top and bottom,
//! each measured from a reference with an offset, kept as the reference so that a changed model
//! or stock moves them. The engine resolves them into the numbers the strategies take when it
//! works a toolpath out: clearance and retract into the placement, top and bottom into the
//! operation's own fields.

use serde::{Deserialize, Serialize};

use crate::placement::Placement;
use crate::units::MAX_COORDINATE;
use crate::{Error, Result};

/// What a height is measured from.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HeightFrom {
    /// The setup's own clearance or retract height (for the others, as `Feature`).
    Setup,
    /// The top or bottom of the setup's stock box.
    StockTop,
    StockBottom,
    /// The top or bottom of the box of the bodies the setup machines.
    ModelTop,
    ModelBottom,
    /// What the operation would use by itself: a pocket's floor, a hole's top, the outline's
    /// bottom, the stock's top where a cut starts.
    #[default]
    Feature,
    /// A point picked on the model (`HeightRef::at`).
    Point,
    /// The WCS's Z zero: the offset is the height.
    Z,
}

/// One height: where it is measured from, and how far above that it is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HeightRef {
    #[serde(default)]
    pub from: HeightFrom,
    /// mm, up positive.
    #[serde(default)]
    pub offset: f64,
    /// For `Point`: the point picked, in model coordinates.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<[f64; 3]>,
}

/// An operation's heights; one not given is the operation's own.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Heights {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clearance: Option<HeightRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retract: Option<HeightRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top: Option<HeightRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bottom: Option<HeightRef>,
}

impl HeightRef {
    /// Checks the offset and that a picked point is given exactly when it is measured from one.
    pub fn validate(&self, what: &str) -> Result<()> {
        if !(self.offset.abs() <= MAX_COORDINATE) {
            return Err(Error::BadValue(format!("the {what} height's offset must be finite and within {MAX_COORDINATE} mm")));
        }
        match (self.from, self.at) {
            (HeightFrom::Point, Some(at)) if at.iter().all(|v| v.abs() <= MAX_COORDINATE) => Ok(()),
            (HeightFrom::Point, _) => {
                Err(Error::BadValue(format!("the {what} height is from a point: give `at`, finite and within {MAX_COORDINATE} mm")))
            }
            (_, Some(_)) => Err(Error::BadValue(format!("the {what} height is not from a point: `at` is for one that is"))),
            (_, None) => Ok(()),
        }
    }

    /// The Z in the WCS of `placement` this height is: `own` is the operation's own value, for
    /// `Feature` and `Setup`; `point_z` the WCS Z of the point picked, for `Point` (the engine
    /// reads it from the model).
    pub fn resolve(&self, placement: &Placement, own: f64, point_z: Option<f64>) -> Result<f64> {
        let base = match self.from {
            HeightFrom::Setup | HeightFrom::Feature => own,
            HeightFrom::StockTop => placement.stock.max.z,
            HeightFrom::StockBottom => placement.stock.min.z,
            HeightFrom::ModelTop => placement.model.max.z,
            HeightFrom::ModelBottom => placement.model.min.z,
            HeightFrom::Z => 0.0,
            HeightFrom::Point => point_z.ok_or_else(|| Error::BadValue("a height from a point picked on the model has no point".into()))?,
        };
        let z = base + self.offset;
        if !(z.abs() <= MAX_COORDINATE) {
            return Err(Error::BadValue(format!("a height of {z} mm is beyond {MAX_COORDINATE} mm")));
        }
        Ok(z)
    }
}

impl Heights {
    pub fn validate(&self) -> Result<()> {
        for (what, height) in [("clearance", &self.clearance), ("retract", &self.retract), ("top", &self.top), ("bottom", &self.bottom)] {
            if let Some(height) = height {
                height.validate(what)?;
            }
        }
        Ok(())
    }

    /// `placement` with this operation's clearance and retract heights in place of the setup's;
    /// `points` are the WCS Z of their picked points. A retract above the clearance is refused.
    pub fn placed(&self, placement: &Placement, points: (Option<f64>, Option<f64>)) -> Result<Placement> {
        let mut placed = *placement;
        if let Some(height) = &self.clearance {
            placed.clearance_z = height.resolve(placement, placement.clearance_z, points.0)?;
        }
        if let Some(height) = &self.retract {
            placed.retract_z = height.resolve(placement, placement.retract_z, points.1)?;
        }
        if !(placed.clearance_z >= placed.retract_z) {
            return Err(Error::BadValue(format!(
                "the clearance height (Z {:.3} mm) must be at or above the retract height (Z {:.3} mm)",
                placed.clearance_z, placed.retract_z
            )));
        }
        Ok(placed)
    }
}

#[cfg(test)]
#[path = "heights_tests.rs"]
mod tests;
