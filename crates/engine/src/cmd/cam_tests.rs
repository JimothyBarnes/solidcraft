use serde_json::{Value, json};

use crate::Session;

fn tool(number: u32) -> Value {
    json!({
        "number": number, "name": format!("6 mm flat T{number}"), "kind": "flat_end_mill",
        "geometry": {"diameter": 6.0, "corner_radius": 0.0, "flute_length": 18.0, "shoulder_length": 0.0, "overall_length": 60.0,
                     "shank_diameter": 6.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 3, "holder": {"stick_out": 25.0, "segments": []}, "length_offset": number, "diameter_offset": number,
        "coolant": "flood", "cutting_data": [], "overrides": {"rpm": null, "feed": null}
    })
}

/// A point as the CAM data writes it: `[x, y, z]` in mm.
fn p(x: f64, y: f64, z: f64) -> Value {
    json!([x, y, z])
}

/// A tool change, a plunge, a quarter arc and a retract on a 0.001 mm grid.
fn bracket() -> Value {
    json!({"start": p(0.0, 0.0, 50.0), "moves": [
        {"kind": "tool_change", "tool": 1},
        {"kind": "spindle", "rpm": 1200.0, "direction": "clockwise"},
        {"kind": "rapid", "to": p(10.0, 0.0, 5.0)},
        {"kind": "linear", "to": p(10.0, 0.0, -1.0), "feed": 200.0},
        {"kind": "arc", "to": p(0.0, 10.0, -1.0), "center": p(0.0, 0.0, -1.0), "plane": "xy", "direction": "counter_clockwise", "feed": 400.0},
        {"kind": "rapid", "to": p(0.0, 10.0, 5.0)},
        {"kind": "spindle_stop"}
    ]})
}

fn run(s: &mut Session, id: &str, params: Value) -> Value {
    s.execute(id, &params).unwrap_or_else(|e| panic!("{id} {params}: {e}"))
}

fn refused(s: &mut Session, id: &str, params: Value) -> String {
    match s.execute(id, &params) {
        Ok(v) => panic!("{id} {params} was accepted: {v}"),
        Err(e) => e.to_string(),
    }
}

/// A design with tool 1, the default setup and the hand-made bracket toolpath.
fn shop() -> Session {
    let mut s = Session::default();
    run(&mut s, "cam.tool.create", json!({"tool": tool(1)}));
    assert_eq!(run(&mut s, "cam.setup.create", json!({}))["id"], "setup-1");
    assert_eq!(run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": bracket()}))["id"], "op-1");
    s
}

#[test]
fn a_new_design_has_no_manufacture_data() {
    let mut s = Session::default();
    let d = run(&mut s, "cam.data", json!({}));
    assert_eq!((d["setups"].as_u64(), d["tools"].as_u64(), d["operations"].as_u64()), (Some(0), Some(0), Some(0)));
    assert!(s.doc.plugin_data.is_empty(), "reading writes nothing");
}

#[test]
fn a_setup_saves_with_the_design_reopens_and_its_data_is_ready_not_missing() {
    let mut s = shop();
    let before = run(&mut s, "cam.data", json!({}));
    let setup = &before["data"]["setups"][0];
    // The vise defaults: stock 1 mm over on the sides and the top, origin at the stock's top centre, G54.
    assert_eq!(setup["work_offset"], 1);
    assert_eq!(
        setup["stock"],
        json!({"kind": "relative_box", "offsets": {"x_min": 1.0, "x_max": 1.0, "y_min": 1.0, "y_max": 1.0, "z_min": 0.0, "z_max": 1.0}})
    );
    assert_eq!(setup["wcs"]["origin"]["anchor"], json!({"x": "center", "y": "center", "z": "max"}));
    assert_eq!(setup["safe_heights"], json!({"clearance": 25.0, "retract": 5.0}));
    assert!(s.doc.plugin_data.contains_key("solidcraft-cam/data.json") && s.doc.plugin_data.contains_key("solidcraft-cam/meta.json"));
    // Saved in the design container and opened again: the same data.
    let bytes = solidcraft_io::write_design_container(&s.doc).unwrap();
    let mut reopened = Session::new(solidcraft_io::read_design(&bytes).unwrap());
    assert_eq!(run(&mut reopened, "cam.data", json!({})), before);
    // This build writes Manufacture data itself, so the app doesn't call its plugin missing.
    let listed = run(&mut reopened, "storage.list", json!({}));
    let manufacture = listed["plugins"].as_array().unwrap().iter().find(|p| p["plugin"] == "solidcraft-cam").unwrap().clone();
    assert_eq!(manufacture["status"], "ready", "{manufacture}");
    assert_eq!(manufacture["read_only"], false);
}

#[test]
fn a_setup_posts_on_every_built_in_post_and_reads_back() {
    let mut s = shop();
    let fanuc = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "fanuc_mill"}));
    let text = fanuc["text"].as_str().unwrap();
    assert!(text.starts_with("%\nO0001 (SETUP 1)\nN10 (UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)\n"), "{text}");
    assert!(
        text.contains("\nN90 G43 H1 G0 X10. Y0. Z5.\n") && text.contains(" G3 X0. Y10. I-10. J0. F400.\n") && text.ends_with("M30\n%\n"),
        "{text}"
    );
    assert_eq!((fanuc["moves"].as_u64(), fanuc["machine_retracts"].as_u64()), (Some(7), Some(2)));
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill", "program_number": 42, "units": "inch"}));
    let text = haas["text"].as_str().unwrap();
    assert!(text.starts_with("%\nO00042 (SETUP 1)\n") && text.contains("G20") && text.contains("X0.3937 Y0. Z0.1969"), "{text}");
    let rs = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274", "program_name": "bracket"}));
    assert!(rs["text"].as_str().unwrap().starts_with("(bracket)\n(UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)\n"));
    // Posting reads the design; it never changes it.
    assert_eq!(s.undo.len(), 3);
}

#[test]
fn a_hand_made_hole_posts_as_a_canned_cycle_where_the_post_has_one() {
    let mut s = Session::default();
    run(&mut s, "cam.tool.create", json!({"tool": tool(1)}));
    run(&mut s, "cam.setup.create", json!({}));
    let holes = json!({"start": p(0.0, 0.0, 50.0), "moves": [
        {"kind": "tool_change", "tool": 1},
        {"kind": "rapid", "to": p(5.0, 5.0, 10.0)},
        {"kind": "drill", "to": p(5.0, 5.0, -8.0), "r": 2.0, "retract": 10.0, "cycle": {"type": "chip_break", "peck": 3.0}, "feed": 120.0},
        {"kind": "drill", "to": p(15.0, 5.0, -8.0), "r": 2.0, "retract": 10.0, "cycle": {"type": "chip_break", "peck": 3.0}, "feed": 120.0},
        {"kind": "rapid", "to": p(15.0, 5.0, 50.0)}
    ]});
    run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": holes}));
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = haas["text"].as_str().unwrap();
    assert!(text.contains("\nG98 G73 X5. Y5. Z-8. R2. Q3. F120.\nX15. Y5.\nG80\nG0 Z50.\n"), "{text}");
    assert_eq!(haas["moves"], 5);
    // RS274/NGC has no G73: the same holes as plain moves, said so in the notes.
    let rs = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"}));
    assert!(!rs["text"].as_str().unwrap().contains("G73"));
    assert!(rs["notes"].as_array().unwrap().iter().any(|n| n.as_str().unwrap().contains("plain moves")), "{rs}");
    // A hole that would start below its R plane is refused when it is added, not when it is posted.
    let low = json!({"start": p(0.0, 0.0, 1.0), "moves": [
        {"kind": "drill", "to": p(0.0, 0.0, -5.0), "r": 2.0, "retract": 2.0, "cycle": {"type": "drill", "dwell": 0.0}, "feed": 100.0}
    ]});
    assert!(refused(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": low})).contains("R plane"));
}

/// `[x, y, z]` from JSON, each within 1e-9 mm of `want`.
fn near(v: &Value, want: [f64; 3]) -> bool {
    v.as_array().is_some_and(|a| a.len() == 3 && a.iter().zip(want).all(|(x, w)| x.as_f64().is_some_and(|x| (x - w).abs() <= 1e-9)))
}

#[test]
fn a_setup_measures_its_stock_and_wcs_on_the_models_exact_box() {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0], "body_name": "Block"}));
    run(&mut s, "cam.setup.create", json!({}));
    let undo = s.undo.len();
    let m = run(&mut s, "cam.setup.measure", json!({"id": "setup-1"}));
    assert_eq!(m["bodies"], json!(["Block"]));
    assert!(near(&m["model_box"]["min"], [0.0, 0.0, 0.0]) && near(&m["model_box"]["max"], [60.0, 40.0, 20.0]), "{m}");
    // The vise defaults: stock 1 mm over on the sides and the top, so (-1, -1, 0) to (61, 41, 21),
    // and the origin at its top centre (30, 20, 21). Heights: stock top (0) + 25 and + 5.
    let placed = &m["placement"];
    assert!(near(&placed["origin"], [30.0, 20.0, 21.0]), "{placed}");
    assert!(near(&placed["stock"]["min"], [-31.0, -21.0, -21.0]) && near(&placed["stock"]["max"], [31.0, 21.0, 0.0]), "{placed}");
    assert!(near(&placed["model"]["max"], [30.0, 20.0, -1.0]), "{placed}");
    assert_eq!((placed["clearance_z"].as_f64(), placed["retract_z"].as_f64()), (Some(25.0), Some(5.0)));
    assert_eq!(s.undo.len(), undo, "measuring changes nothing");
    // A second body widens the box; a setup that lists one body measures only that one.
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 30, "corner": [100, 0, 0], "operation": "new", "body_name": "Post"}));
    let both = run(&mut s, "cam.setup.measure", json!({"id": "setup-1"}));
    assert!(near(&both["model_box"]["max"], [110.0, 40.0, 30.0]), "{both}");
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "model_bodies": ["Post"]}));
    let post = run(&mut s, "cam.setup.measure", json!({"id": "setup-1"}));
    assert_eq!(post["bodies"], json!(["Post"]));
    assert!(near(&post["model_box"]["min"], [100.0, 0.0, 0.0]), "{post}");
    // A body the design no longer has, a missing setup and a turned WCS are refused by name.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "model_bodies": ["Gone"]}));
    assert!(refused(&mut s, "cam.setup.measure", json!({"id": "setup-1"})).contains("'Gone' is not in the design"));
    assert!(refused(&mut s, "cam.setup.measure", json!({"id": "setup-9"})).contains("no setup 'setup-9'"));
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "model_bodies": [], "wcs": {"origin": {"kind": "stock_box", "anchor": {"x": "center", "y": "center", "z": "max"}}, "orientation": {"kind": "explicit", "z": [0, 1, 0], "x": [1, 0, 0]}}}),
    );
    assert!(refused(&mut s, "cam.setup.measure", json!({"id": "setup-1"})).contains("not supported yet"));
}

#[test]
fn a_setup_with_nothing_to_machine_says_so() {
    let mut s = Session::default();
    run(&mut s, "cam.setup.create", json!({}));
    assert!(refused(&mut s, "cam.setup.measure", json!({"id": "setup-1"})).contains("no bodies to machine"));
}

/// Tool 3: a 10 mm, 2-flute, 118° drill with data for aluminium (100 m/min, 0.1 mm a tooth): it
/// turns 100 000 / (10 π) = 3183.1 rev/min and feeds 0.2 mm a revolution, 636.6 mm/min.
fn drill10() -> Value {
    json!({
        "number": 3, "name": "10 mm drill", "kind": "drill",
        "geometry": {"diameter": 10.0, "corner_radius": 0.0, "flute_length": 40.0, "shoulder_length": 0.0, "overall_length": 90.0,
                     "shank_diameter": 10.0, "tip_angle": 118f64.to_radians(), "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 2, "holder": {"stick_out": 50.0, "segments": []}, "length_offset": 3, "diameter_offset": 3, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": 100.0, "chip_load": 0.1, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

/// A 60 x 40 x 20 block in the vise setup (stock top at Z 0, clearance 25, retract 5), aluminium.
fn drilling() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0], "body_name": "Block"}));
    run(&mut s, "cam.tool.create", json!({"tool": drill10()}));
    run(&mut s, "cam.setup.create", json!({"material": "aluminium"}));
    s
}

fn three_holes() -> Value {
    json!({"holes": [[-20, -10], [20, -10], [20, 10]], "depth": 10, "cycle": {"type": "peck", "peck": 3}})
}

#[test]
fn a_drill_operation_works_out_its_toolpath_from_the_setup_and_the_tool() {
    let mut s = drilling();
    assert_eq!(run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}))["id"], "op-1");
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let rpm = 100_000.0 / (10.0 * std::f64::consts::PI);
    let n = &t["numbers"];
    assert!((n["rpm"].as_f64().unwrap() - rpm).abs() < 1e-9 && (n["feed"].as_f64().unwrap() - 0.2 * rpm).abs() < 1e-9, "{n}");
    assert_eq!((n["bottom"].as_f64(), n["r"].as_f64(), n["clearance"].as_f64()), (Some(-10.0), Some(5.0), Some(25.0)));
    // Each hole: 15 mm from R to the bottom in five pecks, each after the first re-entering 0.5 mm
    // above the last depth (the generic control): 17 mm cut a hole.
    assert!((t["feed_length"].as_f64().unwrap() - 51.0).abs() < 1e-9, "{t}");
    // Rapids, worked out peck by peck: 93 for the first hole (20 down to R), 113 for the second
    // (40 across), 93 for the third (20 across), 20 back up to clearance.
    assert!((t["rapid_length"].as_f64().unwrap() - 319.0).abs() < 1e-9, "{}", t["rapid_length"]);
    assert!(near(&t["min"], [-20.0, -10.0, -10.0]) && near(&t["max"], [20.0, 10.0, 25.0]), "{t}");
    let holes: Vec<&Value> = t["toolpath"]["moves"].as_array().unwrap().iter().filter(|m| m["kind"] == "drill").collect();
    assert_eq!(holes.len(), 3);
    // Posted for the Haas: one G83 cycle back to R between holes, the drill's speed and coolant.
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = haas["text"].as_str().unwrap();
    let expected =
        "\nS3183 M3\nM8\nG43 H3 G0 X-20. Y-10. Z25.\nG99 G83 X-20. Y-10. Z-10. R5. Q3. F636.6\nX20. Y-10.\nX20. Y10.\nG80\nG0 Z25.\nM9\nM5\n";
    assert!(text.contains(expected), "{text}");
    // The design keeps the parameters, never the toolpath.
    let data = run(&mut s, "cam.data", json!({}));
    let op = &data["data"]["setups"][0]["operations"][0];
    assert_eq!(op["operation"]["kind"], "drill");
    assert!(op["operation"].get("toolpath").is_none() && op["operation"]["drill"]["holes"].as_array().unwrap().len() == 3, "{op}");
}

#[test]
fn a_drill_operation_follows_its_setup_and_says_what_it_needs() {
    let mut s = drilling();
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    // 4 mm more stock on top: the WCS origin rises with the stock top (21 to 25 on the model), and
    // the holes, measured from that top, keep their numbers in the WCS: bottom -10, R 5.
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "stock": {"kind": "relative_box", "offsets": {"x_min": 1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 5}}}),
    );
    assert!(near(&run(&mut s, "cam.setup.measure", json!({"id": "setup-1"}))["placement"]["origin"], [30.0, 20.0, 25.0]));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!((t["numbers"]["bottom"].as_f64(), t["numbers"]["r"].as_f64()), (Some(-10.0), Some(5.0)));
    // No material: the error names the two ways out; giving one back fixes it.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": ""}));
    let e = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(e.contains("stock material") && e.contains("rpm and feed"), "{e}");
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"})).contains("stock material"));
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": "aluminium"}));
    run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    // A tool that can't drill, a tool the design lacks, bad holes: refused, nothing added.
    run(&mut s, "cam.tool.create", json!({"tool": tool(1)}));
    let mut ball = tool(2);
    ball["kind"] = json!("ball_end_mill");
    ball["geometry"]["corner_radius"] = json!(3.0);
    run(&mut s, "cam.tool.create", json!({"tool": ball}));
    let undo = s.undo.len();
    assert!(refused(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 2, "drill": three_holes()})).contains("does not drill"));
    assert!(refused(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 9, "drill": three_holes()})).contains("tool 9"));
    assert!(
        refused(
            &mut s,
            "cam.op.drill",
            json!({"setup": "setup-1", "tool": 3, "drill": {"holes": [], "depth": 5, "cycle": {"type": "drill", "dwell": 0}}})
        )
        .contains("holes")
    );
    assert!(refused(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": {"holes": [[0, 0]], "depth": 5}})).contains("cycle"));
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-9"})).contains("no operation 'op-9'"));
    assert_eq!(s.undo.len(), undo);
    // A plunging flat end mill is fine.
    run(
        &mut s,
        "cam.op.drill",
        json!({"setup": "setup-1", "tool": 1, "drill": {"holes": [[0, 0]], "depth": 3, "cycle": {"type": "drill", "dwell": 0}, "rpm": 3000, "feed": 100}}),
    );
}

#[test]
fn a_hand_made_toolpath_and_a_drill_operation_post_as_one_program() {
    let mut s = shop();
    run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0], "body_name": "Block"}));
    run(&mut s, "cam.tool.create", json!({"tool": drill10()}));
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": "aluminium"}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    for post in ["rs274", "fanuc_mill", "haas_mill"] {
        let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": post}));
        let text = posted["text"].as_str().unwrap();
        assert!(text.contains("T1 M6") && text.contains("T3 M6") && text.contains("G83"), "{post}:\n{text}");
        assert!(text.find("T1 M6") < text.find("T3 M6"), "{post}");
    }
}

/// Tool 4: a 20 mm, 4-flute flat end mill with data for aluminium (200 m/min, 0.05 mm a tooth,
/// plunging at half): 200 000 / (20 π) = 3183.1 rev/min, 636.6 mm/min, 318.3 down.
fn mill20() -> Value {
    json!({
        "number": 4, "name": "20 mm flat end mill", "kind": "flat_end_mill",
        "geometry": {"diameter": 20.0, "corner_radius": 0.0, "flute_length": 30.0, "shoulder_length": 0.0, "overall_length": 100.0,
                     "shank_diameter": 20.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 4, "holder": {"stick_out": 50.0, "segments": []}, "length_offset": 4, "diameter_offset": 4, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": 200.0, "chip_load": 0.05, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

#[test]
fn a_facing_operation_takes_the_stock_top_down_to_the_part() {
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    assert_eq!(run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}))["id"], "op-1");
    // The stock is X -31 to 31, Y -21 to 21 in the WCS with 1 mm over the part. One level at Z -1;
    // stepover 14 (70% of 20) puts passes at Y 17, 5.667, -5.667, -17, each from X -43 to 43.
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let rpm = 200_000.0 / (20.0 * std::f64::consts::PI);
    let n = &t["numbers"];
    assert!((n["rpm"].as_f64().unwrap() - rpm).abs() < 1e-9 && (n["feed"].as_f64().unwrap() - 0.2 * rpm).abs() < 1e-9, "{n}");
    assert!((n["plunge_feed"].as_f64().unwrap() - 0.1 * rpm).abs() < 1e-9, "{n}");
    assert_eq!((n["levels"].clone(), n["passes"].as_array().unwrap().len(), n["stepover"].as_f64()), (json!([-1.0]), 4, Some(14.0)));
    // Cut: a 6 mm plunge, four passes of 86, three steps of 34/3. Rapids: 20 down, 6 and 20 up.
    assert!((t["feed_length"].as_f64().unwrap() - 384.0).abs() < 1e-9 && (t["rapid_length"].as_f64().unwrap() - 46.0).abs() < 1e-9, "{t}");
    assert!(near(&t["min"], [-43.0, -17.0, -1.0]) && near(&t["max"], [43.0, 17.0, 25.0]), "{t}");
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = haas["text"].as_str().unwrap();
    let expected = "\nT4 M6\nS3183 M3\nM8\nG43 H4 G0 X-43. Y17. Z25.\nZ5.\nG1 Z-1. F318.3\nX43. F636.6\nY5.667\nX-43.\nY-5.667\nX43.\nY-17.\nX-43.\nG0 Z5.\nZ25.\nM9\nM5\n";
    assert!(text.contains(expected), "{text}");
    // Changed like any parameter: one way, in two levels to 2 mm below the stock top.
    run(
        &mut s,
        "cam.op.face",
        json!({"setup": "setup-1", "tool": 4, "name": "Face deeper", "face": {"bottom": -2, "stepdown": 1, "pattern": "one_way"}}),
    );
    let deeper = run(&mut s, "cam.op.toolpath", json!({"id": "op-2"}));
    assert_eq!(deeper["numbers"]["levels"], json!([-1.0, -2.0]));
    // A tool that can't face is refused when the operation is added; no stock on top when its toolpath is asked for.
    assert!(refused(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 3})).contains("does not face"));
    assert!(refused(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4, "face": {"angle": 45}})).contains("face"));
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "stock": {"kind": "relative_box", "offsets": {"x_min": 1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 0}}}),
    );
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"})).contains("nothing to face"));
}

#[test]
fn every_change_is_one_undo_step() {
    let mut s = Session::default();
    run(&mut s, "cam.setup.create", json!({"name": "Op 1"}));
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "work_offset": 2}));
    assert_eq!(run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["work_offset"], 2);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["work_offset"], 1);
    s.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(run(&mut s, "cam.data", json!({}))["setups"], 0);
    s.execute("edit.redo", &json!({})).unwrap();
    assert_eq!(run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["name"], "Op 1");
}

#[test]
fn refusals_name_the_problem_and_change_nothing() {
    let mut s = shop();
    let before = run(&mut s, "cam.data", json!({}));
    let undo = s.undo.len();
    let cases = [
        ("cam.tool.create", json!({"tool": tool(1)}), "already has a tool 1"),
        ("cam.tool.create", json!({"tool": {"number": 2}}), "tool:"),
        ("cam.tool.delete", json!({"number": 1}), "uses tool 1"),
        ("cam.tool.delete", json!({"number": 9}), "no tool 9"),
        ("cam.tool.edit", json!({"number": 1, "tool": tool(5)}), "not one of the design's tools"),
        ("cam.op.manual", json!({"setup": "setup-1", "tool": 9, "toolpath": bracket()}), "not one of the design's tools"),
        ("cam.op.manual", json!({"setup": "nope", "tool": 1, "toolpath": bracket()}), "no setup 'nope'"),
        (
            "cam.op.manual",
            json!({"setup": "setup-1", "tool": 1, "toolpath": {"start": p(0.0, 0.0, 0.0), "moves": [{"kind": "linear", "to": p(1.0, 0.0, 0.0), "feed": -5.0}]}}),
            "feed",
        ),
        ("cam.op.delete", json!({"id": "op-9"}), "no operation 'op-9'"),
        ("cam.setup.create", json!({"work_offset": 7}), "work offset"),
        ("cam.setup.create", json!({"stock": {"kind": "from_setup", "setup": "setup-9"}}), "not an earlier setup"),
        ("cam.setup.create", json!({"stock": {"kind": "teleport"}}), "stock:"),
        ("cam.setup.edit", json!({"id": "setup-1", "safe_heights": {"clearance": 1.0, "retract": 5.0}}), "clearance at or above retract"),
        ("cam.setup.edit", json!({"id": "setup-9", "name": "x"}), "no setup 'setup-9'"),
        ("cam.setup.delete", json!({"id": "setup-9"}), "no setup 'setup-9'"),
        ("cam.post", json!({"setup": "setup-1", "post": "nope"}), "no post 'nope'"),
        ("cam.post", json!({"setup": "setup-1", "post": "rs274", "units": "furlongs"}), "mm or inch"),
        ("cam.post", json!({"setup": "setup-1", "post": "fanuc_mill", "program_number": 123456}), "digits"),
        ("cam.data", json!({"extra": 1}), "parameters"),
    ];
    for (id, params, expect) in cases {
        let e = refused(&mut s, id, params.clone());
        assert!(e.contains(expect), "{id} {params}: {e:?} does not mention {expect:?}");
    }
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
    assert_eq!(s.undo.len(), undo, "a refused command leaves no undo step");
}

#[test]
fn a_later_setup_holds_on_to_the_one_its_stock_comes_from() {
    let mut s = shop();
    run(&mut s, "cam.setup.create", json!({"name": "Op 2 (flipped)", "stock": {"kind": "from_setup", "setup": "setup-1"}}));
    assert!(refused(&mut s, "cam.setup.delete", json!({"id": "setup-1"})).contains("takes its stock from 'setup-1'"));
    run(&mut s, "cam.setup.delete", json!({"id": "setup-2"}));
    run(&mut s, "cam.setup.delete", json!({"id": "setup-1"}));
    assert_eq!(run(&mut s, "cam.data", json!({}))["setups"], 0);
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"})).contains("no setup 'setup-1'"));
}

#[test]
fn operations_post_in_order_and_must_continue_where_the_last_ended() {
    let mut s = shop();
    run(&mut s, "cam.setup.create", json!({}));
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-2", "post": "rs274"})).contains("no operations to post"));
    // A second operation that starts where the first ended is posted after it.
    let more = json!({"start": p(0.0, 10.0, 5.0), "moves": [{"kind": "spindle", "rpm": 1200.0, "direction": "clockwise"}, {"kind": "rapid", "to": p(0.0, 0.0, 5.0)}, {"kind": "spindle_stop"}]});
    assert_eq!(run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": more}))["id"], "op-2");
    assert_eq!(run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"}))["moves"], 10);
    // One that starts somewhere else would jump without a move: refused, naming the gap.
    let away = json!({"start": p(50.0, 50.0, 50.0), "moves": [{"kind": "rapid", "to": p(0.0, 0.0, 50.0)}]});
    run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 1, "toolpath": away}));
    let e = refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"}));
    assert!(e.contains("op-3 starts") && e.contains("mm away"), "{e}");
}

#[test]
fn damaged_manufacture_data_is_kept_and_never_overwritten() {
    let mut s = Session::default();
    let damaged: &[u8] = b"{ this is not the data";
    s.doc_mut().plugin_data.insert("solidcraft-cam/data.json".into(), damaged.into());
    assert!(refused(&mut s, "cam.data", json!({})).contains("can't be read"));
    assert!(refused(&mut s, "cam.setup.create", json!({})).contains("kept as it is"));
    assert_eq!(s.doc.plugin_data.get("solidcraft-cam/data.json").map(|b| b.as_ref().to_vec()), Some(damaged.to_vec()));
}
