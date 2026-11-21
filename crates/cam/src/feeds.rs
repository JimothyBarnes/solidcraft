//! Spindle speed and feeds for one operation (decision 0009): the operation's own numbers where
//! it gives them, otherwise the tool's cutting data for the setup's stock material.

use crate::tool::{Engagement, FeedWarning, Tool};
use crate::toolpath::MAX_FEED;
use crate::{Error, Result};

/// The top spindle speed used while there are no machine definitions, rev/min (decision 0009).
pub const GENERIC_MAX_RPM: f64 = 10_000.0;
/// The share of a cutting feed used to plunge when the operation gives both its numbers, so the
/// tool's data is never asked.
const TYPED_PLUNGE_FACTOR: f64 = 0.5;

/// What an operation runs at.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct OpFeeds {
    pub rpm: f64,
    /// Cutting feed, mm/min.
    pub feed: f64,
    /// Feed straight down, mm/min.
    pub plunge_feed: f64,
    /// Things worth showing beside the numbers (a capped spindle speed, a thin chip).
    pub notes: Vec<String>,
}

/// Speed and feeds for `tool` at `engagement` in the setup's `material`, with the operation's own
/// `rpm` and `feed` used where given (both checked by the operation). A speed given alone keeps
/// the tool's chip: the feeds scale with it.
pub(crate) fn op_feeds(tool: &Tool, engagement: Engagement, material: Option<&str>, rpm: Option<f64>, feed: Option<f64>) -> Result<OpFeeds> {
    if let (Some(rpm), Some(feed)) = (rpm, feed) {
        return Ok(OpFeeds { rpm, feed, plunge_feed: feed * TYPED_PLUNGE_FACTOR, notes: Vec::new() });
    }
    let sf = tool.speeds_feeds(material.unwrap_or(""), engagement, GENERIC_MAX_RPM).map_err(|e| {
        if material.is_none() { Error::BadValue(format!("{e}: give the setup its stock material, or the operation its rpm and feed")) } else { e }
    })?;
    // Warnings about a number the operation gives itself don't apply.
    let applies = |w: &&FeedWarning| match w {
        FeedWarning::SpindleCapped { .. } => rpm.is_none(),
        FeedWarning::ChipTooThin { .. } | FeedWarning::ChipTooThick { .. } => feed.is_none(),
    };
    let notes = sf.warnings.iter().filter(applies).map(describe).collect();
    let rpm = rpm.unwrap_or(sf.rpm);
    let cutting = feed.unwrap_or(sf.feed * rpm / sf.rpm);
    let plunge_feed = cutting * sf.plunge_feed / sf.feed;
    if !(cutting > 0.0 && cutting <= MAX_FEED && plunge_feed > 0.0) {
        return Err(Error::BadValue(format!("the feed {cutting} mm/min is out of range")));
    }
    Ok(OpFeeds { rpm, feed: cutting, plunge_feed, notes })
}

fn describe(w: &FeedWarning) -> String {
    match *w {
        FeedWarning::SpindleCapped { wanted, max } => {
            format!("spindle speed capped at {max:.0} rpm (the tool's data asks {wanted:.0}); no machine is set, so this is the generic top speed")
        }
        FeedWarning::ChipTooThin { chip, intended } => {
            format!("the chip is {chip:.4} mm, much thinner than the {intended:.4} mm intended: the edge rubs")
        }
        FeedWarning::ChipTooThick { chip, intended } => format!("the chip is {chip:.4} mm, much thicker than the {intended:.4} mm intended"),
    }
}
