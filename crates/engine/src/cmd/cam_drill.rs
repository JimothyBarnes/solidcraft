//! Drill's holes (decision 0040 of the CAM work): the holes of a group at one top and one depth,
//! holes picked on the model of any size, each found among the holes the setup can drill each
//! time the toolpath is worked out and drilled from its own top to its own bottom (decision 0012's
//! rule), and points picked on the model, each drilled from its own height.
use solidcore_record::Vec3;
use solidcraft_cam::{DepthTo, DrillCycle, DrillOp, HoleAt, Placement};

use crate::schema::cam::{HoleBottom, HoleGroup};

/// How far past the part a through hole is drilled, at the drill's full diameter, mm.
pub const MANUFACTURE_BREAKTHROUGH: f64 = 1.0;
/// How many pitches further a tap's tip goes through a through hole, so that its full thread is
/// past the part and not only its chamfer.
const TAP_LEAD: f64 = 3.0;

/// The depth a hole that ends as `bottom`, its wall `depth` long, is cut to from its top when no
/// depth is given, and what it is measured to (decision 0012 of the CAM work): with a drill, a
/// through hole to the drill's full diameter a little past the part, a pointed hole to its full
/// diameter at the end of its wall, and a flat-floored hole to its tip, so that a point never cuts
/// below the floor; with a tap of `tap` pitch, to its tip, a few pitches past a through hole and a
/// pitch short of a blind one's bottom.
fn own_depth(bottom: HoleBottom, depth: f64, tap: Option<f64>) -> (f64, DepthTo) {
    match (tap, bottom) {
        (None, HoleBottom::Through) => (depth + MANUFACTURE_BREAKTHROUGH, DepthTo::FullDiameter),
        (None, HoleBottom::Point) => (depth, DepthTo::FullDiameter),
        (None, HoleBottom::Flat) => (depth, DepthTo::Tip),
        (Some(pitch), HoleBottom::Through) => (depth + MANUFACTURE_BREAKTHROUGH + TAP_LEAD * pitch, DepthTo::Tip),
        (Some(pitch), HoleBottom::Flat | HoleBottom::Point) => ((depth - pitch).max(pitch), DepthTo::Tip),
    }
}

/// The depth the holes of `group` are cut to with a drill (`tap`: `None`) or a tap of that pitch
/// when no depth is typed, and what it is measured to, as `cam.op.drill` takes it (`tip` or
/// `full_diameter`), for a form to show what the command would do.
pub fn manufacture_hole_depth(group: &HoleGroup, tap: Option<f64>) -> (f64, &'static str) {
    let (depth, to) = own_depth(group.bottom, group.depth, tap);
    (
        depth,
        match to {
            DepthTo::Tip => "tip",
            DepthTo::FullDiameter => "full_diameter",
        },
    )
}

fn point(p: &[f64; 3]) -> Vec3 {
    Vec3 { x: p[0], y: p[1], z: p[2] }
}

/// The holes `drill` drills in `placement`'s WCS, each with its own top and depth as the operation
/// gives them, before its heights: the holes of a group, the points picked, or the holes picked,
/// found among `groups` (read only for those). A hole picked twice is drilled once. Refused, with
/// the point, where a hole picked is no longer found.
pub(super) fn holes_of(
    drill: &DrillOp,
    placement: &Placement,
    groups: impl FnOnce() -> Result<Vec<HoleGroup>, String>,
) -> Result<Vec<HoleAt>, String> {
    if !drill.holes.is_empty() {
        let top = drill.top.unwrap_or(placement.model.max.z);
        return Ok(drill.holes.iter().map(|&at| HoleAt { at, top, depth: drill.depth, depth_to: drill.depth_to }).collect());
    }
    if !drill.points.is_empty() {
        return Ok(drill
            .points
            .iter()
            .map(|p| {
                let at = super::cam::in_wcs(placement, point(p));
                HoleAt { at: [at.x, at.y], top: at.z, depth: drill.depth, depth_to: drill.depth_to }
            })
            .collect());
    }
    let groups = groups()?;
    let tap = match drill.cycle {
        DrillCycle::Tap { pitch } => Some(pitch),
        _ => None,
    };
    let mut taken: Vec<(usize, usize)> = Vec::new();
    let mut holes = Vec::with_capacity(drill.picked.len());
    for p in &drill.picked {
        let Some((g, h)) = super::cam::hole_picked(&groups, super::cam::in_wcs(placement, point(p))) else {
            return Err(format!(
                "the hole picked at X {:.3} Y {:.3} Z {:.3} is not there now: no hole the setup can drill has its wall or bottom there; pick it again",
                p[0], p[1], p[2]
            ));
        };
        let (Some(group), false) = (groups.get(g), taken.contains(&(g, h))) else { continue };
        let Some(&at) = group.holes.get(h) else { continue };
        taken.push((g, h));
        let (depth, depth_to) = if drill.depth > 0.0 { (drill.depth, drill.depth_to) } else { own_depth(group.bottom, group.depth, tap) };
        holes.push(HoleAt { at, top: group.top, depth, depth_to });
    }
    Ok(holes)
}

/// How far `tool` drilling the points of `drill` is meant to cut into the part as the model has
/// it, mm: a point picked is drilled where the model may have no hole yet (a spot, a hole to come),
/// as deep as its hole goes below its own height, so the simulation does not call that a gouge.
/// None for holes, which are the model's own.
pub(super) fn into_part(drill: &DrillOp, holes: &[HoleAt], tool: &solidcraft_cam::Tool) -> f64 {
    if drill.points.is_empty() {
        return 0.0;
    }
    let g = tool.geometry;
    let point = if g.tip_angle > 0.0 { 0.5 * g.diameter / (0.5 * g.tip_angle).tan() } else { 0.0 };
    holes
        .iter()
        .map(|h| match h.depth_to {
            DepthTo::FullDiameter => h.depth + point,
            DepthTo::Tip => h.depth,
        })
        .fold(0.0, f64::max)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn group(bottom: HoleBottom) -> HoleGroup {
        HoleGroup { diameter: 6.0, top: 0.0, depth: 10.0, bottom, holes: vec![[0.0, 0.0]] }
    }

    #[test]
    fn a_holes_own_depth_is_what_its_bottom_asks_of_a_drill_or_a_tap() {
        assert_eq!(manufacture_hole_depth(&group(HoleBottom::Through), None), (11.0, "full_diameter"));
        assert_eq!(manufacture_hole_depth(&group(HoleBottom::Point), None), (10.0, "full_diameter"));
        assert_eq!(manufacture_hole_depth(&group(HoleBottom::Flat), None), (10.0, "tip"));
        // A tap of 1.5 mm: 1 past the part and three pitches more, or a pitch short of a floor.
        assert_eq!(manufacture_hole_depth(&group(HoleBottom::Through), Some(1.5)), (15.5, "tip"));
        assert_eq!(manufacture_hole_depth(&group(HoleBottom::Flat), Some(1.5)), (8.5, "tip"));
        // Never less than a pitch, in a hole shallower than two.
        assert_eq!(manufacture_hole_depth(&HoleGroup { depth: 1.0, ..group(HoleBottom::Point) }, Some(1.5)), (1.5, "tip"));
    }
}
