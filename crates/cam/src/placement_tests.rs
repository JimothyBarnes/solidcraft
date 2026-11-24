use super::*;
use crate::manufacture::{BoxOffsets, SafeHeights, SetupKind, Wcs};

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

fn anchor(x: BoxSide, y: BoxSide, z: BoxSide) -> BoxAnchor {
    BoxAnchor { x, y, z }
}

fn bounds(min: Vec3, max: Vec3) -> Bounds {
    Bounds { min, max }
}

/// The usual vise setup: stock 1 mm over the sides and the top, the origin at the stock's top centre.
fn vise() -> Setup {
    Setup {
        id: "setup-1".into(),
        name: "Op 1 (vise)".into(),
        kind: SetupKind::Milling,
        machine: None,
        material: None,
        wcs: Wcs {
            origin: WcsOrigin::StockBox { anchor: anchor(BoxSide::Center, BoxSide::Center, BoxSide::Max) },
            orientation: WcsOrientation::Model,
        },
        work_offset: 1,
        stock: Stock::RelativeBox { offsets: BoxOffsets { x_min: 1.0, x_max: 1.0, y_min: 1.0, y_max: 1.0, z_min: 0.0, z_max: 1.0 } },
        model_bodies: vec![],
        fixture_bodies: vec![],
        safe_heights: SafeHeights { clearance: 25.0, retract: 5.0 },
        operations: vec![],
    }
}

/// A 60 x 40 x 20 block with its least corner at the model origin.
fn block() -> Bounds {
    bounds(p(0.0, 0.0, 0.0), p(60.0, 40.0, 20.0))
}

#[test]
fn the_vise_setup_puts_the_origin_on_top_of_the_stock_at_its_centre() {
    // Stock: (-1, -1, 0) to (61, 41, 21) in the model; its top centre (30, 20, 21) is the origin.
    let placed = vise().place(block()).unwrap();
    assert_eq!(placed.origin, p(30.0, 20.0, 21.0));
    assert_eq!((placed.x_axis, placed.y_axis, placed.z_axis), (X, Y, Z));
    assert_eq!(placed.stock, bounds(p(-31.0, -21.0, -21.0), p(31.0, 21.0, 0.0)));
    assert_eq!(placed.model, bounds(p(-30.0, -20.0, -21.0), p(30.0, 20.0, -1.0)));
    assert_eq!((placed.clearance_z, placed.retract_z), (25.0, 5.0));
    assert_eq!(placed.stock.size(), p(62.0, 42.0, 21.0));
}

#[test]
fn a_fixed_stock_box_shares_the_anchor_point_with_the_model() {
    let mut s = vise();
    // Least corners together: stock (0, 0, 0) to (70, 50, 25), origin at its top front-left corner.
    s.stock = Stock::FixedBox { size: p(70.0, 50.0, 25.0), anchor: anchor(BoxSide::Min, BoxSide::Min, BoxSide::Min) };
    s.wcs.origin = WcsOrigin::StockBox { anchor: anchor(BoxSide::Min, BoxSide::Min, BoxSide::Max) };
    let placed = s.place(block()).unwrap();
    assert_eq!(placed.origin, p(0.0, 0.0, 25.0));
    assert_eq!(placed.stock, bounds(p(0.0, 0.0, -25.0), p(70.0, 50.0, 0.0)));
    assert_eq!(placed.model, bounds(p(0.0, 0.0, -25.0), p(60.0, 40.0, -5.0)));
    // Centred in X and Y with the tops flush: the model's top centre (30, 20, 20) is the stock's.
    s.stock = Stock::FixedBox { size: p(70.0, 50.0, 25.0), anchor: anchor(BoxSide::Center, BoxSide::Center, BoxSide::Max) };
    s.wcs.origin = WcsOrigin::StockBox { anchor: anchor(BoxSide::Center, BoxSide::Center, BoxSide::Max) };
    let placed = s.place(block()).unwrap();
    assert_eq!(placed.origin, p(30.0, 20.0, 20.0));
    assert_eq!(placed.stock, bounds(p(-35.0, -25.0, -25.0), p(35.0, 25.0, 0.0)));
    assert_eq!((placed.clearance_z, placed.retract_z), (25.0, 5.0));
    // A box exactly the model's size holds it.
    s.stock = Stock::FixedBox { size: p(60.0, 40.0, 20.0), anchor: anchor(BoxSide::Min, BoxSide::Max, BoxSide::Center) };
    assert_eq!(s.place(block()).unwrap().stock.size(), p(60.0, 40.0, 20.0));
}

#[test]
fn an_explicit_origin_and_axes_equal_to_the_models_are_used_as_given() {
    let mut s = vise();
    s.wcs = Wcs {
        origin: WcsOrigin::Explicit { point: p(10.0, 0.0, 20.0) },
        orientation: WcsOrientation::Explicit { z: p(0.0, 0.0, 2.0), x: p(3.0, 0.0, 0.0) },
    };
    let placed = s.place(block()).unwrap();
    assert_eq!(placed.origin, p(10.0, 0.0, 20.0));
    assert_eq!(placed.model, bounds(p(-10.0, 0.0, -20.0), p(50.0, 40.0, 0.0)));
    // The stock top is at 1 in the WCS here, so the heights are 26 and 6.
    assert_eq!((placed.clearance_z, placed.retract_z), (26.0, 6.0));
}

#[test]
fn what_cannot_be_placed_yet_is_refused_by_name() {
    let cases: Vec<(Setup, &str)> = vec![
        (Setup { wcs: Wcs { orientation: WcsOrientation::FromFace { face: "face:top".into(), x_edge: None }, ..vise().wcs }, ..vise() }, "turned"),
        (Setup { wcs: Wcs { orientation: WcsOrientation::Explicit { z: p(0.0, 1.0, 0.0), x: p(1.0, 0.0, 0.0) }, ..vise().wcs }, ..vise() }, "turned"),
        (Setup { wcs: Wcs { origin: WcsOrigin::ModelPoint { reference: "vertex:1".into() }, ..vise().wcs }, ..vise() }, "model point"),
        (Setup { stock: Stock::RelativeCylinder { radial: 1.0, z_min: 0.0, z_max: 1.0 }, ..vise() }, "cylinder"),
        (Setup { stock: Stock::FromBody { body: "body:Stock".into() }, ..vise() }, "from a body"),
        (Setup { stock: Stock::FromSetup { setup: "setup-0".into() }, ..vise() }, "earlier setup"),
        (
            Setup { stock: Stock::FixedBox { size: p(50.0, 40.0, 20.0), anchor: anchor(BoxSide::Min, BoxSide::Min, BoxSide::Min) }, ..vise() },
            "along X",
        ),
        (
            Setup { stock: Stock::FixedBox { size: p(70.0, 50.0, 19.0), anchor: anchor(BoxSide::Min, BoxSide::Min, BoxSide::Max) }, ..vise() },
            "along Z",
        ),
    ];
    for (s, expect) in cases {
        let e = s.place(block()).unwrap_err().to_string();
        assert!(e.contains(expect), "{:?}: {e:?} does not mention {expect:?}", s.stock);
    }
    for model in [
        bounds(p(0.0, 0.0, 0.0), p(f64::NAN, 1.0, 1.0)),
        bounds(p(0.0, 0.0, 0.0), p(f64::INFINITY, 1.0, 1.0)),
        bounds(p(0.0, 0.0, 2.0), p(1.0, 1.0, 1.0)),
        bounds(p(0.0, 0.0, 0.0), p(200_000.0, 1.0, 1.0)),
    ] {
        assert!(vise().place(model).is_err(), "{model:?}");
    }
    // A model at the edge of the coordinate range with stock past it is refused, not overflowed.
    let edge = bounds(p(0.0, 0.0, 0.0), p(1.0, 1.0, MAX_COORDINATE));
    let mut tall = vise();
    tall.wcs.origin = WcsOrigin::Explicit { point: p(0.0, 0.0, -MAX_COORDINATE) };
    assert!(tall.place(edge).is_err());
}

#[test]
fn a_placement_is_plain_json() {
    let placed = vise().place(block()).unwrap();
    let json = serde_json::to_value(placed).unwrap();
    assert_eq!(json["origin"], serde_json::json!([30.0, 20.0, 21.0]));
    assert_eq!(json["stock"]["max"], serde_json::json!([31.0, 21.0, 0.0]));
    assert_eq!(json["clearance_z"], 25.0);
    assert_eq!(serde_json::from_value::<Placement>(json).unwrap(), placed);
}

#[test]
fn a_setup_on_a_machine_is_kept_to_its_top_speed_and_an_unknown_machine_is_refused() {
    // On no machine of its own a setup is on the generic mill; the Haas VF-4SS runs to 12 000.
    let model = Bounds { min: Vec3 { x: 0.0, y: 0.0, z: 0.0 }, max: Vec3 { x: 60.0, y: 40.0, z: 20.0 } };
    let mut setup = vise();
    assert_eq!(setup.place(model).unwrap().max_rpm, 10_000.0);
    setup.machine = Some("haas_vf4ss".into());
    assert_eq!(setup.place(model).unwrap().max_rpm, 12_000.0);
    setup.machine = Some("generic_mill".into());
    assert_eq!(setup.place(model).unwrap().max_rpm, 10_000.0);
    // A machine this version does not know: the setup can't be placed, and the reason names it.
    setup.machine = Some("haas_umc750".into());
    let why = setup.place(model).unwrap_err().to_string();
    assert!(why.contains("haas_umc750") && why.contains("generic_mill, haas_vf4ss"), "{why}");
    // A placement written before machines were known reads as the generic mill's.
    let mut old = serde_json::to_value(vise().place(model).unwrap()).unwrap();
    old.as_object_mut().unwrap().remove("max_rpm");
    assert_eq!(serde_json::from_value::<Placement>(old).unwrap().max_rpm, 10_000.0);
}
