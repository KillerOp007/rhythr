//! Debug tool: extract the game's built-in skin assets from rhythia.exe.
//! Usage: extract_exe <rhythia.exe> <out-dir>
fn main() {
    let mut args = std::env::args().skip(1);
    let (exe, out) = (args.next().expect("exe path"), args.next().expect("out dir"));
    match rhythia_render::exe_assets::extract_to_dir(
        std::path::Path::new(&exe),
        std::path::Path::new(&out),
    ) {
        Ok(n) => println!("extracted {n} assets -> {out}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
