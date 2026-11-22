//! The CAM examples (`examples/cam/`) must run from their scripts and post G-code that reads back
//! as what was written (the CAM work's P0 demo).

use serde_json::Value;
use solidcraft_engine::{Session, io};

#[test]
fn the_p0_outline_posts_on_every_post_and_reads_back() {
    let script: Value = serde_json::from_str(include_str!("../../../examples/cam/p0-outline.json")).unwrap();
    let mut session = Session::default();
    let results = session.run_script(&script).unwrap();
    let commands: Vec<&str> = script["commands"].as_array().unwrap().iter().map(|c| c["command"].as_str().unwrap()).collect();
    let posted: Vec<&Value> = results.iter().zip(&commands).filter(|(_, c)| **c == "cam.post").map(|(r, _)| r).collect();
    assert_eq!(posted.len(), 4);
    for p in &posted {
        let text = p["text"].as_str().unwrap();
        // 13 moves, each read back within half the output resolution (cam.post refuses otherwise).
        assert_eq!(p["moves"], 13, "{text}");
        assert!(text.contains("UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM") || text.contains("unverified output from solidcraft cam"), "{text}");
        assert!(text.contains("G3 X33. Y-13.") || text.contains("G3 X1.2992 Y-0.5118"), "the quarter-round corner is an arc: {text}");
    }
    // The Fanuc- and Haas-style posts go to machine Z home before the tool change and at the end.
    assert_eq!((posted[1]["machine_retracts"].as_u64(), posted[2]["machine_retracts"].as_u64()), (Some(2), Some(2)));
    assert!(posted[1]["text"].as_str().unwrap().starts_with("%\nO1001 "));
    assert!(posted[2]["text"].as_str().unwrap().starts_with("%\nO01001 "));
    // In inches the Haas post writes 4 places and G20, and reads back within 0.00127 mm.
    let inch = posted[3]["text"].as_str().unwrap();
    assert!(inch.contains("G20") && inch.contains("\nG0 X-1.2992 Y-0.9055\nG43 H1 Z0.1969\n"), "{inch}");
    assert!((posted[3]["tolerance_mm"].as_f64().unwrap() - 0.00127).abs() < 1e-12);

    // The design keeps the setup, tool and operation, not the toolpaths' G-code.
    let saved = io::write_design_container(&session.doc).unwrap();
    let mut reopened = Session::new(io::read_design(&saved).unwrap());
    let data = reopened.execute("cam.data", &serde_json::json!({})).unwrap();
    assert_eq!((data["setups"].as_u64(), data["tools"].as_u64(), data["operations"].as_u64()), (Some(1), Some(1), Some(1)));
    let again = reopened.execute("cam.post", &serde_json::json!({"setup": "setup-1", "post": "fanuc_mill", "program_number": 1001})).unwrap();
    assert_eq!(again["text"], posted[1]["text"], "a reopened design posts the same program");
}
