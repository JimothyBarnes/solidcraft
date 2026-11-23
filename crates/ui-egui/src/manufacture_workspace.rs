//! The Manufacture workspace: setups, tools and operations of the CAM work, on the model, with
//! its own camera. Everything it does is a `cam.*` command (data in, commands out, as RFC 0015
//! asks of plugin workspaces), so it can move behind a plugin later without a rewrite.
//!
//! Entering keeps the Design camera and restores Manufacture's own; leaving does the reverse
//! (UX.md §2: switching keeps both cameras).
use crate::SolveApp;
use solidcraft_viewport::Camera;

pub(crate) mod browser;
pub(crate) mod dialogs;
pub(crate) mod scene;

/// Manufacture's ribbon tabs.
pub const TABS: &[&str] = &["MANUFACTURE"];

/// The ribbon's Milling panel, in its order: what an end mill does, a hole bored with one too.
/// The first four are its buttons (a panel has four at most) and the rest are in its menu, from
/// where one can be pinned to the ribbon (`crate::workspace` has the panel's row).
pub(crate) const MILLING: &[&str] = &["cam.op.face", "cam.op.contour", "cam.op.pocket", "cam.op.chamfer", "cam.op.bore"];

/// How the workspace menu shows Manufacture.
pub(crate) const STYLE: crate::ui_kit::WorkspaceStyle = crate::ui_kit::WorkspaceStyle {
    slug: "manufacture",
    description: "ui.workspace_manufacture_cam_description",
    tabs: Some("ui.workspace_manufacture_tabs"),
    ramp: None,
};

/// Manufacture's row in the workspace menu: current while shown, waiting while an operation is under way.
pub(crate) fn menu_item(app: &SolveApp) -> crate::ui_kit::Item {
    crate::context_menu::act("ui.workspace.manufacture", "Manufacture", "ws_manufacture")
        .with(serde_json::json!({"workspace":"manufacture"}))
        .key(if active(app) { "current" } else { "" })
        .on(entry_open(app))
}

#[derive(Default)]
pub struct State {
    /// The Design camera and projection while Manufacture is shown.
    design_camera: Option<(Camera, bool)>,
    /// Manufacture's own camera and projection, kept while Design is shown.
    manufacture_camera: Option<(Camera, bool)>,
    /// The Design tab to go back to.
    previous_tab: Option<String>,
    /// The setup the viewport draws, by id; the design's first when `None` or gone.
    pub(crate) active_setup: Option<String>,
    /// What the viewport draws of it, kept until the design changes.
    pub(crate) scene: std::sync::Mutex<scene::Cache>,
    /// The open form (Add Tool, Import Tools, New Setup, Face, Drill, Post Process), if any.
    pub(crate) dialog: Option<dialogs::Dialog>,
}

pub fn active(app: &SolveApp) -> bool {
    TABS.contains(&app.ui.tab.as_str())
}

/// Whether Manufacture can be entered now: not while a sketch, tool, dialog or load is under way.
pub fn entry_open(app: &SolveApp) -> bool {
    app.session.active_sketch.is_none() && app.tool.is_none() && app.dialog.is_none() && !app.file_load.busy()
}

pub fn enter(app: &mut SolveApp, manufacture: bool) -> Result<(), String> {
    if manufacture {
        if !entry_open(app) {
            return Err(solidcraft_ui_text::tr("manufacture.error_finish_operation").into());
        }
        if !active(app) {
            app.manufacture_workspace.previous_tab = Some(app.ui.tab.clone());
            app.manufacture_workspace.design_camera = Some((app.cam, app.ui.perspective));
            if let Some((camera, perspective)) = app.manufacture_workspace.manufacture_camera.take() {
                app.cam = camera;
                app.ui.up_axis = camera.up;
                app.ui.perspective = perspective;
            }
            app.ui.tab = "MANUFACTURE".into();
        }
    } else if active(app) {
        app.manufacture_workspace.manufacture_camera = Some((app.cam, app.ui.perspective));
        if let Some((camera, perspective)) = app.manufacture_workspace.design_camera.take() {
            app.cam = camera;
            app.ui.up_axis = camera.up;
            app.ui.perspective = perspective;
        }
        app.ui.tab = app.manufacture_workspace.previous_tab.take().unwrap_or_else(|| "MODEL".into());
    }
    app.ui.ribbon_context = None;
    app.cancel_view_animation();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::scenario::Harness;
    use serde_json::json;

    fn harness() -> Harness {
        let mut h = Harness::new();
        h.app.home.open = false;
        h
    }

    #[test]
    fn the_workspace_menu_lists_manufacture_and_enters_it() {
        let mut h = harness();
        let item =
            crate::context_menu::items(&h.app, &crate::context_menu::Target::Workspace).into_iter().find(|item| item.label == "Manufacture").unwrap();
        assert!(item.enabled && item.id == "ui.workspace.manufacture");
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        assert!(active(&h.app));
        assert_eq!(crate::workspace::visible_tabs(&h.app), TABS);
        assert_eq!(crate::workspace::destination(solidcraft_engine::find_command("cam.setup.create").unwrap()), Some(("MANUFACTURE", "SETUP")));
    }

    #[test]
    fn switching_to_manufacture_and_back_keeps_both_cameras() {
        // UX.md §6 P0: Design's camera and Manufacture's are each kept.
        let mut h = harness();
        h.app.cam.distance = 123.0;
        let design = h.app.cam;
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        h.app.cam.distance = 456.0;
        let manufacture = h.app.cam;
        crate::workspace::enter(&mut h.app, "design").unwrap();
        assert!(!active(&h.app));
        assert_eq!(h.app.cam, design);
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        assert_eq!(h.app.cam, manufacture);
        // Leaving for Drawing or Render goes through Design's camera too.
        crate::workspace::enter(&mut h.app, "drawing").unwrap();
        assert!(!active(&h.app) && crate::drawing_workspace::active(&h.app));
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        assert!(active(&h.app) && !crate::drawing_workspace::active(&h.app));
        assert_eq!(h.app.cam, manufacture);
    }

    #[test]
    fn the_control_channel_enters_manufacture_and_reports_it() {
        let mut h = harness();
        assert_eq!(h.call("ui.workspace", json!({"workspace": "manufacture"}))["result"]["workspace"], "manufacture");
        assert!(active(&h.app));
        assert_eq!(h.call("ui.workspace", json!({}))["result"]["workspace"], "manufacture");
        assert_eq!(h.call("ui.workspace", json!({"workspace": "design"}))["result"]["workspace"], "design");
        assert!(!active(&h.app));
    }

    #[test]
    fn a_setup_made_from_the_ribbon_is_saved_in_the_design() {
        let mut h = harness();
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        h.app.run("cam.setup.create", json!({})).unwrap();
        let data = h.app.run("cam.data", json!({})).unwrap();
        assert_eq!(data["setups"], 1);
        assert_eq!(data["data"]["setups"][0]["name"], "Setup 1");
    }

    /// The labels of the Browser rows drawn this frame.
    fn browser_labels(h: &mut Harness) -> Vec<String> {
        h.frames(3);
        crate::browser::drawn_rows().into_iter().map(|(label, ..)| label).collect()
    }

    #[test]
    fn the_browser_lists_setups_operations_and_tools() {
        let mut h = harness();
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        let tool = json!({
            "number": 3, "name": "6 mm flat", "kind": "flat_end_mill",
            "geometry": {"diameter": 6.0, "corner_radius": 0.0, "flute_length": 18.0, "shoulder_length": 0.0, "overall_length": 60.0,
                         "shank_diameter": 6.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
            "flutes": 3, "holder": {"stick_out": 25.0, "segments": []}, "length_offset": 3, "diameter_offset": 3,
            "coolant": "flood", "cutting_data": [], "overrides": {"rpm": null, "feed": null}
        });
        h.app.run("cam.tool.create", json!({"tool": tool})).unwrap();
        h.app.run("cam.setup.create", json!({"name": "Op 1", "work_offset": 2})).unwrap();
        let path = json!({"start": [0.0, 0.0, 50.0], "moves": [{"kind": "rapid", "to": [0.0, 0.0, 5.0]}]});
        h.app.run("cam.op.manual", json!({"setup": "setup-1", "tool": 3, "toolpath": path, "name": "Face"})).unwrap();
        let labels = browser_labels(&mut h);
        for expected in ["Op 1 (G55)", "Face (T3)"] {
            assert!(labels.iter().any(|l| l == expected), "{expected:?} not in {labels:?}");
        }
        // Back in Design the Browser shows the model again, not the setups.
        crate::workspace::enter(&mut h.app, "design").unwrap();
        assert!(!browser_labels(&mut h).iter().any(|l| l == "Op 1 (G55)"));
    }

    /// A 60 x 40 x 20 block, a 20 mm end mill with data for aluminium, the vise setup and a facing.
    fn faced_block(h: &mut Harness) {
        h.app.run("solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0]})).unwrap();
        let tool = json!({
            "number": 4, "name": "20 mm flat end mill", "kind": "flat_end_mill",
            "geometry": {"diameter": 20.0, "corner_radius": 0.0, "flute_length": 30.0, "shoulder_length": 0.0, "overall_length": 100.0,
                         "shank_diameter": 20.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
            "flutes": 4, "holder": {"stick_out": 50.0, "segments": []}, "length_offset": 4, "diameter_offset": 4, "coolant": "flood",
            "cutting_data": [{"material": "aluminium", "surface_speed": 200.0, "chip_load": 0.05, "plunge_factor": 0.5, "ramp_factor": 0.5}],
            "overrides": {"rpm": null, "feed": null}
        });
        h.app.run("cam.tool.create", json!({"tool": tool})).unwrap();
        h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
        h.app.run("cam.op.face", json!({"setup": "setup-1", "tool": 4})).unwrap();
    }

    #[test]
    fn the_viewport_draws_the_active_setups_stock_origin_and_toolpaths() {
        use crate::gpu::LINE_SIZE;
        let mut h = harness();
        faced_block(&mut h);
        // Nothing of Manufacture is drawn in Design.
        assert_eq!((scene::counts(&h.app), scene::key(&h.app)), ((0, 0, 0), 0));
        let design = crate::viewport::build_highlight(&h.app).merged();
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        // The stock's six sides (two triangles each); its twelve edges and the origin's three
        // arms; the facing's eleven pieces (three rapids, a plunge, four passes, three steps).
        assert_eq!(scene::counts(&h.app), (12, 15, 11), "{:?}", scene::trouble(&h.app));
        // The highlight scene, the path a real frame takes, gains exactly those.
        let shown = crate::viewport::build_highlight(&h.app).merged();
        assert_eq!((shown.lines.len() - design.lines.len()) / LINE_SIZE, 11);
        assert_eq!((shown.guide_lines.len() - design.guide_lines.len()) / LINE_SIZE, 15);
        assert_eq!((shown.guide_trans.len() - design.guide_trans.len()) / crate::gpu::TRI_SIZE, 36);
        // A second setup is drawn once its radio is picked: its stock and origin, no toolpath yet.
        h.app.run("cam.setup.create", json!({"name": "Op 2"})).unwrap();
        assert_eq!(scene::counts(&h.app), (12, 15, 11));
        let first = scene::key(&h.app);
        h.app.manufacture_workspace.active_setup = Some("setup-2".into());
        assert_ne!(scene::key(&h.app), first);
        assert_eq!(scene::counts(&h.app), (12, 15, 0));
        // A setup that can't be placed draws nothing and says why.
        h.app.run("cam.setup.edit", json!({"id": "setup-2", "model_bodies": ["Gone"]})).unwrap();
        assert_eq!(scene::counts(&h.app), (0, 0, 0));
        assert!(scene::trouble(&h.app).unwrap().contains("'Gone' is not in the design"));
        // A setup that is gone falls back to the first.
        h.app.run("cam.setup.delete", json!({"id": "setup-2"})).unwrap();
        assert_eq!(scene::counts(&h.app), (12, 15, 11));
        assert!(scene::trouble(&h.app).is_none());
    }

    #[test]
    fn the_browser_marks_the_active_setup_and_says_why_one_cannot_be_drawn() {
        let mut h = harness();
        faced_block(&mut h);
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        h.app.run("cam.setup.create", json!({"name": "Op 2", "model_bodies": ["Gone"]})).unwrap();
        let labels = browser_labels(&mut h);
        assert!(labels.iter().any(|l| l == "Setup 1 (G54)") && labels.iter().any(|l| l == "Face 1 (T4)"), "{labels:?}");
        let why = "the setup's model body 'Gone' is not in the design";
        assert!(h.widget(why).is_none(), "the first setup is the one drawn, and it is fine");
        h.app.manufacture_workspace.active_setup = Some("setup-2".into());
        h.frames(3);
        assert!(h.widget(why).is_some(), "the Browser says why the active setup is not drawn");
    }

    #[test]
    fn the_browser_says_why_an_operation_has_no_toolpath() {
        let mut h = harness();
        faced_block(&mut h);
        crate::workspace::enter(&mut h.app, "manufacture").unwrap();
        h.frames(3);
        assert!(scene::operation_trouble(&h.app, "op-1").is_none());
        // Without its stock material the setup can't give the facing its feeds: the reason is
        // under the operation's row, and its toolpath is gone from the viewport.
        h.app.run("cam.setup.edit", json!({"id": "setup-1", "material": ""})).unwrap();
        h.frames(3);
        let why = scene::operation_trouble(&h.app, "op-1").unwrap();
        assert!(why.starts_with("operation op-1 (Face 1): ") && why.contains("stock material"), "{why}");
        assert!(h.widget(&why).is_some(), "{why}");
        assert_eq!(scene::counts(&h.app), (12, 15, 0));
    }

    #[test]
    fn manufacture_waits_while_an_operation_is_under_way() {
        let mut h = harness();
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        assert!(h.app.session.active_sketch.is_some());
        assert!(crate::workspace::enter(&mut h.app, "manufacture").is_err());
        assert!(!active(&h.app));
    }
}
