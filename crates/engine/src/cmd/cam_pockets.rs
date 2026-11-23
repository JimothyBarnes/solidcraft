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
//! That last check is a boolean for every pocket. A pocket that is cut is always checked so; the
//! list of a part's pockets, which a form shows as it opens, looks up from places along each
//! pocket's wall with rays instead (a sample, as the holes' list is), so a part with dozens of
//! counterbores lists at once. What the rays miss is refused, with the reason, when it is cut.
use solidcore_record::{Loop2, Plane, Region2, Vec2};
use solidcraft_geom::prelude::*;
use solidcraft_kernel::{Body, BoolOp};

use super::cam_outline::{FACING, cut_faces, hair_of};

/// The most faces looked at for floors in one body.
const MAX_FLOOR_FACES: usize = 20_000;
/// The most pockets listed.
pub(super) const MAX_FOUND_POCKETS: usize = 1_000;
/// The most places along a pocket's wall its list looks up from.
const MAX_LOOKS: usize = 200;

/// A closed pocket, in the coordinates of the plane it was looked for in.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Pocket {
    /// Its floor's height above the plane.
    pub floor: f64,
    /// The air just above its floor: the wall's outline, with the islands as holes.
    pub region: Region2,
}

/// What is under a point of the plane, seen from above.
enum Under {
    /// Nothing of the bodies: the open, or an opening through them.
    Nothing,
    /// A flat face that faces up, this high above the plane.
    Floor(f64),
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
        self.clear_above(&pocket, at)?;
        Ok(pocket)
    }

    /// The floor under the point `at` of the plane and the air just above it, whatever is over it.
    fn region_at(&mut self, at: Vec2) -> Result<Pocket, String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let floor = match self.under(at)? {
            Under::Nothing => return Err(format!("there is nothing of the part under {here}")),
            Under::NotFlat => return Err(format!("the part is not flat and facing up at {here}: a pocket is named by a point on its floor")),
            Under::Floor(floor) => floor,
        };
        if self.top - floor <= 2.0 * self.hair {
            return Err(format!("{here} is on top of the part, not on the floor of a pocket"));
        }
        let Some(region) = self.air_at(floor, at)? else {
            return Err(format!("the pocket at {here} is open to the outside of the part: only pockets with walls all round are cut, for now"));
        };
        Ok(Pocket { floor, region })
    }

    /// What is under the point `at` of the plane, looking down from above everything.
    fn under(&self, at: Vec2) -> Result<Under, String> {
        let (up, above) = (self.plane.normal(), self.top + 1.0);
        let from = self.plane.to_world(at) + up * above;
        let mut floor: Option<(f64, &Body, usize)> = None;
        for &(name, body) in self.bodies {
            if body.is_mesh() {
                return Err(format!("body '{name}' is a mesh and has no exact faces: convert it to a solid first"));
            }
            if let Some(hit) = body.fire_ray(from, -up).map_err(|e| format!("body '{name}': {e}"))?
                && floor.is_none_or(|(nearest, ..)| hit.depth < nearest)
            {
                floor = Some((hit.depth, body, hit.face));
            }
        }
        let Some((depth, body, face)) = floor else { return Ok(Under::Nothing) };
        if !body.planar_face(face).is_some_and(|on| on.normal().dot(up) >= FACING) {
            return Ok(Under::NotFlat);
        }
        // Without the last bits of the ray's arithmetic, so that a floor at -7 reads -7.
        Ok(Under::Floor(((above - depth) * 1e9).round() / 1e9))
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
            let window = Pocket { floor: lowest, region };
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
                Under::Floor(floor) => floor,
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
        let opening = Pocket { floor, region };
        self.clear_above(&opening, at)?;
        Ok(opening)
    }

    /// Checks that nothing is in the way above `pocket` (found at `at`): its walls go straight
    /// down to its floor, and nothing hangs over it. Exactly: the region, shrunk by a hair, as a
    /// prism from above the floor to above the bodies meets none of them.
    fn clear_above(&self, pocket: &Pocket, at: Vec2) -> Result<(), String> {
        let here = format!("X {} Y {}", at.x, at.y);
        let (cut, height) = (self.plane.offset(pocket.floor + self.hair), self.top - pocket.floor);
        let within = solidcraft_cam::offset_region(&pocket.region, self.hair).map_err(|e| format!("the pocket at {here}: {e}"))?;
        if within.is_empty() || height <= 2.0 * self.hair {
            return Ok(());
        }
        let prisms = solidcraft_kernel::extrude(&cut, &within, self.hair, height).map_err(|e| format!("checking the pocket at {here}: {e}"))?;
        for &(name, body) in self.bodies {
            for prism in &prisms {
                let met = solidcraft_kernel::boolean(prism, body, BoolOp::Intersect)
                    .map_err(|e| format!("checking the pocket at {here} against body '{name}': {e}"))?;
                if met.is_some() {
                    return Err(format!(
                        "something of body '{name}' is in the way above the floor of the pocket at {here} (a fillet at the foot of its walls, an undercut or an overhang): it can't be cut straight down"
                    ));
                }
            }
        }
        Ok(())
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

    /// The closed pockets of the bodies: one for each flat face that faces up below the top and
    /// is the floor of one, each with a point on its floor. A floor in several faces is one
    /// pocket. The largest first. Nothing was seen above any of them from places along its wall
    /// ([`Finder::seen_above`]); a pocket is checked exactly when it is cut.
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
                let Ok(rings) = super::cam_outline::face_rings(body, face, &self.plane) else { continue };
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
            if found.iter().any(|(_, pocket)| (pocket.floor - height).abs() <= self.hair && pocket.region.contains(at)) {
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
