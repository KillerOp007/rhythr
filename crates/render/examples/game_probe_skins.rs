//! Debug tool: builds the in-game probe skins that answer the open
//! questions about the Steam client's mesh loader before the 2.5D note
//! kit commits to them: back-face culling, stacked depth, multiple obj
//! objects, triangle budget, and per-material alpha (by material name as
//! the built-in cube does it, and by the .mtl `d` value).
//!
//! Each probe is a complete .rhs (config + obj + mtl + colorset) plus a
//! PNG rendered through rhythr's own pipeline as a sanity picture.
//! Usage: game_probe_skins <out_dir>
use rhythia_render::designer::{mesh_from_plan, plan_note, NotePlan};
use rhythia_render::mesh::Mesh;
use std::fmt::Write as _;
use std::io::Write as _;
use std::path::Path;

/// One obj object with its own material: vertices plus triangles.
struct Part {
    object: String,
    material: String,
    verts: Vec<[f32; 3]>,
    tris: Vec<[u32; 3]>,
}

fn circle(r: f32, n: usize) -> Vec<[f32; 2]> {
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            [r * t.cos(), r * t.sin()]
        })
        .collect()
}

fn square(h: f32) -> Vec<[f32; 2]> {
    vec![[-h, -h], [h, -h], [h, h], [-h, h]]
}

/// Extrudes a plan into z [z_back, z_front] (front = +z, toward the
/// player, the convention of every built-in mesh). The kernel extrudes
/// symmetrically and renormalises anything smaller than the unit box
/// back up to it, so a part is built at unit size and scaled in x/y
/// AFTER extrusion (`xy_scale`). `flip_front` reverses the front cap's
/// winding for the culling probe.
fn extrude(
    plan: &NotePlan,
    z_back: f32,
    z_front: f32,
    xy_scale: f32,
    flip_front: bool,
) -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
    let half = (z_front - z_back) * 0.5;
    let centre = (z_front + z_back) * 0.5;
    let mesh: Mesh = mesh_from_plan(plan, half).expect("plan extrudes");
    let verts: Vec<[f32; 3]> = mesh
        .vertices
        .iter()
        .map(|v| [v.pos[0] * xy_scale, v.pos[1] * xy_scale, v.pos[2] + centre])
        .collect();
    let mut tris: Vec<[u32; 3]> = Vec::with_capacity(mesh.indices.len() / 3);
    for t in mesh.indices.chunks_exact(3) {
        let on_front = t
            .iter()
            .all(|&i| (mesh.vertices[i as usize].pos[2] - half).abs() < 1e-5);
        if flip_front && on_front {
            tris.push([t[0], t[2], t[1]]);
        } else {
            tris.push([t[0], t[1], t[2]]);
        }
    }
    (verts, tris)
}

fn part(
    object: &str,
    material: &str,
    plan: &NotePlan,
    z_back: f32,
    z_front: f32,
    xy_scale: f32,
    flip: bool,
) -> Part {
    let (verts, tris) = extrude(plan, z_back, z_front, xy_scale, flip);
    Part {
        object: object.into(),
        material: material.into(),
        verts,
        tris,
    }
}

/// Wavefront text. `merge_objects` writes everything under one `o`, the
/// alternative keeps each part as its own object; indices are global in
/// obj either way.
fn obj_text(parts: &[Part], merge_objects: bool) -> String {
    let mut s = String::new();
    s.push_str("# rhythr game probe\nmtllib note.mtl\n");
    if merge_objects {
        s.push_str("o probe\n");
    }
    let mut base = 0u32;
    // Vertices first (all parts), then faces per part: a layout every
    // obj parser accepts.
    for p in parts {
        for v in &p.verts {
            let _ = writeln!(s, "v {:.6} {:.6} {:.6}", v[0], v[1], v[2]);
        }
    }
    for p in parts {
        if !merge_objects {
            let _ = writeln!(s, "o {}", p.object);
        }
        let _ = writeln!(s, "usemtl {}", p.material);
        for t in &p.tris {
            let _ = writeln!(
                s,
                "f {} {} {}",
                base + t[0] + 1,
                base + t[1] + 1,
                base + t[2] + 1
            );
        }
        base += p.verts.len() as u32;
    }
    s
}

fn mtl_text(materials: &[(&str, f32)]) -> String {
    let mut s = String::from("# rhythr game probe\n");
    for (name, d) in materials {
        let _ = write!(
            s,
            "newmtl {name}\nNs 250.000000\nKa 1.000000 1.000000 1.000000\nKd 1.000000 1.000000 1.000000\nKs 0.000000 0.000000 0.000000\nKe 0.000000 0.000000 0.000000\nNi 1.450000\nd {d:.6}\nillum 1\n\n"
        );
    }
    s
}

fn write_rhs(dir: &Path, name: &str, obj: &str, mtl: &str) {
    let config =
        format!(r#"{{"NoteSkin":{{"Value":"{name}.obj"}},"ColorSet":{{"Value":"{name}"}}}}"#);
    let mut zw = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    let mut put = |path: &str, bytes: &[u8]| {
        zw.start_file(path, opts).unwrap();
        zw.write_all(bytes).unwrap();
    };
    put("config", config.as_bytes());
    put(&format!("noteSkin/{name}.obj"), obj.as_bytes());
    put("noteSkin/note.mtl", mtl.as_bytes());
    put(
        &format!("colorSet/{name}.txt"),
        b"#ff5d6c\r\n#2fd6d0\r\n#f2c14e\r\n",
    );
    let bytes = zw.finish().unwrap().into_inner();
    std::fs::write(dir.join(format!("{name}.rhs")), &bytes).unwrap();
    // Round trip through rhythr's own loader: a probe that rhythr cannot
    // read is not worth sending anywhere.
    let cfg = rhythia_render::config::SkinConfig::from_rhs(&bytes).expect("rhs loads back");
    assert!(cfg.note_mesh.is_some(), "{name}: no mesh in pack");
}

/// Renders the probe through rhythr's pipeline: three notes side by side
/// at staggered depths, the designer's still scene, so stacking and
/// perspective are visible in the picture.
fn render_png(dir: &Path, name: &str, obj: &str) {
    let mut cfg = rhythia_render::config::SkinConfig::default();
    cfg.note_mesh = Some(obj.as_bytes().to_vec());
    cfg.colorset = rhythia_render::config::parse_colorset("#ff5d6c\n#2fd6d0\n#f2c14e");
    cfg.camera_fov = 34.0;
    cfg.cursor_opacity = 0.0;
    cfg.cursor_trail_enabled = false;
    let mut map = rhythia_formats::map::Map::default();
    for (i, t) in [360i64, 480, 620].into_iter().enumerate() {
        map.notes.push(rhythia_formats::map::Note {
            time_ms: t,
            x: i as f32,
            y: 1.0,
        });
    }
    let data = {
        let mut b: Vec<u8> = Vec::new();
        b.extend((20260803i32).to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.push(0);
        b.push(0);
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.push(0);
        b.push(1);
        b.push(2);
        b.extend(b"[]");
        b.push(0);
        b.extend(1.0f32.to_le_bytes());
        b.extend(0i64.to_le_bytes());
        b.extend(100.0f32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0i32.to_le_bytes());
        b.extend(0.0f32.to_le_bytes());
        b.extend((-1i32).to_le_bytes());
        b.push(0);
        b.push(0);
        b.extend(1i32.to_le_bytes());
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
        .render_still(&params, &cfg, &skin, &replay, &map, 300.0, None)
        .unwrap();
    rhythia_render::write_png(&dir.join(format!("{name}.png")), &px, 960, 560).unwrap();
}

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).expect("out dir"));
    std::fs::create_dir_all(&dir).unwrap();

    let disc = plan_note(&circle(1.0, 128), None).unwrap();
    let ring = plan_note(&circle(1.0, 128), Some(0.28)).unwrap();
    // Smaller parts are unit plans scaled down at extrusion time.
    let plate = plan_note(&circle(1.0, 96), None).unwrap();
    let inset = plan_note(&square(1.0), None).unwrap();

    let plain = mtl_text(&[("Material.001", 1.0)]);
    let mut summary = String::new();

    // 1. Culling: a disc whose front cap is wound backwards.
    {
        let parts = [part("probe", "Material.001", &disc, -0.1, 0.1, 1.0, true)];
        let obj = obj_text(&parts, true);
        write_rhs(&dir, "probe1_cull", &obj, &plain);
        render_png(&dir, "probe1_cull", &obj);
        let _ = writeln!(summary, "probe1_cull: {} tris", parts[0].tris.len());
    }
    // 2. Stacking: ring at the plane, plate recessed half a unit, ONE object.
    {
        let parts = [
            part("ring", "Material.001", &ring, -0.1, 0.1, 1.0, false),
            part("plate", "Material.001", &plate, -0.6, -0.4, 0.55, false),
        ];
        let obj = obj_text(&parts, true);
        write_rhs(&dir, "probe2_stack", &obj, &plain);
        render_png(&dir, "probe2_stack", &obj);
        let _ = writeln!(
            summary,
            "probe2_stack: {} tris",
            parts.iter().map(|p| p.tris.len()).sum::<usize>()
        );
    }
    // 3. Objects: the same geometry as two `o` objects.
    {
        let parts = [
            part("ring", "Material.001", &ring, -0.1, 0.1, 1.0, false),
            part("plate", "Material.001", &plate, -0.6, -0.4, 0.55, false),
        ];
        let obj = obj_text(&parts, false);
        write_rhs(&dir, "probe3_objects", &obj, &plain);
        render_png(&dir, "probe3_objects", &obj);
        let _ = writeln!(summary, "probe3_objects: 2 objects");
    }
    // 4. Budget: a dense wavy ring, well above anything the kit will emit.
    {
        let wavy: Vec<[f32; 2]> = (0..2400)
            .map(|i| {
                let t = i as f32 / 2400.0 * std::f32::consts::TAU;
                let r = 1.0 + 0.12 * (24.0 * t).sin();
                [r * t.cos(), r * t.sin()]
            })
            .collect();
        let dense = plan_note(&wavy, Some(0.2)).unwrap();
        let parts = [part("probe", "Material.001", &dense, -0.1, 0.1, 1.0, false)];
        let obj = obj_text(&parts, true);
        write_rhs(&dir, "probe4_dense", &obj, &plain);
        render_png(&dir, "probe4_dense", &obj);
        let _ = writeln!(summary, "probe4_dense: {} tris", parts[0].tris.len());
    }
    // 5. Alpha by material NAME: the built-in cube's "Transparent" with d 1.
    {
        let parts = [
            part("disc", "Material.001", &disc, -0.1, 0.1, 1.0, false),
            part("inlay", "Transparent", &inset, 0.1, 0.2, 0.45, false),
        ];
        let obj = obj_text(&parts, true);
        write_rhs(
            &dir,
            "probe5_alpha_name",
            &obj,
            &mtl_text(&[("Material.001", 1.0), ("Transparent", 1.0)]),
        );
        render_png(&dir, "probe5_alpha_name", &obj);
        let _ = writeln!(summary, "probe5_alpha_name: material Transparent, d 1.0");
    }
    // 6. Alpha by .mtl value: a neutral name with d 0.35.
    {
        let parts = [
            part("disc", "Material.001", &disc, -0.1, 0.1, 1.0, false),
            part("inlay", "Glass", &inset, 0.1, 0.2, 0.45, false),
        ];
        let obj = obj_text(&parts, true);
        write_rhs(
            &dir,
            "probe6_alpha_d",
            &obj,
            &mtl_text(&[("Material.001", 1.0), ("Glass", 0.35)]),
        );
        render_png(&dir, "probe6_alpha_d", &obj);
        let _ = writeln!(summary, "probe6_alpha_d: material Glass, d 0.35");
    }
    // 7. Alpha, done right: the translucent part sits in the ring's HOLE,
    // over the black background, where any alpha below one must show as a
    // darker colour. (Probes 5 and 6 put it over an opaque disc of the same
    // colour, which no alpha can ever make visible.)
    for (name, material, d) in [
        ("probe7a_alpha_hole_d", "Glass", 0.35f32),
        ("probe7b_alpha_hole_name", "Transparent", 1.0f32),
    ] {
        let parts = [
            part("ring", "Material.001", &ring, -0.1, 0.1, 1.0, false),
            part("inlay", material, &plate, -0.1, 0.1, 0.5, false),
        ];
        let obj = obj_text(&parts, true);
        write_rhs(&dir, name, &obj, &mtl_text(&[("Material.001", 1.0), (material, d)]));
        render_png(&dir, name, &obj);
        let _ = writeln!(summary, "{name}: inlay in hole, material {material}, d {d}");
    }
    print!("{summary}");
    println!("wrote probes to {}", dir.display());
}
