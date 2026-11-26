//! A contour along an open chain of edges picked on the model (decision 0043 of the CAM work): the
//! foot of a step's wall, a channel's walls, the top edge of a wall the part goes down from. The
//! tool goes along the chain on its open side, where nothing stands above the bottom, as an open
//! pocket's last pass goes along its walls (decision 0035): its centre the tool's radius and the
//! stock left from everything that stands there (the verified offsetter's loops, the walls moved
//! out), kept where that runs along the chain, and lengthened straight on past the chain's ends
//! into the air beside the stock where it can be. Where a wall stands across its way the pass
//! stops there and the tool lifts. A pass with no air at its start is cut from its other end, or
//! entered down a ramp along itself.
//!
//! The engine reads the walls (the section of the part a hair above the bottom, checked to have
//! nothing above reaching out past it) and the chains from the model; nothing here keeps any.

use solidcore_record::{Region2, Seg2, Vec2, Vec3};
use solidcraft_geom::Seg2Geom;

use crate::Result;
use crate::contour::{ContourOp, Contoured, DEFAULT_CONTOUR_CLEAR, DEFAULT_CONTOUR_STEPOVER, Depths, bad};
use crate::feeds::op_feeds;
use crate::open_pocket::plan_along;
use crate::placement::Placement;
use crate::pocket::{DEFAULT_RAMP_ANGLE, Entry, MAX_RAMP_TURNS, Step, cut_step};
use crate::tool::{Engagement, Tool};
use crate::toolpath::{Coolant, Move, SpindleDirection, Toolpath};

impl ContourOp {
    /// The toolpath from `start` (where the tool is; `None` for above where it first goes down, at
    /// the clearance height) for `tool` in a setup placed at `placement`, with its stock
    /// `material`, along each of the open `chains` (pieces end to end, X and Y in the WCS) on the
    /// side away from `walls`: what stands above the cut's bottom, the chains on their outline,
    /// none overlapping another. Each chain in turn, all its levels, its passes the furthest from
    /// the walls first. Tabs are refused: a contour along an open chain cuts nothing free.
    pub fn toolpath_along(
        &self,
        start: Option<Vec3>,
        tool: &Tool,
        placement: &Placement,
        material: Option<&str>,
        walls: &[Region2],
        chains: &[Vec<Seg2>],
    ) -> Result<Contoured> {
        self.validate(tool)?;
        if self.tabs.is_some() {
            return bad(
                "tabs hold a part that a contour round its outside cuts free: a contour along an open chain cuts nothing free, so it takes no tabs"
                    .into(),
            );
        }
        let Depths { top, levels, per_level, stepdown, .. } = self.depths(tool, placement)?;
        let (stock, g) = (placement.stock, tool.geometry);
        let (retract, clearance) = (placement.retract_z, placement.clearance_z);
        if chains.is_empty() || walls.is_empty() {
            return bad("there is no open chain to cut along, or nothing stands above the bottom beside it".into());
        }
        let radius = 0.5 * g.diameter;
        let offset = radius + self.leave.unwrap_or(0.0);
        // Where the tool is in the air: beside the stock, its edge `clear` from it.
        let beyond = radius + self.clear.unwrap_or(DEFAULT_CONTOUR_CLEAR);
        let beside = (Vec2 { x: stock.min.x - beyond, y: stock.min.y - beyond }, Vec2 { x: stock.max.x + beyond, y: stock.max.y + beyond });
        let passes = self.passes.unwrap_or(1);
        let stepover = if passes > 1 { self.stepover.unwrap_or(DEFAULT_CONTOUR_STEPOVER * g.diameter) } else { 0.0 };
        let angle = self.ramp_angle.unwrap_or(DEFAULT_RAMP_ANGLE);
        let mut plans: Vec<Vec<Step>> = Vec::with_capacity(chains.len());
        let mut against = 0;
        for chain in chains {
            let plan = plan_along(walls, chain, beside, radius, offset, (passes, stepover), self.direction)?;
            let from = chain.first().map_or(Vec2 { x: 0.0, y: 0.0 }, Seg2Geom::start);
            if plan.steps.is_empty() {
                return bad(format!(
                    "the tool can't go along the open chain from X {:.3} Y {:.3}: no stretch of its path {offset} mm from what stands above the bottom runs along the chain (the wall there is not upright, or the tool does not fit beside it)",
                    from.x, from.y
                ));
            }
            // A pass with no air at either end goes down a ramp there and back along itself.
            for step in plan.steps.iter().filter(|step| step.entry == Entry::ZigZag) {
                let turns = (per_level / angle.tan() / (2.0 * step.length())).ceil();
                if !(turns <= MAX_RAMP_TURNS) {
                    return bad(format!(
                        "a pass {:.3} mm long along the open chain from X {:.3} Y {:.3} has no air at either end, and the ramp down {per_level} mm along it would take {turns} turns: give the operation a smaller stepdown",
                        step.length(),
                        from.x,
                        from.y
                    ));
                }
            }
            against += plan.against;
            plans.push(plan.steps);
        }
        let f = op_feeds(tool, Engagement { axial: per_level.max(1e-4), radial: g.diameter }, material, placement.max_rpm, self.rpm, self.feed)?;
        let at = |p: Vec2, z: f64| Vec3 { x: p.x, y: p.y, z };
        let Some(first) = plans.iter().flatten().next().map(|step| step.from.unwrap_or(step.start)) else {
            return bad("there is nothing to cut along the open chains".into());
        };
        let mut moves = vec![Move::ToolChange { tool: tool.number }, Move::Spindle { rpm: f.rpm, direction: SpindleDirection::Clockwise }];
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: tool.coolant });
        }
        moves.push(Move::Rapid { to: at(first, clearance) });
        let mut last = first;
        for steps in &plans {
            let mut above = top;
            for &z in &levels {
                for step in steps {
                    last = cut_step(&mut moves, step, (z, above), (retract, angle), &f);
                }
                above = z;
            }
        }
        moves.push(Move::Rapid { to: at(last, clearance) });
        if tool.coolant != Coolant::Off {
            moves.push(Move::Coolant { mode: Coolant::Off });
        }
        moves.push(Move::SpindleStop);
        let toolpath = Toolpath { start: start.unwrap_or(at(first, clearance)), moves };
        toolpath.validate()?;

        let steps: Vec<&Step> = plans.iter().flatten().collect();
        let mut notes = f.notes;
        if against > 0 {
            notes.push(format!(
                "{against} pass(es) along the open chains are cut the other way than asked: the tool gets to them only from their far end, beside the stock"
            ));
        }
        let ramps = steps.iter().filter(|step| step.entry == Entry::ZigZag).count();
        if ramps > 0 {
            notes.push(format!("{ramps} pass(es) along the open chains have no air at either end: the tool goes down a ramp along each"));
        }
        Ok(Contoured {
            toolpath,
            rpm: f.rpm,
            feed: f.feed,
            plunge_feed: f.plunge_feed,
            top,
            levels,
            stepdown,
            offset,
            passes,
            stepover,
            round: steps.iter().map(|step| step.length()).sum(),
            entries: steps.iter().map(|step| step.from.unwrap_or(step.start)).map(|p| [p.x, p.y]).collect(),
            leads: steps.iter().map(|_| 0.0).collect(),
            tabs: Vec::new(),
            tab_top: None,
            notes,
        })
    }
}

impl Contoured {
    /// This contour's cuts and then `then`'s, under one tool change: a contour round closed chains
    /// and along open ones in one operation. Both must have been worked out by the same operation
    /// with the same tool, so their speeds, feeds and levels are the same.
    pub fn and_then(mut self, then: Contoured) -> Result<Contoured> {
        // This one up to the clearance height at its end, without the coolant and spindle
        // stopping; the other from its rapid at the clearance height to where it first goes down.
        let Some(up) = self.toolpath.moves.iter().rposition(|m| matches!(m, Move::Rapid { .. })) else {
            return bad("a contour with no moves can't be joined to another".into());
        };
        let Some(over) = then.toolpath.moves.iter().position(|m| matches!(m, Move::Rapid { .. })) else {
            return bad("a contour with no moves can't be joined to another".into());
        };
        self.toolpath.moves.truncate(up + 1);
        self.toolpath.moves.extend(then.toolpath.moves.into_iter().skip(over));
        self.toolpath.validate()?;
        self.round += then.round;
        self.entries.extend(then.entries);
        self.leads.extend(then.leads);
        for note in then.notes {
            if !self.notes.contains(&note) {
                self.notes.push(note);
            }
        }
        Ok(self)
    }
}

#[cfg(test)]
#[path = "open_contour_tests.rs"]
mod tests;
