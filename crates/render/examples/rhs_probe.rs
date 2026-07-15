//! One-off: inspect what SkinConfig extracts from real .rhs packs.
fn main() {
    for path in std::env::args().skip(1) {
        match rhythia_render::SkinConfig::from_path(&path) {
            Ok(c) => {
                let base = std::path::Path::new(&path)
                    .file_name()
                    .unwrap()
                    .to_string_lossy();
                println!(
                    "{base}: fov={} AR={} note_shape={:?} trail={} | note_tex={} border_tex={} cursor_tex={} colorset={:?}",
                    c.camera_fov, c.approach_rate, c.note_shape, c.cursor_trail_enabled,
                    c.note_texture.as_ref().map_or("-".into(), |b| format!("{}B", b.len())),
                    c.border_texture.as_ref().map_or("-".into(), |b| format!("{}B", b.len())),
                    c.cursor_texture.as_ref().map_or("-".into(), |b| format!("{}B", b.len())),
                    c.colorset,
                );
            }
            Err(e) => println!("{path}: ERROR {e}"),
        }
    }
}
