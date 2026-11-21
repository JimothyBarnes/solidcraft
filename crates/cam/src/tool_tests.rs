use std::f64::consts::PI;

use super::*;

fn close(a: f64, b: f64, tol: f64) -> bool {
    (a - b).abs() <= tol
}

/// A 6 mm, 3-flute carbide flat end mill with data for aluminium: 200 m/min, 0.04 mm/tooth.
fn flat6() -> Tool {
    Tool {
        number: 1,
        name: "6 mm flat end mill".into(),
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
        holder: Holder { stick_out: 25.0, segments: vec![HolderSegment { length: 40.0, lower_diameter: 20.0, upper_diameter: 32.0 }] },
        length_offset: 1,
        diameter_offset: 1,
        coolant: Coolant::Flood,
        cutting_data: vec![CuttingData {
            material: "aluminium_6061".into(),
            surface_speed: 200.0,
            chip_load: 0.04,
            plunge_factor: 0.3,
            ramp_factor: 0.6,
        }],
        overrides: Overrides::default(),
    }
}

#[test]
fn a_full_slot_takes_speed_from_surface_speed_and_feed_from_chip_load() {
    // n = 200·1000/(π·6) = 10610.33 rpm; slotting (ae ≥ D/2) keeps fz = 0.04; vf = 0.04·3·n.
    let sf = flat6().speeds_feeds("aluminium_6061", Engagement { axial: 3.0, radial: 6.0 }, 20_000.0).unwrap();
    let n = 200_000.0 / (PI * 6.0);
    assert!(close(sf.rpm, n, 1e-9) && close(sf.rpm, 10_610.33, 0.01));
    assert!(close(sf.feed, 0.04 * 3.0 * n, 1e-9) && close(sf.feed, 1273.24, 0.01));
    assert!(close(sf.plunge_feed, 0.3 * sf.feed, 1e-9) && close(sf.ramp_feed, 0.6 * sf.feed, 1e-9));
    assert!(close(sf.surface_speed, 200.0, 1e-9) && close(sf.chip_thickness, 0.04, 1e-12));
    assert!(sf.warnings.is_empty() && !sf.overridden);
}

#[test]
fn a_light_stepover_raises_the_feed_so_the_chip_keeps_its_thickness() {
    // ae = D/10: the chip thins to 2·sqrt(0.1 − 0.01) = 0.6 of fz, so fz = 0.04/0.6.
    let sf = flat6().speeds_feeds("aluminium_6061", Engagement { axial: 6.0, radial: 0.6 }, 20_000.0).unwrap();
    assert!(close(sf.feed_per_tooth, 0.04 / 0.6, 1e-12));
    assert!(close(sf.chip_thickness, 0.04, 1e-12));
    assert!(close(sf.feed, 0.04 / 0.6 * 3.0 * 200_000.0 / (PI * 6.0), 1e-9));
}

#[test]
fn the_machine_speed_caps_the_spindle_and_says_so() {
    let sf = flat6().speeds_feeds("aluminium_6061", Engagement { axial: 3.0, radial: 6.0 }, 8000.0).unwrap();
    assert_eq!(sf.rpm, 8000.0);
    assert!(close(sf.feed, 0.04 * 3.0 * 8000.0, 1e-9)); // 960 mm/min: the chip load is kept
    assert!(close(sf.surface_speed, PI * 6.0 * 8.0, 1e-9)); // 150.8 m/min, below the 200 asked for
    assert!(matches!(sf.warnings.as_slice(), [FeedWarning::SpindleCapped { max, .. }] if *max == 8000.0));
}

#[test]
fn a_ball_cutting_shallow_cuts_on_a_smaller_diameter_and_spins_faster() {
    let mut ball = flat6();
    ball.kind = ToolKind::BallEndMill;
    ball.geometry.corner_radius = 3.0;
    // ap 0.5 on r 3: Dc = 2·sqrt(9 − 6.25) = 3.3166 mm.
    let dc = 2.0 * (9.0f64 - 6.25).sqrt();
    assert!(close(ball.effective_diameter(0.5), dc, 1e-12));
    assert!(close(ball.effective_diameter(5.0), 6.0, 0.0));
    let sf = ball.speeds_feeds("aluminium_6061", Engagement { axial: 0.5, radial: 0.5 }, 30_000.0).unwrap();
    assert!(close(sf.rpm, 200_000.0 / (PI * dc), 1e-9) && close(sf.rpm, 19_194.81, 0.01));
}

#[test]
fn a_taps_feed_is_its_pitch_per_revolution() {
    let tap = Tool {
        number: 7,
        name: "M6x1 tap".into(),
        kind: ToolKind::Tap,
        geometry: ToolGeometry {
            diameter: 6.0,
            flute_length: 12.0,
            overall_length: 60.0,
            shank_diameter: 6.0,
            thread_pitch: 1.0,
            ..flat6().geometry
        },
        cutting_data: vec![CuttingData { material: "steel_1018".into(), surface_speed: 10.0, chip_load: 0.1, plunge_factor: 1.0, ramp_factor: 1.0 }],
        ..flat6()
    };
    let sf = tap.speeds_feeds("steel_1018", Engagement { axial: 10.0, radial: 0.0 }, 6000.0).unwrap();
    assert!(close(sf.rpm, 10_000.0 / (PI * 6.0), 1e-9)); // 530.5 rpm
    assert!(close(sf.feed, sf.rpm * 1.0, 1e-9));
    assert!(sf.warnings.is_empty());
}

#[test]
fn typed_in_numbers_win_and_a_bad_chip_is_flagged() {
    let mut t = flat6();
    t.overrides = Overrides { rpm: Some(10_000.0), feed: Some(300.0) };
    let sf = t.speeds_feeds("aluminium_6061", Engagement { axial: 3.0, radial: 6.0 }, 20_000.0).unwrap();
    assert!(sf.overridden && sf.rpm == 10_000.0 && sf.feed == 300.0);
    // 300/(10000·3) = 0.01 mm/tooth: a quarter of the 0.04 intended, so it rubs.
    assert!(matches!(sf.warnings.as_slice(), [FeedWarning::ChipTooThin { .. }]));
    t.overrides.feed = Some(3000.0); // 0.1 mm/tooth: two and a half times too thick
    let sf = t.speeds_feeds("aluminium_6061", Engagement { axial: 3.0, radial: 6.0 }, 20_000.0).unwrap();
    assert!(matches!(sf.warnings.as_slice(), [FeedWarning::ChipTooThick { .. }]));
    // A typed-in speed above the machine's is capped like a derived one.
    t.overrides = Overrides { rpm: Some(50_000.0), feed: None };
    let sf = t.speeds_feeds("aluminium_6061", Engagement { axial: 3.0, radial: 6.0 }, 12_000.0).unwrap();
    assert!(sf.rpm == 12_000.0 && matches!(sf.warnings.as_slice(), [FeedWarning::SpindleCapped { .. }]));
}

#[test]
fn no_cutting_data_means_no_numbers_rather_than_a_guess() {
    let e = flat6().speeds_feeds("titanium_6al4v", Engagement { axial: 1.0, radial: 1.0 }, 10_000.0).unwrap_err();
    assert!(e.to_string().contains("no cutting data"));
}

#[test]
fn hostile_tools_and_engagements_are_refused() {
    let nan = f64::NAN;
    let mutations: Vec<Box<dyn Fn(&mut Tool)>> = vec![
        Box::new(|t| t.number = 0),
        Box::new(move |t| t.geometry.diameter = nan),
        Box::new(|t| t.geometry.diameter = 0.0),
        Box::new(|t| t.geometry.overall_length = 10.0), // shorter than the flutes
        Box::new(|t| t.flutes = 0),
        Box::new(|t| t.kind = ToolKind::BallEndMill), // radius 0 on a ball
        Box::new(|t| t.kind = ToolKind::Drill),       // no tip angle
        Box::new(|t| t.kind = ToolKind::Tap),         // no pitch
        Box::new(|t| t.geometry.tip_angle = 1.0),     // an angle on a flat end mill
        Box::new(|t| t.holder.stick_out = 100.0),
        Box::new(|t| t.holder.segments[0].length = -1.0),
        Box::new(|t| t.cutting_data[0].plunge_factor = 0.0),
        Box::new(|t| t.cutting_data[0].surface_speed = f64::INFINITY),
        Box::new(|t| {
            let again = t.cutting_data[0].clone();
            t.cutting_data.push(again);
        }),
        Box::new(|t| t.overrides.rpm = Some(f64::INFINITY)),
        Box::new(|t| t.overrides.feed = Some(-1.0)),
    ];
    for (i, mutate) in mutations.iter().enumerate() {
        let mut t = flat6();
        mutate(&mut t);
        assert!(t.validate().is_err(), "mutation {i} was accepted");
        assert!(t.speeds_feeds("aluminium_6061", Engagement { axial: 1.0, radial: 1.0 }, 10_000.0).is_err(), "mutation {i} was computed");
    }
    let t = flat6();
    for (ap, ae, max) in [(0.0, 1.0, 1e4), (1.0, 0.0, 1e4), (nan, 1.0, 1e4), (1.0, nan, 1e4), (1.0, 1.0, 0.0), (1.0, 1.0, f64::INFINITY)] {
        assert!(t.speeds_feeds("aluminium_6061", Engagement { axial: ap, radial: ae }, max).is_err(), "{ap} {ae} {max}");
    }
}

#[test]
fn a_library_round_trips_refuses_clashes_and_searches() {
    let mut ball = flat6();
    ball.number = 2;
    ball.name = "6 mm Ball".into();
    ball.kind = ToolKind::BallEndMill;
    ball.geometry.corner_radius = 3.0;
    let mut big = flat6();
    big.number = 3;
    big.name = "12 mm flat".into();
    big.geometry = ToolGeometry { diameter: 12.0, flute_length: 26.0, overall_length: 83.0, shank_diameter: 12.0, ..big.geometry };
    let lib = ToolLibrary { format: TOOL_LIBRARY_FORMAT, name: "Shop".into(), tools: vec![flat6(), ball, big] };
    let json = lib.to_json().unwrap();
    assert!(json.lines().count() > 50, "pretty, diffable JSON");
    assert_eq!(ToolLibrary::from_json(&json).unwrap(), lib);
    assert_eq!(lib.search("BALL", None, None).len(), 1);
    assert_eq!(lib.search("", Some(ToolKind::FlatEndMill), None).len(), 2);
    assert_eq!(lib.search("", None, Some((5.0, 7.0))).len(), 2);
    let mut clash = lib.clone();
    clash.tools[1].number = 1;
    assert!(clash.to_json().unwrap_err().to_string().contains("used twice"));
    assert!(ToolLibrary::from_json(&json.replacen("\"format\": 1", "\"format\": 2", 1)).is_err());
    assert!(ToolLibrary::from_json(&json.replacen("\"flutes\": 3", "\"flutes\": 3, \"magic\": 1", 1)).is_err());
    assert!(ToolLibrary::from_json("[]").is_err());
}
