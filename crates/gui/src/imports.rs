//! Outline imports for both note editors: SVG files and typed glyphs
//! become note contours in the unit frame (centred, longest half-extent
//! 1). The kernel never touches fonts or files; these commands resolve
//! them once and the outline is stored in the design.
//!
//! SVG: every visible path contributes its fill and its stroke. A stroke
//! is outlined at its width with its caps, joins and dashes, so a line
//! icon imports as the bands it draws. Paths group into layers by paint
//! colour in order of first appearance: the kit makes one part per layer
//! (`kit::import::layer_modes` decides Add or Cut, so a figure on a badge
//! shows as a hole, in the game too), the old designer takes every layer
//! merged into one outline.

use rhythia_render::kit::doc::Mode;
use rhythia_render::kit::import::layer_modes;
use rhythia_render::kit::v1::clean_contours;
use serde::Serialize;

/// One colour of an SVG as the kit receives it.
#[derive(Serialize, Debug, Clone, PartialEq)]
pub struct SvgLayer {
    pub contours: Vec<Vec<[f32; 2]>>,
    pub mode: Mode,
    /// The paint as written, `#rrggbb` for a colour; a label for the part.
    pub paint: String,
}

pub const NO_PATHS: &str = "the SVG has no filled or stroked paths (text is not imported)";

fn err_str<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Splits a kurbo path into flat contours at an adaptive tolerance: the
/// shared tail of every import route. Coordinates are kept; the caller
/// centres and scales.
pub(crate) fn contours_from_bezpath(path: &kurbo::BezPath) -> Vec<Vec<[f32; 2]>> {
    let mut extent = 0.0f64;
    for el in path.elements() {
        let pts: &[kurbo::Point] = match el {
            kurbo::PathEl::MoveTo(p) | kurbo::PathEl::LineTo(p) => std::slice::from_ref(p),
            kurbo::PathEl::QuadTo(a, b) => {
                extent = extent.max(a.x.abs()).max(a.y.abs());
                std::slice::from_ref(b)
            }
            kurbo::PathEl::CurveTo(a, b, c) => {
                extent = extent
                    .max(a.x.abs())
                    .max(a.y.abs())
                    .max(b.x.abs())
                    .max(b.y.abs());
                std::slice::from_ref(c)
            }
            kurbo::PathEl::ClosePath => &[],
        };
        for p in pts {
            extent = extent.max(p.x.abs()).max(p.y.abs());
        }
    }
    // A 256 unit disc flattens to about 80 segments: no visible facets on
    // a note, far below the part budget.
    let tol = extent.max(1.0) * 0.0005;
    let mut contours: Vec<Vec<[f32; 2]>> = Vec::new();
    let mut current: Vec<[f32; 2]> = Vec::new();
    kurbo::flatten(path.elements().iter().copied(), tol, |el| match el {
        kurbo::PathEl::MoveTo(p) => {
            if current.len() >= 3 {
                contours.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            current.push([p.x as f32, p.y as f32]);
        }
        kurbo::PathEl::LineTo(p) => current.push([p.x as f32, p.y as f32]),
        kurbo::PathEl::ClosePath => {
            if current.len() >= 3 {
                contours.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
        }
        _ => {}
    });
    if current.len() >= 3 {
        contours.push(current);
    }
    contours
}

/// Centres a set of contour groups on the origin and scales the longest
/// half-extent to 1, with ONE frame for all groups, so the layers of an
/// SVG keep their places relative to each other.
fn centre_groups(groups: &mut [Vec<Vec<[f32; 2]>>]) {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for p in groups.iter().flatten().flatten() {
        min_x = min_x.min(p[0]);
        min_y = min_y.min(p[1]);
        max_x = max_x.max(p[0]);
        max_y = max_y.max(p[1]);
    }
    if min_x > max_x {
        return;
    }
    let (cx, cy) = ((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
    let half = ((max_x - min_x).max(max_y - min_y) * 0.5).max(1e-6);
    for p in groups.iter_mut().flatten().flatten() {
        p[0] = (p[0] - cx) / half;
        p[1] = (p[1] - cy) / half;
    }
}

/// Centres one outline: imports land in the editor the size a note should
/// be instead of wherever the source's coordinate system put them.
pub(crate) fn centre_contours(contours: Vec<Vec<[f32; 2]>>) -> Vec<Vec<[f32; 2]>> {
    let mut groups = [contours];
    centre_groups(&mut groups);
    let [c] = groups;
    c
}

fn paint_key(p: &usvg::Paint) -> String {
    match p {
        usvg::Paint::Color(c) => format!("#{:02x}{:02x}{:02x}", c.red, c.green, c.blue),
        usvg::Paint::LinearGradient(g) => format!("gradient {}", g.id()),
        usvg::Paint::RadialGradient(g) => format!("gradient {}", g.id()),
        usvg::Paint::Pattern(g) => format!("pattern {}", g.id()),
    }
}

/// The region a stroke paints, as a path to fill under nonzero: dashes
/// first, then the outline at the stroke's width with its caps and joins.
/// In the path's own units; the transform only sets the curve resolution.
fn stroke_outline(
    path: &tiny_skia_path::Path,
    s: &usvg::Stroke,
    t: &tiny_skia_path::Transform,
) -> Option<tiny_skia_path::Path> {
    let res = tiny_skia_path::PathStroker::compute_resolution_scale(t);
    let dashed = s
        .dasharray()
        .and_then(|d| tiny_skia_path::StrokeDash::new(d.to_vec(), s.dashoffset()))
        .and_then(|dash| path.dash(&dash, res));
    let source = dashed.as_ref().unwrap_or(path);
    let stroke = tiny_skia_path::Stroke {
        width: s.width().get(),
        miter_limit: s.miterlimit().get(),
        line_cap: match s.linecap() {
            usvg::LineCap::Butt => tiny_skia_path::LineCap::Butt,
            usvg::LineCap::Round => tiny_skia_path::LineCap::Round,
            usvg::LineCap::Square => tiny_skia_path::LineCap::Square,
        },
        line_join: match s.linejoin() {
            usvg::LineJoin::Miter => tiny_skia_path::LineJoin::Miter,
            usvg::LineJoin::MiterClip => tiny_skia_path::LineJoin::MiterClip,
            usvg::LineJoin::Round => tiny_skia_path::LineJoin::Round,
            usvg::LineJoin::Bevel => tiny_skia_path::LineJoin::Bevel,
        },
        dash: None,
    };
    source.stroke(&stroke, res)
}

/// A tiny-skia path through the node's transform into flat, oriented
/// contours, y up as the note plane is.
fn path_contours(
    path: &tiny_skia_path::Path,
    t: &tiny_skia_path::Transform,
    even_odd: bool,
) -> Vec<Vec<[f32; 2]>> {
    let map = |pt: tiny_skia_path::Point| {
        let x = t.sx * pt.x + t.kx * pt.y + t.tx;
        let y = t.ky * pt.x + t.sy * pt.y + t.ty;
        kurbo::Point::new(x as f64, -y as f64)
    };
    let mut bez = kurbo::BezPath::new();
    for seg in path.segments() {
        use tiny_skia_path::PathSegment as S;
        match seg {
            S::MoveTo(a) => bez.move_to(map(a)),
            S::LineTo(a) => bez.line_to(map(a)),
            S::QuadTo(a, b) => bez.quad_to(map(a), map(b)),
            S::CubicTo(a, b, c) => bez.curve_to(map(a), map(b), map(c)),
            S::Close => bez.close_path(),
        }
    }
    clean_contours(&contours_from_bezpath(&bez), even_odd)
}

/// Layers in paint order, keyed by paint; a path's fill and stroke land
/// in their own colours' layers.
fn collect_layers(group: &usvg::Group, layers: &mut Vec<(String, Vec<Vec<[f32; 2]>>)>) {
    let add = |layers: &mut Vec<(String, Vec<Vec<[f32; 2]>>)>,
               key: String,
               contours: Vec<Vec<[f32; 2]>>| {
        if contours.is_empty() {
            return;
        }
        match layers.iter_mut().find(|(k, _)| *k == key) {
            Some((_, list)) => list.extend(contours),
            None => layers.push((key, contours)),
        }
    };
    for node in group.children() {
        match node {
            usvg::Node::Group(g) => collect_layers(g, layers),
            usvg::Node::Path(p) => {
                if !p.is_visible() {
                    continue;
                }
                let t = p.abs_transform();
                let fill = p.fill().filter(|f| f.opacity().get() > 0.0).map(|f| {
                    let even_odd = matches!(f.rule(), usvg::FillRule::EvenOdd);
                    (paint_key(f.paint()), path_contours(p.data(), &t, even_odd))
                });
                let stroke = p
                    .stroke()
                    .filter(|s| s.opacity().get() > 0.0)
                    .and_then(|s| {
                        stroke_outline(p.data(), s, &t).map(|outline| {
                            (paint_key(s.paint()), path_contours(&outline, &t, false))
                        })
                    });
                let stroke_first = matches!(p.paint_order(), usvg::PaintOrder::StrokeAndFill);
                let (first, second) = if stroke_first {
                    (stroke, fill)
                } else {
                    (fill, stroke)
                };
                for (key, contours) in [first, second].into_iter().flatten() {
                    add(layers, key, contours);
                }
            }
            _ => {}
        }
    }
}

/// The SVG import core: one layer per paint colour in paint order, every
/// layer's shapes merged under nonzero, all in one shared unit frame, with
/// Add or Cut decided by what lies under each layer.
pub fn svg_layers(data: &[u8]) -> Result<Vec<SvgLayer>, String> {
    let tree = usvg::Tree::from_data(data, &usvg::Options::default())
        .map_err(|e| format!("not a readable SVG: {e}"))?;
    let mut raw: Vec<(String, Vec<Vec<[f32; 2]>>)> = Vec::new();
    collect_layers(tree.root(), &mut raw);
    // Same-colour shapes of one layer merge into one region, so the layer
    // rule judges the colour's whole area, not every path on its own.
    let mut groups: Vec<Vec<Vec<[f32; 2]>>> =
        raw.iter().map(|(_, c)| clean_contours(c, false)).collect();
    groups.retain(|g| !g.is_empty());
    let paints: Vec<String> = raw
        .into_iter()
        .map(|(k, c)| (k, clean_contours(&c, false)))
        .filter(|(_, c)| !c.is_empty())
        .map(|(k, _)| k)
        .collect();
    if groups.is_empty() {
        return Err(NO_PATHS.into());
    }
    centre_groups(&mut groups);
    let modes = layer_modes(&groups).map_err(|e| e.to_string())?;
    Ok(groups
        .into_iter()
        .zip(modes)
        .zip(paints)
        .map(|((contours, mode), paint)| SvgLayer {
            contours,
            mode,
            paint,
        })
        .collect())
}

/// Every layer merged: the old designer's single outline.
pub fn svg_to_contours(data: &[u8]) -> Result<Vec<Vec<[f32; 2]>>, String> {
    let layers = svg_layers(data)?;
    Ok(layers.into_iter().flat_map(|l| l.contours).collect())
}

/// Outlines a typed character with the first system font that covers
/// it. Regular glyphs outline cleanly; colour emoji fonts carry no
/// outlines and report as not found.
pub fn glyph_contours(ch: &str) -> Result<Vec<Vec<[f32; 2]>>, String> {
    let c = ch.trim().chars().next().ok_or("type a character first")?;
    struct B {
        bez: kurbo::BezPath,
    }
    impl ttf_parser::OutlineBuilder for B {
        fn move_to(&mut self, x: f32, y: f32) {
            self.bez.move_to((x as f64, y as f64));
        }
        fn line_to(&mut self, x: f32, y: f32) {
            self.bez.line_to((x as f64, y as f64));
        }
        fn quad_to(&mut self, x1: f32, y1: f32, x: f32, y: f32) {
            self.bez
                .quad_to((x1 as f64, y1 as f64), (x as f64, y as f64));
        }
        fn curve_to(&mut self, x1: f32, y1: f32, x2: f32, y2: f32, x: f32, y: f32) {
            self.bez.curve_to(
                (x1 as f64, y1 as f64),
                (x2 as f64, y2 as f64),
                (x as f64, y as f64),
            );
        }
        fn close(&mut self) {
            self.bez.close_path();
        }
    }
    let mut db = fontdb::Database::new();
    db.load_system_fonts();
    let mut ids: Vec<fontdb::ID> = Vec::new();
    // Prefer a plain sans face so letters look like letters; fall back to
    // anything that covers the character.
    if let Some(id) = db.query(&fontdb::Query {
        families: &[fontdb::Family::SansSerif],
        ..Default::default()
    }) {
        ids.push(id);
    }
    ids.extend(db.faces().map(|f| f.id));
    for id in ids {
        let out = db.with_face_data(id, |data, index| {
            let face = ttf_parser::Face::parse(data, index).ok()?;
            let glyph = face.glyph_index(c)?;
            let mut b = B {
                bez: kurbo::BezPath::new(),
            };
            face.outline_glyph(glyph, &mut b)?;
            let contours = contours_from_bezpath(&b.bez);
            if contours.is_empty() {
                return None;
            }
            Some(clean_contours(&contours, false))
        });
        if let Some(Some(contours)) = out {
            if !contours.is_empty() {
                return Ok(centre_contours(contours));
            }
        }
    }
    Err(format!("no installed font has an outline for '{c}'"))
}

/// The kit's SVG import: one layer per colour, each a part.
#[tauri::command]
pub async fn kit_import_svg(path: String) -> Result<Vec<SvgLayer>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let data = std::fs::read(&path).map_err(err_str)?;
        svg_layers(&data)
    })
    .await
    .map_err(err_str)?
}

/// The kit's Letter import: the glyph's outline, counters as holes.
#[tauri::command]
pub async fn kit_import_glyph(ch: String) -> Result<Vec<Vec<[f32; 2]>>, String> {
    tauri::async_runtime::spawn_blocking(move || glyph_contours(&ch))
        .await
        .map_err(err_str)?
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A kit design with the imported contours as its one part, the way
    /// the window adds an SVG or a letter.
    fn contours_design(contours: Vec<Vec<[f32; 2]>>) -> rhythia_render::kit::doc::Design {
        use rhythia_render::kit::doc::{Design, Shape};
        let mut d = Design::default_note();
        d.parts[0].shape = Shape::Contours {
            contours,
            label: String::new(),
        };
        d
    }

    fn area(c: &[[f32; 2]]) -> f32 {
        let mut a = 0.0;
        for i in 0..c.len() {
            let p = c[i];
            let q = c[(i + 1) % c.len()];
            a += p[0] * q[1] - q[0] * p[1];
        }
        a * 0.5
    }

    fn extent(contours: &[Vec<[f32; 2]>]) -> f32 {
        contours
            .iter()
            .flatten()
            .map(|p| p[0].abs().max(p[1].abs()))
            .fold(0.0f32, f32::max)
    }

    fn max_abs(contours: &[Vec<[f32; 2]>], axis: usize) -> f32 {
        contours
            .iter()
            .flatten()
            .map(|p| p[axis].abs())
            .fold(0.0f32, f32::max)
    }

    #[test]
    fn svg_import_keeps_donut_holes_and_merges_shapes() {
        // A donut drawn with even-odd (both rings wound the same way,
        // only the rule makes the hole) plus an overlapping square.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <path fill="red" fill-rule="evenodd"
            d="M50 10 A40 40 0 1 0 50 90 A40 40 0 1 0 50 10 Z
               M50 30 A20 20 0 1 0 50 70 A20 20 0 1 0 50 30 Z"/>
          <rect x="70" y="40" width="25" height="20" fill="blue"/>
        </svg>"##;
        let contours = svg_to_contours(svg.as_bytes()).unwrap();
        // Donut outer + donut hole + rect: three contours, one clockwise.
        assert!(contours.len() >= 3, "got {} contours", contours.len());
        assert!(
            contours.iter().any(|c| area(c) < 0.0),
            "no clockwise hole survived the even-odd resolve"
        );
        // Centred and unit-scaled: the extent must touch 1.
        let m = extent(&contours);
        assert!((0.99..=1.01).contains(&m), "extent {m}");
        // And the kit accepts it as a part with the hole intact: the
        // evaluated outline keeps more than one contour.
        let ev = rhythia_render::kit::levels::evaluate(&contours_design(contours)).unwrap();
        assert!(
            ev.parts[0].shapes.iter().any(|s| s.len() > 1),
            "the donut hole vanished in the evaluation"
        );
    }

    #[test]
    fn a_stroke_icon_imports_as_the_bands_it_draws() {
        // A horizontal line, 20 long, 4 wide with round caps: the band is
        // 24 by 4, so after centring it spans 1 along x and a sixth along y.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" fill="none">
          <path d="M2 12 L22 12" stroke="#000000" stroke-width="4" stroke-linecap="round"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].mode, Mode::Add);
        assert_eq!(layers[0].paint, "#000000");
        let c = &layers[0].contours;
        assert_eq!(c.len(), 1, "one band");
        assert!(
            c[0].len() >= 12,
            "round caps need points, got {}",
            c[0].len()
        );
        assert!((max_abs(c, 0) - 1.0).abs() < 0.01);
        assert!(
            (max_abs(c, 1) - 4.0 / 24.0).abs() < 0.01,
            "band half-height {}",
            max_abs(c, 1)
        );
        assert!(area(&c[0]) > 0.0, "the band is a filled outer contour");
    }

    #[test]
    fn line_elements_styled_by_a_stylesheet_import_at_the_default_width() {
        // What icon sites emit: lines with a class, the stroke in a
        // <style> block, no explicit width (so 1 user unit).
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 48 48" fill="#000000">
          <defs><style>.o{fill:none;stroke:#000000;stroke-linecap:round;}</style></defs>
          <g><line class="o" x1="13" y1="12" x2="13" y2="36"/>
             <line class="o" x1="24" y1="18" x2="24" y2="30"/></g>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 1, "one colour");
        assert_eq!(layers[0].contours.len(), 2, "two bars");
        // Bars 24 tall plus caps span y; the longer bar sets the frame, so a
        // bar's half-width is half a unit over the frame's 12.5 half-extent.
        let widths: Vec<f32> = layers[0]
            .contours
            .iter()
            .map(|c| {
                let xs: Vec<f32> = c.iter().map(|p| p[0]).collect();
                xs.iter().cloned().fold(f32::MIN, f32::max)
                    - xs.iter().cloned().fold(f32::MAX, f32::min)
            })
            .collect();
        for w in widths {
            assert!((w - 1.0 / 12.5).abs() < 0.01, "bar width {w}");
        }
    }

    #[test]
    fn a_dashed_stroke_imports_as_its_dashes() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24">
          <path d="M0 12 L24 12" stroke="#123456" stroke-width="2" stroke-dasharray="4 4" fill="none"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 1);
        assert_eq!(layers[0].contours.len(), 3, "three dashes of four over 24");
        assert_eq!(layers[0].paint, "#123456");
    }

    #[test]
    fn colours_become_layers_in_paint_order_and_a_figure_on_a_badge_is_cut() {
        // The badge shape: a disc with a figure of another colour on it.
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
          <circle fill="#FF4500" cx="128" cy="128" r="128"/>
          <rect fill="#FFFFFF" x="96" y="96" width="64" height="64"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].paint, "#ff4500");
        assert_eq!(layers[0].mode, Mode::Add);
        assert_eq!(layers[1].paint, "#ffffff");
        assert_eq!(layers[1].mode, Mode::Cut);
        // One frame for both: the disc reaches 1, the square a quarter.
        assert!((extent(&layers[0].contours) - 1.0).abs() < 0.01);
        assert!((extent(&layers[1].contours) - 0.25).abs() < 0.01);
        // The old designer still gets everything as one outline.
        let merged = svg_to_contours(svg.as_bytes()).unwrap();
        assert_eq!(merged.len(), 2);
    }

    #[test]
    fn colours_side_by_side_all_add_and_same_colours_merge() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 90 30">
          <rect fill="#ff0000" x="0" y="0" width="30" height="30"/>
          <rect fill="#00ff00" x="30" y="0" width="30" height="30"/>
          <rect fill="#ff0000" x="60" y="0" width="30" height="30"/>
          <rect fill="#ff0000" x="55" y="0" width="10" height="30"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 2, "two colours");
        assert!(layers.iter().all(|l| l.mode == Mode::Add));
        // Red: the left square and the merged right pair, two regions.
        assert_eq!(layers[0].contours.len(), 2);
    }

    #[test]
    fn a_path_with_fill_and_stroke_of_two_colours_feeds_both_layers() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100">
          <rect fill="#0000ff" stroke="#ff0000" stroke-width="10" x="20" y="20" width="60" height="60"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[0].paint, "#0000ff");
        assert_eq!(layers[1].paint, "#ff0000");
        // The stroke band lies half on the fill: half its area is
        // covered, which is not more than the cut share, so it adds.
        assert_eq!(layers[1].mode, Mode::Add);
        assert_eq!(layers[1].contours.len(), 2, "a band is outer plus hole");
    }

    #[test]
    fn a_large_disc_imports_with_a_smooth_outline() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
          <circle fill="#ff4500" cx="128" cy="128" r="128"/>
        </svg>"##;
        let layers = svg_layers(svg.as_bytes()).unwrap();
        let n = layers[0].contours[0].len();
        assert!(n >= 64, "a disc with {n} points shows its facets");
        assert!(n <= 400, "{n} points is more than the kit needs");
    }

    #[test]
    fn svg_without_paths_reports_cleanly() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
          <text x="1" y="8">hi</text>
        </svg>"##;
        assert_eq!(svg_layers(svg.as_bytes()).unwrap_err(), NO_PATHS);
        let invisible = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 10 10">
          <path d="M0 0 L10 10" stroke="red" stroke-opacity="0" fill="none"/>
        </svg>"##;
        assert_eq!(svg_layers(invisible.as_bytes()).unwrap_err(), NO_PATHS);
        assert!(svg_layers(b"not svg")
            .unwrap_err()
            .starts_with("not a readable SVG"));
    }

    #[test]
    fn glyph_import_outlines_a_letter_with_its_counter() {
        // Depends on system fonts; any sane install has a face with 'A'.
        let Ok(contours) = glyph_contours("A") else {
            // A fontless CI box is not a designer bug.
            return;
        };
        assert!(!contours.is_empty());
        let ev = rhythia_render::kit::levels::evaluate(&contours_design(contours)).unwrap();
        // The letter A encloses a triangular counter: it must be a hole.
        assert!(ev.parts[0].shapes.iter().any(|s| s.len() > 1));
    }

    /// Probe for real files: `RHYTHR_SVG_PROBE=/some/dir cargo test -p
    /// rhythia-gui imports::tests::probe -- --nocapture` prints one line per
    /// .svg in that directory. Without the variable there is nothing to do.
    #[test]
    fn probe_svg_files_from_the_environment() {
        let Ok(dir) = std::env::var("RHYTHR_SVG_PROBE") else {
            return;
        };
        let mut names: Vec<_> = std::fs::read_dir(&dir)
            .expect("probe directory")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("svg")))
            .collect();
        names.sort();
        for path in names {
            let data = std::fs::read(&path).unwrap();
            match svg_layers(&data) {
                Ok(layers) => {
                    let summary: Vec<String> = layers
                        .iter()
                        .map(|l| {
                            let points: usize = l.contours.iter().map(|c| c.len()).sum();
                            format!(
                                "{} {:?} {} contours {} points extent {:.2}",
                                l.paint,
                                l.mode,
                                l.contours.len(),
                                points,
                                extent(&l.contours)
                            )
                        })
                        .collect();
                    println!(
                        "{}: {} layer(s): {}",
                        path.display(),
                        layers.len(),
                        summary.join(" | ")
                    );
                }
                Err(e) => println!("{}: ERROR {e}", path.display()),
            }
        }
    }
}
