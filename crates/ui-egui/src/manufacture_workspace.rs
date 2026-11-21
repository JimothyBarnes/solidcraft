//! The Manufacture workspace: setups, tools and operations of the CAM work, on the model, with
//! its own camera. Everything it does is a `cam.*` command (data in, commands out, as RFC 0015
//! asks of plugin workspaces), so it can move behind a plugin later without a rewrite.
//!
//! Entering keeps the Design camera and restores Manufacture's own; leaving does the reverse
//! (UX.md §2: switching keeps both cameras).
use crate::SolveApp;
use solidcraft_viewport::Camera;

pub(crate) mod browser;

/// Manufacture's ribbon tabs.
pub const TABS: &[&str] = &["MANUFACTURE"];

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

    #[test]
    fn manufacture_waits_while_an_operation_is_under_way() {
        let mut h = harness();
        h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
        assert!(h.app.session.active_sketch.is_some());
        assert!(crate::workspace::enter(&mut h.app, "manufacture").is_err());
        assert!(!active(&h.app));
    }
}
