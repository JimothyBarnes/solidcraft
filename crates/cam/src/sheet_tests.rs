use solidcore_record::Vec3;

use super::*;
use crate::placement::Bounds;

fn p(x: f64, y: f64, z: f64) -> Vec3 {
    Vec3 { x, y, z }
}

/// The 60 x 40 x 20 block in stock 1 mm bigger all round and on top, the WCS at the middle of
/// the stock's top: clearance 25, retract 5.
fn block() -> Placement {
    Placement {
        origin: p(30.0, 20.0, 21.0),
        x_axis: p(1.0, 0.0, 0.0),
        y_axis: p(0.0, 1.0, 0.0),
        z_axis: p(0.0, 0.0, 1.0),
        model: Bounds { min: p(-30.0, -20.0, -21.0), max: p(30.0, 20.0, -1.0) },
        stock: Bounds { min: p(-31.0, -21.0, -21.0), max: p(31.0, 21.0, 0.0) },
        clearance_z: 25.0,
        retract_z: 5.0,
    }
}

fn tool(number: u32, name: &str, diameter: f64, stick_out: f64) -> SheetTool {
    SheetTool {
        number,
        name: name.into(),
        kind: "flat end mill".into(),
        diameter,
        flutes: 4,
        stick_out,
        length_offset: number,
        diameter_offset: number,
    }
}

fn operation(name: &str, tool: u32, kind: &str, lowest_z: f64, minutes: f64) -> SheetOperation {
    SheetOperation {
        name: name.into(),
        tool,
        kind: kind.into(),
        rpm: Some(3183.4),
        feed: Some(636.6),
        lowest_z: Some(lowest_z),
        minutes: Some(minutes),
        trouble: None,
        cuts: Vec::new(),
    }
}

/// Faced with T4 and contoured with it, a drill T3 loaded and not used.
fn sheet() -> SetupSheet {
    let round = vec![[0.0, -30.0, -30.0, -30.0], [-30.0, -30.0, -40.0, -20.0], [-40.0, -20.0, -40.0, 20.0]];
    SetupSheet {
        title: "Bracket".into(),
        setup: "Setup 1".into(),
        date: Some("2026-11-22".into()),
        units: LengthUnit::Millimetre,
        material: Some("aluminium".into()),
        work_offset: 1,
        origin: "the middle of the stock's top".into(),
        placement: block(),
        tools: vec![tool(3, "10 mm drill", 10.0, 50.0), tool(4, "20 mm flat end mill", 20.0, 50.0)],
        operations: vec![
            SheetOperation { cuts: vec![[-45.0, -12.0, 45.0, -12.0], [45.0, 0.0, -45.0, 0.0]], ..operation("Face 1", 4, "face", -1.0, 0.5) },
            SheetOperation { cuts: round, ..operation("Contour 2", 4, "contour", -21.0, 1.45) },
        ],
        notes: vec!["spindle speed capped at 10000 rpm".into()],
    }
}

#[test]
fn a_sheet_says_what_the_machine_needs_set_up() {
    let html = sheet().html().unwrap();
    assert!(html.starts_with("<!DOCTYPE html>\n<html lang=\"en\">") && html.ends_with("</body>\n</html>\n"));
    let has = |text: &str| assert!(html.contains(text), "no {text:?} in:\n{html}");
    has("<title>Setup sheet: Bracket, Setup 1</title>");
    has("<p class=\"sub\">Work offset G54 · 2026-11-22</p>");
    has("Unverified program. Prove it out on the machine first: air cut, then single block.");
    // The stock and the part by their sizes, and where each lies from the origin.
    has("<tr><th>Stock</th><td>62.00 × 42.00 × 21.00 mm, aluminium</td></tr>");
    has("<tr><th>Part</th><td>60.00 × 40.00 × 20.00 mm</td></tr>");
    has("<tr><th>Work origin</th><td>G54 at the middle of the stock&#39;s top</td></tr>");
    has("<tr><th>Stock from the origin</th><td>X -31.00 to 31.00, Y -21.00 to 21.00, Z -21.00 to 0.00 mm</td></tr>");
    has("<tr><th>Part from the origin</th><td>X -30.00 to 30.00, Y -20.00 to 20.00, Z -21.00 to -1.00 mm</td></tr>");
    has("<tr><th>Safe heights</th><td>clearance Z 25.00, retract Z 5.00 mm</td></tr>");
    // Each tool with what sticks out, its offsets and the lowest its tip goes in any operation.
    has(
        "<td class=\"n\">3</td><td>10 mm drill <span class=\"note\">flat end mill</span></td><td class=\"n\">10.00</td><td class=\"n\">4</td><td class=\"n\">50.00</td><td class=\"n\">3</td><td class=\"n\">3</td><td class=\"n\">not used</td>",
    );
    has(
        "<td class=\"n\">4</td><td>20 mm flat end mill</td><td class=\"n\">20.00</td><td class=\"n\">4</td><td class=\"n\">50.00</td><td class=\"n\">4</td><td class=\"n\">4</td><td class=\"n\">-21.00</td>",
    );
    // The operations in order, with speed, feed, lowest Z and time; and the time in all. A kind
    // is added to a name that does not say it already (the drill above is listed as an end mill).
    has(
        "<td class=\"n\">1</td><td>Face 1</td><td class=\"n\">4</td><td class=\"n\">3183</td><td class=\"n\">637</td><td class=\"n\">-1.00</td><td class=\"n\">0:30</td>",
    );
    has(
        "<td class=\"n\">2</td><td>Contour 2</td><td class=\"n\">4</td><td class=\"n\">3183</td><td class=\"n\">637</td><td class=\"n\">-21.00</td><td class=\"n\">1:27</td>",
    );
    has("<th colspan=\"6\">Cutting time in all (rapids and tool changes come on top)</th><th class=\"n\">1:57</th>");
    has("<h2>To check</h2>\n<ul>\n<li>spindle speed capped at 10000 rpm</li>\n</ul>");
    // Nothing fetched: no link, script or image from elsewhere.
    for outside in ["http://", "https://", "<script", "<link", "<img", "url("] {
        let allowed = if outside == "http://" {
            html.matches(outside).count() == html.matches("xmlns=\"http://www.w3.org/2000/svg\"").count()
        } else {
            !html.contains(outside)
        };
        assert!(allowed, "{outside} in the sheet");
    }
    // The same sheet twice is the same page.
    assert_eq!(html, sheet().html().unwrap());
}

#[test]
fn the_picture_shows_the_stock_the_part_the_cuts_and_the_origin() {
    let html = sheet().html().unwrap();
    // The facing passes run to X ±45 and the contour to Y -30, past the stock (Y to 21): 90 by
    // 51, with a margin of 8% of the stock's longer side (4.96) all round, drawn 720 pixels wide
    // and 720 x 60.92 / 99.92 high, with Y up (so the box starts at minus its top, 25.96).
    assert!(html.contains("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"720\" height=\"439\" viewBox=\"-49.96 -25.96 99.92 60.92\""), "{html}");
    assert!(html.contains("<g transform=\"scale(1,-1)\""));
    assert!(html.contains("<rect x=\"-31.00\" y=\"-21.00\" width=\"62.00\" height=\"42.00\" fill=\"#f0ead8\""));
    assert!(html.contains("<rect x=\"-30.00\" y=\"-20.00\" width=\"60.00\" height=\"40.00\" fill=\"#ffffff\""));
    // Each operation's cuts as one path: a run that goes on from the last piece is one line, a
    // piece that starts elsewhere a new one.
    assert!(html.contains("<path d=\"M-45.00 -12.00L45.00 -12.00M45.00 0.00L-45.00 0.00\" stroke=\"#1f5fbf\""));
    assert!(html.contains("<path d=\"M0.00 -30.00L-30.00 -30.00L-40.00 -20.00L-40.00 20.00\" stroke=\"#1f5fbf\""));
    assert!(html.contains("<circle cx=\"0\" cy=\"0\"") && html.contains(">X</text>") && html.contains(">Y</text>"));
    // A cut straight down is the tool's own circle, once however many pecks went down it: two
    // holes drilled with the 10 mm drill.
    let mut drilled = sheet();
    let down = |x: f64| [x, 5.0, x, 5.0];
    drilled.operations.push(SheetOperation { cuts: vec![down(-20.0), down(-20.0), down(20.0)], ..operation("Drill 3", 3, "drill", -12.0, 0.1) });
    let html = drilled.html().unwrap();
    assert_eq!(html.matches("r=\"5.00\" stroke=\"#1f5fbf\"").count(), 2, "{html}");
    assert!(html.contains("<circle cx=\"-20.00\" cy=\"5.00\" r=\"5.00\"") && html.contains("<circle cx=\"20.00\" cy=\"5.00\" r=\"5.00\""));
    assert!(html.contains("<td>Drill 3</td>"), "the name says the kind already");
    // A ring of a thousand chords of 0.19 mm is drawn in far fewer pieces, and ends where it ends.
    let ring: Vec<[f64; 4]> = (0..1000)
        .map(|k| {
            let at = |k: usize| {
                let a = std::f64::consts::TAU * k as f64 / 1000.0;
                (30.0 * a.cos(), 30.0 * a.sin())
            };
            let ((x0, y0), (x1, y1)) = (at(k), at(k + 1));
            [x0, y0, x1, y1]
        })
        .collect();
    let mut one = sheet();
    one.operations = vec![SheetOperation { cuts: ring, ..operation("Round", 4, "contour", -5.0, 1.0) }];
    let html = one.html().unwrap();
    let path = html.lines().find(|line| line.starts_with("<path d=\"M30.00 0.00")).unwrap();
    let pieces = path.matches('L').count();
    assert!((300..=520).contains(&pieces), "{pieces} pieces");
    assert!(path.contains("L30.00 -0.00\"") || path.contains("L30.00 0.00\""), "{}", &path[path.len().saturating_sub(80)..]);
    // More pieces than a page should carry are left out, and the page is still a page.
    one.operations[0].cuts = (0..30_000).map(|k| [f64::from(k), 0.0, f64::from(k), 1.0]).collect();
    let html = one.html().unwrap();
    assert!(html.matches("L").count() <= MAX_SHEET_PIECES + 200 && html.ends_with("</html>\n"));
    // No cuts: the stock, the part and the origin all the same.
    one.operations.clear();
    let html = one.html().unwrap();
    assert!(html.contains("<svg") && !html.contains("stroke=\"#1f5fbf\"") && html.contains("viewBox=\"-35.96 -25.96 71.92 51.92\""));
}

#[test]
fn text_from_the_design_is_written_as_text() {
    let mut odd = sheet();
    odd.title = "<script>alert(1)</script>".into();
    odd.setup = "Op \"1\" & 2".into();
    odd.origin = "a <b>corner</b>".into();
    odd.material = Some("6061 & 'T6'".into());
    odd.tools[0].name = "drill\n<img src=x>".into();
    odd.operations[0].name = "Face </td>".into();
    odd.notes = vec!["x".repeat(5000), "tab\there".into()];
    odd.date = Some("<today>".into());
    let html = odd.html().unwrap();
    assert!(!html.contains("<script>") && !html.contains("<img") && !html.contains("<b>") && !html.contains("</td></td>"), "{html}");
    assert!(html.contains("Setup sheet: &lt;script&gt;alert(1)&lt;/script&gt;, Op &quot;1&quot; &amp; 2"));
    assert!(
        html.contains("G54 at a &lt;b&gt;corner&lt;/b&gt;") && html.contains(", 6061 &amp; &#39;T6&#39;</td>") && html.contains("· &lt;today&gt;")
    );
    assert!(html.contains("drill &lt;img src=x&gt;") && html.contains("Face &lt;/td&gt;") && html.contains("<li>tab here</li>"));
    // A note far too long is cut to what a page shows.
    assert!(html.contains(&format!("<li>{}</li>", "x".repeat(400))) && !html.contains(&"x".repeat(401)));
}

#[test]
fn inches_times_and_what_is_missing_are_written_as_a_shop_reads_them() {
    let mut inch = sheet();
    inch.units = LengthUnit::Inch;
    inch.date = None;
    let html = inch.html().unwrap();
    assert!(html.contains("<p class=\"sub\">Work offset G54</p>"));
    // 62 x 42 x 21 mm; 636.6 mm/min is 25.1 inches a minute; Z -21 mm is -0.827 in.
    assert!(html.contains("<td>2.441 × 1.654 × 0.827 in, aluminium</td>"), "{html}");
    assert!(html.contains("<th class=\"n\">Feed, in/min</th>") && html.contains("<td class=\"n\">25.1</td><td class=\"n\">-0.827</td>"));
    // An operation with no toolpath is said to be out of the program, with the reason; its
    // numbers are dashes and it adds no time.
    let mut partial = sheet();
    partial.operations[1] = SheetOperation {
        rpm: None,
        feed: None,
        lowest_z: None,
        minutes: None,
        trouble: Some("the part is wider above".into()),
        ..operation("Contour 2", 4, "contour", 0.0, 0.0)
    };
    let html = partial.html().unwrap();
    assert!(html.contains("Contour 2<br><span class=\"bad\">Not in the program: the part is wider above</span></td><td class=\"n\">4</td><td class=\"n\">–</td><td class=\"n\">–</td><td class=\"n\">–</td><td class=\"n\">–</td>"), "{html}");
    assert!(
        html.contains("<th class=\"n\">0:30</th>") && html.contains("<td class=\"n\">-1.00</td></tr>\n</table>"),
        "T4's lowest is now the facing's"
    );
    // A tool whose only operation has no toolpath has no lowest Z to give; it is not "not used".
    partial.operations[1].tool = 3;
    let html = partial.html().unwrap();
    assert!(html.contains("<td class=\"n\">3</td><td class=\"n\">3</td><td class=\"n\">–</td></tr>") && !html.contains("not used"), "{html}");
    partial.operations[1].tool = 4;
    // Another work offset.
    partial.work_offset = 3;
    assert!(partial.html().unwrap().contains("Work offset G56") && partial.html().unwrap().contains("G56 at the middle"));
}

#[test]
fn a_tool_that_goes_deeper_than_it_sticks_out_is_pointed_out() {
    // T4 goes 21 mm below the top of the stock: with 20 mm out of its holder that is a line to
    // check, with 21 or more it is not.
    let mut short = sheet();
    short.tools[1].stick_out = 20.0;
    let html = short.html().unwrap();
    assert!(
        html.contains("<li>T4 goes 21.00 mm below the top of the stock and sticks out 20.00 mm from its holder: the holder would reach the stock unless it is clear beside the cut.</li>"),
        "{html}"
    );
    short.tools[1].stick_out = 21.0;
    assert!(!short.html().unwrap().contains("the holder would reach"));
    // With nothing to check there is no such heading.
    let mut clean = sheet();
    clean.notes.clear();
    assert!(!clean.html().unwrap().contains("To check"));
}

#[test]
fn hostile_sheets_are_refused_never_written() {
    let bad = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY];
    for &v in &bad {
        let mut s = sheet();
        s.placement.stock.max.x = v;
        assert!(s.html().is_err());
        let mut s = sheet();
        s.placement.clearance_z = v;
        assert!(s.html().is_err());
        let mut s = sheet();
        s.tools[0].stick_out = v;
        assert!(s.html().is_err());
        let mut s = sheet();
        s.operations[0].minutes = Some(v);
        assert!(s.html().is_err());
        let mut s = sheet();
        s.operations[0].cuts.push([0.0, v, 1.0, 1.0]);
        assert!(s.html().is_err());
    }
    // Huge but finite numbers make a page, not a panic.
    for v in [1e300, -1e300, f64::MAX, f64::MIN, 5e-324, 0.0] {
        let mut s = sheet();
        (s.placement.stock.max.x, s.placement.model.min.y, s.tools[0].diameter, s.operations[1].lowest_z) = (v, v, v, Some(v));
        s.operations[0].cuts.push([v, 0.0, 0.0, v]);
        let _ = s.html();
    }
    let mut s = sheet();
    s.work_offset = 0;
    assert!(s.html().unwrap_err().to_string().contains("work offset 0"));
    s.work_offset = 7;
    assert!(s.html().is_err());
    let mut s = sheet();
    s.notes = vec![String::new(); MAX_SHEET_ROWS + 1];
    assert!(s.html().unwrap_err().to_string().contains("at most 10000"));
}

#[test]
fn a_kind_is_said_in_the_words_its_name_does_not_have() {
    let mut named = sheet();
    named.operations = vec![
        operation("Contour 2", 4, "inside contour", -21.0, 1.0),
        operation("Bore 3", 4, "inside contour", -9.0, 1.0),
        operation("Tap M6", 4, "tap", -13.0, 1.0),
        operation("Holes", 4, "drill", -13.0, 1.0),
    ];
    let html = named.html().unwrap();
    for row in [
        "<td>Contour 2 <span class=\"note\">inside</span></td>",
        "<td>Bore 3 <span class=\"note\">inside contour</span></td>",
        "<td>Tap M6</td>",
        "<td>Holes <span class=\"note\">drill</span></td>",
    ] {
        assert!(html.contains(row), "no {row:?} in:\n{html}");
    }
}
