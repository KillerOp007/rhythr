//! The game's own mod icons, packed into one texture the HUD draws from.
//!
//! The icons are the ones the player's game installation carries (see
//! [`crate::builtin`]); nothing is bundled with rhythr. Without a connected
//! installation the atlas stays empty and the HUD falls back to its drawn
//! speed notation and to mod names as text.
//!
//! A run's speed is not in its mod list: the replay stores it as a number,
//! and the game has one icon per ranked speed, named for the step
//! ([`speed_icon_name`]).

use std::collections::HashMap;

/// Edge of one atlas cell. The game's icons are 135 px squares; 144 leaves
/// a transparent margin around each and is a multiple of 8, so every
/// smaller mip level averages whole cells and no icon bleeds into its
/// neighbour.
pub const CELL: u32 = 144;
const COLS: u32 = 8;
const ROWS: u32 = 4;
/// Atlas size in pixels: room for 32 icons (the game has 21).
pub const ATLAS_W: u32 = CELL * COLS;
pub const ATLAS_H: u32 = CELL * ROWS;
/// Mip levels the atlas carries. An icon is drawn at a fifth of its size on
/// a 720p HUD and near full size on a 4K results screen, and one level
/// sampled that far down shimmers.
pub const MIP_LEVELS: u32 = 4;

/// Where one icon sits in the atlas.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Icon {
    /// `[u0, v0, u1, v1]`: the icon's full height, and of its width only
    /// the part that is drawn. The speed icons differ a lot there (a lone
    /// `S` next to `S++++`), and a row of full squares would space them
    /// unevenly.
    pub uv: [f32; 4],
    /// Width of that rectangle over its height.
    pub aspect: f32,
}

/// The icons of one game installation, by name (`mod_hardrock`, `mod_45`).
#[derive(Default)]
pub struct ModIcons {
    icons: HashMap<String, Icon>,
    key: u64,
}

impl ModIcons {
    pub fn get(&self, name: &str) -> Option<Icon> {
        self.icons.get(name).copied()
    }

    pub fn is_empty(&self) -> bool {
        self.icons.is_empty()
    }

    /// What [`source_key`] gave for the list this atlas was built from.
    pub fn key(&self) -> u64 {
        self.key
    }

    /// Packs `sources` (name and PNG bytes) into an atlas. Returns the
    /// lookup and the RGBA pixels of each mip level, largest first. A file
    /// that does not decode is left out, and so is everything past the
    /// atlas's 32 cells.
    pub fn build(sources: &[(String, Vec<u8>)]) -> (ModIcons, Vec<Vec<u8>>) {
        let mut base = vec![0u8; (ATLAS_W * ATLAS_H * 4) as usize];
        let mut icons = HashMap::new();
        let mut slot = 0u32;
        for (name, png) in sources {
            if slot >= COLS * ROWS {
                break;
            }
            let Some(img) = image::load_from_memory(png).ok().map(|i| i.to_rgba8()) else {
                continue;
            };
            // Anything larger than a cell's inside is scaled down to it.
            let inside = CELL - 8;
            let img = if img.width().max(img.height()) > inside {
                let s = inside as f32 / img.width().max(img.height()) as f32;
                image::imageops::resize(
                    &img,
                    ((img.width() as f32 * s).round() as u32).max(1),
                    ((img.height() as f32 * s).round() as u32).max(1),
                    image::imageops::FilterType::Triangle,
                )
            } else {
                img
            };
            let (iw, ih) = img.dimensions();
            let Some((x0, x1)) = drawn_columns(&img) else {
                continue;
            };
            let (cx, cy) = ((slot % COLS) * CELL, (slot / COLS) * CELL);
            let (ox, oy) = (cx + (CELL - iw) / 2, cy + (CELL - ih) / 2);
            for y in 0..ih {
                let src = (y * iw * 4) as usize;
                let dst = (((oy + y) * ATLAS_W + ox) * 4) as usize;
                base[dst..dst + (iw * 4) as usize].copy_from_slice(&img.as_raw()[src..src + (iw * 4) as usize]);
            }
            // One texel of air on each side, so the filter has something
            // transparent to fade into.
            let (u0, u1) = (ox + x0.saturating_sub(1), (ox + x1 + 1).min(cx + CELL));
            icons.insert(
                name.clone(),
                Icon {
                    uv: [
                        u0 as f32 / ATLAS_W as f32,
                        oy as f32 / ATLAS_H as f32,
                        u1 as f32 / ATLAS_W as f32,
                        (oy + ih) as f32 / ATLAS_H as f32,
                    ],
                    aspect: (u1 - u0) as f32 / ih as f32,
                },
            );
            slot += 1;
        }

        let mut levels = vec![base];
        let (mut w, mut h) = (ATLAS_W, ATLAS_H);
        for _ in 1..MIP_LEVELS {
            let next = halve(levels.last().expect("a level"), w, h);
            levels.push(next);
            w /= 2;
            h /= 2;
        }
        (ModIcons { icons, key: source_key(sources) }, levels)
    }
}

/// A cheap fingerprint of an icon list (names and sizes), so a renderer can
/// tell whether the atlas it holds was built from this list. Zero for an
/// empty list.
pub fn source_key(sources: &[(String, Vec<u8>)]) -> u64 {
    if sources.is_empty() {
        return 0;
    }
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut eat = |b: u8| {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    };
    for (name, png) in sources {
        name.bytes().for_each(&mut eat);
        (png.len() as u64).to_le_bytes().into_iter().for_each(&mut eat);
    }
    h.max(1)
}

/// The icon of a ranked speed: the game names the faster ones for the step
/// above 100 percent (`mod_45` is 1.45x) and the slower ones for the
/// percentage itself (`mod_87` is 0.87x). `None` for any other speed, which
/// the game has no icon for.
pub fn speed_icon_name(speed: f32) -> Option<&'static str> {
    const RANKED: [(f32, &str); 8] = [
        (1.0, "mod_100"),
        (1.15, "mod_15"),
        (1.25, "mod_25"),
        (1.35, "mod_35"),
        (1.45, "mod_45"),
        (0.87, "mod_87"),
        (0.8, "mod_80"),
        (0.75, "mod_75"),
    ];
    RANKED.iter().find(|(s, _)| (speed - s).abs() < 0.005).map(|(_, n)| *n)
}

/// First and one-past-last column that has anything in it, or `None` for a
/// fully transparent image.
fn drawn_columns(img: &image::RgbaImage) -> Option<(u32, u32)> {
    let mut span: Option<(u32, u32)> = None;
    for (x, _, p) in img.enumerate_pixels() {
        if p.0[3] > 8 {
            span = Some(match span {
                None => (x, x + 1),
                Some((a, b)) => (a.min(x), b.max(x + 1)),
            });
        }
    }
    span
}

/// The next mip level: each pixel the average of a 2x2 block, colours
/// weighted by their alpha so a transparent neighbour does not darken an
/// edge.
fn halve(src: &[u8], w: u32, h: u32) -> Vec<u8> {
    let (ow, oh) = (w / 2, h / 2);
    let mut out = vec![0u8; (ow * oh * 4) as usize];
    for oy in 0..oh {
        for ox in 0..ow {
            let (mut r, mut g, mut b, mut a) = (0u32, 0u32, 0u32, 0u32);
            for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
                let i = (((oy * 2 + dy) * w + ox * 2 + dx) * 4) as usize;
                let pa = src[i + 3] as u32;
                r += src[i] as u32 * pa;
                g += src[i + 1] as u32 * pa;
                b += src[i + 2] as u32 * pa;
                a += pa;
            }
            let o = ((oy * ow + ox) * 4) as usize;
            if let (Some(r), Some(g), Some(b)) = (r.checked_div(a), g.checked_div(a), b.checked_div(a)) {
                out[o] = r as u8;
                out[o + 1] = g as u8;
                out[o + 2] = b as u8;
            }
            out[o + 3] = (a / 4) as u8;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `w`x`h` PNG, transparent but for an opaque block of columns.
    fn png(w: u32, h: u32, cols: std::ops::Range<u32>) -> Vec<u8> {
        let mut img = image::RgbaImage::new(w, h);
        for y in 0..h {
            for x in cols.clone() {
                img.put_pixel(x, y, image::Rgba([200, 100, 50, 255]));
            }
        }
        let mut out = std::io::Cursor::new(Vec::new());
        img.write_to(&mut out, image::ImageFormat::Png).expect("png");
        out.into_inner()
    }

    #[test]
    fn every_ranked_speed_has_its_icon_and_no_other_speed_has_one() {
        for (speed, name) in [
            (1.0, "mod_100"),
            (1.15, "mod_15"),
            (1.25, "mod_25"),
            (1.35, "mod_35"),
            (1.45, "mod_45"),
            (0.87, "mod_87"),
            (0.8, "mod_80"),
            (0.75, "mod_75"),
        ] {
            assert_eq!(speed_icon_name(speed), Some(name), "{speed}");
        }
        for speed in [0.5, 0.9, 1.1, 1.2, 1.5, 2.0] {
            assert_eq!(speed_icon_name(speed), None, "{speed}");
        }
    }

    #[test]
    fn an_icon_keeps_its_height_and_loses_its_empty_sides() {
        let (icons, levels) = ModIcons::build(&[("mod_a".into(), png(135, 135, 40..95))]);
        let icon = icons.get("mod_a").expect("packed");
        let width = (icon.uv[2] - icon.uv[0]) * ATLAS_W as f32;
        let height = (icon.uv[3] - icon.uv[1]) * ATLAS_H as f32;
        assert!((width - 57.0).abs() < 0.01, "55 columns and a texel of air each side, got {width}");
        assert!((height - 135.0).abs() < 0.01, "{height}");
        assert!((icon.aspect - 57.0 / 135.0).abs() < 1e-4);
        assert_eq!(levels.len(), MIP_LEVELS as usize);
        assert_eq!(levels[0].len(), (ATLAS_W * ATLAS_H * 4) as usize);
        assert_eq!(levels[3].len(), ((ATLAS_W / 8) * (ATLAS_H / 8) * 4) as usize);
    }

    #[test]
    fn icons_stay_inside_their_cells_at_every_level() {
        // Two full-width icons side by side: at the smallest level the
        // texels of the first cell hold only the first icon's colour and
        // the second cell only the second's.
        let mut second = image::RgbaImage::new(135, 135);
        for p in second.pixels_mut() {
            *p = image::Rgba([10, 20, 250, 255]);
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        second.write_to(&mut bytes, image::ImageFormat::Png).expect("png");
        let (_, levels) =
            ModIcons::build(&[("mod_a".into(), png(135, 135, 0..135)), ("mod_b".into(), bytes.into_inner())]);
        let (w, cell) = (ATLAS_W / 8, CELL / 8);
        let small = &levels[3];
        for y in 0..cell {
            for x in 0..2 * cell {
                let i = ((y * w + x) * 4) as usize;
                if small[i + 3] == 0 {
                    continue;
                }
                let want = if x < cell { [200, 100, 50] } else { [10, 20, 250] };
                assert_eq!(&small[i..i + 3], &want, "texel {x},{y}");
            }
        }
    }

    #[test]
    fn a_broken_file_is_left_out_and_the_key_follows_the_list() {
        let good = png(135, 135, 10..120);
        let list = vec![("mod_bad".to_string(), vec![1, 2, 3]), ("mod_good".to_string(), good)];
        let (icons, _) = ModIcons::build(&list);
        assert!(icons.get("mod_bad").is_none());
        assert!(icons.get("mod_good").is_some());
        assert_eq!(icons.key(), source_key(&list));
        assert_ne!(icons.key(), 0);
        assert_eq!(source_key(&[]), 0);
        assert!(ModIcons::default().is_empty());
        assert_eq!(ModIcons::default().key(), 0);
        assert_ne!(source_key(&list), source_key(&list[1..]));
    }

    #[test]
    fn an_oversized_icon_is_scaled_into_its_cell() {
        let (icons, _) = ModIcons::build(&[("mod_big".into(), png(512, 512, 0..512))]);
        let icon = icons.get("mod_big").expect("packed");
        let height = (icon.uv[3] - icon.uv[1]) * ATLAS_H as f32;
        assert!(height <= (CELL - 8) as f32 + 0.01, "{height}");
    }
}
