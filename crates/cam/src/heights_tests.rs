use super::*;
use crate::manufacture::{BoxAnchor, BoxOffsets, BoxSide, SafeHeights, Setup, SetupKind, Stock, Wcs, WcsOrientation, WcsOrigin};
use crate::placement::Bounds;
use solidcore_record::Vec3;

/// The vise setup on a 100 x 80 x 20 block with 1 mm of stock round it and on top: the WCS at the
/// stock's top, so the block's top is Z -1 and its bottom Z -21; clearance 25, retract 5.
fn placed() -> Placement {
    let setup = Setup {
        id: "setup-1".into(),
        name: "Op 1".into(),
        kind: SetupKind::Milling,
        machine: None,
        material: None,
        wcs: Wcs {
            origin: WcsOrigin::StockBox { anchor: BoxAnchor { x: BoxSide::Center, y: BoxSide::Center, z: BoxSide::Max } },
            orientation: WcsOrientation::Model,
        },
        work_offset: 1,
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 1.0 } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    };
    let p = |x, y, z| Vec3 { x, y, z };
    setup.place(Bounds { min: p(0.0, 0.0, 0.0), max: p(100.0, 80.0, 20.0) }).unwrap()
}

fn from(from: HeightFrom, offset: f64) -> HeightRef {
    HeightRef { from, offset, at: None }
}

#[test]
fn each_reference_is_measured_where_it_says_and_the_offset_is_added() {
    let p = placed();
    let z = |h: HeightRef, own: f64, point: Option<f64>| h.resolve(&p, own, point).unwrap();
    assert_eq!(z(from(HeightFrom::StockTop, 2.0), 99.0, None), 2.0);
    assert_eq!(z(from(HeightFrom::StockBottom, 0.0), 99.0, None), -21.0);
    assert_eq!(z(from(HeightFrom::ModelTop, -0.5), 99.0, None), -1.5);
    assert_eq!(z(from(HeightFrom::ModelBottom, -1.0), 99.0, None), -22.0);
    assert_eq!(z(from(HeightFrom::Feature, 0.3), -7.0, None), -6.7);
    assert_eq!(z(from(HeightFrom::Setup, 10.0), 25.0, None), 35.0);
    assert_eq!(z(from(HeightFrom::Z, -4.25), 99.0, None), -4.25);
    let picked = HeightRef { from: HeightFrom::Point, offset: 1.0, at: Some([10.0, 10.0, 15.0]) };
    assert_eq!(z(picked, 99.0, Some(-6.0)), -5.0);
    // A point not read from the model, an offset or a height out of range: said, not guessed.
    assert!(picked.resolve(&p, 0.0, None).is_err());
    assert!(from(HeightFrom::Z, 1e6).resolve(&p, 0.0, None).is_err());
    assert!(from(HeightFrom::Feature, 1.0).resolve(&p, f64::NAN, None).is_err());
}

#[test]
fn an_operations_clearance_and_retract_replace_the_setups_and_keep_their_order() {
    let p = placed();
    let heights = Heights { clearance: Some(from(HeightFrom::StockTop, 50.0)), retract: Some(from(HeightFrom::ModelTop, 3.0)), ..Heights::default() };
    let placed = heights.placed(&p, (None, None)).unwrap();
    assert_eq!((placed.clearance_z, placed.retract_z), (50.0, 2.0));
    // Only the retract: the setup's clearance stays.
    let only = Heights { retract: Some(from(HeightFrom::Setup, -2.0)), ..Heights::default() };
    let placed = only.placed(&p, (None, None)).unwrap();
    assert_eq!((placed.clearance_z, placed.retract_z), (25.0, 3.0));
    // A retract above the clearance is refused with both heights.
    let upside_down = Heights { clearance: Some(from(HeightFrom::Z, 4.0)), ..Heights::default() };
    let why = upside_down.placed(&p, (None, None)).unwrap_err().to_string();
    assert!(why.contains("Z 4.000") && why.contains("Z 5.000"), "{why}");
}

#[test]
fn heights_are_checked_and_plain_json() {
    let point = HeightRef { from: HeightFrom::Point, offset: 0.0, at: None };
    assert!(point.validate("top").is_err());
    let stray = HeightRef { from: HeightFrom::Z, offset: 0.0, at: Some([0.0; 3]) };
    assert!(stray.validate("top").is_err());
    assert!(from(HeightFrom::Z, f64::INFINITY).validate("top").is_err());
    let heights = Heights {
        top: Some(from(HeightFrom::ModelTop, 0.0)),
        bottom: Some(HeightRef { from: HeightFrom::Point, offset: -0.2, at: Some([1.0, 2.0, 3.0]) }),
        ..Heights::default()
    };
    heights.validate().unwrap();
    let json = serde_json::to_value(heights).unwrap();
    assert_eq!(
        json,
        serde_json::json!({"top": {"from": "model_top", "offset": 0.0}, "bottom": {"from": "point", "offset": -0.2, "at": [1.0, 2.0, 3.0]}})
    );
    assert_eq!(serde_json::from_value::<Heights>(json).unwrap(), heights);
    // A reference given alone is from the feature, with no offset.
    assert_eq!(serde_json::from_value::<HeightRef>(serde_json::json!({})).unwrap(), from(HeightFrom::Feature, 0.0));
    for hostile in [serde_json::json!({"from": "moon"}), serde_json::json!({"offset": "up"}), serde_json::json!({"top": 1})] {
        assert!(serde_json::from_value::<Heights>(hostile.clone()).is_err() || serde_json::from_value::<HeightRef>(hostile).is_err());
    }
}
