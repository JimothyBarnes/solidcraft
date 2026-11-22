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
        Some(Dialog::Setup(_)) => "setup",
        Some(Dialog::Face(_)) => "face",
        Some(Dialog::Drill(_)) => "drill",
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

fn drill_form(h: &mut Harness) -> &mut DrillForm {
    match h.app.manufacture_workspace.dialog.as_mut() {
        Some(Dialog::Drill(form)) => form,
        _ => panic!("Drill is not open"),
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
        ("cam.op.drill", "DRILLING"),
        ("cam.post", "ACTIONS"),
        ("cam.tool.create", "MANAGE"),
    ];
    for (id, panel) in placed {
        assert_eq!(crate::workspace::destination(solidcraft_engine::find_command(id).unwrap()), Some(("MANUFACTURE", panel)), "{id}");
        assert!(crate::scenario::handle_at(&format!("toolbar:{id}")).is_some(), "{id} is on the ribbon");
    }
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
    form.flutes = form.usual_flutes();
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
