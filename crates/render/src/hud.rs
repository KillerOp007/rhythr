//! HUD overlay: side-stat panels, health/progress bars, the combo ring, the
//! faint centre combo number and the title header — everything the game draws
//! on top of the 3D playfield.
//!
//! This module is pure CPU work: it rasterises the bundled font into a
//! coverage atlas once, derives the running stats for a song time, and emits
//! flat 2D [`HudVertex`] quads in **pixel space** (origin top-left). The
//! renderer uploads the atlas as an R8 texture and draws the quads in an
//! orthographic overlay pass over the scene. Every element is gated by the
//! player's config toggles, so turning one off in-game turns it off here too.

use fontdue::{Font, FontSettings};

use crate::config::srgb8_to_linear;
use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_sim::hitreg::{match_hits, MatchOutcome, DEFAULT_WINDOW_MS};

const FONT_BYTES: &[u8] = include_bytes!("../assets/hud-font.ttf");
/// Glyphs are rasterised once at this pixel height; text quads scale from it.
const BASE_PX: f32 = 96.0;
const ATLAS_W: usize = 2048;

/// A single overlay vertex in pixel space (origin top-left). `mode` selects
/// solid fill (0) vs. sampling the glyph atlas as coverage (1).
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HudVertex {
    pub pos: [f32; 2],
    pub uv: [f32; 2],
    pub color: [f32; 4],
    pub mode: f32,
    pub _pad: f32,
}

impl HudVertex {
    fn new(pos: [f32; 2], uv: [f32; 2], color: [f32; 4], mode: f32) -> HudVertex {
        HudVertex {
            pos,
            uv,
            color,
            mode,
            _pad: 0.0,
        }
    }
}

/// Placement metrics (at [`BASE_PX`]) plus the glyph's atlas rectangle.
#[derive(Clone, Copy, Default)]
struct Glyph {
    // Atlas pixel rectangle.
    ax: f32,
    ay: f32,
    aw: f32,
    ah: f32,
    // Baseline-relative metrics, y-up, at BASE_PX.
    xmin: f32,
    ymin: f32,
    advance: f32,
    present: bool,
}

/// A coverage atlas of printable ASCII plus the metrics to lay text out.
pub struct FontAtlas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u8>, // R8 coverage, row-major
    glyphs: [Glyph; 128],
}

impl FontAtlas {
    /// Rasterise the bundled font once into a packed coverage atlas.
    pub fn new() -> FontAtlas {
        let font = Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("bundled HUD font");

        // Rasterise every printable ASCII glyph at the base size.
        let mut cells: Vec<(usize, fontdue::Metrics, Vec<u8>)> = Vec::new();
        for c in 32u8..127 {
            let (m, bm) = font.rasterize(c as char, BASE_PX);
            cells.push((c as usize, m, bm));
        }

        // Shelf-pack into a fixed-width atlas.
        let (mut x, mut y, mut row_h) = (0usize, 0usize, 0usize);
        let mut placed: Vec<(usize, usize)> = Vec::with_capacity(cells.len());
        for (_, m, _) in &cells {
            if x + m.width + 1 > ATLAS_W {
                x = 0;
                y += row_h + 1;
                row_h = 0;
            }
            placed.push((x, y));
            x += m.width + 1;
            row_h = row_h.max(m.height);
        }
        let height = (y + row_h + 1).max(1).next_power_of_two();

        let mut pixels = vec![0u8; ATLAS_W * height];
        let mut glyphs = [Glyph::default(); 128];
        for ((code, m, bm), (px, py)) in cells.iter().zip(placed.iter()) {
            for row in 0..m.height {
                let dst = (py + row) * ATLAS_W + px;
                let src = row * m.width;
                pixels[dst..dst + m.width].copy_from_slice(&bm[src..src + m.width]);
            }
            glyphs[*code] = Glyph {
                ax: *px as f32,
                ay: *py as f32,
                aw: m.width as f32,
                ah: m.height as f32,
                xmin: m.xmin as f32,
                ymin: m.ymin as f32,
                advance: m.advance_width,
                present: true,
            };
        }

        FontAtlas {
            width: ATLAS_W,
            height,
            pixels,
            glyphs,
        }
    }

    fn glyph(&self, ch: char) -> Option<&Glyph> {
        let g = self.glyphs.get(ch as usize)?;
        g.present.then_some(g)
    }

    /// Advance width of `text` at pixel size `px`.
    pub fn measure(&self, text: &str, px: f32) -> f32 {
        let scale = px / BASE_PX;
        text.chars()
            .filter_map(|c| self.glyph(c))
            .map(|g| g.advance * scale)
            .sum()
    }
}

impl Default for FontAtlas {
    fn default() -> Self {
        Self::new()
    }
}

/// Horizontal anchor for a laid-out string.
#[derive(Clone, Copy, PartialEq)]
#[allow(dead_code)] // Left/Right are used as element layout grows.
enum Align {
    Left,
    Center,
    Right,
}

/// Accumulates overlay geometry in pixel space.
struct HudBuilder<'a> {
    atlas: &'a FontAtlas,
    verts: Vec<HudVertex>,
}

impl<'a> HudBuilder<'a> {
    fn new(atlas: &'a FontAtlas) -> HudBuilder<'a> {
        HudBuilder {
            atlas,
            verts: Vec::new(),
        }
    }

    /// A solid (untextured) rectangle.
    fn rect(&mut self, x: f32, y: f32, w: f32, h: f32, color: [f32; 4]) {
        if color[3] <= 0.0 || w <= 0.0 || h <= 0.0 {
            return;
        }
        let z = [0.0, 0.0];
        let (x0, y0, x1, y1) = (x, y, x + w, y + h);
        let quad = [
            HudVertex::new([x0, y0], z, color, 0.0),
            HudVertex::new([x1, y0], z, color, 0.0),
            HudVertex::new([x1, y1], z, color, 0.0),
            HudVertex::new([x0, y0], z, color, 0.0),
            HudVertex::new([x1, y1], z, color, 0.0),
            HudVertex::new([x0, y1], z, color, 0.0),
        ];
        self.verts.extend_from_slice(&quad);
    }

    /// A thin line segment as a quad of width `thick`.
    fn line(&mut self, a: [f32; 2], b: [f32; 2], thick: f32, color: [f32; 4]) {
        if color[3] <= 0.0 {
            return;
        }
        let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt();
        if len < 1e-3 {
            return;
        }
        let (nx, ny) = (-dy / len * thick * 0.5, dx / len * thick * 0.5);
        let z = [0.0, 0.0];
        let p = [
            [a[0] + nx, a[1] + ny],
            [b[0] + nx, b[1] + ny],
            [b[0] - nx, b[1] - ny],
            [a[0] - nx, a[1] - ny],
        ];
        for &[i, j, k] in &[[0usize, 1, 2], [0, 2, 3]] {
            self.verts.push(HudVertex::new(p[i], z, color, 0.0));
            self.verts.push(HudVertex::new(p[j], z, color, 0.0));
            self.verts.push(HudVertex::new(p[k], z, color, 0.0));
        }
    }

    /// Lay out `text` at pixel size `px`, anchored at (`x`, baseline `y`).
    /// Returns the advanced pen x.
    fn text(&mut self, text: &str, x: f32, y: f32, px: f32, align: Align, color: [f32; 4]) -> f32 {
        let scale = px / BASE_PX;
        let start = match align {
            Align::Left => x,
            Align::Center => x - self.atlas.measure(text, px) * 0.5,
            Align::Right => x - self.atlas.measure(text, px),
        };
        let mut pen = start;
        let (aw, ah) = (self.atlas.width as f32, self.atlas.height as f32);
        for c in text.chars() {
            let Some(g) = self.atlas.glyph(c) else {
                continue;
            };
            if g.aw > 0.0 && g.ah > 0.0 && color[3] > 0.0 {
                let gx = pen + g.xmin * scale;
                let gy = y - (g.ymin + g.ah) * scale; // top edge, y-down screen
                let (gw, gh) = (g.aw * scale, g.ah * scale);
                let (u0, v0) = (g.ax / aw, g.ay / ah);
                let (u1, v1) = ((g.ax + g.aw) / aw, (g.ay + g.ah) / ah);
                let quad = [
                    HudVertex::new([gx, gy], [u0, v0], color, 1.0),
                    HudVertex::new([gx + gw, gy], [u1, v0], color, 1.0),
                    HudVertex::new([gx + gw, gy + gh], [u1, v1], color, 1.0),
                    HudVertex::new([gx, gy], [u0, v0], color, 1.0),
                    HudVertex::new([gx + gw, gy + gh], [u1, v1], color, 1.0),
                    HudVertex::new([gx, gy + gh], [u0, v1], color, 1.0),
                ];
                self.verts.extend_from_slice(&quad);
            }
            pen += g.advance * scale;
        }
        pen
    }
}

/// Running gameplay values at a given song time, ready to render.
#[derive(Clone, Copy, Debug)]
pub struct HudStats {
    pub score: i64,
    pub points: f32,
    pub combo: u32,
    pub misses: u32,
    pub hits: u32,
    pub resolved: u32,
    pub accuracy_pct: f32,
    pub health: f32,
    pub grade: Grade,
    /// Milliseconds since the most recent hit (drives the combo ring's
    /// fill/wobble animation). Very large before the first hit.
    pub ms_since_hit: f64,
    /// Milliseconds since the most recent miss (drives the ring's drain
    /// animation). Very large before the first miss.
    pub ms_since_miss: f64,
    /// Combo ring side count (a stateful tier: +1 per full ring, −1 per
    /// miss — not derivable from the combo alone).
    pub ring_sides: u32,
    /// The ring's target fill fraction.
    pub ring_progress: f32,
    /// The fill the ring had when the last miss registered (drain start).
    pub ring_fill_at_miss: f32,
}

/// Accuracy letter grade. Thresholds are Rhythia/SS-style on final accuracy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Grade {
    SS,
    S,
    A,
    B,
    C,
    D,
}

impl Grade {
    /// Grade from an accuracy percentage (0..100). Official thresholds:
    /// SS 100%, S ≥99%, A 95–98.99%, B 90–94.99%, C 85–89.99%, D below.
    pub fn from_accuracy(acc: f32) -> Grade {
        match acc {
            a if a >= 100.0 => Grade::SS,
            a if a >= 99.0 => Grade::S,
            a if a >= 95.0 => Grade::A,
            a if a >= 90.0 => Grade::B,
            a if a >= 85.0 => Grade::C,
            _ => Grade::D,
        }
    }

    fn label(self) -> &'static str {
        match self {
            Grade::SS => "SS",
            Grade::S => "S",
            Grade::A => "A",
            Grade::B => "B",
            Grade::C => "C",
            Grade::D => "D",
        }
    }

    /// sRGB colour, pixel-picked from game/site screenshots of every grade.
    fn color(self) -> [u8; 3] {
        match self {
            Grade::SS => [144, 80, 224], // #9050e0 deep purple
            Grade::S => [144, 136, 248], // #9088f8 blue-violet
            Grade::A => [144, 248, 144], // #90f890 green
            Grade::B => [224, 248, 184], // #e0f8b8 pale lime
            Grade::C => [248, 240, 176], // #f8f0b0 pale yellow
            Grade::D => [248, 208, 176], // #f8d0b0 pale orange
        }
    }
}

/// Per-note score weight: a note hit at combo `k` is worth `100·k` in the
/// game (the "{k}x" ring is literally the points multiplier — verified at
/// combo 53: 100·(1+…+53) = 143,100, and at 185: 1,720,500). The SCORE is
/// the raw cumulative sum — the game applies no normalisation.
fn note_multiplier(combo: u32) -> f64 {
    combo as f64
}

/// Per-replay hit resolution, computed once and reused for every frame.
pub struct HudState {
    outcome: MatchOutcome,
}

impl HudState {
    pub fn new(map: &Map, replay: &Replay) -> HudState {
        HudState {
            outcome: match_hits(&map.notes, &replay.frames, DEFAULT_WINDOW_MS),
        }
    }

    /// Running stats at `song_time_ms` — hits/combo/misses derived from the
    /// per-note resolution, score/points interpolated from the replay totals
    /// by hit fraction (the exact in-game score curve isn't in the file).
    pub fn stats_at(&self, map: &Map, replay: &Replay, song_time_ms: f64) -> HudStats {
        let mut hits = 0u32;
        let mut misses = 0u32;
        let mut resolved = 0u32;
        let mut combo = 0u32;
        let mut acc_weight = 0.0f64;
        let mut last_hit_ms = f64::NEG_INFINITY;
        let mut last_miss_ms = f64::NEG_INFINITY;
        let mut ring = RingState::new();
        let mut ring_fill_at_miss = 0.0f32;
        for r in &self.outcome.results {
            let note_t = map.notes[r.note_index].time_ms as f64;
            if r.hit {
                let ht = r.hit_ms.unwrap_or(note_t);
                if ht <= song_time_ms {
                    hits += 1;
                    resolved += 1;
                    combo += 1;
                    acc_weight += note_multiplier(combo);
                    last_hit_ms = ht;
                    ring.on_hit(ht);
                }
            } else if note_t + DEFAULT_WINDOW_MS < song_time_ms {
                misses += 1;
                resolved += 1;
                combo = 0;
                last_miss_ms = note_t + DEFAULT_WINDOW_MS;
                ring_fill_at_miss = ring.on_miss(last_miss_ms);
            }
        }
        let accuracy_pct = if hits + misses == 0 {
            100.0
        } else {
            hits as f32 / (hits + misses) as f32 * 100.0
        };
        HudStats {
            // The game's live score is literally 100 per combo step: at 185x
            // it reads 1,720,500 = 100·(1+…+185). No normalisation — the raw
            // formula matches the game frame-for-frame.
            score: (acc_weight * 100.0) as i64,
            points: live_rp(replay.points, accuracy_pct, replay.accuracy_pct),
            combo,
            misses,
            hits,
            resolved,
            accuracy_pct,
            health: health_at(replay, song_time_ms),
            grade: Grade::from_accuracy(accuracy_pct),
            ms_since_hit: (song_time_ms - last_hit_ms).max(0.0),
            ms_since_miss: (song_time_ms - last_miss_ms).max(0.0),
            ring_sides: ring.sides_at(song_time_ms),
            ring_progress: ring.progress(),
            ring_fill_at_miss,
        }
    }

    /// Per-note hit resolution, in note order (for PushBack rendering).
    pub fn results(&self) -> &[rhythia_sim::hitreg::NoteResult] {
        &self.outcome.results
    }

    /// Grid cells of notes missed within the last [`MISS_X_MS`] before
    /// `song_time_ms`, with how long ago each miss registered — the red X
    /// markers the game flashes on a missed note's cell.
    pub fn recent_misses(&self, map: &Map, song_time_ms: f64) -> Vec<(f32, f32, f64)> {
        self.outcome
            .results
            .iter()
            .filter(|r| !r.hit)
            .filter_map(|r| {
                let note = &map.notes[r.note_index];
                let miss_t = note.time_ms as f64;
                let age = song_time_ms - miss_t;
                (0.0..MISS_X_MS)
                    .contains(&age)
                    .then_some((note.x, note.y, age))
            })
            .collect()
    }
}

/// Live RP (Rhythia Points, like osu pp) at the current accuracy. The real
/// per-play value is an osu!relax-style server calculation over the map's
/// star rating (per the Rhythia wiki), which a renderer can't reproduce —
/// but its accuracy response observed in footage is a steep curve above
/// ~90% (0 RP at 92.31%, 2 at 96.67%, 4 at 97.37% on one map, a constant 78
/// through a 100%-accuracy run on another). We model that shape with a
/// cubic above 90% and normalise it to the replay's stored final RP, so a
/// constant-accuracy run shows a constant value and every run ends exactly
/// on its real RP.
fn live_rp(final_rp: f32, acc_now_pct: f32, acc_final_pct: f32) -> f32 {
    fn curve(acc: f32) -> f32 {
        ((acc - 90.0) / 10.0).max(0.0).powi(3)
    }
    let denom = curve(acc_final_pct);
    if denom <= 1e-6 || final_rp <= 0.0 {
        return final_rp.max(0.0);
    }
    final_rp * curve(acc_now_pct) / denom
}

/// Linear-interpolated health (0..1) at `song_time_ms` from the frame stream.
fn health_at(replay: &Replay, t: f64) -> f32 {
    let f = &replay.frames;
    if f.is_empty() {
        return 1.0;
    }
    match f.binary_search_by(|fr| fr.ms.partial_cmp(&t).unwrap_or(std::cmp::Ordering::Less)) {
        Ok(i) => f[i].health,
        Err(0) => f[0].health,
        Err(i) if i >= f.len() => f[f.len() - 1].health,
        Err(i) => {
            let (a, b) = (&f[i - 1], &f[i]);
            let span = (b.ms - a.ms) as f32;
            if span <= 0.0 {
                a.health
            } else {
                let u = ((t - a.ms) as f32 / span).clamp(0.0, 1.0);
                a.health + (b.health - a.health) * u
            }
        }
    }
}

/// Format an integer with thousands separators, e.g. 143100 -> "143,100".
fn thousands(n: i64) -> String {
    let s = n.abs().to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    if n < 0 {
        format!("-{out}")
    } else {
        out
    }
}

/// Format a song time in ms as `MM:SS` (leading-zero minutes, as the game).
fn clock(ms: f64) -> String {
    let total = (ms.max(0.0) / 1000.0) as i64;
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// Playfield geometry the HUD anchors to, in pixels.
pub struct Playfield {
    pub cx: f32,
    pub cy: f32,
    pub half: f32,
}

/// Build all HUD geometry for a frame. Positions/sizes scale with frame size.
#[allow(clippy::too_many_arguments)]
pub fn build_hud(
    atlas: &FontAtlas,
    cfg: &crate::config::SkinConfig,
    stats: &HudStats,
    replay: &Replay,
    map: &Map,
    song_time_ms: f64,
    field: &Playfield,
    miss_marks: &[(f32, f32, f64)],
    width: u32,
    height: u32,
) -> Vec<HudVertex> {
    let hud = &cfg.hud;
    let mut b = HudBuilder::new(atlas);
    let (w, _h) = (width as f32, height as f32);
    let refd = w.min(_h);
    // Sizes measured against a same-moment game/render pair at the user's
    // settings (ratios of the playfield bracket span).
    let label_px = refd * 0.0163;
    let value_px = refd * 0.0237 * cfg.interface_values_font_size;
    let panel_a = cfg.panel_opacity.clamp(0.0, 1.0);
    let label_col = srgb8_to_linear([138, 138, 146], panel_a);
    let value_col = srgb8_to_linear(cfg.interface_text_color, panel_a);

    // Big faint centre combo number.
    if hud.playfield_combo_text && stats.combo > 0 {
        let cy = field.cy - field.half + field.half * 2.0 * (hud.combo_text_vpos_pct / 100.0);
        // The config font size maps to ~0.44px em per unit at 1440p (190 →
        // "185" 155px wide, measured against the game; DejaVu runs a bit
        // wider than the game's face, hence not a round 0.5).
        let px = hud.combo_text_font_size * 0.44 * refd / 1440.0;
        let col = srgb8_to_linear(hud.combo_text_color, hud.combo_text_opacity);
        b.text(
            &stats.combo.to_string(),
            field.cx,
            cy + px * 0.35,
            px,
            Align::Center,
            col,
        );
    }

    // Column centre just past the box; PanelGap pushes it further out.
    let col_dx = field.half + refd * 0.085 + cfg.panel_gap * refd / 1440.0;
    let left_x = field.cx - col_dx;
    let right_x = field.cx + col_dx;
    let row = refd * 0.132; // vertical stride between stat entries

    // Optional panel background cards behind the stat columns.
    if cfg.panel_background_opacity > 0.0 {
        let card = srgb8_to_linear(cfg.panel_color, cfg.panel_background_opacity);
        let (cw, ch2) = (refd * 0.16, field.half * 1.9);
        for x in [left_x, right_x] {
            b.rect(x - cw * 0.5, field.cy - ch2 * 0.5, cw, ch2, card);
        }
    }

    // Combo ring (top of the left column).
    if hud.combo_ring {
        combo_ring(
            &mut b,
            left_x,
            field.cy - field.half * 0.62,
            refd * 0.048,
            stats,
            srgb8_to_linear(hud.combo_ring_color, hud.combo_ring_opacity),
        );
    }

    // Red X on each freshly missed note's cell: scales/fades in over the
    // first ~100 ms, holds, fades out by 500 ms (from footage; #e83040).
    if hud.miss_effect_opacity > 0.0 {
        for &(mx, my, age) in miss_marks {
            let a_in = (age / 100.0).clamp(0.0, 1.0) as f32;
            let a_out = 1.0 - ((age - 300.0) / (MISS_X_MS - 300.0)).clamp(0.0, 1.0) as f32;
            let alpha = a_in * a_out * hud.miss_effect_opacity;
            let half = refd * 0.032 * (0.6 + 0.4 * a_in);
            let col = srgb8_to_linear([232, 48, 64], alpha);
            let thick = half * 0.42;
            // Two thick strokes, slightly tilted like the game's hand-drawn X.
            let tilt = 0.12f32;
            let (s, c) = (std::f32::consts::FRAC_PI_4 + tilt).sin_cos();
            b.line(
                [mx - half * c, my - half * s],
                [mx + half * c, my + half * s],
                thick,
                col,
            );
            let (s2, c2) = (-std::f32::consts::FRAC_PI_4 + tilt).sin_cos();
            b.line(
                [mx - half * c2, my - half * s2],
                [mx + half * c2, my + half * s2],
                thick,
                col,
            );
        }
    }

    // A label-over-value stat entry, centred on the column.
    let entry = |b: &mut HudBuilder, x: f32, y: f32, label: &str, value: &str, vcol: [f32; 4]| {
        b.text(label, x, y, label_px, Align::Center, label_col);
        b.text(value, x, y + value_px * 1.15, value_px, Align::Center, vcol);
    };

    // Left column (below the ring): Pauses, Grade (bare colour letter), Acc.
    let mut ly = field.cy - field.half * 0.18;
    if hud.pauses {
        entry(&mut b, left_x, ly, "PAUSES", "0", value_col);
        ly += row;
    }
    if hud.grade {
        let g = stats.grade;
        b.text(
            g.label(),
            left_x,
            ly + value_px * 0.9,
            value_px * 1.5,
            Align::Center,
            srgb8_to_linear(g.color(), 1.0),
        );
        ly += row;
    }
    if hud.accuracy {
        // "--" until the first note resolves; then two decimals, trailing
        // zeros stripped ("92.31%", "100%") — as the game formats it.
        let acc = if stats.resolved == 0 {
            "--".to_string()
        } else {
            let s = format!("{:.2}", stats.accuracy_pct);
            format!("{}%", s.trim_end_matches('0').trim_end_matches('.'))
        };
        entry(&mut b, left_x, ly, "ACCURACY", &acc, value_col);
    }

    // Right column: Score, Points, Misses, Notes.
    let mut ry = field.cy - field.half * 0.79;
    if hud.score {
        entry(
            &mut b,
            right_x,
            ry,
            "SCORE",
            &thousands(stats.score),
            value_col,
        );
        ry += row;
    }
    if hud.points {
        // RP (Rhythia Points, like osu pp) — "--" until a note resolves.
        let pts = if stats.resolved == 0 {
            "--".to_string()
        } else {
            format!("{:.0}", stats.points)
        };
        entry(&mut b, right_x, ry, "POINTS", &pts, value_col);
        ry += row;
    }
    if hud.misses {
        entry(
            &mut b,
            right_x,
            ry,
            "MISSES",
            &stats.misses.to_string(),
            value_col,
        );
        ry += row;
    }
    if hud.notes {
        entry(
            &mut b,
            right_x,
            ry,
            "NOTES",
            &format!("{}/{}", stats.hits, stats.resolved),
            value_col,
        );
    }

    // Health bar just below the playfield.
    if hud.health_bar {
        let bw = field.half * 2.0;
        let bx = field.cx - field.half;
        let by = field.cy + field.half + refd * 0.014;
        let bh = (refd * 0.0088).max(2.0);
        let frac = stats.health.clamp(0.0, 1.0);
        // faint track + filled portion
        b.rect(bx, by, bw, bh, srgb8_to_linear([40, 40, 44], 0.6));
        b.rect(
            bx,
            by,
            bw * frac,
            bh,
            srgb8_to_linear(hud.health_bar_color, hud.health_bar_alpha),
        );
    }

    // Song progress bar just above the playfield.
    if hud.song_progress_bar {
        let dur = replay.length_ms().max(map.meta.duration_ms as f64).max(1.0);
        let frac = (song_time_ms / dur).clamp(0.0, 1.0) as f32;
        let bw = field.half * 2.0;
        let bx = field.cx - field.half;
        let by = field.cy - field.half - refd * 0.016 - refd * 0.0075;
        let bh = (refd * 0.0075).max(2.0);
        // Grey track with a light elapsed portion, as in the footage (the
        // config colour is black there yet the track reads mid-grey).
        b.rect(bx, by, bw, bh, srgb8_to_linear([64, 64, 68], 0.9));
        b.rect(
            bx,
            by,
            bw * frac,
            bh,
            srgb8_to_linear([225, 225, 230], hud.song_progress_alpha),
        );
    }

    // Speed notation under the health bar: "S" plus coloured step dots and a
    // +/- sign. Ranked speeds (user-verified): 0.75 S··−, 0.8 S·−, 0.87 S−,
    // 1.0 nothing, 1.15 S+, 1.25 S:+, 1.35 S::+, 1.45 S:::+ (dot columns of
    // two above 1x, single dots below; green→orange→red). Anything else is
    // unranked and shows a slashed circle.
    if hud.speed_label {
        let sy = field.cy + field.half + refd * 0.014 + refd * 0.0088 + refd * 0.020;
        speed_label(&mut b, field.cx, sy, refd, replay.speed, value_col);
    }

    // Title header + clock, sitting just above the playfield (as the game).
    if hud.song_info {
        let title = if replay.player_name.is_empty() {
            map.meta.song_name.clone()
        } else {
            format!(
                "Watching {} play {}",
                replay.player_name, map.meta.song_name
            )
        };
        let ty = field.cy - field.half - refd * 0.053;
        b.text(
            &title,
            field.cx,
            ty,
            refd * 0.0187,
            Align::Center,
            value_col,
        );
        let dur = replay.length_ms().max(map.meta.duration_ms as f64);
        let time = format!("{} / {}", clock(song_time_ms), clock(dur));
        b.text(
            &time,
            field.cx,
            ty + refd * 0.024,
            refd * 0.0149,
            Align::Center,
            label_col,
        );
    }

    // Fail vignette: a red edge glow that grows as health drains, banded to
    // fake a gradient (no gradient primitive in the overlay pass).
    if cfg.fail_vignette_opacity > 0.0 {
        let danger = (1.0 - stats.health.clamp(0.0, 1.0)).powi(2);
        let alpha = cfg.fail_vignette_opacity * danger;
        if alpha > 0.002 {
            let bands = 6;
            let depth = refd * 0.06;
            for k in 0..bands {
                let a = srgb8_to_linear([185, 25, 30], alpha * (1.0 - k as f32 / bands as f32));
                let t = depth / bands as f32;
                let o = k as f32 * t;
                b.rect(o, o, w - 2.0 * o, t, a); // top
                b.rect(o, _h - o - t, w - 2.0 * o, t, a); // bottom
                b.rect(o, o + t, t, _h - 2.0 * (o + t), a); // left
                b.rect(w - o - t, o + t, t, _h - 2.0 * (o + t), a); // right
            }
        }
    }

    b.verts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grade_thresholds_are_official() {
        // Official table: SS 100, S >=99, A 95-98.99, B 90-94.99, C 85-89.99.
        assert_eq!(Grade::from_accuracy(100.0), Grade::SS);
        assert_eq!(Grade::from_accuracy(99.0), Grade::S);
        assert_eq!(Grade::from_accuracy(98.99), Grade::A);
        assert_eq!(Grade::from_accuracy(95.0), Grade::A);
        assert_eq!(Grade::from_accuracy(94.99), Grade::B);
        assert_eq!(Grade::from_accuracy(90.0), Grade::B);
        assert_eq!(Grade::from_accuracy(89.99), Grade::C);
        assert_eq!(Grade::from_accuracy(85.0), Grade::C);
        assert_eq!(Grade::from_accuracy(84.99), Grade::D);
        // Footage fixed points stay consistent: 97.83/95.24 A, 92.31 B.
        assert_eq!(Grade::from_accuracy(97.83), Grade::A);
        assert_eq!(Grade::from_accuracy(92.31), Grade::B);
    }

    /// Feeds hits 100 ms apart starting at t=0; returns (state, end time).
    fn ring_after_hits(n: usize) -> (RingState, f64) {
        let mut r = RingState::new();
        let mut t = 0.0;
        for _ in 0..n {
            t += 100.0;
            r.on_hit(t);
        }
        (r, t)
    }

    #[test]
    fn ring_gains_a_side_every_8_streak_hits_capped_at_octagon() {
        // Clean-run fixed points from footage: 0x/7x triangle, 8x square
        // (empty), 15x square 7/8, 16x pentagon, 24x hexagon, 32x heptagon,
        // 40x octagon full — and stays full (53x, 186x).
        let shape = |n: usize| {
            let (r, t) = ring_after_hits(n);
            (r.sides_at(t), r.progress())
        };
        assert_eq!(shape(0), (3, 0.0));
        assert_eq!(shape(7), (3, 7.0 / 8.0));
        assert_eq!(shape(8), (4, 0.0));
        assert_eq!(shape(15), (4, 7.0 / 8.0));
        assert_eq!(shape(16), (5, 0.0));
        assert_eq!(shape(24), (6, 0.0));
        assert_eq!(shape(32), (7, 0.0));
        assert_eq!(shape(40), (8, 1.0));
        assert_eq!(shape(53), (8, 1.0));
        assert_eq!(shape(186), (8, 1.0));
    }

    #[test]
    fn ring_shrinks_after_a_miss_until_a_hit_freezes_it() {
        // Footage: a 35x heptagon read as a hexagon right after one miss and
        // a pentagon after the second; "1x" kept counting in the pentagon
        // because the player kept hitting (the hit freezes the shrink).
        let (mut r, t) = ring_after_hits(35);
        assert_eq!(r.sides_at(t), 7);
        let fill = r.on_miss(t + 10.0);
        assert!((fill - 3.0 / 8.0).abs() < 1e-6); // 35 % 8 = 3
        assert_eq!(r.sides_at(t + 20.0), 6); // one side drops immediately
        r.on_miss(t + 400.0);
        assert_eq!(r.sides_at(t + 410.0), 5);
        r.on_hit(t + 700.0); // freezes the pentagon
        assert_eq!(r.sides_at(t + 710.0), 5);
        assert_eq!(r.sides_at(t + 5000.0), 5); // frozen, no further decay
                                               // Growth follows the pure combo formula: the pentagon floor holds
                                               // until the streak catches up; the octagon needs a true 40+ streak
                                               // (user-reported: it must NOT refill at ~30 combo after a miss).
        let mut now = t + 700.0;
        for _ in 0..7 {
            now += 100.0;
            r.on_hit(now); // streak 8
        }
        assert_eq!((r.sides_at(now), r.progress()), (5, 0.0));
        for _ in 0..16 {
            now += 100.0;
            r.on_hit(now); // streak 24
        }
        assert_eq!((r.sides_at(now), r.progress()), (6, 0.0));
        for _ in 0..16 {
            now += 100.0;
            r.on_hit(now); // streak 40
        }
        assert_eq!((r.sides_at(now), r.progress()), (8, 1.0));
    }

    #[test]
    fn ring_decays_to_the_empty_triangle_when_idle_after_a_miss() {
        // User: losing an 80 combo and not hitting anything brings the ring
        // back to the empty triangle.
        let (mut r, t) = ring_after_hits(80);
        assert_eq!(r.sides_at(t), 8);
        r.on_miss(t);
        assert_eq!(r.sides_at(t + 10.0), 7);
        assert_eq!(r.sides_at(t + 5.0 * RING_SHRINK_STEP_MS + 10.0), 3);
        assert!((r.progress() - 0.0).abs() < 1e-6);
        // The shape never shrinks below the triangle.
        let mut tri = RingState::new();
        tri.on_miss(0.0);
        assert_eq!(tri.sides_at(60_000.0), 3);
    }

    #[test]
    fn thousands_separates() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(143100), "143,100");
        assert_eq!(thousands(34084600), "34,084,600");
        assert_eq!(thousands(-2500), "-2,500");
    }

    #[test]
    fn clock_is_mm_ss() {
        assert_eq!(clock(0.0), "00:00");
        assert_eq!(clock(24_751.0), "00:24");
        assert_eq!(clock(443_000.0), "07:23");
        assert_eq!(clock(-5.0), "00:00");
    }

    #[test]
    fn score_weight_is_cumulative_combo() {
        // The verified game formula: a full 53-combo run weighs 1+…+53 = 1431,
        // which scaled by 100 is the game's 143,100 at combo 53.
        let w: f64 = (1..=53).map(note_multiplier).sum();
        assert_eq!(w, 1431.0);
        assert_eq!((w * 100.0) as i64, 143_100);
    }

    #[test]
    fn live_rp_tracks_accuracy_and_lands_on_the_final_value() {
        // A constant-accuracy run shows its final RP the whole way through
        // (the 100%-run footage shows a constant 78).
        assert!((live_rp(78.0, 100.0, 100.0) - 78.0).abs() < 1e-3);
        // At the end the value is exactly the stored RP.
        assert!((live_rp(20.0, 98.1775, 98.1775) - 20.0).abs() < 1e-3);
        // Higher current accuracy than final → higher live value; below 90%
        // it bottoms out at zero (footage: 0 RP at 92.31% early in a run).
        assert!(live_rp(20.0, 100.0, 98.0) > 20.0);
        assert_eq!(live_rp(20.0, 88.0, 98.0), 0.0);
        // Degenerate final accuracy doesn't divide by zero.
        assert_eq!(live_rp(5.0, 95.0, 85.0), 5.0);
    }

    #[test]
    fn font_atlas_lays_out_ascii() {
        let atlas = FontAtlas::new();
        assert!(atlas.glyph('A').is_some());
        assert!(atlas.glyph('5').is_some());
        // A wider string measures wider.
        assert!(atlas.measure("SCORE", 40.0) > atlas.measure("S", 40.0));
    }
}

/// Hits per combo-ring tier: the polygon gains a side every 8 streak hits
/// (triangle → octagon) — verified frame-by-frame against game footage.
const RING_TIER: u32 = 8;
/// The fill animates toward the new progress over this long after a hit.
const RING_FILL_MS: f64 = 150.0;
/// After a miss the leftover fill drains to zero over this long.
const RING_DRAIN_MS: f64 = 250.0;
/// The shape wobbles this long when it gains a side (except becoming the
/// octagon), and a miss X marker lives this long.
const RING_WOBBLE_MS: f64 = 250.0;
/// Lifetime of the red X drawn on a missed note's cell.
const MISS_X_MS: f64 = 500.0;

/// The combo ring's shape state. The side count is **not** a pure function
/// of the current combo: a full ring (8 streak hits) adds a side, and each
/// miss removes one (min triangle) while the combo itself resets to 0 —
/// verified in footage where a 35x heptagon dropped to a hexagon then a
/// pentagon over two misses, and "1x" kept counting inside the pentagon.
/// The octagon (reached at 40 streak on a clean run) renders full.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RingState {
    /// Post-miss floor the shape shrinks from (3..=8).
    floor: u32,
    /// Song time the shrink is anchored to; `None` when frozen (a hit after
    /// the miss stops further shrinking).
    shrink_since: Option<f64>,
    /// Current hit streak (== displayed combo).
    streak: u32,
}

/// After a miss the shape steps down one side immediately, then one more
/// per this interval until it reaches the pure-formula size (triangle at
/// combo 0) — unless a hit freezes it first. Reconciles the footage (a 35x
/// heptagon read as a pentagon while the player kept hitting) with the
/// user's report that an idle combo loss ends back at the empty triangle.
const RING_SHRINK_STEP_MS: f64 = 1000.0;

impl RingState {
    fn new() -> RingState {
        RingState {
            floor: 3,
            shrink_since: None,
            streak: 0,
        }
    }

    fn on_hit(&mut self, now_ms: f64) {
        if self.shrink_since.is_some() {
            // A hit freezes the post-miss shrink at its current size.
            self.floor = self.sides_at(now_ms);
            self.shrink_since = None;
        }
        self.streak += 1;
    }

    /// Registers a miss; returns the pre-miss fill so the drain animation
    /// can start from it.
    fn on_miss(&mut self, now_ms: f64) -> f32 {
        let fill = self.progress();
        // One side drops immediately; the rest shrink over time.
        self.floor = self.sides_at(now_ms).saturating_sub(1).max(3);
        self.shrink_since = Some(now_ms);
        self.streak = 0;
        fill
    }

    /// Displayed side count at `now_ms`. Growth always follows the pure
    /// combo formula ("same combo, same progress, up to 40"); the post-miss
    /// floor only holds the shape up — and decays a side per
    /// [`RING_SHRINK_STEP_MS`] while no hit has frozen it.
    fn sides_at(&self, now_ms: f64) -> u32 {
        let floor = match self.shrink_since {
            Some(t0) => {
                let steps = ((now_ms - t0).max(0.0) / RING_SHRINK_STEP_MS) as u32;
                self.floor.saturating_sub(steps).max(3)
            }
            None => self.floor,
        };
        (3 + self.streak / RING_TIER).max(floor).min(8)
    }

    /// Target fill fraction of the outline. Only a true 40+ streak renders
    /// the ring full — a shape held up by the post-miss floor keeps its
    /// normal fill cycle.
    fn progress(&self) -> f32 {
        if self.streak >= 5 * RING_TIER {
            1.0
        } else {
            (self.streak % RING_TIER) as f32 / RING_TIER as f32
        }
    }
}

/// Colours of the speed step marks, indexed from the ± sign outward: the
/// mark next to the sign is green, then orange, then red (so 1.45x reads
/// red–yellow–green left to right, per the zoomed screenshot).
const SPEED_DOT_COLORS: [[u8; 3]; 3] = [
    [140, 200, 70], // green
    [235, 180, 60], // yellow/orange
    [225, 70, 60],  // red
];

/// Draw the speed-modifier notation centred at (`cx`, baseline `y`): "S",
/// step dots, and a +/- sign — or a slashed circle for unranked speeds.
fn speed_label(b: &mut HudBuilder, cx: f32, y: f32, refd: f32, speed: f32, col: [f32; 4]) {
    let px = refd * 0.0163;
    let close = |s: f32| (speed - s).abs() < 0.005;
    // (steps, upward?) for each ranked speed; 1.0x shows nothing.
    let ranked: Option<(usize, bool)> = if close(1.0) {
        return;
    } else if close(0.87) {
        Some((0, false))
    } else if close(0.8) {
        Some((1, false))
    } else if close(0.75) {
        Some((2, false))
    } else if close(1.15) {
        Some((0, true))
    } else if close(1.25) {
        Some((1, true))
    } else if close(1.35) {
        Some((2, true))
    } else if close(1.45) {
        Some((3, true))
    } else {
        None
    };

    let Some((steps, up)) = ranked else {
        // Unranked: a slashed circle.
        let r = px * 0.55;
        let cy = y - px * 0.35;
        let thick = (r * 0.22).max(1.5);
        let n = 20;
        for k in 0..n {
            let a0 = std::f32::consts::TAU * k as f32 / n as f32;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / n as f32;
            b.line(
                [cx + r * a0.cos(), cy + r * a0.sin()],
                [cx + r * a1.cos(), cy + r * a1.sin()],
                thick,
                col,
            );
        }
        let d = r * std::f32::consts::FRAC_1_SQRT_2;
        b.line([cx - d, cy - d], [cx + d, cy + d], thick, col);
        return;
    };

    let sign = if up { "+" } else { "-" };
    let dot = (px * 0.16).max(1.5);
    // Above 1x each step is a "<"-shaped triple of dots — a middle dot with
    // an upper and lower dot one stride to its right — and consecutive steps
    // interlock (a step's outer dots share the column of the next step's
    // middle dot), reading like dotted pluses. Below 1x a step is a single
    // dot. Colours run green→orange→red outward from the sign (so 1.45x is
    // red, yellow, green left to right).
    let stride = dot * 1.8;
    let marks_w = if up {
        steps as f32 * stride + dot
    } else {
        steps as f32 * (dot * 2.2)
    };
    let total = b.atlas.measure("S", px) + px * 0.10 + marks_w + b.atlas.measure(sign, px);
    let mut x = cx - total * 0.5;
    x = b.text("S", x, y, px, Align::Left, col) + px * 0.10;
    let mid_y = y - px * 0.30;
    for i in 0..steps {
        let c = srgb8_to_linear(SPEED_DOT_COLORS[(steps - 1 - i).min(2)], col[3]);
        if up {
            let mx = x + i as f32 * stride;
            b.rect(mx, mid_y, dot, dot, c);
            b.rect(mx + stride, mid_y - stride, dot, dot, c);
            b.rect(mx + stride, mid_y + stride, dot, dot, c);
        } else {
            b.rect(x + i as f32 * dot * 2.2, mid_y, dot, dot, c);
        }
    }
    b.text(sign, x + marks_w, y, px, Align::Left, col);
}

/// Converts a .NET tick timestamp (100ns units since 0001-01-01) to a
/// `MM/DD/YYYY HH:MM:SS` string, as the results screen shows.
fn format_ticks(ticks: i64) -> String {
    const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;
    let secs = (ticks - UNIX_EPOCH_TICKS) / 10_000_000;
    let days = secs.div_euclid(86_400);
    let tod = secs.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!(
        "{:02}/{:02}/{:04} {:02}:{:02}:{:02}",
        m,
        d,
        y,
        tod / 3600,
        (tod / 60) % 60,
        tod % 60
    )
}

/// Health-graph line colour by health level (green → orange → red), as the
/// results screen colours its curve.
fn health_color(h: f32) -> [u8; 3] {
    if h > 0.6 {
        [80, 200, 120]
    } else if h > 0.3 {
        [235, 180, 60]
    } else {
        [225, 90, 60]
    }
}

/// Build the results screen shown after a finish or fail: title block,
/// big grade, statistics with dotted leaders, health graph and mods box.
/// The blurred-cover background and the cover image itself are drawn by the
/// renderer (textured quads); this emits the text/line geometry over them.
#[allow(clippy::too_many_arguments)]
pub fn build_results(
    atlas: &FontAtlas,
    replay: &Replay,
    map: &Map,
    stats: &HudStats,
    width: u32,
    height: u32,
) -> Vec<HudVertex> {
    let mut b = HudBuilder::new(atlas);
    let (w, h) = (width as f32, height as f32);
    let white = srgb8_to_linear([240, 240, 245], 1.0);
    let green = srgb8_to_linear([60, 220, 90], 1.0);
    let line_col = srgb8_to_linear([120, 120, 126], 0.55);

    // --- Title block (right of the cover, drawn by the renderer) ---------
    let tx = w * 0.227;
    b.text(
        &map.meta.song_name,
        tx,
        h * 0.082,
        h * 0.037,
        Align::Left,
        white,
    );
    if !map.meta.title.is_empty() {
        b.text(
            &format!("< {} >", map.meta.title),
            tx,
            h * 0.117,
            h * 0.026,
            Align::Left,
            green,
        );
    }
    if !map.meta.mappers.is_empty() {
        b.text(
            &format!("by {}", map.meta.mappers.join(", ")),
            tx,
            h * 0.152,
            h * 0.024,
            Align::Left,
            white,
        );
    }
    let played = format!(
        "Played by {} on {}",
        replay.player_name,
        format_ticks(replay.timestamp_ticks)
    );
    b.text(&played, tx, h * 0.34, h * 0.026, Align::Left, white);

    // --- Big grade, top right --------------------------------------------
    let failed = replay.failed();
    let (glabel, gcol) = if failed {
        ("F", [200u8, 40, 45])
    } else {
        (stats.grade.label(), stats.grade.color())
    };
    b.text(
        glabel,
        w * 0.895,
        h * 0.26,
        h * 0.13,
        Align::Center,
        srgb8_to_linear(gcol, 1.0),
    );

    // --- Statistics with dotted leaders ----------------------------------
    b.text(
        "Statistics",
        w * 0.032,
        h * 0.43,
        h * 0.028,
        Align::Left,
        white,
    );
    let label_px = h * 0.0225;
    let mut rows: Vec<(String, String)> = vec![
        ("Score".into(), thousands(stats.score)),
        ("Accuracy".into(), format!("{:.2}%", stats.accuracy_pct)),
        ("Hits".into(), stats.hits.to_string()),
        ("Misses".into(), stats.misses.to_string()),
        ("RP".into(), format!("{:.0}", replay.points)),
    ];
    if failed {
        rows.push(("Fail Time".into(), clock(replay.fail_time_ms as f64)));
    }
    let (lx, rx) = (w * 0.038, w * 0.475);
    let mut y = h * 0.492;
    for (label, value) in &rows {
        b.text(label, lx, y, label_px, Align::Left, white);
        b.text(value, rx, y, label_px, Align::Right, white);
        let line_a = lx + atlas.measure(label, label_px) + w * 0.012;
        let line_b = rx - atlas.measure(value, label_px) - w * 0.012;
        if line_b > line_a {
            b.line(
                [line_a, y - label_px * 0.30],
                [line_b, y - label_px * 0.30],
                (h * 0.0022).max(1.0),
                line_col,
            );
        }
        y += h * 0.063;
    }

    // --- Health graph ------------------------------------------------------
    b.text(
        "Health Graph",
        w * 0.515,
        h * 0.43,
        h * 0.028,
        Align::Left,
        white,
    );
    let (gx0, gx1) = (w * 0.53, w * 0.965);
    let (gy0, gy1) = (h * 0.50, h * 0.645);
    let end_ms = if failed {
        replay.fail_time_ms as f64
    } else {
        replay.length_ms()
    }
    .max(1.0);
    b.text("00:00", gx0, gy0 - h * 0.014, h * 0.02, Align::Left, white);
    b.text(
        &clock(end_ms),
        gx1,
        gy0 - h * 0.014,
        h * 0.02,
        Align::Right,
        white,
    );
    let steps = 240;
    let mut prev: Option<([f32; 2], f32)> = None;
    for k in 0..=steps {
        let t = end_ms * k as f64 / steps as f64;
        let hp = health_at(replay, t).clamp(0.0, 1.0);
        let p = [
            gx0 + (gx1 - gx0) * k as f32 / steps as f32,
            gy1 - (gy1 - gy0) * hp,
        ];
        if let Some((q, qhp)) = prev {
            b.line(
                q,
                p,
                (h * 0.0022).max(1.0),
                srgb8_to_linear(health_color(qhp.min(hp)), 1.0),
            );
        }
        prev = Some((p, hp));
    }

    // --- Mods box -----------------------------------------------------------
    b.text("Mods", w * 0.515, h * 0.70, h * 0.028, Align::Left, white);
    let my = h * 0.775;
    // The results screen shows the speed letter even at 1.0x.
    if (replay.speed - 1.0).abs() < 0.005 {
        b.text("S", w * 0.545, my, h * 0.032, Align::Center, white);
    } else {
        speed_label(&mut b, w * 0.545, my, h * 1.6, replay.speed, white);
    }
    if !replay.mods.is_empty() && replay.mods != "[]" {
        let mods = replay
            .mods
            .trim_matches(['[', ']'])
            .replace(['"', ' '], "")
            .replace("mod_", "")
            .replace(',', "  ")
            .to_uppercase();
        b.text(&mods, w * 0.58, my, h * 0.026, Align::Left, white);
    }

    b.verts
}

/// Draw the combo ring: a polygon whose side count is the stateful tier
/// (see [`RingState`]), whose outline fills clockwise from the top vertex.
/// The fill runs to its new value over ~150 ms after each hit, drains to
/// zero over ~250 ms after a miss, and the shape wobbles briefly when it
/// gains a side — except when it becomes the octagon, matching the game.
fn combo_ring(
    b: &mut HudBuilder,
    cx: f32,
    cy: f32,
    radius: f32,
    stats: &HudStats,
    color: [f32; 4],
) {
    let combo = stats.combo;
    let sides = stats.ring_sides;
    let target = stats.ring_progress;
    let changed_shape = combo > 0 && combo.is_multiple_of(RING_TIER);
    let progress = if sides >= 8 {
        1.0
    } else if combo == 0 && stats.ms_since_miss < RING_DRAIN_MS {
        // Leftover fill drains away after a miss.
        let t = (stats.ms_since_miss / RING_DRAIN_MS).clamp(0.0, 1.0) as f32;
        stats.ring_fill_at_miss * (1.0 - t)
    } else {
        // Fill animates from the pre-hit value; a fresh shape starts empty.
        let prev = if combo == 0 || changed_shape {
            0.0
        } else {
            ((combo - 1) % RING_TIER) as f32 / RING_TIER as f32
        };
        let t = (stats.ms_since_hit / RING_FILL_MS).clamp(0.0, 1.0) as f32;
        prev + (target - prev) * t
    };
    // Brief rotational wobble on a gained side (not when becoming the octagon).
    let wobble = if changed_shape && sides < 8 && stats.ms_since_hit < RING_WOBBLE_MS {
        let p = (stats.ms_since_hit / RING_WOBBLE_MS).clamp(0.0, 1.0) as f32;
        (p * std::f32::consts::PI * 3.0).sin() * (1.0 - p) * 0.12
    } else {
        0.0
    };

    let thick = (radius * 0.13).max(2.0);
    let n = sides as usize;
    // Pointy-top polygon: first vertex straight up; fill runs clockwise.
    let vert = |k: f32| -> [f32; 2] {
        let a = -std::f32::consts::FRAC_PI_2 + wobble + std::f32::consts::TAU * k / (n as f32);
        [cx + radius * a.cos(), cy + radius * a.sin()]
    };
    let lerp2 =
        |a: [f32; 2], c: [f32; 2], t: f32| [a[0] + (c[0] - a[0]) * t, a[1] + (c[1] - a[1]) * t];
    // Extends a segment by half the stroke width at each requested end so
    // adjacent edges join without gaps or spurs at the corners.
    let extend = |a: [f32; 2], c: [f32; 2], head: bool, tail: bool| -> ([f32; 2], [f32; 2]) {
        let (dx, dy) = (c[0] - a[0], c[1] - a[1]);
        let len = (dx * dx + dy * dy).sqrt().max(1e-3);
        let (ux, uy) = (dx / len * thick * 0.5, dy / len * thick * 0.5);
        (
            if head { [a[0] - ux, a[1] - uy] } else { a },
            if tail { [c[0] + ux, c[1] + uy] } else { c },
        )
    };
    let faint = [color[0], color[1], color[2], color[3] * 0.06];
    for k in 0..n {
        let (a, c) = extend(vert(k as f32), vert(k as f32 + 1.0), true, true);
        b.line(a, c, thick, faint);
    }
    // Bright fill: whole edges plus a partial edge, continuous around the
    // rim. The partial end interpolates ALONG THE EDGE (not by angle — an
    // angular point sits on the circumcircle and would jut outside the line).
    let fill_edges = progress.clamp(0.0, 1.0) * n as f32;
    let whole = fill_edges.floor() as usize;
    for k in 0..whole.min(n) {
        let (a, c) = extend(vert(k as f32), vert(k as f32 + 1.0), true, true);
        b.line(a, c, thick, color);
    }
    let part = fill_edges - whole as f32;
    if part > 1e-3 && whole < n {
        let (v0, v1) = (vert(whole as f32), vert(whole as f32 + 1.0));
        let end = lerp2(v0, v1, part);
        let (a, c) = extend(v0, end, true, false);
        b.line(a, c, thick, color);
    }
    // Centre "{combo}x", shrunk when a long number would overflow the ring.
    let txt = format!("{combo}x");
    let mut px = radius * 0.55;
    let max_w = radius * 1.3;
    let w = b.atlas.measure(&txt, px);
    if w > max_w {
        px *= max_w / w;
    }
    b.text(
        &txt,
        cx,
        cy + px * 0.36,
        px,
        Align::Center,
        [1.0, 1.0, 1.0, 1.0],
    );
}
