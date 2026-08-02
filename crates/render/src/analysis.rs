//! Replay analytics for the Analyze mode: movement, timing, error
//! forensics and integrity signals — all derived from the `.rhr` frame
//! stream and the (geometry-modded) map, no game code involved.
//!
//! Units: distances in grid cells (a note spans ±0.5 cells,
//! `GRID_SPACING` is 1), speeds in cells per WALL-CLOCK second (frame
//! times are song time; a speed mod compresses wall time, and the hand
//! moves in wall time), times in song-time ms to match the scrubber.
//!
//! Numerical care: every aggregate must stay finite — a NaN would break
//! JSON serialization of the whole payload. Guard all divisions.

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_sim::hitreg::{hit_window_ms, match_hits};
use rhythia_sim::integrity;
use serde::Serialize;

/// A frame gap larger than this (song ms) is a pause or a recording gap —
/// movement across it must not count as cursor motion.
const PAUSE_GAP_MS: f64 = 500.0;
/// The cursor counts as "moving" above this speed (cells/s).
const MOVING_SPEED: f64 = 0.5;
/// Series (speed, rolling UR) are downsampled to at most this many points.
const MAX_SERIES_POINTS: usize = 2000;

#[derive(Serialize, Clone)]
pub struct Analysis {
    pub meta: Meta,
    pub frames: FrameArrays,
    pub speed_series: Series,
    pub cursor: CursorStats,
    pub overshoot: Overshoot,
    pub direction_bias: DirectionBias,
    pub snap_flow: SnapFlow,
    pub jitter: Jitter,
    pub timing: Timing,
    pub rolling_ur: Series,
    pub notes: Vec<NoteAnalysis>,
    pub misses: MissSummary,
    pub sections: Vec<Section>,
    pub heatmap: Heatmap,
    pub frame_deltas: FrameDeltas,
    pub signals: Vec<Signal>,
    /// Overall verdict for the integrity panel: "clean", "notice" or "warn"
    /// — the strongest severity among the signals (info never escalates).
    pub verdict: String,
}

#[derive(Serialize, Clone)]
pub struct Meta {
    pub frame_count: usize,
    pub first_ms: f64,
    pub last_ms: f64,
    pub speed: f32,
    pub hits: u32,
    pub misses: u32,
}

/// Compact parallel arrays for the overlay path (world/cell coordinates).
#[derive(Serialize, Clone)]
pub struct FrameArrays {
    pub t: Vec<f32>,
    pub x: Vec<f32>,
    pub y: Vec<f32>,
}

#[derive(Serialize, Clone, Default)]
pub struct Series {
    pub t: Vec<f32>,
    pub v: Vec<f32>,
}

#[derive(Serialize, Clone, Default)]
pub struct Extremum {
    pub v: f64,
    pub t: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct CursorStats {
    pub avg_speed: f64,
    pub p95_speed: f64,
    pub max_speed: Extremum,
    pub max_accel: Extremum,
    pub total_path_cells: f64,
    pub optimal_path_cells: f64,
    /// optimal / actual in percent (100 = shortest possible route).
    pub efficiency_pct: f64,
    pub moving_pct: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct Overshoot {
    /// Share of qualifying approaches that overshot, in percent.
    pub rate_pct: f64,
    pub avg_cells: f64,
    pub worst: Option<Extremum>,
    pub samples: u32,
}

#[derive(Serialize, Clone, Default)]
pub struct DirectionBias {
    /// Mean hit offset from the note centre, world axes (+x right, +y up).
    pub dx: f64,
    pub dy: f64,
    pub magnitude: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct SnapFlow {
    pub snap_pct: f64,
    pub flow_pct: f64,
    pub samples: u32,
}

#[derive(Serialize, Clone, Default)]
pub struct Jitter {
    /// RMS deviation from the smoothed path while moving, in cells.
    pub rms_cells: f64,
    /// Share of 500 ms moving windows with near-zero micro-jitter.
    pub smooth_windows_pct: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct Timing {
    /// Unstable rate: 10 × the standard deviation of hit errors (ms).
    pub ur: f64,
    pub mean_err_ms: f64,
    pub median_err_ms: f64,
    /// Histogram of hit errors, `hist_start_ms + i*hist_bin_ms` per bucket.
    pub hist_bin_ms: f64,
    pub hist_start_ms: f64,
    pub hist: Vec<u32>,
    /// Positive = drifting late over the run (fatigue), ms per minute.
    pub drift_ms_per_min: f64,
    pub first_half: HalfStats,
    pub second_half: HalfStats,
}

#[derive(Serialize, Clone, Default)]
pub struct HalfStats {
    pub acc_pct: f64,
    pub ur: f64,
    pub avg_speed: f64,
}

#[derive(Serialize, Clone)]
pub struct NoteAnalysis {
    /// Index into the map's note list.
    pub i: u32,
    pub t: f64,
    pub gx: f32,
    pub gy: f32,
    pub hit: bool,
    pub hit_ms: Option<f64>,
    pub err_ms: Option<f64>,
    /// Cursor offset from the note centre at hit time (world axes, cells).
    pub off_x: Option<f32>,
    pub off_y: Option<f32>,
    pub dist: Option<f32>,
    /// Misses: closest the cursor came within ±250 ms of the note time.
    pub near_dist: Option<f32>,
    /// Mean cursor speed in the 150 ms before the note (cells/s).
    pub approach_v: f32,
}

#[derive(Serialize, Clone, Default)]
pub struct MissSummary {
    pub count: u32,
    pub avg_near_dist: f64,
    /// Near misses: cursor got within 0.65 cells but the hit didn't land.
    pub barely_pct: f64,
    /// Lost: cursor never came within 1.2 cells.
    pub lost_pct: f64,
    pub on_fast_jumps: u32,
    pub on_streams: u32,
    pub other: u32,
}

#[derive(Serialize, Clone)]
pub struct Section {
    pub start_ms: f64,
    pub end_ms: f64,
    pub acc_pct: f64,
    pub ur: f64,
    pub misses: u32,
    pub notes: u32,
    pub avg_speed: f64,
}

#[derive(Serialize, Clone, Default)]
pub struct Heatmap {
    pub size: u32,
    /// Half-extent in cells; a bucket spans `2*extent/size` cells.
    pub extent: f32,
    /// Row-major dwell-time weights, normalized to 0..=255.
    pub counts: Vec<u8>,
}

#[derive(Serialize, Clone, Default)]
pub struct FrameDeltas {
    pub avg_ms: f64,
    pub median_ms: f64,
    /// 1 ms buckets 0..40 (last bucket collects everything above).
    pub hist: Vec<u32>,
}

#[derive(Serialize, Clone)]
pub struct Signal {
    pub id: String,
    /// "info" | "notice" | "warn" — info is context, never suspicion.
    pub severity: String,
    pub title: String,
    pub detail: String,
    /// Up to a handful of song times to jump to.
    pub times: Vec<f64>,
}

// ------------------------------------------------------------------ core

struct Kinematics {
    /// Per segment i (between frame i and i+1): wall-clock dt (s),
    /// distance (cells), speed (cells/s); pauses get speed 0 and dt 0.
    seg_dt: Vec<f64>,
    seg_dist: Vec<f64>,
    seg_v: Vec<f64>,
    /// Smoothed per-frame speed samples aligned to frame times.
    smooth_t: Vec<f64>,
    smooth_v: Vec<f64>,
}

fn kinematics(replay: &Replay) -> Kinematics {
    let f = &replay.frames;
    let speed = if replay.speed > 0.0 { replay.speed as f64 } else { 1.0 };
    let n = f.len().saturating_sub(1);
    let mut seg_dt = Vec::with_capacity(n);
    let mut seg_dist = Vec::with_capacity(n);
    let mut seg_v = Vec::with_capacity(n);
    for w in f.windows(2) {
        let dt_song = w[1].ms - w[0].ms;
        let dx = (w[1].x - w[0].x) as f64;
        let dy = (w[1].y - w[0].y) as f64;
        let dist = (dx * dx + dy * dy).sqrt();
        if dt_song <= 0.0 || dt_song > PAUSE_GAP_MS {
            seg_dt.push(0.0);
            seg_dist.push(0.0);
            seg_v.push(0.0);
        } else {
            let dt_wall = dt_song / speed / 1000.0;
            seg_dt.push(dt_wall);
            seg_dist.push(dist);
            seg_v.push(dist / dt_wall);
        }
    }
    // Light smoothing (window of 3 segments) for series and extrema — raw
    // per-frame speed is quantization noise.
    let mut smooth_t = Vec::with_capacity(n);
    let mut smooth_v = Vec::with_capacity(n);
    for i in 0..n {
        let lo = i.saturating_sub(1);
        let hi = (i + 2).min(n);
        let (mut d, mut t) = (0.0, 0.0);
        for j in lo..hi {
            d += seg_dist[j];
            t += seg_dt[j];
        }
        smooth_t.push(f[i + 1].ms);
        smooth_v.push(if t > 0.0 { d / t } else { 0.0 });
    }
    Kinematics { seg_dt, seg_dist, seg_v, smooth_t, smooth_v }
}

fn cursor_at(frames: &[rhythia_formats::rhr::Frame], t: f64) -> (f32, f32) {
    if frames.is_empty() {
        return (0.0, 0.0);
    }
    let i = frames.partition_point(|f| f.ms <= t);
    if i == 0 {
        return (frames[0].x, frames[0].y);
    }
    if i >= frames.len() {
        let f = &frames[frames.len() - 1];
        return (f.x, f.y);
    }
    let (a, b) = (&frames[i - 1], &frames[i]);
    if b.ms <= a.ms {
        return (a.x, a.y);
    }
    let k = ((t - a.ms) / (b.ms - a.ms)).clamp(0.0, 1.0) as f32;
    (a.x + (b.x - a.x) * k, a.y + (b.y - a.y) * k)
}

fn finite(v: f64) -> f64 {
    if v.is_finite() { v } else { 0.0 }
}

fn downsample(t: &[f64], v: &[f64]) -> Series {
    let n = t.len();
    if n == 0 {
        return Series::default();
    }
    let step = n.div_ceil(MAX_SERIES_POINTS).max(1);
    let mut out = Series::default();
    let mut i = 0;
    while i < n {
        let hi = (i + step).min(n);
        // Keep the bucket maximum — peaks are what the eye looks for.
        let (mut bt, mut bv) = (t[i], v[i]);
        for j in i..hi {
            if v[j] > bv {
                bv = v[j];
                bt = t[j];
            }
        }
        out.t.push(bt as f32);
        out.v.push(finite(bv) as f32);
        i = hi;
    }
    out
}

fn mean(v: &[f64]) -> f64 {
    if v.is_empty() { 0.0 } else { v.iter().sum::<f64>() / v.len() as f64 }
}

fn std_dev(v: &[f64]) -> f64 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = mean(v);
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f64>() / v.len() as f64).sqrt()
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

/// Full analysis of one replay against ITS map (geometry mods applied —
/// pass the map from `mods::map_for_replay`).
pub fn analyze(map: &Map, replay: &Replay) -> Analysis {
    let f = &replay.frames;
    let speed = if replay.speed > 0.0 { replay.speed as f64 } else { 1.0 };
    let kin = kinematics(replay);

    // ---- judgement base (same pipeline as the HUD meters).
    // Attempted window: a practice run starts at start_from_ms and a
    // failed run ends at fail_time_ms — notes outside that span were
    // never played and must not count as misses (same rule the
    // integrity checker pins empirically).
    let attempt_lo = if replay.start_from_ms > 0 {
        f64::from(replay.start_from_ms)
    } else {
        f64::NEG_INFINITY
    };
    let window = hit_window_ms(replay);
    let attempt_hi = if replay.fail_time_ms >= 0 {
        f64::from(replay.fail_time_ms) + window
    } else {
        f64::INFINITY
    };
    let outcome = match_hits(&map.notes, f, window);
    let mut notes: Vec<NoteAnalysis> = Vec::with_capacity(map.notes.len());
    for (i, note) in map.notes.iter().enumerate() {
        if (note.time_ms as f64) < attempt_lo || (note.time_ms as f64) > attempt_hi {
            continue;
        }
        let r = &outcome.results[i];
        let (nx, ny) = crate::scene::grid_to_world(note.x, note.y);
        let t = note.time_ms as f64;
        // Mean speed in the 150 ms approach before the note.
        let lo = kin.smooth_t.partition_point(|&st| st < t - 150.0);
        let hi = kin.smooth_t.partition_point(|&st| st <= t);
        let approach_v = if hi > lo {
            (kin.smooth_v[lo..hi].iter().sum::<f64>() / (hi - lo) as f64) as f32
        } else {
            0.0
        };
        let mut na = NoteAnalysis {
            i: i as u32,
            t,
            gx: note.x,
            gy: note.y,
            hit: r.hit,
            hit_ms: r.hit_ms,
            err_ms: r.hit_ms.map(|h| h - t),
            off_x: None,
            off_y: None,
            dist: None,
            near_dist: None,
            approach_v,
        };
        if let Some(h) = r.hit_ms {
            let (cx, cy) = cursor_at(f, h);
            let (ox, oy) = (cx - nx, cy - ny);
            na.off_x = Some(ox);
            na.off_y = Some(oy);
            na.dist = Some((ox * ox + oy * oy).sqrt());
        } else {
            // Closest approach within ±250 ms — sampled at frame times.
            let mut best = f32::MAX;
            let lo = f.partition_point(|fr| fr.ms < t - 250.0);
            for fr in &f[lo..] {
                if fr.ms > t + 250.0 {
                    break;
                }
                let (dx, dy) = (fr.x - nx, fr.y - ny);
                best = best.min((dx * dx + dy * dy).sqrt());
            }
            if best < f32::MAX {
                na.near_dist = Some(best);
            }
        }
        notes.push(na);
    }

    // ---- cursor stats
    let total_path: f64 = kin.seg_dist.iter().sum();
    let attempted_notes: Vec<&rhythia_formats::map::Note> = map
        .notes
        .iter()
        .filter(|n| {
            let t = n.time_ms as f64;
            t >= attempt_lo && t <= attempt_hi
        })
        .collect();
    let optimal_path: f64 = attempted_notes
        .windows(2)
        .map(|w| {
            let (ax, ay) = crate::scene::grid_to_world(w[0].x, w[0].y);
            let (bx, by) = crate::scene::grid_to_world(w[1].x, w[1].y);
            (((bx - ax) as f64).powi(2) + ((by - ay) as f64).powi(2)).sqrt()
        })
        .sum();
    let wall_time: f64 = kin.seg_dt.iter().sum();
    let moving_time: f64 = kin
        .seg_dt
        .iter()
        .zip(&kin.seg_v)
        .filter(|(_, v)| **v > MOVING_SPEED)
        .map(|(dt, _)| *dt)
        .sum();
    let mut sorted_v: Vec<f64> = kin.smooth_v.iter().copied().filter(|v| *v > 0.0).collect();
    sorted_v.sort_by(|a, b| a.total_cmp(b));
    let (mut max_v, mut max_vt) = (0.0f64, 0.0f64);
    for (i, &v) in kin.smooth_v.iter().enumerate() {
        if v > max_v {
            max_v = v;
            max_vt = kin.smooth_t[i];
        }
    }
    // Acceleration from consecutive smoothed samples.
    let (mut max_a, mut max_at) = (0.0f64, 0.0f64);
    let mut accel_events: Vec<f64> = Vec::new();
    for i in 1..kin.smooth_v.len() {
        let dt = kin.seg_dt[i];
        if dt <= 0.0 {
            continue;
        }
        let a = ((kin.smooth_v[i] - kin.smooth_v[i - 1]) / dt).abs();
        if a > max_a {
            max_a = a;
            max_at = kin.smooth_t[i];
        }
        if a > 4000.0 {
            accel_events.push(kin.smooth_t[i]);
        }
    }
    let cursor = CursorStats {
        avg_speed: finite(if wall_time > 0.0 { total_path / wall_time } else { 0.0 }),
        p95_speed: finite(percentile(&sorted_v, 0.95)),
        max_speed: Extremum { v: finite(max_v), t: max_vt },
        max_accel: Extremum { v: finite(max_a), t: max_at },
        total_path_cells: finite(total_path),
        optimal_path_cells: finite(optimal_path),
        efficiency_pct: finite(if total_path > 0.0 {
            (optimal_path / total_path * 100.0).min(100.0)
        } else {
            0.0
        }),
        moving_pct: finite(if wall_time > 0.0 { moving_time / wall_time * 100.0 } else { 0.0 }),
    };

    // ---- overshoot: after a hit at the end of a real approach, does the
    // cursor keep sliding along its approach direction before correcting?
    let mut over_n = 0u32;
    let mut over_sum = 0.0;
    let mut over_worst: Option<Extremum> = None;
    let mut over_samples = 0u32;
    for na in &notes {
        let (Some(h), Some(_)) = (na.hit_ms, na.dist) else { continue };
        let (p0x, p0y) = cursor_at(f, h - 60.0);
        let (p1x, p1y) = cursor_at(f, h);
        let (dirx, diry) = ((p1x - p0x) as f64, (p1y - p0y) as f64);
        let len = (dirx * dirx + diry * diry).sqrt();
        if len < 0.15 {
            continue; // not a travelling approach
        }
        over_samples += 1;
        let (ux, uy) = (dirx / len, diry / len);
        let mut worst = 0.0f64;
        for dt in [40.0, 80.0, 120.0] {
            let (qx, qy) = cursor_at(f, h + dt);
            let proj = ((qx - p1x) as f64) * ux + ((qy - p1y) as f64) * uy;
            worst = worst.max(proj);
        }
        if worst > 0.15 {
            over_n += 1;
            over_sum += worst;
            if over_worst.as_ref().is_none_or(|w| worst > w.v) {
                over_worst = Some(Extremum { v: finite(worst), t: h });
            }
        }
    }
    let overshoot = Overshoot {
        rate_pct: finite(if over_samples > 0 {
            over_n as f64 / over_samples as f64 * 100.0
        } else {
            0.0
        }),
        avg_cells: finite(if over_n > 0 { over_sum / over_n as f64 } else { 0.0 }),
        worst: over_worst,
        samples: over_samples,
    };

    // ---- direction bias
    let offs: Vec<(f64, f64)> = notes
        .iter()
        .filter_map(|n| Some((n.off_x? as f64, n.off_y? as f64)))
        .collect();
    let bias = if offs.is_empty() {
        DirectionBias::default()
    } else {
        let dx = offs.iter().map(|o| o.0).sum::<f64>() / offs.len() as f64;
        let dy = offs.iter().map(|o| o.1).sum::<f64>() / offs.len() as f64;
        DirectionBias { dx: finite(dx), dy: finite(dy), magnitude: finite((dx * dx + dy * dy).sqrt()) }
    };

    // ---- snap vs flow between consecutive hit notes
    let (mut snap, mut flow) = (0u32, 0u32);
    for w in notes.windows(2) {
        let (a, b) = (&w[0], &w[1]);
        if !(a.hit && b.hit) {
            continue;
        }
        let dt = b.t - a.t;
        let (ax, ay) = crate::scene::grid_to_world(a.gx, a.gy);
        let (bx, by) = crate::scene::grid_to_world(b.gx, b.gy);
        let dist = (((bx - ax) as f64).powi(2) + ((by - ay) as f64).powi(2)).sqrt();
        if !(50.0..=1200.0).contains(&dt) || dist < 0.5 {
            continue;
        }
        // Speed profile within the interval.
        let lo = kin.smooth_t.partition_point(|&st| st < a.t);
        let hi = kin.smooth_t.partition_point(|&st| st <= b.t);
        let seg = &kin.smooth_v[lo..hi];
        if seg.len() < 2 {
            continue;
        }
        let peak = seg.iter().copied().fold(0.0f64, f64::max);
        if peak <= 0.0 {
            continue;
        }
        let avg = seg.iter().sum::<f64>() / seg.len() as f64;
        let rest = seg.iter().filter(|v| **v < peak * 0.2).count() as f64;
        let cnt = seg.len() as f64;
        if peak / avg.max(1e-6) >= 2.2 && rest / cnt >= 0.25 {
            snap += 1;
        } else {
            flow += 1;
        }
    }
    let sf_total = (snap + flow).max(1);
    let snap_flow = SnapFlow {
        snap_pct: finite(snap as f64 / sf_total as f64 * 100.0),
        flow_pct: finite(flow as f64 / sf_total as f64 * 100.0),
        samples: snap + flow,
    };

    // ---- jitter: deviation from a smoothed path while moving
    let mut dev_sq = 0.0f64;
    let mut dev_n = 0.0f64;
    let half = 3usize;
    let mut window_flags: Vec<(f64, bool)> = Vec::new(); // (t, is_smooth) per moving 500ms window
    {
        let mut win_start = f.first().map(|x| x.ms).unwrap_or(0.0);
        let mut win_dev = 0.0f64;
        let mut win_n = 0.0f64;
        let mut win_moving = 0.0f64;
        for i in half..f.len().saturating_sub(half) {
            let (mut sx, mut sy) = (0.0f64, 0.0f64);
            for j in i - half..=i + half {
                sx += f[j].x as f64;
                sy += f[j].y as f64;
            }
            let m = (2 * half + 1) as f64;
            let (mx, my) = (sx / m, sy / m);
            let d2 = (f[i].x as f64 - mx).powi(2) + (f[i].y as f64 - my).powi(2);
            let moving = i > 0 && i - 1 < kin.seg_v.len() && kin.seg_v[i - 1] > MOVING_SPEED;
            if moving {
                dev_sq += d2;
                dev_n += 1.0;
                win_dev += d2;
                win_n += 1.0;
                win_moving += 1.0;
            }
            if f[i].ms - win_start >= 500.0 {
                if win_moving >= 10.0 && win_n > 0.0 {
                    let rms = (win_dev / win_n).sqrt();
                    window_flags.push((win_start, rms < 0.004));
                }
                win_start = f[i].ms;
                win_dev = 0.0;
                win_n = 0.0;
                win_moving = 0.0;
            }
        }
    }
    let smooth_windows = window_flags.iter().filter(|(_, s)| *s).count();
    let jitter = Jitter {
        rms_cells: finite(if dev_n > 0.0 { (dev_sq / dev_n).sqrt() } else { 0.0 }),
        smooth_windows_pct: finite(if window_flags.is_empty() {
            0.0
        } else {
            smooth_windows as f64 / window_flags.len() as f64 * 100.0
        }),
    };

    // ---- timing
    let errs: Vec<f64> = notes.iter().filter_map(|n| n.err_ms).collect();
    let mut sorted_e = errs.clone();
    sorted_e.sort_by(|a, b| a.total_cmp(b));
    let hist_bin = 5.0;
    let hist_start = -(window + 2.5);
    let bins = ((window * 2.0 + 5.0) / hist_bin).ceil() as usize;
    let mut hist = vec![0u32; bins];
    for e in &errs {
        let b = (((e - hist_start) / hist_bin) as isize).clamp(0, bins as isize - 1) as usize;
        hist[b] += 1;
    }
    // Drift: least-squares slope of err over hit time.
    let drift = if errs.len() >= 8 {
        let ts: Vec<f64> = notes.iter().filter(|n| n.err_ms.is_some()).map(|n| n.t).collect();
        let tm = mean(&ts);
        let em = mean(&errs);
        let mut num = 0.0;
        let mut den = 0.0;
        for (t, e) in ts.iter().zip(&errs) {
            num += (t - tm) * (e - em);
            den += (t - tm) * (t - tm);
        }
        if den > 0.0 { num / den * 60_000.0 } else { 0.0 }
    } else {
        0.0
    };
    let first_ms = f.first().map(|x| x.ms).unwrap_or(0.0);
    let last_ms = f.last().map(|x| x.ms).unwrap_or(0.0);
    let mid = (first_ms + last_ms) / 2.0;
    let half_stats = |lo: f64, hi: f64| -> HalfStats {
        let ns: Vec<&NoteAnalysis> = notes.iter().filter(|n| n.t >= lo && n.t < hi).collect();
        let hits = ns.iter().filter(|n| n.hit).count();
        let errs: Vec<f64> = ns.iter().filter_map(|n| n.err_ms).collect();
        let mut d = 0.0;
        let mut t = 0.0;
        for (j, &st) in kin.smooth_t.iter().enumerate() {
            if st >= lo && st < hi && kin.seg_dt[j] > 0.0 {
                d += kin.seg_dist[j];
                t += kin.seg_dt[j];
            }
        }
        HalfStats {
            acc_pct: finite(if ns.is_empty() {
                0.0
            } else {
                hits as f64 / ns.len() as f64 * 100.0
            }),
            ur: finite(std_dev(&errs) * 10.0),
            avg_speed: finite(if t > 0.0 { d / t } else { 0.0 }),
        }
    };
    let timing = Timing {
        ur: finite(std_dev(&errs) * 10.0),
        mean_err_ms: finite(mean(&errs)),
        median_err_ms: finite(percentile(&sorted_e, 0.5)),
        hist_bin_ms: hist_bin,
        hist_start_ms: hist_start,
        hist,
        drift_ms_per_min: finite(drift),
        first_half: half_stats(first_ms, mid),
        second_half: half_stats(mid, last_ms + 1.0),
    };

    // ---- rolling UR (window of 20 hits, step 5)
    let hit_pairs: Vec<(f64, f64)> = notes
        .iter()
        .filter_map(|n| Some((n.hit_ms?, n.err_ms?)))
        .collect();
    let mut roll_t = Vec::new();
    let mut roll_v = Vec::new();
    let win = 20usize;
    let mut i = 0usize;
    while i + win <= hit_pairs.len() {
        let seg: Vec<f64> = hit_pairs[i..i + win].iter().map(|p| p.1).collect();
        roll_t.push(hit_pairs[i + win - 1].0);
        roll_v.push(std_dev(&seg) * 10.0);
        i += 5;
    }
    let rolling_ur = downsample(&roll_t, &roll_v);

    // ---- miss summary
    let miss_notes: Vec<&NoteAnalysis> = notes.iter().filter(|n| !n.hit).collect();
    let near: Vec<f64> = miss_notes.iter().filter_map(|n| n.near_dist.map(|d| d as f64)).collect();
    let mut on_fast = 0u32;
    let mut on_stream = 0u32;
    let mut other = 0u32;
    for n in &miss_notes {
        // Previous ATTEMPTED note by position (n.i is the map-wide index
        // and no longer matches positions once the window filter applies).
        let pos = notes.partition_point(|m| m.t < n.t).min(notes.len());
        let fast = pos > 0 && {
            let p = &notes[pos - 1];
            let (ax, ay) = crate::scene::grid_to_world(p.gx, p.gy);
            let (bx, by) = crate::scene::grid_to_world(n.gx, n.gy);
            let d = (((bx - ax) as f64).powi(2) + ((by - ay) as f64).powi(2)).sqrt();
            d > 1.0 && (n.t - p.t) < 350.0
        };
        let density = notes.iter().filter(|m| (m.t - n.t).abs() <= 1000.0).count();
        if fast {
            on_fast += 1;
        } else if density >= 8 {
            on_stream += 1;
        } else {
            other += 1;
        }
    }
    let misses = MissSummary {
        count: miss_notes.len() as u32,
        avg_near_dist: finite(mean(&near)),
        barely_pct: finite(if near.is_empty() {
            0.0
        } else {
            near.iter().filter(|d| **d < 0.65).count() as f64 / near.len() as f64 * 100.0
        }),
        lost_pct: finite(if near.is_empty() {
            0.0
        } else {
            near.iter().filter(|d| **d >= 1.2).count() as f64 / near.len() as f64 * 100.0
        }),
        on_fast_jumps: on_fast,
        on_streams: on_stream,
        other,
    };

    // ---- sections (30 s buckets over the run)
    let mut sections = Vec::new();
    if last_ms > first_ms {
        let sec = 30_000.0;
        let mut s = first_ms;
        while s < last_ms {
            let e = (s + sec).min(last_ms);
            let ns: Vec<&NoteAnalysis> = notes.iter().filter(|n| n.t >= s && n.t < e).collect();
            if !ns.is_empty() {
                let hits = ns.iter().filter(|n| n.hit).count();
                let errs: Vec<f64> = ns.iter().filter_map(|n| n.err_ms).collect();
                let mut d = 0.0;
                let mut t = 0.0;
                for (j, &st) in kin.smooth_t.iter().enumerate() {
                    if st >= s && st < e && kin.seg_dt[j] > 0.0 {
                        d += kin.seg_dist[j];
                        t += kin.seg_dt[j];
                    }
                }
                sections.push(Section {
                    start_ms: s,
                    end_ms: e,
                    acc_pct: finite(hits as f64 / ns.len() as f64 * 100.0),
                    ur: finite(std_dev(&errs) * 10.0),
                    misses: (ns.len() - hits) as u32,
                    notes: ns.len() as u32,
                    avg_speed: finite(if t > 0.0 { d / t } else { 0.0 }),
                });
            }
            s = e;
        }
    }

    // ---- heatmap (dwell-time weighted, ±1.6 cells)
    let hm_size = 48usize;
    let hm_extent = 1.6f32;
    let mut hm = vec![0.0f64; hm_size * hm_size];
    for (i, fr) in f.iter().enumerate() {
        let dt = if i < kin.seg_dt.len() { kin.seg_dt[i] } else { 0.0 };
        if dt <= 0.0 {
            continue;
        }
        let u = ((fr.x + hm_extent) / (2.0 * hm_extent) * hm_size as f32).floor() as isize;
        let v = ((hm_extent - fr.y) / (2.0 * hm_extent) * hm_size as f32).floor() as isize;
        if (0..hm_size as isize).contains(&u) && (0..hm_size as isize).contains(&v) {
            hm[v as usize * hm_size + u as usize] += dt;
        }
    }
    let hm_max = hm.iter().copied().fold(0.0f64, f64::max);
    let heatmap = Heatmap {
        size: hm_size as u32,
        extent: hm_extent,
        counts: hm
            .iter()
            .map(|v| {
                if hm_max > 0.0 {
                    // sqrt compresses the range so paths stay visible
                    ((v / hm_max).sqrt() * 255.0) as u8
                } else {
                    0
                }
            })
            .collect(),
    };

    // ---- frame deltas
    let deltas: Vec<f64> = f
        .windows(2)
        .map(|w| w[1].ms - w[0].ms)
        .filter(|d| *d > 0.0 && *d <= PAUSE_GAP_MS)
        .collect();
    let mut sorted_d = deltas.clone();
    sorted_d.sort_by(|a, b| a.total_cmp(b));
    let mut dhist = vec![0u32; 41];
    for d in &deltas {
        let b = (*d as usize).min(40);
        dhist[b] += 1;
    }
    let frame_deltas = FrameDeltas {
        avg_ms: finite(mean(&deltas)),
        median_ms: finite(percentile(&sorted_d, 0.5)),
        hist: dhist,
    };

    // ---- integrity signals (hints with context, never verdicts)
    let mut signals: Vec<Signal> = Vec::new();
    let report = integrity::verify_replay(replay, map);
    if !report.consistent() {
        let practice = replay.start_from_ms > 0;
        // Fewer hit flags than the header claims, with zero orphans, is
        // the RECORDER dropping frames under load — seen on legitimate
        // leaderboard scores. An edited file looks different (orphans,
        // impossible stats), so keep the scary wording for those.
        let header_hits = u32::try_from(replay.hits).unwrap_or(0);
        let dropped_only =
            report.orphan_flags == 0 && report.flagged_frames < header_hits;
        signals.push(Signal {
            id: "integrity".into(),
            severity: if practice || dropped_only { "notice" } else { "warn" }.into(),
            title: if dropped_only {
                format!(
                    "Incomplete recording: {} hit flag(s) missing",
                    header_hits - report.flagged_frames
                )
            } else {
                "File integrity check failed".into()
            },
            detail: if practice {
                "Header stats and frame data disagree. This is a practice-mode run                  (started mid-song) — header semantics for partial runs are not fully                  pinned yet, so treat this as informational."
            } else if dropped_only {
                "The header counts more hits than the file has flag frames — the                  game's recorder dropped frames (common under load). Derived stats                  undercount accordingly; the score itself is not in question."
            } else {
                "Header stats and frame data disagree — the file may be corrupted or edited."
            }
            .into(),
            times: vec![],
        });
    }
    // Teleports: large displacement in a tiny time step.
    let mut tp_times = Vec::new();
    for (i, w) in f.windows(2).enumerate() {
        // Frame times are song time; the recorder ticks in wall time, so
        // the single-frame gate must be wall ms or speed mods disable it.
        let dt_wall = (w[1].ms - w[0].ms) / speed;
        if dt_wall <= 0.0 || dt_wall > 25.0 {
            continue;
        }
        if kin.seg_dist.get(i).copied().unwrap_or(0.0) > 1.8 {
            tp_times.push(w[1].ms);
        }
    }
    if !tp_times.is_empty() {
        signals.push(Signal {
            id: "teleport".into(),
            // Population check (37 real leaderboard plays): tablet players
            // routinely show dozens of these — never escalate on count.
            severity: "notice".into(),
            title: format!("{} instant jump(s) across the field", tp_times.len()),
            detail: "The cursor crossed more than 1.8 cells within a single ~16 ms frame. \
                     Normal for absolute input devices (tablets) and frame drops; \
                     only meaningful together with other signals."
                .into(),
            times: tp_times.iter().copied().take(5).collect(),
        });
    }
    if accel_events.len() >= 5 {
        signals.push(Signal {
            id: "accel".into(),
            // Population check (37 real leaderboard plays, all speeds and
            // mods): 35 of 37 legitimate runs trip this — fast play plus
            // frame quantization IS extreme acceleration. Context only.
            severity: "info".into(),
            title: format!("{} extreme acceleration spikes", accel_events.len()),
            detail: "Acceleration beyond what smooth mouse movement produces. Nearly \
                     every fast legitimate play shows these (speed mods and tablets \
                     amplify them) — only meaningful together with other signals."
                .into(),
            times: accel_events.iter().copied().take(5).collect(),
        });
    }
    if jitter.smooth_windows_pct > 60.0 {
        signals.push(Signal {
            id: "smooth".into(),
            severity: "info".into(),
            title: "Very low micro-jitter".into(),
            detail: format!(
                "{:.0}% of moving windows show almost no hand tremor. Common for skilled \
                 mouse players on low sensitivity — NOT evidence on its own; only meaningful \
                 together with other signals.",
                jitter.smooth_windows_pct
            ),
            times: vec![],
        });
    }
    {
        let hits_with_dist: Vec<f64> =
            notes.iter().filter_map(|n| n.dist.map(|d| d as f64)).collect();
        if hits_with_dist.len() >= 30 {
            let centred =
                hits_with_dist.iter().filter(|d| **d < 0.06).count() as f64 / hits_with_dist.len() as f64;
            if centred > 0.4 {
                signals.push(Signal {
                    id: "center".into(),
                    severity: "notice".into(),
                    title: format!("{:.0}% of hits are dead-centre", centred * 100.0),
                    detail: "Human aim scatters around note centres; a very high dead-centre \
                             rate is unusual."
                        .into(),
                    times: vec![],
                });
            }
        }
    }
    // Recording-rate shift within the run.
    if deltas.len() > 400 {
        let win = deltas.len() / 8;
        let mut medians = Vec::new();
        for c in deltas.chunks(win) {
            let mut s: Vec<f64> = c.to_vec();
            s.sort_by(|a, b| a.total_cmp(b));
            medians.push(percentile(&s, 0.5));
        }
        let lo = medians.iter().copied().fold(f64::MAX, f64::min);
        let hi = medians.iter().copied().fold(0.0f64, f64::max);
        if hi > 0.0 && lo / hi < 0.6 {
            signals.push(Signal {
                id: "delta_shift".into(),
                severity: "notice".into(),
                title: "Recording rate changes mid-run".into(),
                detail: format!(
                    "Frame spacing shifts from ~{lo:.1} ms to ~{hi:.1} ms between sections — \
                     can indicate splicing, or simply performance dips."
                ),
                times: vec![],
            });
        }
    }
    if outcome.orphan_flags > 0 {
        signals.push(Signal {
            id: "orphans".into(),
            severity: "notice".into(),
            title: format!("{} hit flag(s) match no note", outcome.orphan_flags),
            detail: "Frames marked as hits that align with no map note — inconsistent data.".into(),
            times: vec![],
        });
    }
    let verdict = if signals.iter().any(|s| s.severity == "warn") {
        "warn"
    } else if signals.iter().any(|s| s.severity == "notice") {
        "notice"
    } else {
        "clean"
    }
    .to_string();

    let hits_total = notes.iter().filter(|n| n.hit).count() as u32;
    Analysis {
        meta: Meta {
            frame_count: f.len(),
            first_ms,
            last_ms,
            speed: speed as f32,
            hits: hits_total,
            misses: notes.len() as u32 - hits_total,
        },
        frames: FrameArrays {
            t: f.iter().map(|x| x.ms as f32).collect(),
            x: f.iter().map(|x| x.x).collect(),
            y: f.iter().map(|x| x.y).collect(),
        },
        speed_series: downsample(&kin.smooth_t, &kin.smooth_v),
        cursor,
        overshoot,
        direction_bias: bias,
        snap_flow,
        jitter,
        timing,
        rolling_ur,
        notes,
        misses,
        sections,
        heatmap,
        frame_deltas,
        signals,
        verdict,
    }
}

/// Cursor distance between two replays over time (ghost comparison),
/// sampled on the primary's frame times, in cells.
pub fn cursor_distance(a: &Replay, b: &Replay) -> Series {
    let mut t = Vec::new();
    let mut v = Vec::new();
    if a.frames.is_empty() || b.frames.is_empty() {
        return Series::default();
    }
    let lo = a.frames.first().unwrap().ms.max(b.frames.first().unwrap().ms);
    let hi = a.frames.last().unwrap().ms.min(b.frames.last().unwrap().ms);
    for fr in &a.frames {
        if fr.ms < lo || fr.ms > hi {
            continue;
        }
        let (bx, by) = cursor_at(&b.frames, fr.ms);
        let d = (((fr.x - bx) as f64).powi(2) + ((fr.y - by) as f64).powi(2)).sqrt();
        t.push(fr.ms);
        v.push(d);
    }
    downsample(&t, &v)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_formats::map::{Map, MapMeta, Note};
    use rhythia_formats::rhr::{Frame, Replay};

    fn frame(ms: f64, x: f32, y: f32, hit: bool) -> Frame {
        Frame { ms, x, y, health: 1.0, hit }
    }

    fn test_map(notes: Vec<Note>) -> Map {
        Map { meta: MapMeta::default(), notes, audio: None, cover: None }
    }

    /// Header stats derived from frames+notes so the integrity check
    /// passes — synthetic replays must not trip the "integrity" signal.
    fn replay_for(frames: Vec<Frame>, note_count: usize) -> Replay {
        let hits = frames.iter().filter(|f| f.hit).count() as i32;
        let misses = note_count as i32 - hits;
        let accuracy_pct = if note_count > 0 {
            hits as f32 / note_count as f32 * 100.0
        } else {
            100.0
        };
        Replay {
            version: 6,
            timestamp_ticks: 0,
            player_name: "Test".into(),
            legacy_map_id: String::new(),
            map_id: 0,
            start_from_ms: 0,
            mode: String::new(),
            passed: true,
            mods: "[]".into(),
            spin: false,
            speed: 1.0,
            total_score: 0,
            accuracy_pct,
            hits,
            misses,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            frames,
            trailing_bytes: 0,
        }
    }

    fn test_replay(frames: Vec<Frame>) -> Replay {
        replay_for(frames, 0)
    }

    /// Straight move from (−1,0) to (1,0) over one second: 2 cells/s.
    #[test]
    fn speed_and_path_length() {
        let frames: Vec<Frame> = (0..=60)
            .map(|i| frame(i as f64 * 16.0, -1.0 + i as f32 * (2.0 / 60.0), 0.0, false))
            .collect();
        let a = analyze(&test_map(vec![]), &test_replay(frames));
        assert!((a.cursor.total_path_cells - 2.0).abs() < 0.01, "{}", a.cursor.total_path_cells);
        let expect_v = 2.0 / (60.0 * 16.0 / 1000.0);
        assert!(
            (a.cursor.max_speed.v - expect_v).abs() < 0.15,
            "max {} vs {}",
            a.cursor.max_speed.v,
            expect_v
        );
        assert!(a.cursor.moving_pct > 95.0);
    }

    /// A 2× speed-mod replay covers the same song span in half the wall
    /// time — cursor speeds double.
    #[test]
    fn speed_mod_scales_wall_clock() {
        let frames: Vec<Frame> = (0..=60)
            .map(|i| frame(i as f64 * 16.0, -1.0 + i as f32 * (2.0 / 60.0), 0.0, false))
            .collect();
        let mut r = test_replay(frames);
        r.speed = 2.0;
        let a = analyze(&test_map(vec![]), &r);
        let expect_v = 2.0 * 2.0 / (60.0 * 16.0 / 1000.0);
        assert!((a.cursor.max_speed.v - expect_v).abs() < 0.3);
    }

    /// A pause (5 s gap) must not register as movement or speed.
    #[test]
    fn pause_gap_excluded() {
        let mut frames = vec![frame(0.0, -1.0, 0.0, false), frame(16.0, -1.0, 0.0, false)];
        frames.push(frame(5016.0, 1.0, 0.0, false)); // jump across the pause
        frames.push(frame(5032.0, 1.0, 0.0, false));
        let a = analyze(&test_map(vec![]), &test_replay(frames));
        assert!(a.cursor.total_path_cells < 0.01, "{}", a.cursor.total_path_cells);
        assert!(a.signals.iter().all(|s| s.id != "teleport"));
    }

    /// An in-frame 2-cell jump in 10 ms is a teleport signal.
    #[test]
    fn teleport_flagged() {
        let mut frames: Vec<Frame> =
            (0..30).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        frames.push(frame(480.0, 2.0, 0.0, false));
        frames.push(frame(496.0, 2.0, 0.0, false));
        let a = analyze(&test_map(vec![]), &test_replay(frames));
        assert!(a.signals.iter().any(|s| s.id == "teleport"));
        assert_ne!(a.verdict, "clean");
    }

    /// Perfect timing on every note: UR 0; constant +10 ms: UR 0 but mean 10.
    #[test]
    fn timing_ur_and_mean() {
        let notes = vec![
            Note { time_ms: 500, x: 1.0, y: 1.0 },
            Note { time_ms: 1000, x: 1.0, y: 1.0 },
            Note { time_ms: 1500, x: 1.0, y: 1.0 },
        ];
        let mut frames: Vec<Frame> =
            (0..125).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        for f in frames.iter_mut() {
            if [510.0, 1010.0, 1510.0].contains(&f.ms.round()) {}
        }
        // insert hit flags at +10ms via dedicated frames
        frames.push(frame(510.0, 0.0, 0.0, true));
        frames.push(frame(1010.0, 0.0, 0.0, true));
        frames.push(frame(1510.0, 0.0, 0.0, true));
        frames.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let n = notes.len();
        let a = analyze(&test_map(notes), &replay_for(frames, n));
        assert_eq!(a.meta.hits, 3);
        assert!((a.timing.mean_err_ms - 10.0).abs() < 0.5, "{}", a.timing.mean_err_ms);
        assert!(a.timing.ur < 1.0, "{}", a.timing.ur);
    }

    /// One hit note dead centre, one missed far away: near-miss recorded.
    #[test]
    fn near_miss_distance() {
        let notes = vec![
            Note { time_ms: 500, x: 1.0, y: 1.0 }, // world (0,0)
            Note { time_ms: 1000, x: 2.0, y: 1.0 }, // world (1,0)
        ];
        let mut frames: Vec<Frame> =
            (0..80).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        frames.push(frame(500.0, 0.0, 0.0, true));
        frames.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let n = notes.len();
        let a = analyze(&test_map(notes), &replay_for(frames, n));
        assert!(a.notes[0].hit);
        assert!((a.notes[0].dist.unwrap()).abs() < 0.01);
        assert!(!a.notes[1].hit);
        let nd = a.notes[1].near_dist.unwrap();
        assert!((nd - 1.0).abs() < 0.05, "{nd}");
        assert_eq!(a.misses.count, 1);
    }

    /// Efficiency: cursor wanders 4 cells while notes only need 2.
    #[test]
    fn movement_efficiency() {
        let notes = vec![
            Note { time_ms: 100, x: 0.0, y: 1.0 },
            Note { time_ms: 1900, x: 2.0, y: 1.0 },
        ];
        // Legs: (−1→1) + (1→0) + (0→1) = 2 + 1 + 1 = 4 cells travelled.
        let mut frames = Vec::new();
        let mut t = 0.0;
        for (from, to) in [(-1.0, 1.0), (1.0, 0.0), (0.0, 1.0f32)] {
            for i in 0..40 {
                let k = i as f32 / 40.0;
                frames.push(frame(t, from + (to - from) * k, 0.0, false));
                t += 16.0;
            }
        }
        let n = notes.len();
        let a = analyze(&test_map(notes), &replay_for(frames, n));
        assert!((a.cursor.total_path_cells - 4.0).abs() < 0.1, "{}", a.cursor.total_path_cells);
        assert!((a.cursor.optimal_path_cells - 2.0).abs() < 0.01);
        assert!((a.cursor.efficiency_pct - 50.0).abs() < 3.0, "{}", a.cursor.efficiency_pct);
    }

    /// Perfectly linear long moves ⇒ smooth-windows signal fires (info).
    #[test]
    fn too_smooth_is_info_only() {
        // Triangle wave: perfectly linear strokes back and forth — no
        // teleport-like wrap, just an unnaturally clean path.
        let frames: Vec<Frame> = (0..500)
            .map(|i| {
                let c = i % 200;
                let k = if c < 100 { c } else { 200 - c } as f32 / 100.0;
                frame(i as f64 * 16.0, -1.0 + 2.0 * k, 0.0, false)
            })
            .collect();
        let a = analyze(&test_map(vec![]), &test_replay(frames));
        let s = a.signals.iter().find(|s| s.id == "smooth");
        assert!(s.is_some());
        assert_eq!(s.unwrap().severity, "info");
        // info alone must not taint the verdict
        assert_eq!(a.verdict, "clean");
    }

    /// Sections: two 30 s buckets with different accuracy.
    #[test]
    fn sections_split() {
        let mut notes = Vec::new();
        for i in 0..10 {
            notes.push(Note { time_ms: 1000 + i * 2000, x: 1.0, y: 1.0 });
        }
        for i in 0..10 {
            notes.push(Note { time_ms: 31000 + i * 2000, x: 1.0, y: 1.0 });
        }
        let mut frames: Vec<Frame> =
            (0..3200).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        // hit all of bucket 1, none of bucket 2
        for i in 0..10 {
            frames.push(frame((1000 + i * 2000) as f64, 0.0, 0.0, true));
        }
        frames.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let n = notes.len();
        let a = analyze(&test_map(notes), &replay_for(frames, n));
        assert!(a.sections.len() >= 2);
        assert!(a.sections[0].acc_pct > 99.0);
        assert!(a.sections[1].acc_pct < 1.0);
    }

    /// The whole payload serializes to JSON without NaN/Inf issues.
    #[test]
    fn serializes_without_nan() {
        let a = analyze(&test_map(vec![]), &test_replay(vec![]));
        let s = serde_json::to_string(&a).unwrap();
        assert!(!s.contains("null,null"));
        // empty replay: everything finite / defaulted
        assert_eq!(a.meta.frame_count, 0);
    }

    /// A 2× speed mod stretches song-time frame deltas to ~33 ms — the
    /// teleport gate must still fire (it compares WALL time).
    #[test]
    fn teleport_detected_under_speed_mod() {
        let mut frames: Vec<Frame> =
            (0..30).map(|i| frame(i as f64 * 33.4, 0.0, 0.0, false)).collect();
        frames.push(frame(1002.0, 2.0, 0.0, false));
        frames.push(frame(1035.4, 2.0, 0.0, false));
        let mut r = test_replay(frames);
        r.speed = 2.0;
        let a = analyze(&test_map(vec![]), &r);
        assert!(a.signals.iter().any(|s| s.id == "teleport"));
    }

    /// A failed run: notes after the fail point were never attempted and
    /// must not count as misses.
    #[test]
    fn failed_run_ignores_post_fail_notes() {
        let notes = vec![
            Note { time_ms: 500, x: 1.0, y: 1.0 },
            Note { time_ms: 1000, x: 1.0, y: 1.0 },
            Note { time_ms: 5000, x: 1.0, y: 1.0 },
            Note { time_ms: 6000, x: 1.0, y: 1.0 },
        ];
        let mut frames: Vec<Frame> =
            (0..80).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        frames.push(frame(500.0, 0.0, 0.0, true));
        frames.push(frame(1000.0, 0.0, 0.0, true));
        frames.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let mut r = replay_for(frames, 2); // 2 attempted notes
        r.passed = false;
        r.fail_time_ms = 1300;
        let a = analyze(&test_map(notes), &r);
        assert_eq!(a.meta.hits, 2);
        assert_eq!(a.meta.misses, 0, "post-fail notes are not misses");
        assert_eq!(a.notes.len(), 2);
    }

    /// A practice run starting mid-song must not count skipped intro
    /// notes as misses, and a header mismatch stays a notice, not a warn.
    #[test]
    fn practice_run_skips_pre_start_notes() {
        let notes = vec![
            Note { time_ms: 500, x: 1.0, y: 1.0 },
            Note { time_ms: 10_000, x: 1.0, y: 1.0 },
        ];
        let mut frames: Vec<Frame> = (0..40)
            .map(|i| frame(9800.0 + i as f64 * 16.0, 0.0, 0.0, false))
            .collect();
        frames.push(frame(10_000.0, 0.0, 0.0, true));
        frames.sort_by(|a, b| a.ms.total_cmp(&b.ms));
        let mut r = replay_for(frames, 1);
        r.start_from_ms = 9000;
        let a = analyze(&test_map(notes), &r);
        assert_eq!(a.meta.hits, 1);
        assert_eq!(a.meta.misses, 0, "pre-start notes are not misses");
        assert_ne!(a.verdict, "warn", "practice runs must never hard-warn on header checks");
    }

    #[test]
    fn ghost_cursor_distance() {
        let fa: Vec<Frame> = (0..100).map(|i| frame(i as f64 * 16.0, 0.0, 0.0, false)).collect();
        let fb: Vec<Frame> = (0..100).map(|i| frame(i as f64 * 16.0, 1.0, 0.0, false)).collect();
        let s = cursor_distance(&test_replay(fa), &test_replay(fb));
        assert!(!s.v.is_empty());
        assert!(s.v.iter().all(|v| (v - 1.0).abs() < 0.01));
    }
}
