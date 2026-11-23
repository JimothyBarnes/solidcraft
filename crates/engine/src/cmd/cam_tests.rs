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
        text.contains("\nN90 G0 X10. Y0.\nN100 G43 H1 Z5.\n") && text.contains(" G3 X0. Y10. I-10. J0. F400.\n") && text.ends_with("M30\n%\n"),
        "{text}"
    );
    assert_eq!((fanuc["moves"].as_u64(), fanuc["machine_retracts"].as_u64()), (Some(7), Some(2)));
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill", "program_number": 42, "units": "inch"}));
    let text = haas["text"].as_str().unwrap();
    assert!(text.starts_with("%\nO00042 (SETUP 1)\n") && text.contains("G20") && text.contains("\nG0 X0.3937 Y0.\nG43 H1 Z0.1969\n"), "{text}");
    assert!(text.ends_with("\nG28 G91 Z0.\nG28 Y0.\nG90\nM30\n%\n"), "the table comes home after Z: {text}");
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
    // 10 deep from the part's top (Z -1, under 1 mm of stock): bottom -11. R is the setup's retract
    // above the stock top: 5.
    assert_eq!((n["bottom"].as_f64(), n["r"].as_f64(), n["clearance"].as_f64()), (Some(-11.0), Some(5.0), Some(25.0)));
    // Each hole: 16 mm from R to the bottom in six pecks (to 2, -1, -4, -7, -10, -11), each after
    // the first re-entering 0.5 mm above the last depth (the generic control): 18.5 mm cut a hole.
    assert!((t["feed_length"].as_f64().unwrap() - 55.5).abs() < 1e-9, "{t}");
    // Rapids, worked out peck by peck: out and back in 87.5 a hole and 16 up at its end. The first
    // hole adds 20 down to R, the second 40 across, the third 20 across; then 20 up to clearance.
    assert!((t["rapid_length"].as_f64().unwrap() - 410.5).abs() < 1e-9, "{}", t["rapid_length"]);
    assert!(near(&t["min"], [-20.0, -10.0, -11.0]) && near(&t["max"], [20.0, 10.0, 25.0]), "{t}");
    let holes: Vec<&Value> = t["toolpath"]["moves"].as_array().unwrap().iter().filter(|m| m["kind"] == "drill").collect();
    assert_eq!(holes.len(), 3);
    // Posted for the Haas: one G83 cycle back to R between holes, the drill's speed and coolant.
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = haas["text"].as_str().unwrap();
    let expected =
        "\nS3183 M3\nM8\nG0 X-20. Y-10.\nG43 H3 Z25.\nG99 G83 X-20. Y-10. Z-11. R5. Q3. F636.6\nX20. Y-10.\nX20. Y10.\nG80\nG0 Z25.\nM9\nM5\n";
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
    let bottom = |s: &mut Session| run(s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["bottom"].as_f64().unwrap();
    assert_eq!(bottom(&mut s), -11.0);
    // 4 mm more stock on top: the WCS origin rises with the stock top (21 to 25 on the model). The
    // holes stay where they are on the part, 10 below its top (Z 10 on the model), so their bottom
    // is now -15 in the WCS; R stays 5 above the stock.
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "stock": {"kind": "relative_box", "offsets": {"x_min": 1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 5}}}),
    );
    assert!(near(&run(&mut s, "cam.setup.measure", json!({"id": "setup-1"}))["placement"]["origin"], [30.0, 20.0, 25.0]));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!((t["numbers"]["bottom"].as_f64(), t["numbers"]["r"].as_f64()), (Some(-15.0), Some(5.0)));
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
fn a_faced_and_drilled_part_posts_as_one_program() {
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    // Facing takes the stock down to the part's top (Z -1); the holes are 10 deep from there (Z -11).
    // The drill starts where facing ended, changes tool with the spindle stopped, and comes in
    // from the clearance height.
    let haas = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = haas["text"].as_str().unwrap();
    let join = "\nX-43.\nG0 Z5.\nZ25.\nM9\nM5\nG28 G91 Z0.\nG90\nT3 M6\nS3183 M3\nM8\nG0 X-20. Y-10.\nG43 H3 Z25.\nG99 G83 X-20. Y-10. Z-11. R5. Q3. F636.6\n";
    assert!(text.contains(join), "{text}");
    assert!(haas["notes"].as_array().unwrap().is_empty(), "{haas}");
    for post in ["rs274", "fanuc_mill"] {
        let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": post, "units": "inch"}));
        let text = posted["text"].as_str().unwrap();
        assert!(text.find("T4 M6") < text.find("T3 M6") && text.contains("G83"), "{post}:\n{text}");
    }
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
    let expected = "\nT4 M6\nS3183 M3\nM8\nG0 X-43. Y17.\nG43 H4 Z25.\nZ5.\nG1 Z-1. F318.3\nX43. F636.6\nY5.667\nX-43.\nY-5.667\nX43.\nY-17.\nX-43.\nG0 Z5.\nZ25.\nM9\nM5\n";
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
fn a_view_of_a_setup_has_its_placement_and_its_operations_as_drawn_pieces() {
    use solidcraft_cam::Drawn;
    let mut s = drilling();
    // Nothing to draw before there is a setup. A setup that is gone falls back to the first.
    assert!(crate::cmd::manufacture_view(&Session::default(), None).unwrap().is_none());
    assert!(crate::cmd::manufacture_view(&Session::default(), Some("setup-1")).unwrap().is_none());
    assert_eq!(crate::cmd::manufacture_view(&s, Some("setup-9")).unwrap().unwrap().id, "setup-1");
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    // An operation that can't be worked out says why and the rest are still drawn.
    run(
        &mut s,
        "cam.op.drill",
        json!({"setup": "setup-1", "tool": 3, "name": "Too deep", "drill": {"holes": [[0, 0]], "depth": 39.9, "depth_to": "full_diameter", "cycle": {"type": "drill", "dwell": 0}}}),
    );
    let undo = s.undo.len();
    let view = crate::cmd::manufacture_view(&s, None).unwrap().unwrap();
    assert_eq!(s.undo.len(), undo, "a view changes nothing");
    assert_eq!((view.id.as_str(), view.operations.len()), ("setup-1", 3));
    assert_eq!((view.placement.origin.x, view.placement.origin.y, view.placement.origin.z), (30.0, 20.0, 21.0));
    // Facing: 2 rapids in (the first goes nowhere and is left out: 1), a plunge, 4 passes and 3
    // steps, 2 rapids out.
    let face = view.operations[0].pieces.as_ref().unwrap();
    let count = |pieces: &[(Drawn, solidcore_record::Vec3, solidcore_record::Vec3)], kind: Drawn| pieces.iter().filter(|x| x.0 == kind).count();
    assert_eq!((count(face, Drawn::Rapid), count(face, Drawn::Plunge), count(face, Drawn::Cut)), (3, 1, 7));
    // Drilling starts where facing ended (X -43, Y -17 at the clearance height) and its pecks cut
    // straight down: six a hole.
    let drill = view.operations[1].pieces.as_ref().unwrap();
    assert_eq!((drill[0].1.x, drill[0].1.y, drill[0].1.z), (-43.0, -17.0, 25.0));
    assert_eq!((count(drill, Drawn::Plunge), count(drill, Drawn::Cut)), (18, 0));
    assert!(view.operations[2].pieces.as_ref().unwrap_err().contains("flutes"), "{:?}", view.operations[2].pieces.as_ref().err());
    // A setup that can't be placed says why, in plain words.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "model_bodies": ["Gone"]}));
    let why = crate::cmd::manufacture_view(&s, Some("setup-1")).err().unwrap();
    assert!(why.contains("'Gone' is not in the design") && !why.contains("cam.view"), "{why}");
}

#[test]
fn a_program_is_posted_to_a_file_and_never_over_one_by_surprise() {
    let mut s = shop();
    let dir = std::env::temp_dir().join(format!("solidcraft-cam-post-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("bracket.nc").to_string_lossy().to_string();
    let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill", "path": file}));
    assert_eq!(posted["path"], json!(file));
    // The file holds exactly the program returned.
    assert_eq!(std::fs::read_to_string(&file).unwrap(), posted["text"].as_str().unwrap());
    // A second post to the same file is refused unless it is asked to replace it.
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274", "path": file})).contains("could not write the program"));
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("%\nO00001"), "the Haas program is still there");
    run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274", "path": file, "overwrite": true}));
    assert!(std::fs::read_to_string(&file).unwrap().starts_with("(Setup 1)"));
    // Without a path nothing is written and none is reported.
    assert_eq!(run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274"}))["path"], json!(null));
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "rs274", "path": ""})).contains("invalid file path"));
    // A program that can't be posted writes nothing.
    let missing = dir.join("none.nc").to_string_lossy().to_string();
    assert!(refused(&mut s, "cam.post", json!({"setup": "setup-9", "post": "rs274", "path": missing})).contains("no setup"));
    assert!(!std::path::Path::new(&missing).exists());
}

#[test]
fn forms_are_offered_the_posts_and_the_tools_that_can_do_the_work() {
    let posts = crate::cmd::manufacture_posts();
    let ids: Vec<&str> = posts.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ids, ["fanuc_mill", "haas_mill", "rs274"]);
    assert!(posts.iter().all(|(_, name)| !name.is_empty()));
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    let numbers = |s: &Session, operation: &str| crate::cmd::manufacture_tools(&s.doc, operation).iter().map(|t| t.number).collect::<Vec<_>>();
    // Tool 3 is a drill, tool 4 a flat end mill: the mill faces, both drill (a flat end mill plunges).
    assert_eq!((numbers(&s, "face"), numbers(&s, "drill"), numbers(&s, "any")), (vec![4], vec![3, 4], vec![3, 4]));
    assert_eq!(crate::cmd::manufacture_tools(&s.doc, "face")[0].diameter, 20.0);
    assert!(crate::cmd::manufacture_tools(&Session::default().doc, "face").is_empty());
}

#[test]
fn a_form_previews_an_operation_before_it_is_made() {
    use solidcraft_cam::Drawn;
    use std::f64::consts::PI;
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    let before = run(&mut s, "cam.data", json!({}));
    let preview = |s: &Session, command: &str, params: Value| crate::cmd::manufacture_preview(s, command, &params);
    let count = |pieces: &[(Drawn, solidcore_record::Vec3, solidcore_record::Vec3)], kind: Drawn| pieces.iter().filter(|x| x.0 == kind).count();
    // The facing `cam.op.face` would make (see the facing test): 200 m/min on a 20 mm tool, 0.05 mm
    // a tooth on four teeth; three rapids, a plunge, four passes and three steps. It cuts a 6 mm
    // plunge at half the feed and 378 mm at the feed: 1950 / rpm minutes, which is 0.195 pi.
    let face = preview(&s, "cam.op.face", json!({"setup": "setup-1", "tool": 4})).unwrap();
    let rpm = 200_000.0 / (20.0 * PI);
    assert!((face.rpm - rpm).abs() < 1e-9 && (face.feed - 0.2 * rpm).abs() < 1e-9, "{} rpm, {} mm/min", face.rpm, face.feed);
    assert_eq!((count(&face.pieces, Drawn::Rapid), count(&face.pieces, Drawn::Plunge), count(&face.pieces, Drawn::Cut)), (3, 1, 7));
    assert!((face.cut_minutes - 0.195 * PI).abs() < 1e-9, "{}", face.cut_minutes);
    assert_eq!((face.placement.origin.x, face.placement.origin.y, face.placement.origin.z), (30.0, 20.0, 21.0));
    assert!(face.notes.is_empty(), "{:?}", face.notes);
    // A speed and feed typed in are the ones used. At half the stepover the first pass is at Y 24
    // (the stock's edge at 21, the tool's radius 10 out, 7 in) and the last at -24: 48 mm in gaps
    // of at most 7 is eight passes and seven steps.
    let typed = preview(&s, "cam.op.face", json!({"setup": "setup-1", "tool": 4, "face": {"rpm": 5000, "feed": 1200, "stepover": 7}})).unwrap();
    assert_eq!((typed.rpm, typed.feed), (5000.0, 1200.0));
    assert_eq!(count(&typed.pieces, Drawn::Cut), 8 + 7);
    // A drill: 100 m/min on 10 mm, three holes of six pecks each.
    let drill = preview(&s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()})).unwrap();
    assert!((drill.rpm - 100_000.0 / (10.0 * PI)).abs() < 1e-9);
    assert_eq!((count(&drill.pieces, Drawn::Plunge), count(&drill.pieces, Drawn::Cut)), (18, 0));
    // What would stop the operation is said in plain words, with no operation or command named.
    let why = |s: &Session, command: &str, params: Value| preview(s, command, params).err().unwrap_or_else(|| panic!("{command} was previewed"));
    for (command, params, expect) in [
        ("cam.op.face", json!({"setup": "setup-9", "tool": 4}), "the design has no setup 'setup-9'"),
        ("cam.op.face", json!({"setup": "setup-1", "tool": 9}), "the design has no tool 9"),
        ("cam.op.face", json!({"setup": "setup-1", "tool": 3}), "does not face"),
        ("cam.op.face", json!({"setup": "setup-1", "tool": 4, "face": {"stepover": 0}}), "the stepover must be at least"),
        ("cam.op.face", json!({"setup": "setup-1", "tool": 4, "face": {"rpm": 1e300}}), "rpm must be above 0"),
        ("cam.op.face", json!({"setup": 5}), "parameters:"),
        ("cam.op.face", json!(null), "parameters:"),
        ("cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": {"holes": []}}), "drill:"),
        (
            "cam.post",
            json!({"setup": "setup-1", "post": "rs274"}),
            "not a command that adds a drill, face, contour, bore, pocket or chamfer operation",
        ),
    ] {
        let said = why(&s, command, params.clone());
        assert!(said.contains(expect), "{command} {params}: {said:?} does not say {expect:?}");
        assert!(!said.starts_with("operation") && !said.contains("cam.preview"), "{said}");
    }
    // Without a stock material the tool's data can't be used: the two ways out are named, and a
    // speed and feed typed in is one of them.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": ""}));
    let said = why(&s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    assert!(said.contains("stock material") && said.contains("rpm and feed") && !said.starts_with("operation"), "{said}");
    assert!(preview(&s, "cam.op.face", json!({"setup": "setup-1", "tool": 4, "face": {"rpm": 5000, "feed": 1200}})).is_ok());
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": "aluminium"}));
    // A preview adds nothing to the design.
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
    // One of the design's own operations still says which it is.
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": ""}));
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"})).contains("operation op-1 (Face 1): "));
}

#[test]
fn a_form_previews_a_setup_before_it_is_made() {
    let mut s = Session::default();
    let preview = |s: &Session, params: Value| crate::cmd::manufacture_setup_preview(s, "cam.setup.create", &params);
    // Nothing to machine yet: the reason, as the setup's own commands give it.
    assert!(preview(&s, json!({})).unwrap_err().contains("has no bodies to machine"));
    run(&mut s, "solid.box", json!({"length": 60, "width": 40, "height": 20, "corner": [0, 0, 0], "body_name": "Block"}));
    let before = run(&mut s, "cam.data", json!({}));
    // The vise setup on the 60 x 40 x 20 block: 1 mm over on the sides and top, origin at the top
    // centre of the stock.
    let vise = preview(&s, json!({})).unwrap();
    assert_eq!((vise.origin.x, vise.origin.y, vise.origin.z), (30.0, 20.0, 21.0));
    assert_eq!((vise.stock.min.x, vise.stock.min.y, vise.stock.min.z), (-31.0, -21.0, -21.0));
    assert_eq!((vise.stock.max.x, vise.stock.max.y, vise.stock.max.z), (31.0, 21.0, 0.0));
    // No parameters at all are the defaults too, as for the command.
    assert_eq!(preview(&s, json!(null)).unwrap(), vise);
    // 5 mm on the sides, 2 on top, 3 underneath, origin at the front left corner of the top: the
    // stock runs from the origin 70 along X, 50 along Y and 25 down.
    let corner = preview(
        &s,
        json!({
            "stock": {"kind": "relative_box", "offsets": {"x_min": 5, "x_max": 5, "y_min": 5, "y_max": 5, "z_min": 3, "z_max": 2}},
            "wcs": {"origin": {"kind": "stock_box", "anchor": {"x": "min", "y": "min", "z": "max"}}, "orientation": {"kind": "model"}},
            "work_offset": 2
        }),
    )
    .unwrap();
    assert_eq!((corner.origin.x, corner.origin.y, corner.origin.z), (-5.0, -5.0, 22.0));
    assert_eq!((corner.stock.min.x, corner.stock.min.y, corner.stock.min.z), (0.0, 0.0, -25.0));
    assert_eq!((corner.stock.max.x, corner.stock.max.y, corner.stock.max.z), (70.0, 50.0, 0.0));
    // What the command would refuse, the preview refuses in the same words, without its name.
    for (params, expect) in [
        (json!({"work_offset": 9}), "work offset"),
        (json!({"stock": {"kind": "relative_box", "offsets": {"x_min": -1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 1}}}), "stock"),
        (json!({"stock": {"kind": "round"}}), "stock:"),
        (json!({"name": 5}), "parameters:"),
        (json!([1, 2]), "parameters:"),
    ] {
        let said = preview(&s, params.clone()).err().unwrap_or_else(|| panic!("{params} was previewed"));
        assert!(said.contains(expect) && !said.contains("cam.preview"), "{params}: {said:?} does not say {expect:?}");
        let command = refused(&mut s, "cam.setup.create", params.clone());
        assert!(command.contains(&said), "{params}: the command says {command:?}, the preview {said:?}");
    }
    // A setup that names a body the design lacks can be made (the body may come back), but there
    // is nowhere to draw it: the preview says so.
    assert!(preview(&s, json!({"model_bodies": ["Gone"]})).unwrap_err().contains("'Gone' is not in the design"));
    // A preview adds nothing to the design.
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
}

#[test]
fn commands_that_measure_the_model_bring_it_up_to_date_first() {
    // A setup is placed on the bodies' exact boxes, so measuring it, working out a toolpath and
    // posting read the evaluated model: the engine finishes a stale model (one left behind by a
    // live drag) before they run. The others read and write only the design's Manufacture data.
    for spec in super::COMMANDS {
        let measures = matches!(
            spec.id,
            "cam.setup.measure"
                | "cam.setup.holes"
                | "cam.setup.openings"
                | "cam.setup.outline"
                | "cam.setup.pockets"
                | "cam.setup.sheet"
                | "cam.op.toolpath"
                | "cam.simulate"
                | "cam.post"
        );
        assert_eq!(spec.reads_model, measures, "{}", spec.id);
    }
}

/// The block of `drilling()` (60 x 40 x 20, its top at Z -1 in the WCS under 1 mm of stock, the
/// origin over its middle) with holes cut in it: four of 6 through it, two of 8 that are 10 deep
/// with flat floors, and a counterbore of 12, 4 deep, on the first of the four. And four things
/// that are not holes from above: a quarter round down one corner, a hole of 5 from below that
/// stops half way up, a hole of 4 along X, and a slot's rounded end is left for the offsetter.
fn holed() -> Session {
    let mut s = drilling();
    let mut cut = |diameter: f64, base: [f64; 3], height: f64, axis: &str| {
        run(
            &mut s,
            "solid.cylinder",
            json!({"diameter": diameter, "height": height, "base": base, "axis": axis, "operation": "cut", "targets": ["Block"]}),
        );
    };
    for (x, y) in [(10.0, 10.0), (50.0, 10.0), (50.0, 30.0), (10.0, 30.0)] {
        cut(6.0, [x, y, -5.0], 30.0, "Z");
    }
    for y in [10.0, 30.0] {
        cut(8.0, [30.0, y, 10.0], 15.0, "Z");
    }
    cut(12.0, [10.0, 10.0, 16.0], 10.0, "Z");
    cut(10.0, [60.0, 40.0, -5.0], 30.0, "Z");
    cut(5.0, [45.0, 20.0, -5.0], 15.0, "Z");
    cut(4.0, [-5.0, 20.0, 15.0], 70.0, "X");
    s
}

#[test]
fn a_setups_holes_are_found_on_the_model_and_grouped() {
    let mut s = holed();
    let before = run(&mut s, "cam.data", json!({}));
    let found = run(&mut s, "cam.setup.holes", json!({"id": "setup-1"}));
    // The most holes first; among the single ones the smaller first.
    //  - Three of the four through holes: the fourth is shorter under its counterbore.
    //  - The two flat-floored holes.
    //  - The fourth through hole, from the counterbore's floor (Z -5) down 16.
    //  - The counterbore: 4 deep, and what is under it is a floor (the step down to the hole of 6),
    //    though its axis runs out of the part.
    let expected = json!([
        {"diameter": 6.0, "top": -1.0, "depth": 20.0, "bottom": "through", "holes": [[20.0, -10.0], [-20.0, 10.0], [20.0, 10.0]]},
        {"diameter": 8.0, "top": -1.0, "depth": 10.0, "bottom": "flat", "holes": [[0.0, -10.0], [0.0, 10.0]]},
        {"diameter": 6.0, "top": -5.0, "depth": 16.0, "bottom": "through", "holes": [[-20.0, -10.0]]},
        {"diameter": 12.0, "top": -1.0, "depth": 4.0, "bottom": "flat", "holes": [[-20.0, -10.0]]},
    ]);
    assert_eq!(found["groups"], expected, "{found:#}");
    assert_eq!((found["id"].as_str(), found["holes"].as_u64()), (Some("setup-1"), Some(7)));
    // The form reads the same list, and finding holes changes nothing.
    let groups = crate::cmd::manufacture_holes(&s, "setup-1").unwrap();
    assert_eq!(serde_json::to_value(&groups).unwrap(), expected);
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
    // More stock on top moves the origin up with it: the holes keep their place on the part.
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "stock": {"kind": "relative_box", "offsets": {"x_min": 1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 5}}}),
    );
    let higher = run(&mut s, "cam.setup.holes", json!({"id": "setup-1"}));
    assert_eq!(
        (higher["groups"][0]["top"].as_f64(), higher["groups"][0]["depth"].as_f64(), higher["groups"][0]["holes"].clone()),
        (Some(-5.0), Some(20.0), expected[0]["holes"].clone())
    );
    // The origin at a corner of the stock moves every centre by half the stock: 31 and 21.
    run(
        &mut s,
        "cam.setup.edit",
        json!({"id": "setup-1", "wcs": {"origin": {"kind": "stock_box", "anchor": {"x": "min", "y": "min", "z": "max"}}, "orientation": {"kind": "model"}}}),
    );
    let corner = run(&mut s, "cam.setup.holes", json!({"id": "setup-1"}));
    assert_eq!(corner["groups"][0]["holes"], json!([[51.0, 11.0], [11.0, 31.0], [51.0, 31.0]]));
    // A boss is no hole, a setup with nothing to machine says so, and so does a missing setup.
    let mut boss = Session::default();
    run(&mut boss, "solid.cylinder", json!({"diameter": 20, "height": 10, "body_name": "Boss"}));
    run(&mut boss, "cam.setup.create", json!({}));
    assert_eq!(run(&mut boss, "cam.setup.holes", json!({"id": "setup-1"}))["groups"], json!([]));
    let mut empty = Session::default();
    run(&mut empty, "cam.setup.create", json!({}));
    assert!(refused(&mut empty, "cam.setup.holes", json!({"id": "setup-1"})).contains("has no bodies to machine"));
    assert!(refused(&mut s, "cam.setup.holes", json!({"id": "setup-9"})).contains("no setup 'setup-9'"));
    assert!(crate::cmd::manufacture_holes(&s, "setup-9").unwrap_err().contains("no setup 'setup-9'"));
}

#[test]
fn an_operation_is_edited_in_place_and_keeps_its_kind() {
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    let passes = |s: &mut Session| run(s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["passes"].as_array().unwrap().len();
    assert_eq!(passes(&mut s), 4);
    // Half the stepover on the facing: eight passes (see the preview's test), under its own name.
    let undo = s.undo.len();
    let edited = run(&mut s, "cam.op.edit", json!({"id": "op-1", "name": "Face fine", "face": {"stepover": 7}}));
    assert_eq!(edited, json!({"id": "op-1", "setup": "setup-1", "operations": 2}));
    assert_eq!((passes(&mut s), s.undo.len()), (8, undo + 1));
    let data = run(&mut s, "cam.data", json!({}))["data"].clone();
    let op = &data["setups"][0]["operations"][0];
    assert_eq!((op["name"].as_str(), op["tool"].as_u64(), op["operation"]["face"]["stepover"].as_f64()), (Some("Face fine"), Some(4), Some(7.0)));
    // Its parameters are replaced whole: what the new ones leave out goes back to its default.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "face": {"pattern": "one_way"}}));
    assert_eq!(passes(&mut s), 4);
    // Switched off, it is left out of the program; switched on again, it is back.
    let lines = |s: &mut Session| run(s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}))["lines"].as_u64().unwrap();
    let both = lines(&mut s);
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "suppressed": true}));
    assert!(lines(&mut s) < both);
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "suppressed": false}));
    assert_eq!(lines(&mut s), both);
    // The drill operation: deeper holes with another cycle.
    run(&mut s, "cam.op.edit", json!({"id": "op-2", "drill": {"holes": [[0, 0]], "depth": 12, "cycle": {"type": "drill", "dwell": 0}}}));
    assert_eq!(run(&mut s, "cam.op.toolpath", json!({"id": "op-2"}))["numbers"]["bottom"], json!(-13.0));
    // Refused, and nothing changed: another kind's parameters, a tool that can't do the work, a
    // tool or an operation the design lacks, junk.
    let before = run(&mut s, "cam.data", json!({}));
    let undo = s.undo.len();
    for (params, expect) in [
        (json!({"id": "op-1", "drill": three_holes()}), "operation op-1 is a facing: it takes `face`, not `drill`"),
        (json!({"id": "op-2", "face": {}}), "operation op-2 is a drill operation: it takes `drill`, not `face`"),
        (json!({"id": "op-2", "toolpath": bracket()}), "it takes `drill`, not `toolpath`"),
        (json!({"id": "op-1", "tool": 3}), "does not face"),
        (json!({"id": "op-1", "tool": 9}), "tool 9"),
        (json!({"id": "op-1", "face": {"stepover": -1}}), "stepover"),
        (json!({"id": "op-9", "name": "x"}), "no operation 'op-9'"),
        (json!({"id": "op-1", "depth": 5}), "parameters:"),
    ] {
        let said = refused(&mut s, "cam.op.edit", params.clone());
        assert!(said.contains(expect), "{params}: {said:?} does not say {expect:?}");
    }
    assert_eq!((run(&mut s, "cam.data", json!({})), s.undo.len()), (before, undo));
    // A hand-made toolpath is replaced the same way: here it retracts 3 mm higher. One that no
    // longer holds together (an arc whose start is off its circle) is refused.
    let mut hand = shop();
    let mut path = bracket();
    path["moves"][5]["to"] = p(0.0, 10.0, 8.0);
    run(&mut hand, "cam.op.edit", json!({"id": "op-1", "toolpath": path}));
    assert_eq!(run(&mut hand, "cam.op.toolpath", json!({"id": "op-1"}))["toolpath"]["moves"][5]["to"], p(0.0, 10.0, 8.0));
    let mut broken = bracket();
    broken["moves"][3]["to"] = p(12.0, 0.0, -1.0);
    assert!(refused(&mut hand, "cam.op.edit", json!({"id": "op-1", "toolpath": broken})).contains("operation op-1"));
}

#[test]
fn a_form_previews_a_changed_setup_before_it_is_changed() {
    let mut s = drilling();
    let before = run(&mut s, "cam.data", json!({}));
    let preview = |s: &Session, command: &str, params: Value| crate::cmd::manufacture_setup_preview(s, command, &params);
    // As it is: the vise setup on the 60 x 40 x 20 block.
    let same = preview(&s, "cam.setup.edit", json!({"id": "setup-1"})).unwrap();
    assert_eq!((same.origin.x, same.origin.y, same.origin.z), (30.0, 20.0, 21.0));
    // 5 mm on top instead of 1: the origin, at the stock's top, rises 4.
    let taller = preview(
        &s,
        "cam.setup.edit",
        json!({"id": "setup-1", "stock": {"kind": "relative_box", "offsets": {"x_min": 1, "x_max": 1, "y_min": 1, "y_max": 1, "z_min": 0, "z_max": 5}}}),
    )
    .unwrap();
    assert_eq!((taller.origin.z, taller.stock.min.z, taller.stock.max.z), (25.0, -25.0, 0.0));
    // What the command would refuse, and what can't be placed, in plain words.
    assert!(preview(&s, "cam.setup.edit", json!({"id": "setup-9"})).unwrap_err().contains("no setup 'setup-9'"));
    assert!(preview(&s, "cam.setup.edit", json!({"id": "setup-1", "work_offset": 9})).unwrap_err().contains("work offset"));
    assert!(preview(&s, "cam.setup.edit", json!({"id": "setup-1", "model_bodies": ["Gone"]})).unwrap_err().contains("'Gone' is not in the design"));
    assert!(preview(&s, "cam.setup.delete", json!({"id": "setup-1"})).unwrap_err().contains("not a command that makes or changes a setup"));
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
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

/// The block of `drilling()` with the 20 mm end mill (tool 4) in the design.
fn milling() -> Session {
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    s
}

fn about(v: &Value, want: f64) -> bool {
    v.as_f64().is_some_and(|got| (got - want).abs() <= 1e-9 * want.abs().max(1.0))
}

#[test]
fn a_contour_goes_round_the_outline_read_from_the_model() {
    use std::f64::consts::PI;
    let mut s = milling();
    // The block's outline at its bottom (Z -21 in the WCS, under 1 mm of stock): 60 x 40 about
    // the origin, four lines.
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert_eq!(
        (found["id"].as_str(), found["z"].as_f64(), found["loops"].as_array().map(Vec::len)),
        (Some("setup-1"), Some(-21.0), Some(1)),
        "{found:#}"
    );
    let l = &found["loops"][0];
    assert!(about(&l["length"], 200.0) && about(&l["area"], 2400.0), "{l:#}");
    assert_eq!((l["min"].clone(), l["max"].clone()), (json!([-30.0, -20.0]), json!([30.0, 20.0])));
    let segments = l["segments"].as_array().unwrap();
    assert_eq!(segments.len(), 4);
    assert!(segments.iter().all(|seg| seg["type"] == "line"), "{segments:?}");
    // The contour: three levels of 7 (21 mm in steps of at most half the 20 mm tool), the centre
    // 10 out, in from the front at Y -33 and on to the wall by an arc of 2 (see the strategy's
    // own test for every move).
    assert_eq!(run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}))["id"], "op-1");
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert_eq!(n["levels"], json!([-7.0, -14.0, -21.0]));
    assert_eq!((n["stepdown"].as_f64(), n["offset"].as_f64(), n["entries"].clone()), (Some(10.0), Some(10.0), json!([[2.0, -33.0]])));
    assert_eq!((n["leads"].clone(), n["tabs"].clone(), n["tab_top"].clone()), (json!([2.0]), json!([]), Value::Null));
    let rpm = 200_000.0 / (20.0 * PI);
    assert!(about(&n["rpm"], rpm) && about(&n["feed"], 0.2 * rpm) && about(&n["plunge_feed"], 0.1 * rpm), "{n}");
    assert!(about(&n["round"], 200.0 + 20.0 * PI), "{n}");
    assert!(about(&t["feed_length"], 57.0 + 3.0 * (201.0 + 22.0 * PI)) && about(&t["rapid_length"], 97.0 + 2.0 * 17.0_f64.sqrt()), "{t}");
    assert_eq!((t["min"].clone(), t["max"].clone()), (json!([-40.0, -33.0, -21.0]), json!([40.0, 30.0, 25.0])));
    // It posts with its arcs, and the program read back is the toolpath.
    let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    let text = posted["text"].as_str().unwrap();
    // Four clockwise corner arcs at the first level; the lower levels repeat them. The arcs on
    // to the wall and off it turn the other way: two a level.
    assert!(text.lines().filter(|line| line.contains("G2")).count() >= 4, "{text}");
    assert_eq!(text.lines().filter(|line| line.split_whitespace().any(|word| word == "G3")).count(), 6, "{text}");
    // A raised bottom cuts the block half way up: the same outline, read from a section.
    let half = run(&mut s, "cam.setup.outline", json!({"id": "setup-1", "z": -11}));
    assert_eq!(half["z"].as_f64(), Some(-11.0));
    assert!(about(&half["loops"][0]["length"], 200.0) && about(&half["loops"][0]["area"], 2400.0), "{half:#}");
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"bottom": -11, "leave": 0.5}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!((t["numbers"]["levels"].clone(), t["numbers"]["offset"].as_f64()), (json!([-5.5, -11.0]), Some(10.5)));
    // A height below the part reads its lowest outline: a cut through into a spoil board.
    assert_eq!(run(&mut s, "cam.setup.outline", json!({"id": "setup-1", "z": -50}))["z"].as_f64(), Some(-21.0));
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"bottom": -22, "stepdown": 11}}));
    assert_eq!(run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["levels"], json!([-11.0, -22.0]));
    // The outline follows the model: a quarter round of radius 5 cut down one corner is in it
    // the next time it is read (an outline is kept only for the geometry it was read from).
    run(&mut s, "solid.cylinder", json!({"diameter": 10, "height": 30, "base": [60, 40, -5], "axis": "Z", "operation": "cut", "targets": ["Block"]}));
    let cut = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert!(about(&cut["loops"][0]["length"], 190.0 + 2.5 * PI) && about(&cut["loops"][0]["area"], 2400.0 - 6.25 * PI), "{cut:#}");
    assert_eq!(run(&mut s, "cam.setup.outline", json!({"id": "setup-1"})), cut, "read again, it is the same");
    // Above the part there is no outline; neither is there for a setup that isn't.
    assert!(refused(&mut s, "cam.setup.outline", json!({"id": "setup-1", "z": 5})).contains("has no outline at Z 5 mm"));
    assert!(refused(&mut s, "cam.setup.outline", json!({"id": "setup-1", "z": 1e300})).contains("finite"));
    assert!(refused(&mut s, "cam.setup.outline", json!({"id": "setup-9"})).contains("no setup 'setup-9'"));
    // Another kind's parameters are refused, and so is a tool that can't cut a wall.
    assert!(refused(&mut s, "cam.op.edit", json!({"id": "op-1", "face": {}})).contains("is a contour: it takes `contour`, not `face`"));
    assert!(refused(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 3})).contains("does not cut a contour"));
    assert!(refused(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4, "contour": {"tabs": 4}})).contains("contour"));
}

#[test]
fn an_outline_keeps_the_models_arcs_and_leaves_its_holes_alone() {
    use std::f64::consts::{FRAC_PI_2, PI};
    // The holed block: holes through it and into it, and a quarter round of radius 5 down the
    // corner at (30, 20). Its outline is the four sides, 5 short at that corner, and the quarter
    // arc; the holes are openings, not outline.
    let mut s = holed();
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert_eq!(found["loops"].as_array().map(Vec::len), Some(1), "{found:#}");
    let l = &found["loops"][0];
    assert!(about(&l["length"], 190.0 + 2.5 * PI) && about(&l["area"], 2400.0 - 6.25 * PI), "{l:#}");
    let arcs: Vec<&Value> = l["segments"].as_array().unwrap().iter().filter(|seg| seg["type"] == "arc").collect();
    assert_eq!(arcs.len(), 1, "{l:#}");
    assert!(about(&arcs[0]["radius"], 5.0) && about(&arcs[0]["center"][0], 30.0) && about(&arcs[0]["center"][1], 20.0), "{:#}", arcs[0]);
    // The 20 mm tool is too big for that corner: its centre rolls 10 from each end of the arc,
    // (25, 20) and (30, 15), and the two turns meet on the diagonal, sqrt(100 - 12.5) from the
    // middle between them. Each turns from its side to there: a quarter less the angle of
    // (h + 2.5, h - 2.5), h being that distance along each axis.
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let h = (87.5_f64 / 2.0).sqrt();
    let turn = FRAC_PI_2 - (h - 2.5).atan2(h + 2.5);
    assert!(about(&t["numbers"]["round"], 60.0 + 40.0 + 55.0 + 35.0 + 15.0 * PI + 2.0 * 10.0 * turn), "{}", t["numbers"]);
}

#[test]
fn a_contour_is_refused_where_the_part_is_wider_above() {
    // A 20 x 20 foot, 10 high, under a 40 x 40 cap, 10 high: at the foot's outline the tool
    // would cut the cap away.
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 10, "corner": [10, 10, 0], "body_name": "Part"}));
    run(&mut s, "solid.box", json!({"length": 40, "width": 40, "height": 10, "corner": [0, 0, 10], "operation": "join", "targets": ["Part"]}));
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.setup.create", json!({"material": "aluminium"}));
    let said = refused(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert!(said.contains("'Part' is wider above the contour's bottom than at it") && said.contains("raise the bottom"), "{said}");
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    let said = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(said.contains("operation op-1 (Contour 1)") && said.contains("wider above"), "{said}");
    // The form's preview says the same, in its own words.
    let previewed = crate::cmd::manufacture_preview(&s, "cam.op.contour", &json!({"setup": "setup-1", "tool": 4})).err().unwrap();
    assert!(previewed.contains("wider above") && !previewed.contains("cam.preview") && !previewed.contains("operation"), "{previewed}");
    // From the cap's bottom (Z -11 in the WCS) up, the outline is the cap's: 40 x 40.
    let cap = run(&mut s, "cam.setup.outline", json!({"id": "setup-1", "z": -11}));
    assert!(about(&cap["loops"][0]["length"], 160.0) && about(&cap["loops"][0]["area"], 1600.0), "{cap:#}");
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"bottom": -11}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 160.0 + 20.0 * std::f64::consts::PI), "{}", t["numbers"]);
    let ok = crate::cmd::manufacture_preview(&s, "cam.op.contour", &json!({"setup": "setup-1", "tool": 4, "contour": {"bottom": -11}})).unwrap();
    assert!(ok.pieces.len() > 20 && ok.rpm > 0.0 && ok.feed > 0.0 && ok.cut_minutes > 0.0);
    // The other way up (a cap under a foot) the part is widest at its bottom: fine as it is.
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 40, "width": 40, "height": 10, "corner": [0, 0, 0], "body_name": "Part"}));
    run(&mut s, "solid.box", json!({"length": 20, "width": 20, "height": 10, "corner": [10, 10, 10], "operation": "join", "targets": ["Part"]}));
    run(&mut s, "cam.setup.create", json!({"material": "aluminium"}));
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert!(about(&found["loops"][0]["length"], 160.0), "{found:#}");
}

#[test]
fn rounded_corners_are_arcs_in_the_outline_and_a_chamfer_underneath_is_an_overhang() {
    use std::f64::consts::PI;
    // The block with its four upright edges rounded to a radius of 5: sides of 50 and 30 and four
    // quarter arcs. The 20 mm tool's centre runs 10 out: the same sides, the arcs at 15.
    let mut s = milling();
    run(&mut s, "solid.fillet", json!({"edges": [[0, 0, 10], [60, 0, 10], [60, 40, 10], [0, 40, 10]], "radius": 5, "body": "Block"}));
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert_eq!(found["loops"].as_array().map(Vec::len), Some(1), "{found:#}");
    let l = &found["loops"][0];
    assert!(about(&l["length"], 160.0 + 10.0 * PI) && about(&l["area"], 2400.0 - (4.0 - PI) * 25.0), "{l:#}");
    let arcs: Vec<&Value> = l["segments"].as_array().unwrap().iter().filter(|seg| seg["type"] == "arc").collect();
    assert!(arcs.len() == 4 && arcs.iter().all(|arc| about(&arc["radius"], 5.0)), "{l:#}");
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 160.0 + 30.0 * PI), "{}", t["numbers"]);
    // The top edges rounded too: the part is narrower at its top, which is no overhang.
    run(&mut s, "solid.chamfer", json!({"edges": [[30, 0, 20], [30, 40, 20]], "distance": 2, "body": "Block"}));
    assert!(about(&run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["round"], 160.0 + 30.0 * PI));
    // A 2 mm chamfer along one bottom edge: at the bottom the part is 2 mm narrower than just
    // above it, so its outline there is not the part's. From the chamfer's top up it is.
    run(&mut s, "solid.chamfer", json!({"edges": [[30, 0, 0]], "distance": 2, "body": "Block"}));
    let said = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(said.contains("wider above the contour's bottom"), "{said}");
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"bottom": -19}}));
    assert!(about(&run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["round"], 160.0 + 30.0 * PI));
}

#[test]
fn round_parts_and_several_bodies_have_their_outlines() {
    use std::f64::consts::PI;
    // A disc of diameter 40: its outline is its circle, and the tool's centre runs at 30.
    let mut s = Session::default();
    run(&mut s, "solid.cylinder", json!({"diameter": 40, "height": 10, "body_name": "Disc"}));
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.setup.create", json!({"material": "aluminium"}));
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    let l = &found["loops"][0];
    assert!(about(&l["length"], 40.0 * PI) && about(&l["area"], 400.0 * PI), "{l:#}");
    assert!(l["segments"].as_array().unwrap().iter().all(|seg| seg["type"] == "arc" && about(&seg["radius"], 20.0)), "{l:#}");
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 60.0 * PI), "{}", t["numbers"]);
    // Round the disc clockwise, in arcs about its middle; on to it and off it by an arc of 2 the
    // other way round, at each of the two levels.
    let moves = t["toolpath"]["moves"].as_array().unwrap();
    let arcs: Vec<&Value> = moves.iter().filter(|m| m["kind"] == "arc").collect();
    let round = |m: &&&Value| m["center"][0].as_f64().is_some_and(|x| x.abs() < 1e-9) && m["center"][1].as_f64().is_some_and(|y| y.abs() < 1e-9);
    assert!(arcs.iter().filter(round).count() >= 4 && arcs.iter().filter(round).all(|m| m["direction"] == "clockwise"), "{moves:?}");
    let leads: Vec<&&Value> = arcs.iter().filter(|m| !round(m)).collect();
    assert!(leads.len() == 4 && leads.iter().all(|m| m["direction"] == "counter_clockwise"), "{moves:?}");
    assert_eq!(t["numbers"]["leads"], json!([2.0]));
    // Two bodies standing apart are two loops, each gone round.
    let mut s = milling();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 30, "corner": [100, 0, 0], "operation": "new", "body_name": "Post"}));
    let found = run(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    let mut lengths: Vec<f64> = found["loops"].as_array().unwrap().iter().map(|l| l["length"].as_f64().unwrap()).collect();
    lengths.sort_by(f64::total_cmp);
    assert!(lengths.len() == 2 && (lengths[0] - 40.0).abs() < 1e-9 && (lengths[1] - 200.0).abs() < 1e-9, "{lengths:?}");
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4, "contour": {"bottom": -20}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 240.0 + 40.0 * PI), "{}", t["numbers"]);
    assert_eq!(t["numbers"]["entries"].as_array().map(Vec::len), Some(2));
    // A body hanging in the air above the cut's bottom, beside the others, would be cut into.
    let mut s = milling();
    run(&mut s, "solid.box", json!({"length": 10, "width": 10, "height": 10, "corner": [100, 0, 5], "operation": "new", "body_name": "Shelf"}));
    let said = refused(&mut s, "cam.setup.outline", json!({"id": "setup-1"}));
    assert!(said.contains("'Shelf' is wider above the contour's bottom"), "{said}");
}

#[test]
fn an_outline_with_free_form_edges_is_fitted_with_arcs() {
    use solidcore_record::{Loop2, Plane, Region2, Seg2, Vec2, Vec3};
    use solidcraft_geom::prelude::*;
    let q = |x: f64, y: f64| Vec2 { x, y };
    let post =
        |outline: Loop2| solidcraft_kernel::extrude(&Plane::XY, &[Region2 { outer: outline, holes: Vec::new() }], 0.0, 10.0).unwrap().remove(0);
    let at = |z: f64| Plane { origin: Vec3 { x: 0.0, y: 0.0, z }, ..Plane::XY };
    // A post shaped like a D, 10 high: a straight side of 40 and one cubic curve bulging 18.75
    // from it (three quarters of its 25 mm handles).
    let cubic = Seg2::Cubic { p0: q(20.0, 0.0), p1: q(20.0, 25.0), p2: q(-20.0, 25.0), p3: q(-20.0, 0.0) };
    let d = post(Loop2 { segs: vec![Seg2::Line { a: q(-20.0, 0.0), b: q(20.0, 0.0) }, cubic] });
    let found = super::super::cam_outline::outline(&[("Post", &d)], &at(0.0)).unwrap();
    assert_eq!(found.len(), 1);
    let outline = &found[0];
    // Lines and arcs only, closed, counter-clockwise, and never more than 2 microns from the
    // curve: every piece's ends and middle are measured against the curve itself.
    assert!(outline.is_line_arc() && outline.is_closed(1e-9) && outline.is_ccw(), "{outline:?}");
    assert!(outline.segs.len() >= 5, "{} pieces", outline.segs.len());
    let off = |p: Vec2| if p.y <= 1e-9 { p.y.abs() } else { p.dist(cubic.point_at(cubic.closest_param(p))) };
    let worst = outline.segs.iter().flat_map(|seg| [seg.start(), seg.mid(), seg.end()]).map(off).fold(0.0, f64::max);
    assert!(worst <= 0.0021, "{worst} mm from the curve");
    // The area under a cubic with upright handles of 25 over a 40 mm chord is 3/5 of 40 x 25.
    assert!((outline.signed_area() - 600.0).abs() < 0.2, "{}", outline.signed_area());
    // Half way up it is the same outline, read from a section; above the post there is none.
    let cut = super::super::cam_outline::outline(&[("Post", &d)], &at(5.0)).unwrap();
    assert!(cut.len() == 1 && (cut[0].signed_area() - outline.signed_area()).abs() < 0.2, "{cut:?}");
    assert!(super::super::cam_outline::outline(&[("Post", &d)], &at(10.0)).unwrap().is_empty());
    // An elliptical post, 40 by 20 across: four quarter ellipses held exactly (rational
    // quadratics of weight cos 45°). Its outline is arcs within 2 microns of the ellipse,
    // measured to first order (f / |grad f| for f = (x/rx)^2 + (y/ry)^2 - 1).
    let (rx, ry, w) = (20.0, 10.0, std::f64::consts::FRAC_1_SQRT_2);
    let quarter = |a: Vec2, apex: Vec2, b: Vec2| Seg2::Conic { a, apex, b, w };
    let ellipse = post(Loop2 {
        segs: vec![
            quarter(q(rx, 0.0), q(rx, ry), q(0.0, ry)),
            quarter(q(0.0, ry), q(-rx, ry), q(-rx, 0.0)),
            quarter(q(-rx, 0.0), q(-rx, -ry), q(0.0, -ry)),
            quarter(q(0.0, -ry), q(rx, -ry), q(rx, 0.0)),
        ],
    });
    let found = super::super::cam_outline::outline(&[("Oval", &ellipse)], &at(0.0)).unwrap();
    let oval = &found[0];
    assert!(found.len() == 1 && oval.is_line_arc() && oval.is_closed(1e-9) && oval.is_ccw() && oval.segs.len() >= 8, "{oval:?}");
    let off = |p: Vec2| ((p.x / rx).powi(2) + (p.y / ry).powi(2) - 1.0).abs() / (2.0 * p.x / (rx * rx)).hypot(2.0 * p.y / (ry * ry));
    let worst = oval.segs.iter().flat_map(|seg| [seg.start(), seg.mid(), seg.end()]).map(off).fold(0.0, f64::max);
    assert!(worst <= 0.0021, "{worst} mm from the ellipse");
    assert!((oval.signed_area() - std::f64::consts::PI * rx * ry).abs() < 0.2, "{}", oval.signed_area());
    // The offsetter takes the fitted outline: 10 mm out, a smooth convex loop is longer by a
    // full turn of radius 10, and it is still about the ellipse's own 96.88 mm round.
    let length = |l: &Loop2| l.segs.iter().map(|seg| seg.length()).sum::<f64>();
    let grown = solidcraft_cam::offset_region(&Region2 { outer: oval.clone(), holes: Vec::new() }, -10.0).unwrap();
    assert_eq!(grown.len(), 1);
    assert!((length(&grown[0].outer) - (length(oval) + 20.0 * std::f64::consts::PI)).abs() < 1e-3, "{} {}", length(&grown[0].outer), length(oval));
    assert!((length(oval) - 96.884).abs() < 0.01, "{}", length(oval));
}

/// Tool 2: a 10 mm, 3-flute flat end mill with 22 mm of flutes and data for aluminium (150 m/min,
/// 0.05 mm a tooth, plunging and ramping at half): 4774.6 rev/min, 716.2 mm/min.
fn mill10() -> Value {
    json!({
        "number": 2, "name": "10 mm flat end mill", "kind": "flat_end_mill",
        "geometry": {"diameter": 10.0, "corner_radius": 0.0, "flute_length": 22.0, "shoulder_length": 0.0, "overall_length": 72.0,
                     "shank_diameter": 10.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 3, "holder": {"stick_out": 32.0, "segments": []}, "length_offset": 2, "diameter_offset": 2, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": 150.0, "chip_load": 0.05, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

/// A 100 x 80 x 20 plate in the vise setup (1 mm of stock on top and all round, the WCS over its
/// middle at the stock's top) with the 10 mm end mill: in the WCS the plate is X -50 to 50,
/// Y -40 to 40, its top at Z -1.
fn plate() -> Session {
    let mut s = Session::default();
    run(&mut s, "solid.box", json!({"length": 100, "width": 80, "height": 20, "corner": [0, 0, 0], "body_name": "Plate"}));
    run(&mut s, "cam.tool.create", json!({"tool": mill10()}));
    run(&mut s, "cam.setup.create", json!({"material": "aluminium"}));
    s
}

/// A box cut from (or, with `join`, added to) the plate: its corner and size in the plate's own
/// coordinates.
fn boxed(s: &mut Session, corner: [f64; 3], size: [f64; 3], operation: &str) {
    run(
        s,
        "solid.box",
        json!({"length": size[0], "width": size[1], "height": size[2], "corner": corner, "operation": operation, "targets": ["Plate"]}),
    );
}

#[test]
fn a_pocket_is_found_on_the_model_and_cleared_in_rings() {
    // A pocket 60 x 40, 6 deep, in the middle of the plate's top: its floor at Z -7 in the WCS.
    let mut s = plate();
    boxed(&mut s, [20.0, 20.0, 14.0], [60.0, 40.0, 6.0], "cut");
    let found = run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}));
    assert_eq!(found["pockets"].as_array().map(Vec::len), Some(1), "{found:#}");
    let pocket = &found["pockets"][0];
    assert_eq!(
        (pocket["floor"].as_f64(), pocket["depth"].as_f64(), pocket["size"].clone(), pocket["islands"].as_u64()),
        (Some(-7.0), Some(6.0), json!([60.0, 40.0]), Some(0))
    );
    assert!(about(&pocket["area"], 2400.0) && about(&pocket["widest"], 40.0), "{pocket:#}");
    // Nothing narrows it: a tool passes everywhere for as long as it fits.
    assert!((39.5..=40.0).contains(&pocket["passes"].as_f64().unwrap()), "{pocket:#}");
    let at = pocket["at"].clone();
    assert!(at[0].as_f64().unwrap().abs() < 30.0 && at[1].as_f64().unwrap().abs() < 20.0, "{at}");
    // The form reads the same list, and finding pockets changes nothing.
    assert_eq!(serde_json::to_value(crate::cmd::manufacture_pockets(&s, "setup-1").unwrap()).unwrap(), found["pockets"]);
    // The 10 mm tool's centre keeps 5 from the walls: rings of 26 x 6, 34 x 14, 42 x 22 and
    // 50 x 30, in two levels of 3.5 (see the strategy's own test for every length).
    assert_eq!(run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [at]}}))["id"], "op-1");
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert_eq!(
        (n["levels"].clone(), n["rings"].clone(), n["stepover"].as_f64(), n["offset"].as_f64()),
        (json!([[-3.5, -7.0]]), json!([4]), Some(4.0), Some(5.0))
    );
    let level = 128.0_f64.hypot(3.5) + 64.0 + 3.0 * 4.0 + 96.0 + 128.0 + 160.0;
    assert!(about(&t["feed_length"], 2.0 * level + 13.5), "{t}");
    assert_eq!((t["min"][2].as_f64(), t["max"][2].as_f64()), (Some(-7.0), Some(25.0)));
    assert!(t["min"][0].as_f64().unwrap() >= -25.0 - 1e-9 && t["max"][0].as_f64().unwrap() <= 25.0 + 1e-9, "{t}");
    // It posts, ramps and all, and the program read back is the toolpath.
    let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    assert!(posted["text"].as_str().unwrap().contains("T2 M6"), "{posted}");
    // The pocket follows the model: 4 mm deeper, and the same operation goes down to Z -11.
    boxed(&mut s, [20.0, 20.0, 10.0], [60.0, 40.0, 6.0], "cut");
    let deeper = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let levels: Vec<f64> = deeper["numbers"]["levels"][0].as_array().unwrap().iter().map(|z| z.as_f64().unwrap()).collect();
    assert!(levels.len() == 3 && levels.iter().zip([-11.0 / 3.0, -22.0 / 3.0, -11.0]).all(|(z, want)| (z - want).abs() < 1e-9), "{levels:?}");
    // Changed in place; another kind's parameters are refused, and so is a tool that can't.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "pocket": {"pockets": [[0, 0]], "stepover": 5, "leave": 0.5}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!((t["numbers"]["stepover"].as_f64(), t["numbers"]["offset"].as_f64()), (Some(5.0), Some(5.5)));
    assert!(refused(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {}})).contains("is a pocket operation: it takes `pocket`, not `contour`"));
    assert!(refused(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2})).contains("pocket"));
    // A stepover of more than half the tool is refused when the operation is made, with the reason.
    let wide = refused(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[0, 0]], "stepover": 6}}));
    assert!(wide.contains("half of tool 2's diameter"), "{wide}");
    assert!(refused(&mut s, "cam.setup.pockets", json!({"id": "setup-9"})).contains("no setup 'setup-9'"));
}

#[test]
fn islands_holes_and_steps_are_told_apart() {
    // A pocket 80 x 60, 6 deep, with a 20 x 20 island left standing in its middle and a hole of
    // 8 through its floor beside the island.
    let mut s = plate();
    boxed(&mut s, [10.0, 10.0, 14.0], [80.0, 60.0, 6.0], "cut");
    boxed(&mut s, [40.0, 30.0, 14.0], [20.0, 20.0, 6.0], "join");
    run(&mut s, "solid.cylinder", json!({"diameter": 8, "height": 30, "base": [25, 40, -5], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    let found = run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}));
    assert_eq!(found["pockets"].as_array().map(Vec::len), Some(1), "{found:#}");
    let pocket = &found["pockets"][0];
    // The island is one; the hole is air, not an island, and the tool passes over it.
    assert_eq!((pocket["floor"].as_f64(), pocket["size"].clone(), pocket["islands"].as_u64()), (Some(-7.0), json!([80.0, 60.0]), Some(1)));
    assert!(about(&pocket["area"], 4800.0 - 400.0), "{pocket:#}");
    // The widest circle that fits is not the 30 between the island's side and an end wall but a
    // little more, in a corner, touching two walls and the island's corner: its radius r has
    // (30 - r)^2 + (20 - r)^2 = r^2, so r = 50 - sqrt(1200) and it is 30.72 across. The number
    // given is from a search and never more than what fits.
    let widest = pocket["widest"].as_f64().unwrap();
    assert!((30.6..=2.0 * (50.0 - 1200.0_f64.sqrt()) + 1e-9).contains(&widest), "{pocket:#}");
    // A tool passes everywhere while it is narrower than the 20 mm between the island and the
    // long walls; a wider one that fits clears the two ends only.
    assert!((19.5..=20.0).contains(&pocket["passes"].as_f64().unwrap()), "{pocket:#}");
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [pocket["at"]]}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!(t["numbers"]["rings"], json!([6]), "{}", t["numbers"]);
    // A point on the island's top, on the plate's top or off the part is no pocket's floor.
    let at = |s: &mut Session, x: f64, y: f64| {
        run(s, "cam.op.edit", json!({"id": "op-1", "pocket": {"pockets": [[x, y]]}}));
        refused(s, "cam.op.toolpath", json!({"id": "op-1"}))
    };
    assert!(at(&mut s, 0.0, 0.0).contains("on top of the part"), "the island's top is the part's top");
    assert!(at(&mut s, 45.0, 35.0).contains("on top of the part"));
    assert!(at(&mut s, 80.0, 0.0).contains("nothing of the part under"));
    // Over the hole there is no floor: the ray goes through.
    assert!(at(&mut s, -25.0, 0.0).contains("nothing of the part under"));
    // A step cut into the plate's edge is open to the outside: not offered, and said so when asked for.
    let mut s = plate();
    boxed(&mut s, [70.0, 20.0, 14.0], [30.0, 40.0, 6.0], "cut");
    assert_eq!(run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}))["pockets"], json!([]));
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[35, 0]]}}));
    let said = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(said.contains("operation op-1 (Pocket 1)") && said.contains("open to the outside of the part"), "{said}");
    let previewed = crate::cmd::manufacture_preview(&s, "cam.op.pocket", &json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[35, 0]]}}))
        .err()
        .unwrap();
    assert!(previewed.contains("open to the outside") && !previewed.contains("operation"), "{previewed}");
}

#[test]
fn a_pocket_the_tool_cannot_cut_straight_down_is_refused() {
    // A pocket 40 x 40 at the top that widens to 60 x 60 lower down (a T-slot's shape): the floor
    // is seen through the opening, but the opening's rim hangs over it.
    let mut s = plate();
    boxed(&mut s, [30.0, 20.0, 17.0], [40.0, 40.0, 3.0], "cut");
    boxed(&mut s, [20.0, 10.0, 11.0], [60.0, 60.0, 6.0], "cut");
    assert_eq!(run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}))["pockets"], json!([]));
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[0, 0]]}}));
    let said = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(said.contains("in the way above the floor") && said.contains("'Plate'"), "{said}");
    // A pocket inside a pocket: both are found, the larger first. The upper one's region runs
    // over the lower one (air), so it has no island.
    let mut s = plate();
    boxed(&mut s, [20.0, 20.0, 16.0], [60.0, 40.0, 4.0], "cut");
    boxed(&mut s, [30.0, 30.0, 10.0], [20.0, 20.0, 6.0], "cut");
    let found = run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}));
    let floors: Vec<(f64, Value, u64)> = found["pockets"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| (p["floor"].as_f64().unwrap(), p["size"].clone(), p["islands"].as_u64().unwrap()))
        .collect();
    assert_eq!(floors, vec![(-5.0, json!([60.0, 40.0]), 0), (-11.0, json!([20.0, 20.0]), 0)], "the larger first: {found:#}");
    // A round blind hole is a round pocket; the hole through the plate is not one.
    let mut s = plate();
    run(&mut s, "solid.cylinder", json!({"diameter": 30, "height": 10, "base": [30, 40, 12], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    run(&mut s, "solid.cylinder", json!({"diameter": 12, "height": 30, "base": [75, 40, -5], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    let found = run(&mut s, "cam.setup.pockets", json!({"id": "setup-1"}));
    assert_eq!(found["pockets"].as_array().map(Vec::len), Some(1), "{found:#}");
    let round = &found["pockets"][0];
    assert_eq!((round["floor"].as_f64(), round["size"].clone()), (Some(-9.0), json!([30.0, 30.0])));
    assert!((round["widest"].as_f64().unwrap() - 30.0).abs() <= 0.02 && about(&round["area"], 225.0 * std::f64::consts::PI), "{round:#}");
    // Cleared in circles of 10, 6 and 2 (the centre's own 10, then 4 apart), down a helix.
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [round["at"]]}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert_eq!(t["numbers"]["rings"], json!([3]), "{}", t["numbers"]);
    let moves = t["toolpath"]["moves"].as_array().unwrap();
    assert!(
        moves.iter().filter(|m| m["kind"] == "arc").count() >= 12
            && moves.iter().all(|m| m["kind"] != "arc" || m["direction"] == "counter_clockwise")
    );
    let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    assert!(
        posted["text"].as_str().unwrap().lines().any(|line| line.starts_with("G3") && line.contains('Z')),
        "a helix is posted as arcs that go down: {}",
        posted["text"]
    );
}

#[test]
fn a_setup_is_simulated_on_its_stock_and_checked_against_the_part() {
    use std::f64::consts::PI;
    // The plate with a pocket 60 x 40, 6 deep: faced with the 20 mm tool, pocketed and
    // contoured with the 10 mm one. In stock 1 mm bigger all round and on top.
    let mut s = plate();
    boxed(&mut s, [20.0, 20.0, 14.0], [60.0, 40.0, 6.0], "cut");
    run(&mut s, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[0, 0]]}}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 2}));
    let before = run(&mut s, "cam.data", json!({}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    // The stock's longer side (102) in 400 cells of 0.255 mm; its 82 in 322, a hair narrower.
    assert_eq!((sim["cell"].as_f64(), sim["cells"].clone(), sim["tolerance"].as_f64()), (Some(0.255), json!([400, 322]), Some(0.01)));
    assert!(about(&sim["stock_volume"], 102.0 * 82.0 * 21.0), "{sim:#}");
    // Nothing cut into the part, no rapid through material, nothing below the stock.
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64(), sim["below_stock"].as_f64()), (Some(0), Some(0), Some(0.0)), "{sim:#}");
    assert_eq!((sim["gouges"].clone(), sim["rapid_hits"].clone()), (json!([]), json!([])));
    // Each operation's share, to within what the grid can tell: the 1 mm on top; the pocket's
    // 6 mm but for the four corners a 10 mm tool can't reach; the 1 mm of stock round the plate
    // over the 20 mm left of it.
    let ops = sim["operations"].as_array().unwrap();
    let removed: Vec<f64> = ops.iter().map(|op| op["removed"].as_f64().unwrap()).collect();
    assert_eq!(ops.iter().map(|op| op["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Face 1", "Pocket 2", "Contour 3"]);
    assert!(ops.iter().all(|op| op["skipped"].is_null() && op["gouges"] == 0 && op["rapid_hits"] == 0), "{ops:?}");
    let corners = 4.0 * (25.0 - 25.0 * PI / 4.0);
    let want = [102.0 * 82.0, (2400.0 - corners) * 6.0, (102.0 * 82.0 - 8000.0) * 20.0];
    for (got, want) in removed.iter().zip(want) {
        assert!((got - want).abs() < 0.02 * want, "{removed:?}, not {want}");
    }
    assert!(about(&sim["removed"], removed.iter().sum()), "{sim:#}");
    // What stands above the part is those corners; beside the part nothing to speak of.
    assert!((sim["over_part"].as_f64().unwrap() - corners * 6.0).abs() < 0.3 * corners * 6.0, "{}", sim["over_part"]);
    assert!(sim["beside_part"].as_f64().unwrap() < 0.02 * want[2], "{}", sim["beside_part"]);
    // A coarser grid on request; simulating changes nothing.
    let coarse = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 1}));
    assert_eq!((coarse["cells"].clone(), coarse["gouge_count"].as_u64()), (json!([102, 82]), Some(0)));
    assert_eq!(run(&mut s, "cam.data", json!({})), before);
    assert!(refused(&mut s, "cam.simulate", json!({"setup": "setup-9"})).contains("no setup 'setup-9'"));
    assert!(refused(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.001})).contains("at least 0.01 mm"));
}

#[test]
fn a_simulation_finds_a_cut_into_the_part_and_a_rapid_through_the_stock() {
    let mut s = plate();
    // By hand: down to Z -10 (9 mm into the plate, whose top is at -1) and 60 mm along X; then
    // a rapid back at Z -3 beside the slot it cut, through the stock.
    let path = json!({"start": p(-30.0, 0.0, 25.0), "moves": [
        {"kind": "tool_change", "tool": 2},
        {"kind": "spindle", "rpm": 4000.0, "direction": "clockwise"},
        {"kind": "rapid", "to": p(-30.0, 0.0, 5.0)},
        {"kind": "linear", "to": p(-30.0, 0.0, -10.0), "feed": 200.0},
        {"kind": "linear", "to": p(30.0, 0.0, -10.0), "feed": 400.0},
        {"kind": "rapid", "to": p(30.0, 0.0, 5.0)},
        {"kind": "rapid", "to": p(30.0, 20.0, 5.0)},
        {"kind": "rapid", "to": p(30.0, 20.0, -3.0)},
        {"kind": "rapid", "to": p(-30.0, 20.0, -3.0)},
        {"kind": "rapid", "to": p(-30.0, 20.0, 25.0)},
        {"kind": "spindle_stop"}
    ]});
    run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 2, "toolpath": path, "name": "By hand"}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    // Two moves cut into the part (the plunge and the slot), 9 mm below its top.
    assert_eq!(sim["gouge_count"].as_u64(), Some(2), "{sim:#}");
    let deepest = &sim["gouges"][1];
    assert_eq!(
        (deepest["operation"].as_str(), deepest["move"].as_u64(), deepest["depth"].as_f64(), deepest["at"][2].as_f64()),
        (Some("op-1"), Some(4), Some(9.0), Some(-10.0))
    );
    // Three rapids pass through material: straight down into it, across at Z -3, and back up
    // out of it.
    assert_eq!(sim["rapid_hit_count"].as_u64(), Some(3), "{sim:#}");
    let hits: Vec<(u64, f64)> =
        sim["rapid_hits"].as_array().unwrap().iter().map(|h| (h["move"].as_u64().unwrap(), h["depth"].as_f64().unwrap())).collect();
    assert_eq!(hits, vec![(7, 3.0), (8, 3.0), (9, 3.0)]);
    assert_eq!(sim["operations"][0]["gouges"].as_u64(), Some(2));
    // An operation whose toolpath can't be worked out is left out, with the reason; the rest run.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": "titanium"}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 2}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    let skipped = sim["operations"][1]["skipped"].as_str().unwrap_or_default();
    assert!(skipped.contains("rpm and feed") || skipped.contains("cutting data"), "{sim:#}");
    assert_eq!(sim["operations"][0]["gouges"].as_u64(), Some(2));
}

#[test]
fn the_part_a_simulation_checks_against_is_read_from_its_exact_faces() {
    // The plate with a boss of 20 across standing 5 above it, a dome of radius 8 on it, and a
    // 3 mm chamfer along the front edge of its top. The dome is the highest thing: 28 above the
    // plate's bottom, 1 under the stock's top. In the WCS the plate's top is Z -9, the boss's
    // -4, the dome's summit -1; the plate's middle is the origin.
    let mut s = plate();
    run(&mut s, "solid.cylinder", json!({"diameter": 20, "height": 5, "base": [30, 40, 20], "axis": "Z", "operation": "join", "targets": ["Plate"]}));
    run(&mut s, "solid.sphere", json!({"radius": 8, "center": [70, 40, 20], "operation": "join", "targets": ["Plate"]}));
    run(&mut s, "solid.chamfer", json!({"edges": [[50, 0, 20]], "distance": 3, "body": "Plate"}));
    let done = crate::cmd::manufacture_simulation(&s, "setup-1", Some(0.5)).unwrap();
    let (field, part) = (&done.stock.stock, done.stock.part().unwrap());
    assert_eq!((done.placement.model.max.z, done.placement.model.min.z, done.stock.floor), (-1.0, -29.0, -29.0));
    // The part's height over the cell nearest a place, and that cell's own centre.
    let over = |x: f64, y: f64| {
        let (i, j) = (((x - field.x) / field.dx - 0.5).round() as usize, ((y - field.y) / field.dy - 0.5).round() as usize);
        (field.centre(i, j), part[j * field.nx + i])
    };
    // Flat faces, exactly: the plate's top, the boss's top, and nothing beside the plate.
    assert_eq!((over(0.0, 20.0).1, over(-20.0, 0.0).1, over(-27.0, 5.0).1), (-9.0, -4.0, -4.0));
    assert!(over(50.6, 0.0).1 < done.stock.floor && over(0.0, -40.6).1 < done.stock.floor);
    // Just outside the boss's 10 mm radius it is the plate again.
    assert_eq!(over(-31.0, 0.0).1, -9.0);
    // The dome, asked with rays: over a cell d from its axis it stands sqrt(64 - d^2) above the plate.
    for (x, y) in [(20.0, 0.0), (23.0, 2.0), (15.0, -4.0), (26.9, 0.0)] {
        let (at, height) = over(x, y);
        let d = (at.x - 20.0).hypot(at.y);
        assert!((height - (-9.0 + (64.0 - d * d).sqrt())).abs() < 1e-6, "the dome {d} mm from its axis: {height}");
    }
    assert_eq!(over(29.0, 0.0).1, -9.0, "past the dome's foot");
    // The chamfer: from 3 below the top at the plate's front edge (Y -40) up to the top 3 mm in.
    for y in [-39.6, -38.6, -37.4] {
        let (at, height) = over(0.0, y);
        assert!((height - (-12.0 + (at.y + 40.0))).abs() < 1e-6, "the chamfer at Y {}: {height}", at.y);
    }
    assert_eq!(over(0.0, -36.6).1, -9.0);
    // With nothing cut, what stands above the part is the stock less the part within its
    // outline; the 1 mm of stock round it stands beside it.
    let (above, beside) = done.stock.leftover();
    assert!((beside - (102.0 * 82.0 - 8000.0) * 29.0).abs() < 0.02 * 364.0 * 29.0, "{beside}");
    let part_volume = 100.0 * 80.0 * 20.0 + 100.0 * std::f64::consts::PI * 5.0 + 2.0 / 3.0 * std::f64::consts::PI * 512.0 - 0.5 * 9.0 * 100.0;
    assert!((above - (8000.0 * 29.0 - part_volume)).abs() < 0.01 * 8000.0 * 9.0, "{above} and {}", 8000.0 * 29.0 - part_volume);
}

#[test]
fn a_drill_through_the_parts_own_hole_is_not_a_cut_into_the_part() {
    // The block with a hole of 10 through its middle, drilled with the 10 mm drill from its top
    // (Z -1 under 1 mm of stock) to its full diameter 1 mm past its bottom at Z -21. The drill's
    // point (5 / tan 59 long) goes that much further: below the stock, where there is neither
    // part nor material.
    let mut s = drilling();
    run(&mut s, "solid.cylinder", json!({"diameter": 10, "height": 30, "base": [30, 20, -5], "axis": "Z", "operation": "cut", "targets": ["Block"]}));
    let through = |at: [f64; 2]| json!({"holes": [at], "top": -1, "depth": 21, "depth_to": "full_diameter", "cycle": {"type": "peck", "peck": 5}});
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": through([0.0, 0.0])}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    assert!(about(&sim["below_stock"], 1.0 + 5.0 / 59f64.to_radians().tan()), "{sim:#}");
    // The same hole 15 mm to the side, where the part has none, cuts into it: from its top to
    // the stock's bottom, 20 mm.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "drill": through([15.0, 0.0])}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!(
        (sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64(), sim["gouges"][0]["depth"].as_f64()),
        (Some(1), Some(0), Some(20.0)),
        "{sim:#}"
    );
}

#[test]
fn a_contour_leaves_tabs_and_the_simulation_finds_them_standing() {
    let mut s = milling();
    // Two tabs round the block with the 20 mm tool: the middles of its left and right sides,
    // 2 mm high above the bottom at Z -21, on the tool's centre path 10 out from the block.
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4, "contour": {"tabs": {"count": 2}}}));
    let n = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"].clone();
    assert_eq!(n["tab_top"].as_f64(), Some(-19.0), "{n}");
    let tabs = n["tabs"].as_array().unwrap();
    assert_eq!(tabs.len(), 2);
    assert!(
        about(&tabs[0][0], -40.0)
            && tabs[0][1].as_f64().unwrap().abs() < 1e-9
            && about(&tabs[1][0], 40.0)
            && tabs[1][1].as_f64().unwrap().abs() < 1e-9,
        "{n}"
    );
    // The stock is 1 mm bigger than the block all round, and the contour takes all of that but
    // the tabs: each is that 1 mm strip for the 26 mm the tool is up, less what its round ends
    // take (twice the area under a circle of 10 from 9 to 10 out), 2 mm high.
    let under = 25.0 * std::f64::consts::PI - (4.5 * 19.0_f64.sqrt() + 50.0 * 0.9_f64.asin());
    let held = 2.0 * 2.0 * (26.0 - 2.0 * under);
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    let beside = sim["beside_part"].as_f64().unwrap();
    assert!((beside - held).abs() < 0.15 * held, "{beside} mm3 beside the part, not {held}");
    // Without tabs nothing is left beside it.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {}}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert!(sim["beside_part"].as_f64().unwrap() < 0.05 * held, "{sim:#}");
    // Tabs that can't be made are refused with the reason, and so are words that are not tabs.
    let refusal = |s: &mut Session, contour: Value| {
        run(s, "cam.op.edit", json!({"id": "op-1", "contour": contour}));
        refused(s, "cam.op.toolpath", json!({"id": "op-1"}))
    };
    assert!(refusal(&mut s, json!({"tabs": {"count": 6}})).contains("6 tab(s) 6 mm wide do not fit"));
    assert!(refusal(&mut s, json!({"tabs": {"count": 2, "height": 30}})).contains("as high as the cut"));
    assert!(refused(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"tabs": {"count": 2, "shape": "round"}}})).contains("contour"));
    // Straight in and out when the arcs are turned off.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"lead": 0}}));
    let n = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"].clone();
    assert_eq!((n["leads"].clone(), n["entries"].clone()), (json!([0.0]), json!([[0.0, -33.0]])));
}

/// An M6 x 1 tap as tool 5, with data for aluminium at 15 m/min: 795.8 rev/min on its 6 mm.
fn tap6() -> Value {
    json!({
        "number": 5, "name": "M6 x 1 tap", "kind": "tap",
        "geometry": {"diameter": 6.0, "corner_radius": 0.0, "flute_length": 20.0, "shoulder_length": 0.0, "overall_length": 80.0,
                     "shank_diameter": 6.0, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 1.0},
        "flutes": 3, "holder": {"stick_out": 40.0, "segments": []}, "length_offset": 5, "diameter_offset": 5, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": 15.0, "chip_load": 0.1, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

#[test]
fn a_tap_cuts_its_holes_in_step_with_the_spindle_and_posts_as_g84() {
    let mut s = drilling();
    run(&mut s, "cam.tool.create", json!({"tool": tap6()}));
    let tapped = |pitch: f64| json!({"holes": [[-20, -10], [20, -10]], "depth": 12, "cycle": {"type": "tap", "pitch": pitch}});
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 5, "drill": tapped(1.0)}));
    // A whole 796 rev/min, and one pitch a turn: 796 mm/min. 12 deep from the part's top at Z -1.
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert_eq!(
        (n["rpm"].as_f64(), n["feed"].as_f64(), n["bottom"].as_f64(), n["r"].as_f64()),
        (Some(796.0), Some(796.0), Some(-13.0), Some(5.0)),
        "{t}"
    );
    let holes: Vec<&Value> = t["toolpath"]["moves"].as_array().unwrap().iter().filter(|m| m["kind"] == "drill").collect();
    assert!(holes.len() == 2 && holes.iter().all(|m| m["cycle"] == json!({"type": "tap", "pitch": 1.0}) && m["feed"] == json!(796.0)), "{holes:?}");
    // In and back out at the feed: 2 x 18 mm a hole from R 5 to Z -13.
    assert!(about(&t["feed_length"], 72.0), "{t}");
    // Haas and NIST write it as G84, and the program reads back as the tapped holes it is.
    for post in ["haas_mill", "rs274"] {
        let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": post}));
        let text = posted["text"].as_str().unwrap();
        assert!(text.contains("S796 M3") && text.contains("G99 G84 X-20. Y-10. Z-13. R5. F796.\nX20. Y-10.\nG80"), "{post}:\n{text}");
    }
    // The Fanuc-style post has no tapping set up: it says so, and writes nothing.
    let said = refused(&mut s, "cam.post", json!({"setup": "setup-1", "post": "fanuc_mill"}));
    assert!(said.contains("a tapped hole can't be written as plain moves") && said.contains("does not write G84"), "{said}");
    // A thread is not simulated: the operation is left out by name.
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    let skipped = sim["operations"][0]["skipped"].as_str().unwrap_or_default();
    assert!(skipped.contains("cuts a thread") && sim["gouge_count"] == 0, "{sim:#}");
    // The setup sheet calls it what it is: a drilling operation that taps is a tap.
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert!(sheet["html"].as_str().unwrap().contains("<td>Drill 1 <span class=\"note\">tap</span></td>"), "{}", sheet["html"]);
    // Drill's form is offered the tap with its pitch, beside the drill; Contour's is not.
    let offered: Vec<(u32, Option<f64>)> = crate::cmd::manufacture_tools(&s.doc, "drill").iter().map(|t| (t.number, t.pitch)).collect();
    assert_eq!(offered, vec![(3, None), (5, Some(1.0))]);
    assert!(crate::cmd::manufacture_tools(&s.doc, "contour").is_empty());
    // A tap cuts with the tap cycle at its own pitch and takes no feed; the tap cycle needs a tap.
    let edited = |s: &mut Session, drill: Value| refused(s, "cam.op.edit", json!({"id": "op-1", "drill": drill}));
    assert!(edited(&mut s, tapped(1.25)).contains("the cycle's pitch (1.25 mm) is not tool 5's (1 mm)"));
    assert!(edited(&mut s, three_holes()).contains("tool 5 is a tap: it cuts with the tap cycle, at its pitch of 1 mm"));
    let mut fed = tapped(1.0);
    fed["feed"] = json!(500);
    assert!(edited(&mut s, fed).contains("give the speed (rpm), not a feed"));
    assert!(refused(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": tapped(1.0)})).contains("the tap cycle needs a tap"));
    // A speed given is taken to the whole turn, and the feed follows it.
    let mut slow = tapped(1.0);
    slow["rpm"] = json!(300.4);
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "drill": slow}));
    let n = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"].clone();
    assert_eq!((n["rpm"].as_f64(), n["feed"].as_f64()), (Some(300.0), Some(300.0)));
}

/// A 10 mm chamfer mill, 90 degrees at its point, as tool 6, with data for aluminium.
fn chamfer10() -> Value {
    json!({
        "number": 6, "name": "10 mm chamfer mill", "kind": "chamfer_mill",
        "geometry": {"diameter": 10.0, "corner_radius": 0.0, "flute_length": 10.0, "shoulder_length": 0.0, "overall_length": 60.0,
                     "shank_diameter": 10.0, "tip_angle": std::f64::consts::FRAC_PI_2, "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 4, "holder": {"stick_out": 30.0, "segments": []}, "length_offset": 6, "diameter_offset": 6, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": 100.0, "chip_load": 0.05, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

#[test]
fn a_chamfer_goes_round_the_parts_top_edge_and_the_simulation_takes_it_as_meant() {
    use std::f64::consts::PI;
    // The block, faced and contoured with the 20 mm tool, then its top edge broken.
    let mut s = milling();
    run(&mut s, "cam.tool.create", json!({"tool": chamfer10()}));
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    assert_eq!(run(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6}))["id"], "op-3");
    // Half a millimetre wide with a 90 degree tool: 0.5 down the wall from the block's top at
    // Z -1, the tip 1 below that, the axis 1 outside the wall, on and off by arcs of 1 (see the
    // strategy's own test for every move).
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-3"}));
    let n = &t["numbers"];
    assert_eq!(
        (n["top"].as_f64(), n["z"].as_f64(), n["width"].as_f64(), n["leg"].as_f64(), n["offset"].as_f64()),
        (Some(-1.0), Some(-2.5), Some(0.5), Some(0.5), Some(1.0)),
        "{t}"
    );
    assert!(about(&n["round"], 200.0 + 2.0 * PI), "{n}");
    assert!(about(&n["entries"][0][0], 1.0) && about(&n["entries"][0][1], -22.0) && n["leads"] == json!([1.0]), "{n}");
    // The model's edge is sharp, and the chamfer is meant: the simulation finds nothing cut into
    // the part, and the chamfer's triangle (0.5 x 0.5 / 2) taken off round 200 mm of edge, as
    // nearly as cells of 0.155 mm can tell.
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    let ops = sim["operations"].as_array().unwrap();
    assert!(ops.len() == 3 && ops.iter().all(|op| op["skipped"].is_null() && op["gouges"] == 0), "{ops:?}");
    let taken = ops[2]["removed"].as_f64().unwrap();
    assert!((taken - 25.0).abs() < 8.0, "{taken} mm3 by the chamfer");
    // It posts with its tool, and Chamfer's form is offered that tool and no other.
    let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}));
    assert!(posted["text"].as_str().unwrap().contains("T6 M6"), "{posted}");
    let offered: Vec<u32> = crate::cmd::manufacture_tools(&s.doc, "chamfer").iter().map(|t| t.number).collect();
    assert_eq!(offered, vec![6]);
    // The same toolpath made by hand has no such meaning: every move along the wall cuts into
    // the part.
    let by_hand = t["toolpath"].clone();
    run(&mut s, "cam.op.delete", json!({"id": "op-3"}));
    run(&mut s, "cam.op.manual", json!({"setup": "setup-1", "tool": 6, "toolpath": by_hand}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert!(sim["gouge_count"].as_u64().unwrap() >= 8 && sim["gouges"][0]["depth"].as_f64().unwrap() <= 0.5 + 1e-9, "{sim:#}");
    // What can't be chamfered is refused with the reason; another kind's parameters too.
    assert!(refused(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 4})).contains("does not cut a chamfer"));
    let too_wide = refused(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6, "chamfer": {"width": 4.5}}));
    assert!(too_wide.contains("is cut 11 mm across on the tool, and tool 6 is 10 mm"), "{too_wide}");
    run(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6}));
    let id = run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["operations"][3]["id"].as_str().unwrap().to_string();
    assert!(refused(&mut s, "cam.op.edit", json!({"id": id, "face": {}})).contains("is a chamfer: it takes `chamfer`, not `face`"));
    run(&mut s, "cam.op.edit", json!({"id": id, "chamfer": {"width": 1.0, "tip_below": 0.5}}));
    let n = run(&mut s, "cam.op.toolpath", json!({"id": id}))["numbers"].clone();
    assert_eq!((n["z"].as_f64(), n["offset"].as_f64(), n["leads"].clone()), (Some(-2.5), Some(0.5), json!([1.5])));

    // Round a disc of 40 the tool goes in arcs, which the simulation cuts as chords a hair
    // inside them: that hair is not a cut into the part. The chamfer's triangle (0.125 mm2)
    // goes once round at a radius of 19.83: 15.6 mm3, as nearly as cells of 0.105 mm can tell.
    let mut disc = Session::default();
    run(&mut disc, "solid.cylinder", json!({"diameter": 40, "height": 10, "body_name": "Disc"}));
    run(&mut disc, "cam.tool.create", json!({"tool": mill20()}));
    run(&mut disc, "cam.tool.create", json!({"tool": chamfer10()}));
    run(&mut disc, "cam.setup.create", json!({"material": "aluminium"}));
    run(&mut disc, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut disc, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    run(&mut disc, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6}));
    let sim = run(&mut disc, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!(
        (sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64(), sim["body_hit_count"].as_u64()),
        (Some(0), Some(0), Some(0)),
        "{sim:#}"
    );
    let taken = sim["operations"][2]["removed"].as_f64().unwrap();
    assert!((taken - 15.6).abs() < 4.0, "{taken} mm3 by the chamfer round the disc");

    // A plate with a pocket and an island standing in it as high as the plate: just under the
    // top the island is a piece of its own, inside the plate's outline. The chamfer goes round
    // the plate (100 x 80: 360 mm and the four corners' quarter turns of radius 1) and says
    // that the island's edge is left sharp.
    let mut s = plate();
    boxed(&mut s, [10.0, 10.0, 14.0], [80.0, 60.0, 6.0], "cut");
    boxed(&mut s, [40.0, 30.0, 14.0], [20.0, 20.0, 6.0], "join");
    run(&mut s, "cam.tool.create", json!({"tool": chamfer10()}));
    run(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 360.0 + 2.0 * PI) && t["numbers"]["entries"].as_array().is_some_and(|e| e.len() == 1), "{}", t["numbers"]);
    let notes: Vec<&str> = t["notes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(notes.iter().any(|n| n.starts_with("1 edge(s) inside the part's outline (an island standing in a pocket) left sharp")), "{notes:?}");
}

#[test]
fn a_setup_sheet_lists_what_the_shop_sets_up_and_what_to_check() {
    // The block faced and contoured with the 20 mm tool (T4), and three holes pecked 10 deep with
    // the 10 mm drill (T3).
    let mut s = milling();
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    // So far there is nothing to check: no notes, and no heading for them.
    let clean = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert_eq!((clean["notes"].clone(), clean["tools"].as_u64(), clean["operations"].as_u64()), (json!([]), Some(1), Some(2)));
    assert!(!clean["html"].as_str().unwrap().contains("To check"));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "date": "2026-11-22"}));
    assert_eq!(
        (sheet["tools"].as_u64(), sheet["operations"].as_u64(), sheet["left_out"].as_u64(), sheet["path"].clone()),
        (Some(2), Some(3), Some(0), Value::Null),
        "{sheet:#}"
    );
    let html = sheet["html"].as_str().unwrap();
    let has = |text: &str| assert!(html.contains(text), "no {text:?} in:\n{html}");
    has("Work offset G54 · 2026-11-22");
    has("Unverified program. Prove it out on the machine first: air cut, then single block.");
    has("<tr><th>Stock</th><td>62.00 × 42.00 × 21.00 mm, aluminium</td></tr>");
    has("<tr><th>Part</th><td>60.00 × 40.00 × 20.00 mm</td></tr>");
    has("<tr><th>Work origin</th><td>G54 at the middle of the stock&#39;s top</td></tr>");
    has("<tr><th>Safe heights</th><td>clearance Z 25.00, retract Z 5.00 mm</td></tr>");
    // The tools the operations use, each with the lowest its tip goes: the drill to Z -11, the
    // end mill to the bottom of the block.
    has(
        "<td>10 mm drill</td><td class=\"n\">10.00</td><td class=\"n\">2</td><td class=\"n\">50.00</td><td class=\"n\">3</td><td class=\"n\">3</td><td class=\"n\">-11.00</td>",
    );
    has(
        "<td>20 mm flat end mill</td><td class=\"n\">20.00</td><td class=\"n\">4</td><td class=\"n\">50.00</td><td class=\"n\">4</td><td class=\"n\">4</td><td class=\"n\">-21.00</td>",
    );
    // The operations in the order they run, and the time in all: what each toolpath gives.
    let minutes: f64 =
        ["op-1", "op-2", "op-3"].iter().map(|id| run(&mut s, "cam.op.toolpath", json!({"id": id}))["cut_minutes"].as_f64().unwrap()).sum();
    assert!(about(&sheet["minutes"], minutes), "{} and {minutes}", sheet["minutes"]);
    for (k, name) in ["Face 1", "Contour 2", "Drill 3"].iter().enumerate() {
        has(&format!("<td class=\"n\">{}</td><td>{name}</td>", k + 1));
    }
    // From above: the three holes as the drill's own circles, and the cuts as lines.
    assert_eq!(html.matches("r=\"5.00\" stroke=\"#1f5fbf\"").count(), 3, "{html}");
    assert!(html.contains("<path d=\"M") && html.contains("<circle cx=\"0\" cy=\"0\""));
    // The model has no holes where the drill goes, and the sheet says what the simulation found.
    assert_eq!(sheet["notes"].as_array().map(Vec::len), Some(1), "{}", sheet["notes"]);
    has("<h2>To check</h2>\n<ul>\n<li>The simulation found 3 move(s) that cut into the part, the deepest ");

    // In inches when asked, or when the design is drawn in inches.
    let inch = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "units": "inch"}));
    assert!(
        inch["html"].as_str().unwrap().contains("<td>2.441 × 1.654 × 0.827 in, aluminium</td>") && !inch["html"].as_str().unwrap().contains("2026")
    );
    assert!(refused(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "units": "furlong"})).contains("mm or inch"));
    assert!(refused(&mut s, "cam.setup.sheet", json!({"setup": "setup-9"})).contains("no setup 'setup-9'"));

    // What is worth checking gets a line: a drill that goes through into what is under the part,
    // a tool that goes deeper than it sticks out of its holder, a cut into the part.
    run(&mut s, "cam.op.edit", json!({"id": "op-3", "drill": {"holes": [[-20, -10]], "depth": 24, "cycle": {"type": "peck", "peck": 5}}}));
    let mut short = mill20();
    short["holder"]["stick_out"] = json!(20.0);
    run(&mut s, "cam.tool.edit", json!({"number": 4, "tool": short}));
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    let notes: Vec<&str> = sheet["notes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    let html = sheet["html"].as_str().unwrap();
    assert!(notes.iter().any(|n| n.contains("A tool goes 4.00 mm below the bottom of the stock")), "{notes:?}");
    assert!(notes.iter().any(|n| n.contains("The simulation found 1 move(s) that cut into the part")), "{notes:?}");
    assert!(html.contains("T4 goes 21.00 mm below the top of the stock and sticks out 20.00 mm from its holder"), "{html}");
    assert!(html.contains("<h2>To check</h2>"));
    // An operation whose toolpath can't be worked out is listed as not in the program.
    run(&mut s, "cam.setup.edit", json!({"id": "setup-1", "material": "titanium"}));
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert_eq!((sheet["left_out"].as_u64(), sheet["minutes"].as_f64()), (Some(3), Some(0.0)));
    assert!(sheet["html"].as_str().unwrap().contains("Not in the program:"));
}

#[test]
fn a_setup_sheet_is_written_to_an_html_file_and_never_over_one_by_surprise() {
    let mut s = milling();
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    let dir = std::env::temp_dir().join(format!("solidcraft-cam-sheet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("block.html").to_string_lossy().to_string();
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "path": file}));
    assert_eq!(sheet["path"], json!(file));
    assert_eq!(std::fs::read_to_string(&file).unwrap(), sheet["html"].as_str().unwrap());
    assert!(refused(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "path": file})).contains("could not write the setup sheet"));
    run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "path": file, "overwrite": true}));
    // A sheet is a page: another kind of file is refused, and nothing is written.
    let other = dir.join("block.nc").to_string_lossy().to_string();
    assert!(refused(&mut s, "cam.setup.sheet", json!({"setup": "setup-1", "path": other})).contains("an .html file"));
    assert!(!std::path::Path::new(&other).exists());
    // A setup with no operations still has its stock and origin to say.
    run(&mut s, "cam.op.delete", json!({"id": "op-1"}));
    let empty = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert_eq!((empty["tools"].as_u64(), empty["operations"].as_u64()), (Some(0), Some(0)));
    assert!(empty["html"].as_str().unwrap().contains("<tr><th>Stock</th><td>62.00 × 42.00 × 21.00 mm, aluminium</td></tr>"));
}

#[test]
fn a_simulation_finds_a_holder_in_the_stock() {
    // The plate with a pocket 60 x 40, 15 deep: its floor is at Z -16 under the stock's top. The
    // 10 mm end mill sticks 12 out of a holder 40 across, so at the floor the holder's face is
    // at Z -4, in the stock round the pocket wherever the tool is near a wall.
    let mut s = plate();
    boxed(&mut s, [20.0, 20.0, 5.0], [60.0, 40.0, 15.0], "cut");
    let mut held = mill10();
    held["holder"] = json!({"stick_out": 12.0, "segments": [{"length": 30.0, "lower_diameter": 40.0, "upper_diameter": 40.0}]});
    run(&mut s, "cam.tool.edit", json!({"number": 2, "tool": held}));
    run(&mut s, "cam.op.pocket", json!({"setup": "setup-1", "tool": 2, "pocket": {"pockets": [[0, 0]]}}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    let count = sim["body_hit_count"].as_u64().unwrap();
    assert!(count > 0 && sim["operations"][0]["body_hits"].as_u64() == Some(count), "{sim:#}");
    let hits = sim["body_hits"].as_array().unwrap();
    assert!(hits.iter().all(|hit| hit["part"] == "holder" && hit["operation"] == "op-1"), "{hits:?}");
    let deepest = hits.iter().map(|hit| hit["depth"].as_f64().unwrap()).fold(0.0, f64::max);
    assert!((deepest - 4.0).abs() < 1e-9, "{deepest}");
    // The setup sheet gives it as a line to check.
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    let said =
        format!("The simulation found {count} move(s) with a tool's shank or holder in material, the deepest 4.00 mm (the holder), in Pocket 1.");
    assert!(sheet["notes"].as_array().unwrap().iter().any(|note| note == &json!(said)), "{}", sheet["notes"]);
    // Sticking 20 out, the holder stays above the stock, and nothing is found.
    let mut longer = mill10();
    longer["holder"] = json!({"stick_out": 20.0, "segments": [{"length": 30.0, "lower_diameter": 40.0, "upper_diameter": 40.0}]});
    run(&mut s, "cam.tool.edit", json!({"number": 2, "tool": longer}));
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    assert_eq!((sim["body_hit_count"].as_u64(), sim["body_hits"].clone()), (Some(0), json!([])));
}

#[test]
fn operations_are_moved_and_copied_within_their_setup() {
    // The block faced, contoured and drilled, in that order: op-1, op-2, op-3.
    let mut s = milling();
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    let order = |s: &mut Session| -> Vec<(String, String)> {
        let d = run(s, "cam.data", json!({}));
        let ops = d["data"]["setups"][0]["operations"].as_array().unwrap().clone();
        ops.iter().map(|o| (o["id"].as_str().unwrap().to_string(), o["name"].as_str().unwrap().to_string())).collect()
    };
    let ids = |s: &mut Session| order(s).into_iter().map(|(id, _)| id).collect::<Vec<_>>().join(" ");
    assert_eq!(ids(&mut s), "op-1 op-2 op-3");
    // The drilling first: holes before the part is cut round. The program follows the order.
    let moved = run(&mut s, "cam.op.move", json!({"id": "op-3", "to": 0}));
    assert_eq!((moved["id"].as_str(), moved["setup"].as_str(), moved["operations"].as_u64()), (Some("op-3"), Some("setup-1"), Some(3)));
    assert_eq!(ids(&mut s), "op-3 op-1 op-2");
    let text = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}))["text"].as_str().unwrap().to_string();
    let (drill, mill) = (text.find("T3 M6").unwrap(), text.find("T4 M6").unwrap());
    assert!(drill < mill, "the drill's tool change comes first: {text}");
    // Past the end is the end; to its own place changes nothing; one undo puts a move back.
    run(&mut s, "cam.op.move", json!({"id": "op-3", "to": 99}));
    assert_eq!(ids(&mut s), "op-1 op-2 op-3");
    run(&mut s, "cam.op.move", json!({"id": "op-2", "to": 1}));
    assert_eq!(ids(&mut s), "op-1 op-2 op-3");
    run(&mut s, "cam.op.move", json!({"id": "op-1", "to": 1}));
    assert_eq!(ids(&mut s), "op-2 op-1 op-3");
    run(&mut s, "edit.undo", json!({}));
    assert_eq!(ids(&mut s), "op-1 op-2 op-3");

    // A copy comes right after what it copies, with an id and a name of its own and the same
    // tool and parameters; a second copy takes the next number.
    let copy = run(&mut s, "cam.op.duplicate", json!({"id": "op-2"}));
    assert_eq!((copy["id"].as_str(), copy["operations"].as_u64()), (Some("op-4"), Some(4)));
    run(&mut s, "cam.op.duplicate", json!({"id": "op-2"}));
    let named: Vec<String> = order(&mut s).into_iter().map(|(id, name)| format!("{id} {name}")).collect();
    assert_eq!(named, ["op-1 Face 1", "op-2 Contour 2", "op-5 Contour 2 (3)", "op-4 Contour 2 (2)", "op-3 Drill 3"]);
    let d = run(&mut s, "cam.data", json!({}));
    let ops = &d["data"]["setups"][0]["operations"];
    assert_eq!(
        (ops[1]["operation"].clone(), ops[1]["tool"].clone(), ops[1]["suppressed"].clone()),
        (ops[3]["operation"].clone(), ops[3]["tool"].clone(), ops[3]["suppressed"].clone())
    );
    let same = |a: &str, b: &str, s: &mut Session| {
        run(s, "cam.op.toolpath", json!({"id": a}))["feed_length"] == run(s, "cam.op.toolpath", json!({"id": b}))["feed_length"]
    };
    assert!(same("op-2", "op-4", &mut s));
    // A copy can be given its name, and a changed copy leaves what it was copied from alone.
    let named = run(&mut s, "cam.op.duplicate", json!({"id": "op-3", "name": "Drill again"}));
    run(&mut s, "cam.op.edit", json!({"id": named["id"], "suppressed": true}));
    let d = run(&mut s, "cam.data", json!({}));
    let ops = d["data"]["setups"][0]["operations"].as_array().unwrap();
    assert_eq!(
        (ops[5]["name"].as_str(), ops[5]["suppressed"].as_bool(), ops[4]["suppressed"].as_bool()),
        (Some("Drill again"), Some(true), Some(false))
    );

    // What is not there is said; hostile numbers are refused, not run.
    assert!(refused(&mut s, "cam.op.move", json!({"id": "op-99", "to": 0})).contains("no operation 'op-99'"));
    assert!(refused(&mut s, "cam.op.duplicate", json!({"id": "op-99"})).contains("no operation 'op-99'"));
    assert!(refused(&mut s, "cam.op.move", json!({"id": "op-1", "to": -1})).contains("invalid"));
    assert!(refused(&mut s, "cam.op.move", json!({"id": "op-1"})).contains("to"));
    assert!(refused(&mut s, "cam.op.duplicate", json!({"id": "op-1", "copies": 2})).contains("copies"));
}

#[test]
fn an_inside_contour_goes_round_an_opening_read_from_the_model() {
    use std::f64::consts::PI;
    // The plate (in the WCS X -50 to 50, Y -40 to 40, its top at Z -1 and its bottom at Z -21)
    // with a window 40 x 30 through it about (-15, 0) and a round pocket of 24, 8 deep, about
    // (30, 0). Tool 2 is the 10 mm end mill.
    let mut s = plate();
    boxed(&mut s, [15.0, 25.0, -5.0], [40.0, 30.0, 30.0], "cut");
    run(&mut s, "solid.cylinder", json!({"diameter": 24, "height": 8, "base": [80, 40, 12], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    // Round the inside of the window, named by a point in it: nothing is under the point, so the
    // cut goes through, to the plate's bottom. The tool's centre is 5 inside the wall, 30 x 20
    // and 100 round; 21 mm from the stock's top in steps of at most 5 is five levels of 4.2. It
    // comes down over the middle of the front side and leaves the wall by a quarter turn of 1.
    assert_eq!(
        run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 2, "contour": {"side": "inside", "openings": [[-15, 0]]}}))["id"],
        "op-1"
    );
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert!(about(&n["round"], 100.0) && about(&n["offset"], 5.0), "{n}");
    let levels = n["levels"].as_array().unwrap();
    assert!(
        levels.len() == 5 && levels.iter().zip([-4.2, -8.4, -12.6, -16.8, -21.0]).all(|(level, z)| about(level, z)) && n["leads"] == json!([1.0]),
        "{n}"
    );
    assert!(about(&n["entries"][0][0], -15.0) && about(&n["entries"][0][1], -10.0), "{n}");
    assert!(
        t["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().is_some_and(|note| note.contains("a core about 10.0 mm across is left in its middle"))),
        "{}",
        t["notes"]
    );
    // The simulation: the band the tool passes over is the window with its corners left round
    // (1200 less 4 (25 - 25 pi / 4)) less the middle 20 x 10, through the stock's 21 mm; nothing
    // of the plate is cut into.
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.2}));
    assert_eq!(
        (sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64(), sim["body_hit_count"].as_u64()),
        (Some(0), Some(0), Some(0)),
        "{sim:#}"
    );
    let band = (1200.0 - 4.0 * (25.0 - 25.0 * PI / 4.0) - 200.0) * 21.0;
    let removed = sim["operations"][0]["removed"].as_f64().unwrap();
    assert!((removed - band).abs() < 0.015 * band, "{removed} mm3, not {band}");

    // The round pocket is bored by the same operation under its other name, Bore, which is
    // inside whatever side it is given: its floor is under the point, so the cut goes down to
    // it, Z -9, in two levels of 4.5; the path is a circle of radius 7 about the pocket's
    // middle, every turn of the way down an arc about it.
    assert_eq!(
        run(&mut s, "cam.op.bore", json!({"setup": "setup-1", "tool": 2, "contour": {"side": "outside", "openings": [[30, 0]]}}))["id"],
        "op-2"
    );
    let kept = run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["operations"][1].clone();
    assert_eq!(
        (kept["name"].as_str(), kept["operation"]["kind"].as_str(), kept["operation"]["contour"]["side"].as_str()),
        (Some("Bore 2"), Some("contour"), Some("inside"))
    );
    assert!(refused(&mut s, "cam.op.bore", json!({"setup": "setup-1", "tool": 2})).contains("1 to 1000 openings, not 0"));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-2"}));
    assert!(about(&t["numbers"]["round"], 14.0 * PI) && t["numbers"]["levels"] == json!([-4.5, -9.0]), "{}", t["numbers"]);
    let arcs: Vec<&Value> = t["toolpath"]["moves"].as_array().unwrap().iter().filter(|m| m["kind"] == "arc").collect();
    let about_the_middle = arcs.iter().filter(|m| about(&m["center"][0], 30.0) && m["center"][1].as_f64().is_some_and(|y| y.abs() < 1e-9)).count();
    assert!(about_the_middle >= 12 && arcs.iter().all(|m| m["direction"] == "counter_clockwise"), "{about_the_middle} of {}", arcs.len());
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.2}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    // The pocket is more than twice as wide as the tool, whose centre runs 7 from its middle:
    // the tool passes over the ring from 2 to 12 and leaves a core 4 across, which the
    // operation says. 9 mm of that ring from the stock's top, as nearly as the cells can tell.
    assert!(
        t["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().is_some_and(|note| note.contains("a core about 4.0 mm across is left in its middle"))),
        "{}",
        t["notes"]
    );
    let bored = sim["operations"][1]["removed"].as_f64().unwrap();
    assert!((bored - 140.0 * PI * 9.0).abs() < 0.02 * 140.0 * PI * 9.0, "{bored} mm3");
    // The setup sheet says which side each goes round, in the words its name does not have.
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    let html = sheet["html"].as_str().unwrap();
    assert!(
        html.contains("<td>Contour 1 <span class=\"note\">inside</span></td>")
            && html.contains("<td>Bore 2 <span class=\"note\">inside contour</span></td>"),
        "{html}"
    );
    // It posts, the helix as arcs with Z on them.
    let text = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}))["text"].as_str().unwrap().to_string();
    assert!(text.lines().any(|line| line.starts_with("G3 ") && line.contains(" Z-")), "{text}");

    // A bottom a little under the part breaks through: the wall is read at the part's own
    // bottom, and the cut goes as far as it was told (21.5 mm in five levels of 4.3).
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside", "openings": [[-15, 0]], "bottom": -21.5}}));
    let through = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["levels"].clone();
    assert!(through.as_array().is_some_and(|levels| levels.len() == 5) && about(&through[0], -4.3) && about(&through[4], -21.5), "{through}");
    // Under the pocket's floor there is no opening: a bottom there is refused, not cut.
    run(&mut s, "cam.op.edit", json!({"id": "op-2", "contour": {"side": "inside", "openings": [[30, 0]], "bottom": -12}}));
    let why = refused(&mut s, "cam.op.toolpath", json!({"id": "op-2"}));
    assert!(why.contains("X 30 Y 0 is not in an opening of the part at Z -12 mm"), "{why}");
    // Given its bottom, the window is cut that far and no further.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside", "openings": [[-15, 0]], "bottom": -10}}));
    assert_eq!(run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["numbers"]["levels"], json!([-5.0, -10.0]));
    // What it keeps is what it was given.
    let kept = run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["operations"][0]["operation"]["contour"].clone();
    assert_eq!((kept["side"].clone(), kept["openings"].clone(), kept["bottom"].clone()), (json!("inside"), json!([[-15.0, 0.0]]), json!(-10.0)));

    // Openings of different depths are an operation each; a point on the part or beside it is
    // in no opening; an inside contour needs its openings, and an outside one takes none.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside", "openings": [[-15, 0], [30, 0]]}}));
    let why = refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(why.contains("the openings are cut to different depths (Z -21 and Z -9 mm)"), "{why}");
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside", "openings": [[45, 35]]}}));
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"})).contains("X 45 Y 35 is on top of the part, not in an opening"));
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside", "openings": [[70, 0]]}}));
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"})).contains("X 70 Y 0 is not in an opening of the part at Z -21 mm"));
    // (Those two are refused as the operation is changed: nothing of the model is needed to say so.)
    assert!(refused(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"side": "inside"}})).contains("1 to 1000 openings, not 0"));
    assert!(
        refused(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"openings": [[-15, 0]]}})).contains("openings are for an inside contour")
    );
}

#[test]
fn a_setups_openings_are_listed_for_an_inside_contour() {
    use std::f64::consts::PI;
    // The plate (in the WCS X -50 to 50, Y -40 to 40, its top at Z -1 and its bottom at Z -21)
    // with a window 40 x 30 through it about (-15, 0), a round pocket of 24, 8 deep, about
    // (30, 0), a hole of 10 through it about (30, 25), and a recess of 20 x 10, 5 up into its
    // underside about (-15, -30), which is no opening from above.
    let mut s = plate();
    boxed(&mut s, [15.0, 25.0, -5.0], [40.0, 30.0, 30.0], "cut");
    run(&mut s, "solid.cylinder", json!({"diameter": 24, "height": 8, "base": [80, 40, 12], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    run(&mut s, "solid.cylinder", json!({"diameter": 10, "height": 30, "base": [80, 65, -5], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    boxed(&mut s, [25.0, 5.0, -1.0], [20.0, 10.0, 6.0], "cut");
    let found = run(&mut s, "cam.setup.openings", json!({"id": "setup-1"}));
    let openings = found["openings"].as_array().unwrap();
    // The ones through the part first, the largest first, then the pocket.
    assert_eq!(openings.len(), 3, "{found:#}");
    let (window, hole, pocket) = (&openings[0], &openings[1], &openings[2]);
    assert_eq!((window["through"].clone(), window["bottom"].clone(), window["depth"].clone()), (json!(true), json!(-21.0), json!(20.0)), "{window}");
    assert_eq!((window["size"].clone(), window["diameter"].clone(), window["islands"].clone()), (json!([40.0, 30.0]), json!(null), json!(0)));
    assert!(about(&window["area"], 1200.0) && window["widest"].as_f64().is_some_and(|widest| (widest - 30.0).abs() < 0.05), "{window}");
    let inside = |opening: &Value, x: [f64; 2], y: [f64; 2]| {
        let (px, py) = (opening["at"][0].as_f64().unwrap(), opening["at"][1].as_f64().unwrap());
        px > x[0] && px < x[1] && py > y[0] && py < y[1]
    };
    assert!(inside(window, [-35.0, 5.0], [-15.0, 15.0]), "{window}");
    assert_eq!((hole["through"].clone(), hole["diameter"].clone(), hole["bottom"].clone()), (json!(true), json!(10.0), json!(-21.0)), "{hole}");
    assert!(inside(hole, [25.0, 35.0], [20.0, 30.0]) && about(&hole["area"], 25.0 * PI), "{hole}");
    assert_eq!(
        (pocket["through"].clone(), pocket["diameter"].clone(), pocket["bottom"].clone()),
        (json!(false), json!(24.0), json!(-9.0)),
        "{pocket}"
    );
    assert_eq!((pocket["depth"].clone(), pocket["size"].clone()), (json!(8.0), json!([24.0, 24.0])), "{pocket}");
    // Each is named by its point: a contour round the inside of every one of them is worked out
    // as it is listed, to the bottom it says (with a 6 mm end mill, tool 3, which goes into the
    // hole of 10).
    let mut small = mill10();
    (small["number"], small["name"]) = (json!(3), json!("6 mm flat end mill"));
    (small["geometry"]["diameter"], small["geometry"]["shank_diameter"]) = (json!(6.0), json!(6.0));
    run(&mut s, "cam.tool.create", json!({"tool": small}));
    for (k, opening) in openings.iter().enumerate() {
        let made = run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 3, "contour": {"side": "inside", "openings": [opening["at"]]}}));
        let t = run(&mut s, "cam.op.toolpath", json!({"id": made["id"]}));
        let levels = t["numbers"]["levels"].as_array().unwrap();
        assert!(about(levels.last().unwrap(), opening["bottom"].as_f64().unwrap()), "opening {k}: {}", t["numbers"]);
    }
    // No such setup; and a design with nothing to machine says so.
    assert!(refused(&mut s, "cam.setup.openings", json!({"id": "setup-9"})).contains("no setup 'setup-9'"));
    assert!(refused(&mut s, "cam.setup.openings", json!({"id": "setup-1", "z": 1})).contains("z"));
}

#[test]
fn a_chamfer_of_all_the_edges_breaks_the_rims_of_the_openings_read_from_the_model() {
    use std::f64::consts::PI;
    // The plate (in the WCS X -50 to 50, Y -40 to 40, its top at Z -1) with a pocket 50 x 40,
    // 8 deep, about (-15, 0) and a round post of 12 standing in its middle; a hole of 8 through
    // the plate about (30, 20); and a recess 10 x 10, 1 deep, about (30, -20), which is not as
    // deep as the tool's point goes (1.5 below the edge).
    let mut s = plate();
    boxed(&mut s, [10.0, 20.0, 12.0], [50.0, 40.0, 8.0], "cut");
    run(&mut s, "solid.cylinder", json!({"diameter": 12, "height": 8, "base": [35, 40, 12], "axis": "Z", "operation": "join", "targets": ["Plate"]}));
    run(&mut s, "solid.cylinder", json!({"diameter": 8, "height": 30, "base": [80, 60, -5], "axis": "Z", "operation": "cut", "targets": ["Plate"]}));
    boxed(&mut s, [75.0, 15.0, 19.0], [10.0, 10.0, 1.0], "cut");
    run(&mut s, "cam.tool.create", json!({"tool": chamfer10()}));
    // The outside edge alone, as before: 360 round the plate and a quarter turn of 1 at each
    // corner; the post in the pocket is said to be left sharp.
    assert_eq!(run(&mut s, "cam.op.chamfer", json!({"setup": "setup-1", "tool": 6}))["id"], "op-1");
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    assert!(about(&t["numbers"]["round"], 360.0 + 2.0 * PI), "{}", t["numbers"]);
    assert!(
        t["notes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|note| note.as_str().is_some_and(|note| note.contains("left sharp: a chamfer goes round the outside only")))
    );
    // All the edges: 1 inside the pocket's wall (48 x 38: 172), 1 outside the post (a circle of
    // radius 7), 1 inside the hole (a circle of radius 3); the recess is left sharp, and said.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "chamfer": {"edges": "all"}}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert!(about(&n["round"], 360.0 + 2.0 * PI + 172.0 + 14.0 * PI + 6.0 * PI), "{n}");
    assert_eq!((n["entries"].as_array().map(Vec::len), n["z"].as_f64()), (Some(4), Some(-2.5)), "{n}");
    let notes: Vec<&str> = t["notes"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert_eq!(notes, ["1 opening(s) with something in the way of the tool's point, which goes 1.50 mm below the edge, left sharp"]);
    // The simulation takes every one of them as meant: nothing is cut into the part.
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.2}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    // What it keeps is what it was given, and a word that is no choice of edges is refused.
    let kept = run(&mut s, "cam.data", json!({}))["data"]["setups"][0]["operations"][0]["operation"]["chamfer"].clone();
    assert_eq!(kept["edges"], "all");
    assert!(refused(&mut s, "cam.op.edit", json!({"id": "op-1", "chamfer": {"edges": "inside"}})).contains("unknown variant"));
}

#[test]
fn operations_that_follow_one_another_with_one_tool_keep_it_in_the_program() {
    // The block faced and then contoured with the same 20 mm tool (T4), then drilled (T3).
    let mut s = milling();
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    for post in ["haas_mill", "fanuc_mill", "rs274"] {
        let posted = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": post}));
        let text = posted["text"].as_str().unwrap();
        let lines = |word: &str| text.lines().filter(|line| line.split_whitespace().any(|w| w == word)).count();
        // One change to T4 for its two operations and one to T3; the spindle is started and
        // stopped, and the coolant turned on and off, once for each tool.
        assert_eq!((lines("M6"), text.matches("T4 M6").count(), text.matches("T3 M6").count()), (2, 1, 1), "{post}:\n{text}");
        assert_eq!((lines("M3"), lines("M5"), lines("M8"), lines("M9")), (2, 2, 2, 2), "{post}:\n{text}");
    }
    // The contour's toolpath is the same toolpath: it is the program that leaves the change out.
    let contour = run(&mut s, "cam.op.toolpath", json!({"id": "op-2"}));
    assert_eq!(contour["toolpath"]["moves"][0], json!({"kind": "tool_change", "tool": 4}));
    // The simulation and the setup sheet are as they were: each operation with its own tool.
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert_eq!((sheet["tools"].as_u64(), sheet["operations"].as_u64()), (Some(2), Some(3)));
}

#[test]
fn a_simulation_is_cut_any_part_of_the_way_and_says_where_the_tool_is() {
    // The block faced and contoured with the 20 mm tool (T4), and three holes pecked with the
    // 10 mm drill (T3): the program is the three toolpaths one after another.
    let mut s = milling();
    run(&mut s, "cam.op.face", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 4}));
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 3, "drill": three_holes()}));
    let lengths: Vec<f64> = ["op-1", "op-2", "op-3"]
        .iter()
        .map(|id| {
            let t = run(&mut s, "cam.op.toolpath", json!({"id": id}));
            t["feed_length"].as_f64().unwrap() + t["rapid_length"].as_f64().unwrap()
        })
        .collect();
    let whole = run(&mut s, "cam.simulate", json!({"setup": "setup-1"}));
    let length = whole["length"].as_f64().unwrap();
    // One after another they are as long as each alone and the two ways across between them,
    // at the clearance height: each less than the block's own diagonal and its stock (75 mm).
    let between = length - lengths.iter().sum::<f64>();
    assert!(between > 0.0 && between < 150.0 && whole["along"] == whole["length"] && whole["tool"].is_null(), "{between} in {whole:#}");
    let at = |s: &mut Session, along: f64| run(s, "cam.simulate", json!({"setup": "setup-1", "along": along}));
    // At the start nothing is cut, and the tool is the first operation's, where its toolpath
    // starts.
    let start = at(&mut s, 0.0);
    assert_eq!(
        (start["removed"].as_f64(), start["tool"]["operation"].as_str(), start["tool"]["number"].as_u64()),
        (Some(0.0), Some("op-1"), Some(4))
    );
    assert_eq!(start["tool"]["at"], run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}))["toolpath"]["start"]);
    // Half way through the contour: the facing is done whole, the drill has done nothing, the
    // tool is the contour's, and the contour has removed some of what it will.
    let part = at(&mut s, lengths[0] + 0.5 * lengths[1]);
    assert_eq!(part["operations"][0]["removed"], whole["operations"][0]["removed"]);
    assert_eq!((part["operations"][2]["removed"].as_f64(), part["tool"]["operation"].as_str()), (Some(0.0), Some("op-2")));
    let contoured = (part["operations"][1]["removed"].as_f64().unwrap(), whole["operations"][1]["removed"].as_f64().unwrap());
    assert!(contoured.0 > 0.0 && contoured.0 < contoured.1 && part["tool"]["minutes"].as_f64().is_some_and(|minutes| minutes > 0.0), "{contoured:?}");
    // The further along, the more is removed, never less; all the way is the simulation,
    // number for number.
    let mut removed = 0.0;
    for k in 0..=20 {
        let now = at(&mut s, length * f64::from(k) / 20.0)["removed"].as_f64().unwrap();
        assert!(now >= removed - 1e-9, "{k}: {now} after {removed}");
        removed = now;
    }
    let end = at(&mut s, length);
    for key in ["removed", "over_part", "beside_part", "gouge_count", "gouges", "rapid_hit_count", "body_hit_count", "operations"] {
        assert_eq!(end[key], whole[key], "{key}");
    }
    assert!(end["tool"].is_null());
    // Past the end is the end, before the start the start; what is no number is refused.
    assert_eq!(at(&mut s, 1e9)["removed"], whole["removed"]);
    assert_eq!(at(&mut s, -5.0)["removed"].as_f64(), Some(0.0));
    let why = refused(&mut s, "cam.simulate", json!({"setup": "setup-1", "along": "half"}));
    assert!(why.contains("invalid type: string \"half\", expected f64"), "{why}");
}

#[test]
fn a_contour_in_passes_cuts_a_shoulder_down_to_its_floor() {
    use std::f64::consts::PI;
    // The plate cut down to 12 thick but for a boss 60 x 40 left standing in its middle, 8
    // high: in the WCS the boss is X -30 to 30, Y -20 to 20, its top at Z -1, and the step
    // round it has its floor at Z -9, 20 wide (21 with the stock's millimetre, 29.7 at the
    // corners). Tool 2 is the 10 mm end mill.
    let mut s = plate();
    boxed(&mut s, [0.0, 0.0, 12.0], [100.0, 80.0, 8.0], "cut");
    boxed(&mut s, [20.0, 20.0, 12.0], [60.0, 40.0, 8.0], "join");
    // Round the boss down to the step's floor: the outline read there is the boss's own. One
    // pass reaches 10 from it; six, four tenths of the tool apart, reach 30: past the corners.
    assert_eq!(run(&mut s, "cam.op.contour", json!({"setup": "setup-1", "tool": 2, "contour": {"bottom": -9, "passes": 6}}))["id"], "op-1");
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let n = &t["numbers"];
    assert_eq!(
        (n["passes"].as_u64(), n["stepover"].as_f64(), n["offset"].as_f64(), n["levels"].clone()),
        (Some(6), Some(4.0), Some(5.0), json!([-4.5, -9.0])),
        "{n}"
    );
    // The six rounds are 5, 9, 13, 17, 21 and 25 out from the boss: 200 and a turn of each.
    assert!(about(&n["round"], 1200.0 + 180.0 * PI) && n["entries"].as_array().is_some_and(|entries| entries.len() == 6), "{n}");
    // The simulation: nothing of the part is cut into, and all that stands above it afterwards
    // is the boss's own millimetre of stock, 60 x 40.
    let sim = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.25}));
    assert_eq!((sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64()), (Some(0), Some(0)), "{sim:#}");
    assert!(about(&sim["over_part"], 2400.0), "{}", sim["over_part"]);
    // With one pass most of the step still stands.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "contour": {"bottom": -9}}));
    let one = run(&mut s, "cam.simulate", json!({"setup": "setup-1", "cell": 0.25}));
    assert!(one["over_part"].as_f64().is_some_and(|over| over > 2400.0 + 20_000.0), "{}", one["over_part"]);
    // Tabs go with one pass, and a stepover with more than one.
    let edit = |s: &mut Session, contour: Value| refused(s, "cam.op.edit", json!({"id": "op-1", "contour": contour}));
    assert!(edit(&mut s, json!({"passes": 2, "tabs": {"count": 4}})).contains("tabs go with one pass"));
    assert!(edit(&mut s, json!({"stepover": 3})).contains("give the number of passes"));
    assert!(edit(&mut s, json!({"passes": 2, "stepover": 12})).contains("the stepover must be 0.001 mm to the tool's diameter (10 mm)"));
}

#[test]
fn a_reamed_hole_is_fed_in_and_fed_back_out_and_posts_as_g85() {
    // The block with a 10 mm reamer (tool 7) beside its drill: two holes reamed 12 deep from
    // the block's top at Z -1, from R 5.
    let mut s = drilling();
    let mut reamer = drill10();
    (reamer["number"], reamer["name"], reamer["kind"]) = (json!(7), json!("10 mm reamer"), json!("reamer"));
    reamer["geometry"]["tip_angle"] = json!(0.0);
    run(&mut s, "cam.tool.create", json!({"tool": reamer}));
    let reamed = |dwell: f64| json!({"holes": [[-20, -10], [20, -10]], "depth": 12, "cycle": {"type": "ream", "dwell": dwell}});
    run(&mut s, "cam.op.drill", json!({"setup": "setup-1", "tool": 7, "drill": reamed(0.0)}));
    let t = run(&mut s, "cam.op.toolpath", json!({"id": "op-1"}));
    let holes: Vec<&Value> = t["toolpath"]["moves"].as_array().unwrap().iter().filter(|m| m["kind"] == "drill").collect();
    assert!(holes.len() == 2 && holes.iter().all(|m| m["cycle"] == json!({"type": "ream", "dwell": 0.0})), "{holes:?}");
    // In and back out at the feed: twice the 18 mm from R 5 to Z -13, for each hole.
    assert!(about(&t["feed_length"], 72.0), "{t}");
    // Haas and NIST write it as G85; the Fanuc-style post, which has no cycle for it yet,
    // writes the moves themselves and says so.
    for post in ["haas_mill", "rs274"] {
        let text = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": post}))["text"].as_str().unwrap().to_string();
        assert!(text.contains("G99 G85 X-20. Y-10. Z-13. R5. F") && text.contains("\nX20. Y-10.\nG80"), "{post}:\n{text}");
    }
    let plain = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "fanuc_mill"}));
    assert!(!plain["text"].as_str().unwrap().contains("G85"), "{}", plain["text"]);
    assert!(
        plain["notes"].as_array().unwrap().iter().any(|n| n.as_str().is_some_and(|n| n.contains("plain moves") && n.contains("G85"))),
        "{}",
        plain["notes"]
    );
    // The setup sheet calls it what it is.
    let sheet = run(&mut s, "cam.setup.sheet", json!({"setup": "setup-1"}));
    assert!(sheet["html"].as_str().unwrap().contains("<td>Drill 1 <span class=\"note\">ream</span></td>"), "{}", sheet["html"]);
    // With a dwell at the bottom it is G89 on the Haas.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "drill": reamed(0.5)}));
    let text = run(&mut s, "cam.post", json!({"setup": "setup-1", "post": "haas_mill"}))["text"].as_str().unwrap().to_string();
    assert!(text.contains("G99 G89 X-20. Y-10. Z-13. R5. P0.500 F"), "{text}");
    // Drill's form is offered both tools, and told which one reams.
    let offered: Vec<(u32, bool)> = crate::cmd::manufacture_tools(&s.doc, "drill").iter().map(|t| (t.number, t.reams)).collect();
    assert_eq!(offered, vec![(3, false), (7, true)]);
    // A dwell that is no dwell is refused when the toolpath is worked out, as a drilled hole's is.
    run(&mut s, "cam.op.edit", json!({"id": "op-1", "drill": reamed(-1.0)}));
    assert!(refused(&mut s, "cam.op.toolpath", json!({"id": "op-1"})).contains("dwell must be 0 to"));
}
