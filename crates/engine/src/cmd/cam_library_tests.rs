use serde_json::{Value, json};

use crate::Session;

fn tool(number: u32, diameter: f64) -> Value {
    json!({
        "number": number, "name": format!("{diameter} mm flat"), "kind": "flat_end_mill",
        "geometry": {"diameter": diameter, "corner_radius": 0.0, "flute_length": 18.0, "shoulder_length": 0.0, "overall_length": 60.0,
                     "shank_diameter": diameter, "tip_angle": 0.0, "tip_diameter": 0.0, "thread_pitch": 0.0},
        "flutes": 3, "holder": {"stick_out": 25.0, "segments": []}, "length_offset": number, "diameter_offset": number,
        "coolant": "flood", "cutting_data": [], "overrides": {"rpm": null, "feed": null}
    })
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

/// A fresh folder for one test's files.
fn folder(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("solidcraft-cam-library-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn statuses(listed: &Value) -> Vec<(u64, String)> {
    listed["tools"].as_array().unwrap().iter().map(|t| (t["number"].as_u64().unwrap(), t["status"].as_str().unwrap().to_string())).collect()
}

#[test]
fn a_library_exports_reads_back_and_imports_into_another_design() {
    let dir = folder("round-trip");
    let file = dir.join("vf4.json").to_string_lossy().to_string();
    let mut shop = Session::default();
    run(&mut shop, "cam.tool.create", json!({"tool": tool(1, 6.0)}));
    run(&mut shop, "cam.tool.create", json!({"tool": tool(2, 10.0)}));
    let exported = run(&mut shop, "cam.library.export", json!({"path": file, "name": "VF-4 tools"}));
    assert_eq!(exported["tools"], 2);
    // The file is a readable tool library.
    let text = std::fs::read_to_string(&file).unwrap();
    assert!(text.contains("\"name\": \"VF-4 tools\"") && text.contains("\"number\": 2"));
    assert_eq!(statuses(&run(&mut shop, "cam.library.read", json!({"path": file}))), vec![(1, "same".into()), (2, "same".into())]);
    // Another design takes one of them.
    let mut other = Session::default();
    assert_eq!(statuses(&run(&mut other, "cam.library.read", json!({"path": file}))), vec![(1, "new".into()), (2, "new".into())]);
    let imported = run(&mut other, "cam.library.import", json!({"path": file, "numbers": [2]}));
    assert_eq!(imported["added"], json!([2]));
    let data = run(&mut other, "cam.data", json!({}));
    assert_eq!(data["data"]["tools"][0]["geometry"]["diameter"], 10.0);
    assert_eq!(data["tools"], 1);
    // Exporting never changes the design.
    assert_eq!(shop.undo.len(), 2);
}

#[test]
fn a_library_that_differs_is_shown_and_updates_only_when_asked() {
    let dir = folder("differs");
    let file = dir.join("lib.json").to_string_lossy().to_string();
    let mut library = Session::default();
    run(&mut library, "cam.tool.create", json!({"tool": tool(1, 8.0)}));
    run(&mut library, "cam.tool.create", json!({"tool": tool(3, 3.0)}));
    run(&mut library, "cam.library.export", json!({"path": file}));
    let mut design = Session::default();
    run(&mut design, "cam.tool.create", json!({"tool": tool(1, 6.0)}));
    assert_eq!(statuses(&run(&mut design, "cam.library.read", json!({"path": file}))), vec![(1, "differs".into()), (3, "new".into())]);
    // Without replace a different tool 1 is refused, and nothing is imported.
    let undo = design.undo.len();
    assert!(refused(&mut design, "cam.library.import", json!({"path": file})).contains("tool 1 differs"));
    assert_eq!(design.undo.len(), undo);
    assert_eq!(run(&mut design, "cam.data", json!({}))["tools"], 1);
    // With replace it is the "library differs: update?" answer.
    let imported = run(&mut design, "cam.library.import", json!({"path": file, "replace": true}));
    assert_eq!((imported["added"].clone(), imported["updated"].clone()), (json!([3]), json!([1])));
    assert_eq!(run(&mut design, "cam.data", json!({}))["data"]["tools"][0]["geometry"]["diameter"], 8.0);
    // And it is one undo step.
    design.execute("edit.undo", &json!({})).unwrap();
    assert_eq!(run(&mut design, "cam.data", json!({}))["data"]["tools"][0]["geometry"]["diameter"], 6.0);
}

#[test]
fn files_are_checked_and_never_overwritten_by_surprise() {
    let dir = folder("files");
    let file = dir.join("lib.json").to_string_lossy().to_string();
    let mut s = Session::default();
    run(&mut s, "cam.tool.create", json!({"tool": tool(1, 6.0)}));
    run(&mut s, "cam.library.export", json!({"path": file}));
    assert!(refused(&mut s, "cam.library.export", json!({"path": file})).contains("could not write the tool library"));
    run(&mut s, "cam.library.export", json!({"path": file, "overwrite": true}));
    let txt = dir.join("lib.txt").to_string_lossy().to_string();
    assert!(refused(&mut s, "cam.library.export", json!({"path": txt})).contains(".json"));
    let missing = dir.join("none.json").to_string_lossy().to_string();
    assert!(refused(&mut s, "cam.library.read", json!({"path": missing})).contains("could not read"));
    assert!(refused(&mut s, "cam.library.import", json!({"path": file, "numbers": [9]})).contains("no tool 9"));
    // Hostile files are refused when read.
    for (name, text) in [
        ("broken.json", "{ not json"),
        ("format.json", r#"{"format": 9, "name": "x", "tools": []}"#),
        ("extra.json", r#"{"format": 1, "name": "x", "tools": [], "run": "rm -rf"}"#),
        ("twice.json", &format!(r#"{{"format": 1, "name": "x", "tools": [{}, {}]}}"#, tool(4, 6.0), tool(4, 8.0))),
    ] {
        let path = dir.join(name);
        std::fs::write(&path, text).unwrap();
        let e = refused(&mut s, "cam.library.read", json!({"path": path.to_string_lossy()}));
        assert!(e.contains("tool library"), "{name}: {e}");
    }
}
