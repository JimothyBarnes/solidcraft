//! The setup sheet (decision 0019 of the CAM work): what the person at the machine needs to set
//! a job up and to check a program against, as one page of HTML that needs nothing else to show
//! or print. It is made from numbers the engine has already worked out; nothing is measured or
//! decided here.
//!
//! A sheet says where the work origin is and how big the stock is, which tools to load with
//! what sticking out and where each one's tip gets lowest, the operations in the order they run
//! with their speeds, feeds and times, and shows the cuts from above. It ends as every program
//! begins: unverified, to be proved out on the machine.

use std::fmt::Write as _;

use crate::placement::Placement;
use crate::units::LengthUnit;
use crate::{Error, Result};

/// The most tools and operations a sheet lists, and the most cutting pieces its picture draws.
pub const MAX_SHEET_ROWS: usize = 10_000;
pub const MAX_SHEET_PIECES: usize = 20_000;
/// The longest text taken for one name or note, in characters.
const MAX_TEXT: usize = 400;
/// How wide the picture is drawn, in CSS pixels, and the least and most it is scaled by a mm.
const PICTURE_WIDTH: f64 = 720.0;
/// The margin round the stock in the picture, as a share of its longer side.
const PICTURE_MARGIN: f64 = 0.08;
/// Pieces of a cut shorter than this are drawn joined to the next, in the picture's own units
/// (mm): a ring of a thousand chords is one line to the eye.
const PICTURE_STEP: f64 = 0.2;

/// One tool as the sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetTool {
    pub number: u32,
    pub name: String,
    /// Its kind in words ("flat end mill").
    pub kind: String,
    /// mm.
    pub diameter: f64,
    pub flutes: u32,
    /// From the holder's lower face to the tool's tip, mm.
    pub stick_out: f64,
    pub length_offset: u32,
    pub diameter_offset: u32,
}

/// One operation as the sheet lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetOperation {
    pub name: String,
    pub tool: u32,
    /// Its kind in words ("pocket").
    pub kind: String,
    /// What it runs at, where its strategy says: rev/min and mm/min.
    pub rpm: Option<f64>,
    pub feed: Option<f64>,
    /// The lowest Z its tool's tip reaches, in the WCS, and its cutting time in minutes: `None`
    /// when it has no toolpath.
    pub lowest_z: Option<f64>,
    pub minutes: Option<f64>,
    /// Why it has no toolpath, when it has none: it is not in the program.
    pub trouble: Option<String>,
    /// Its cutting moves seen from above, as straight pieces `[x0, y0, x1, y1]` in the WCS, mm.
    pub cuts: Vec<[f64; 4]>,
}

/// Everything a setup sheet says.
#[derive(Clone, Debug, PartialEq)]
pub struct SetupSheet {
    /// The design's name.
    pub title: String,
    /// The setup's name.
    pub setup: String,
    /// The day it was made, as the caller writes it; left out when `None`.
    pub date: Option<String>,
    /// The unit lengths are written in.
    pub units: LengthUnit,
    /// The stock's material, when the setup names one.
    pub material: Option<String>,
    /// 1 to 6, for G54 to G59.
    pub work_offset: u8,
    /// Where the work origin is, in words ("the middle of the stock's top").
    pub origin: String,
    /// Where the setup sits on the model: the stock, the part's box and the safe heights.
    pub placement: Placement,
    pub tools: Vec<SheetTool>,
    /// In the order they run.
    pub operations: Vec<SheetOperation>,
    /// Things worth a line of their own (what a strategy or the simulation noted).
    pub notes: Vec<String>,
}

/// `text` safe between tags and in attributes, cut to a length a sheet can show.
fn escaped(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars().take(MAX_TEXT) {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// A kind in words after a name, less the words the name has already: "8 mm Drill" is a drill
/// and says no more, and a "Contour 2" that goes round the inside says only "inside".
fn kind_after(name: &str, kind: &str) -> String {
    let name = name.to_lowercase();
    let unsaid: Vec<&str> = kind.split_whitespace().filter(|word| !name.contains(&word.to_lowercase())).collect();
    if unsaid.is_empty() { String::new() } else { format!(" <span class=\"note\">{}</span>", escaped(&unsaid.join(" "))) }
}

/// Minutes as `min:s`, to the second.
fn minutes_seconds(minutes: f64) -> String {
    let seconds = (minutes * 60.0).round().clamp(0.0, 1e9) as u64;
    format!("{}:{:02}", seconds / 60, seconds % 60)
}

impl SetupSheet {
    /// A length in the sheet's unit, without the unit: two decimals of a millimetre, three of
    /// an inch.
    fn length(&self, mm: f64) -> String {
        match self.units {
            LengthUnit::Millimetre => format!("{:.2}", mm + 0.0),
            LengthUnit::Inch => format!("{:.3}", self.units.from_mm(mm) + 0.0),
        }
    }

    fn unit(&self) -> &'static str {
        match self.units {
            LengthUnit::Millimetre => "mm",
            LengthUnit::Inch => "in",
        }
    }

    /// A feed in the sheet's unit a minute, without the unit.
    fn feed(&self, mm_per_minute: f64) -> String {
        match self.units {
            LengthUnit::Millimetre => format!("{mm_per_minute:.0}"),
            LengthUnit::Inch => format!("{:.1}", self.units.from_mm(mm_per_minute)),
        }
    }

    /// Checks the sheet's numbers: finite, and not more rows or pieces than a page can hold.
    fn check(&self) -> Result<()> {
        let bad = |m: String| Err(Error::BadValue(m));
        if self.tools.len() > MAX_SHEET_ROWS || self.operations.len() > MAX_SHEET_ROWS || self.notes.len() > MAX_SHEET_ROWS {
            return bad(format!("a setup sheet lists at most {MAX_SHEET_ROWS} tools, operations and notes"));
        }
        if !(1..=6).contains(&self.work_offset) {
            return bad(format!("work offset {} (1 to 6 for G54 to G59)", self.work_offset));
        }
        let p = &self.placement;
        let boxes = [p.stock.min, p.stock.max, p.model.min, p.model.max];
        let finite = |v: f64| v.is_finite();
        if !boxes.iter().all(|c| finite(c.x) && finite(c.y) && finite(c.z)) || !finite(p.clearance_z) || !finite(p.retract_z) {
            return bad("the setup's stock, part and safe heights must be finite".into());
        }
        let tools = self.tools.iter().all(|t| finite(t.diameter) && finite(t.stick_out));
        let operations = self.operations.iter().all(|o| {
            [o.rpm, o.feed, o.lowest_z, o.minutes].iter().all(|v| v.is_none_or(finite)) && o.cuts.iter().all(|piece| piece.iter().all(|v| finite(*v)))
        });
        if !tools || !operations {
            return bad("a setup sheet's numbers must be finite".into());
        }
        Ok(())
    }

    /// The lowest Z the tip of tool `number` reaches in any operation, if any uses it.
    fn lowest(&self, number: u32) -> Option<f64> {
        self.operations.iter().filter(|o| o.tool == number).filter_map(|o| o.lowest_z).min_by(f64::total_cmp)
    }

    /// The sheet as one page of HTML: its own style, its picture drawn in it, nothing fetched.
    pub fn html(&self) -> Result<String> {
        self.check()?;
        let p = &self.placement;
        let unit = self.unit();
        let size = |lo: f64, hi: f64| self.length(hi - lo);
        let span = |lo: f64, hi: f64| format!("{} to {}", self.length(lo), self.length(hi));
        let offset = format!("G{}", 53 + u32::from(self.work_offset));
        let mut h = String::new();
        let title = format!("Setup sheet: {}, {}", escaped(&self.title), escaped(&self.setup));
        // Writing to a String never fails; the results are dropped on that ground alone.
        let _ = write!(
            h,
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{title}</title>\n<style>\n{STYLE}</style>\n</head>\n<body>\n<h1>{title}</h1>\n"
        );
        let date = self.date.as_deref().map(|d| format!(" · {}", escaped(d))).unwrap_or_default();
        let _ = writeln!(h, "<p class=\"sub\">Work offset {offset}{date}</p>");
        let _ = writeln!(h, "<p class=\"warn\">Unverified program. Prove it out on the machine first: air cut, then single block.</p>");

        let _ = writeln!(h, "<h2>Stock and work origin</h2>\n<table>");
        let material = self.material.as_deref().map(|m| format!(", {}", escaped(m))).unwrap_or_default();
        let _ = writeln!(
            h,
            "<tr><th>Stock</th><td>{} × {} × {} {unit}{material}</td></tr>",
            size(p.stock.min.x, p.stock.max.x),
            size(p.stock.min.y, p.stock.max.y),
            size(p.stock.min.z, p.stock.max.z)
        );
        let _ = writeln!(
            h,
            "<tr><th>Part</th><td>{} × {} × {} {unit}</td></tr>",
            size(p.model.min.x, p.model.max.x),
            size(p.model.min.y, p.model.max.y),
            size(p.model.min.z, p.model.max.z)
        );
        let _ = writeln!(h, "<tr><th>Work origin</th><td>{offset} at {}</td></tr>", escaped(&self.origin));
        for (what, b) in [("Stock from the origin", &p.stock), ("Part from the origin", &p.model)] {
            let _ = writeln!(
                h,
                "<tr><th>{what}</th><td>X {}, Y {}, Z {} {unit}</td></tr>",
                span(b.min.x, b.max.x),
                span(b.min.y, b.max.y),
                span(b.min.z, b.max.z)
            );
        }
        let _ = writeln!(
            h,
            "<tr><th>Safe heights</th><td>clearance Z {}, retract Z {} {unit}</td></tr>\n</table>",
            self.length(p.clearance_z),
            self.length(p.retract_z)
        );
        self.picture(&mut h);

        let _ = writeln!(
            h,
            "<h2>Tools</h2>\n<table>\n<tr><th class=\"n\">T</th><th>Tool</th><th class=\"n\">Diameter, {unit}</th><th class=\"n\">Flutes</th><th class=\"n\">Sticks out, {unit}</th><th class=\"n\">H</th><th class=\"n\">D</th><th class=\"n\">Lowest Z, {unit}</th></tr>"
        );
        for t in &self.tools {
            // No operation names it, or those that do have no toolpath.
            let used = self.operations.iter().any(|o| o.tool == t.number);
            let lowest = self.lowest(t.number).map_or_else(|| if used { "–" } else { "not used" }.to_string(), |z| self.length(z));
            let _ = writeln!(
                h,
                "<tr><td class=\"n\">{}</td><td>{}{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{lowest}</td></tr>",
                t.number,
                escaped(&t.name),
                kind_after(&t.name, &t.kind),
                self.length(t.diameter),
                t.flutes,
                self.length(t.stick_out),
                t.length_offset,
                t.diameter_offset
            );
        }
        let _ = writeln!(h, "</table>");

        let _ = writeln!(
            h,
            "<h2>Operations</h2>\n<table>\n<tr><th class=\"n\">#</th><th>Operation</th><th class=\"n\">T</th><th class=\"n\">Speed, rev/min</th><th class=\"n\">Feed, {unit}/min</th><th class=\"n\">Lowest Z, {unit}</th><th class=\"n\">Cutting time, min:s</th></tr>"
        );
        let dash = || "–".to_string();
        for (k, o) in self.operations.iter().enumerate() {
            let trouble =
                o.trouble.as_deref().map(|why| format!("<br><span class=\"bad\">Not in the program: {}</span>", escaped(why))).unwrap_or_default();
            let _ = writeln!(
                h,
                "<tr><td class=\"n\">{}</td><td>{}{}{trouble}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td><td class=\"n\">{}</td></tr>",
                k + 1,
                escaped(&o.name),
                kind_after(&o.name, &o.kind),
                o.tool,
                o.rpm.map_or_else(dash, |v| format!("{v:.0}")),
                o.feed.map_or_else(dash, |v| self.feed(v)),
                o.lowest_z.map_or_else(dash, |v| self.length(v)),
                o.minutes.map_or_else(dash, minutes_seconds)
            );
        }
        let total: f64 = self.operations.iter().filter_map(|o| o.minutes).sum();
        let _ = writeln!(
            h,
            "<tr><th colspan=\"6\">Cutting time in all (rapids and tool changes come on top)</th><th class=\"n\">{}</th></tr>\n</table>",
            minutes_seconds(total)
        );

        let notes: Vec<String> = self.reach_notes().into_iter().chain(self.notes.iter().map(|n| escaped(n))).collect();
        if !notes.is_empty() {
            let _ = writeln!(h, "<h2>To check</h2>\n<ul>");
            for note in notes {
                let _ = writeln!(h, "<li>{note}</li>");
            }
            let _ = writeln!(h, "</ul>");
        }
        let _ = writeln!(h, "</body>\n</html>");
        Ok(h)
    }

    /// A line for each tool that goes further below the top of the stock than it sticks out of
    /// its holder: the holder would be in the stock, unless the cut is beside it.
    fn reach_notes(&self) -> Vec<String> {
        let top = self.placement.stock.max.z;
        let unit = self.unit();
        self.tools
            .iter()
            .filter_map(|t| {
                let down = top - self.lowest(t.number)?;
                (down > t.stick_out + 1e-9).then(|| {
                    format!(
                        "T{} goes {} {unit} below the top of the stock and sticks out {} {unit} from its holder: the holder would reach the stock unless it is clear beside the cut.",
                        t.number,
                        self.length(down),
                        self.length(t.stick_out)
                    )
                })
            })
            .collect()
    }

    /// The setup from above, drawn in the page: the stock, the part's box, each operation's cuts
    /// and the work origin with its X and Y.
    fn picture(&self, h: &mut String) {
        let (stock, model) = (&self.placement.stock, &self.placement.model);
        let (wide, deep) = (stock.max.x - stock.min.x, stock.max.y - stock.min.y);
        // Everything the picture shows: the stock, the origin, the cuts.
        let mut lo = [stock.min.x.min(0.0), stock.min.y.min(0.0)];
        let mut hi = [stock.max.x.max(0.0), stock.max.y.max(0.0)];
        for piece in self.operations.iter().flat_map(|o| &o.cuts) {
            (lo[0], lo[1]) = (lo[0].min(piece[0]).min(piece[2]), lo[1].min(piece[1]).min(piece[3]));
            (hi[0], hi[1]) = (hi[0].max(piece[0]).max(piece[2]), hi[1].max(piece[1]).max(piece[3]));
        }
        let margin = PICTURE_MARGIN * wide.max(deep).max(1.0);
        let (x0, y0, w, d) = (lo[0] - margin, lo[1] - margin, hi[0] - lo[0] + 2.0 * margin, hi[1] - lo[1] + 2.0 * margin);
        if !(w > 0.0 && d > 0.0 && w.is_finite() && d.is_finite()) {
            return;
        }
        let height = (PICTURE_WIDTH * d / w).clamp(1.0, 4.0 * PICTURE_WIDTH);
        // Y up, as on the machine: drawn mirrored, in millimetres.
        let thin = w / PICTURE_WIDTH;
        let _ = writeln!(
            h,
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{PICTURE_WIDTH:.0}\" height=\"{height:.0}\" viewBox=\"{x0:.2} {:.2} {w:.2} {d:.2}\" role=\"img\" aria-label=\"The setup from above\">",
            -(y0 + d)
        );
        let _ = writeln!(h, "<g transform=\"scale(1,-1)\" fill=\"none\" stroke-linecap=\"round\" stroke-linejoin=\"round\">");
        let rect = |b: &crate::placement::Bounds| {
            format!("x=\"{:.2}\" y=\"{:.2}\" width=\"{:.2}\" height=\"{:.2}\"", b.min.x, b.min.y, b.max.x - b.min.x, b.max.y - b.min.y)
        };
        let _ = writeln!(h, "<rect {} fill=\"#f0ead8\" stroke=\"#8a7a4a\" stroke-width=\"{:.3}\"/>", rect(stock), 1.5 * thin);
        let _ = writeln!(
            h,
            "<rect {} fill=\"#ffffff\" stroke=\"#444\" stroke-width=\"{:.3}\" stroke-dasharray=\"{:.2} {:.2}\"/>",
            rect(model),
            thin,
            4.0 * thin,
            3.0 * thin
        );
        let mut left = MAX_SHEET_PIECES;
        let near = |a: [f64; 2], b: [f64; 2], within: f64| (a[0] - b[0]).hypot(a[1] - b[1]) <= within;
        for o in &self.operations {
            let mut path = String::new();
            // Where the line drawn so far ends, and where the cut has got to: pieces too short to
            // see run on into the next, and a run is always drawn to its own end.
            let (mut pen, mut end): (Option<[f64; 2]>, Option<[f64; 2]>) = (None, None);
            let finish = |path: &mut String, pen: Option<[f64; 2]>, end: Option<[f64; 2]>| {
                if let (Some(pen), Some(end)) = (pen, end)
                    && !near(pen, end, 1e-9)
                {
                    let _ = write!(path, "L{:.2} {:.2}", end[0], end[1]);
                }
            };
            // A cut straight down shows as the tool's own circle: a drilled hole, where a tool
            // went in.
            let radius = self.tools.iter().find(|t| t.number == o.tool).map_or(0.0, |t| 0.5 * t.diameter);
            let mut holes: Vec<[f64; 2]> = Vec::new();
            for piece in &o.cuts {
                let (from, to) = ([piece[0], piece[1]], [piece[2], piece[3]]);
                if near(from, to, 1e-9) {
                    if radius > 0.0 && left > 0 && !holes.iter().any(|hole| near(*hole, from, 1e-6)) {
                        holes.push(from);
                        left -= 1;
                    }
                    continue;
                }
                if !end.is_some_and(|end| near(end, from, 1e-6)) {
                    // Another run of the cut.
                    finish(&mut path, pen, end);
                    if left == 0 {
                        (pen, end) = (None, None);
                        break;
                    }
                    let _ = write!(path, "M{:.2} {:.2}", from[0], from[1]);
                    pen = Some(from);
                }
                if left > 0 && pen.is_some_and(|pen| !near(pen, to, PICTURE_STEP)) {
                    let _ = write!(path, "L{:.2} {:.2}", to[0], to[1]);
                    left -= 1;
                    pen = Some(to);
                }
                end = Some(to);
            }
            finish(&mut path, pen, end);
            if !path.is_empty() {
                let _ = writeln!(h, "<path d=\"{path}\" stroke=\"#1f5fbf\" stroke-width=\"{:.3}\"/>", 0.8 * thin);
            }
            for hole in holes {
                let _ = writeln!(
                    h,
                    "<circle cx=\"{:.2}\" cy=\"{:.2}\" r=\"{radius:.2}\" stroke=\"#1f5fbf\" stroke-width=\"{:.3}\"/>",
                    hole[0],
                    hole[1],
                    0.8 * thin
                );
            }
        }
        // The work origin: X to the right, Y up.
        let arm = 0.12 * wide.max(deep).max(1.0);
        let _ = writeln!(h, "<path d=\"M0 0L{arm:.2} 0\" stroke=\"#c8102e\" stroke-width=\"{:.3}\"/>", 2.0 * thin);
        let _ = writeln!(h, "<path d=\"M0 0L0 {arm:.2}\" stroke=\"#1a8f3a\" stroke-width=\"{:.3}\"/>", 2.0 * thin);
        let _ = writeln!(h, "<circle cx=\"0\" cy=\"0\" r=\"{:.2}\" fill=\"#111\" stroke=\"none\"/>", 3.0 * thin);
        let _ = writeln!(h, "</g>");
        let size = 14.0 * thin;
        let _ = writeln!(
            h,
            "<text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{size:.2}\" fill=\"#c8102e\">X</text><text x=\"{:.2}\" y=\"{:.2}\" font-size=\"{size:.2}\" fill=\"#1a8f3a\">Y</text>",
            arm + 0.4 * size,
            0.35 * size,
            -0.35 * size,
            -(arm + 0.4 * size)
        );
        let _ = writeln!(
            h,
            "</svg>\n<p class=\"note\">From above: the stock, the part's box (dashed), the cuts (a circle where a tool goes straight down), and the work origin.</p>"
        );
    }
}

/// The page's own style: plain, and fit to print.
const STYLE: &str = "body{font:14px/1.45 system-ui,sans-serif;margin:24px;color:#111;max-width:900px}
h1{font-size:20px;margin:0 0 2px}
h2{font-size:15px;margin:22px 0 6px;padding-bottom:2px;border-bottom:1px solid #999}
table{border-collapse:collapse;width:100%}
th,td{text-align:left;padding:3px 8px;border-bottom:1px solid #ddd;vertical-align:top;font-weight:400}
th{color:#333;font-weight:600}
td.n,th.n{text-align:right;font-variant-numeric:tabular-nums;white-space:nowrap}
.sub{margin:0 0 10px;color:#444}
.warn{border:2px solid #b00020;padding:8px 10px;margin:12px 0;font-weight:600}
.note{color:#555}
.bad{color:#b00020}
svg{max-width:100%;height:auto;border:1px solid #ccc;margin-top:10px;background:#fafafa}
@media print{body{margin:10mm}}
";

#[cfg(test)]
#[path = "sheet_tests.rs"]
mod tests;
