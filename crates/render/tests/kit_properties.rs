//! Random designs against the validator. Any failure here is a kernel
//! bug by definition: the kit promises watertight meshes for every
//! input the editor can produce.
use proptest::prelude::*;
use proptest::test_runner::TestCaseError;
use rhythia_render::kit::doc::*;
use rhythia_render::kit::export::{export_rhs, to_obj};
use rhythia_render::kit::levels::evaluate;
use rhythia_render::kit::mesh::build_mesh;
use rhythia_render::kit::validate::{validate, KitMesh};
use rhythia_render::kit::{KitError, KIT_SCALE};
use std::collections::HashMap;

/// Within one material, no vertex may sit strictly inside another
/// triangle's edge at the same z: a seam between two faces has to be split
/// identically on both sides, or the caps and walls meet in T-junctions.
/// Exact on the lattice; vertices are bucketed by z and sorted by x so an
/// edge scans only the vertices inside its x span.
fn no_t_junctions(m: &KitMesh) -> bool {
    let li = |v: f32| (v * KIT_SCALE).round() as i64;
    let pts: Vec<[i64; 2]> = m.positions.iter().map(|p| [li(p[0]), li(p[1])]).collect();
    for range in [m.solid.clone(), m.glass.clone()] {
        let idx = &m.indices[range];
        let mut by_z: HashMap<u32, Vec<u32>> = HashMap::new();
        for &i in idx {
            by_z.entry(m.positions[i as usize][2].to_bits())
                .or_default()
                .push(i);
        }
        for v in by_z.values_mut() {
            v.sort_unstable();
            v.dedup();
            v.sort_by_key(|&i| pts[i as usize][0]);
        }
        for t in idx.chunks_exact(3) {
            for (a, b) in [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])] {
                let za = m.positions[a as usize][2].to_bits();
                if za != m.positions[b as usize][2].to_bits() {
                    continue;
                }
                let (pa, pb) = (pts[a as usize], pts[b as usize]);
                let (x0, x1) = (pa[0].min(pb[0]), pa[0].max(pb[0]));
                let verts = &by_z[&za];
                let start = verts.partition_point(|&i| pts[i as usize][0] < x0);
                for &v in &verts[start..] {
                    let pv = pts[v as usize];
                    if pv[0] > x1 {
                        break;
                    }
                    if v == a || v == b {
                        continue;
                    }
                    let d = [pb[0] - pa[0], pb[1] - pa[1]];
                    let e = [pv[0] - pa[0], pv[1] - pa[1]];
                    let inside_x = pv[0] > x0 && pv[0] < x1;
                    let inside_y = pv[1] > pa[1].min(pb[1]) && pv[1] < pa[1].max(pb[1]);
                    if d[0] * e[1] - d[1] * e[0] == 0 && (inside_x || inside_y) {
                        return false;
                    }
                }
            }
        }
    }
    true
}

/// Half the time on a 0.1 grid, so parts sit flush and seams run exactly
/// collinear, which is where face sharing is hardest.
fn coord() -> impl Strategy<Value = f32> {
    prop_oneof![(-0.6f32..0.6), (-6i32..=6).prop_map(|k| k as f32 * 0.1)]
}

fn size() -> impl Strategy<Value = f32> {
    prop_oneof![(0.2f32..1.0), (2i32..=10).prop_map(|k| k as f32 * 0.1)]
}

fn shape() -> impl Strategy<Value = Shape> {
    prop_oneof![
        Just(Shape::Circle),
        Just(Shape::Square { rounding: 0.0 }),
        (0.0f32..1.0).prop_map(|rounding| Shape::Square { rounding }),
        (3u32..10, 0.0f32..1.0).prop_map(|(sides, rounding)| Shape::Polygon { sides, rounding }),
        (3u32..9, 0.2f32..0.9).prop_map(|(points, inner)| Shape::Star { points, inner }),
        (0.1f32..0.8).prop_map(|thickness| Shape::Ring { thickness }),
        (0.2f32..1.0).prop_map(|aspect| Shape::Pill { aspect }),
    ]
}

fn effects() -> impl Strategy<Value = Effects> {
    (
        proptest::option::of((1u32..=8, 0.05f32..0.4).prop_map(|(count, depth)| Waves {
            count,
            depth,
            style: WaveStyle::Sine,
        })),
        proptest::option::of(0.02f32..0.3),
        proptest::option::of(0.05f32..0.4),
        prop_oneof![
            Just(Mirror::None),
            Just(Mirror::X),
            Just(Mirror::Y),
            Just(Mirror::Both)
        ],
        proptest::option::of(2u32..=4),
    )
        .prop_map(|(waves, round, rim, mirror, repeat)| Effects {
            waves,
            round,
            rim,
            mirror,
            repeat,
        })
}

fn part(i: usize) -> impl Strategy<Value = Part> {
    (
        shape(),
        coord(),
        coord(),
        size(),
        size(),
        prop_oneof![2 => 0.0f32..360.0, 1 => Just(0.0f32), 1 => Just(90.0f32)],
        prop_oneof![3 => Just(Mode::Add), 1 => Just(Mode::Cut)],
        -1.9f32..0.4,
        0.05f32..0.5,
        prop_oneof![4 => Just(Material::Solid), 1 => Just(Material::Glass)],
        effects(),
    )
        .prop_map(
            move |(shape, x, y, w, h, rotation, mode, back, span, material, effects)| Part {
                id: format!("p{i}"),
                name: format!("p{i}"),
                visible: true,
                locked: false,
                shape,
                transform: Transform {
                    x,
                    y,
                    w,
                    h,
                    rotation,
                },
                mode,
                depth: Depth {
                    front: (back + span).min(0.5),
                    back,
                },
                material,
                effects,
            },
        )
}

fn design() -> impl Strategy<Value = Design> {
    (1usize..=4).prop_flat_map(|n| {
        let parts: Vec<_> = (0..n).map(part).collect();
        parts.prop_map(|parts| Design {
            version: 2,
            name: "p".into(),
            colors: vec!["#ffffff".into()],
            parts,
            groups: vec![],
        })
    })
}

proptest! {
    // A case costs about a second in release and ten times that in debug,
    // so the plain `cargo test` run samples 10 designs and the release run
    // 200; shrinking is bounded in steps and in time.
    #![proptest_config(ProptestConfig {
        cases: if cfg!(debug_assertions) { 10 } else { 200 },
        max_shrink_iters: 300,
        max_shrink_time: 120_000,
        ..ProptestConfig::default()
    })]

    #[test]
    fn every_design_is_watertight_in_the_box_or_cleanly_empty(d in design()) {
        match evaluate(&d) {
            Err(KitError::Empty) => {}
            Err(e) => prop_assert!(false, "unexpected error {e}"),
            Ok(ev) => {
                // The triangle budget is a clean refusal, not a defect: dense
                // in-range designs (waves times repeat times mirror on four
                // parts) legitimately pass 40k triangles.
                let m = match build_mesh(&ev) {
                    Ok(m) => m,
                    Err(KitError::Budget(_)) => return Ok(()),
                    Err(e) => return Err(TestCaseError::fail(format!("build_mesh failed: {e} on {d:?}"))),
                };
                let r = validate(&m).unwrap();
                prop_assert!(no_t_junctions(&m), "a vertex sits inside another triangle's edge: {:?}", d);
                prop_assert!(r.triangles <= 40_000);
                prop_assert!(m.positions.iter().all(|p| p[0].abs() <= 1.0001 && p[1].abs() <= 1.0001));
                let bytes = export_rhs(&d, "p").unwrap();
                let cfg = rhythia_render::config::SkinConfig::from_rhs(&bytes).unwrap();
                // export_rhs evaluates and builds on its own, so the archive's
                // obj equals ours only if the kernel is deterministic.
                let obj = to_obj(&m, "p");
                prop_assert_eq!(cfg.note_mesh.as_deref(), Some(obj.as_bytes()));
            }
        }
    }
}
