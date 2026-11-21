use super::*;
use crate::tool::{Holder, Overrides, ToolGeometry, ToolKind};
use crate::toolpath::{Coolant, Move};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn tool(number: u32) -> Tool {
    Tool {
        number,
        name: format!("T{number}"),
        kind: ToolKind::FlatEndMill,
        geometry: ToolGeometry {
            diameter: 6.0,
            corner_radius: 0.0,
            flute_length: 18.0,
            shoulder_length: 0.0,
            overall_length: 60.0,
            shank_diameter: 6.0,
            tip_angle: 0.0,
            tip_diameter: 0.0,
            thread_pitch: 0.0,
        },
        flutes: 3,
        holder: Holder::default(),
        length_offset: number,
        diameter_offset: number,
        coolant: Coolant::Flood,
        cutting_data: vec![],
        overrides: Overrides::default(),
    }
}

/// One milling setup in a vise: stock 1 mm over the model on the sides and 2 mm on top, origin
/// at the top centre of the stock, G54, one hand-made toolpath with tool 1.
fn shop() -> ManufactureData {
    ManufactureData {
        format: MANUFACTURE_FORMAT,
        tools: vec![tool(1), tool(2)],
        setups: vec![Setup {
            id: "setup-1".into(),
            name: "Op 1 (vise)".into(),
            kind: SetupKind::Milling,
            machine: None,
            wcs: Wcs {
                origin: WcsOrigin::StockBox { anchor: BoxAnchor { x: BoxSide::Center, y: BoxSide::Center, z: BoxSide::Max } },
                orientation: WcsOrientation::Model,
            },
            work_offset: 1,
            stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 2.0 } },
            model_bodies: vec!["body:Bracket".into()],
            fixture_bodies: vec![],
            safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
            operations: vec![Operation {
                id: "op-1".into(),
                name: "Hand-made square".into(),
                tool: 1,
                suppressed: false,
                operation: OperationKind::Manual {
                    toolpath: Toolpath {
                        start: p(0.0, 0.0, 25.0),
                        moves: vec![Move::Rapid { to: p(0.0, 0.0, 5.0) }, Move::Linear { to: p(10.0, 0.0, -1.0), feed: 300.0 }],
                    },
                },
            }],
        }],
    }
}

#[test]
fn empty_and_full_data_round_trip_through_json() {
    let empty = ManufactureData::default();
    assert_eq!(ManufactureData::from_json(&empty.to_json().unwrap()).unwrap(), empty);
    let full = shop();
    let json = full.to_json().unwrap();
    assert!(json.contains(r#""kind": "relative_box""#) && json.contains(r#""kind": "stock_box""#) && json.contains(r#""kind": "manual""#));
    assert_eq!(ManufactureData::from_json(&json).unwrap(), full);
}

#[test]
fn a_setup_may_take_its_stock_from_an_earlier_setup_only() {
    let mut d = shop();
    let mut second = d.setups[0].clone();
    second.id = "setup-2".into();
    second.operations.clear();
    second.stock = Stock::FromSetup { setup: "setup-1".into() };
    d.setups.push(second.clone());
    assert!(d.validate().is_ok());
    // Pointing forward (or at itself) would make a loop: refused.
    d.setups[0].stock = Stock::FromSetup { setup: "setup-2".into() };
    assert!(d.validate().unwrap_err().to_string().contains("not an earlier setup"));
}

#[test]
fn hostile_data_is_refused_with_a_reason() {
    let nan = f64::NAN;
    let cases: Vec<(&str, Box<dyn Fn(&mut ManufactureData)>)> = vec![
        ("format", Box::new(|d| d.format = 99)),
        ("used twice", Box::new(|d| d.tools[1].number = 1)),
        (
            "used twice",
            Box::new(|d| {
                let again = d.setups[0].clone();
                d.setups.push(again);
            }),
        ),
        ("not one of the design's tools", Box::new(|d| d.setups[0].operations[0].tool = 9)),
        ("id of 1 to 64", Box::new(|d| d.setups[0].id = "has space".into())),
        ("id of 1 to 64", Box::new(|d| d.setups[0].operations[0].id = String::new())),
        ("work offset", Box::new(|d| d.setups[0].work_offset = 0)),
        ("work offset", Box::new(|d| d.setups[0].work_offset = 7)),
        ("clearance at or above retract", Box::new(|d| d.setups[0].safe_heights = SafeHeights { clearance: 2.0, retract: 5.0 })),
        ("clearance at or above retract", Box::new(move |d| d.setups[0].safe_heights.clearance = nan)),
        ("stock sizes", Box::new(|d| d.setups[0].stock = Stock::RelativeBox { offsets: BoxOffsets { x_min: -1.0, ..BoxOffsets::default() } })),
        (
            "stock sizes",
            Box::new(|d| {
                d.setups[0].stock =
                    Stock::FixedBox { size: p(0.0, 10.0, 10.0), anchor: BoxAnchor { x: BoxSide::Min, y: BoxSide::Min, z: BoxSide::Min } }
            }),
        ),
        ("WCS origin", Box::new(move |d| d.setups[0].wcs.origin = WcsOrigin::Explicit { point: p(nan, 0.0, 0.0) })),
        ("not parallel", Box::new(|d| d.setups[0].wcs.orientation = WcsOrientation::Explicit { z: p(0.0, 0.0, 1.0), x: p(0.0, 0.0, 2.0) })),
        ("not parallel", Box::new(|d| d.setups[0].wcs.orientation = WcsOrientation::Explicit { z: p(0.0, 0.0, 0.0), x: p(1.0, 0.0, 0.0) })),
        ("control characters", Box::new(|d| d.setups[0].model_bodies = vec!["body\u{0}x".into()])),
        ("control characters", Box::new(|d| d.setups[0].wcs.origin = WcsOrigin::ModelPoint { reference: String::new() })),
        ("name of at most", Box::new(|d| d.setups[0].name = "x".repeat(201))),
        (
            "finite",
            Box::new(move |d| {
                let OperationKind::Manual { toolpath } = &mut d.setups[0].operations[0].operation;
                toolpath.moves.push(Move::Rapid { to: p(nan, 0.0, 0.0) });
            }),
        ),
    ];
    for (expect, mutate) in &cases {
        let mut d = shop();
        mutate(&mut d);
        let e = d.validate().expect_err(expect).to_string();
        assert!(e.contains(expect), "{e:?} does not mention {expect:?}");
        assert!(d.to_json().is_err());
    }
    // An operation id must be unique across the whole design, not just its setup.
    let mut d = shop();
    let mut second = d.setups[0].clone();
    second.id = "setup-2".into();
    d.setups.push(second);
    assert!(d.validate().unwrap_err().to_string().contains("op-1"));
    // Unknown fields and broken JSON are refused when read.
    let json = shop().to_json().unwrap();
    assert!(ManufactureData::from_json(&json.replacen("\"suppressed\": false", "\"suppressed\": false, \"extra\": 1", 1)).is_err());
    assert!(ManufactureData::from_json("{").is_err());
}
