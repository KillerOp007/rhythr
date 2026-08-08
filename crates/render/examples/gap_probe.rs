//! Measures recording gaps in a replay: stretches with no frames.
//! Usage: gap_probe <replay.rhr>
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let replay = rhythia_formats::rhr::Replay::from_path(&a[0]).unwrap();
    let f = &replay.frames;
    let mut gaps = Vec::new();
    for w in f.windows(2) {
        let dt = w[1].ms - w[0].ms;
        if dt > 300.0 {
            gaps.push((w[0].ms, dt, (w[0].x, w[0].y), (w[1].x, w[1].y)));
        }
    }
    println!(
        "frames={} span={:.0}ms gaps>300ms={} (>500ms: {})",
        f.len(),
        f.last().map(|x| x.ms).unwrap_or(0.0),
        gaps.len(),
        gaps.iter().filter(|g| g.1 > 500.0).count()
    );
    for (ms, dt, a, b) in gaps.iter().take(12) {
        println!(
            "  gap {:.0}ms at {:.0}ms: ({:.2},{:.2}) -> ({:.2},{:.2})",
            dt, ms, a.0, a.1, b.0, b.1
        );
    }
}
