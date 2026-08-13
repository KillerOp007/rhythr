//! Debug tool: list every resource entry the exe's index yields, one per
//! line as `name<TAB>size`. For diffing two game versions against each
//! other, which is how a new release announces what it added or renamed
//! before any of it is wired up.
//! Usage: dump_exe_index <rhythia.exe>
fn main() {
    let exe = std::env::args().nth(1).expect("exe path");
    let data = std::fs::read(&exe).expect("reading exe");
    let entries = rhythia_render::exe_assets::scan_entries(&data);
    match rhythia_render::exe_assets::locate_blob_base(&data, &entries) {
        Some(base) => eprintln!("{} entries, blob base {base:#x}", entries.len()),
        None => eprintln!("{} entries, blob base NOT FOUND", entries.len()),
    }
    for e in &entries {
        println!("{}\t{}\t{}", e.name, e.size, e.offset);
    }
    // A second argument extracts one entry by exact name to stdout-adjacent
    // file `dumped.bin`, for reading embedded text resources (the schema
    // migrations say what a release changed before any UI does).
    if let Some(want) = std::env::args().nth(2) {
        let base = rhythia_render::exe_assets::locate_blob_base(&data, &entries)
            .expect("blob base needed for extraction");
        let e = entries
            .iter()
            .find(|e| e.name == want)
            .expect("no entry by that name");
        let s = (base + e.offset) as usize;
        std::fs::write("dumped.bin", &data[s..s + e.size as usize]).expect("write dumped.bin");
        eprintln!("wrote {} bytes -> dumped.bin", e.size);
    }
}
