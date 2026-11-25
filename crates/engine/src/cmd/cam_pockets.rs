//! Pockets found on the model for the Manufacture commands (decision 0014 of the CAM work), read
//! exactly from the bodies' B-rep, never a mesh.
//!
//! A pocket is named by a point on its floor. A ray fired down at the point finds the floor: the
//! first face it meets must be flat and face up. The pocket's region is the air just above the
//! floor: in the section of the bodies a hair above it ([`super::cam_outline::cut_faces`]), the
//! hole of a cut face that the point lies in is the pocket's wall, and the cut faces inside that
//! hole are its islands. Nothing may be in the way above: the region, shrunk by a hair, as a
//! prism from above the floor to above the bodies must not meet them.
//!
//! A floor with no such hole round it is a pocket open at a side (decision 0035): a step, a
//! shoulder, a channel open at an end, the floor round a boss. It is read as its floor (the
//! flat face under the point and the faces of its height joined to it) and what stands above
//! it (the outlines of the section's cut faces, walls and islands alike). Nothing above the
//! floor's height may reach out past those outlines: checked exactly, as a contour's outline is.
//!
//! That last check is a boolean for every pocket. A pocket that is cut is always checked so; the
//! list of a part's pockets, which a form shows as it opens, looks up from places along each
//! pocket's wall with rays instead (a sample, as the holes' list is), so a part with dozens of
//! counterbores lists at once. What the rays miss is refused, with the reason, when it is cut.
use solidcore_record::brep::Loop;
use solidcore_record::{Loop2, Plane, Region2, Vec2, Vec3};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{Body, BoolOp};

use super::cam_outline::{FACING, cut_faces, face_rings, hair_of};

/// The most faces looked at for floors in one body.
const MAX_FLOOR_FACES: usize = 20_000;
/// The most pockets listed.
pub(super) const MAX_FOUND_POCKETS: usize = 1_000;
/// The most places along a pocket's wall its list looks up from.
const MAX_LOOKS: usize = 200;

/// A pocket, in the coordinates of the plane it was looked for in.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pocket {
    /// Its floor's height above the plane.
    pub floor: f64,
    /// The air just above its floor: the wall's outline, with the islands as holes. Of a pocket
    /// open at a side, the face of its floor that was found, with the openings in it as holes.
    pub region: Region2,
    /// For a pocket open at a side: its floor and what stands above it. `None` for one with a
    /// wall all round.
    pub open: Option<solidcraft_cam::OpenFloor>,
}

/// How far round a picked point the pockets are asked about, near and far: a point picked on a
/// wall is on a region's edge, and a display mesh is a little off a curved wall.
const PICK_REACH: [f64; 2] = [0.05, 0.5];

/// Which of `pockets` the point `at` was picked on (decision 0030 of the CAM work): `at` is in
/// the plane's X and Y with its height above the plane as Z, and `top` is the height of the
/// part's top. The pocket is one whose region the point is in, on its floor or on its wall
/// between the floor and the top. The region is asked about the point and about places close
/// round it: the pocket most of them are in is the one, and of two that tie (a pocket in
/// another's floor, one wall over the other) the one whose floor is nearer below the point.
pub(super) fn picked<'p>(pockets: impl Iterator<Item = &'p Pocket>, at: Vec3, top: f64) -> Option<usize> {
    if !(at.x.is_finite() && at.y.is_finite() && at.z.is_finite() && top.is_finite()) {
        return None;
    }
    let round = PICK_REACH.iter().flat_map(|reach| {
        (0..8).map(move |k| {
            let turn = f64::from(k) * std::f64::consts::FRAC_PI_4;
            Vec2 { x: at.x + reach * turn.cos(), y: at.y + reach * turn.sin() }
        })
    });
    let places: Vec<Vec2> = std::iter::once(Vec2 { x: at.x, y: at.y }).chain(round).collect();
    // The pocket, how many of the places are in it, and how far its floor is below the point.
    let mut best: Option<(usize, usize, f64)> = None;
    for (index, pocket) in pockets.enumerate() {
        if at.z < pocket.floor - PICK_REACH[0] || at.z > top + PICK_REACH[0] {
            continue;
        }
        // A pocket open at a side is its whole floor, whichever of its faces it was found by.
        let within =
            |place: Vec2| pocket.region.contains(place) || pocket.open.as_ref().is_some_and(|open| open.floor.iter().any(|f| f.contains(place)));
        let inside = places.iter().filter(|place| within(**place)).count();
        let below = at.z - pocket.floor;
        if inside > 0 && best.is_none_or(|(_, most, nearest)| inside > most || (inside == most && below < nearest)) {
            best = Some((index, inside, below));
        }
    }
    best.map(|(index, ..)| index)
}

/// What is under a point of the plane, seen from above.
enum Under {
    /// Nothing of the bodies: the open, or an opening through them.
    Nothing,
    /// A flat face that faces up, this high above the plane: which body's (its place in the
    /// finder's list) and which of its faces.
    Floor(f64, usize, usize),
    /// Something that is not a flat face facing up.
    NotFlat,
}

/// Looks for pockets in `bodies` (each with its name, for errors) from above `plane`: its
/// normal is up, and `top` is a height above it that nothing of the bodies passes.
pub(super) struct Finder<'a> {
    bodies: &'a [(&'a str, &'a Body)],
    plane: Plane,
    top: f64,
    hair: f64,
    /// The cut faces of the bodies' sections read so far, by the height they were read at: each
    /// face's rings, the outer one first.
    sections: Vec<(f64, Vec<Vec<Loop2>>)>,
}

impl<'a> Finder<'a> {
    pub(super) fn new(bodies: &'a [(&'a str, &'a Body)], plane: &Plane, top: f64) -> Finder<'a> {
        let size = bodies.iter().map(|(_, body)| body.size()).fold(0.0, f64::max);
        Finder { bodies, plane: *plane, top, hair: hair_of(size), sections: Vec::new() }
    }

    /// The cut faces of every body's section a hair above `floor`.
    fn faces_above(&mut self, floor: f64) -> Result<&[Vec<Loop2>], String> {
        let at = self.sections.iter().position(|(height, _)| (height - floor).abs() <= 1e-9);
        let at = match at {
            Some(at) => at,
            None => {
                let cut = self.plane.offset(floor + self.hair);
                let mut faces = Vec::new();
                for &(name, body) in self.bodies {
                    faces.extend(cut_faces(body, &cut).map_err(|e| format!("body '{name}': {e}"))?.map(|above| above.faces).unwrap_or_default());
                }
                self.sections.push((floor, faces));
                self.sections.len() - 1
            }
        };
        Ok(self.sections.get(at).map_or(&[], |(_, faces)| faces.as_slice()))
    }

    /// The pocket whose floor is under the point `at` of the plane, or why there is none to cut:
    /// its floor and the air above it, with nothing in the way above (checked exactly).
    pub(super) fn pocket_at(&mut self, at: Vec2) -> Result<Pocket, String> {
        let pocket = self.region_at(at)?;
        match &pocket.open {
            None => self.clear_above(&pocket, at)?,
            Some(open) => self.nothing_past(&open.walls, pocket.floor, at)?,
        }
        Ok(pocket)
    }

    /// The floor under the point `at` of the plane and the air just above it, whatever is over it.
    fn region_at(&mut self, at: Vec2) -> Result<Pocket, String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let (floor, body, face) = match self.under(at)? {
            Under::Nothing => return Err(format!("there is nothing of the part under {here}")),
            Under::NotFlat => return Err(format!("the part is not flat and facing up at {here}: a pocket is named by a point on its floor")),
            Under::Floor(floor, body, face) => (floor, body, face),
        };
        if self.top - floor <= 2.0 * self.hair {
            return Err(format!("{here} is on top of the part, not on the floor of a pocket"));
        }
        let Some(region) = self.air_at(floor, at)? else {
            // No wall all round it: open at a side.
            return self.open_at(floor, body, face, at);
        };
        Ok(Pocket { floor, region, open: None })
    }

    /// The pocket open at a side whose floor, `floor` above the plane, is the face `face` of the
    /// finder's body `body` and the faces of that height joined to it: the floor, and what
    /// stands above it, whatever is over it.
    fn open_at(&mut self, floor: f64, body: usize, face: usize, at: Vec2) -> Result<Pocket, String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let Some(&(name, solid)) = self.bodies.get(body) else { return Err(format!("there is nothing of the part under {here}")) };
        let faces = floor_faces(solid, face, &self.plane, floor, self.hair).map_err(|e| format!("body '{name}', the floor at {here}: {e}"))?;
        let as_region = |rings: &[Loop2]| {
            let mut rings = rings.iter();
            rings.next().map(|outer| Region2 { outer: outer.ccw(), holes: rings.map(|ring| ring.ccw().reversed()).collect() })
        };
        let Some(region) = faces.first().and_then(|rings| as_region(rings)) else {
            return Err(format!("the floor at {here} has no outline"));
        };
        // The tool passes over the openings in a floor: its outlines alone.
        let around: Vec<Region2> =
            faces.iter().filter_map(|rings| rings.first()).map(|outer| Region2 { outer: outer.ccw(), holes: Vec::new() }).collect();
        // What stands above it: the outline of each cut face of the section a hair above, but
        // for those inside another (an island in a closed pocket of a wall).
        let cut = self.faces_above(floor)?;
        let outers: Vec<&Loop2> = cut.iter().filter_map(|rings| rings.first()).collect();
        let walls: Vec<Region2> = outers
            .iter()
            .filter(|outer| {
                !outers.iter().any(|other| !std::ptr::eq(*other, **outer) && outer.segs.first().is_some_and(|seg| other.contains(seg.start())))
            })
            .map(|outer| Region2 { outer: outer.ccw(), holes: Vec::new() })
            .collect();
        if walls.is_empty() {
            return Err(format!("{here} is on top of the part, not on the floor of a pocket"));
        }
        if walls.len() > MAX_FOUND_POCKETS {
            return Err(format!("more than {MAX_FOUND_POCKETS} pieces of the part stand above the floor at {here}"));
        }
        Ok(Pocket { floor, region, open: Some(solidcraft_cam::OpenFloor { floor: around, walls }) })
    }

    /// Checks that nothing of the bodies above the height `floor` reaches out past `walls`, the
    /// outlines of what stands there, for the pocket open at a side found at `at`: its passes
    /// keep off those outlines, and what stuck out over them would be cut into. Exactly, as a
    /// contour's outline is checked: each outline grown by a hair, as a prism from just under
    /// the cut to above the bodies, is cut from what the bodies have above the cut, and nothing
    /// may be left.
    fn nothing_past(&self, walls: &[Region2], floor: f64, at: Vec2) -> Result<(), String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let (cut, height) = (self.plane.offset(floor + self.hair), self.top - floor);
        if height <= 2.0 * self.hair {
            return Ok(());
        }
        let mut grown = Vec::with_capacity(walls.len());
        for wall in walls {
            let wider = solidcraft_cam::offset_region(wall, -self.hair).map_err(|e| format!("the walls of the pocket at {here}: {e}"))?;
            grown.extend(wider.into_iter().map(|r| Region2 { outer: r.outer, holes: Vec::new() }));
        }
        let prisms = solidcraft_kernel::extrude(&cut, &grown, -self.hair, height).map_err(|e| format!("checking the pocket at {here}: {e}"))?;
        for &(name, body) in self.bodies {
            let Some(above) = cut_faces(body, &cut).map_err(|e| format!("body '{name}': {e}"))? else { continue };
            let mut left = Some(above.kept);
            for prism in &prisms {
                let Some(rest) = left else { break };
                left = solidcraft_kernel::boolean(&rest, prism, BoolOp::Cut)
                    .map_err(|e| format!("checking the pocket at {here} against body '{name}': {e}"))?;
            }
            if left.is_some() {
                return Err(format!(
                    "something of body '{name}' above the floor of the pocket at {here} reaches out past the foot of its walls (an undercut or an overhang): it can't be cut straight down"
                ));
            }
        }
        Ok(())
    }

    /// What is under the point `at` of the plane, looking down from above everything.
    fn under(&self, at: Vec2) -> Result<Under, String> {
        let (up, above) = (self.plane.normal(), self.top + 1.0);
        let from = self.plane.to_world(at) + up * above;
        let mut floor: Option<(f64, usize, &Body, usize)> = None;
        for (index, &(name, body)) in self.bodies.iter().enumerate() {
            if body.is_mesh() {
                return Err(format!("body '{name}' is a mesh and has no exact faces: convert it to a solid first"));
            }
            if let Some(hit) = body.fire_ray(from, -up).map_err(|e| format!("body '{name}': {e}"))?
                && floor.is_none_or(|(nearest, ..)| hit.depth < nearest)
            {
                floor = Some((hit.depth, index, body, hit.face));
            }
        }
        let Some((depth, index, body, face)) = floor else { return Ok(Under::Nothing) };
        if !body.planar_face(face).is_some_and(|on| on.normal().dot(up) >= FACING) {
            return Ok(Under::NotFlat);
        }
        // Without the last bits of the ray's arithmetic, so that a floor at -7 reads -7.
        Ok(Under::Floor(((above - depth) * 1e9).round() / 1e9, index, face))
    }

    /// The air round the point `at` a hair above the height `floor`: the smallest hole of a cut
    /// face of the bodies' section there that the point is in (a wall all round it), with the
    /// cut faces inside that hole as its own holes: material standing there, islands, but for
    /// those that stand inside another one (in a pocket of an island). `None` when the point is
    /// in no such hole: on the part, or open to its outside.
    fn air_at(&mut self, floor: f64, at: Vec2) -> Result<Option<Region2>, String> {
        let faces = self.faces_above(floor)?;
        let wall = faces
            .iter()
            .flat_map(|rings| rings.iter().skip(1))
            .filter(|ring| ring.contains(at))
            .min_by(|a, b| a.signed_area().abs().total_cmp(&b.signed_area().abs()));
        Ok(wall.map(|wall| air_within(faces, wall)))
    }

    /// The openings through the bodies, for the list a form shows: one for each hole of a cut
    /// face of their section a hair above `lowest`, their own bottom, which is its floor here,
    /// with a point in it; the largest first. Nothing was seen above any of them from places
    /// along its wall ([`Finder::seen_above`]: a recess in the underside is not one); an
    /// opening is checked exactly when it is cut.
    pub(super) fn windows(&mut self, lowest: f64) -> Result<Vec<(Vec2, Pocket)>, String> {
        let faces = self.faces_above(lowest)?;
        let walls: Vec<Region2> = faces.iter().flat_map(|rings| rings.iter().skip(1)).map(|wall| air_within(faces, wall)).collect();
        if walls.len() > MAX_FOUND_POCKETS {
            return Err(format!("the part has more than {MAX_FOUND_POCKETS} openings through it"));
        }
        let mut found: Vec<(Vec2, Pocket)> = Vec::new();
        for region in walls {
            let window = Pocket { floor: lowest, region, open: None };
            if !self.seen_above(&window) {
                found.push((window.region.interior_point(), window));
            }
        }
        found.sort_by(|a, b| b.1.region.area().total_cmp(&a.1.region.area()).then(a.0.y.total_cmp(&b.0.y)).then(a.0.x.total_cmp(&b.0.x)));
        Ok(found)
    }

    /// The opening the point `at` of the plane is in, for a cut round the inside of its wall
    /// (decision 0021 of the CAM work), with the height its wall is read at as its floor:
    /// `bottom` when given (`lowest`, the bodies' own bottom, for a cut that goes on under them:
    /// there is no wall to read there); else the floor under the point, or `lowest` where
    /// nothing is under it and the opening goes through. The wall and what stands in the
    /// opening are read a hair above that height, with nothing in the way above.
    pub(super) fn opening_at(&mut self, at: Vec2, bottom: Option<f64>, lowest: f64) -> Result<Pocket, String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let floor = match bottom {
            Some(bottom) => bottom.max(lowest),
            None => match self.under(at)? {
                Under::Floor(floor, ..) => floor,
                Under::Nothing => lowest,
                Under::NotFlat => return Err(format!("the part is not flat and facing up under {here}: give the contour its bottom")),
            },
        };
        if self.top - floor <= 2.0 * self.hair {
            return Err(format!("{here} is on top of the part, not in an opening"));
        }
        let Some(region) = self.air_at(floor, at)? else {
            return Err(format!(
                "{here} is not in an opening of the part at Z {floor} mm: an inside contour goes round a wall that is closed all round"
            ));
        };
        let opening = Pocket { floor, region, open: None };
        self.clear_above(&opening, at)?;
        Ok(opening)
    }

    /// Checks that nothing is in the way above `pocket` (found at `at`): its walls go straight
    /// down to its floor, and nothing hangs over it. Exactly: the region, shrunk by a hair, as a
    /// prism from above the floor to above the bodies meets none of them.
    fn clear_above(&self, pocket: &Pocket, at: Vec2) -> Result<(), String> {
        self.nothing_over(pocket, &format!("the pocket at X {} Y {}", at.x, at.y))
    }

    /// [`Finder::clear_above`] for the pocket `what` names.
    fn nothing_over(&self, pocket: &Pocket, what: &str) -> Result<(), String> {
        let (cut, height) = (self.plane.offset(pocket.floor + self.hair), self.top - pocket.floor);
        let within = solidcraft_cam::offset_region(&pocket.region, self.hair).map_err(|e| format!("{what}: {e}"))?;
        if within.is_empty() || height <= 2.0 * self.hair {
            return Ok(());
        }
        let prisms = solidcraft_kernel::extrude(&cut, &within, self.hair, height).map_err(|e| format!("checking {what}: {e}"))?;
        for &(name, body) in self.bodies {
            for prism in &prisms {
                let met =
                    solidcraft_kernel::boolean(prism, body, BoolOp::Intersect).map_err(|e| format!("checking {what} against body '{name}': {e}"))?;
                if met.is_some() {
                    return Err(format!(
                        "something of body '{name}' is in the way above the floor of {what} (a fillet at the foot of its walls, an undercut or an overhang): it can't be cut straight down"
                    ));
                }
            }
        }
        Ok(())
    }

    /// The pocket inside the closed chain `wall` (in the plane's coordinates), cut down to `floor`
    /// (decision 0041 of the CAM work), as `what` names it: the air within the chain a hair above
    /// the floor, with what of the bodies stands in it there and the chains `islands` as its
    /// islands (an island of the bodies inside one picked is that one; one picked inside an island
    /// of the bodies, or of its outline, is that island), and nothing in the way above (checked
    /// exactly).
    pub(super) fn inside_chain(&mut self, floor: f64, wall: &Loop2, islands: &[Loop2], what: &str) -> Result<Pocket, String> {
        let found = air_within(self.faces_above(floor)?, wall);
        let starts_in = |ring: &Loop2, rings: &[Loop2]| ring.segs.first().is_some_and(|seg| rings.iter().any(|r| r.ccw().contains(seg.start())));
        let of_the_bodies =
            |island: &Loop2| starts_in(island, &found.holes) || found.holes.iter().any(|hole| super::cam_chains::same_outline(hole, island));
        let picked: Vec<Loop2> = islands.iter().filter(|&island| !of_the_bodies(island)).map(|island| island.ccw()).collect();
        let holes =
            found.holes.iter().filter(|&hole| !starts_in(hole, &picked)).cloned().chain(picked.iter().map(|island| island.reversed())).collect();
        let pocket = Pocket { floor, region: Region2 { outer: found.outer, holes }, open: None };
        self.nothing_over(&pocket, what)?;
        Ok(pocket)
    }

    /// Which side of the closed chain `ring` (in the plane's coordinates, counter-clockwise) the
    /// bodies are on, for a chamfer of edges picked (decision 0042 of the CAM work), as `what`
    /// names it: the section of the bodies at the height `at` (a hair below the chamfer's lower
    /// edge) has a ring of the chain's outline, the outer ring of a piece (the bodies are inside
    /// it: the tool goes round outside) or a hole's (the tool goes round inside, in its air).
    /// Checked exactly: the air beside the chain is open from `tip`, the height of the tool's
    /// point, up: inside a rim, its air; outside an edge, a band `reach` wide round it.
    pub(super) fn chamfer_wall(&mut self, ring: &Loop2, at: f64, tip: f64, reach: f64, what: &str) -> Result<solidcraft_cam::ChamferWall, String> {
        let faces = self.faces_above(at - self.hair)?;
        let mut side = None;
        for rings in faces {
            if let Some(k) = rings.iter().position(|r| super::cam_chains::same_outline(r, ring)) {
                side = Some(match rings.get(k) {
                    Some(hole) if k > 0 => Some(air_within(faces, hole)),
                    _ => None,
                });
                break;
            }
        }
        let Some(side) = side else {
            return Err(format!(
                "{what} is not the top edge of a wall that goes straight down from it: a chamfer breaks an edge whose wall stands upright under it (not a drafted wall, nor where two flat faces meet)"
            ));
        };
        match side {
            None => {
                let grown = solidcraft_cam::offset_region(&Region2 { outer: ring.ccw(), holes: Vec::new() }, -reach).map_err(|e| format!("{what}: {e}"))?;
                let Some(outer) = grown.into_iter().next().map(|region| region.outer) else {
                    return Err(format!("{what}: nothing to go round"));
                };
                let band = Region2 { outer, holes: vec![ring.ccw().reversed()] };
                if !self.open_above(&band, tip)? {
                    return Err(format!(
                        "something of the part stands within {reach:.3} mm of {what}, above where the tool's point goes (Z {tip:.3} mm): the tool would cut into it; a smaller chamfer, the tip less far below it, or a narrower tool"
                    ));
                }
                Ok(solidcraft_cam::ChamferWall::Outside(ring.ccw()))
            }
            Some(air) => {
                if !self.open_above(&air, tip)? {
                    return Err(format!(
                        "the opening inside {what} is not open as far down as the tool's point goes (Z {tip:.3} mm): something in it would be cut into; a smaller chamfer, or the tip less far below it"
                    ));
                }
                Ok(solidcraft_cam::ChamferWall::Inside(air))
            }
        }
    }

    /// The openings in the bodies' top whose rims a chamfer breaks (decision 0018 of the CAM
    /// work, all the edges): the air in every hole of a cut face of the bodies' section at the
    /// height `at`, with what stands in it as its holes, that is open from `tip`, the height of
    /// the tool's point, up to above the bodies ([`Finder::open_above`]); and how many were left
    /// out because something is in the way there.
    pub(super) fn rims(&mut self, at: f64, tip: f64) -> Result<(Vec<Region2>, usize), String> {
        let faces = self.faces_above(at - self.hair)?;
        let airs: Vec<Region2> = faces.iter().flat_map(|rings| rings.iter().skip(1)).map(|wall| air_within(faces, wall)).collect();
        if airs.len() > MAX_FOUND_POCKETS {
            return Err(format!("the part has more than {MAX_FOUND_POCKETS} openings in its top"));
        }
        let mut open = Vec::with_capacity(airs.len());
        for air in &airs {
            if self.open_above(air, tip)? {
                open.push(air.clone());
            }
        }
        let left_out = airs.len() - open.len();
        Ok((open, left_out))
    }

    /// Whether the opening whose air is `air` is open from the height `tip` up to above the
    /// bodies. Exactly: the air, shrunk by a hair, as a prism between those heights meets none
    /// of them. A step, a boss or a floor in the opening that high is in the way of the whole
    /// of it.
    fn open_above(&self, air: &Region2, tip: f64) -> Result<bool, String> {
        let (cut, height) = (self.plane.offset(tip + self.hair), self.top - tip);
        let within = solidcraft_cam::offset_region(air, self.hair).map_err(|e| format!("an opening's rim: {e}"))?;
        if within.is_empty() || height <= 2.0 * self.hair {
            return Ok(false);
        }
        let prisms = solidcraft_kernel::extrude(&cut, &within, self.hair, height).map_err(|e| format!("checking an opening's rim: {e}"))?;
        for &(name, body) in self.bodies {
            for prism in &prisms {
                let met = solidcraft_kernel::boolean(prism, body, BoolOp::Intersect)
                    .map_err(|e| format!("checking an opening's rim against body '{name}': {e}"))?;
                if met.is_some() {
                    return Ok(false);
                }
            }
        }
        Ok(true)
    }

    /// Whether anything is seen above `pocket` from places a hair inside its wall and round its
    /// islands, looking straight up from just above its floor: a sample, for the list.
    fn seen_above(&self, pocket: &Pocket) -> bool {
        let up = self.plane.normal();
        let Ok(within) = solidcraft_cam::offset_region(&pocket.region, self.hair) else { return true };
        let loops = within.iter().flat_map(|region| std::iter::once(&region.outer).chain(&region.holes));
        let places: Vec<Vec2> = loops.flat_map(|l| l.segs.iter().flat_map(|seg| [seg.start(), seg.mid()])).collect();
        let step = places.len().div_ceil(MAX_LOOKS).max(1);
        places.iter().step_by(step).chain(std::iter::once(&pocket.region.interior_point())).any(|&at| {
            let from = self.plane.to_world(at) + up * (pocket.floor + 2.0 * self.hair);
            self.bodies.iter().any(|(_, body)| !matches!(body.fire_ray(from, up), Ok(None)))
        })
    }

    /// The pockets of the bodies, those with a wall all round and those open at a side: one
    /// for each flat face that faces up below the top, each with a point on its floor. A floor
    /// in several faces is one pocket. The largest first. Nothing was seen above any of them
    /// from places along its wall ([`Finder::seen_above`]); a pocket is checked exactly when it
    /// is cut.
    pub(super) fn pockets(&mut self) -> Result<Vec<(Vec2, Pocket)>, String> {
        let up = self.plane.normal();
        // A point on every flat face that faces up below the top.
        let mut floors: Vec<(f64, Vec2)> = Vec::new();
        for &(name, body) in self.bodies {
            if body.is_mesh() {
                continue;
            }
            let count = body.face_count();
            if count > MAX_FLOOR_FACES {
                return Err(format!("body '{name}' has more than {MAX_FLOOR_FACES} faces"));
            }
            for face in 0..count {
                let Some(on) = body.planar_face(face).filter(|on| on.normal().dot(up) >= FACING) else { continue };
                let height = self.plane.height(on.origin);
                if self.top - height <= 2.0 * self.hair {
                    continue;
                }
                // A face that can't be read in the plane (a free-form edge it can't follow) is
                // not offered; cutting it would say why.
                let Ok(rings) = face_rings(body, face, &self.plane) else { continue };
                let mut rings = rings.into_iter();
                let Some(outer) = rings.next() else { continue };
                let on_it = Region2 { outer: outer.ccw(), holes: rings.map(|ring| ring.ccw().reversed()).collect() }.interior_point();
                floors.push((height, on_it));
            }
        }
        floors.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.y.total_cmp(&b.1.y)).then(a.1.x.total_cmp(&b.1.x)));
        let mut found: Vec<(Vec2, Pocket)> = Vec::new();
        for (height, at) in floors {
            // A floor in several faces is found once.
            let on_its_floor =
                |pocket: &Pocket| pocket.region.contains(at) || pocket.open.as_ref().is_some_and(|open| open.floor.iter().any(|f| f.contains(at)));
            if found.iter().any(|(_, pocket)| (pocket.floor - height).abs() <= self.hair && on_its_floor(pocket)) {
                continue;
            }
            if let Ok(pocket) = self.region_at(at)
                && !self.seen_above(&pocket)
            {
                if found.len() >= MAX_FOUND_POCKETS {
                    return Err(format!("the part has more than {MAX_FOUND_POCKETS} pockets"));
                }
                found.push((at, pocket));
            }
        }
        found.sort_by(|a, b| b.1.region.area().total_cmp(&a.1.region.area()).then(b.1.floor.total_cmp(&a.1.floor)));
        Ok(found)
    }
}

/// The flat faces of `body` that face up at the height `floor` above `plane` and are joined to
/// its face `face` through edges they share, that face first: the rings of each read in the
/// plane, the ring round the others first.
fn floor_faces(body: &Body, face: usize, plane: &Plane, floor: f64, hair: f64) -> Result<Vec<Vec<Loop2>>, String> {
    let count = body.face_count();
    if count > MAX_FLOOR_FACES {
        return Err(format!("it has more than {MAX_FLOOR_FACES} faces"));
    }
    let up = plane.normal();
    let record = body.record();
    let edges_of = |f: usize| -> Vec<usize> {
        let loops = record.faces.get(f).map(|face| face.loops.as_slice()).unwrap_or_default();
        loops
            .iter()
            .filter_map(|l| match l {
                Loop::Edges { edges } => Some(edges.as_slice()),
                Loop::Vertex { .. } => None,
            })
            .flatten()
            .map(|used| used.edge as usize)
            .collect()
    };
    let level = |f: usize| body.planar_face(f).is_some_and(|on| on.normal().dot(up) >= FACING && (plane.height(on.origin) - floor).abs() <= hair);
    let mut joined = vec![face];
    let mut edges: std::collections::BTreeSet<usize> = edges_of(face).into_iter().collect();
    let mut others: Vec<usize> = (0..count).filter(|&f| f != face && level(f)).collect();
    loop {
        let (now, later): (Vec<usize>, Vec<usize>) = others.into_iter().partition(|&f| edges_of(f).iter().any(|edge| edges.contains(edge)));
        if now.is_empty() {
            break;
        }
        for &f in &now {
            edges.extend(edges_of(f));
        }
        joined.extend(now);
        others = later;
    }
    joined.into_iter().map(|f| face_rings(body, f, plane)).collect()
}

/// The air within `wall`, a hole of one of `faces` (each a cut face's rings, the outer one
/// first): the wall's outline, with the cut faces inside it as its holes: material standing
/// there, islands, but for those that stand inside another one (in a pocket of an island).
fn air_within(faces: &[Vec<Loop2>], wall: &Loop2) -> Region2 {
    let inside: Vec<&Loop2> =
        faces.iter().filter_map(|rings| rings.first()).filter(|outer| outer.segs.first().is_some_and(|seg| wall.contains(seg.start()))).collect();
    let islands = inside.iter().filter(|outer| {
        !inside.iter().any(|other| !std::ptr::eq(*other, **outer) && outer.segs.first().is_some_and(|seg| other.contains(seg.start())))
    });
    Region2 { outer: wall.ccw(), holes: islands.map(|island| island.ccw().reversed()).collect() }
}
