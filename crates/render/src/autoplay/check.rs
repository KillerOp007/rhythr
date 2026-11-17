//! Recognising an Autoplay video by the path its cursor takes.
//!
//! Nothing is hidden in the picture. The checker gets the video and its
//! map, computes the Autoplay path for every setting and measures whether
//! the cursor in the video runs on one of them:
//!
//!  1. coarse: for every candidate path and picture scale a small window
//!     rides along the predicted cursor position on a small copy of the
//!     video. Where the prediction is right the cursor stands still in
//!     that window and its average shows a bright spot; where it is wrong
//!     the cursor smears away.
//!  2. fine: scale and time offset of the best paths at full size, judged
//!     by how sharp the averaged window is.
//!  3. cursor: the median of the riding window is the cursor's picture
//!     (whatever the skin makes it look like); it is searched in every
//!     frame near the prediction.
//!  4. verdict: the found positions against every candidate path, as the
//!     share of the frames in which the cursor sits on the curve.
//!
//! A real play of the same map passes the same notes at the same times,
//! but not on the same curve: in the trial its share was 7 percent, against
//! 90 to 98 for Autoplay videos, re-encoded or not.
//!
//! The path is the proof, so a change to the Autoplay movement changes what
//! this checker accepts: videos of released versions verify only as long as
//! their generator is still here (see `docs/AUTOPLAY-VERIFICATION.md`).

use super::{generate, Aim, AutoplaySettings, DanceAmount, Movement};
use crate::mark::fft::Fft2;
use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rustfft::num_complex::Complex;

/// Where the checker gets its pictures from. The CLI decodes with ffmpeg;
/// tests draw their own.
pub trait FrameSource {
    /// Decodes the video from its start as 8 bit gray scaled to `w` by `h`
    /// at the constant frame rate of [`VideoInfo::fps`], handing every
    /// frame (its index, its pixels) to `f` in order until `f` returns
    /// false or the video ends.
    fn frames(&mut self, w: usize, h: usize, f: &mut dyn FnMut(usize, &[u8]) -> bool) -> Result<(), String>;
}

#[derive(Clone, Copy, Debug)]
pub struct VideoInfo {
    pub width: usize,
    pub height: usize,
    pub fps: f64,
    /// The frame count when the container says how long the video is.
    pub frames: Option<usize>,
}

/// A key's pattern: it decides the sides of the path's arcs, so every key
/// has its own variant of each path (they differ in a tenth of the time).
pub struct Key {
    pub label: String,
    pub dice: Vec<f32>,
}

#[derive(Clone, Debug)]
pub struct Options {
    /// The speed mods to try; empty means every one the game has.
    pub speed_mods: Vec<f32>,
    /// The rest threshold the video was rendered with, when known.
    pub rest_secs: Option<f64>,
    /// Map time at the video's first frame, in ms.
    pub start_ms: f64,
    /// Playback speed of the video on top of the speed mod (the `--speed`
    /// of a render).
    pub playback: f64,
    /// Only this many frames from the start are looked at.
    pub max_frames: usize,
    /// Worker threads; 0 picks by the machine.
    pub threads: usize,
}

impl Default for Options {
    fn default() -> Self {
        Options { speed_mods: Vec::new(), rest_secs: None, start_ms: 0.0, playback: 1.0, max_frames: 7200, threads: 0 }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum VerdictKind {
    Found,
    Weak,
    None,
}

/// At or above this share of frames on the curve the video is an Autoplay
/// render; below [`WEAK_PCT`] it is not. The trial's Autoplay clips sat at
/// 90 to 98 percent, a real play of the same map at 7.
pub const FOUND_PCT: f32 = 60.0;
pub const WEAK_PCT: f32 = 25.0;
/// "On the curve": this close, in pixels of a 1920 wide picture.
const ON_PX: f32 = 4.0;
/// Frames in which the path moves slower than this (cursor units per video
/// second) do not count: a cursor at rest sits on any path that rests.
const MOVING: f32 = 0.3;
/// Fewer moving frames than this are not enough to tell.
const MIN_MOVING: usize = 60;

#[derive(Clone, Debug)]
pub struct Report {
    pub kind: VerdictKind,
    /// Share of the frames whose cursor lies on the best path's curve
    /// within a frame or so of time (a converted frame rate shows some
    /// frames a little early or late).
    pub on_curve_pct: f32,
    /// The same at the frame's own time.
    pub on_time_pct: f32,
    /// Median distance between cursor and path, in pixels of a 1920 wide
    /// picture.
    pub median_px: f32,
    /// Frames in which the cursor was looked for, and how many of them
    /// have the path moving (only those count).
    pub frames: usize,
    pub moving_frames: usize,
    /// The settings of the best path and the key it was made with.
    pub settings: AutoplaySettings,
    pub key: String,
    /// Picture pixels per cursor unit, and the video's offset in ms.
    pub scale: f32,
    pub time_offset_ms: f32,
    /// How well the cursor's picture matched, median over the frames (1 is
    /// a perfect match).
    pub match_quality: f32,
    /// The next best other setting and its share.
    pub runner_up: Option<(String, f32)>,
    /// Why there is no verdict from the numbers, when there is none.
    pub note: Option<String>,
}

/// The settings in words, as the verdict names them.
pub fn describe(s: &AutoplaySettings) -> String {
    let aim = match s.aim {
        Aim::Centre => "centre",
        Aim::Flow => "flow",
    };
    match s.movement {
        Movement::Normal => format!("normal movement, aim {aim}, {:.2}x", s.speed_mod),
        Movement::Dance => {
            let amount = match s.dance {
                DanceAmount::Subtle => "subtle",
                DanceAmount::Medium => "medium",
                DanceAmount::Wild => "wild",
            };
            format!("cursor dance {amount}, aim {aim}, {:.2}x", s.speed_mod)
        }
    }
}

/// Every movement the Autoplay settings can produce (the dance amount
/// means nothing to the normal movement).
fn settings_grid() -> Vec<(Movement, Aim, DanceAmount)> {
    let mut g = Vec::new();
    for aim in [Aim::Centre, Aim::Flow] {
        for dance in [DanceAmount::Subtle, DanceAmount::Medium, DanceAmount::Wild] {
            g.push((Movement::Dance, aim, dance));
        }
        g.push((Movement::Normal, aim, DanceAmount::Medium));
    }
    g
}

/// A path as a function of map time.
struct Curve {
    t: Vec<f64>,
    x: Vec<f32>,
    y: Vec<f32>,
}

impl Curve {
    fn of(run: &Replay) -> Curve {
        let mut c = Curve { t: Vec::with_capacity(run.frames.len()), x: Vec::new(), y: Vec::new() };
        for f in &run.frames {
            if c.t.last().is_some_and(|&l| f.ms <= l) {
                continue;
            }
            c.t.push(f.ms);
            c.x.push(f.x);
            c.y.push(f.y);
        }
        c
    }

    fn at(&self, ms: f64) -> (f32, f32) {
        let n = self.t.len();
        if n == 0 {
            return (0.0, 0.0);
        }
        if ms <= self.t[0] {
            return (self.x[0], self.y[0]);
        }
        if ms >= self.t[n - 1] {
            return (self.x[n - 1], self.y[n - 1]);
        }
        let i = self.t.partition_point(|&t| t <= ms);
        let k = ((ms - self.t[i - 1]) / (self.t[i] - self.t[i - 1])) as f32;
        (self.x[i - 1] + (self.x[i] - self.x[i - 1]) * k, self.y[i - 1] + (self.y[i] - self.y[i - 1]) * k)
    }
}

struct Candidate {
    settings: AutoplaySettings,
    key: usize,
    curve: Curve,
}

#[derive(Clone, Copy)]
enum Measure {
    /// How far the brightest spot stands above the window: tolerant, for
    /// the coarse pass.
    Peak,
    /// The energy of the window's edges: a cursor that stands still keeps
    /// its outline, one that wanders is a soft blob with hardly any edge.
    Edges,
}

fn measure(acc: &[u32], size: usize, count: u32, how: Measure) -> f32 {
    if count == 0 {
        return 0.0;
    }
    let inv = 1.0 / count as f32;
    match how {
        Measure::Peak => {
            let mut v = acc.to_vec();
            v.sort_unstable();
            (v[v.len() - 1] - v[v.len() / 2]) as f32 * inv
        }
        Measure::Edges => {
            let mut sum = 0.0f64;
            for y in 1..size - 1 {
                for x in 1..size - 1 {
                    let gx = (acc[y * size + x + 1] as f32 - acc[y * size + x - 1] as f32) * 0.5 * inv;
                    let gy = (acc[(y + 1) * size + x] as f32 - acc[(y - 1) * size + x] as f32) * 0.5 * inv;
                    sum += (gx * gx + gy * gy) as f64;
                }
            }
            (sum / ((size - 2) * (size - 2)) as f64) as f32
        }
    }
}

/// One candidate of a ride: which track it follows and how track
/// coordinates become pixels (`x` times `.1`, `y` times `.2`, from the
/// picture's centre).
type Rider = (usize, f32, f32);

fn ride_batch(
    batch: &[(usize, Vec<u8>)],
    acc: &mut [u32],
    cnt: &mut [u32],
    riders: &[Rider],
    tracks: &[Vec<(f32, f32)>],
    (w, h, half): (usize, usize, usize),
    threads: usize,
) {
    let size = 2 * half + 1;
    let area = size * size;
    let pw = w + 2 * half;
    let per = riders.len().div_ceil(threads.max(1)).max(1);
    std::thread::scope(|s| {
        for ((acc_c, cnt_c), riders_c) in acc.chunks_mut(per * area).zip(cnt.chunks_mut(per)).zip(riders.chunks(per)) {
            s.spawn(move || {
                for (ci, &(k, sx, sy)) in riders_c.iter().enumerate() {
                    let a = &mut acc_c[ci * area..(ci + 1) * area];
                    for (j, p) in batch {
                        let (x, y) = tracks[k][*j];
                        let cx = (w as f32 / 2.0 + sx * x).round() as i64;
                        let cy = (h as f32 / 2.0 + sy * y).round() as i64;
                        if cx < 0 || cy < 0 || cx >= w as i64 || cy >= h as i64 {
                            continue;
                        }
                        // the frame is padded by `half`, so the window's
                        // top left corner is the centre's own coordinate
                        let (cx, cy) = (cx as usize, cy as usize);
                        for r in 0..size {
                            let row = &p[(cy + r) * pw + cx..(cy + r) * pw + cx + size];
                            for (d, &v) in a[r * size..(r + 1) * size].iter_mut().zip(row) {
                                *d += v as u32;
                            }
                        }
                        cnt_c[ci] += 1;
                    }
                }
            });
        }
    });
}

/// Sums, for every rider, the window that rides along its predicted
/// position over every `step`th frame, and measures the average.
/// `tracks[k][j]` is track k's path position at the j-th used frame.
#[allow(clippy::too_many_arguments)]
fn ride(
    src: &mut dyn FrameSource,
    (w, h): (usize, usize),
    step: usize,
    half: usize,
    tracks: &[Vec<(f32, f32)>],
    riders: &[Rider],
    how: Measure,
    threads: usize,
) -> Result<Vec<f32>, String> {
    let size = 2 * half + 1;
    let (pw, ph) = (w + 2 * half, h + 2 * half);
    let n_used = tracks.iter().map(|t| t.len()).min().unwrap_or(0);
    let mut acc = vec![0u32; riders.len() * size * size];
    let mut cnt = vec![0u32; riders.len()];
    let batch_max = ((32usize << 20) / (pw * ph)).clamp(1, 64);
    let mut batch: Vec<(usize, Vec<u8>)> = Vec::with_capacity(batch_max);
    let mut seen = 0u32;
    src.frames(w, h, &mut |n, px| {
        if !n.is_multiple_of(step) {
            return true;
        }
        let j = n / step;
        if j >= n_used {
            return false;
        }
        let mut p = vec![0u8; pw * ph];
        for y in 0..h {
            p[(y + half) * pw + half..(y + half) * pw + half + w].copy_from_slice(&px[y * w..(y + 1) * w]);
        }
        batch.push((j, p));
        seen += 1;
        if batch.len() >= batch_max {
            ride_batch(&batch, &mut acc, &mut cnt, riders, tracks, (w, h, half), threads);
            batch.clear();
        }
        true
    })?;
    if !batch.is_empty() {
        ride_batch(&batch, &mut acc, &mut cnt, riders, tracks, (w, h, half), threads);
    }
    // a rider that left the picture most of the time has no say
    let min_cnt = (seen / 4).max(10);
    Ok((0..riders.len())
        .map(|c| if cnt[c] < min_cnt { 0.0 } else { measure(&acc[c * size * size..(c + 1) * size * size], size, cnt[c], how) })
        .collect())
}

/// Where the cursor was found: the frame, the position in working pixels
/// and how well its picture matched there.
struct Track {
    n: Vec<usize>,
    mx: Vec<f32>,
    my: Vec<f32>,
    q: Vec<f32>,
}

/// The picture position as a function of the path position: the six terms
/// of a quadratic for x and for y (the camera sways with the cursor, which
/// bends the straight relation a little).
struct Fit {
    cx: [f64; 6],
    cy: [f64; 6],
}

fn design(p: (f32, f32)) -> [f64; 6] {
    let (x, y) = (p.0 as f64, p.1 as f64);
    [1.0, x, y, x * x, x * y, y * y]
}

#[allow(clippy::needless_range_loop)]
fn solve6(mut a: [[f64; 6]; 6], mut b: [[f64; 2]; 6]) -> Option<[[f64; 2]; 6]> {
    let scale = a.iter().flatten().fold(0.0f64, |m, v| m.max(v.abs()));
    for i in 0..6 {
        let mut p = i;
        for r in i + 1..6 {
            if a[r][i].abs() > a[p][i].abs() {
                p = r;
            }
        }
        if a[p][i].abs() <= 1e-10 * scale {
            return None;
        }
        a.swap(i, p);
        b.swap(i, p);
        for r in i + 1..6 {
            let f = a[r][i] / a[i][i];
            if f == 0.0 {
                continue;
            }
            for c in i..6 {
                a[r][c] -= f * a[i][c];
            }
            b[r][0] -= f * b[i][0];
            b[r][1] -= f * b[i][1];
        }
    }
    let mut x = [[0.0f64; 2]; 6];
    for i in (0..6).rev() {
        for k in 0..2 {
            let mut v = b[i][k];
            for c in i + 1..6 {
                v -= a[i][c] * x[c][k];
            }
            x[i][k] = v / a[i][i];
        }
    }
    Some(x)
}

/// Least squares over the frames marked in `ok`.
fn fit(pts: &[(f32, f32)], track: &Track, ok: &[bool]) -> Option<Fit> {
    let mut a = [[0.0f64; 6]; 6];
    let mut b = [[0.0f64; 2]; 6];
    for i in 0..pts.len() {
        if !ok[i] {
            continue;
        }
        let d = design(pts[i]);
        for r in 0..6 {
            for c in 0..6 {
                a[r][c] += d[r] * d[c];
            }
            b[r][0] += d[r] * track.mx[i] as f64;
            b[r][1] += d[r] * track.my[i] as f64;
        }
    }
    let x = solve6(a, b)?;
    Some(Fit { cx: x.map(|v| v[0]), cy: x.map(|v| v[1]) })
}

/// Distance of every found cursor from where the fit puts the path, in
/// pixels of a 1920 wide picture.
fn residuals(f: &Fit, pts: &[(f32, f32)], track: &Track, to1920: f32) -> Vec<f32> {
    pts.iter()
        .enumerate()
        .map(|(i, &p)| {
            let d = design(p);
            let px: f64 = d.iter().zip(&f.cx).map(|(a, b)| a * b).sum();
            let py: f64 = d.iter().zip(&f.cy).map(|(a, b)| a * b).sum();
            ((px - track.mx[i] as f64).hypot(py - track.my[i] as f64)) as f32 * to1920
        })
        .collect()
}

/// A fit is only believed when its straight part is the picture scale the
/// search found: a fit that folds the path onto one spot would put a
/// cursor that never moves "on the path".
fn plausible(f: &Fit, scale: f32) -> bool {
    let (a, b, c, d) = (f.cx[1], f.cx[2], f.cy[1], f.cy[2]);
    let q = a * a + b * b + c * c + d * d;
    let det = a * d - b * c;
    let root = (q * q - 4.0 * det * det).max(0.0).sqrt();
    let (big, small) = (((q + root) / 2.0).sqrt(), ((q - root) / 2.0).max(0.0).sqrt());
    let s = scale as f64;
    small >= 0.5 * s && big <= 2.0 * s
}

#[derive(Clone, Copy)]
struct Judged {
    on_curve: f32,
    on_time: f32,
    median_px: f32,
    lag_ms: f32,
    moving: usize,
}

/// How much of the found cursor's way lies on this curve. `at(frame,
/// shift_ms)` is the map time of a video frame.
fn judge(track: &Track, curve: &Curve, at: &dyn Fn(usize, f64) -> f64, scale: f32, to1920: f32) -> Option<Judged> {
    let n = track.n.len();
    let good: Vec<bool> = track.q.iter().map(|&q| q > 0.5).collect();
    let moving: Vec<bool> = track
        .n
        .iter()
        .map(|&f| {
            let a = curve.at(at(f, -50.0));
            let b = curve.at(at(f, 50.0));
            (b.0 - a.0).hypot(b.1 - a.1) / 0.1 >= MOVING
        })
        .collect();
    let n_moving = moving.iter().filter(|m| **m).count();
    if n_moving < MIN_MOVING {
        return None;
    }
    let share = |r: &[f32], lim: f32| 100.0 * r.iter().zip(&moving).filter(|(r, m)| **m && **r < lim).count() as f32 / n_moving as f32;
    let path = |shift: f64| -> Vec<(f32, f32)> { track.n.iter().map(|&f| curve.at(at(f, shift))).collect() };

    // the cursor is drawn a moment before or after the frame's time: the
    // best small lag
    let mut best: Option<(f32, f32, f64, Fit)> = None;
    for step in -24..=24 {
        let lag = step as f64 * 0.5;
        let pts = path(lag);
        let mut ok = good.clone();
        let mut last: Option<(Fit, Vec<f32>)> = None;
        for lim in [10.0f32, 5.0, 3.0] {
            if ok.iter().filter(|v| **v).count() < 30 {
                break;
            }
            let Some(f) = fit(&pts, track, &ok) else { break };
            let r = residuals(&f, &pts, track, to1920);
            for i in 0..n {
                ok[i] = good[i] && r[i] < lim;
            }
            last = Some((f, r));
        }
        let Some((f, r)) = last else { continue };
        if !plausible(&f, scale) {
            continue;
        }
        let sh = share(&r, ON_PX);
        let mut m: Vec<f32> = r.iter().zip(&moving).filter(|(_, m)| **m).map(|(r, _)| *r).collect();
        m.sort_by(|a, b| a.total_cmp(b));
        let med = m[m.len() / 2];
        // the larger share wins, the smaller distance between equal shares
        if best.as_ref().is_none_or(|b| sh > b.0 || (sh == b.0 && med < b.1)) {
            best = Some((sh, med, lag, f));
        }
    }
    let (on_time, median_px, lag, f) = best?;
    // on the curve: the nearest point of the path within a frame or so
    let mut near = vec![f32::INFINITY; n];
    for step in -12..=12 {
        let r = residuals(&f, &path(lag + step as f64 * 2.0), track, to1920);
        for (a, b) in near.iter_mut().zip(&r) {
            *a = a.min(*b);
        }
    }
    Some(Judged { on_curve: share(&near, ON_PX), on_time, median_px, lag_ms: lag as f32, moving: n_moving })
}

fn even(v: f64) -> usize {
    (((v / 2.0).round() as usize) * 2).max(2)
}

fn no_verdict(why: &str) -> Report {
    Report {
        kind: VerdictKind::None,
        on_curve_pct: 0.0,
        on_time_pct: 0.0,
        median_px: 0.0,
        frames: 0,
        moving_frames: 0,
        settings: AutoplaySettings::default(),
        key: String::new(),
        scale: 0.0,
        time_offset_ms: 0.0,
        match_quality: 0.0,
        runner_up: None,
        note: Some(why.to_string()),
    }
}

/// Checks a video against the Autoplay paths of its map. `keys[0]` is the
/// key the search runs with; the verdict tries every key. `progress` hears
/// what the checker is doing.
pub fn check(
    src: &mut dyn FrameSource,
    info: &VideoInfo,
    map: &Map,
    keys: &[Key],
    opts: &Options,
    progress: &mut dyn FnMut(&str),
) -> Result<Report, String> {
    if keys.is_empty() {
        return Err("no key to make the paths with".into());
    }
    if map.notes.is_empty() {
        return Err("the map has no notes".into());
    }
    if !(info.fps.is_finite() && info.fps >= 1.0) || info.width < 64 || info.height < 64 {
        return Err("the video's size or frame rate could not be read".into());
    }
    let threads = if opts.threads > 0 { opts.threads } else { std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).min(8) };
    let frame_ms = 1000.0 / info.fps;
    let total = info.frames.unwrap_or(opts.max_frames).min(opts.max_frames).max(1);
    // the generator samples at the render's rate, at least 60 a second
    let gen_fps = (info.fps.round() as u32).clamp(24, 240).max(60);
    let speeds: Vec<f32> = if opts.speed_mods.is_empty() { (0..=24).map(|i| (80 + 5 * i) as f32 / 100.0).collect() } else { opts.speed_mods.clone() };
    let rest = opts.rest_secs.unwrap_or(AutoplaySettings::default().rest_secs);
    let (start_ms, playback) = (opts.start_ms, opts.playback.max(0.01));
    let clock = move |frame: usize, shift_ms: f64, speed: f32| start_ms + (frame as f64 * frame_ms + shift_ms) * speed as f64 * playback;
    let make = |movement: Movement, aim: Aim, dance: DanceAmount, speed: f32, rest: f64, key: usize| -> Result<Candidate, String> {
        let settings = AutoplaySettings { movement, dance, aim, rest_secs: rest, speed_mod: speed, banner: true }.normalized();
        let run = generate(map, &settings, gen_fps, &keys[key].dice).map_err(|e| e.to_string())?;
        Ok(Candidate { settings, key, curve: Curve::of(&run) })
    };

    // 1. coarse, on a copy 480 wide
    let grid = settings_grid();
    let mut cands: Vec<Candidate> = Vec::with_capacity(speeds.len() * grid.len());
    for &speed in &speeds {
        for &(movement, aim, dance) in &grid {
            cands.push(make(movement, aim, dance, speed, rest, 0)?);
        }
    }
    let ww = info.width.min(1920);
    let wh = if ww == info.width { info.height } else { even(info.height as f64 * ww as f64 / info.width as f64) };
    let cw = ww.min(480);
    let ch = if cw == info.width { info.height } else { even(info.height as f64 * cw as f64 / info.width as f64) };
    let step_c = total.div_ceil(1000).max(3);
    let used_c = total.div_ceil(step_c);
    let tracks: Vec<Vec<(f32, f32)>> =
        cands.iter().map(|c| (0..used_c).map(|j| c.curve.at(clock(j * step_c, 0.0, c.settings.speed_mod))).collect()).collect();
    let mut riders: Vec<Rider> = Vec::with_capacity(cands.len() * 100);
    for k in 0..cands.len() {
        for i in 0..50 {
            let s = (0.070 + 0.002 * i as f32) * cw as f32;
            riders.push((k, s, -s));
            riders.push((k, s, s));
        }
    }
    progress(&format!("looking for the cursor along {} candidate paths", cands.len()));
    let half_c = (24 * cw / 480).max(6);
    let scores = ride(src, (cw, ch), step_c, half_c, &tracks, &riders, Measure::Peak, threads)?;
    let (best, &top) = scores.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).ok_or("no candidate")?;
    if top < 6.0 {
        return Ok(no_verdict("nothing in the video follows any Autoplay path of this map"));
    }
    let (_, s0c, sy0) = riders[best];
    let fy = sy0.signum();
    let mut per_path = vec![0.0f32; cands.len()];
    for (i, &(k, _, _)) in riders.iter().enumerate() {
        per_path[k] = per_path[k].max(scores[i]);
    }
    let mut order: Vec<usize> = (0..cands.len()).collect();
    order.sort_by(|&a, &b| per_path[b].total_cmp(&per_path[a]));
    let top_paths: Vec<usize> = order.into_iter().take(3).collect();

    // 2. fine: scale and time offset at full size for the three best paths
    let k = ww as f32 / 1920.0;
    let s0 = s0c * ww as f32 / cw as f32;
    let half = ((64.0 * k).round() as usize).max(8);
    let step_f = total.div_ceil(700).max(4);
    let used_f = total.div_ceil(step_f);
    let shifts: Vec<f64> = (-24..=24).map(|i| i as f64 * 4.0).collect();
    let mut tracks_f: Vec<Vec<(f32, f32)>> = Vec::new();
    let mut riders_f: Vec<Rider> = Vec::new();
    let mut meta_f: Vec<(usize, f32, f64)> = Vec::new();
    for &p in &top_paths {
        for &d in &shifts {
            let c = &cands[p];
            tracks_f.push((0..used_f).map(|j| c.curve.at(clock(j * step_f, d, c.settings.speed_mod))).collect());
            for i in -3..=3 {
                let s = s0 + i as f32 * 2.0 * k;
                riders_f.push((tracks_f.len() - 1, s, fy * s));
                meta_f.push((p, s, d));
            }
        }
    }
    progress(&format!("refining scale and timing ({})", describe(&cands[top_paths[0]].settings)));
    let scores_f = ride(src, (ww, wh), step_f, half, &tracks_f, &riders_f, Measure::Edges, threads)?;
    let best_f = scores_f.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).map(|(i, _)| i).ok_or("no candidate")?;
    let (p1, s1, d1) = meta_f[best_f];
    let speed1 = cands[p1].settings.speed_mod;
    drop((tracks_f, riders_f, tracks, riders));

    // 3. the cursor's picture and where it is in every frame
    let r_search = ((40.0 * k).round() as usize).max(4);
    let hc = half + r_search;
    let side = 2 * hc + 1;
    let step_t = total.div_ceil(3000).max(1);
    let mut crops: Vec<u8> = Vec::new();
    let mut centres: Vec<(usize, i64, i64)> = Vec::new();
    progress("following the cursor frame by frame");
    {
        let curve = &cands[p1].curve;
        src.frames(ww, wh, &mut |n, px| {
            if n >= total {
                return false;
            }
            if !n.is_multiple_of(step_t) {
                return true;
            }
            let (x, y) = curve.at(clock(n, d1, speed1));
            let cx = (ww as f32 / 2.0 + s1 * x).round() as i64;
            let cy = (wh as f32 / 2.0 + fy * s1 * y).round() as i64;
            if cx < 0 || cy < 0 || cx >= ww as i64 || cy >= wh as i64 {
                return true;
            }
            let at = crops.len();
            crops.resize(at + side * side, 0);
            for r in 0..side {
                let yy = cy - hc as i64 + r as i64;
                if yy < 0 || yy >= wh as i64 {
                    continue;
                }
                let x0 = (cx - hc as i64).max(0);
                let x1 = (cx + hc as i64 + 1).min(ww as i64);
                let dst = at + r * side + (x0 - (cx - hc as i64)) as usize;
                let from = yy as usize * ww + x0 as usize;
                crops[dst..dst + (x1 - x0) as usize].copy_from_slice(&px[from..from + (x1 - x0) as usize]);
            }
            centres.push((n, cx, cy));
            true
        })?;
    }
    let n_crops = centres.len();
    if n_crops < MIN_MOVING {
        return Ok(no_verdict("the video is too short to tell"));
    }
    // the median of the riding window: the mean keeps the trail's haze,
    // which lies behind the cursor in ever changing directions; the median
    // drops it and keeps the head
    let size0 = 2 * half + 1;
    let every = if n_crops > 1200 { 2 } else { 1 };
    let mut hist = vec![0u32; size0 * size0 * 256];
    let mut counted = 0u32;
    for i in (0..n_crops).step_by(every) {
        let crop = &crops[i * side * side..(i + 1) * side * side];
        for y in 0..size0 {
            for x in 0..size0 {
                let v = crop[(y + r_search) * side + x + r_search] as usize;
                hist[(y * size0 + x) * 256 + v] += 1;
            }
        }
        counted += 1;
    }
    let window: Vec<f32> = hist
        .chunks(256)
        .map(|h| {
            let mut seen = 0u32;
            let mut v = 0usize;
            while v < 255 {
                seen += h[v];
                if seen * 2 > counted {
                    break;
                }
                v += 1;
            }
            v as f32
        })
        .collect();
    drop(hist);
    let mut sorted = window.clone();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let (floor, peak) = (sorted[sorted.len() / 2], sorted[sorted.len() - 1]);
    if peak - floor < 6.0 {
        return Ok(no_verdict("no cursor stands out along the best path"));
    }
    // the cursor's core, its centre and its size
    let level = floor + 0.5 * (peak - floor);
    let (mut sum_x, mut sum_y, mut count) = (0usize, 0usize, 0usize);
    let (mut lo_x, mut hi_x, mut lo_y, mut hi_y) = (size0, 0usize, size0, 0usize);
    for y in 0..size0 {
        for x in 0..size0 {
            if window[y * size0 + x] > level {
                sum_x += x;
                sum_y += y;
                count += 1;
                lo_x = lo_x.min(x);
                hi_x = hi_x.max(x);
                lo_y = lo_y.min(y);
                hi_y = hi_y.max(y);
            }
        }
    }
    let rad = (hi_x - lo_x).max(hi_y - lo_y).div_ceil(2);
    let th = (rad + (4.0 * k).round() as usize).clamp(6, half - 2);
    let px0 = ((sum_x as f32 / count as f32).round() as usize).clamp(th, size0 - th - 1);
    let py0 = ((sum_y as f32 / count as f32).round() as usize).clamp(th, size0 - th - 1);
    let ts = 2 * th + 1;
    let mut tmpl: Vec<f32> = Vec::with_capacity(ts * ts);
    for y in 0..ts {
        for x in 0..ts {
            tmpl.push(window[(py0 - th + y) * size0 + px0 - th + x]);
        }
    }
    let mean = tmpl.iter().sum::<f32>() / tmpl.len() as f32;
    for v in &mut tmpl {
        *v -= mean;
    }
    let tn = tmpl.iter().map(|v| v * v).sum::<f32>().sqrt();
    if tn < 1.0 {
        return Ok(no_verdict("no cursor stands out along the best path"));
    }
    let (off_x, off_y) = (px0 as i64 - half as i64, py0 as i64 - half as i64);

    // the cursor's picture searched within the radius around the
    // prediction: normalised cross-correlation, the product by FFT, the
    // local energy from exact sums
    let m = 2 * r_search + 1;
    let ss = 2 * (r_search + th) + 1;
    let (ox, oy) = ((half as i64 - th as i64 + off_x) as usize, (half as i64 - th as i64 + off_y) as usize);
    let npx = (ts * ts) as f64;
    let plan = Fft2::new(ss, ss);
    let ft: Vec<Complex<f32>> = plan.forward(&tmpl, ts, ts).iter().map(|c| c.conj()).collect();
    let mut found: Vec<(i32, i32, f32)> = vec![(0, 0, 0.0); n_crops];
    let per = n_crops.div_ceil(threads).max(1);
    std::thread::scope(|s| {
        for (ti, out) in found.chunks_mut(per).enumerate() {
            let (crops, ft) = (&crops, &ft);
            s.spawn(move || {
                let fft = Fft2::new(ss, ss);
                let mut win = vec![0.0f32; ss * ss];
                let mut i1 = vec![0u64; (ss + 1) * (ss + 1)];
                let mut i2 = vec![0u64; (ss + 1) * (ss + 1)];
                for (oi, o) in out.iter_mut().enumerate() {
                    let crop = &crops[(ti * per + oi) * side * side..(ti * per + oi + 1) * side * side];
                    for y in 0..ss {
                        let (mut r1, mut r2) = (0u64, 0u64);
                        for x in 0..ss {
                            let v = crop[(oy + y) * side + ox + x];
                            win[y * ss + x] = v as f32 - 128.0;
                            r1 += v as u64;
                            r2 += v as u64 * v as u64;
                            i1[(y + 1) * (ss + 1) + x + 1] = i1[y * (ss + 1) + x + 1] + r1;
                            i2[(y + 1) * (ss + 1) + x + 1] = i2[y * (ss + 1) + x + 1] + r2;
                        }
                    }
                    let mut spec = fft.forward(&win, ss, ss);
                    for (a, b) in spec.iter_mut().zip(ft) {
                        *a *= *b;
                    }
                    let cc = fft.inverse(&spec);
                    let boxed = |i: &[u64], y: usize, x: usize| -> f64 {
                        (i[(y + ts) * (ss + 1) + x + ts] + i[y * (ss + 1) + x] - i[y * (ss + 1) + x + ts] - i[(y + ts) * (ss + 1) + x]) as f64
                    };
                    let mut bst = (0usize, 0usize, f32::NEG_INFINITY);
                    for y in 0..m {
                        for x in 0..m {
                            let sum = boxed(&i1, y, x);
                            // a flat patch is no cursor however it correlates
                            let energy = (boxed(&i2, y, x) - sum * sum / npx).max(npx * 4.0);
                            let v = cc[y * fft.w + x] / (tn * energy.sqrt() as f32);
                            if v > bst.2 {
                                bst = (x, y, v);
                            }
                        }
                    }
                    *o = (bst.0 as i32 - r_search as i32, bst.1 as i32 - r_search as i32, bst.2);
                }
            });
        }
    });
    drop(crops);
    let track = Track {
        n: centres.iter().map(|c| c.0).collect(),
        mx: centres.iter().zip(&found).map(|(c, f)| (c.1 + off_x + f.0 as i64) as f32).collect(),
        my: centres.iter().zip(&found).map(|(c, f)| (c.2 + off_y + f.1 as i64) as f32).collect(),
        q: found.iter().map(|f| f.2).collect(),
    };
    let mut qs = track.q.clone();
    qs.sort_by(|a, b| a.total_cmp(b));
    let match_quality = qs[qs.len() / 2];

    // 4. the verdict: every setting and every key at the speed found
    progress("comparing with every setting");
    let to1920 = 1920.0 / ww as f32;
    let at = |frame: usize, shift: f64| clock(frame, d1 + shift, speed1);
    let mut rows: Vec<(Judged, AutoplaySettings, usize)> = Vec::new();
    for key in 0..keys.len() {
        for &(movement, aim, dance) in &grid {
            let c = make(movement, aim, dance, speed1, rest, key)?;
            if let Some(j) = judge(&track, &c.curve, &at, s1, to1920) {
                rows.push((j, c.settings, c.key));
            }
        }
    }
    rows.sort_by(|a, b| b.0.on_curve.total_cmp(&a.0.on_curve));
    let Some(&(mut bj, mut bs, bk)) = rows.first() else {
        return Ok(no_verdict("the cursor hardly moves in the part of the video that was looked at"));
    };
    // the rest threshold changes the path only inside long pauses: try the
    // others for the best setting when the caller did not name one
    if opts.rest_secs.is_none() {
        for i in 0..=12 {
            let r = 2.0 + 0.5 * i as f64;
            if (r - rest).abs() < 1e-9 {
                continue;
            }
            let c = make(bs.movement, bs.aim, bs.dance, speed1, r, bk)?;
            if let Some(j) = judge(&track, &c.curve, &at, s1, to1920) {
                if j.on_curve > bj.on_curve + 0.5 {
                    bj = j;
                    bs = c.settings;
                }
            }
        }
    }
    let runner_up = rows
        .iter()
        .find(|r| (r.1.movement, r.1.aim, r.1.dance) != (bs.movement, bs.aim, bs.dance))
        .map(|r| (describe(&r.1), r.0.on_curve));
    let kind = if bj.on_curve >= FOUND_PCT {
        VerdictKind::Found
    } else if bj.on_curve >= WEAK_PCT {
        VerdictKind::Weak
    } else {
        VerdictKind::None
    };
    Ok(Report {
        kind,
        on_curve_pct: bj.on_curve,
        on_time_pct: bj.on_time,
        median_px: bj.median_px,
        frames: track.n.len(),
        moving_frames: bj.moving,
        settings: bs,
        key: keys[bk].label.clone(),
        scale: s1,
        time_offset_ms: (d1 + bj.lag_ms as f64) as f32,
        match_quality,
        runner_up,
        note: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rhythia_formats::map::Note;

    /// A drawn video: dark picture, a few bright things that stand still,
    /// and a soft bright disc where `pos` says the cursor is.
    struct Drawn {
        w: usize,
        pos: Vec<(f32, f32)>,
    }

    impl FrameSource for Drawn {
        fn frames(&mut self, w: usize, h: usize, f: &mut dyn FnMut(usize, &[u8]) -> bool) -> Result<(), String> {
            let k = w as f32 / self.w as f32;
            let r = 4.5 * k;
            for (n, &(px, py)) in self.pos.iter().enumerate() {
                let mut img = vec![16u8; w * h];
                // a HUD bar and two blocks that never move
                for y in h / 20..h / 20 + 4 {
                    img[y * w + w / 4..y * w + 3 * w / 4].fill(180);
                }
                for (bx, by) in [(w / 3, h / 3), (2 * w / 3, 2 * h / 3)] {
                    for y in by..by + 6 {
                        img[y * w + bx..y * w + bx + 6].fill(140);
                    }
                }
                let (cx, cy) = (px * k, py * k);
                let (x0, x1) = ((cx - r - 2.0).max(0.0) as usize, ((cx + r + 3.0) as usize).min(w));
                let (y0, y1) = ((cy - r - 2.0).max(0.0) as usize, ((cy + r + 3.0) as usize).min(h));
                for y in y0..y1 {
                    for x in x0..x1 {
                        let d = (x as f32 - cx).hypot(y as f32 - cy);
                        let v = ((r + 1.0 - d).clamp(0.0, 1.0) * 220.0) as u8;
                        img[y * w + x] = img[y * w + x].max(16 + v);
                    }
                }
                if !f(n, &img) {
                    break;
                }
            }
            Ok(())
        }
    }

    fn a_map(notes: usize) -> Map {
        let mut map = Map::default();
        for i in 0..notes {
            map.notes.push(Note { time_ms: 400 + i as i64 * 230, x: (i * 37 % 250) as f32 / 100.0, y: (i * 53 % 250) as f32 / 100.0 });
        }
        map
    }

    fn dev_key() -> Key {
        let seed = crate::mark::seed_from_hex(crate::mark::DEV_KEY_HEX).unwrap();
        Key { label: "dev".into(), dice: crate::mark::pattern(&seed) }
    }

    const W: usize = 640;
    const H: usize = 360;
    const FPS: f64 = 60.0;
    const SCALE: f32 = 66.0;

    /// The video of a run: frame n shows the cursor at map time n / fps
    /// times `speed`.
    fn video_of(run: &Replay, frames: usize, speed: f64) -> Drawn {
        let curve = Curve::of(run);
        let pos = (0..frames)
            .map(|n| {
                let (x, y) = curve.at(n as f64 * 1000.0 / FPS * speed);
                (W as f32 / 2.0 + SCALE * x, H as f32 / 2.0 - SCALE * y)
            })
            .collect();
        Drawn { w: W, pos }
    }

    fn info(frames: usize) -> VideoInfo {
        VideoInfo { width: W, height: H, fps: FPS, frames: Some(frames) }
    }

    #[test]
    fn an_autoplay_video_is_found_with_its_setting() {
        let map = a_map(60);
        let key = dev_key();
        let settings = AutoplaySettings { aim: Aim::Flow, ..AutoplaySettings::default() };
        let run = generate(&map, &settings, 60, &key.dice).unwrap();
        let mut video = video_of(&run, 840, 1.0);
        let opts = Options { speed_mods: vec![1.0], ..Options::default() };
        let r = check(&mut video, &info(840), &map, &[key], &opts, &mut |_| {}).unwrap();
        eprintln!("{r:?}");
        assert_eq!(r.kind, VerdictKind::Found, "{r:?}");
        assert!(r.on_curve_pct > 85.0, "{r:?}");
        assert_eq!((r.settings.movement, r.settings.aim), (Movement::Dance, Aim::Flow), "{r:?}");
        assert!((r.scale - SCALE).abs() < 3.0, "{r:?}");
    }

    #[test]
    fn a_play_through_the_same_notes_on_its_own_way_is_not() {
        // a player: straight from note to note with an eased pace, every
        // note hit a little off its centre
        let map = a_map(60);
        let mut stops: Vec<(f64, f32, f32)> = vec![(0.0, 0.0, 0.0)];
        for (i, n) in map.notes.iter().enumerate() {
            let (x, y) = crate::scene::grid_to_world(n.x, n.y);
            let off = |salt: usize| (((i * 7919 + salt * 104729) % 1000) as f32 / 1000.0 - 0.5) * 0.3;
            stops.push((n.time_ms as f64, x + off(1), y + off(2)));
        }
        let pos = (0..840)
            .map(|n| {
                let t = n as f64 * 1000.0 / FPS;
                let i = stops.partition_point(|s| s.0 <= t).clamp(1, stops.len() - 1);
                let (a, b) = (stops[i - 1], stops[i]);
                let u = (((t - a.0) / (b.0 - a.0)).clamp(0.0, 1.0)) as f32;
                let e = u * u * (3.0 - 2.0 * u);
                (W as f32 / 2.0 + SCALE * (a.1 + (b.1 - a.1) * e), H as f32 / 2.0 - SCALE * (a.2 + (b.2 - a.2) * e))
            })
            .collect();
        let mut video = Drawn { w: W, pos };
        let opts = Options { speed_mods: vec![1.0], ..Options::default() };
        let r = check(&mut video, &info(840), &map, &[dev_key()], &opts, &mut |_| {}).unwrap();
        eprintln!("{r:?}");
        assert_eq!(r.kind, VerdictKind::None, "{r:?}");
    }

    #[test]
    fn the_speed_mod_is_found_among_several() {
        let map = a_map(60);
        let key = dev_key();
        let settings = AutoplaySettings { speed_mod: 1.25, movement: Movement::Normal, ..AutoplaySettings::default() };
        let run = generate(&map, &settings, 60, &key.dice).unwrap();
        let mut video = video_of(&run, 660, 1.25);
        let opts = Options { speed_mods: vec![1.0, 1.25, 1.5], ..Options::default() };
        let r = check(&mut video, &info(660), &map, &[key], &opts, &mut |_| {}).unwrap();
        eprintln!("{r:?}");
        assert_eq!(r.kind, VerdictKind::Found, "{r:?}");
        assert!((r.settings.speed_mod - 1.25).abs() < 1e-6, "{r:?}");
        assert_eq!(r.settings.movement, Movement::Normal, "{r:?}");
    }

    /// The verdict's last step by itself: a cursor found exactly where the
    /// path is counts fully; one that swings 25 pixels around it (a hand
    /// near the same line) hardly counts.
    #[test]
    fn a_cursor_near_the_path_is_not_on_it() {
        let map = a_map(60);
        let key = dev_key();
        let run = generate(&map, &AutoplaySettings::default(), 60, &key.dice).unwrap();
        let curve = Curve::of(&run);
        let at = |frame: usize, shift: f64| frame as f64 * 1000.0 / FPS + shift;
        let track_with = |wobble: f32| {
            let mut t = Track { n: Vec::new(), mx: Vec::new(), my: Vec::new(), q: Vec::new() };
            for n in 0..800usize {
                let (x, y) = curve.at(at(n, 0.0));
                let phase = n as f32 * 0.21;
                t.n.push(n);
                t.mx.push(960.0 + 200.0 * x + wobble * phase.sin());
                t.my.push(540.0 - 200.0 * y + wobble * (phase * 1.3).cos());
                t.q.push(0.9);
            }
            t
        };
        let exact = judge(&track_with(0.0), &curve, &at, 200.0, 1.0).unwrap();
        assert!(exact.on_curve > 99.0 && exact.median_px < 0.5, "{} {}", exact.on_curve, exact.median_px);
        let near = judge(&track_with(25.0), &curve, &at, 200.0, 1.0).unwrap();
        assert!(near.on_curve < WEAK_PCT, "{}", near.on_curve);
    }

    /// The path is the proof, so it has to stay where it is: these are
    /// points of the eight paths of one map. When this fails the Autoplay
    /// movement changed, and videos made before the change no longer
    /// verify. Up to the first release that ships this checker the numbers
    /// may move together with the movement (no released video has these
    /// paths yet). From that release on a changed movement means: keep the
    /// old generator for the checker to try as well (see "For maintainers"
    /// in docs/AUTOPLAY-VERIFICATION.md), and only then add the new
    /// numbers next to these.
    #[test]
    fn a_released_path_stays_where_it_is() {
        let map = a_map(40);
        let key = dev_key();
        let mut got: Vec<[f32; 2]> = Vec::new();
        for (movement, aim, dance) in settings_grid() {
            let settings = AutoplaySettings { movement, aim, dance, ..AutoplaySettings::default() };
            let curve = Curve::of(&generate(&map, &settings, 60, &key.dice).unwrap());
            for t in [1000.0, 2350.0, 4100.0, 6900.0, 8800.0] {
                let (x, y) = curve.at(t);
                got.push([x, y]);
            }
        }
        #[rustfmt::skip]
        const WANT: [[f32; 2]; 40] = [
            [-0.071, -0.446], [-0.436, -1.125], [-0.049, 0.001], [-0.944, -0.858], [-0.058, -0.993],
            [-0.129, -0.482], [-0.473, -1.151], [-0.021, 0.009], [-0.978, -0.849], [-0.092, -1.014],
            [-0.159, -0.501], [-0.498, -1.158], [-0.002, 0.025], [-1.026, -0.808], [-0.116, -1.019],
            [-0.035, -0.383], [-0.363, -0.993], [-0.048, -0.026], [-0.543, -0.826], [0.013, -0.857],
            [-0.071, -0.446], [-0.436, -1.125], [-0.049, 0.001], [-0.944, -0.858], [-0.058, -0.993],
            [-0.129, -0.482], [-0.473, -1.151], [-0.021, 0.009], [-0.978, -0.849], [-0.092, -1.014],
            [-0.159, -0.501], [-0.498, -1.158], [-0.002, 0.025], [-1.026, -0.808], [-0.116, -1.019],
            [-0.016, -0.409], [-0.399, -1.091], [-0.067, 0.001], [-0.907, -0.865], [-0.025, -0.962],
        ];
        if std::env::var_os("RHYTHR_PRINT_PATH_POINTS").is_some() {
            for row in got.chunks(5) {
                let cells: Vec<String> = row.iter().map(|p| format!("[{:.3}, {:.3}]", p[0], p[1])).collect();
                eprintln!("            {},", cells.join(", "));
            }
        }
        for (i, (g, w)) in got.iter().zip(WANT).enumerate() {
            assert!(
                (g[0] - w[0]).abs() < 2e-3 && (g[1] - w[1]).abs() < 2e-3,
                "path {} point {}: {g:?} is not {w:?}. The Autoplay movement changed: read the comment on this test before touching the numbers",
                i / 5,
                i % 5
            );
        }
    }

    #[test]
    fn a_curve_is_read_between_its_frames_and_held_at_its_ends() {
        let c = Curve { t: vec![0.0, 10.0, 20.0], x: vec![0.0, 1.0, 1.0], y: vec![0.0, 0.0, 2.0] };
        assert_eq!(c.at(-5.0), (0.0, 0.0));
        assert_eq!(c.at(5.0), (0.5, 0.0));
        assert_eq!(c.at(15.0), (1.0, 1.0));
        assert_eq!(c.at(99.0), (1.0, 2.0));
    }

    #[test]
    fn a_fit_that_folds_the_path_onto_one_spot_is_not_believed() {
        let flat = Fit { cx: [320.0, 0.01, 0.0, 0.0, 0.0, 0.0], cy: [180.0, 0.0, 0.01, 0.0, 0.0, 0.0] };
        let real = Fit { cx: [320.0, 66.0, 0.5, 0.0, 0.0, 0.0], cy: [180.0, 0.3, -66.0, 0.0, 0.0, 0.0] };
        assert!(!plausible(&flat, 66.0));
        assert!(plausible(&real, 66.0));
    }
}
