//! Manufacture's forms, used the way a person uses them: the ribbon's buttons, the fields, the
//! form's own buttons and the save prompt.
use serde_json::{Value, json};

use super::*;
use crate::manufacture_workspace::scene;
use crate::scenario::Harness;

/// The app in Manufacture, with nothing in the design.
fn harness() -> Harness {
    let mut h = Harness::new();
    h.app.home.open = false;
    crate::workspace::enter(&mut h.app, "manufacture").unwrap();
    h.frames(2);
    h
}

/// A 60 x 40 x 20 block to machine.
fn block(h: &mut Harness) {
    h.app.run("solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0]})).unwrap();
}

/// A click on a ribbon button.
fn ribbon(h: &mut Harness, id: &str) {
    h.step(&json!({"click": {"handle": format!("toolbar:{id}")}})).unwrap_or_else(|e| panic!("the ribbon's {id}: {e}"));
    h.frames(3);
}

/// A click on a command in a panel's menu, where the commands the panel has no button for are.
fn panel_menu(h: &mut Harness, panel: &str, id: &str) {
    h.step(&json!({"click": {"handle": format!("panel:MANUFACTURE:{panel}")}})).unwrap_or_else(|e| panic!("the {panel} panel's menu: {e}"));
    h.frames(3);
    h.step(&json!({"click": {"handle": format!("dropdown:{id}")}})).unwrap_or_else(|e| panic!("{id} in the {panel} panel's menu: {e}"));
    h.frames(3);
}

/// A click on a button of the open form.
fn press(h: &mut Harness, label: &str) {
    h.step(&json!({"widget": label})).unwrap_or_else(|e| panic!("{e}"));
    h.frames(3);
}

fn data(h: &mut Harness) -> Value {
    h.app.run("cam.data", json!({})).unwrap()["data"].clone()
}

fn open(h: &Harness) -> &'static str {
    match &h.app.manufacture_workspace.dialog {
        None => "none",
        Some(Dialog::Tool(_)) => "tool",
        Some(Dialog::Import(_)) => "import",
        Some(Dialog::Setup(_)) => "setup",
        Some(Dialog::Face(_)) => "face",
        Some(Dialog::Contour(_)) => "contour",
        Some(Dialog::Pocket(_)) => "pocket",
        Some(Dialog::Chamfer(_)) => "chamfer",
        Some(Dialog::Drill(_)) => "drill",
        Some(Dialog::Bore(_)) => "bore",
        Some(Dialog::Simulate(_)) => "simulate",
        Some(Dialog::Post(_)) => "post",
        Some(Dialog::Posted(_)) => "posted",
    }
}

fn tool_form(h: &mut Harness) -> &mut ToolForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Tool(form)) => form,
        _ => panic!("Add Tool is not open"),
    }
}

fn setup_form(h: &mut Harness) -> &mut SetupForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Setup(form)) => form,
        _ => panic!("New Setup is not open"),
    }
}

fn face_form(h: &mut Harness) -> &mut FaceForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Face(form)) => form,
        _ => panic!("Face is not open"),
    }
}

fn contour_form(h: &mut Harness) -> &mut ContourForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Contour(form)) => form,
        _ => panic!("Contour is not open"),
    }
}

fn pocket_form(h: &mut Harness) -> &mut PocketForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Pocket(form)) => form,
        _ => panic!("Pocket is not open"),
    }
}

fn simulate_form(h: &mut Harness) -> &mut SimForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Simulate(form)) => form,
        _ => panic!("Simulate is not open"),
    }
}

fn drill_form(h: &mut Harness) -> &mut DrillForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Drill(form)) => form,
        _ => panic!("Drill is not open"),
    }
}

fn import_form(h: &mut Harness) -> &mut ImportForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Import(form)) => form,
        _ => panic!("Import Tools is not open"),
    }
}

fn post_form(h: &mut Harness) -> &mut PostForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Post(form)) => form,
        _ => panic!("Post Process is not open"),
    }
}

/// The 20 mm, four-flute end mill of the engine's facing tests, made with Add Tool: 200 m/min and
/// 0.05 mm a tooth in aluminium unless `material` is empty.
fn add_mill(h: &mut Harness, material: &str) {
    ribbon(h, "cam.tool.create");
    let form = tool_form(h);
    form.diameter = 20.0;
    form.flutes = 4;
    form.flute_length = 30.0;
    form.overall_length = 100.0;
    form.material = material.into();
    press(h, "Create");
    assert_eq!(open(h), "none", "{:?}", h.app.status);
}

#[test]
fn a_part_goes_from_a_tool_to_a_saved_program_through_the_forms() {
    let mut h = harness();
    block(&mut h);
    // Add Tool starts from a 10 mm end mill at the first free number.
    ribbon(&mut h, "cam.tool.create");
    let form = tool_form(&mut h);
    assert_eq!((form.number, form.diameter, form.flutes), (1, 10.0, 3));
    press(&mut h, "Cancel");
    assert_eq!((open(&h), data(&mut h)["tools"].clone()), ("none", json!([])));
    add_mill(&mut h, "aluminium");
    let tool = data(&mut h)["tools"][0].clone();
    assert_eq!(
        (tool["number"].as_u64(), tool["name"].as_str(), tool["kind"].as_str()),
        (Some(1), Some("20 mm Flat end mill"), Some("flat_end_mill"))
    );
    assert_eq!(
        (tool["geometry"]["diameter"].as_f64(), tool["flutes"].as_u64(), tool["holder"]["stick_out"].as_f64()),
        (Some(20.0), Some(4), Some(40.0))
    );
    assert_eq!(
        tool["cutting_data"],
        json!([{"material": "aluminium", "surface_speed": 200.0, "chip_load": 0.05, "plunge_factor": 0.5, "ramp_factor": 0.5}])
    );

    // New Setup takes the material the tool has data for, and draws the stock round the block
    // before the setup exists: six sides, twelve edges and the origin's three arms.
    ribbon(&mut h, "cam.setup.create");
    assert_eq!(setup_form(&mut h).material, "aluminium");
    assert!(!setup_form(&mut h).material_unknown());
    // A material no tool has data for is pointed out while it is typed, and is still allowed.
    setup_form(&mut h).material = "Aluminium ".into();
    h.frames(3);
    assert!(setup_form(&mut h).material_unknown() && h.widget(tr("manufacture.setup_material_unknown")).is_some());
    setup_form(&mut h).material = " aluminium ".into();
    h.frames(3);
    assert!(!setup_form(&mut h).material_unknown() && h.widget(tr("manufacture.setup_material_unknown")).is_none());
    assert_eq!(scene::counts(&h.app), (12, 15, 0));
    assert!(h.widget("62.00 × 42.00 × 21.00 mm").is_some(), "1 mm over the 60 x 40 x 20 block on the sides and on top");
    assert!(h.widget(tr("manufacture.preview_note")).is_some(), "what is drawn is marked as a preview");
    // 5 mm on the sides instead: the size follows, and still nothing is in the design.
    setup_form(&mut h).side = 5.0;
    h.frames(3);
    assert!(h.widget("70.00 × 50.00 × 21.00 mm").is_some());
    assert_eq!(data(&mut h)["setups"], json!([]));
    setup_form(&mut h).side = 1.0;
    press(&mut h, "Create");
    assert_eq!((open(&h), h.app.manufacture_workspace.active_setup.as_deref()), ("none", Some("setup-1")));
    let setup = data(&mut h)["setups"][0].clone();
    assert_eq!((setup["name"].as_str(), setup["material"].as_str(), setup["work_offset"].as_u64()), (Some("Setup 1"), Some("aluminium"), Some(1)));
    assert_eq!(scene::counts(&h.app), (12, 15, 0));

    // Face shows the speed and feed the tool's data gives (200 m/min on 20 mm is 3183.1 rev/min,
    // shown to the whole turn; 0.05 mm on four teeth is a fifth of the speed in mm/min) and draws
    // the toolpath before the operation exists: three rapids, a plunge, four passes and three
    // steps, 0.195 pi minutes.
    ribbon(&mut h, "cam.op.face");
    let rpm = 200_000.0 / (20.0 * std::f64::consts::PI);
    let form = face_form(&mut h);
    assert!(form.rpm == 3183.0 && (form.feed - 0.2 * rpm).abs() < 1e-9, "{} rev/min, {} mm/min", form.rpm, form.feed);
    assert_eq!(form.stepover, 14.0);
    assert_eq!(scene::counts(&h.app), (12, 15, 11));
    assert!(h.widget("0:37").is_some(), "36.8 s of cutting");
    assert!(h.widget(tr("manufacture.preview_note")).is_some(), "what is drawn is marked as a preview");
    assert_eq!(data(&mut h)["setups"][0]["operations"], json!([]));
    press(&mut h, "Create");
    assert_eq!(open(&h), "none");
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    assert!((made["numbers"]["rpm"].as_f64().unwrap() - rpm).abs() < 1e-9 && made["numbers"]["stepover"] == json!(14.0), "{}", made["numbers"]);
    assert_eq!(scene::counts(&h.app), (12, 15, 11), "the operation's own toolpath, where the preview was");
    // Nothing was typed, so only the pattern is kept and the facing goes on following its tool.
    assert_eq!(
        data(&mut h)["setups"][0]["operations"][0]["operation"],
        json!({"kind": "face", "face": {"top": null, "bottom": null, "stepdown": null, "stepover": null, "clear": null, "pattern": "zig_zag", "rpm": null, "feed": null}})
    );

    // Post Process saves the Haas program where the save prompt says and shows what was written.
    ribbon(&mut h, "cam.post");
    let design = h.app.session.doc.name.clone();
    let form = post_form(&mut h);
    assert_eq!((form.params()["post"].as_str(), form.params()["units"].as_str()), (Some("haas_mill"), Some("mm")));
    form.program_number = 1001;
    let name = form.file_name(&design);
    assert!(h.widget(tr("manufacture.post_warning")).is_some(), "the form says the output is unverified");
    press(&mut h, "Save Program…");
    h.frames(5);
    assert_eq!(open(&h), "posted", "{:?}", h.app.status);
    let file = h.dir.join(&name);
    let Some(Dialog::Posted(posted)) = &h.app.manufacture_workspace.dialog else { panic!("no report") };
    assert_eq!(posted.path, file.to_string_lossy());
    let (lines, moves) = (posted.lines, posted.moves);
    let text = std::fs::read_to_string(&file).unwrap();
    // The file is the program the command gives for the same parameters, read back by the command.
    let again = h.app.run("cam.post", json!({"setup": "setup-1", "post": "haas_mill", "units": "mm", "program_number": 1001})).unwrap();
    assert_eq!(Some(text.as_str()), again["text"].as_str());
    assert_eq!((Some(lines), Some(moves)), (again["lines"].as_u64(), again["moves"].as_u64()));
    assert!(
        text.starts_with("%\nO01001 (SETUP 1)\n") && text.contains("\nT1 M6\nS3183 M3\n") && text.contains("\nG1 Z-1. F318.3\nX43. F636.6\n"),
        "{text}"
    );
    assert!(h.widget(&format!("{lines} lines written to {}", file.to_string_lossy())).is_some());
    assert!(h.widget(tr("manufacture.post_warning")).is_some(), "and so does the report");
    press(&mut h, "Close");
    assert_eq!(open(&h), "none");
}

#[test]
fn a_form_says_what_is_missing_and_why_nothing_would_be_made() {
    let mut h = harness();
    // Nothing to face with, nothing to post, nothing to hold: each form says so and makes nothing.
    ribbon(&mut h, "cam.op.face");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    press(&mut h, "Create");
    assert_eq!(open(&h), "face", "Create does nothing until there is something to make");
    ribbon(&mut h, "cam.post");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    ribbon(&mut h, "cam.setup.create");
    assert!(h.widget("setup 'setup-1' has no bodies to machine: show one, or list them in its model_bodies").is_some());
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"].clone()), ("setup", json!([])));
    // With a block the same form places its stock.
    block(&mut h);
    h.frames(3);
    assert!(h.widget("62.00 × 42.00 × 21.00 mm").is_some());
    // Stock that ends inside the part is refused before the setup is made, in the command's words.
    setup_form(&mut h).top = -3.0;
    h.frames(3);
    let why = setup_form(&mut h).placed.trouble().unwrap_or_default().to_string();
    assert!(why.contains("stock"), "{why:?}");
    assert!(h.widget(&why).is_some());
    assert!(setup_form(&mut h).params()["stock"]["offsets"]["z_max"] == json!(-3.0));
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"].clone()), ("setup", json!([])));
    setup_form(&mut h).top = 1.0;
    press(&mut h, "Create");
    assert_eq!(open(&h), "none");

    // A setup but no tool that faces: Face says which tools would.
    ribbon(&mut h, "cam.op.face");
    assert!(h.widget(tr("manufacture.no_tool_for_face")).is_some());
    press(&mut h, "Cancel");
    // A tool with no cutting data: Face has no speed or feed to show, and says the two ways out.
    add_mill(&mut h, "");
    ribbon(&mut h, "cam.op.face");
    let form = face_form(&mut h);
    assert_eq!((form.rpm, form.feed), (0.0, 0.0));
    let why = form.worked.trouble().unwrap_or_default().to_string();
    assert!(why.contains("stock material") && why.contains("rpm and feed") && !why.starts_with("operation"), "{why:?}");
    assert!(h.widget(&why).is_some());
    assert_eq!(scene::counts(&h.app), (12, 15, 0), "no toolpath to draw yet");
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("face", json!([])));
    // A speed alone still needs the tool's chip; with a feed too the facing is worked out and made.
    let form = face_form(&mut h);
    (form.rpm, form.typed_rpm) = (5000.0, true);
    h.frames(3);
    assert!(face_form(&mut h).worked.trouble().is_some());
    let form = face_form(&mut h);
    (form.feed, form.typed_feed) = (1200.0, true);
    h.frames(3);
    assert!(face_form(&mut h).worked.trouble().is_none());
    assert_eq!(scene::counts(&h.app), (12, 15, 11));
    press(&mut h, "Create");
    assert_eq!(open(&h), "none");
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    assert_eq!((made["numbers"]["rpm"].as_f64(), made["numbers"]["feed"].as_f64()), (Some(5000.0), Some(1200.0)));

    // A tool number that is taken: the command's refusal stays in the form until the number is free.
    ribbon(&mut h, "cam.tool.create");
    assert_eq!(tool_form(&mut h).number, 2, "the first free number");
    tool_form(&mut h).number = 1;
    press(&mut h, "Create");
    assert_eq!(open(&h), "tool");
    let why = tool_form(&mut h).error.clone().unwrap_or_default();
    assert!(why.contains("the design already has a tool 1"), "{why:?}");
    assert!(h.widget(&why).is_some());
    tool_form(&mut h).number = 7;
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["tools"].as_array().map(Vec::len)), ("none", Some(2)));
}

#[test]
fn what_is_not_typed_in_face_follows_the_tool() {
    let mut h = harness();
    block(&mut h);
    add_mill(&mut h, "aluminium");
    // A second tool: the form's own 10 mm, three-flute end mill, with the same cutting data.
    ribbon(&mut h, "cam.tool.create");
    tool_form(&mut h).material = "aluminium".into();
    press(&mut h, "Create");
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.face");
    let rpm20 = 200_000.0 / (20.0 * std::f64::consts::PI);
    let form = face_form(&mut h);
    assert_eq!(form.params(), json!({"setup": "setup-1", "tool": 1, "face": {"pattern": "zig_zag"}}));
    assert!((form.stepover, form.typed_stepover) == (14.0, false) && form.rpm == 3183.0);
    // The 10 mm tool: 70% of it is 7, and the same surface speed on half the diameter is twice the
    // spindle speed; three teeth at 0.05 mm are 0.15 mm a turn.
    form.tool = 1;
    h.frames(3);
    let form = face_form(&mut h);
    assert_eq!((form.stepover, form.params()["tool"].as_u64()), (7.0, Some(2)));
    assert!(form.rpm == 6366.0 && (form.feed - 0.3 * rpm20).abs() < 1e-9, "{} rev/min, {} mm/min", form.rpm, form.feed);
    // What is typed is kept as typed, whichever tool is picked after.
    (form.stepover, form.typed_stepover) = (5.0, true);
    (form.multiple_depths, form.stepdown) = (true, 0.5);
    form.pattern = 1;
    (form.rpm, form.typed_rpm) = (9000.0, true);
    form.tool = 0;
    h.frames(3);
    let form = face_form(&mut h);
    assert_eq!(
        form.params(),
        json!({"setup": "setup-1", "tool": 1, "face": {"pattern": "one_way", "stepover": 5.0, "stepdown": 0.5, "rpm": 9000.0}})
    );
    // A speed typed alone keeps the tool's chip. 0.05 mm on four teeth at 9000 rev/min would be
    // 1800 mm/min; with only a quarter of the diameter engaged the chip thins to
    // 2 sqrt(1/4 - 1/16) = sqrt(3)/2 of the feed per tooth, so the feed is raised by 2/sqrt(3).
    assert_eq!((form.stepover, form.rpm), (5.0, 9000.0));
    assert!((form.feed - 3600.0 / 3f64.sqrt()).abs() < 1e-9, "{}", form.feed);
    press(&mut h, "Create");
    assert_eq!(open(&h), "none");
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    // 1 mm of stock in levels of at most 0.5.
    assert_eq!(made["numbers"]["levels"], json!([-0.5, -1.0]));
    assert_eq!((made["numbers"]["rpm"].as_f64(), made["numbers"]["stepover"].as_f64()), (Some(9000.0), Some(5.0)));
    // A second setup picked in the form becomes the one the viewport draws.
    h.app.run("cam.setup.create", json!({"name": "Op 2", "material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.face");
    assert_eq!(face_form(&mut h).setup, 0, "the active setup");
    face_form(&mut h).setup = 1;
    h.frames(3);
    assert_eq!(h.app.manufacture_workspace.active_setup.as_deref(), Some("setup-2"));
    assert_eq!(face_form(&mut h).params()["setup"], "setup-2");
}

#[test]
fn add_tool_writes_the_tool_each_kind_needs() {
    let mut h = harness();
    let form = ToolForm::new(&h.app);
    // An end mill has no corner radius and no tip angle; no material typed, no cutting data.
    let flat = form.params("mm")["tool"].clone();
    assert_eq!((flat["name"].as_str(), flat["kind"].as_str()), (Some("10 mm Flat end mill"), Some("flat_end_mill")));
    assert_eq!((flat["geometry"]["corner_radius"].as_f64(), flat["geometry"]["tip_angle"].as_f64()), (Some(0.0), Some(0.0)));
    assert_eq!((flat["cutting_data"].clone(), flat["length_offset"].as_u64(), flat["diameter_offset"].as_u64()), (json!([]), Some(1), Some(1)));
    // 22 mm of flute and 10 more out of the holder.
    assert_eq!(flat["holder"], json!({"stick_out": 32.0, "segments": []}));
    let of = |kind: &str, change: &dyn Fn(&mut ToolForm)| {
        let mut form = ToolForm::new(&h.app);
        form.kind = KINDS.iter().position(|k| k.0 == kind).unwrap();
        change(&mut form);
        form.params("mm")["tool"].clone()
    };
    // A ball end mill's corner radius is half its diameter, whatever the field holds; a bull-nose
    // end mill's is the one typed; a drill's tip angle is written in radians.
    assert_eq!(of("ball_end_mill", &|f| f.diameter = 6.0)["geometry"]["corner_radius"], json!(3.0));
    assert_eq!(of("bull_nose_end_mill", &|f| f.corner_radius = 0.5)["geometry"]["corner_radius"], json!(0.5));
    let drill = of("drill", &|f| f.tip_angle = 90.0);
    assert!((drill["geometry"]["tip_angle"].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!((drill["name"].as_str(), drill["geometry"]["corner_radius"].as_f64()), (Some("10 mm Drill"), Some(0.0)));
    assert_eq!(of("reamer", &|_| {})["geometry"]["tip_angle"], json!(0.0));
    // A tap has its thread's pitch, and no other kind has one.
    let tap = of("tap", &|f| (f.diameter, f.pitch) = (6.0, 1.25));
    assert_eq!(
        (tap["name"].as_str(), tap["geometry"]["thread_pitch"].as_f64(), tap["geometry"]["tip_angle"].as_f64()),
        (Some("6 mm Tap"), Some(1.25), Some(0.0))
    );
    assert_eq!(drill["geometry"]["thread_pitch"], json!(0.0));
    // A chamfer mill has its angle and the flat at its tip.
    let chamfer = of("chamfer_mill", &|f| (f.tip_angle, f.tip_flat) = (f.usual_tip_angle(), 1.0));
    assert!((chamfer["geometry"]["tip_angle"].as_f64().unwrap() - std::f64::consts::FRAC_PI_2).abs() < 1e-12);
    assert_eq!((chamfer["name"].as_str(), chamfer["geometry"]["tip_diameter"].as_f64()), (Some("10 mm Chamfer mill"), Some(1.0)));
    assert_eq!(drill["geometry"]["tip_diameter"], json!(0.0));
    // A name typed is kept; cutting data comes with a material.
    let named = of("flat_end_mill", &|f| {
        f.name = "  Rougher ".into();
        f.material = " steel ".into();
        f.surface_speed = 120.0;
        f.chip_load = 0.04;
    });
    assert_eq!(named["name"], "Rougher");
    assert_eq!(
        named["cutting_data"],
        json!([{"material": "steel", "surface_speed": 120.0, "chip_load": 0.04, "plunge_factor": 0.5, "ramp_factor": 0.5}])
    );
    // Every kind offered is a tool the command takes as the form first shows it, at the next free number.
    for (i, (kind, _)) in KINDS.iter().enumerate() {
        let mut form = ToolForm::new(&h.app);
        assert_eq!(form.number as usize, i + 1);
        form.kind = i;
        assert!(form.accept(&mut h.app).is_none(), "{kind}: {:?}", h.app.status);
    }
    assert_eq!(data(&mut h)["tools"].as_array().map(Vec::len), Some(KINDS.len()));
    // Picking a kind sets the flutes such a tool usually has, until the number is typed.
    ribbon(&mut h, "cam.tool.create");
    let form = tool_form(&mut h);
    form.kind = 4;
    assert_eq!((form.usual_flutes(), form.pointed()), (2, true));
}

#[test]
fn an_inch_design_is_shown_and_started_in_inches() {
    let mut h = harness();
    assert!(imperial("in") && imperial("ft") && !imperial("mm") && !imperial("cm") && !imperial("m") && !imperial("furlong"));
    assert_eq!((plain_length(10.0, "mm"), plain_length(9.525, "in"), plain_length(12.7, "in")), ("10 mm".into(), "0.375 in".into(), "0.5 in".into()));
    assert_eq!((plain_length(2.5, "cm"), plain_length(0.0, "mm")), ("0.25 cm".to_string(), "0 mm".to_string()));
    // The 60 x 40 x 20 mm block is made first: a design's own commands read plain numbers in its
    // units, and it is in inches from here on.
    block(&mut h);
    h.app.run("document.units", json!({"units": "in"})).unwrap();
    // Half an inch, 650 ft/min and two thou a tooth; 0.04 in of stock; the program in inches.
    let tool = ToolForm::new(&h.app);
    assert_eq!((tool.diameter, tool.flute_length, tool.overall_length), (12.7, 31.75, 76.2));
    assert!((tool.surface_speed / 0.3048 - 650.0).abs() < 1e-9 && (tool.chip_load / 25.4 - 0.002).abs() < 1e-12);
    assert_eq!(tool.params("in")["tool"]["name"], "0.5 in Flat end mill");
    let setup = SetupForm::new(&h.app);
    assert_eq!((setup.side, setup.top, setup.bottom), (1.016, 1.016, 0.0));
    assert_eq!(PostForm::new(&h.app).params()["units"], "inch");
    // The stock's size reads in inches: the block with 0.04 in over on the sides and on top.
    ribbon(&mut h, "cam.setup.create");
    assert_eq!(setup_form(&mut h).stock_size("in", 3).as_deref(), Some("2.442 × 1.655 × 0.827 in"));
}

#[test]
fn post_process_names_the_file_and_comes_back_when_nothing_is_saved() {
    let mut h = harness();
    block(&mut h);
    assert_eq!(PostForm::new(&h.app).file_name("//"), "program.nc", "no setup and no name to go by");
    h.app.run("cam.setup.create", json!({"name": "Op 1: top"})).unwrap();
    ribbon(&mut h, "cam.post");
    let form = post_form(&mut h);
    // Letters, digits and underscores are kept; every run of anything else is one dash.
    assert_eq!(form.file_name("Bracket v2"), "Bracket-v2-Op-1-top.nc");
    assert_eq!(form.file_name("a/b\\c:  d_e"), "a-b-c-d_e-Op-1-top.nc");
    assert_eq!(form.file_name(""), "Op-1-top.nc");
    // A typed program name is sent; left empty, the command uses the setup's.
    assert!(form.params().get("program_name").is_none());
    form.program_name = " TOP SIDE ".into();
    form.program_number = 7;
    assert_eq!(form.params(), json!({"setup": "setup-1", "post": "haas_mill", "units": "mm", "program_number": 7, "program_name": "TOP SIDE"}));
    // No file chosen in the save prompt: the form is back as it was, and nothing was posted.
    let slot = crate::picker_test_support::Slot::default();
    crate::picker_test_support::save(&mut h.app, &slot);
    press(&mut h, "Save Program…");
    assert_eq!(open(&h), "none", "the save prompt is up");
    *slot.borrow_mut() = Some(None);
    h.frames(5);
    assert_eq!((open(&h), post_form(&mut h).program_number, post_form(&mut h).error.clone()), ("post", 7, None));
    // A file chosen, but the setup has nothing to post: the form is back with the command's reason
    // and no file is written.
    let file = h.dir.join("nothing.nc");
    press(&mut h, "Save Program…");
    *slot.borrow_mut() = Some(Some(file.to_string_lossy().into_owned()));
    h.frames(5);
    assert_eq!(open(&h), "post");
    let why = post_form(&mut h).error.clone().unwrap_or_default();
    assert!(why.contains("setup 'setup-1' has no operations to post"), "{why:?}");
    assert!(h.widget(&why).is_some() && !file.exists());
    assert_eq!(
        (minutes_seconds(0.195 * std::f64::consts::PI), minutes_seconds(2.0), minutes_seconds(61.51)),
        ("0:37".into(), "2:00".into(), "61:31".into())
    );
    assert_eq!((minutes_seconds(-1.0), minutes_seconds(f64::NAN)), ("0:00".to_string(), "0:00".to_string()));
}

#[test]
fn the_ribbon_has_each_form_and_a_form_belongs_to_manufacture() {
    let mut h = harness();
    let placed = [
        ("cam.setup.create", "SETUP"),
        ("cam.op.face", "MILLING"),
        ("cam.op.contour", "MILLING"),
        ("cam.op.pocket", "MILLING"),
        ("cam.op.chamfer", "MILLING"),
        ("cam.op.drill", "DRILLING"),
        ("cam.simulate", "ACTIONS"),
        ("cam.post", "ACTIONS"),
        ("cam.setup.sheet", "ACTIONS"),
        ("cam.tool.create", "MANAGE"),
    ];
    for (id, panel) in placed {
        assert_eq!(crate::workspace::destination(solidcraft_engine::find_command(id).unwrap()), Some(("MANUFACTURE", panel)), "{id}");
        assert!(crate::scenario::handle_at(&format!("toolbar:{id}")).is_some(), "{id} is on the ribbon");
    }
    // Bore is a milling operation too, the panel's fifth: a panel has four buttons, so it is in
    // the panel's menu.
    let bore = solidcraft_engine::find_command("cam.op.bore").unwrap();
    assert_eq!(crate::workspace::destination(bore), Some(("MANUFACTURE", "MILLING")));
    assert!(crate::scenario::handle_at("toolbar:cam.op.bore").is_none(), "no button of its own");
    panel_menu(&mut h, "MILLING", "cam.op.bore");
    assert_eq!(open(&h), "bore");
    h.app.manufacture_workspace.dialog = None;
    // Other commands have no form here, and run as before.
    assert!(!start(&mut h.app, "cam.op.manual") && !start(&mut h.app, "solid.box"));
    assert_eq!(open(&h), "none");
    // Another command's own dialog takes the form's place: the two are never open together.
    ribbon(&mut h, "cam.tool.create");
    h.app.start("component.create");
    h.frames(2);
    assert!(h.app.dialog.is_some(), "New Component has a dialog of its own");
    assert_eq!(open(&h), "none");
    crate::dialogs::cancel(&mut h.app);
    crate::workspace::enter(&mut h.app, "manufacture").unwrap();
    h.frames(2);
    // Leaving Manufacture closes the form.
    ribbon(&mut h, "cam.tool.create");
    assert_eq!(open(&h), "tool");
    crate::workspace::enter(&mut h.app, "design").unwrap();
    h.frames(2);
    assert_eq!(open(&h), "none");
    // Started from Design (the command search), a form opens in Manufacture.
    h.app.start("cam.setup.create");
    h.frames(2);
    assert!(crate::manufacture_workspace::active(&h.app));
    assert_eq!(open(&h), "setup");
    // Not while a sketch is being edited: Manufacture can't be entered then, and the reason is said.
    press(&mut h, "Cancel");
    crate::workspace::enter(&mut h.app, "design").unwrap();
    h.app.run("sketch.create", json!({"plane": "XY"})).unwrap();
    assert!(start(&mut h.app, "cam.setup.create"), "the command is ours even when its form can't open");
    assert!(!crate::manufacture_workspace::active(&h.app));
    assert_eq!(open(&h), "none");
    assert_eq!(h.app.status.as_ref().map(|s| (s.0.as_str(), s.2)), Some((tr("manufacture.error_finish_operation"), true)));
}

/// The 60 x 40 x 20 block with the holes of the engine's hole tests: four of 6 through it, the
/// first under a counterbore of 12 that is 4 deep, and two of 8 that are 10 deep with flat floors.
/// Under the vise setup the block's top is at Z -1 and the origin is over its middle.
fn holed(h: &mut Harness) {
    block(h);
    let mut cut = |diameter: f64, base: [f64; 3], height: f64| {
        h.app.run("solid.cylinder", json!({"diameter": diameter, "height": height, "base": base, "operation": "cut"})).unwrap();
    };
    for (x, y) in [(10.0, 10.0), (50.0, 10.0), (50.0, 30.0), (10.0, 30.0)] {
        cut(6.0, [x, y, -5.0], 30.0);
    }
    for y in [10.0, 30.0] {
        cut(8.0, [30.0, y, 10.0], 15.0);
    }
    cut(12.0, [10.0, 10.0, 16.0], 10.0);
}

/// A tool of `kind` and `diameter` with 40 mm of flute and Add Tool's own cutting data for
/// aluminium (200 m/min, 0.05 mm a tooth), at the next free number.
fn add_tool(h: &mut Harness, kind: &str, diameter: f64) {
    let mut form = ToolForm::new(&h.app);
    form.kind = KINDS.iter().position(|k| k.0 == kind).unwrap();
    form.diameter = diameter;
    (form.flutes, form.tip_angle) = (form.usual_flutes(), form.usual_tip_angle());
    (form.flute_length, form.overall_length) = (40.0, 90.0);
    form.material = "aluminium".into();
    assert!(form.accept(&mut h.app).is_none(), "{:?}", h.app.status);
}

#[test]
fn drill_offers_the_holes_found_on_the_model_and_suits_the_tool_and_depth_to_them() {
    let mut h = harness();
    holed(&mut h);
    // T1 a 6 mm drill, T2 a 10 mm drill, T3 an 8 mm end mill.
    add_tool(&mut h, "drill", 6.0);
    add_tool(&mut h, "drill", 10.0);
    add_tool(&mut h, "flat_end_mill", 8.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.drill");
    // The largest group first: the three holes of 6 that go through 20 mm of part. The widest
    // tool that is no wider is the 6 mm drill; the depth is the part's 20 and 1 more, to the
    // drill's full diameter.
    let form = drill_form(&mut h);
    assert_eq!((form.groups().len(), form.group, form.tool, form.depth), (4, 0, 0, 21.0));
    let three = json!([[20.0, -10.0], [-20.0, 10.0], [20.0, 10.0]]);
    let plain = json!({"type": "drill", "dwell": 0.0});
    assert_eq!(
        form.params(),
        json!({"setup": "setup-1", "tool": 1, "drill": {"holes": three, "top": -1.0, "depth": 21.0, "depth_to": "full_diameter", "cycle": plain}})
    );
    // 200 m/min on 6 mm asks 10610 rev/min: capped at the generic 10 000, and the form says so.
    // 0.05 mm on two lips is 1000 mm/min there.
    assert!(form.rpm == 10_000.0 && (form.feed - 1000.0).abs() < 1e-9, "{} rev/min, {} mm/min: {:?}", form.rpm, form.feed, form.worked.trouble());
    let capped = form.worked.get().map(|(_, preview)| preview.notes.join(" ")).unwrap_or_default();
    assert!(capped.contains("capped at 10000 rpm"), "{capped:?}");
    assert!(h.widget(&capped).is_some());
    assert!(h.widget("3 × Ø6.00 × 20.00 mm, through").is_some());
    assert!(h.widget(&solidcraft_ui_text::format("manufacture.drill_depth_through", &[("past", "1.00 mm")])).is_some());
    let drawn = scene::counts(&h.app);
    assert!(drawn.2 > 0 && (drawn.0, drawn.1) == (12, 15), "the drilling is drawn before it exists: {drawn:?}");

    // The two flat-floored holes of 8: the end mill of 8 suits them, and the depth is their 10 to
    // the tool's tip, so nothing cuts below the floor.
    drill_form(&mut h).group = 1;
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!((form.tool, form.depth, form.params()["tool"].as_u64(), form.params()["drill"]["depth_to"].as_str()), (2, 10.0, Some(3), Some("tip")));
    assert_eq!(form.params()["drill"]["holes"], json!([[0.0, -10.0], [0.0, 10.0]]));
    assert!(h.widget("2 × Ø8.00 × 10.00 mm, flat floor").is_some() && h.widget(tr("manufacture.drill_depth_flat")).is_some());
    // The counterbore of 12 is 4 deep with a floor (the step to the hole of 6): the 10 mm drill is
    // the widest that fits, to its tip.
    drill_form(&mut h).group = 3;
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!(
        (form.tool, form.depth, form.params()["drill"]["top"].as_f64(), form.params()["drill"]["depth_to"].as_str()),
        (1, 4.0, Some(-1.0), Some("tip"))
    );
    // A tool picked by hand stays picked, and one wider than the holes is pointed out.
    (form.tool, form.picked_tool) = (1, true);
    form.group = 0;
    h.frames(3);
    assert_eq!(drill_form(&mut h).tool, 1);
    assert!(h.widget(tr("manufacture.drill_tool_wider")).is_some());
    // A depth typed is kept for these holes and is not carried to others.
    let form = drill_form(&mut h);
    (form.tool, form.picked_tool) = (0, false);
    (form.depth, form.typed_depth) = (12.5, true);
    h.frames(3);
    assert_eq!(drill_form(&mut h).params()["drill"]["depth"], json!(12.5));
    assert!(h.widget(tr("manufacture.drill_tool_wider")).is_none());
    drill_form(&mut h).group = 2;
    h.frames(3);
    // The hole under the counterbore: from the counterbore's floor (Z -5), 16 of part and 1 more.
    let form = drill_form(&mut h);
    assert_eq!((form.depth, form.typed_depth, form.params()["drill"]["top"].as_f64()), (17.0, false, Some(-5.0)));
    // Pecking takes the tool's diameter a peck until another is typed.
    form.group = 0;
    form.cycle = 1;
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!(form.params()["drill"]["cycle"], json!({"type": "peck", "peck": 6.0}));
    (form.peck, form.typed_peck) = (2.0, true);
    form.cycle = 2;
    h.frames(3);
    assert_eq!(drill_form(&mut h).params()["drill"]["cycle"], json!({"type": "chip_break", "peck": 2.0}));
    drill_form(&mut h).cycle = 0;
    assert_eq!(data(&mut h)["setups"][0]["operations"], json!([]));
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    assert_eq!(scene::counts(&h.app), drawn, "the operation's own toolpath, where the preview was");

    // The program: one canned cycle over the three holes, nearest next. A 118 degree point on
    // 6 mm is 3 / tan 59 = 1.803 long, so the tip goes to -1 - 21 - 1.803; R is the setup's 5 mm
    // above the stock.
    let posted = h.app.run("cam.post", json!({"setup": "setup-1", "post": "haas_mill"})).unwrap();
    let text = posted["text"].as_str().unwrap();
    assert!(text.contains("\nT1 M6\nS10000 M3\n"), "{text}");
    assert!(text.contains("G99 G81 X20. Y-10. Z-23.803 R5. F1000.\nX20. Y10.\nX-20. Y10.\nG80\n"), "{text}");
}

#[test]
fn drill_says_what_is_missing_and_when_there_is_nothing_to_drill() {
    let mut h = harness();
    block(&mut h);
    ribbon(&mut h, "cam.op.drill");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    h.app.run("cam.setup.create", json!({})).unwrap();
    ribbon(&mut h, "cam.op.drill");
    assert!(h.widget(tr("manufacture.no_tool_for_drill")).is_some());
    // A drill, but a block with no holes: nothing to pick, and Create does nothing.
    add_tool(&mut h, "drill", 6.0);
    ribbon(&mut h, "cam.op.drill");
    assert!(drill_form(&mut h).groups().is_empty() && h.widget(tr("manufacture.drill_no_holes")).is_some());
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("drill", json!([])));
    // A hole cut while the form is open is found: the design changed under it.
    h.app.run("solid.cylinder", json!({"diameter": 6, "height": 30, "base": [30, 20, -5], "operation": "cut"})).unwrap();
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!((form.groups().len(), form.params()["drill"]["holes"].clone()), (1, json!([[0.0, 0.0]])));
    // A setup that can't be placed says why, in the engine's words.
    h.app.run("cam.setup.edit", json!({"id": "setup-1", "model_bodies": ["Gone"]})).unwrap();
    h.frames(3);
    let why = drill_form(&mut h).found.trouble().unwrap_or_default().to_string();
    assert!(why.contains("'Gone' is not in the design"), "{why:?}");
    assert!(h.widget(&why).is_some());
    // A hole deeper than the drill's flutes is refused before it is made: Add Tool's own drill
    // has 22 mm of flute, and 28 mm to the full diameter needs the point's 1.8 mm more.
    h.app.run("cam.setup.edit", json!({"id": "setup-1", "model_bodies": []})).unwrap();
    add_tool(&mut h, "drill", 5.0);
    let short = ToolForm::new(&h.app);
    let number = short.number;
    let mut short = short;
    short.kind = 4;
    short.diameter = 6.0;
    assert!(short.accept(&mut h.app).is_none());
    ribbon(&mut h, "cam.op.drill");
    let form = drill_form(&mut h);
    form.tool = form.tools.iter().position(|tool| tool.number == number).unwrap();
    form.picked_tool = true;
    (form.depth, form.typed_depth) = (28.0, true);
    h.frames(3);
    let why = drill_form(&mut h).worked.trouble().unwrap_or_default().to_string();
    assert!(why.contains("flute"), "{why:?}");
    assert!(h.widget(&why).is_some());
    // The depth each kind of bottom asks for.
    let group = |bottom| HoleGroup { diameter: 6.0, top: -1.0, depth: 20.0, bottom, holes: vec![[0.0, 0.0]] };
    assert_eq!(drill_depth(&group(HoleBottom::Through)), (21.0, "full_diameter"));
    assert_eq!(drill_depth(&group(HoleBottom::Point)), (20.0, "full_diameter"));
    assert_eq!(drill_depth(&group(HoleBottom::Flat)), (20.0, "tip"));
}

/// A right click on a Browser row, and a pick from the menu it opens.
fn row_menu(h: &mut Harness, row: &str, entry: &str) {
    h.step(&json!({"click": {"browser": row}, "button": "right"})).unwrap_or_else(|e| panic!("{row}: {e}"));
    h.frames(3);
    h.step(&json!({"click": {"handle": format!("browser:menu:{entry}")}})).unwrap_or_else(|e| panic!("{row} > {entry}: {e}"));
    h.frames(3);
}

/// The block with a 20 mm end mill (T1), the vise setup in aluminium and a facing.
fn faced(h: &mut Harness) {
    block(h);
    add_tool(h, "flat_end_mill", 20.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    h.app.run("cam.op.face", json!({"setup": "setup-1", "tool": 1})).unwrap();
    h.frames(3);
}

#[test]
fn the_browser_edits_and_deletes_what_its_rows_stand_for() {
    let mut h = harness();
    faced(&mut h);
    assert_eq!(scene::counts(&h.app), (12, 15, 11));
    // Edit on the facing's row opens its form on what the facing keeps: nothing typed.
    row_menu(&mut h, "Face 1 (T1)", "manufacture.edit");
    assert_eq!(open(&h), "face");
    let form = face_form(&mut h);
    assert_eq!((form.tool, form.stepover, form.typed_stepover, form.typed_rpm), (0, 14.0, false, false));
    assert!(h.widget("Face 1").is_some() && h.widget("OK").is_some() && h.widget("Create").is_none(), "the window is the facing's own");
    assert!(h.widget(tr("manufacture.preview_note_edit")).is_some() && h.widget(tr("manufacture.preview_note")).is_none());
    // The facing is drawn as the form would leave it, once: its own toolpath gives way.
    assert_eq!(scene::counts(&h.app), (12, 15, 11));
    let form = face_form(&mut h);
    (form.stepover, form.typed_stepover) = (7.0, true);
    h.frames(3);
    // Eight passes and seven steps at half the stepover, with the same three rapids and plunge.
    assert_eq!(scene::counts(&h.app), (12, 15, 19));
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][0].clone();
    assert_eq!((kept["id"].as_str(), kept["name"].as_str(), kept["tool"].as_u64()), (Some("op-1"), Some("Face 1"), Some(1)));
    assert_eq!(kept["operation"]["face"]["stepover"], json!(7.0));
    assert_eq!(scene::counts(&h.app), (12, 15, 19));

    // A double click on the setup's row opens its form on the setup as it is; 3 mm on top instead
    // of 1 raises the origin with the stock's top, and the setup keeps its id and its operation.
    h.step(&json!({"click": {"browser": "Setup 1 (G54)"}, "double": true})).unwrap();
    h.frames(3);
    assert_eq!(open(&h), "setup");
    let form = setup_form(&mut h);
    assert_eq!(
        (form.name.as_str(), form.material.as_str(), form.side, form.top, form.bottom, form.origin, form.work_offset),
        ("Setup 1", "aluminium", 1.0, 1.0, 0.0, 0, 0)
    );
    form.top = 3.0;
    form.work_offset = 1;
    h.frames(3);
    assert!(h.widget("62.00 × 42.00 × 23.00 mm").is_some());
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let setup = data(&mut h)["setups"][0].clone();
    assert_eq!(
        (setup["id"].as_str(), setup["work_offset"].as_u64(), setup["stock"]["offsets"]["z_max"].as_f64()),
        (Some("setup-1"), Some(2), Some(3.0))
    );
    assert_eq!((setup["material"].as_str(), setup["operations"].as_array().map(Vec::len)), (Some("aluminium"), Some(1)));

    // The tool: a tool an operation uses can't be deleted (the command says so, on the status
    // line), but it can be changed.
    h.step(&json!({"click": {"chevron": "Tools"}})).unwrap();
    h.frames(3);
    row_menu(&mut h, "T1 20 mm Flat end mill", "cam.tool.delete");
    assert_eq!(data(&mut h)["tools"].as_array().map(Vec::len), Some(1));
    assert!(h.app.status.as_ref().is_some_and(|s| s.2 && s.0.contains("uses tool 1")), "{:?}", h.app.status);
    row_menu(&mut h, "T1 20 mm Flat end mill", "manufacture.edit");
    assert_eq!(open(&h), "tool");
    let form = tool_form(&mut h);
    assert_eq!(
        (form.number, form.name.as_str(), form.diameter, form.flutes, form.material.as_str()),
        (1, "20 mm Flat end mill", 20.0, 3, "aluminium")
    );
    form.flutes = 4;
    form.name = "Rougher".into();
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let tool = data(&mut h)["tools"][0].clone();
    assert_eq!(
        (tool["name"].as_str(), tool["flutes"].as_u64(), tool["holder"]["stick_out"].as_f64(), tool["length_offset"].as_u64()),
        (Some("Rougher"), Some(4), Some(50.0), Some(1))
    );
    // Four teeth instead of three: the facing, which follows its tool, feeds a third faster. At
    // 0.05 mm a tooth that is a fifth of the speed in mm/min, raised because 7 mm of a 20 mm tool
    // thins the chip to 2 sqrt(0.35 x 0.65) of the feed per tooth.
    let rpm = 200_000.0 / (20.0 * std::f64::consts::PI);
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    let feed = 0.2 * rpm / (2.0 * (0.35f64 * 0.65).sqrt());
    assert!((made["numbers"]["feed"].as_f64().unwrap() - feed).abs() < 1e-9, "{}", made["numbers"]);

    // Delete on the rows: the facing, then its tool, then the setup. Each is one undo step.
    row_menu(&mut h, "Face 1 (T1)", "cam.op.delete");
    assert_eq!(data(&mut h)["setups"][0]["operations"], json!([]));
    row_menu(&mut h, "T1 Rougher", "cam.tool.delete");
    assert_eq!(data(&mut h)["tools"], json!([]));
    row_menu(&mut h, "Setup 1 (G55)", "cam.setup.delete");
    assert_eq!(data(&mut h)["setups"], json!([]));
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(data(&mut h)["setups"].as_array().map(Vec::len), Some(1));
}

#[test]
fn a_form_changes_only_what_it_shows() {
    let mut h = harness();
    holed(&mut h);
    add_tool(&mut h, "drill", 6.0);
    add_tool(&mut h, "flat_end_mill", 20.0);
    // A setup and operations made by a script, with parts the forms do not show: safe heights, a
    // facing's own heights, a drill's order, dwell and R plane, a tool's holder and second material.
    h.app.run("cam.setup.create", json!({"name": "Top", "material": "aluminium", "safe_heights": {"clearance": 40.0, "retract": 3.0}})).unwrap();
    h.app.run("cam.op.face", json!({"setup": "setup-1", "tool": 2, "face": {"bottom": -1.5, "clear": 4.0, "stepover": 12.0}})).unwrap();
    let drill = json!({
        "holes": [[20.0, -10.0], [20.0, 10.0]], "order": "as_given", "top": -1.0, "depth": 9.0, "depth_to": "full_diameter",
        "r_above": 2.0, "between": "clearance", "cycle": {"type": "drill", "dwell": 0.5}, "rpm": 2000.0
    });
    h.app.run("cam.op.drill", json!({"setup": "setup-1", "tool": 1, "drill": drill})).unwrap();
    let mut tool = data(&mut h)["tools"][0].clone();
    tool["holder"] = json!({"stick_out": 45.0, "segments": [{"length": 30.0, "lower_diameter": 20.0, "upper_diameter": 32.0}]});
    tool["cutting_data"]
        .as_array_mut()
        .unwrap()
        .push(json!({"material": "steel", "surface_speed": 80.0, "chip_load": 0.03, "plunge_factor": 0.4, "ramp_factor": 0.4}));
    tool["length_offset"] = json!(21);
    h.app.run("cam.tool.edit", json!({"number": 1, "tool": tool})).unwrap();
    let kept = data(&mut h);
    // Opened and left alone, each form gives back exactly what the design keeps.
    assert_eq!(ToolForm::edit(&h.app, 1).unwrap().params("mm"), json!({"number": 1, "tool": kept["tools"][0]}));
    let face = FaceForm::edit(&h.app, "op-1").unwrap();
    assert_eq!(
        face.params(),
        json!({"setup": "setup-1", "tool": 2, "face": {"top": null, "bottom": -1.5, "clear": 4.0, "stepover": 12.0, "pattern": "zig_zag"}})
    );
    let drilled = DrillForm::edit(&h.app, "op-2").unwrap();
    assert_eq!(drilled.params()["drill"], {
        let mut same = kept["setups"][0]["operations"][1]["operation"]["drill"].clone();
        same.as_object_mut().unwrap().remove("feed");
        same
    });
    let setup = SetupForm::edit(&h.app, "setup-1").unwrap().params();
    assert_eq!((setup["id"].as_str(), setup["name"].as_str(), setup["material"].as_str()), (Some("setup-1"), Some("Top"), Some("aluminium")));
    assert_eq!((setup["stock"].clone(), setup["wcs"].clone()), (kept["setups"][0]["stock"].clone(), kept["setups"][0]["wcs"].clone()));
    assert!(setup.get("safe_heights").is_none() && setup.get("model_bodies").is_none());

    // The drill operation's form keeps its two holes and changes its depth and cycle.
    h.frames(3);
    row_menu(&mut h, "Drill 2 (T1)", "manufacture.edit");
    assert_eq!(open(&h), "drill");
    assert!(h.widget("2, as made").is_some() && h.widget(tr("manufacture.drill_depth_point")).is_some());
    let form = drill_form(&mut h);
    assert_eq!((form.depth, form.typed_depth, form.rpm, form.typed_rpm, form.typed_feed), (9.0, true, 2000.0, true, false));
    form.depth = 11.0;
    form.cycle = 1;
    (form.peck, form.typed_peck) = (3.0, true);
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let after = data(&mut h)["setups"][0]["operations"][1]["operation"]["drill"].clone();
    assert_eq!((after["depth"].as_f64(), after["cycle"].clone()), (Some(11.0), json!({"type": "peck", "peck": 3.0})));
    for key in ["holes", "order", "top", "depth_to", "r_above", "between", "rpm"] {
        assert_eq!(after[key], drill[key], "{key} is as the script made it");
    }
    // The setup's safe heights are still the script's after its form changed its name.
    h.step(&json!({"click": {"browser": "Top (G54)"}, "double": true})).unwrap();
    h.frames(3);
    setup_form(&mut h).name = "Top side".into();
    press(&mut h, "OK");
    let setup = data(&mut h)["setups"][0].clone();
    assert_eq!((setup["name"].as_str(), setup["safe_heights"].clone()), (Some("Top side"), json!({"clearance": 40.0, "retract": 3.0})));

    // What a form can't show, it does not open: a fixed stock box, a hand-made toolpath. The
    // status line says why.
    h.app
        .run(
            "cam.setup.create",
            json!({"name": "Bar", "stock": {"kind": "fixed_box", "size": [80, 50, 25], "anchor": {"x": "center", "y": "center", "z": "min"}}}),
        )
        .unwrap();
    assert_eq!(SetupForm::edit(&h.app, "setup-2").err().as_deref(), Some(tr("manufacture.edit_unsupported_setup")));
    edit(&mut h.app, &Target::Setup("setup-2".into()));
    assert_eq!((open(&h), h.app.status.as_ref().map(|s| (s.0.as_str(), s.2))), ("none", Some((tr("manufacture.edit_unsupported_setup"), true))));
    let path = json!({"start": [0.0, 0.0, 50.0], "moves": [{"kind": "rapid", "to": [0.0, 0.0, 5.0]}]});
    h.app.run("cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": path})).unwrap();
    edit(&mut h.app, &Target::Operation("op-3".into()));
    assert_eq!((open(&h), h.app.status.as_ref().map(|s| s.0.as_str())), ("none", Some(tr("manufacture.edit_unsupported_operation"))));
    edit(&mut h.app, &Target::Operation("op-9".into()));
    assert!(h.app.status.as_ref().is_some_and(|s| s.0.contains("no operation 'op-9'")));
    edit(&mut h.app, &Target::Tool(9));
    assert!(h.app.status.as_ref().is_some_and(|s| s.0.contains("no tool 9")));
}

#[test]
fn tools_go_out_to_a_library_file_and_back_in() {
    let mut h = harness();
    for id in ["cam.library.import", "cam.library.export"] {
        assert_eq!(crate::workspace::destination(solidcraft_engine::find_command(id).unwrap()), Some(("MANUFACTURE", "MANAGE")), "{id}");
    }
    let status = |h: &Harness| h.app.status.as_ref().map(|s| (s.0.clone(), s.2));
    // Nothing to export yet: said, and no file is asked for.
    h.app.start("cam.library.export");
    h.frames(3);
    assert_eq!(status(&h), Some((tr("manufacture.library_no_tools").to_string(), true)));
    // Two tools go out to the file the save prompt names.
    add_tool(&mut h, "flat_end_mill", 10.0);
    add_tool(&mut h, "drill", 6.0);
    h.app.start("cam.library.export");
    h.frames(5);
    let file = h.dir.join(file_name(&format!("{} tools", h.app.session.doc.name), "tools", "json"));
    let path = file.to_string_lossy().to_string();
    assert!(file.exists(), "{path}: {:?}", h.app.status);
    assert_eq!(status(&h), Some((solidcraft_ui_text::format("manufacture.library_exported", &[("tools", "2"), ("path", &path)]), false)));

    // Into a design without them: both are new and ticked, and Import copies them.
    for number in [1, 2] {
        h.app.run("cam.tool.delete", json!({"number": number})).unwrap();
    }
    h.app.start("cam.library.import");
    h.frames(3);
    assert_eq!(open(&h), "import");
    assert!(h.widget(tr("manufacture.library_choose")).is_some());
    import_form(&mut h).path = path.clone();
    h.frames(3);
    let form = import_form(&mut h);
    assert_eq!((form.library().map(|l| l.tools.len()), form.picked.iter().copied().collect::<Vec<_>>()), (Some(2), vec![1, 2]));
    assert_eq!(form.params(), json!({"path": path, "numbers": [1, 2], "replace": false}));
    assert!(h.widget("T1 10 mm Flat end mill, Ø10.00 mm: new").is_some() && h.widget("T2 6 mm Drill, Ø6.00 mm: new").is_some());
    press(&mut h, "Import");
    assert_eq!((open(&h), data(&mut h)["tools"].as_array().map(Vec::len)), ("none", Some(2)));
    assert_eq!(status(&h), Some((solidcraft_ui_text::format("manufacture.library_imported", &[("added", "2"), ("updated", "0")]), false)));

    // Again: both are in the design as they are in the file, so there is nothing to import.
    h.app.start("cam.library.import");
    import_form(&mut h).path = path.clone();
    h.frames(3);
    assert!(import_form(&mut h).picked.is_empty());
    assert!(h.widget("T1 10 mm Flat end mill, Ø10.00 mm: already in the design").is_some());
    press(&mut h, "Import");
    assert_eq!(open(&h), "import", "Import does nothing with nothing ticked");
    press(&mut h, "Cancel");

    // The design's drill changed since: it differs, and is only updated when ticked by hand.
    let mut drill = data(&mut h)["tools"][1].clone();
    drill["flutes"] = json!(3);
    h.app.run("cam.tool.edit", json!({"number": 2, "tool": drill})).unwrap();
    h.app.start("cam.library.import");
    import_form(&mut h).path = path.clone();
    h.frames(3);
    let form = import_form(&mut h);
    assert_eq!(form.params(), json!({"path": path, "numbers": [], "replace": false}));
    assert!(h.widget(&format!("T2 6 mm Drill, Ø6.00 mm: {}", tr("manufacture.library_differs"))).is_some());
    import_form(&mut h).picked.insert(2);
    assert_eq!(import_form(&mut h).params(), json!({"path": path, "numbers": [2], "replace": true}));
    press(&mut h, "Import");
    assert_eq!((open(&h), data(&mut h)["tools"][1]["flutes"].as_u64()), ("none", Some(2)));
    assert_eq!(status(&h), Some((solidcraft_ui_text::format("manufacture.library_imported", &[("added", "0"), ("updated", "1")]), false)));

    // A file that is no tool library: the reason, in the form. Browse takes the file the system's
    // prompt gives.
    h.app.start("cam.library.import");
    import_form(&mut h).path = "notes.txt".into();
    h.frames(3);
    let why = import_form(&mut h).listed.trouble().unwrap_or_default().to_string();
    assert!(why.contains("a tool library is a .json file"), "{why:?}");
    assert!(h.widget(&why).is_some());
    let slot = crate::picker_test_support::Slot::default();
    crate::picker_test_support::open(&mut h.app, &slot);
    press(&mut h, tr("manufacture.library_browse"));
    *slot.borrow_mut() = Some(Some(path.clone()));
    h.frames(5);
    assert_eq!((open(&h), import_form(&mut h).path.clone()), ("import", path));
    assert_eq!(
        (file_name("Bracket v2 tools", "tools", "json"), file_name(" / ", "tools", "json")),
        ("Bracket-v2-tools.json".to_string(), "tools.json".to_string())
    );
}

#[test]
fn contour_cuts_round_the_part_and_keeps_only_what_is_typed() {
    use std::f64::consts::PI;
    let mut h = harness();
    // Nothing to cut round or with: the form says what to make first.
    ribbon(&mut h, "cam.op.contour");
    assert_eq!(open(&h), "contour");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    press(&mut h, "Cancel");
    block(&mut h);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.contour");
    assert!(h.widget(tr("manufacture.no_tool_for_contour")).is_some());
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("contour", json!([])));
    press(&mut h, "Cancel");
    add_tool(&mut h, "drill", 6.0);
    ribbon(&mut h, "cam.op.contour");
    assert!(h.widget(tr("manufacture.no_tool_for_contour")).is_some(), "a drill cuts no wall");
    press(&mut h, "Cancel");

    // With the 20 mm end mill (T2) the form shows what the engine works out, nothing typed: down
    // to the block's bottom (Z -21 under 1 mm of stock), in steps of at most half the tool, at
    // the speed and feed of the tool's data for a cut as wide as the tool.
    add_mill(&mut h, "aluminium");
    ribbon(&mut h, "cam.op.contour");
    let rpm = 200_000.0 / (20.0 * PI);
    let form = contour_form(&mut h);
    assert_eq!((form.bottom, form.typed_bottom, form.stepdown, form.typed_stepdown, form.leave, form.direction), (-21.0, false, 10.0, false, 0.0, 0));
    // On to the wall by an arc of a tenth of the tool, and no tabs.
    assert_eq!((form.lead, form.typed_lead, form.tabs), (2.0, false, 0));
    assert!(form.rpm == 3183.0 && (form.feed - 0.2 * rpm).abs() < 1e-9, "{} rev/min, {} mm/min", form.rpm, form.feed);
    assert!(h.widget("3 × 7.00 mm").is_some(), "21 mm in three levels of 7");
    // 57 mm of plunging at half the feed and three rounds of 201 + 22 pi at the feed: 87.1 s.
    assert!(h.widget("1:27").is_some());
    assert!(h.widget(tr("manufacture.contour_loose")).is_some() && h.widget(tr("manufacture.preview_note")).is_some());
    assert!(h.widget(tr("manufacture.contour_tabs_note")).is_none() && h.widget(tr("manufacture.contour_tab_width")).is_none());
    // Drawn before it exists: a rapid down to the retract height and one back up at the end,
    // and at each of the three levels a plunge, the way in, the arc on to the wall in six
    // pieces, five straight runs (the side it starts on is in two), four corners of thirteen
    // pieces each, the arc off the wall in six and a rapid up; and between levels a hop back to
    // where it goes down.
    let drawn = scene::counts(&h.app);
    assert_eq!(drawn, (12, 15, 2 + 3 * (1 + 1 + 6 + 5 + 4 * 13 + 6 + 1) + 2));
    assert_eq!(data(&mut h)["setups"][0]["operations"], json!([]));
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    // Nothing was typed, so only the direction is kept: the contour goes on following the model
    // and its tool.
    assert_eq!(
        data(&mut h)["setups"][0]["operations"][0]["operation"],
        json!({"kind": "contour", "contour": {"top": null, "bottom": null, "stepdown": null, "leave": null, "direction": "climb", "clear": null, "rpm": null, "feed": null, "lead": null, "tabs": null, "side": "outside", "openings": [], "ramp_angle": null}})
    );
    assert_eq!(scene::counts(&h.app), drawn, "the operation's own toolpath, where the preview was");
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    assert_eq!(made["numbers"]["levels"], json!([-7.0, -14.0, -21.0]));
    h.frames(3);
    h.step(&json!({"click": {"browser": "Contour 1 (T2)"}})).unwrap_or_else(|e| panic!("its row in the Browser: {e}"));

    // What is typed is kept: 10 mm short of the bottom, 4 mm steps, half a millimetre left on the
    // wall, the other way round.
    ribbon(&mut h, "cam.op.contour");
    let form = contour_form(&mut h);
    (form.bottom, form.typed_bottom) = (-11.0, true);
    (form.stepdown, form.typed_stepdown) = (4.0, true);
    form.leave = 0.5;
    form.direction = 1;
    // Two tabs, and a bigger arc on to the wall: the tabs' width and height appear, 6 and 2
    // until typed, and the note says what holds the part.
    (form.lead, form.typed_lead, form.tabs) = (5.0, true, 2);
    h.frames(3);
    assert!(h.widget("3 × 3.67 mm").is_some(), "11 mm in three levels");
    assert!(h.widget(tr("manufacture.contour_tabs_note")).is_some() && h.widget(tr("manufacture.contour_loose")).is_none());
    assert!(h.widget(tr("manufacture.contour_tab_width")).is_some() && h.widget(tr("manufacture.contour_tab_height")).is_some());
    let form = contour_form(&mut h);
    assert_eq!((form.tab_width, form.tab_height, form.worked.trouble()), (6.0, 2.0, None));
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][1]["operation"]["contour"].clone();
    assert_eq!(
        (kept["bottom"].as_f64(), kept["stepdown"].as_f64(), kept["leave"].as_f64(), kept["direction"].as_str(), kept["rpm"].clone()),
        (Some(-11.0), Some(4.0), Some(0.5), Some("conventional"), Value::Null)
    );
    // The tabs are kept as a number of them: their size stays the engine's own until typed.
    assert_eq!((kept["lead"].as_f64(), kept["tabs"].clone()), (Some(5.0), json!({"count": 2, "width": null, "height": null})));
    assert_eq!(h.app.run("cam.op.toolpath", json!({"id": "op-2"})).unwrap()["numbers"]["tab_top"], json!(-9.0));
    // Opened again, the form shows them; with the tabs taken away it keeps none.
    let mut form = ContourForm::edit(&h.app, "op-2").unwrap();
    assert_eq!((form.lead, form.typed_lead, form.tabs, form.tab_width, form.typed_tab_width), (5.0, true, 2, 6.0, false));
    assert_eq!(form.params()["contour"]["tabs"], json!({"count": 2}));
    (form.tab_height, form.typed_tab_height) = (1.5, true);
    assert_eq!(form.params()["contour"]["tabs"], json!({"count": 2, "height": 1.5}));
    form.tabs = 0;
    assert_eq!(form.params()["contour"].get("tabs"), None);
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-2"})).unwrap();
    assert_eq!((made["numbers"]["levels"].as_array().map(Vec::len), made["numbers"]["offset"].as_f64()), (Some(3), Some(10.5)));
}

#[test]
fn contour_says_why_it_would_cut_into_the_part_and_its_form_changes_only_what_it_shows() {
    let mut h = harness();
    // A 20 x 20 foot under a 40 x 40 cap: round the foot's outline the tool would cut the cap.
    h.app.run("solid.box", json!({"length": 20, "width": 20, "height": 10, "corner": [10, 10, 0], "body_name": "Part"})).unwrap();
    h.app.run("solid.box", json!({"length": 40, "width": 40, "height": 10, "corner": [0, 0, 10], "operation": "join", "targets": ["Part"]})).unwrap();
    add_mill(&mut h, "aluminium");
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.contour");
    let form = contour_form(&mut h);
    let why = form.worked.trouble().unwrap_or_default().to_string();
    assert!(why.contains("wider above the contour's bottom") && !why.starts_with("operation"), "{why:?}");
    // The part's bottom is shown all the same: it is the number to change.
    assert_eq!((form.bottom, form.typed_bottom), (-21.0, false));
    assert!(h.widget(&why).is_some());
    assert_eq!(scene::counts(&h.app).2, 0, "no toolpath to draw");
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("contour", json!([])));
    // Down to the cap's bottom only (Z -11), the outline is the cap's and the contour is made.
    let form = contour_form(&mut h);
    (form.bottom, form.typed_bottom) = (-11.0, true);
    h.frames(3);
    assert!(contour_form(&mut h).worked.trouble().is_none() && scene::counts(&h.app).2 > 0);
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    assert_eq!(data(&mut h)["setups"][0]["operations"][0]["operation"]["contour"]["bottom"], json!(-11.0));

    // A contour made by a script, with parts the form does not show (its top, its clearance):
    // opened and left alone, the form gives back what the design keeps.
    h.app
        .run("cam.op.contour", json!({"setup": "setup-1", "tool": 1, "contour": {"top": -1.0, "bottom": -11.0, "clear": 4.0, "stepdown": 5.0}}))
        .unwrap();
    let form = ContourForm::edit(&h.app, "op-2").unwrap();
    assert_eq!(
        form.params(),
        json!({"setup": "setup-1", "tool": 1, "contour": {"top": -1.0, "bottom": -11.0, "stepdown": 5.0, "direction": "climb", "clear": 4.0, "side": "outside", "openings": [], "ramp_angle": null}})
    );
    assert!(ContourForm::edit(&h.app, "op-9").is_err());
    // Its row's Edit opens the form on what it keeps; a change is one edit of the operation.
    h.frames(3);
    row_menu(&mut h, "Contour 2 (T1)", "manufacture.edit");
    assert_eq!(open(&h), "contour");
    assert!(h.widget("Contour 2").is_some() && h.widget("OK").is_some() && h.widget("Create").is_none());
    assert!(h.widget(tr("manufacture.preview_note_edit")).is_some());
    let form = contour_form(&mut h);
    assert_eq!((form.bottom, form.typed_bottom, form.stepdown, form.typed_stepdown, form.typed_rpm), (-11.0, true, 5.0, true, false));
    form.leave = 0.2;
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][1].clone();
    assert_eq!((kept["id"].as_str(), kept["name"].as_str(), kept["tool"].as_u64()), (Some("op-2"), Some("Contour 2"), Some(1)));
    assert_eq!(
        kept["operation"]["contour"],
        json!({"top": -1.0, "bottom": -11.0, "stepdown": 5.0, "leave": 0.2, "direction": "climb", "clear": 4.0, "rpm": null, "feed": null, "lead": null, "tabs": null, "side": "outside", "openings": [], "ramp_angle": null})
    );
    // A double click on the row opens the same form.
    h.step(&json!({"click": {"browser": "Contour 1 (T1)"}, "double": true})).unwrap();
    h.frames(3);
    assert_eq!(open(&h), "contour");
    press(&mut h, "Cancel");
}

/// A 100 x 80 x 20 plate with a pocket 60 x 40, 6 deep, in the middle of its top, and a round
/// one of 16, 4 deep, near a corner.
fn pocketed_plate(h: &mut Harness) {
    h.app.run("solid.box", json!({"length": 100, "width": 80, "height": 20, "corner": [0, 0, 0], "body_name": "Plate"})).unwrap();
    h.app
        .run("solid.box", json!({"length": 60, "width": 40, "height": 6, "corner": [20, 20, 14], "operation": "cut", "targets": ["Plate"]}))
        .unwrap();
    h.app.run("solid.cylinder", json!({"diameter": 16, "height": 10, "base": [10, 70, 16], "operation": "cut", "targets": ["Plate"]})).unwrap();
}

#[test]
fn pocket_offers_the_pockets_found_and_the_widest_tool_that_fits() {
    let mut h = harness();
    // No pockets to find in a plain block: the form says what a pocket is.
    block(&mut h);
    add_tool(&mut h, "flat_end_mill", 10.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.pocket");
    assert_eq!(open(&h), "pocket");
    assert!(h.widget(tr("manufacture.pocket_none")).is_some());
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("pocket", json!([])));
    press(&mut h, "Cancel");

    // The plate's two pockets, the larger first. T1 is the 10 mm end mill, T2 a 20 mm one.
    let mut h = harness();
    pocketed_plate(&mut h);
    add_tool(&mut h, "flat_end_mill", 10.0);
    add_tool(&mut h, "flat_end_mill", 20.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.pocket");
    let form = pocket_form(&mut h);
    let found: Vec<(f64, [f64; 2])> = form.pockets().iter().map(|p| (p.floor, p.size)).collect();
    assert_eq!(found, vec![(-7.0, [60.0, 40.0]), (-5.0, [16.0, 16.0])]);
    // Both at once, to begin with: the widest tool that passes everywhere in both is the 10 mm
    // one (the round pocket is 16 across), and both are previewed. Their levels and rings are
    // each pocket's own, so the form gives none.
    assert_eq!((form.pocket, form.picked().len(), form.tool, form.picked_tool, form.stepover, form.stepdown), (0, 2, 0, false, 4.0, 5.0));
    assert_eq!(form.params()["pocket"]["pockets"].as_array().map(Vec::len), Some(2));
    assert!(h.widget("All 2 pockets").is_some() && h.widget(tr("manufacture.pocket_rings")).is_none());
    assert!(!pocket_form(&mut h).tool_too_wide() && scene::counts(&h.app).2 > 0 && h.widget(tr("manufacture.preview_note")).is_some());
    let both = scene::counts(&h.app).2;
    // The big one alone: the 20 mm tool fits and is the widest that does. Its centre's 40 x 20
    // and one ring 8 in; 7 mm deep in one level of at most 10.
    pocket_form(&mut h).pocket = 1;
    h.frames(3);
    let form = pocket_form(&mut h);
    assert_eq!((form.picked().len(), form.tool, form.picked_tool, form.stepover, form.stepdown), (1, 1, false, 8.0, 10.0));
    assert!(h.widget("60.00 × 40.00 × 6.00 mm").is_some(), "the pocket by its size and its depth below the part's top");
    assert!(h.widget("1 × 7.00 mm").is_some() && !pocket_form(&mut h).tool_too_wide());
    assert!(scene::counts(&h.app).2 > 0 && scene::counts(&h.app).2 < both);
    // The round one: only the 10 mm tool fits it. 5 mm deep under the stock's top, one level;
    // the centre's circle of radius 3 is the one ring.
    pocket_form(&mut h).pocket = 2;
    h.frames(3);
    let form = pocket_form(&mut h);
    assert_eq!((form.tool, form.stepover, form.stepdown), (0, 4.0, 5.0));
    assert!(h.widget("1 × 5.00 mm").is_some(), "one level of 5");
    // A tool picked by hand stays picked; one too wide for the pocket is pointed out and the
    // engine says how wide the pocket is.
    let form = pocket_form(&mut h);
    (form.tool, form.picked_tool) = (1, true);
    h.frames(3);
    let form = pocket_form(&mut h);
    assert!(form.tool_too_wide());
    let why = form.worked.trouble().unwrap_or_default().to_string();
    assert!(why.contains("does not fit") && why.contains("16.0"), "{why:?}");
    assert!(h.widget(tr("manufacture.pocket_tool_wide")).is_some() && h.widget(&why).is_some());
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("pocket", json!([])));
    // The 10 mm tool in the big pocket, by hand: four rings in two levels of 3.5.
    let form = pocket_form(&mut h);
    (form.pocket, form.tool) = (1, 0);
    h.frames(3);
    assert!(h.widget("2 × 3.50 mm").is_some() && h.widget("4").is_some());
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    // Only the pocket's point and the direction are kept: the rest follows the tool.
    let kept = data(&mut h)["setups"][0]["operations"][0].clone();
    assert_eq!((kept["name"].as_str(), kept["tool"].as_u64(), kept["operation"]["kind"].as_str()), (Some("Pocket 1"), Some(1), Some("pocket")));
    let pocket = &kept["operation"]["pocket"];
    assert_eq!(
        (pocket["pockets"].as_array().map(Vec::len), pocket["direction"].as_str(), pocket["stepover"].clone(), pocket["stepdown"].clone()),
        (Some(1), Some("climb"), Value::Null, Value::Null)
    );
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    assert_eq!((made["numbers"]["rings"].clone(), made["numbers"]["levels"].clone()), (json!([4]), json!([[-3.5, -7.0]])));

    // Its row's Edit opens the form on the pocket it has; a change is one edit of the operation.
    h.frames(3);
    row_menu(&mut h, "Pocket 1 (T1)", "manufacture.edit");
    assert_eq!(open(&h), "pocket");
    assert!(h.widget("1, as made").is_some() && h.widget("OK").is_some() && h.widget("Create").is_none());
    let form = pocket_form(&mut h);
    assert_eq!((form.tool, form.picked_tool, form.typed_stepover), (0, true, false));
    (form.stepover, form.typed_stepover) = (5.0, true);
    form.leave = 0.25;
    form.direction = 1;
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let after = data(&mut h)["setups"][0]["operations"][0]["operation"]["pocket"].clone();
    assert_eq!(
        (after["pockets"].clone(), after["stepover"].as_f64(), after["leave"].as_f64(), after["direction"].as_str()),
        (pocket["pockets"].clone(), Some(5.0), Some(0.25), Some("conventional"))
    );
    assert!(PocketForm::edit(&h.app, "op-9").is_err());
    // Left at its defaults, the form makes one operation of both pockets with the tool that
    // suits both: Create is all it takes.
    ribbon(&mut h, "cam.op.pocket");
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][1].clone();
    assert_eq!((kept["tool"].as_u64(), kept["operation"]["pocket"]["pockets"].as_array().map(Vec::len)), (Some(1), Some(2)));
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-2"})).unwrap();
    assert_eq!(made["numbers"]["levels"], json!([[-3.5, -7.0], [-5.0]]));
    // One pocket found is offered as it is, with no "all".
    let mut h = harness();
    h.app.run("solid.box", json!({"length": 100, "width": 80, "height": 20, "corner": [0, 0, 0], "body_name": "Plate"})).unwrap();
    h.app
        .run("solid.box", json!({"length": 60, "width": 40, "height": 6, "corner": [20, 20, 14], "operation": "cut", "targets": ["Plate"]}))
        .unwrap();
    add_tool(&mut h, "flat_end_mill", 10.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.pocket");
    let form = pocket_form(&mut h);
    assert_eq!((form.pocket, form.picked().len(), form.pockets().len()), (0, 1, 1));
    assert!(h.widget("60.00 × 40.00 × 6.00 mm").is_some() && h.widget("2 × 3.50 mm").is_some());
}

#[test]
fn simulate_draws_the_stock_as_it_is_left_and_says_what_it_found() {
    let mut h = harness();
    // Nothing to simulate yet: the window says what to make first.
    ribbon(&mut h, "cam.simulate");
    assert_eq!(open(&h), "simulate");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    press(&mut h, "Close");
    assert_eq!(open(&h), "none");

    // The plate with its two pockets, faced with the 20 mm tool (T2) and the big pocket cut
    // with the 10 mm one (T1).
    pocketed_plate(&mut h);
    add_tool(&mut h, "flat_end_mill", 10.0);
    add_tool(&mut h, "flat_end_mill", 20.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    h.app.run("cam.op.face", json!({"setup": "setup-1", "tool": 2})).unwrap();
    h.app.run("cam.op.pocket", json!({"setup": "setup-1", "tool": 1, "pocket": {"pockets": [[0, 0]]}})).unwrap();
    h.frames(3);
    assert_eq!((scene::counts(&h.app).0, scene::stock_triangles(&h.app)), (12, 0), "the stock's box, and no simulated stock");
    ribbon(&mut h, "cam.simulate");
    let simulated = simulate_form(&mut h).simulated().expect("a simulation");
    assert_eq!((simulated.stock.gouge_count, simulated.stock.rapid_hit_count, simulated.operations.len()), (0, 0, 2));
    // The stock is 102 x 82 x 21: 175.64 cm3. Facing takes the 1 mm on top, 8.36 cm3.
    assert!(h.widget("175.64 cm³").is_some() && h.widget("8.36 cm³").is_some());
    assert!(h.widget("Face 1").is_some() && h.widget("Pocket 2").is_some());
    assert!(h.widget(tr("manufacture.sim_no_gouge")).is_some() && h.widget(tr("manufacture.sim_no_rapid")).is_some());
    assert!(h.widget(tr("manufacture.sim_no_body")).is_some());
    assert!(h.widget(tr("manufacture.sim_note")).is_some(), "a check, not a proof");
    // On the model: the stock as it is left, in place of its box (whose edges stay) and of the
    // toolpaths that left it so.
    assert_eq!(scene::counts(&h.app), (0, 15, 0));
    assert!(scene::stock_triangles(&h.app) > 100_000, "{}", scene::stock_triangles(&h.app));
    // The surface is a hair above the plate's top (model Z 20) and the pocket's floor (Z 14),
    // and it comes down on the open side of the pocket's walls, where the model's own faces
    // can't hide it: every piece of it that goes below the top lies wholly over the pocket's
    // opening (X 20 to 80, Y 20 to 60), none of it behind a wall. The millimetre of stock still
    // standing all round the plate has its sides too, outside the plate's own walls and down
    // to the stock's bottom (Z 0).
    let surface = scene::stock_surface(&h.app);
    let low: Vec<_> = surface.iter().filter(|triangle| triangle.iter().any(|v| v[2] < 19.9)).collect();
    assert!(low.len() > 50_000, "{}", low.len());
    let over_pocket = |v: &[f32; 3]| (20.0..=80.0).contains(&v[0]) && (20.0..=60.0).contains(&v[1]);
    let beside_plate = |v: &[f32; 3]| !(0.0..=100.0).contains(&v[0]) || !(0.0..=80.0).contains(&v[1]);
    let sides = low.iter().filter(|triangle| triangle.iter().all(beside_plate)).count();
    let behind = low.iter().filter(|triangle| !triangle.iter().all(over_pocket) && !triangle.iter().all(beside_plate)).count();
    assert!(behind == 0 && sides > 1_000, "{behind} behind a wall and {sides} at the stock's sides, of {}", low.len());
    let heights = |from: f32, to: f32| surface.iter().flatten().filter(|v| v[2] > from && v[2] < to).count();
    assert!(heights(20.0, 20.04) > 100_000 && heights(14.0, 14.04) > 100_000 && heights(14.04, 20.0) == 0 && heights(20.04, 30.0) == 0);
    assert!(heights(-0.01, 0.04) > 1_000 && heights(0.04, 14.0) == 0, "the sides reach the stock's bottom");
    press(&mut h, "Close");
    assert_eq!((open(&h), scene::counts(&h.app).0, scene::stock_triangles(&h.app)), ("none", 12, 0));

    // A toolpath made by hand that goes 9 mm into the plate and then rapids through the stock:
    // the window says how many moves, how deep and in which operation.
    let at = |x: f64, y: f64, z: f64| json!([x, y, z]);
    let path = json!({"start": at(-30.0, -30.0, 25.0), "moves": [
        {"kind": "tool_change", "tool": 1},
        {"kind": "spindle", "rpm": 4000.0, "direction": "clockwise"},
        {"kind": "rapid", "to": at(-30.0, -30.0, 5.0)},
        {"kind": "linear", "to": at(-30.0, -30.0, -10.0), "feed": 200.0},
        {"kind": "linear", "to": at(30.0, -30.0, -10.0), "feed": 400.0},
        {"kind": "rapid", "to": at(30.0, -30.0, 5.0)},
        {"kind": "rapid", "to": at(30.0, 30.0, 5.0)},
        {"kind": "rapid", "to": at(30.0, 30.0, -4.0)},
        {"kind": "rapid", "to": at(30.0, 30.0, 25.0)},
        {"kind": "spindle_stop"}
    ]});
    h.app.run("cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": path, "name": "By hand"})).unwrap();
    ribbon(&mut h, "cam.simulate");
    let simulated = simulate_form(&mut h).simulated().expect("a simulation");
    assert_eq!((simulated.stock.gouge_count, simulated.stock.rapid_hit_count), (2, 2));
    let gouges = solidcraft_ui_text::format("manufacture.sim_gouges", &[("count", "2"), ("depth", "9.00 mm"), ("operation", "By hand")]);
    let rapids = solidcraft_ui_text::format("manufacture.sim_rapids", &[("count", "2"), ("depth", "3.00 mm"), ("operation", "By hand")]);
    assert!(h.widget(&gouges).is_some() && h.widget(&rapids).is_some(), "{gouges} / {rapids}");
    assert!(h.widget(tr("manufacture.sim_no_gouge")).is_none());
    press(&mut h, "Close");
}

#[test]
fn drill_taps_with_a_tap_at_its_pitch_and_a_speed_only() {
    let mut h = harness();
    holed(&mut h);
    // T1 a 6 mm drill, T2 a tap of 7 with Add Tool's 1 mm pitch.
    add_tool(&mut h, "drill", 6.0);
    add_tool(&mut h, "tap", 7.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    ribbon(&mut h, "cam.op.drill");
    // The holes of 6 get the drill: a tap is never picked for a hole's width.
    let form = drill_form(&mut h);
    assert_eq!((form.group, form.tool, form.depth), (0, 0, 21.0));
    assert!(h.widget(tr("manufacture.drill_cycle")).is_some() && h.widget(tr("manufacture.tool_thread_pitch")).is_none());
    // The tap picked: its own cycle at its pitch, and through the part's 20 mm the tip goes 1 mm
    // and three pitches further, so that its full thread is through and not only its chamfer.
    let form = drill_form(&mut h);
    (form.tool, form.picked_tool) = (1, true);
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!(form.depth, 24.0);
    let drill = form.params()["drill"].clone();
    assert_eq!(
        (drill["cycle"].clone(), drill["depth"].as_f64(), drill["depth_to"].as_str()),
        (json!({"type": "tap", "pitch": 1.0}), Some(24.0), Some("tip"))
    );
    // 200 m/min on 7 mm is 9095 rev/min, to the whole turn, and one pitch a turn.
    assert_eq!((form.rpm, form.feed, form.worked.trouble()), (9095.0, 9095.0, None));
    assert!(h.widget(tr("manufacture.tool_thread_pitch")).is_some() && h.widget("1.00 mm").is_some());
    assert!(h.widget(tr("manufacture.drill_cycle")).is_none() && h.widget(tr("manufacture.drill_tool_wider")).is_none());
    assert!(h.widget(tr("manufacture.drill_depth_tap")).is_some() && h.widget(tr("manufacture.drill_tap_feed")).is_some());
    assert!(h.widget("9095.00 mm/min").is_some(), "the feed is shown, not typed");
    // A speed typed is the tap's; the feed follows it and is never sent.
    let form = drill_form(&mut h);
    (form.rpm, form.typed_rpm, form.typed_feed) = (400.0, true, true);
    h.frames(3);
    let form = drill_form(&mut h);
    assert_eq!((form.feed, form.params()["drill"].get("feed"), form.params()["drill"]["rpm"].as_f64()), (400.0, None, Some(400.0)));
    assert!(h.widget("400.00 mm/min").is_some());
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][0].clone();
    assert_eq!((kept["tool"].as_u64(), kept["operation"]["drill"]["cycle"].clone()), (Some(2), json!({"type": "tap", "pitch": 1.0})));
    assert_eq!((kept["operation"]["drill"]["rpm"].as_f64(), kept["operation"]["drill"]["feed"].clone()), (Some(400.0), Value::Null));
    // Opened again it is still a tapped hole: the pitch shown, no cycle to choose.
    let form = DrillForm::edit(&h.app, "op-1").unwrap();
    assert_eq!((form.tool, form.params()["drill"]["cycle"].clone()), (1, json!({"type": "tap", "pitch": 1.0})));
    // Blind holes are tapped to one pitch short of their bottom: the two of 8 that are 10 deep.
    ribbon(&mut h, "cam.op.drill");
    let form = drill_form(&mut h);
    (form.group, form.tool, form.picked_tool) = (1, 1, true);
    h.frames(3);
    assert_eq!(drill_form(&mut h).depth, 9.0);
    press(&mut h, "Cancel");
}

fn chamfer_form(h: &mut Harness) -> &mut ChamferForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Chamfer(form)) => form,
        _ => panic!("Chamfer is not open"),
    }
}

#[test]
fn chamfer_breaks_the_top_edge_and_keeps_only_what_is_typed() {
    use std::f64::consts::PI;
    let mut h = harness();
    // Nothing to chamfer or with: the form says what to make first.
    ribbon(&mut h, "cam.op.chamfer");
    assert_eq!(open(&h), "chamfer");
    assert!(h.widget(tr("manufacture.no_setup")).is_some());
    press(&mut h, "Cancel");
    block(&mut h);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    add_tool(&mut h, "flat_end_mill", 10.0);
    ribbon(&mut h, "cam.op.chamfer");
    assert!(h.widget(tr("manufacture.no_tool_for_chamfer")).is_some(), "an end mill cuts no chamfer");
    press(&mut h, "Create");
    assert_eq!((open(&h), data(&mut h)["setups"][0]["operations"].clone()), ("chamfer", json!([])));
    press(&mut h, "Cancel");

    // With a 10 mm, 90 degree chamfer mill (T2): half a millimetre wide, the tip 1 below it. The
    // block's top is Z -1 under 1 mm of stock, so the tip runs at Z -2.5.
    add_tool(&mut h, "chamfer_mill", 10.0);
    ribbon(&mut h, "cam.op.chamfer");
    let form = chamfer_form(&mut h);
    assert_eq!((form.tool, form.width, form.typed_width, form.tip_below, form.typed_tip_below, form.direction), (0, 0.5, false, 1.0, false, 0));
    // 200 m/min on its 10 mm, and its own 0.05 mm a tooth on four teeth: nothing is added for
    // the chip a narrow cut thins.
    let rpm = 200_000.0 / (10.0 * PI);
    let feed = 0.05 * 4.0 * rpm;
    assert!(form.rpm == rpm.round() && (form.feed - feed).abs() < 1e-6, "{} rev/min, {} mm/min: {:?}", form.rpm, form.feed, form.worked.trouble());
    assert!(h.widget("-2.50 mm").is_some(), "where the tip runs");
    assert!(h.widget(tr("manufacture.chamfer_note")).is_some() && h.widget(tr("manufacture.preview_note")).is_some());
    assert!(scene::counts(&h.app).2 > 0, "drawn before it exists");
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    // Nothing was typed, so only the direction is kept.
    assert_eq!(
        data(&mut h)["setups"][0]["operations"][0]["operation"],
        json!({"kind": "chamfer", "chamfer": {"width": null, "tip_below": null, "top": null, "direction": "climb", "lead": null, "rpm": null, "feed": null}})
    );
    let made = h.app.run("cam.op.toolpath", json!({"id": "op-1"})).unwrap();
    assert_eq!((made["numbers"]["z"].as_f64(), made["numbers"]["offset"].as_f64()), (Some(-2.5), Some(1.0)));

    // What is typed is kept: 2 mm wide with the tip half a millimetre below, the other way round.
    ribbon(&mut h, "cam.op.chamfer");
    let form = chamfer_form(&mut h);
    (form.width, form.typed_width, form.tip_below, form.typed_tip_below, form.direction) = (2.0, true, 0.5, true, 1);
    h.frames(3);
    assert!(h.widget("-3.50 mm").is_some(), "2 down the wall and half a millimetre more");
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][1]["operation"]["chamfer"].clone();
    assert_eq!(
        (kept["width"].as_f64(), kept["tip_below"].as_f64(), kept["direction"].as_str(), kept["rpm"].clone()),
        (Some(2.0), Some(0.5), Some("conventional"), Value::Null)
    );
    // A chamfer too wide for the tool says which number to change, and nothing is made.
    ribbon(&mut h, "cam.op.chamfer");
    let form = chamfer_form(&mut h);
    (form.width, form.typed_width) = (4.5, true);
    h.frames(3);
    let why = chamfer_form(&mut h).worked.trouble().unwrap_or_default().to_string();
    assert!(why.contains("a narrower chamfer, the tip less far below, or a bigger tool"), "{why:?}");
    assert!(h.widget(&why).is_some());
    press(&mut h, "Cancel");
    // Its row's Edit opens the form on what it keeps.
    let form = ChamferForm::edit(&h.app, "op-2").unwrap();
    assert_eq!((form.width, form.typed_width, form.tip_below, form.direction, form.typed_rpm), (2.0, true, 0.5, 1, false));
    assert_eq!(form.params()["chamfer"], json!({"width": 2.0, "tip_below": 0.5, "top": null, "direction": "conventional", "lead": null}));
    assert!(ChamferForm::edit(&h.app, "op-9").is_err());
}

#[test]
fn the_setup_sheet_is_saved_where_the_save_prompt_says() {
    let mut h = harness();
    let status = |h: &Harness| h.app.status.as_ref().map(|s| (s.0.clone(), s.2));
    // No setup yet: said, and no file is asked for.
    h.app.start("cam.setup.sheet");
    h.frames(3);
    assert_eq!(status(&h), Some((tr("manufacture.no_setup").to_string(), true)));
    // The block faced with the 20 mm end mill: the button asks where to save, and the page for
    // the setup the viewport draws is written there. No form opens.
    block(&mut h);
    add_mill(&mut h, "aluminium");
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    h.app.run("cam.op.face", json!({"setup": "setup-1", "tool": 1})).unwrap();
    ribbon(&mut h, "cam.setup.sheet");
    h.frames(5);
    let file = h.dir.join(file_name(&format!("{} setup sheet", h.app.session.doc.name), "setup-sheet", "html"));
    let path = file.to_string_lossy().to_string();
    assert!(file.exists(), "{path}: {:?}", h.app.status);
    assert_eq!(status(&h), Some((solidcraft_ui_text::format("manufacture.sheet_saved", &[("path", &path)]), false)));
    assert_eq!(open(&h), "none");
    let html = std::fs::read_to_string(&file).unwrap();
    assert!(html.starts_with("<!DOCTYPE html>") && html.contains("Face 1") && html.contains("<svg") && html.contains("Unverified program"), "{html}");
    assert!(html.contains("<tr><th>Stock</th><td>62.00 × 42.00 × 21.00 mm, aluminium</td></tr>"));
}

#[test]
fn simulate_says_when_a_holder_is_in_the_stock() {
    let mut h = harness();
    // The plate's big pocket (6 deep under 1 mm of stock) cut with a 10 mm end mill that sticks
    // only 5 out of a holder 40 across: at the floor the holder's face is 2 into the stock.
    pocketed_plate(&mut h);
    add_tool(&mut h, "flat_end_mill", 10.0);
    let mut tool = data(&mut h)["tools"][0].clone();
    tool["holder"] = json!({"stick_out": 5.0, "segments": [{"length": 30.0, "lower_diameter": 40.0, "upper_diameter": 40.0}]});
    h.app.run("cam.tool.edit", json!({"number": 1, "tool": tool})).unwrap();
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    h.app.run("cam.op.pocket", json!({"setup": "setup-1", "tool": 1, "pocket": {"pockets": [[0, 0]]}})).unwrap();
    h.frames(3);
    ribbon(&mut h, "cam.simulate");
    let simulated = simulate_form(&mut h).simulated().expect("a simulation");
    let count = simulated.stock.body_hit_count;
    assert!(count > 0 && simulated.stock.gouge_count == 0, "{count}");
    let said = solidcraft_ui_text::format(
        "manufacture.sim_body",
        &[("count", &count.to_string()), ("depth", "2.00 mm"), ("part", tr("manufacture.sim_part_holder")), ("operation", "Pocket 1")],
    );
    assert!(h.widget(&said).is_some(), "{said}");
    assert!(h.widget(tr("manufacture.sim_no_body")).is_none() && h.widget(tr("manufacture.sim_no_gouge")).is_some());
    press(&mut h, "Close");
}

#[test]
fn the_browser_copies_reorders_and_leaves_out_operations() {
    let mut h = harness();
    // The block faced (op-1), then contoured with the same tool (op-2).
    faced(&mut h);
    h.app.run("cam.op.contour", json!({"setup": "setup-1", "tool": 1})).unwrap();
    h.frames(3);
    let order = |h: &mut Harness| -> Vec<String> {
        let ops = data(h)["setups"][0]["operations"].as_array().unwrap().clone();
        ops.iter()
            .map(|o| format!("{} {}{}", o["id"].as_str().unwrap(), o["name"].as_str().unwrap(), if o["suppressed"] == true { " (off)" } else { "" }))
            .collect()
    };
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-2 Contour 2"]);
    // Move down on the facing puts it after the contour; Move up brings it back. The first row
    // has no way up and the last no way down.
    row_menu(&mut h, "Face 1 (T1)", "manufacture.move_down");
    assert_eq!(order(&mut h), ["op-2 Contour 2", "op-1 Face 1"]);
    row_menu(&mut h, "Face 1 (T1)", "manufacture.move_up");
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-2 Contour 2"]);
    // (Their entries are there, greyed: asked all the same, nothing moves.)
    row_menu(&mut h, "Face 1 (T1)", "manufacture.move_up");
    row_menu(&mut h, "Contour 2 (T1)", "manufacture.move_down");
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-2 Contour 2"]);
    // Duplicate adds a copy right after, named for what it copies.
    row_menu(&mut h, "Face 1 (T1)", "cam.op.duplicate");
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-3 Face 1 (2)", "op-2 Contour 2"]);
    // Suppress leaves an operation out of the program and of the drawing; the same entry, then
    // Unsuppress, puts it back.
    let lines = scene::counts(&h.app).2;
    row_menu(&mut h, "Face 1 (2) (T1)", "cam.op.edit");
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-3 Face 1 (2) (off)", "op-2 Contour 2"]);
    assert!(scene::counts(&h.app).2 < lines, "its toolpath is no longer drawn");
    row_menu(&mut h, "Face 1 (2) (T1)", "cam.op.edit");
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-3 Face 1 (2)", "op-2 Contour 2"]);
    assert_eq!(scene::counts(&h.app).2, lines);
    // Each is one step to undo.
    h.app.run("edit.undo", json!({})).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    h.app.run("edit.undo", json!({})).unwrap();
    assert_eq!(order(&mut h), ["op-1 Face 1", "op-2 Contour 2"]);
}

fn bore_form(h: &mut Harness) -> &mut BoreForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Bore(form)) => form,
        _ => panic!("Bore is not open"),
    }
}

#[test]
fn bore_offers_the_holes_and_the_widest_end_mill_that_leaves_room_for_its_helix() {
    let mut h = harness();
    // The block (60 x 40 x 20) with a hole of 24 through it and a hole of 16, 8 deep, and three
    // end mills: 20 (T1), 10 (T2) and 6 mm (T3).
    block(&mut h);
    h.app.run("solid.cylinder", json!({"diameter": 24, "height": 30, "base": [18, 20, -5], "operation": "cut"})).unwrap();
    h.app.run("solid.cylinder", json!({"diameter": 16, "height": 8, "base": [45, 20, 12], "operation": "cut"})).unwrap();
    add_tool(&mut h, "flat_end_mill", 20.0);
    add_tool(&mut h, "flat_end_mill", 10.0);
    add_tool(&mut h, "flat_end_mill", 6.0);
    h.app.run("cam.setup.create", json!({"material": "aluminium"})).unwrap();
    panel_menu(&mut h, "MILLING", "cam.op.bore");
    assert_eq!(open(&h), "bore");
    h.frames(3);
    // The holes Drill offers, and for the first group the widest tool that is no more than nine
    // tenths of the hole: the 20 mm one goes into the hole of 24, the 10 mm one into the 16.
    let form = bore_form(&mut h);
    let sizes: Vec<(f64, usize)> = form.groups().iter().map(|group| (group.diameter, group.holes.len())).collect();
    assert_eq!(sizes.len(), 2, "{sizes:?}");
    let through = form.groups().iter().position(|group| group.diameter == 24.0).unwrap();
    form.group = through;
    h.frames(3);
    let form = bore_form(&mut h);
    assert_eq!((form.tool, form.picked_tool), (0, false));
    form.group = 1 - through;
    h.frames(3);
    let form = bore_form(&mut h);
    assert_eq!(form.tool, 1, "the 10 mm tool for the hole of 16");
    assert!(h.widget(tr("manufacture.bore_note")).is_some() && h.widget(tr("manufacture.preview_note")).is_some());
    assert!(scene::counts(&h.app).2 > 0, "drawn before it exists");
    // Create: an inside contour named Bore on the hole's centre, down to its floor (Z -9: the
    // block's top is at -1 under the stock and the hole 8 deep), with nothing typed kept.
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][0].clone();
    assert_eq!((kept["name"].as_str(), kept["tool"].as_u64(), kept["operation"]["kind"].as_str()), (Some("Bore 1"), Some(2), Some("contour")));
    let contour = &kept["operation"]["contour"];
    assert_eq!(
        (contour["side"].clone(), contour["openings"].clone(), contour["bottom"].clone()),
        (json!("inside"), json!([[15.0, 0.0]]), json!(-9.0))
    );
    assert_eq!((contour["rpm"].clone(), contour["feed"].clone(), contour["direction"].clone()), (json!(null), json!(null), json!("climb")));
    // The hole through the block next: bored through, with the 20 mm tool.
    panel_menu(&mut h, "MILLING", "cam.op.bore");
    h.frames(3);
    bore_form(&mut h).group = through;
    h.frames(3);
    press(&mut h, "Create");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][1].clone();
    assert_eq!(
        (kept["name"].as_str(), kept["tool"].as_u64(), kept["operation"]["contour"]["bottom"].clone()),
        (Some("Bore 2"), Some(1), json!(null))
    );
    // The simulation: both holes cut, nothing of the block cut into.
    let sim = h.app.run("cam.simulate", json!({"setup": "setup-1"})).unwrap();
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    // Edit on a bore's row opens Bore's form on it, on the hole it has.
    row_menu(&mut h, "Bore 1 (T2)", "manufacture.edit");
    assert_eq!(open(&h), "bore");
    let count = solidcraft_ui_text::format("manufacture.drill_holes_kept", &[("count", "1")]);
    assert!(h.widget(&count).is_some() && h.widget("OK").is_some());
    let form = bore_form(&mut h);
    (form.rpm, form.typed_rpm) = (3000.0, true);
    press(&mut h, "OK");
    assert_eq!(open(&h), "none", "{:?}", h.app.status);
    let kept = data(&mut h)["setups"][0]["operations"][0]["operation"]["contour"].clone();
    assert_eq!((kept["rpm"].clone(), kept["side"].clone(), kept["openings"].clone()), (json!(3000.0), json!("inside"), json!([[15.0, 0.0]])));
}
