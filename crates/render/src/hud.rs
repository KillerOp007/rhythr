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
    /// Rasterise the HUD font once into a packed coverage atlas. `custom`
    /// is the game's own font (Nunito, extracted from the user's install);
    /// the bundled DejaVu approximation is the fallback.
    pub fn new(custom: Option<&[u8]>) -> FontAtlas {
        let font = custom
            .and_then(|b| Font::from_bytes(b, FontSettings::default()).ok())
            .unwrap_or_else(|| {
                Font::from_bytes(FONT_BYTES, FontSettings::default()).expect("bundled HUD font")
            });

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
        Self::new(None)
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
/// A second replay rendered as a ghost for comparison: the parsed replay,
/// its resolved hit state, and the overlay colour (sRGB 0..1).
pub struct GhostInput {
    pub replay: Replay,
    pub state: HudState,
    pub color: [f32; 3],
    /// The map as this ghost's player saw it — its own geometry mods
    /// (mirror/hardrock) applied, independent of the main side's.
    pub map: Map,
    /// Grid half-extent of this side's playfield (1.0, or wider under
    /// hardrock); the border follows it.
    pub grid_scale: f32,
}

pub struct HudState {
    outcome: MatchOutcome,
    /// Per-hit detail for the optional error meters, sorted by hit time.
    hit_details: Vec<HitDetail>,
}

/// One registered hit: when, how early/late, and where the cursor sat
/// relative to the note centre (in grid cells; a note spans ±0.5).
#[derive(Debug, Clone, Copy)]
pub struct HitDetail {
    pub hit_ms: f64,
    /// Positive = late, negative = early.
    pub err_ms: f64,
    pub off_x: f32,
    pub off_y: f32,
}

impl HudState {
    pub fn new(map: &Map, replay: &Replay) -> HudState {
        let outcome = match_hits(&map.notes, &replay.frames, DEFAULT_WINDOW_MS);
        // Cursor position at each hit: frames are time-sorted, so a single
        // merged walk resolves every hit's surrounding frame pair.
        let mut hit_details: Vec<HitDetail> = Vec::new();
        let mut hits: Vec<(f64, usize)> = outcome
            .results
            .iter()
            .filter_map(|r| r.hit_ms.filter(|_| r.hit).map(|t| (t, r.note_index)))
            .collect();
        hits.sort_by(|a, b| a.0.total_cmp(&b.0));
        let frames = &replay.frames;
        let mut fi = 0usize;
        for (t, note_index) in hits {
            while fi + 1 < frames.len() && frames[fi + 1].ms <= t {
                fi += 1;
            }
            let (cx, cy) = if fi + 1 < frames.len() && frames[fi + 1].ms > frames[fi].ms {
                let a = &frames[fi];
                let b = &frames[fi + 1];
                let k = (((t - a.ms) / (b.ms - a.ms)).clamp(0.0, 1.0)) as f32;
                (a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k)
            } else {
                frames.get(fi).map(|f| (f.x, f.y)).unwrap_or((0.0, 0.0))
            };
            let note = &map.notes[note_index];
            // Cursor frames live in world space (±1.37 around the origin,
            // +y up); notes are grid coordinates — convert before diffing.
            let (nx, ny) = crate::scene::grid_to_world(note.x, note.y);
            hit_details.push(HitDetail {
                hit_ms: t,
                err_ms: t - note.time_ms as f64,
                off_x: cx - nx,
                off_y: cy - ny,
            });
        }
        HudState {
            outcome,
            hit_details,
        }
    }

    /// Hits registered up to `song_time_ms`, most recent last.
    pub fn hits_until(&self, song_time_ms: f64) -> &[HitDetail] {
        let end = self
            .hit_details
            .partition_point(|d| d.hit_ms <= song_time_ms);
        &self.hit_details[..end]
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

/// Live RP (Rhythia Points) at the current accuracy. The real per-play
/// value is a server-side calculation over the map's star rating (per
/// the Rhythia wiki), which a renderer can't reproduce —
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

/// A movable HUD element's on-screen bounds in frame pixels, as actually
/// drawn — the drag editor's hitboxes come straight from these, so hitbox
/// and pixels can never drift apart.
#[derive(Debug, Clone)]
pub struct HudBox {
    pub key: &'static str,
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

/// Closes out one movable element: applies the user's position override
/// (normalised centre → pixel translate of everything drawn since `start`)
/// and records the resulting bounds.
fn finish_element(
    b: &mut HudBuilder,
    start: usize,
    key: &'static str,
    positions: &std::collections::BTreeMap<String, [f32; 2]>,
    w: f32,
    h: f32,
    boxes: &mut Vec<HudBox>,
) {
    if b.verts.len() == start {
        return;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
    for v in &b.verts[start..] {
        x0 = x0.min(v.pos[0]);
        y0 = y0.min(v.pos[1]);
        x1 = x1.max(v.pos[0]);
        y1 = y1.max(v.pos[1]);
    }
    if let Some(p) = positions.get(key) {
        let (dx, dy) = (p[0] * w - (x0 + x1) * 0.5, p[1] * h - (y0 + y1) * 0.5);
        for v in &mut b.verts[start..] {
            v.pos[0] += dx;
            v.pos[1] += dy;
        }
        x0 += dx;
        x1 += dx;
        y0 += dy;
        y1 += dy;
    }
    boxes.push(HudBox { key, x0, y0, x1, y1 });
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
    state: &HudState,
    stats: &HudStats,
    replay: &Replay,
    map: &Map,
    song_time_ms: f64,
    field: &Playfield,
    miss_marks: &[(f32, f32, f64)],
    width: u32,
    height: u32,
    // Portrait frames (Shorts/TikTok) re-home the stat columns into rows
    // above and below the field — the drag editor can rearrange from there.
    // The caller decides from the OUTPUT frame, not this viewport (a
    // ghost-split half is 8:9 but keeps the landscape layout).
    portrait: bool,
) -> (Vec<HudVertex>, Vec<HudBox>) {
    let hud = &cfg.hud;
    let positions = &cfg.hud.positions;
    let mut boxes: Vec<HudBox> = Vec::new();
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
        let el = b.verts.len();
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
        finish_element(&mut b, el, "combo_text", positions, w, _h, &mut boxes);
    }

    // Column centre just past the box; PanelGap pushes it further out.
    let col_dx = field.half + refd * 0.085 + cfg.panel_gap * refd / 1440.0;
    let left_x = field.cx - col_dx;
    let right_x = field.cx + col_dx;
    let row = refd * 0.132; // vertical stride between stat entries

    // Optional panel background cards behind the stat columns (the
    // portrait rows have no card equivalent).
    if cfg.panel_background_opacity > 0.0 && !portrait {
        let card = srgb8_to_linear(cfg.panel_color, cfg.panel_background_opacity);
        let (cw, ch2) = (refd * 0.16, field.half * 1.9);
        for x in [left_x, right_x] {
            b.rect(x - cw * 0.5, field.cy - ch2 * 0.5, cw, ch2, card);
        }
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

    // The game pins each column's outer slots and spreads the ENABLED
    // entries evenly between them (measured against footage: four entries
    // fill every slot, two sit at both ends, a single one centres on the
    // field). Both columns end level; the left one starts lower to make
    // room for the combo ring's larger footprint.
    let bottom = field.cy - field.half * 0.79 + 3.0 * row;
    let slot_y = move |n: usize, i: usize, top: f32| -> f32 {
        if n <= 1 {
            // A lone entry centres its label+value block on the field.
            field.cy - value_px * 0.4
        } else {
            top + (bottom - top) * i as f32 / (n - 1) as f32
        }
    };
    let top_l = field.cy - field.half * 0.60;
    let top_r = field.cy - field.half * 0.79;
    // PanelAngle fans the entries: level at the field's vertical centre,
    // tilting further the higher/lower an entry sits (the columns lean
    // like billboards in the game's 3D scene; the 1.3 projection factor
    // is measured from footage).
    let fan_tilt = |b: &mut HudBuilder, from: usize, cx: f32, cy: f32| {
        let ang = cfg.panel_angle * (cy - field.cy) / (1.5 * row) * 1.3;
        if ang.abs() < 1e-4 {
            return;
        }
        let (sn, cs) = ang.sin_cos();
        for v in &mut b.verts[from..] {
            let (dx, dy) = (v.pos[0] - cx, v.pos[1] - cy);
            v.pos[0] = cx + dx * cs - dy * sn;
            v.pos[1] = cy + dx * sn + dy * cs;
        }
    };

    // Left group: combo ring, Pauses, Grade, Accuracy — a column beside
    // the field on landscape, a row underneath it on portrait.
    let ln = [hud.combo_ring, hud.pauses, hud.grade, hud.accuracy]
        .iter()
        .filter(|&&e| e)
        .count();
    let row_spread = |i: usize, n: usize| -> f32 {
        w * 0.5 + (i as f32 - (n as f32 - 1.0) * 0.5) * w * 0.22
    };
    let bottom_row_y = field.cy + field.half + refd * 0.135;
    let top_row_y = field.cy - field.half - refd * 0.135;
    let mut li = 0usize;
    let next_left = |i: &mut usize| -> (f32, f32) {
        let out = if portrait {
            (row_spread(*i, ln), bottom_row_y)
        } else {
            (left_x, slot_y(ln, *i, top_l))
        };
        *i += 1;
        out
    };
    if hud.combo_ring {
        let el = b.verts.len();
        let (x, y) = next_left(&mut li);
        combo_ring(
            &mut b,
            x,
            y - refd * 0.016,
            refd * 0.048,
            stats,
            srgb8_to_linear(hud.combo_ring_color, hud.combo_ring_opacity),
        );
        finish_element(&mut b, el, "combo_ring", positions, w, _h, &mut boxes);
    }
    if hud.pauses {
        let el = b.verts.len();
        let (x, y) = next_left(&mut li);
        entry(&mut b, x, y, "PAUSES", "0", value_col);
        if !portrait {
            fan_tilt(&mut b, el, x, y + value_px * 0.6);
        }
        finish_element(&mut b, el, "pauses", positions, w, _h, &mut boxes);
    }
    if hud.grade {
        let el = b.verts.len();
        let (x, y) = next_left(&mut li);
        let g = stats.grade;
        b.text(
            g.label(),
            x,
            y + value_px * 0.9,
            value_px * 1.5,
            Align::Center,
            srgb8_to_linear(g.color(), 1.0),
        );
        if !portrait {
            fan_tilt(&mut b, el, x, y + value_px * 0.6);
        }
        finish_element(&mut b, el, "grade", positions, w, _h, &mut boxes);
    }
    if hud.accuracy {
        let el = b.verts.len();
        let (x, y) = next_left(&mut li);
        // "--" until the first note resolves; then two decimals, trailing
        // zeros stripped ("92.31%", "100%") — as the game formats it.
        let acc = if stats.resolved == 0 {
            "--".to_string()
        } else {
            let s = format!("{:.2}", stats.accuracy_pct);
            format!("{}%", s.trim_end_matches('0').trim_end_matches('.'))
        };
        entry(&mut b, x, y, "ACCURACY", &acc, value_col);
        if !portrait {
            fan_tilt(&mut b, el, x, y + value_px * 0.6);
        }
        finish_element(&mut b, el, "accuracy", positions, w, _h, &mut boxes);
    }

    // Right group: Score, Points, Misses, Notes — column on landscape, a
    // row above the field on portrait.
    let rn = [hud.score, hud.points, hud.misses, hud.notes]
        .iter()
        .filter(|&&e| e)
        .count();
    let mut ri = 0usize;
    let right_slot = |i: &mut usize| -> (f32, f32) {
        let out = if portrait {
            (row_spread(*i, rn), top_row_y)
        } else {
            (right_x, slot_y(rn, *i, top_r))
        };
        *i += 1;
        out
    };
    let mut right_cells: Vec<(&'static str, &'static str, String)> = Vec::new();
    if hud.score {
        right_cells.push(("score", "SCORE", thousands(stats.score)));
    }
    if hud.points {
        // RP (Rhythia Points) — "--" until a note resolves.
        let pts = if stats.resolved == 0 {
            "--".to_string()
        } else {
            format!("{:.0}", stats.points)
        };
        right_cells.push(("points", "POINTS", pts));
    }
    if hud.misses {
        right_cells.push(("misses", "MISSES", stats.misses.to_string()));
    }
    if hud.notes {
        right_cells.push(("notes", "NOTES", format!("{}/{}", stats.hits, stats.resolved)));
    }
    for (key, label, value) in right_cells {
        let el = b.verts.len();
        let (x, y) = right_slot(&mut ri);
        entry(&mut b, x, y, label, &value, value_col);
        if !portrait {
            fan_tilt(&mut b, el, x, y + value_px * 0.6);
        }
        finish_element(&mut b, el, key, positions, w, _h, &mut boxes);
    }

    // Health bar just below the playfield.
    if hud.health_bar {
        let el = b.verts.len();
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
        finish_element(&mut b, el, "health_bar", positions, w, _h, &mut boxes);
    }

    // Song progress bar just above the playfield, with its elapsed/total
    // clock — one movable element (the clock belongs to the bar, not the
    // title; user report 16.07.).
    let ty = field.cy - field.half - refd * 0.053;
    if hud.song_progress_bar {
        let el = b.verts.len();
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
        let time = format!("{} / {}", clock(song_time_ms), clock(dur.max(replay.length_ms())));
        b.text(
            &time,
            field.cx,
            ty + refd * 0.024,
            refd * 0.0149,
            Align::Center,
            label_col,
        );
        finish_element(&mut b, el, "song_progress", positions, w, _h, &mut boxes);
    }

    // Speed notation under the health bar: "S" plus coloured step dots and a
    // +/- sign. Ranked speeds (user-verified): 0.75 S··−, 0.8 S·−, 0.87 S−,
    // 1.0 nothing, 1.15 S+, 1.25 S:+, 1.35 S::+, 1.45 S:::+ (dot columns of
    // two above 1x, single dots below; green→orange→red). Anything else is
    // unranked and shows a slashed circle.
    if hud.speed_label {
        let el = b.verts.len();
        let sy = field.cy + field.half + refd * 0.014 + refd * 0.0088 + refd * 0.020;
        speed_label(&mut b, field.cx, sy, refd, replay.speed, value_col);
        finish_element(&mut b, el, "speed_label", positions, w, _h, &mut boxes);
    }

    // Title header, sitting just above the playfield (as the game).
    if hud.song_info {
        let el = b.verts.len();
        let title = if replay.player_name.is_empty() {
            map.meta.song_name.clone()
        } else {
            format!(
                "Watching {} play {}",
                replay.player_name, map.meta.song_name
            )
        };
        b.text(
            &title,
            field.cx,
            ty,
            refd * 0.0187,
            Align::Center,
            value_col,
        );
        finish_element(&mut b, el, "song_info", positions, w, _h, &mut boxes);
    }

    // Fail vignette — the game's own formula (vignette.fs): a fullscreen
    // radial smoothstep(0.35, 0.85) in red (0.9, 0.05, 0.05), scaled by a
    // strength that grows as health drains. The aspect correction from the
    // shader (p.x *= aspect) is baked into the quad's UVs here.
    if cfg.fail_vignette_opacity > 0.0 {
        let danger = (1.0 - stats.health.clamp(0.0, 1.0)).powi(2);
        let alpha = cfg.fail_vignette_opacity * danger;
        if alpha > 0.002 {
            let aspect = w / _h;
            let color = srgb8_to_linear([230, 13, 13], alpha);
            let (u0, u1) = (0.5 - 0.5 * aspect, 0.5 + 0.5 * aspect);
            let mode = 4.0;
            let v = &mut b.verts;
            let quad = [
                HudVertex::new([0.0, 0.0], [u0, 0.0], color, mode),
                HudVertex::new([w, 0.0], [u1, 0.0], color, mode),
                HudVertex::new([w, _h], [u1, 1.0], color, mode),
                HudVertex::new([0.0, 0.0], [u0, 0.0], color, mode),
                HudVertex::new([w, _h], [u1, 1.0], color, mode),
                HudVertex::new([0.0, _h], [u0, 1.0], color, mode),
            ];
            v.extend_from_slice(&quad);
        }
    }

    // --- Optional renderer extras (not game elements) --------------------
    draw_error_meters(&mut b, cfg, state, song_time_ms, w, _h);

    (b.verts, boxes)
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
    fn ring_drops_one_side_per_miss_and_holds_otherwise() {
        // Footage + user 16.07.2026: heptagon → miss → hexagon → miss →
        // pentagon; the shape only ever moves down on a miss (no time
        // decay) and only the streak formula grows it back.
        let (mut r, t) = ring_after_hits(35);
        assert_eq!(r.sides_at(t), 7);
        let fill = r.on_miss(t + 10.0);
        assert!((fill - 3.0 / 8.0).abs() < 1e-6); // 35 % 8 = 3
        assert_eq!(r.sides_at(t + 20.0), 6);
        r.on_miss(t + 400.0);
        assert_eq!(r.sides_at(t + 410.0), 5);
        // Idle time changes nothing.
        assert_eq!(r.sides_at(t + 60_000.0), 5);
        // A couple of hits (not enough to refill) then another miss: one
        // more side gone.
        r.on_hit(t + 700.0);
        r.on_hit(t + 800.0);
        assert_eq!(r.sides_at(t + 810.0), 5);
        r.on_miss(t + 900.0);
        assert_eq!(r.sides_at(t + 910.0), 4);
        r.on_miss(t + 1000.0);
        assert_eq!(r.sides_at(t + 1010.0), 3);
        r.on_miss(t + 1100.0); // never below the triangle
        assert_eq!(r.sides_at(t + 1110.0), 3);
        // Growth follows the pure combo formula past the floor; the octagon
        // needs a true 40+ streak (must NOT refill at ~30 after a miss).
        let mut now = t + 1200.0;
        for _ in 0..24 {
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
        let atlas = FontAtlas::new(None);
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
    /// Post-miss floor the shape sits on (3..=8). Each miss lowers it by
    /// one side; only the streak formula raises the shape above it again
    /// (user-verified 16.07.2026: no time-based shrinking — the footage's
    /// heptagon → two misses → pentagon is exactly −1 per miss).
    floor: u32,
    /// Current hit streak (== displayed combo).
    streak: u32,
}

impl RingState {
    fn new() -> RingState {
        RingState { floor: 3, streak: 0 }
    }

    fn on_hit(&mut self, _now_ms: f64) {
        self.streak += 1;
    }

    /// Registers a miss; returns the pre-miss fill so the drain animation
    /// can start from it.
    fn on_miss(&mut self, now_ms: f64) -> f32 {
        let fill = self.progress();
        // The shape drops exactly one side per miss, down to the triangle.
        self.floor = self.sides_at(now_ms).saturating_sub(1).max(3);
        self.streak = 0;
        fill
    }

    /// Displayed side count at `now_ms`. Growth always follows the pure
    /// combo formula ("same combo, same progress, up to 40"); the post-miss
    /// floor only holds the shape up — and decays a side per
    /// [`RING_SHRINK_STEP_MS`] while no hit has frozen it.
    fn sides_at(&self, _now_ms: f64) -> u32 {
        (3 + self.streak / RING_TIER).max(self.floor).min(8)
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

/// Which portion of the results screen [`build_results`] emits. A single
/// run renders [`ResultsPart::Full`]; a ghost race shares one map header
/// across the frame and stacks each racer's numbers into a half-width
/// [`ResultsPart::Side`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResultsPart {
    /// Everything — the classic single-player layout.
    Full,
    /// Only the shared map header (title/subtitle/mapper next to the cover).
    Header,
    /// One racer's half: played-by line, grade, statistics, graph, mods.
    Side,
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
    icons_shown: bool,
    part: ResultsPart,
    // Portrait frames stack the blocks under a smaller (always square)
    // cover; fonts and strides reference the short edge so they match the
    // landscape look at the same output width. Decided by the caller from
    // the OUTPUT frame, never from a ghost-split half.
    portrait: bool,
) -> Vec<HudVertex> {
    let mut b = HudBuilder::new(atlas);
    let (w, h) = (width as f32, height as f32);
    let fr = w.min(h);
    let white = srgb8_to_linear([240, 240, 245], 1.0);
    let green = srgb8_to_linear([60, 220, 90], 1.0);
    let line_col = srgb8_to_linear([120, 120, 126], 0.55);

    // --- Title block (right of the cover, drawn by the renderer) ---------
    if part != ResultsPart::Side {
        let tx = if portrait { w * 0.38 } else { w * 0.227 };
        let title_y = if portrait { h * 0.075 } else { h * 0.082 };
        let title_px = fr * 0.037;
        let fit = ((w * 0.95 - tx) / atlas.measure(&map.meta.song_name, title_px)).min(1.0);
        b.text(&map.meta.song_name, tx, title_y, title_px * fit, Align::Left, white);
        // The green line under the song is the DIFFICULTY, not the title:
        // the map's custom name when set, else the standard tier name.
        let diff = if !map.meta.custom_difficulty_name.is_empty() {
            map.meta.custom_difficulty_name.clone()
        } else {
            match map.meta.difficulty {
                1 => "Easy",
                2 => "Medium",
                3 => "Hard",
                4 => "LOGIC?",
                5 => "Tasukete",
                _ => "N/A",
            }
            .to_string()
        };
        b.text(
            &format!("< {diff} >"),
            tx,
            if portrait { h * 0.105 } else { h * 0.117 },
            fr * 0.026,
            Align::Left,
            green,
        );
        if !map.meta.mappers.is_empty() {
            b.text(
                &format!("by {}", map.meta.mappers.join(", ")),
                tx,
                if portrait { h * 0.130 } else { h * 0.152 },
                fr * 0.024,
                Align::Left,
                white,
            );
        }
    }
    if part == ResultsPart::Header {
        return b.verts;
    }

    let played = format!(
        "Played by {} on {}",
        replay.player_name,
        format_ticks(replay.timestamp_ticks)
    );
    let failed = replay.failed();
    let (glabel, gcol) = if failed {
        ("F", [200u8, 40, 45])
    } else {
        (stats.grade.label(), stats.grade.color())
    };
    match part {
        ResultsPart::Full => {
            if portrait {
                // Under the cover block, full width — no clipping.
                b.text(&played, w * 0.05, h * 0.225, fr * 0.026, Align::Left, white);
                b.text(
                    glabel,
                    w * 0.85,
                    h * 0.185,
                    fr * 0.13,
                    Align::Center,
                    srgb8_to_linear(gcol, 1.0),
                );
            } else {
                b.text(&played, w * 0.227, h * 0.34, fr * 0.026, Align::Left, white);
                // Big grade, top right.
                b.text(
                    glabel,
                    w * 0.895,
                    h * 0.26,
                    fr * 0.13,
                    Align::Center,
                    srgb8_to_linear(gcol, 1.0),
                );
            }
        }
        ResultsPart::Side => {
            // Compact strip between the shared header and the statistics.
            b.text(&played, w * 0.038, h * 0.39, h * 0.022, Align::Left, white);
            b.text(
                glabel,
                w * 0.92,
                h * 0.415,
                h * 0.085,
                Align::Center,
                srgb8_to_linear(gcol, 1.0),
            );
        }
        ResultsPart::Header => unreachable!(),
    }

    // --- Statistics with dotted leaders ----------------------------------
    let stats_title_y = if portrait { h * 0.29 } else { h * 0.43 };
    b.text(
        "Statistics",
        w * 0.032,
        stats_title_y,
        fr * 0.028,
        Align::Left,
        white,
    );
    let label_px = fr * 0.0225;
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
    let mut y = if portrait { h * 0.325 } else { h * 0.492 };
    for (label, value) in &rows {
        b.text(label, lx, y, label_px, Align::Left, white);
        b.text(value, rx, y, label_px, Align::Right, white);
        let line_a = lx + atlas.measure(label, label_px) + w * 0.012;
        let line_b = rx - atlas.measure(value, label_px) - w * 0.012;
        if line_b > line_a {
            b.line(
                [line_a, y - label_px * 0.30],
                [line_b, y - label_px * 0.30],
                (fr * 0.0022).max(1.0),
                line_col,
            );
        }
        y += fr * 0.063;
    }

    // --- Health graph ------------------------------------------------------
    b.text(
        "Health Graph",
        w * 0.515,
        stats_title_y,
        fr * 0.028,
        Align::Left,
        white,
    );
    let (gx0, gx1) = (w * 0.53, w * 0.965);
    let (gy0, gy1) = if portrait {
        (h * 0.325, h * 0.325 + fr * 0.145)
    } else {
        (h * 0.50, h * 0.645)
    };
    let end_ms = if failed {
        replay.fail_time_ms as f64
    } else {
        replay.length_ms()
    }
    .max(1.0);
    b.text("00:00", gx0, gy0 - fr * 0.014, fr * 0.02, Align::Left, white);
    b.text(
        &clock(end_ms),
        gx1,
        gy0 - fr * 0.014,
        fr * 0.02,
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
                (fr * 0.0022).max(1.0),
                srgb8_to_linear(health_color(qhp.min(hp)), 1.0),
            );
        }
        prev = Some((p, hp));
    }

    // --- Mods box -----------------------------------------------------------
    let mods_title_y = if portrait { h * 0.52 } else { h * 0.70 };
    let my = if portrait { h * 0.565 } else { h * 0.775 };
    b.text("Mods", w * 0.515, mods_title_y, fr * 0.028, Align::Left, white);
    // The results screen shows the speed letter even at 1.0x.
    if (replay.speed - 1.0).abs() < 0.005 {
        b.text("S", w * 0.545, my, fr * 0.032, Align::Center, white);
    } else {
        speed_label(&mut b, w * 0.545, my, fr * 1.6, replay.speed, white);
    }
    if !icons_shown && !replay.mods.is_empty() && replay.mods != "[]" {
        let mods = replay
            .mods
            .trim_matches(['[', ']'])
            .replace(['"', ' '], "")
            .replace("mod_", "")
            .replace(',', "  ")
            .to_uppercase();
        // At half width the speed notation reaches further into the box.
        let mx = if part == ResultsPart::Side { w * 0.63 } else { w * 0.58 };
        b.text(&mods, mx, my, fr * 0.026, Align::Left, white);
    }

    b.verts
}

/// Where the score card places its cover (x0, y0, size): top-left on
/// landscape cards, centred up top on square/portrait ones. Shared with
/// the renderer, which draws the cover quad itself.
pub fn card_cover_rect(width: u32, height: u32) -> (f32, f32, f32) {
    let (w, h) = (width as f32, height as f32);
    let aspect = w / h;
    if aspect > 1.4 {
        (w * 0.033, h * 0.075, h * 0.35)
    } else if aspect < 0.8 {
        let size = w * 0.42;
        ((w - size) * 0.5, h * 0.045, size)
    } else {
        // Square: small cover top-left, title beside it.
        (w * 0.05, h * 0.05, h * 0.24)
    }
}

/// Builds the shareable score-card overlay (title block, grade and a
/// roomy stat spread) — the Discord-embed companion to a full video, in
/// several aspect ratios: landscape reads left-to-right, square and
/// portrait (Shorts/TikTok) stack everything centred. The cover quad and
/// background are drawn by the renderer.
pub fn build_card(
    atlas: &FontAtlas,
    replay: &Replay,
    map: &Map,
    stats: &HudStats,
    cfg: &crate::config::SkinConfig,
    width: u32,
    height: u32,
) -> Vec<HudVertex> {
    let mut b = HudBuilder::new(atlas);
    let (w, h) = (width as f32, height as f32);
    let ink = srgb8_to_linear(cfg.interface_text_color, 1.0);
    let muted = srgb8_to_linear(cfg.interface_text_color, 0.55);
    let green = srgb8_to_linear([60, 220, 90], 1.0);

    let diff = if !map.meta.custom_difficulty_name.is_empty() {
        map.meta.custom_difficulty_name.clone()
    } else {
        match map.meta.difficulty {
            1 => "Easy",
            2 => "Medium",
            3 => "Hard",
            4 => "LOGIC?",
            5 => "Tasukete",
            _ => "N/A",
        }
        .to_string()
    };
    let failed = replay.failed();
    let (glabel, gcol) = if failed {
        ("F", [200u8, 40, 45])
    } else {
        (stats.grade.label(), stats.grade.color())
    };
    let grade_col = srgb8_to_linear(gcol, 1.0);
    let acc = {
        let s = format!("{:.2}", stats.accuracy_pct);
        format!("{}%", s.trim_end_matches('0').trim_end_matches('.'))
    };
    let mut cells: Vec<(String, String)> = vec![
        ("HITS".into(), stats.hits.to_string()),
        ("MISSES".into(), stats.misses.to_string()),
        ("SPEED".into(), format!("{:.2}x", replay.speed)),
    ];
    if failed {
        cells.push(("FAILED AT".into(), clock(replay.fail_time_ms as f64)));
    } else if replay.points > 0.0 {
        cells.push(("RP".into(), format!("{:.0}", replay.points)));
    }

    if w / h > 1.4 {
        // --- Landscape: cover left, text beside it, stats spread wide ---
        let tx = w * 0.25;
        let title_px = h * 0.073;
        let fit = (w * 0.53 / atlas.measure(&map.meta.song_name, title_px)).min(1.0);
        b.text(&map.meta.song_name, tx, h * 0.155, title_px * fit, Align::Left, ink);
        b.text(&format!("< {diff} >"), tx, h * 0.235, h * 0.045, Align::Left, green);
        if !map.meta.mappers.is_empty() {
            b.text(
                &format!("by {}", map.meta.mappers.join(", ")),
                tx,
                h * 0.30,
                h * 0.038,
                Align::Left,
                muted,
            );
        }
        b.text(&replay.player_name, tx, h * 0.40, h * 0.055, Align::Left, ink);
        b.text(glabel, w * 0.90, h * 0.33, h * 0.27, Align::Center, grade_col);

        b.text("ACCURACY", w * 0.06, h * 0.60, h * 0.034, Align::Left, muted);
        b.text(&acc, w * 0.06, h * 0.725, h * 0.115, Align::Left, ink);
        b.text("SCORE", w * 0.52, h * 0.60, h * 0.034, Align::Left, muted);
        b.text(&thousands(stats.score), w * 0.52, h * 0.725, h * 0.115, Align::Left, ink);

        let (left, right) = (w * 0.06, w * 0.80);
        for (i, (label, value)) in cells.iter().enumerate() {
            let x = left + (right - left) * i as f32 / (cells.len() - 1).max(1) as f32;
            b.text(label, x, h * 0.845, h * 0.030, Align::Left, muted);
            b.text(value, x, h * 0.935, h * 0.062, Align::Left, ink);
        }
        b.text(
            "rhythr",
            w * 0.985 - atlas.measure("rhythr", h * 0.028),
            h * 0.09,
            h * 0.028,
            Align::Left,
            muted,
        );
    } else if w / h >= 0.8 {
        // --- Square: landscape-style head, stats centred below ----------
        let (cx0, _, csize) = card_cover_rect(width, height);
        let tx = cx0 + csize + w * 0.05;
        let title_px = w * 0.045;
        let fit = ((w * 0.95 - tx) / atlas.measure(&map.meta.song_name, title_px)).min(1.0);
        b.text(&map.meta.song_name, tx, h * 0.115, title_px * fit, Align::Left, ink);
        b.text(&format!("< {diff} >"), tx, h * 0.175, w * 0.032, Align::Left, green);
        if !map.meta.mappers.is_empty() {
            b.text(
                &format!("by {}", map.meta.mappers.join(", ")),
                tx,
                h * 0.225,
                w * 0.027,
                Align::Left,
                muted,
            );
        }
        b.text(&replay.player_name, tx, h * 0.29, w * 0.038, Align::Left, ink);

        b.text(glabel, w * 0.5, h * 0.545, w * 0.20, Align::Center, grade_col);
        b.text("ACCURACY", w * 0.28, h * 0.655, w * 0.026, Align::Center, muted);
        b.text("SCORE", w * 0.72, h * 0.655, w * 0.026, Align::Center, muted);
        b.text(&acc, w * 0.28, h * 0.735, w * 0.062, Align::Center, ink);
        b.text(&thousands(stats.score), w * 0.72, h * 0.735, w * 0.062, Align::Center, ink);
        for (i, (label, value)) in cells.iter().enumerate() {
            let x = w * (0.5 + (i as f32 - (cells.len() - 1) as f32 * 0.5) * 0.26);
            b.text(label, x, h * 0.845, w * 0.022, Align::Center, muted);
            b.text(value, x, h * 0.905, w * 0.044, Align::Center, ink);
        }
        b.text(
            "rhythr",
            w * 0.97 - atlas.measure("rhythr", w * 0.024),
            h * 0.97,
            w * 0.024,
            Align::Left,
            muted,
        );
    } else {
        // --- Portrait (Shorts/TikTok): everything stacked and centred ---
        let portrait = true;
        let (_, cy0, csize) = card_cover_rect(width, height);
        let cx = w * 0.5;
        // Text sizes hang off the WIDTH here so both 1:1 and 9:16 read well.
        let mut y = cy0 + csize + w * 0.10;
        let title_px = w * 0.062;
        let fit = (w * 0.9 / atlas.measure(&map.meta.song_name, title_px)).min(1.0);
        b.text(&map.meta.song_name, cx, y, title_px * fit, Align::Center, ink);
        y += w * 0.055;
        b.text(&format!("< {diff} >"), cx, y, w * 0.036, Align::Center, green);
        if !map.meta.mappers.is_empty() {
            y += w * 0.045;
            b.text(
                &format!("by {}", map.meta.mappers.join(", ")),
                cx,
                y,
                w * 0.030,
                Align::Center,
                muted,
            );
        }
        y += w * 0.075;
        b.text(&replay.player_name, cx, y, w * 0.045, Align::Center, ink);

        // Grade centred, then the headline pair side by side.
        y += if portrait { w * 0.30 } else { w * 0.21 };
        b.text(glabel, cx, y, if portrait { w * 0.22 } else { w * 0.17 }, Align::Center, grade_col);
        y += if portrait { w * 0.17 } else { w * 0.12 };
        b.text("ACCURACY", w * 0.28, y, w * 0.026, Align::Center, muted);
        b.text("SCORE", w * 0.72, y, w * 0.026, Align::Center, muted);
        y += w * 0.075;
        b.text(&acc, w * 0.28, y, w * 0.068, Align::Center, ink);
        b.text(&thousands(stats.score), w * 0.72, y, w * 0.068, Align::Center, ink);

        // Secondary cells as a centred row (or 2x2 on portrait).
        y += w * 0.115;
        if portrait && cells.len() == 4 {
            for (i, (label, value)) in cells.iter().enumerate() {
                let x = if i % 2 == 0 { w * 0.28 } else { w * 0.72 };
                let yy = y + (i / 2) as f32 * w * 0.155;
                b.text(label, x, yy, w * 0.024, Align::Center, muted);
                b.text(value, x, yy + w * 0.062, w * 0.048, Align::Center, ink);
            }
        } else {
            for (i, (label, value)) in cells.iter().enumerate() {
                let x = w * (0.5 + (i as f32 - (cells.len() - 1) as f32 * 0.5) * 0.28);
                b.text(label, x, y, w * 0.024, Align::Center, muted);
                b.text(value, x, y + w * 0.062, w * 0.048, Align::Center, ink);
            }
        }
        b.text(
            "rhythr",
            w * 0.97 - atlas.measure("rhythr", w * 0.024),
            h - w * 0.03,
            w * 0.024,
            Align::Left,
            muted,
        );
    }

    b.verts
}

/// How long a hit stays visible in the meters.
const METER_FADE_MS: f64 = 3000.0;
/// Soft pop-in duration for a new tick/dot.
const METER_POP_MS: f64 = 130.0;

/// Envelope for a hit of the given age: quick eased rise, gentle quadratic
/// fall — the difference between ticks snapping in and gliding in.
fn meter_envelope(age_ms: f64) -> f32 {
    let rise = (age_ms / METER_POP_MS).clamp(0.0, 1.0) as f32;
    let rise = rise * rise * (3.0 - 2.0 * rise); // smoothstep
    let fall = (1.0 - age_ms / METER_FADE_MS).clamp(0.0, 1.0) as f32;
    rise * fall * fall
}

/// Colour ramp for an error fraction 0..1: green → yellow → red (sRGB in,
/// linear out via the usual HUD conversion).
fn meter_color(frac: f32, alpha: f32) -> [f32; 4] {
    let f = frac.clamp(0.0, 1.0);
    let (r, g, b) = if f < 0.5 {
        let k = f / 0.5;
        (80.0 + (240.0 - 80.0) * k, 230.0 - 20.0 * k, 120.0 - 40.0 * k)
    } else {
        let k = (f - 0.5) / 0.5;
        (240.0, 210.0 - 130.0 * k, 80.0)
    };
    srgb8_to_linear([r as u8, g as u8, b as u8], alpha)
}

/// The optional danser-style meters: a timing bar (early ← → late over the
/// ±80 ms hit window) and an aim scatter (cursor offset from the note
/// centre, a note spanning ±0.5 grid cells). Both fade each hit over
/// [`METER_FADE_MS`] and follow user-set position/scale/alpha.
fn draw_error_meters(
    b: &mut HudBuilder,
    cfg: &crate::config::SkinConfig,
    state: &HudState,
    t: f64,
    w: f32,
    h: f32,
) {
    let em = cfg.hud.error_meter;
    let am = cfg.hud.aim_meter;
    if !em.enabled && !am.enabled {
        return;
    }
    let hits = state.hits_until(t);
    let recent = || {
        hits.iter()
            .rev()
            .take_while(move |d| t - d.hit_ms < METER_FADE_MS)
    };

    // Accent in the side's cursor colour so a ghost-split's bars read as
    // belonging to their player.
    let accent = srgb8_to_linear(
        [
            (cfg.cursor_color[0] * 255.0) as u8,
            (cfg.cursor_color[1] * 255.0) as u8,
            (cfg.cursor_color[2] * 255.0) as u8,
        ],
        0.9,
    );
    // Meter chrome (tracks, frames, crosshairs) must survive any skin:
    // light grey vanishes on light backgrounds, so flip to near-black at
    // strong opacity when the background is bright.
    let bg = cfg.background_color;
    let light_bg = 0.299 * bg[0] + 0.587 * bg[1] + 0.114 * bg[2] > 0.55;
    let chrome: [u8; 3] = if light_bg { [12, 13, 16] } else { [225, 228, 235] };
    let chrome_boost = if light_bg { 4.0f32 } else { 1.0 };
    if em.enabled {
        // One-sided: hits can only ever be late (the hitbox arms at the
        // note's time — verified across 6k reference hits, min error 0.0),
        // so the bar runs 0 → +80 ms with the anchor on the left.
        let (cx, cy) = (em.x * w, em.y * h);
        let halfw = h * 0.16 * em.scale;
        let (x0, bar_w) = (cx - halfw, halfw * 2.0);
        let bar_h = (h * 0.0028 * em.scale).max(1.5);
        let tick_h = (h * 0.016 * em.scale).max(6.0);
        let tick_w = (h * 0.0055 * em.scale).max(4.0);
        b.rect(
            x0,
            cy - bar_h * 0.5,
            bar_w,
            bar_h,
            srgb8_to_linear(chrome, (0.14 * chrome_boost).min(0.55) * em.alpha),
        );
        // "0 ms" anchor in the accent colour.
        b.rect(
            x0 - 1.0,
            cy - tick_h * 0.75,
            2.0,
            tick_h * 1.5,
            [accent[0], accent[1], accent[2], accent[3] * em.alpha],
        );
        for d in recent() {
            let age = t - d.hit_ms;
            let env = meter_envelope(age);
            let frac = (d.err_ms / rhythia_sim::hitreg::DEFAULT_WINDOW_MS) as f32;
            let x = x0 + frac.clamp(0.0, 1.0) * bar_w;
            let col = meter_color(frac, env * em.alpha);
            // Short and chunky: easier to read than the taller hairlines.
            let th = tick_h * 0.5 * (0.55 + 0.45 * (age / METER_POP_MS).clamp(0.0, 1.0) as f32);
            b.rect(x - tick_w * 0.5, cy - th * 0.5, tick_w, th, col);
        }
        // Rolling average marker (last 20 hits), gliding to each new value
        // instead of snapping when a hit lands.
        let window_avg = |skip: usize| -> Option<f32> {
            let vals: Vec<f64> = hits.iter().rev().skip(skip).take(20).map(|d| d.err_ms).collect();
            (!vals.is_empty()).then(|| {
                (vals.iter().sum::<f64>() / vals.len() as f64
                    / rhythia_sim::hitreg::DEFAULT_WINDOW_MS) as f32
            })
        };
        if let Some(now_avg) = window_avg(0) {
            let prev_avg = window_avg(1).unwrap_or(now_avg);
            let since_last = hits.last().map(|d| t - d.hit_ms).unwrap_or(f64::MAX);
            let k = (since_last / 250.0).clamp(0.0, 1.0) as f32;
            let k = k * k * (3.0 - 2.0 * k);
            let avg = prev_avg + (now_avg - prev_avg) * k;
            let x = x0 + avg.clamp(0.0, 1.0) * bar_w;
            b.rect(
                x - 1.5,
                cy + tick_h * 0.75,
                3.0,
                (h * 0.006 * em.scale).max(3.0),
                [accent[0], accent[1], accent[2], accent[3] * em.alpha],
            );
        }
    }

    if am.enabled {
        let (cx, cy) = (am.x * w, am.y * h);
        let half = h * 0.065 * am.scale;
        // Bright enough to actually see at full opacity.
        let line = srgb8_to_linear(chrome, (0.28 * chrome_boost).min(0.8) * am.alpha);
        let t_px = (h * 0.0042 * am.scale).max(3.5);
        // Square frame (the note's shape) + crosshair.
        b.rect(cx - half, cy - half, half * 2.0, t_px, line);
        b.rect(cx - half, cy + half - t_px, half * 2.0, t_px, line);
        b.rect(cx - half, cy - half, t_px, half * 2.0, line);
        b.rect(cx + half - t_px, cy - half, t_px, half * 2.0, line);
        let cross = (t_px * 0.7).max(1.5);
        b.rect(cx - half, cy - cross * 0.5, half * 2.0, cross, line);
        b.rect(cx - cross * 0.5, cy - half, cross, half * 2.0, line);
        // A hit dead-centre lands on the crosshair; the frame edge is the
        // note's edge (±0.5 cells) plus a little margin.
        const RANGE: f32 = 0.6;
        for d in recent() {
            let age = t - d.hit_ms;
            let env = meter_envelope(age);
            let ox = (d.off_x / RANGE).clamp(-1.0, 1.0) * half;
            // World +y is up; screen +y is down.
            let oy = (-d.off_y / RANGE).clamp(-1.0, 1.0) * half;
            let r = (d.off_x * d.off_x + d.off_y * d.off_y).sqrt() / 0.5;
            let dot = (h * 0.004 * am.scale).max(2.5)
                * (0.6 + 0.4 * (age / METER_POP_MS).clamp(0.0, 1.0) as f32);
            let col = meter_color(r, env * am.alpha);
            b.rect(cx + ox - dot * 0.5, cy + oy - dot * 0.5, dot, dot, col);
        }
    }
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
