//! Debug tool: renders the designer window's preview scene to a PNG,
//! the same three-note synthetic setup the live preview command uses.
//! Usage: designer_preview_png <out.png> [waves] [amp] [rim]
#[allow(clippy::field_reassign_with_default)]
fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("out path");
    let waves: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let amp: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.25);
    let rim: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.22);
    // RHYTHR_SHAPE_JSON: a file with {"import":[[[x,y]..]..],"hollow":t|null}
    // overrides the wavy default, so any designed shape can be rendered.
    let mesh = if let Ok(json_path) = std::env::var("RHYTHR_SHAPE_JSON") {
        let v: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(json_path).unwrap()).unwrap();
        let contours: Vec<Vec<[f32; 2]>> = serde_json::from_value(v["import"].clone()).unwrap();
        let hollow = v["hollow"].as_f64().map(|t| t as f32);
        let depth = v["depth"].as_f64().unwrap_or(0.05) as f32;
        rhythia_render::designer::build_note_mesh_multi(&contours, hollow, depth).unwrap()
    } else {
        let outline: Vec<[f32; 2]> = (0..160)
            .map(|i| {
                let th = i as f32 / 160.0 * std::f32::consts::TAU;
                let r = 1.0 + amp * (waves * th).sin();
                [r * th.cos(), r * th.sin()]
            })
            .collect();
        rhythia_render::designer::build_note_mesh(&outline, Some(rim), 0.05).unwrap()
    };
    let obj = rhythia_render::designer::to_obj(&mesh, "preview");
    let mut cfg = rhythia_render::config::SkinConfig::default();
    cfg.note_mesh = Some(obj.into_bytes());
    cfg.colorset = rhythia_render::config::parse_colorset("#ff5d6c\n#2fd6d0\n#f2c14e");
    cfg.camera_fov = 34.0;
    // The note is the subject; the synthetic cursor only covers it.
    cfg.cursor_opacity = 0.0;
    cfg.cursor_trail_enabled = false;

    let t_ms: f64 = args.next().and_then(|s| s.parse().ok()).unwrap_or(700.0);
    // "still" as the sixth arg renders the designer's still scene instead:
    // three notes side by side at staggered approach depths.
    let still = args.next().as_deref() == Some("still");
    let mut map = rhythia_formats::map::Map::default();
    const SNAKE: [(f32, f32); 9] = [
        (0.0, 0.0),
        (1.0, 0.0),
        (2.0, 0.0),
        (2.0, 1.0),
        (1.0, 1.0),
        (0.0, 1.0),
        (0.0, 2.0),
        (1.0, 2.0),
        (2.0, 2.0),
    ];
    if still {
        for (i, t) in [360i64, 480, 620].into_iter().enumerate() {
            map.notes.push(rhythia_formats::map::Note {
                time_ms: t,
                x: i as f32,
                y: 1.0,
            });
        }
    } else {
        for (i, (x, y)) in SNAKE.into_iter().enumerate() {
            for lap in 0..2i64 {
                map.notes.push(rhythia_formats::map::Note {
                    time_ms: 500 + i as i64 * 300 + lap * 2700,
                    x,
                    y,
                });
            }
        }
    }
    map.notes.sort_by_key(|n| n.time_ms);
    let data = {
        // Minimal synthetic replay via the parser (single idle frame).
        let mut b: Vec<u8> = Vec::new();
        b.extend((20260803i32).to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.push(0); // player ""
        b.push(0); // legacy ""
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.push(0); // mode ""
        b.push(1); // passed
        b.push(2);
        b.extend(b"[]");
        b.push(0); // spin
        b.extend(1.0f32.to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        b.push(0); // hash ""
        b.push(0); // pitch lock
        b.extend(1i32.to_le_bytes()); // frames
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend(1.0f32.to_le_bytes());
        b.push(0);
        b
    };
    let replay = rhythia_formats::rhr::Replay::parse(&data).unwrap();

    let renderer = rhythia_render::Renderer::new(960, 560, cfg.hud_fonts().as_slice()).unwrap();
    let skin = renderer.prepare_skin(&cfg);
    let params = rhythia_render::scene::SceneParams::from(&cfg);
    let px = renderer
        .render_still(
            &params,
            &cfg,
            &skin,
            &replay,
            &map,
            if still {
                300.0
            } else {
                500.0 + t_ms.rem_euclid(2700.0)
            },
            None,
        )
        .unwrap();
    rhythia_render::write_png(std::path::Path::new(&out), &px, 960, 560).unwrap();
    println!("wrote {out}");
}
