//! The CAM examples (`examples/cam/`) must run from their scripts and post G-code that reads back
//! as what was written (the CAM work's P0 demo), and the P1 demo plate must come out of its stock
//! with nothing cut into (the P1 demo), whatever its numbers.

use serde_json::{Value, json};
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

/// The numbers of the P1 demo plate, mm: a plate with a pocket round an island, holes through
/// its corners and a row of tapped holes, cut out of its stock with tabs and its top edge broken.
/// Each can be varied (decision 0002 of the CAM work: a demo part is a script whose dimensions
/// and feature counts vary), within what fits: the test's own plates do.
#[derive(Clone, Debug)]
struct Plate {
    length: f64,
    width: f64,
    thickness: f64,
    /// The pocket's length, width and depth, and where its middle is along X.
    pocket: [f64; 3],
    pocket_x: f64,
    /// The island's diameter (it stands in the pocket's middle, as high as the plate); 0 for none.
    island: f64,
    /// How many of the four corners have a hole of 6 through them.
    corner_holes: usize,
    /// How many M6 x 1 holes are in the row between the pocket and the plate's far end.
    tapped: usize,
    tabs: u32,
    /// How wide the chamfer on the top outside edge is.
    chamfer: f64,
}

impl Default for Plate {
    fn default() -> Self {
        Plate {
            length: 120.0,
            width: 80.0,
            thickness: 16.0,
            pocket: [70.0, 44.0, 8.0],
            pocket_x: -10.0,
            island: 18.0,
            corner_holes: 4,
            tapped: 2,
            tabs: 4,
            chamfer: 0.5,
        }
    }
}

/// How much stock there is round the plate and on top of it, mm.
const STOCK_ROUND: f64 = 12.0;
const STOCK_ON_TOP: f64 = 1.0;

impl Plate {
    fn corner_holes(&self) -> Vec<[f64; 2]> {
        let (x, y) = (0.5 * self.length - 8.0, 0.5 * self.width - 8.0);
        [[-x, -y], [x, -y], [x, y], [-x, y]].into_iter().take(self.corner_holes).collect()
    }

    fn tapped_holes(&self) -> Vec<[f64; 2]> {
        // Midway between the pocket's far wall and the plate's far end, 30 apart.
        let x = 0.5 * (self.pocket_x + 0.5 * self.pocket[0] + 0.5 * self.length);
        (0..self.tapped).map(|k| [x, (k as f64 - 0.5 * (self.tapped as f64 - 1.0)) * 30.0]).collect()
    }

    /// The plate's volume, from its own numbers.
    fn volume(&self) -> f64 {
        let [pl, pw, pd] = self.pocket;
        let round = |d: f64| std::f64::consts::FRAC_PI_4 * d * d;
        self.length * self.width * self.thickness
            - (pl * pw - round(self.island)) * pd
            - self.corner_holes as f64 * round(6.0) * self.thickness
            - self.tapped as f64 * round(5.0) * self.thickness
    }
}

/// A tool for the demo: `kind` and `diameter`, with `flutes` flutes `flute_length` long, sticking
/// `stick_out` out of a plain holder, cutting aluminium at `speed` m/min and `chip` mm a tooth.
fn demo_tool(
    number: u32,
    name: &str,
    kind: &str,
    diameter: f64,
    flutes: u32,
    flute_length: f64,
    stick_out: f64,
    tip_degrees: f64,
    pitch: f64,
    speed: f64,
    chip: f64,
) -> Value {
    json!({
        "number": number, "name": name, "kind": kind,
        "geometry": {"diameter": diameter, "corner_radius": 0.0, "flute_length": flute_length, "shoulder_length": 0.0, "overall_length": stick_out + 40.0,
                     "shank_diameter": diameter, "tip_angle": tip_degrees.to_radians(), "tip_diameter": 0.0, "thread_pitch": pitch},
        "flutes": flutes,
        "holder": {"stick_out": stick_out, "segments": [{"length": 35.0, "lower_diameter": 32.0, "upper_diameter": 32.0}]},
        "length_offset": number, "diameter_offset": number, "coolant": "flood",
        "cutting_data": [{"material": "aluminium", "surface_speed": speed, "chip_load": chip, "plunge_factor": 0.5, "ramp_factor": 0.5}],
        "overrides": {"rpm": null, "feed": null}
    })
}

/// The demo plate's script: the model, seven tools, one setup, and its operations in the order
/// a shop would run them (face, pocket, drill, tap, cut out on tabs, chamfer), then the checks
/// and the programs. The plate is about the origin, its bottom at Z 0; the work origin is the
/// middle of the stock's top, so X and Y are the same in the model and in the program.
fn p1_plate(p: &Plate) -> Value {
    let [pl, pw, pd] = p.pocket;
    let top = p.thickness;
    let mut commands = vec![json!({"command": "solid.box", "params": {
        "length": p.length, "width": p.width, "height": top, "corner": [-0.5 * p.length, -0.5 * p.width, 0.0], "body_name": "Plate"}})];
    let mut shape = |command: &str, params: Value| commands.push(json!({"command": command, "params": params}));
    let through =
        |d: f64, at: &[f64; 2]| json!({"diameter": d, "height": top + 10.0, "base": [at[0], at[1], -5.0], "operation": "cut", "targets": ["Plate"]});
    shape(
        "solid.box",
        json!({"length": pl, "width": pw, "height": pd, "corner": [p.pocket_x - 0.5 * pl, -0.5 * pw, top - pd], "operation": "cut", "targets": ["Plate"]}),
    );
    if p.island > 0.0 {
        shape(
            "solid.cylinder",
            json!({"diameter": p.island, "height": pd, "base": [p.pocket_x, 0.0, top - pd], "operation": "join", "targets": ["Plate"]}),
        );
    }
    for at in &p.corner_holes() {
        shape("solid.cylinder", through(6.0, at));
    }
    for at in &p.tapped_holes() {
        shape("solid.cylinder", through(5.0, at));
    }
    for tool in [
        demo_tool(1, "20 mm flat end mill", "flat_end_mill", 20.0, 4, 30.0, 45.0, 0.0, 0.0, 300.0, 0.06),
        demo_tool(2, "8 mm flat end mill", "flat_end_mill", 8.0, 3, 20.0, 30.0, 0.0, 0.0, 250.0, 0.04),
        demo_tool(3, "10 mm flat end mill", "flat_end_mill", 10.0, 3, 25.0, 35.0, 0.0, 0.0, 250.0, 0.05),
        demo_tool(4, "6 mm drill", "drill", 6.0, 2, 40.0, 50.0, 118.0, 0.0, 80.0, 0.08),
        demo_tool(5, "5 mm drill (M6 tap drill)", "drill", 5.0, 2, 40.0, 50.0, 118.0, 0.0, 80.0, 0.07),
        demo_tool(6, "M6 x 1 tap", "tap", 6.0, 3, 20.0, 40.0, 0.0, 1.0, 15.0, 0.1),
        demo_tool(7, "10 mm 90 degree chamfer mill", "chamfer_mill", 10.0, 4, 10.0, 30.0, 90.0, 0.0, 150.0, 0.03),
    ] {
        shape("cam.tool.create", json!({"tool": tool}));
    }
    let round = STOCK_ROUND;
    shape(
        "cam.setup.create",
        json!({"name": "Op 1 (top)", "material": "aluminium",
        "stock": {"kind": "relative_box", "offsets": {"x_min": round, "x_max": round, "y_min": round, "y_max": round, "z_min": 0.0, "z_max": STOCK_ON_TOP}}}),
    );
    let setup = "setup-1";
    shape("cam.op.face", json!({"setup": setup, "tool": 1}));
    // A point on the pocket's floor: 6 in from its near wall, clear of the island.
    shape("cam.op.pocket", json!({"setup": setup, "tool": 2, "pocket": {"pockets": [[p.pocket_x - 0.5 * pl + 6.0, 0.0]]}}));
    let drilled =
        |holes: Vec<[f64; 2]>| json!({"holes": holes, "depth": top + 1.0, "depth_to": "full_diameter", "cycle": {"type": "peck", "peck": 3.0}});
    if p.corner_holes > 0 {
        shape("cam.op.drill", json!({"setup": setup, "tool": 4, "name": "Drill 6 through", "drill": drilled(p.corner_holes())}));
    }
    if p.tapped > 0 {
        shape("cam.op.drill", json!({"setup": setup, "tool": 5, "name": "Drill 5 for M6", "drill": drilled(p.tapped_holes())}));
        shape(
            "cam.op.drill",
            json!({"setup": setup, "tool": 6, "name": "Tap M6", "drill": {"holes": p.tapped_holes(), "depth": top + 3.0, "cycle": {"type": "tap", "pitch": 1.0}}}),
        );
    }
    shape("cam.op.contour", json!({"setup": setup, "tool": 3, "contour": {"tabs": {"count": p.tabs}}}));
    shape("cam.op.chamfer", json!({"setup": setup, "tool": 7, "chamfer": {"width": p.chamfer}}));
    shape("cam.simulate", json!({"setup": setup, "cell": 0.2}));
    shape("cam.post", json!({"setup": setup, "post": "haas_mill", "program_number": 1101}));
    shape("cam.post", json!({"setup": setup, "post": "rs274"}));
    shape("cam.post", json!({"setup": setup, "post": "haas_mill", "units": "inch", "program_number": 1102}));
    shape("cam.setup.sheet", json!({"setup": setup}));
    json!({"commands": commands})
}

/// Runs a plate's script and checks what the phase asks of its demo part: the simulation finds
/// nothing cut into and no rapid through material, what is left of the stock is the plate (and
/// what was meant to stay beside it), and every program reads back as written.
fn cut_and_checked(p: &Plate) -> (Session, Vec<Value>, Vec<String>) {
    let script = p1_plate(p);
    let mut session = Session::default();
    let results = session.run_script(&script).unwrap_or_else(|e| panic!("{p:?}: {e}"));
    let commands: Vec<String> = script["commands"].as_array().unwrap().iter().map(|c| c["command"].as_str().unwrap().to_string()).collect();
    let of = |name: &str| results.iter().zip(&commands).filter(|(_, c)| c.as_str() == name).map(|(r, _)| r).collect::<Vec<&Value>>();

    let sim = of("cam.simulate")[0];
    let number = |v: &Value, key: &str| v[key].as_f64().unwrap_or_else(|| panic!("no {key} in {v}"));
    assert_eq!(
        (sim["gouge_count"].as_u64(), sim["rapid_hit_count"].as_u64(), sim["body_hit_count"].as_u64()),
        (Some(0), Some(0), Some(0)),
        "{p:?}: {sim:#}"
    );
    // Every operation is cut but the tapping, which a field of heights can't show: said, not hidden.
    for op in sim["operations"].as_array().unwrap() {
        let tapping = op["name"] == "Tap M6";
        assert_eq!(op["skipped"].is_null(), !tapping, "{op}");
        assert!(!tapping || op["skipped"].as_str().unwrap().contains("cuts a thread"), "{op}");
    }
    // What is left of the stock where the plate is, is the plate: the model's own volume from
    // its exact faces, less the chamfer the model leaves sharp (a triangle of the chamfer's
    // width round the top edge). As nearly as the cells can tell: within half a percent.
    let model = session.execute("inspect.center_of_mass", &json!({})).unwrap();
    let exact = number(&model, "volume");
    assert!((exact - p.volume()).abs() < 1e-6 * exact, "{p:?}: the model is {exact} mm3, its numbers give {}", p.volume());
    let chamfer = 0.5 * p.chamfer * p.chamfer * 2.0 * (p.length + p.width);
    let left = number(sim, "stock_volume") - number(sim, "removed") - number(sim, "over_part") - number(sim, "beside_part");
    assert!((left - (exact - chamfer)).abs() < 0.005 * exact, "{p:?}: {left} mm3 left where the plate is, and the plate is {exact} less {chamfer}");
    // Left above the plate: only what a round tool leaves in the pocket's four corners,
    // (1 - pi/4) of the square of its radius each, as deep as the pocket.
    let corners = 4.0 * (1.0 - std::f64::consts::FRAC_PI_4) * 16.0 * p.pocket[2];
    assert!(
        (number(sim, "over_part") - corners).abs() < 0.25 * corners,
        "{p:?}: {} mm3 above the plate, the corners are {corners}",
        number(sim, "over_part")
    );
    // Left beside it: the tabs at least (6 wide, 2 high, across the 10 mm slot).
    assert!(number(sim, "beside_part") > 0.8 * f64::from(p.tabs) * 6.0 * 2.0 * 10.0, "{p:?}: {}", number(sim, "beside_part"));

    let programs: Vec<String> = of("cam.post").iter().map(|r| r["text"].as_str().unwrap().to_string()).collect();
    assert_eq!(programs.len(), 3);
    (session, results, programs)
}

#[test]
fn the_p1_plate_is_cut_out_with_nothing_cut_into_and_posts() {
    let plate = Plate::default();
    let script = p1_plate(&plate);
    // The example on disk is this plate's script, for `solidcraft-cli run examples/cam/p1-plate.json`.
    // After a change to the script above, write it again: SOLIDCRAFT_WRITE_CAM_EXAMPLES=1.
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/cam/p1-plate.json");
    if std::env::var_os("SOLIDCRAFT_WRITE_CAM_EXAMPLES").is_some() {
        let mut text = Vec::new();
        let mut writer = serde_json::Serializer::with_formatter(&mut text, serde_json::ser::PrettyFormatter::with_indent(b" "));
        serde::Serialize::serialize(&script, &mut writer).unwrap();
        text.push(b'\n');
        std::fs::write(path, text).unwrap();
    }
    let on_disk: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    assert_eq!(on_disk, script, "examples/cam/p1-plate.json is not the script the test makes: write it again");

    let (mut session, results, programs) = cut_and_checked(&plate);
    let commands: Vec<&str> = script["commands"].as_array().unwrap().iter().map(|c| c["command"].as_str().unwrap()).collect();
    let of = |name: &str| results.iter().zip(&commands).filter(|(_, c)| **c == name).map(|(r, _)| r).collect::<Vec<&Value>>();

    // What each operation takes, from the plate's own numbers (cells of 0.2 mm).
    let sim = of("cam.simulate")[0];
    let removed: Vec<(&str, f64)> =
        sim["operations"].as_array().unwrap().iter().map(|op| (op["name"].as_str().unwrap(), op["removed"].as_f64().unwrap())).collect();
    let names: Vec<&str> = removed.iter().map(|(name, _)| *name).collect();
    assert_eq!(names, ["Face 1", "Pocket 2", "Drill 6 through", "Drill 5 for M6", "Tap M6", "Contour 6", "Chamfer 7"]);
    let pi = std::f64::consts::PI;
    let near = |k: usize, exact: f64, share: f64| {
        assert!((removed[k].1 - exact).abs() <= share * exact, "{}: {} mm3, not {exact}", removed[k].0, removed[k].1)
    };
    // Facing: the millimetre on top of the stock, 144 x 104.
    near(0, 144.0 * 104.0, 1e-9);
    // The pocket: 70 x 44 less the island's 18 across, 8 deep, less what the 8 mm tool leaves in
    // the four corners.
    near(1, (70.0 * 44.0 - pi * 81.0) * 8.0 - 4.0 * (1.0 - pi / 4.0) * 16.0 * 8.0, 0.01);
    // Four holes of 6 and two of 5 through the 16 the facing left.
    near(2, 4.0 * pi * 9.0 * 16.0, 0.08);
    near(3, 2.0 * pi * 6.25 * 16.0, 0.08);
    assert_eq!(removed[4].1, 0.0);
    // The slot round the plate: the tool's 10 mm along a path 5 outside the wall, 400 round
    // and the four corners' quarter turns, 16 deep; less the four tabs, more where it comes in
    // through the stock's rim.
    near(5, (400.0 + 10.0 * pi) * 10.0 * 16.0, 0.02);
    // The chamfer: a triangle half a millimetre each way, 400 round.
    near(6, 0.125 * 400.0, 0.3);
    let contour = session.execute("cam.op.toolpath", &json!({"id": "op-6"})).unwrap();
    assert_eq!(contour["numbers"]["tabs"].as_array().map(Vec::len), Some(4), "{}", contour["numbers"]);

    // The programs: every tool is called, the holes are pecked and tapped by cycles, and each
    // program read back as its moves (cam.post refuses otherwise).
    let haas = &programs[0];
    assert!(haas.starts_with("%\nO01101 ") && haas.trim_end().ends_with('%'), "{}", &haas[..200.min(haas.len())]);
    for tool in 1..=7 {
        assert!(haas.contains(&format!("\nT{tool} M6 (")), "no tool change to T{tool}");
    }
    // The program says what it is doing: the tools it takes, in the order it takes them, then
    // each operation's name where it starts. A parenthesis in a tool's name is a space.
    let said: Vec<&str> = haas.lines().filter(|line| line.starts_with('(')).collect();
    assert_eq!(
        said,
        [
            "(UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM)",
            "(PROVE OUT FIRST: AIR CUT, SINGLE BLOCK)",
            "(T1 20 MM FLAT END MILL - DIA 20.)",
            "(T2 8 MM FLAT END MILL - DIA 8.)",
            "(T4 6 MM DRILL - DIA 6.)",
            "(T5 5 MM DRILL M6 TAP DRILL - DIA 5.)",
            "(T6 M6 X 1 TAP - DIA 6.)",
            "(T3 10 MM FLAT END MILL - DIA 10.)",
            "(T7 10 MM 90 DEGREE CHAMFER MILL - DIA 10.)",
            "(FACE 1)",
            "(POCKET 2)",
            "(DRILL 6 THROUGH)",
            "(DRILL 5 FOR M6)",
            "(TAP M6)",
            "(CONTOUR 6)",
            "(CHAMFER 7)",
        ]
    );
    assert!(haas.contains("\nM1\n(TAP M6)\nT6 M6 (M6 X 1 TAP)\n"), "{haas}");
    assert!(haas.contains("G83") && haas.contains("G84") && haas.contains("UNVERIFIED OUTPUT FROM SOLIDCRAFT CAM"));
    assert!(programs[1].contains("G84") && !programs[1].contains('%'));
    assert!(programs[2].contains("G20") && programs[2].starts_with("%\nO01102 "));
    let moves: Vec<u64> = of("cam.post").iter().map(|r| r["moves"].as_u64().unwrap()).collect();
    // (In inches a small arc that four decimals can't hold is written as short lines, each within
    // the tolerance: more moves, the same path.)
    assert!(moves[0] > 200 && moves[1] == moves[0] && moves[2] >= moves[0], "{moves:?}");
    // A post that has no tapping cycle says so instead of writing something else.
    let refused = session.execute("cam.post", &json!({"setup": "setup-1", "post": "fanuc_mill"})).unwrap_err().to_string();
    assert!(refused.contains("tap"), "{refused}");

    // The setup sheet lists the seven tools and the seven operations.
    let sheet = of("cam.setup.sheet")[0];
    assert_eq!((sheet["tools"].as_u64(), sheet["operations"].as_u64(), sheet["left_out"].as_u64()), (Some(7), Some(7), Some(0)));
    assert!(sheet["html"].as_str().unwrap().contains("Tap M6"));

    // The design keeps the setup, the tools and the operations; reopened, it posts the same.
    let saved = io::write_design_container(&session.doc).unwrap();
    let mut reopened = Session::new(io::read_design(&saved).unwrap());
    let again = reopened.execute("cam.post", &json!({"setup": "setup-1", "post": "haas_mill", "program_number": 1101})).unwrap();
    assert_eq!(again["text"].as_str(), Some(haas.as_str()), "a reopened design posts the same program");
}

#[test]
fn other_plates_from_the_same_script_are_cut_as_cleanly() {
    // Smaller and thinner, with two corner holes, one tapped hole and three tabs.
    cut_and_checked(&Plate {
        length: 90.0,
        width: 60.0,
        thickness: 12.0,
        pocket: [44.0, 30.0, 5.0],
        pocket_x: -12.0,
        island: 12.0,
        corner_holes: 2,
        tapped: 1,
        tabs: 3,
        chamfer: 0.8,
    });
    // Thicker, with no island and no tapped holes, a wider chamfer and six tabs.
    cut_and_checked(&Plate {
        length: 140.0,
        width: 90.0,
        thickness: 20.0,
        pocket: [80.0, 50.0, 10.0],
        pocket_x: 0.0,
        island: 0.0,
        corner_holes: 4,
        tapped: 0,
        tabs: 6,
        chamfer: 1.0,
    });
}
