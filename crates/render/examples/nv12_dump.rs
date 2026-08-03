//! Converts a raw RGBA file to NV12 with the renderer's own code, so the
//! result can be diffed against what ffmpeg produces from the same input.
//! Usage: nv12_dump <in.rgba> <width> <height> <out.nv12>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let src = std::fs::read(&a[0]).unwrap();
    let w: usize = a[1].parse().unwrap();
    let h: usize = a[2].parse().unwrap();
    let mut dst = vec![0u8; rhythia_render::nv12::nv12_len(w, h)];
    assert!(rhythia_render::nv12::rgba_to_nv12(&src, w, h, &mut dst));
    std::fs::write(&a[3], &dst).unwrap();
    println!("{} bytes", dst.len());
}
