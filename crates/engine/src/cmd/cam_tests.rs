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
