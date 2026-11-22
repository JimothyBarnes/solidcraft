//! What Manufacture draws in the viewport (UX.md §2): the active setup's stock box and work
//! origin on the model, and its operations' toolpaths. It reads the engine's view of the setup
//! (`solidcraft_engine::cmd::manufacture_view`), worked out once per change to the design and kept
//! until the next. It goes into the highlight scene, which is built again only when its key
//! changes. An open form adds what it would make (`super::dialogs::Shown`): New Setup its stock
//! and origin in place of the active setup's, an operation's form its toolpath.
//!
//! Colours: Manufacture has no theme tokens of its own yet (one is a line in every theme file),
//! so it borrows tokens by meaning: the reference-geometry colours for the stock, the one X/Y/Z
//! set for the origin, the manipulator blue for cutting moves, the joint amber for rapids and the
//! proposed-geometry green for plunges. Each already has its colour-vision variants.
use std::hash::{Hash, Hasher};

use solidcraft_engine::cmd::{Drawn, Placement, SetupView, manufacture_view};
use solidcraft_theme::Tokens;

use super::dialogs::Shown;
use crate::SolveApp;
use crate::gpu::GpuScene;
use crate::viewport::c4;

/// The most toolpath pieces drawn; a longer program is drawn up to here.
const MAX_PIECES: usize = 2_000_000;
/// The origin triad's arms, as a share of the stock's longest side.
const TRIAD_SHARE: f64 = 0.2;

/// The active setup's view, kept until the design or the active setup changes.
#[derive(Default)]
pub(crate) struct Cache {
    key: Option<u64>,
    view: Option<Result<Option<SetupView>, String>>,
}

/// What of Manufacture the highlight scene depends on, beyond the design's revision.
pub(crate) fn key(app: &SolveApp) -> u64 {
    if !super::active(app) {
        return 0;
    }
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.manufacture_workspace.active_setup.hash(&mut h);
    form(app).map(|(key, _)| key).hash(&mut h);
    h.finish().max(1)
}

/// What the open form adds, with the number that changes when that does.
fn form(app: &SolveApp) -> Option<(u64, Shown<'_>)> {
    app.manufacture_workspace.dialog.as_ref().and_then(|dialog| dialog.shown())
}

/// Why the active setup can't be drawn, if it can't (no bodies to machine, a turned work
/// coordinate system): for the browser to say.
pub(crate) fn trouble(app: &SolveApp) -> Option<String> {
    with_view(app, |view| view.as_ref().err().cloned())
}

fn with_view<T>(app: &SolveApp, read: impl FnOnce(&Result<Option<SetupView>, String>) -> T) -> T {
    let state = &app.manufacture_workspace;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    app.session.uid.hash(&mut h);
    app.session.revision.hash(&mut h);
    state.active_setup.hash(&mut h);
    let key = h.finish();
    let mut cache = state.scene.lock().unwrap_or_else(|e| e.into_inner());
    if cache.key != Some(key) {
        cache.view = Some(manufacture_view(&app.session, state.active_setup.as_deref()));
        cache.key = Some(key);
    }
    match &cache.view {
        Some(view) => read(view),
        None => read(&Ok(None)),
    }
}

/// Why an operation of the active setup has no toolpath, if it has none: for the browser to say.
pub(crate) fn operation_trouble(app: &SolveApp, id: &str) -> Option<String> {
    with_view(app, |view| match view {
        Ok(Some(view)) => view.operations.iter().find(|o| o.id == id).and_then(|o| o.pieces.as_ref().err().cloned()),
        _ => None,
    })
}

/// Adds the active setup, and what an open form would make, to the highlight scene: nothing
/// outside Manufacture.
pub(crate) fn draw(app: &SolveApp, sc: &mut GpuScene) {
    if !super::active(app) {
        return;
    }
    let form = form(app).map(|(_, shown)| shown);
    if let Some(Shown::Setup(placement)) = form {
        placed(sc, placement);
        return;
    }
    let mut left = MAX_PIECES;
    with_view(app, |view| {
        if let Ok(Some(view)) = view {
            placed(sc, &view.placement);
            for operation in &view.operations {
                if let Ok(pieces) = &operation.pieces {
                    toolpath(sc, &view.placement, pieces, &mut left);
                }
            }
        }
    });
    if let Some(Shown::Operation(preview)) = form {
        toolpath(sc, &preview.placement, &preview.pieces, &mut left);
    }
}

/// A WCS point in the model's coordinates.
fn on_model(p: &Placement, x: f64, y: f64, z: f64) -> [f32; 3] {
    let at = |o: f64, a: f64, b: f64, c: f64| (o + a * x + b * y + c * z) as f32;
    [
        at(p.origin.x, p.x_axis.x, p.y_axis.x, p.z_axis.x),
        at(p.origin.y, p.x_axis.y, p.y_axis.y, p.z_axis.y),
        at(p.origin.z, p.x_axis.z, p.y_axis.z, p.z_axis.z),
    ]
}

/// A setup's stock and work origin.
fn placed(sc: &mut GpuScene, p: &Placement) {
    let t = Tokens::get();
    // The stock: six translucent sides and twelve edges, reference geometry like a construction
    // plane (depth-tested, never cut by a section).
    let (lo, hi) = (p.stock.min, p.stock.max);
    let corner =
        |i: usize| on_model(p, if i & 1 == 0 { lo.x } else { hi.x }, if i & 2 == 0 { lo.y } else { hi.y }, if i & 4 == 0 { lo.z } else { hi.z });
    let fill = sc.color(c4(t.construction_plane));
    for side in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
        for i in [0, 1, 2, 0, 2, 3] {
            sc.guide_tri(corner(side[i]), fill);
        }
    }
    let edge = sc.color(c4(t.construction_geom));
    for (a, b) in [(0, 1), (2, 3), (4, 5), (6, 7), (0, 2), (1, 3), (4, 6), (5, 7), (0, 4), (1, 5), (2, 6), (3, 7)] {
        sc.guide_line(corner(a), corner(b), edge, 1.2);
    }
    // The work origin: X, Y and Z arms in the one set of axis colours.
    let size = p.stock.size();
    let arm = (TRIAD_SHARE * size.x.max(size.y).max(size.z)).max(1.0);
    for (to, colour) in [([arm, 0.0, 0.0], t.axis_x), ([0.0, arm, 0.0], t.axis_y), ([0.0, 0.0, arm], t.axis_z)] {
        let colour = sc.color(c4(colour));
        sc.guide_line(on_model(p, 0.0, 0.0, 0.0), on_model(p, to[0], to[1], to[2]), colour, 2.5);
    }
}

/// A toolpath's pieces, up to `left` of them: rapids thin, cuts heavier; where the model hides a
/// piece it shows faintly.
fn toolpath(sc: &mut GpuScene, p: &Placement, pieces: &[(Drawn, solidcore_record::Vec3, solidcore_record::Vec3)], left: &mut usize) {
    let t = Tokens::get();
    let (cut, rapid, plunge) = (sc.color(c4(t.manipulator)), sc.color(c4(t.joint_manipulator)), sc.color(c4(t.preview_sketch)));
    let take = pieces.len().min(*left);
    *left -= take;
    sc.reserve_lines(take, take);
    for (kind, a, b) in pieces.iter().take(take) {
        let (colour, width) = match kind {
            Drawn::Rapid => (rapid, 1.0),
            Drawn::Cut => (cut, 1.8),
            Drawn::Plunge => (plunge, 1.8),
        };
        let (a, b) = (on_model(p, a.x, a.y, a.z), on_model(p, b.x, b.y, b.z));
        sc.line(a, b, colour, width, false);
        sc.hidden_line(a, b, colour, 1.0);
    }
}

/// How much of a setup the scene holds, for tests: translucent triangles, reference lines and
/// toolpath lines.
#[cfg(test)]
pub(crate) fn counts(app: &SolveApp) -> (usize, usize, usize) {
    let mut sc = GpuScene::default();
    draw(app, &mut sc);
    (sc.guide_trans.len() / crate::gpu::TRI_SIZE / 3, sc.guide_lines.len() / crate::gpu::LINE_SIZE, sc.lines.len() / crate::gpu::LINE_SIZE)
}
