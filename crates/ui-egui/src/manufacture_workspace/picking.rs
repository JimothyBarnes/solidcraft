//! Picking what a form cuts by clicking it on the model (decision 0030 of the CAM work).
//!
//! The picking is the app's own: a pick dialog that draws nothing
//! ([`crate::dialogs::Kind::ManufacturePick`]), as a plugin's dialog has, so the viewport lights
//! what is under the pointer and takes clicks as it does for Extrude's Profiles. The form that
//! is open reads the faces picked each frame ([`Picks::take`]) and says which of the things
//! found on the model they stand for.
//!
//! The picking starts when the pointer is first put to use on the model while the form is open
//! (moved there, or pressed there), not when the form opens: until then nothing about the app
//! is other than it was before a form could take picks.
use solidcore_record::Vec3;
use solidcraft_engine::Sel;
use solidcraft_engine::cmd::{HoleGroup, manufacture_hole_at};
use solidcraft_ui_text::tr;

use crate::SolveApp;
use crate::dialogs::{Dialog, Kind};
use crate::selection::{Accept, FACES, SNAP_POINTS, SelInput, VERTICES, same_item};

/// Whether the viewport picks for a Manufacture form.
pub(crate) fn active(app: &SolveApp) -> bool {
    matches!(app.dialog.as_ref().map(|d| &d.kind), Some(Kind::ManufacturePick))
}

/// Stops the picking, if a Manufacture form had it: what was picked goes with it.
pub(crate) fn stop(app: &mut SolveApp) {
    if active(app) {
        app.dialog = None;
    }
}

/// Starts the picking for the open form, unless it has started or something else has the
/// viewport (a tool, another dialog). `label` is what the form calls what it takes.
fn start(app: &mut SolveApp, label: &'static str) {
    start_taking(app, label, FACES);
}

/// [`start`], for picks of the kinds `accept` (faces, edges).
fn start_taking(app: &mut SolveApp, label: &'static str, accept: crate::selection::Accept) {
    if app.tool.is_none() && app.dialog.is_none() {
        app.dialog = Some(Dialog::new(Kind::ManufacturePick, vec![SelInput::new(label, accept, true)]));
    }
}

/// What the picking takes, while the viewport picks for a Manufacture form.
fn taking(app: &SolveApp) -> Option<Accept> {
    app.dialog.as_ref().filter(|d| matches!(d.kind, Kind::ManufacturePick)).and_then(|d| d.inputs.first()).map(|input| input.accept)
}

/// Lets go of a picking that takes other things than faces (edges, points), so that the form's
/// own picking of faces starts afresh (Contour, back from its edges to the openings found; Drill,
/// back from its points to holes).
pub(crate) fn faces_again(app: &mut SolveApp) {
    if taking(app).is_some_and(|accept| accept != FACES) {
        stop(app);
    }
}

/// What a picking of points takes: a vertex of the model, a point an edge or a face offers when
/// the pointer is on it (a hole's or a boss's centre, an edge's middle and ends, as upstream's
/// snap points wake them), or a place on a face.
const SPOTS: Accept = FACES | VERTICES | SNAP_POINTS;

/// For a form that takes points picked on the model (Drill's, decision 0040 of the CAM work):
/// starts a picking that takes them (in place of another), and takes the point clicked since the
/// last frame, if one was: a vertex, a centre woken on a round edge, or the place on a face.
/// Nothing is left lit: the form says what it took, and its toolpath shows it.
pub(crate) fn clicked_spot(app: &mut SolveApp, ctx: &egui::Context) -> Option<Vec3> {
    if taking(app).is_some_and(|accept| accept != SPOTS) {
        stop(app);
    }
    if on_model(app, ctx) {
        start_taking(app, "manufacture.drill_points", SPOTS);
    }
    let shown = shown(app)?;
    let point = shown.iter().rev().find_map(|sel| match sel {
        Sel::Vertex { point, .. } | Sel::Face { point, .. } => Some(*point),
        _ => None,
    });
    shown.clear();
    point
}

/// For a form that takes edges picked on the model (a contour's chains, decision 0038 of the CAM
/// work): starts a picking that takes edges (in place of one that took faces), and takes the
/// point of the edge clicked since the last frame, if one was. Nothing is left lit: the form says
/// what it took, and its toolpath shows it.
pub(crate) fn clicked_edge(app: &mut SolveApp, ctx: &egui::Context) -> Option<Vec3> {
    if taking(app).is_some_and(|accept| accept & crate::selection::EDGES == 0) {
        stop(app);
    }
    if on_model(app, ctx) {
        start_taking(app, "manufacture.contour_chains", crate::selection::EDGES);
    }
    let shown = shown(app)?;
    let point = shown.iter().rev().find_map(|sel| match sel {
        Sel::Edge { point, .. } => Some(*point),
        _ => None,
    });
    shown.clear();
    point
}

/// The faces picked for the open form, to read and to change: `None` while the viewport does
/// not pick for one.
fn shown(app: &mut SolveApp) -> Option<&mut Vec<Sel>> {
    let picker = app.dialog.as_mut().filter(|d| matches!(d.kind, Kind::ManufacturePick))?;
    picker.inputs.first_mut().map(|input| &mut input.items)
}

/// What is picked on the model for the open form now (to put back after a point is clicked for
/// one of its heights).
pub(crate) fn picked_now(app: &mut SolveApp) -> Vec<Sel> {
    shown(app).map(|items| items.clone()).unwrap_or_default()
}

/// For a form waiting for a point on the model (a height, decision 0037 of the CAM work): starts
/// the picking, and takes the point of the face clicked since the last frame, if one was, putting
/// back `before`, what was picked for the form, so its own picks are as they were.
pub(crate) fn clicked_point(app: &mut SolveApp, ctx: &egui::Context, before: &[Sel]) -> Option<Vec3> {
    if on_model(app, ctx) {
        start(app, "manufacture.height_picking");
    }
    let shown = shown(app)?;
    // A face newly picked, or one already picked and clicked again (which lets it go).
    let added = shown.iter().find(|sel| !before.iter().any(|b| same_item(b, sel)));
    let dropped = before.iter().find(|b| !shown.iter().any(|sel| same_item(b, sel)));
    let point = added.or(dropped).and_then(|sel| match sel {
        Sel::Face { point, .. } | Sel::Edge { point, .. } => Some(*point),
        _ => None,
    })?;
    *shown = before.to_vec();
    Some(point)
}

/// Whether the pointer is over the model's view, clear of every window (the form's own, a
/// menu).
fn on_model(app: &SolveApp, ctx: &egui::Context) -> bool {
    ctx.pointer_latest_pos().is_some_and(|at| {
        let clear = ctx.layer_id_at(at).is_none_or(|layer| layer.order == egui::Order::Background);
        clear && app.viewport.rect.is_some_and(|view| view.contains(at))
    })
}

/// The faces a form has had picked on the model, each with the thing it stands for: `K` is the
/// thing's place in the form's list of what was found.
pub(crate) struct Picks<K> {
    faces: Vec<(Sel, K)>,
    /// What the things were found on, as a number that changes when they do: a changed model
    /// forgets the picks.
    on: Option<u64>,
    /// Whether the last click on the model was on none of the things.
    pub(crate) missed: bool,
    /// Whether the pointer has been put to use on the model since the form opened: the picking
    /// starts then.
    awake: bool,
    /// Where the pointer was at the form's last frame, once there has been one.
    pointer: Option<Option<egui::Pos2>>,
}

impl<K: Clone + PartialEq> Picks<K> {
    pub(crate) fn new() -> Picks<K> {
        Picks { faces: Vec::new(), on: None, missed: false, awake: false, pointer: None }
    }

    /// Wakes the picking when the pointer is put to use on the model: moved there since the
    /// form's last frame, or pressed there. Where it merely rests as the form opens says
    /// nothing.
    fn wake(&mut self, app: &SolveApp, ctx: &egui::Context) {
        let at = ctx.pointer_latest_pos();
        let moved = self.pointer.is_some_and(|before| before != at);
        let pressed = ctx.input(|i| i.pointer.any_pressed());
        self.pointer = Some(at);
        self.awake |= (moved || pressed) && on_model(app, ctx);
    }

    /// Takes what was clicked on the model since the last frame. A face of a thing not yet
    /// picked picks the thing, a face of one already picked takes it out (as a second click on
    /// one face does), and a face of none is dropped and marked as missed. `found` is the
    /// number of the list the things are in, `label` what the form calls them, and `of` says
    /// which thing a picked point is on. The things picked, in the order they were picked,
    /// when they changed; `None` when they did not.
    pub(crate) fn take(
        &mut self,
        app: &mut SolveApp,
        ctx: &egui::Context,
        label: &'static str,
        found: Option<u64>,
        of: impl Fn(&SolveApp, Vec3) -> Option<K>,
    ) -> Option<Vec<K>> {
        self.wake(app, ctx);
        if self.awake {
            start(app, label);
        }
        let before: Vec<K> = self.faces.iter().map(|(_, thing)| thing.clone()).collect();
        // The picks were made on the things as they were found then: on others none stands;
        // and none stands while the viewport does not pick for the form.
        let shown = shown(app).filter(|_| self.on == found);
        let Some(shown) = shown else {
            if let Some(shown) = self::shown(app) {
                shown.clear();
            }
            (self.on, self.missed) = (found, false);
            self.faces.clear();
            return (!before.is_empty()).then(Vec::new);
        };
        let now = shown.clone();
        if now.len() == self.faces.len() && now.iter().zip(&self.faces).all(|(a, (b, _))| same_item(a, b)) {
            return None;
        }
        let mut standing: Vec<(Sel, K)> = Vec::new();
        let mut missed = false;
        for sel in now {
            if let Some(known) = self.faces.iter().find(|(old, _)| same_item(old, &sel)) {
                standing.push(known.clone());
                continue;
            }
            let Sel::Face { point, .. } = &sel else { continue };
            match of(app, *point) {
                Some(thing) if standing.iter().any(|(_, picked)| *picked == thing) => standing.retain(|(_, picked)| *picked != thing),
                Some(thing) => standing.push((sel, thing)),
                None => missed = true,
            }
        }
        self.missed = missed;
        self.faces = standing;
        // The viewport lights the faces that stand, and no others.
        if let Some(shown) = self::shown(app) {
            *shown = self.faces.iter().map(|(sel, _)| sel.clone()).collect();
        }
        let after: Vec<K> = self.faces.iter().map(|(_, thing)| thing.clone()).collect();
        (after != before).then_some(after)
    }

    /// The things picked and the points their faces were clicked at, in the order they were picked.
    pub(crate) fn picked(&self) -> Vec<(K, Vec3)> {
        self.faces
            .iter()
            .filter_map(|(sel, thing)| match sel {
                Sel::Face { point, .. } => Some((thing.clone(), *point)),
                _ => None,
            })
            .collect()
    }

    /// Keeps only the picks of the things `keep` takes: a form that cuts one group of things
    /// at a time lets go of the others' when a thing of another group is picked.
    pub(crate) fn keep(&mut self, app: &mut SolveApp, keep: impl Fn(&K) -> bool) {
        if self.faces.iter().all(|(_, thing)| keep(thing)) {
            return;
        }
        self.faces.retain(|(_, thing)| keep(thing));
        if let Some(shown) = shown(app) {
            *shown = self.faces.iter().map(|(sel, _)| sel.clone()).collect();
        }
    }

    /// Forgets the picks: the form's own row was used to choose.
    pub(crate) fn forget(&mut self, app: &mut SolveApp) {
        if let Some(shown) = shown(app) {
            shown.clear();
        }
        self.faces.clear();
        self.missed = false;
    }
}

/// What was clicked on the model for a form whose row offers all of a list of things (when
/// there are more than one), then each one: Pocket and Slot. One thing picked is the row's own
/// choice of it, none or every one is all of them, and several are a choice the row gains
/// after the things, which stands for the form's `picks`.
pub(crate) struct ListPicks {
    /// The faces clicked, each with the thing's place in the list.
    clicked: Picks<usize>,
    /// The choice as the clicks last left it: another one since is the row's own.
    left_at: usize,
}

impl ListPicks {
    pub(crate) fn new() -> ListPicks {
        ListPicks { clicked: Picks::new(), left_at: 0 }
    }

    /// Whether the last click on the model was on none of the things.
    #[cfg(test)]
    pub(crate) fn missed(&self) -> bool {
        self.clicked.missed
    }

    /// A choice from the form's row (`choice` is another than the clicks left) is the choice:
    /// what was picked on the model (`picks`) is forgotten. For the start of the form's frame.
    pub(crate) fn follow_row(&mut self, app: &mut SolveApp, choice: usize, picks: &mut Vec<usize>) {
        if choice != self.left_at {
            self.clicked.forget(app);
            picks.clear();
            self.left_at = choice;
        }
    }

    /// Takes what was clicked on the model since the last frame and makes it the row's
    /// `choice`: `count` things are in the list numbered `found`, `label` is what the form
    /// calls them and `of` says which one a picked point is on. Several picked, and not all,
    /// are left in `picks`. Whether what is cut changed.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn take(
        &mut self,
        app: &mut SolveApp,
        ctx: &egui::Context,
        label: &'static str,
        found: Option<u64>,
        count: usize,
        of: impl Fn(&SolveApp, Vec3) -> Option<usize>,
        choice: &mut usize,
        picks: &mut Vec<usize>,
    ) -> bool {
        let changed = match self.clicked.take(app, ctx, label, found, of) {
            None => false,
            Some(mut clicked) => {
                clicked.sort_unstable();
                // The row's first choice is all of them, when there are more than one.
                let first = usize::from(count > 1);
                let own = match clicked.as_slice() {
                    [] => Some(0),
                    every if every.len() == count => Some(0),
                    [one] => Some(first + one),
                    _ => None,
                };
                (*choice, *picks) = match own {
                    Some(own) => (own, Vec::new()),
                    None => (first + count, clicked),
                };
                true
            }
        };
        self.left_at = *choice;
        changed
    }

    /// What the form says of the picking under its row: that the things can be clicked
    /// (`hint`), or that the last click was on none (`missed`).
    pub(crate) fn says(&self, hint: &'static str, missed: &'static str) -> &'static str {
        tr(if self.clicked.missed { missed } else { hint })
    }
}

/// What the picks of holes are called.
const HOLES: &str = "Holes";

/// The holes picked on the model for a form that cuts one group of holes at a time (Drill and
/// Bore). The hole clicked last says which group is cut, and the holes of that group that are
/// picked are the ones cut: every hole of the group while none is picked, or when all are.
pub(crate) struct HolePicks {
    /// The holes of the group picked on the model, when they are not all of them: their places
    /// in the group, in order.
    pub(crate) picks: Vec<usize>,
    /// The faces clicked, each with its hole's group and its place in the group.
    clicked: Picks<(usize, usize)>,
    /// The group as the clicks last left it: another one since is the row's own choice.
    left_at: usize,
}

impl HolePicks {
    pub(crate) fn new() -> HolePicks {
        HolePicks { picks: Vec::new(), clicked: Picks::new(), left_at: 0 }
    }

    /// Whether the last click on the model was on no hole.
    #[cfg(test)]
    pub(crate) fn missed(&self) -> bool {
        self.clicked.missed
    }

    /// A group chosen from the form's row (`group` is another than the clicks left) is the
    /// choice: what was picked on the model is forgotten. For the start of the form's frame.
    pub(crate) fn follow_row(&mut self, app: &mut SolveApp, group: usize) {
        if group != self.left_at {
            self.clicked.forget(app);
            self.picks.clear();
            self.left_at = group;
        }
    }

    /// Takes what was clicked on the model since the last frame: `found` is the number of the
    /// list of `groups`, read for the setup `setup`, and `group` the one the form cuts, which
    /// becomes the group of the hole clicked last. Whether what is cut changed.
    pub(crate) fn take(
        &mut self,
        app: &mut SolveApp,
        ctx: &egui::Context,
        found: Option<u64>,
        setup: &str,
        groups: &[HoleGroup],
        group: &mut usize,
    ) -> bool {
        let on_hole = |app: &SolveApp, point| manufacture_hole_at(&app.session, setup, point).ok().flatten();
        let changed = match self.clicked.take(app, ctx, HOLES, found, on_hole) {
            None => false,
            Some(clicked) => {
                match clicked.last() {
                    // None left picked: every hole of the group the form is on.
                    None => self.picks.clear(),
                    Some(&(last, _)) => {
                        // A form cuts one group: the holes of the others are picked no longer.
                        self.clicked.keep(app, |(of, _)| *of == last);
                        let mut picks: Vec<usize> = clicked.iter().filter(|(of, _)| *of == last).map(|(_, hole)| *hole).collect();
                        picks.sort_unstable();
                        if groups.get(last).is_some_and(|all| all.holes.len() == picks.len()) {
                            picks.clear();
                        }
                        (*group, self.picks) = (last, picks);
                    }
                }
                true
            }
        };
        self.left_at = *group;
        changed
    }

    /// Forgets the picks: the form drills something else.
    pub(crate) fn forget(&mut self, app: &mut SolveApp) {
        self.clicked.forget(app);
        self.picks.clear();
    }

    /// The holes of `group` that are cut: the ones picked on the model, or every one.
    pub(crate) fn of(&self, group: &HoleGroup) -> Vec<[f64; 2]> {
        match self.picks.as_slice() {
            [] => group.holes.clone(),
            picks => picks.iter().filter_map(|at| group.holes.get(*at).copied()).collect(),
        }
    }

    /// What the form says of the picking under its row of holes: that holes can be clicked,
    /// how many of the group's `of` are picked, or that the last click was on no hole.
    pub(crate) fn says(&self, of: usize) -> String {
        match (self.clicked.missed, self.picks.len()) {
            (true, _) => tr("manufacture.hole_pick_missed").to_string(),
            (false, 0) => tr("manufacture.hole_pick_hint").to_string(),
            (false, count) => solidcraft_ui_text::format("manufacture.hole_picked", &[("count", &count.to_string()), ("of", &of.to_string())]),
        }
    }
}
