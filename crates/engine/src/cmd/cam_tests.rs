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
        "\nS3183 M3\nM8\nG43 H3 G0 X-20. Y-10. Z25.\nG99 G83 X-20. Y-10. Z-11. R5. Q3. F636.6\nX20. Y-10.\nX20. Y10.\nG80\nG0 Z25.\nM9\nM5\n";
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
    let join = "\nX-43.\nG0 Z5.\nZ25.\nM9\nM5\nG28 G91 Z0.\nG90\nT3 M6\nS3183 M3\nM8\nG43 H3 G0 X-20. Y-10. Z25.\nG99 G83 X-20. Y-10. Z-11. R5. Q3. F636.6\n";
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
        ("cam.post", json!({"setup": "setup-1", "post": "rs274"}), "not a command that adds a drill or face operation"),
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
    let preview = |s: &Session, params: Value| crate::cmd::manufacture_setup_preview(s, &params);
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
        let measures = matches!(spec.id, "cam.setup.measure" | "cam.op.toolpath" | "cam.post");
        assert_eq!(spec.reads_model, measures, "{}", spec.id);
    }
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
