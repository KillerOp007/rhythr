//! Debug tool: build a designer note (wavy hollow ring) and write the
//! .rhs pack, exercising the exact path the designer window's save takes.
//! Usage: designer_pack <out.rhs> [waves] [amp]
fn main() {
    let mut args = std::env::args().skip(1);
    let out = args.next().expect("out path");
    let waves: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(8.0);
    let amp: f32 = args.next().and_then(|s| s.parse().ok()).unwrap_or(0.22);
    let outline: Vec<[f32; 2]> = (0..160)
        .map(|i| {
            let th = i as f32 / 160.0 * std::f32::consts::TAU;
            let r = 1.0 + amp * (waves * th).sin();
            [r * th.cos(), r * th.sin()]
        })
        .collect();
    let mesh = rhythia_render::designer::build_note_mesh(&outline, Some(0.22), 0.05).unwrap();
    let pack = rhythia_render::designer::export_rhs(
        &mesh,
        "wellenring",
        &["#ff5d6c".into(), "#2fd6d0".into(), "#f2c14e".into()],
    )
    .unwrap();
    std::fs::write(&out, pack).unwrap();
    println!("wrote {out}");
}
