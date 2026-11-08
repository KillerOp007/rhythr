//! Planning the cursor between the waypoints of a run and sampling it by
//! time. Every constant that shapes the look is named here.
use super::{AutoplaySettings, Movement, CRUISE};
use rhythia_sim::hitreg::{BASE_HIT_WINDOW_MS, HITBOX_HALF};

/// Below this gap in real seconds a same-spot pair is a hold, above it a
/// boomerang.
pub const HOLD_SECS: f64 = 0.35;
/// Two waypoints closer than this are the same spot (a quarter of the hit
/// area's width).
pub const SAME_SPOT: f32 = 2.0 * HITBOX_HALF * 0.25;
/// A boomerang reaches `CRUISE * t / BOOMERANG_DIVISOR` out before turning.
pub const BOOMERANG_DIVISOR: f64 = 2.2;
/// The return arc of a boomerang is offset by this fraction of the reach, so
/// the two arcs differ.
pub const BOOMERANG_SPREAD: f32 = 0.35;
/// A rest leaves for the centre over at most this many real seconds.
pub const REST_OUT_MAX_SECS: f64 = 1.0;
/// Leaving the centre for the next note takes at least this long.
pub const REST_IN_MIN_SECS: f64 = 0.25;
/// Fraction of the chord used for the belly's reach limit.
pub const L_MAX_FACTOR: f32 = 1.5;
/// The dance's belly per unit of chord: real maps leave about 150 ms
/// between notes, so an arc that waited for time to spare never came. The
/// control points sit this fraction of the chord off it (the curve itself
/// reaches three quarters of that); time to spare widens it, the amount
/// scales it, the barrier has the last word.
pub const BELLY_PER_UNIT: f32 = 0.3;
/// Notes closer than this in real seconds (faster than eight per second)
/// are a stream: planned as one sweep with the jump into it and out of it,
/// no dance belly. Real maps place their jumps at 150 ms and more; their
/// 1/16 and 1/32 runs sit well below this.
pub const STREAM_GAP_SECS: f64 = 0.125;
/// A sweep piece's control points sit this fraction of the chord from its
/// ends, along the direction the neighbours give (the Catmull-Rom third);
/// collinear notes make it a straight line, a corner a rounded one.
pub const STREAM_TANGENT: f32 = 1.0 / 3.0;
/// Players do not turn on the centre of a stream note, they pass beside
/// it (a quarter of a unit in the median, measured on real replays). An
/// inner note of a sweep is passed at the cubic B-spline knot position, the
/// neighbours' pull `(P[k-1] - 2 P[k] + P[k+1]) / 6`, capped at this
/// distance from the centre: nothing on a line, 0.24 inward on a square
/// corner of unit hops, the cap on a reversal. The hit area is 0.57 wide.
pub const CORNER_CUT_MAX: f32 = 0.3;
/// A reversal inside a sweep (back and forth on a line: a turn sharper
/// than this cosine, 162 degrees) is taken as a loop: the cursor comes
/// within `TIP_CUT` of the note and turns around it on a small arc, the
/// way a hand oscillating at a 1/32 does, instead of stopping dead and
/// going back. Consecutive loops alternate sides, so a jack becomes a
/// narrow ellipse. Anything less sharp is an ordinary corner, rounded
/// through the neighbours' direction.
pub const TIP_COS: f32 = -0.95;
pub const TIP_CUT: f32 = 0.1;
/// Along a sweep the cursor may run this far ahead of or behind a note at
/// its chart time (the tube of the taut string), so the speed stays steady
/// where the hops are equal and changes only where a hop forces it. With
/// the corner cut and the ramps this keeps the cursor inside the hit area
/// at the note's time, where the hit flag stays.
pub const TUBE: f32 = 0.18;
/// The tube is also never more than this much of the slower neighbouring
/// hop's time, so the slack is a timing slack a player would have too.
pub const SLACK_MS: f64 = 25.0;
/// A speed change inside a sweep (a faster rhythm, the jump into the
/// stream) is spread over up to this many map ms around the point where
/// the string changes its slope, instead of switching between two frames.
pub const RAMP_MS: f64 = 150.0;
/// A ramp moves the cursor off the string's arc by at most this much (a
/// ramp of half-width `h` between speeds `v0` and `v1` is `|v1 - v0| h / 4`
/// off at its centre); a ramp that would move it more is made shorter.
pub const RAMP_DEV: f32 = 0.04;
/// Margin the flag frame keeps to the hit area's edge, per axis.
pub const HIT_MARGIN: f32 = 0.03;
// A swept note is passed at most CORNER_CUT_MAX off its centre and, at its
// chart time, at most TUBE plus RAMP_DEV along the path from that point:
// the sum must stay inside the hit area with the margin, or a tuning
// breaks hits.
const _: () = assert!(CORNER_CUT_MAX + TUBE + RAMP_DEV < HITBOX_HALF - HIT_MARGIN);
/// Samples per segment for the arc-length table.
const ARC_SAMPLES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub t_ms: f64,
    pub x: f32,
    pub y: f32,
    /// The cursor must be exactly here at exactly this time: a point that
    /// serves two notes at once (a chord's shared spot) has no room to
    /// spare, so a sweep neither cuts it nor runs early or late through it.
    pub exact: bool,
}

/// A cubic Bezier covering `[t0, t1]`, sampled at even pace along its arc.
#[derive(Clone, Debug)]
pub struct Segment {
    pub t0: f64,
    pub t1: f64,
    pub kind: SegmentKind,
    pub p: [(f32, f32); 4],
    /// Cumulative arc length at `ARC_SAMPLES + 1` parameter values.
    arc: Vec<f32>,
    /// The pace along the piece as `(ms from t0, speed in arc units per
    /// ms)` knots, speed linear between them; empty means an even pace.
    pace: Vec<(f64, f32)>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Hold,
    Straight,
    Curve,
    Boomerang,
    RestOut,
    RestIn,
    /// A piece of a stream's sweep between two passed notes.
    Sweep,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub segments: Vec<Segment>,
    /// When each waypoint's hit flag goes, in map ms: its chart time, with
    /// one exception inside a sweep, where the cursor is not yet inside the
    /// note's hit area at that time (the constants rule it out, this is the
    /// safety net): then the first covered moment after it, within the hit
    /// window. Never earlier than the chart time.
    pub visits: Vec<f64>,
}

fn bezier(p: &[(f32, f32); 4], u: f32) -> (f32, f32) {
    let v = 1.0 - u;
    let a = v * v * v;
    let b = 3.0 * v * v * u;
    let c = 3.0 * v * u * u;
    let d = u * u * u;
    (a * p[0].0 + b * p[1].0 + c * p[2].0 + d * p[3].0, a * p[0].1 + b * p[1].1 + c * p[2].1 + d * p[3].1)
}

impl Segment {
    fn new(t0: f64, t1: f64, kind: SegmentKind, p: [(f32, f32); 4]) -> Segment {
        let mut arc = Vec::with_capacity(ARC_SAMPLES + 1);
        let mut acc = 0.0f32;
        let mut prev = p[0];
        arc.push(0.0);
        for i in 1..=ARC_SAMPLES {
            let q = bezier(&p, i as f32 / ARC_SAMPLES as f32);
            acc += ((q.0 - prev.0).powi(2) + (q.1 - prev.1).powi(2)).sqrt();
            arc.push(acc);
            prev = q;
        }
        Segment { t0, t1, kind, p, arc, pace: Vec::new() }
    }

    /// The fraction of the arc covered `tau` ms into the piece under its
    /// pace knots: the integral of the speed, normalised by the whole.
    fn pace_frac(&self, tau: f64) -> f32 {
        let mut so_far = 0.0f64;
        let mut total = 0.0f64;
        for w in self.pace.windows(2) {
            let span = (w[1].0 - w[0].0).max(1e-9);
            let part = |t_end: f64| {
                let d = (t_end - w[0].0).clamp(0.0, span);
                let v_end = w[0].1 as f64 + (w[1].1 - w[0].1) as f64 * (d / span);
                d * (w[0].1 as f64 + v_end) / 2.0
            };
            total += part(w[1].0);
            so_far += part(tau);
        }
        if total <= 1e-12 {
            return 0.0;
        }
        (so_far / total).clamp(0.0, 1.0) as f32
    }

    fn hold(t0: f64, t1: f64, at: (f32, f32)) -> Segment {
        Segment::new(t0, t1, SegmentKind::Hold, [at; 4])
    }

    pub fn length(&self) -> f32 {
        *self.arc.last().unwrap_or(&0.0)
    }

    /// Position at `t_ms`: the parameter whose arc length is the time
    /// fraction of the total, so the pace along the segment is even.
    pub fn at(&self, t_ms: f64) -> (f32, f32) {
        if self.kind == SegmentKind::Hold || self.t1 <= self.t0 {
            return self.p[0];
        }
        let frac = if self.pace.len() >= 2 {
            self.pace_frac(t_ms - self.t0)
        } else {
            ((t_ms - self.t0) / (self.t1 - self.t0)).clamp(0.0, 1.0) as f32
        };
        let total = self.length();
        if total <= 1e-6 {
            return self.p[0];
        }
        let want = frac * total;
        let i = match self.arc.binary_search_by(|v| v.partial_cmp(&want).unwrap()) {
            Ok(i) => i,
            Err(i) => i,
        };
        if i == 0 {
            return self.p[0];
        }
        if i >= self.arc.len() {
            return self.p[3];
        }
        let (a, b) = (self.arc[i - 1], self.arc[i]);
        let local = if b > a { (want - a) / (b - a) } else { 0.0 };
        let u = ((i - 1) as f32 + local) / ARC_SAMPLES as f32;
        bezier(&self.p, u)
    }
}

/// Everything the planner needs to know about the field.
#[derive(Clone, Copy, Debug)]
pub struct Field {
    pub bound: f32,
}

impl Field {
    pub fn clamp(&self, p: (f32, f32)) -> (f32, f32) {
        (p.0.clamp(-self.bound, self.bound), p.1.clamp(-self.bound, self.bound))
    }

    /// How far one can go from `from` along the unit direction `dir` before
    /// touching the barrier.
    pub fn room(&self, from: (f32, f32), dir: (f32, f32)) -> f32 {
        let mut r = f32::INFINITY;
        for (c, d) in [(from.0, dir.0), (from.1, dir.1)] {
            if d.abs() > 1e-6 {
                let edge = if d > 0.0 { self.bound } else { -self.bound };
                r = r.min((edge - c) / d);
            }
        }
        r.max(0.0)
    }
}

fn dist(a: (f32, f32), b: (f32, f32)) -> f32 {
    ((a.0 - b.0).powi(2) + (a.1 - b.1).powi(2)).sqrt()
}

/// A cubic from `a` to `b` with the inner control points pushed off the
/// chord by `belly` (signed, along the chord's left normal).
fn chord_curve(t0: f64, t1: f64, a: (f32, f32), b: (f32, f32), belly: f32, kind: SegmentKind) -> Segment {
    let d = dist(a, b).max(1e-6);
    let n = (-(b.1 - a.1) / d, (b.0 - a.0) / d);
    let p1 = (a.0 + (b.0 - a.0) / 3.0 + n.0 * belly, a.1 + (b.1 - a.1) / 3.0 + n.1 * belly);
    let p2 = (a.0 + 2.0 * (b.0 - a.0) / 3.0 + n.0 * belly, a.1 + 2.0 * (b.1 - a.1) / 3.0 + n.1 * belly);
    Segment::new(t0, t1, kind, [a, p1, p2, b])
}

/// A piece of a sweep from `a` to `b` whose end tangents follow the
/// neighbours (a Catmull-Rom spline on the passed points): `da` and `db`
/// are the directions at `a` and `b`. The control points sit
/// `STREAM_TANGENT` of the chord along them, so collinear points give a
/// straight line and a corner a rounded one. A direction that says nothing
/// falls back to the chord. The times and the pace are filled in after
/// the taut string.
fn sweep_piece(a: (f32, f32), b: (f32, f32), da: (f32, f32), db: (f32, f32)) -> Segment {
    let d = dist(a, b).max(1e-6);
    let reach = d * STREAM_TANGENT;
    let chord = ((b.0 - a.0) / d, (b.1 - a.1) / d);
    let along = |v: (f32, f32)| {
        let len = (v.0 * v.0 + v.1 * v.1).sqrt();
        if len > 1e-9 {
            (v.0 / len * reach, v.1 / len * reach)
        } else {
            (chord.0 * reach, chord.1 * reach)
        }
    };
    let oa = along(da);
    let ob = along(db);
    Segment::new(0.0, 0.0, SegmentKind::Sweep, [a, (a.0 + oa.0, a.1 + oa.1), (b.0 - ob.0, b.1 - ob.1), b])
}

/// Real seconds of a map gap under the speed mod.
fn real_secs(ms: f64, settings: &AutoplaySettings) -> f64 {
    ms / 1000.0 / f64::from(settings.speed_mod.max(0.01))
}

/// What a gap between two waypoints is, before any planning.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gap {
    /// No time passes (a duplicate stamp).
    Degenerate,
    /// Long enough for the cursor to rest in the centre.
    Rest,
    /// The same spot again, quickly: a hold.
    Still,
    /// The same spot again, with time: a boomerang in the dance, a hold
    /// otherwise.
    Boomerang,
    /// A real move within `STREAM_GAP_SECS`.
    Stream,
    /// An ordinary jump.
    Jump,
}

/// The start of the run: the centre at time 0.
const START: Waypoint = Waypoint { t_ms: 0.0, x: 0.0, y: 0.0, exact: true };

/// The waypoint before index `k` (the start for `k == 0`).
fn before(points: &[Waypoint], k: usize) -> Waypoint {
    if k == 0 {
        START
    } else {
        points[k - 1]
    }
}

fn classify(points: &[Waypoint], k: usize, settings: &AutoplaySettings) -> Gap {
    let (p, w) = (before(points, k), points[k]);
    if w.t_ms <= p.t_ms {
        return Gap::Degenerate;
    }
    let t = real_secs(w.t_ms - p.t_ms, settings);
    let d = dist((p.x, p.y), (w.x, w.y));
    if t >= settings.rest_secs {
        Gap::Rest
    } else if d < SAME_SPOT {
        if t >= HOLD_SECS {
            Gap::Boomerang
        } else {
            Gap::Still
        }
    } else if t <= STREAM_GAP_SECS {
        Gap::Stream
    } else {
        Gap::Jump
    }
}

/// The sweep runs: each is a range of segment indices `a..=b` (segment `k`
/// ends at waypoint `k`) covering a stream, the holds inside it, a single
/// jump between two streams, and the one jump into it and out of it when
/// there is one. Runs never overlap.
fn sweep_runs(gaps: &[Gap]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let n = gaps.len();
    let mut k = 0;
    while k < n {
        if gaps[k] != Gap::Stream {
            k += 1;
            continue;
        }
        // the core: stream gaps, holds between them, and a lone jump that
        // leads straight into the next stream
        let mut b = k;
        loop {
            let mut j = b + 1;
            while j < n && gaps[j] == Gap::Still {
                j += 1;
            }
            if j < n && gaps[j] == Gap::Stream {
                b = j;
                continue;
            }
            if j < n && gaps[j] == Gap::Jump {
                let mut j2 = j + 1;
                while j2 < n && gaps[j2] == Gap::Still {
                    j2 += 1;
                }
                if j2 < n && gaps[j2] == Gap::Stream {
                    b = j2;
                    continue;
                }
            }
            break;
        }
        let mut a = k;
        let taken = runs.last().map(|r| r.1 + 1).unwrap_or(0);
        if a > taken && gaps[a - 1] == Gap::Jump {
            a -= 1;
        }
        if b + 1 < n && gaps[b + 1] == Gap::Jump {
            b += 1;
        }
        runs.push((a, b));
        k = b + 1;
    }
    runs
}

/// The taut string through a tube: the shortest monotone path in the
/// (time, arc) plane that passes within `[lo[k], hi[k]]` at every knot
/// time, with the two ends fixed (`lo == hi` there). It is piecewise
/// linear, so the speed is constant between the knots where a wall forced
/// a change. Returns the breakpoints as `(knot, arc)`, first `(0, lo[0])`
/// and last `(n - 1, lo[n - 1])`.
fn taut_string(times: &[f64], lo: &[f32], hi: &[f32]) -> Vec<(usize, f32)> {
    let n = times.len();
    let mut out = vec![(0usize, lo[0])];
    let (mut anchor, mut s_anchor) = (0usize, lo[0]);
    while anchor + 1 < n {
        let t_a = times[anchor];
        let (mut slope_lo, mut slope_hi) = (f32::NEG_INFINITY, f32::INFINITY);
        let (mut lo_at, mut hi_at) = (anchor, anchor);
        let mut next: Option<(usize, f32)> = None;
        for j in (anchor + 1)..n {
            let dt = ((times[j] - t_a) as f32).max(1e-6);
            let (want_lo, want_hi) = ((lo[j] - s_anchor) / dt, (hi[j] - s_anchor) / dt);
            if want_lo > slope_hi {
                // the floor rose above a ceiling seen before: the string
                // touches that ceiling
                next = Some((hi_at, hi[hi_at]));
                break;
            }
            if want_hi < slope_lo {
                next = Some((lo_at, lo[lo_at]));
                break;
            }
            if want_lo > slope_lo {
                slope_lo = want_lo;
                lo_at = j;
            }
            if want_hi < slope_hi {
                slope_hi = want_hi;
                hi_at = j;
            }
        }
        let (k, s) = next.unwrap_or((n - 1, lo[n - 1]));
        out.push((k, s));
        anchor = k;
        s_anchor = s;
    }
    out
}

/// The speed profile of a run: speed in arc units per ms, linear between
/// the knots `(time, speed)`, with the arc at the first knot given.
struct Profile {
    knots: Vec<(f64, f32)>,
    arc0: f32,
}

impl Profile {
    /// The string's piecewise-constant speed with every step replaced by a
    /// ramp of up to `RAMP_MS` centred on the breakpoint (a symmetric ramp
    /// covers the same distance as the step, so the arc at the knots away
    /// from it is untouched).
    fn smoothed(string: &[(usize, f32)], times: &[f64], exact_times: &[f64]) -> Profile {
        let bp: Vec<(f64, f32)> = string.iter().map(|&(k, s)| (times[k], s)).collect();
        let speeds: Vec<f32> = bp.windows(2).map(|w| ((w[1].1 - w[0].1) / ((w[1].0 - w[0].0).max(1e-6)) as f32).max(0.0)).collect();
        let mut knots: Vec<(f64, f32)> = vec![(bp[0].0, speeds[0])];
        for i in 1..speeds.len() {
            let t_b = bp[i].0;
            let mut h = (RAMP_MS / 2.0).min((t_b - bp[i - 1].0) / 2.0).min((bp[i + 1].0 - t_b) / 2.0);
            // a ramp never reaches across an exact knot, and none is
            // centred on one: the arc there must be the string's own
            for &te in exact_times {
                h = h.min((te - t_b).abs());
            }
            // and it never moves the cursor more than RAMP_DEV off the arc
            let dv = (speeds[i] - speeds[i - 1]).abs();
            if dv > 1e-9 {
                h = h.min(f64::from(RAMP_DEV * 4.0 / dv));
            }
            // no room for a ramp: the step stays a step (two knots at one
            // time), never a slope drawn across to the next breakpoint
            let h = h.max(0.0);
            knots.push((t_b - h, speeds[i - 1]));
            knots.push((t_b + h, speeds[i]));
        }
        knots.push((bp[bp.len() - 1].0, speeds[speeds.len() - 1]));
        Profile { knots, arc0: bp[0].1 }
    }

    /// Speed at `t`.
    fn speed(&self, t: f64) -> f32 {
        let k = &self.knots;
        if t <= k[0].0 {
            return k[0].1;
        }
        for w in k.windows(2) {
            if t <= w[1].0 {
                let f = ((t - w[0].0) / (w[1].0 - w[0].0).max(1e-9)) as f32;
                return w[0].1 + (w[1].1 - w[0].1) * f;
            }
        }
        k[k.len() - 1].1
    }

    /// Arc at `t`.
    fn arc(&self, t: f64) -> f32 {
        let k = &self.knots;
        let mut s = self.arc0 as f64;
        for w in k.windows(2) {
            if t <= w[0].0 {
                break;
            }
            let span = (w[1].0 - w[0].0).max(1e-9);
            let d = (t - w[0].0).min(span);
            let v_end = w[0].1 as f64 + (w[1].1 - w[0].1) as f64 * (d / span);
            s += d * (w[0].1 as f64 + v_end) / 2.0;
        }
        s as f32
    }

    /// The first time at which the arc reaches `want`, found by bisection
    /// on the monotone arc (the profile never runs backwards).
    fn time_of(&self, want: f32, lo: f64, hi: f64) -> f64 {
        let (mut a, mut b) = (lo, hi);
        for _ in 0..48 {
            let m = (a + b) / 2.0;
            if self.arc(m) < want {
                a = m;
            } else {
                b = m;
            }
        }
        (a + b) / 2.0
    }
}

/// Plans one sweep run over segments `a..=b`: the waypoint before `a` and
/// the waypoints `a..=b` are the knots, passed on one smooth curve, the
/// inner ones beside their centres, at a speed that the taut string keeps
/// steady wherever the notes allow and that ramps where it must change.
/// Pushes the pieces and fills the visits of the inner waypoints.
fn plan_run(points: &[Waypoint], run: (usize, usize), settings: &AutoplaySettings, field: &Field, dice: &[f32], segs: &mut Vec<Segment>, visits: &mut [f64]) {
    let (a, b) = run;
    let m = b - a + 1;
    let knot = |j: usize| -> Waypoint {
        if j == 0 {
            before(points, a)
        } else {
            points[a + j - 1]
        }
    };
    let times: Vec<f64> = (0..=m).map(|j| knot(j).t_ms).collect();
    let centres: Vec<(f32, f32)> = (0..=m)
        .map(|j| {
            let w = knot(j);
            (w.x, w.y)
        })
        .collect();
    let exact: Vec<bool> = (0..=m).map(|j| j == 0 || j == m || knot(j).exact).collect();
    // the passed points: inner knots beside their centres (a reversal
    // tip close to it), the ends and the exact ones on the spot
    let mut q = centres.clone();
    let mut tip = vec![false; m + 1];
    for j in 1..m {
        let (p, c, n) = (centres[j - 1], centres[j], centres[j + 1]);
        let (u, w) = ((c.0 - p.0, c.1 - p.1), (n.0 - c.0, n.1 - c.1));
        let (lu, lw) = ((u.0 * u.0 + u.1 * u.1).sqrt(), (w.0 * w.0 + w.1 * w.1).sqrt());
        // (the run's start in the centre is not a note to turn around)
        tip[j] = (j > 1 || a > 0) && lu > 1e-6 && lw > 1e-6 && (u.0 * w.0 + u.1 * w.1) / (lu * lw) < TIP_COS;
        let cap = if exact[j] { 0.0 } else if tip[j] { TIP_CUT } else { CORNER_CUT_MAX };
        let mut off = ((p.0 - 2.0 * c.0 + n.0) / 6.0, (p.1 - 2.0 * c.1 + n.1) / 6.0);
        let len = (off.0 * off.0 + off.1 * off.1).sqrt();
        if len > cap {
            off = (off.0 * cap / len, off.1 * cap / len);
        }
        q[j] = field.clamp((c.0 + off.0, c.1 + off.1));
    }
    // the direction at each knot: through the neighbours, or across the
    // line at a tip, alternating sides so a jack becomes an ellipse (the
    // dice pick the first side)
    let mut dir: Vec<(f32, f32)> = vec![(0.0, 0.0); m + 1];
    let mut side_now = side(dice, a);
    for j in 1..m {
        if tip[j] {
            // the line's normal in one fixed orientation, so consecutive
            // tips on the same line alternate sides
            let u = (q[j].0 - q[j - 1].0, q[j].1 - q[j - 1].1);
            let mut n = (-u.1, u.0);
            if n.1 < 0.0 || (n.1 == 0.0 && n.0 < 0.0) {
                n = (-n.0, -n.1);
            }
            dir[j] = (n.0 * side_now, n.1 * side_now);
            side_now = -side_now;
        } else {
            dir[j] = (q[j + 1].0 - q[j - 1].0, q[j + 1].1 - q[j - 1].1);
        }
    }
    // the pieces and the arc length at each knot
    let mut pieces: Vec<Segment> = Vec::with_capacity(m);
    let mut arc = vec![0.0f32; m + 1];
    for j in 0..m {
        let chord = (q[j + 1].0 - q[j].0, q[j + 1].1 - q[j].1);
        let da = if j == 0 { chord } else { dir[j] };
        let db = if j + 1 == m { chord } else { dir[j + 1] };
        let piece = sweep_piece(q[j], q[j + 1], da, db);
        arc[j + 1] = arc[j] + piece.length();
        pieces.push(piece);
    }
    // the tube: TUBE along the arc, but never more than SLACK_MS of the
    // slower neighbouring hop's time, so a slow jump into the stream cannot
    // arrive a tenth of a second early; nothing at the ends and the exact
    // knots
    let mut lo = arc.clone();
    let mut hi = arc.clone();
    for j in 1..m {
        if exact[j] {
            continue;
        }
        let v_in = (arc[j] - arc[j - 1]) / ((times[j] - times[j - 1]).max(1e-6)) as f32;
        let v_out = (arc[j + 1] - arc[j]) / ((times[j + 1] - times[j]).max(1e-6)) as f32;
        let tube = TUBE.min(SLACK_MS as f32 * v_in.min(v_out));
        lo[j] = arc[j] - tube;
        hi[j] = arc[j] + tube;
    }
    let string = taut_string(&times, &lo, &hi);
    let exact_times: Vec<f64> = (1..m).filter(|&j| exact[j]).map(|j| times[j]).collect();
    let profile = Profile::smoothed(&string, &times, &exact_times);
    // when the cursor reaches each knot: the pass times
    let mut pass = vec![0.0f64; m + 1];
    pass[0] = times[0];
    for j in 1..m {
        pass[j] = profile.time_of(arc[j], pass[j - 1], times[m]);
    }
    pass[m] = times[m];
    for (j, piece) in pieces.iter_mut().enumerate() {
        piece.t0 = pass[j];
        piece.t1 = pass[j + 1];
        // the pace inside the piece: the profile's knots in its interval,
        // with the speeds at its ends
        let mut pace = vec![(0.0, profile.speed(pass[j]))];
        for &(t, v) in &profile.knots {
            if t > pass[j] && t < pass[j + 1] {
                pace.push((t - pass[j], v));
            }
        }
        pace.push((pass[j + 1] - pass[j], profile.speed(pass[j + 1])));
        piece.pace = pace;
    }
    // the visits: the chart time, as long as the cursor covers the note
    // then (it does, by the invariant on the constants); otherwise the
    // first moment within the hit window at which it comes closest. Never
    // earlier than the chart time: a note arms at its time, and no player
    // hits before it.
    let at = |t: f64| -> (f32, f32) {
        let i = match pieces.binary_search_by(|s| s.t0.partial_cmp(&t).unwrap()) {
            Ok(i) => i,
            Err(i) => i.saturating_sub(1),
        };
        let s = &pieces[i];
        if t >= s.t1 {
            if i + 1 < pieces.len() {
                pieces[i + 1].p[0]
            } else {
                s.p[3]
            }
        } else {
            s.at(t)
        }
    };
    let window = BASE_HIT_WINDOW_MS * f64::from(settings.speed_mod);
    let covers = |p: (f32, f32), c: (f32, f32)| (p.0 - c.0).abs() <= HITBOX_HALF - HIT_MARGIN && (p.1 - c.1).abs() <= HITBOX_HALF - HIT_MARGIN;
    for j in 1..m {
        let c = centres[j];
        let mut visit = times[j];
        if !covers(at(visit), c) {
            let reach = (window / 2.0).min(times[j + 1] - times[j]).max(0.0);
            let mut best = f32::INFINITY;
            let mut t = times[j];
            while t <= times[j] + reach {
                let p = at(t);
                let d = (p.0 - c.0).powi(2) + (p.1 - c.1).powi(2);
                if d < best {
                    best = d;
                    visit = t;
                }
                t += 1.0;
            }
        }
        let i = a + j - 1;
        visits[i] = visit;
        if i > 0 && visits[i] < visits[i - 1] {
            visits[i] = visits[i - 1];
        }
    }
    segs.extend(pieces);
}

/// Picks a side (+1 or -1) from the dice for decision `i`.
fn side(dice: &[f32], i: usize) -> f32 {
    if dice.is_empty() {
        return 1.0;
    }
    if dice[(i.wrapping_mul(7919)) % dice.len()] >= 0.0 {
        1.0
    } else {
        -1.0
    }
}

/// Plans every segment between consecutive waypoints, starting from the
/// centre at time 0 and ending with an ease back to the centre. Streams
/// (with the jump into and out of them) are planned as one sweep each,
/// everything else segment by segment.
pub fn plan(points: &[Waypoint], settings: &AutoplaySettings, field: &Field, dice: &[f32]) -> Plan {
    let mut segs: Vec<Segment> = Vec::new();
    let mut visits: Vec<f64> = points.iter().map(|w| w.t_ms).collect();
    let centre = (0.0f32, 0.0f32);
    let factor = settings.dance.factor();
    let dance = settings.movement == Movement::Dance;
    let gaps: Vec<Gap> = (0..points.len()).map(|k| classify(points, k, settings)).collect();
    let runs = sweep_runs(&gaps);
    let mut run_at = 0usize;
    let mut i = 0usize;
    while i < points.len() {
        if run_at < runs.len() && runs[run_at].0 == i {
            let (a, b) = runs[run_at];
            plan_run(points, (a, b), settings, field, dice, &mut segs, &mut visits);
            run_at += 1;
            i = b + 1;
            continue;
        }
        let prev = before(points, i);
        let wp = points[i];
        let a = (prev.x, prev.y);
        let b = (wp.x, wp.y);
        let (t0, t1) = (prev.t_ms, wp.t_ms.max(prev.t_ms));
        let t = real_secs(t1 - t0, settings);
        let d = dist(a, b);
        match gaps[i] {
            Gap::Degenerate => segs.push(Segment::hold(t0, t1, b)),
            Gap::Rest => {
                // out to the centre, hold there, in to the note
                let out_secs = REST_OUT_MAX_SECS.min(t / 3.0);
                let out_ms = out_secs * 1000.0 * f64::from(settings.speed_mod);
                let in_secs = (dist(centre, b) / CRUISE) as f64;
                let in_ms = in_secs.max(REST_IN_MIN_SECS) * 1000.0 * f64::from(settings.speed_mod);
                let t_arrive = t0 + out_ms;
                let t_leave = (t1 - in_ms).max(t_arrive);
                segs.push(chord_curve(t0, t_arrive, a, centre, 0.0, SegmentKind::RestOut));
                segs.push(Segment::hold(t_arrive, t_leave, centre));
                segs.push(chord_curve(t_leave, t1, centre, b, 0.0, SegmentKind::RestIn));
            }
            Gap::Boomerang if dance => {
                // boomerang: out along the direction with the most room (the
                // dice break ties), back on a different arc
                let d = std::f32::consts::FRAC_1_SQRT_2;
                let dirs: [(f32, f32); 8] = [
                    (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0),
                    (d, d), (-d, d), (d, -d), (-d, -d),
                ];
                let s = side(dice, i);
                let mut best = dirs[0];
                let mut best_room = -1.0f32;
                for (k, dir) in dirs.iter().enumerate() {
                    let r = field.room(a, *dir) + if (k as f32 - 3.5) * s > 0.0 { 1e-3 } else { 0.0 };
                    if r > best_room {
                        best_room = r;
                        best = *dir;
                    }
                }
                let reach = ((CRUISE as f64 * t / BOOMERANG_DIVISOR) as f32 * factor).min(best_room);
                let tip = (a.0 + best.0 * reach, a.1 + best.1 * reach);
                let mid = t0 + (t1 - t0) / 2.0;
                let spread = BOOMERANG_SPREAD * reach;
                segs.push(chord_curve(t0, mid, a, tip, spread, SegmentKind::Boomerang));
                segs.push(chord_curve(mid, t1, tip, b, spread, SegmentKind::Boomerang));
            }
            Gap::Still | Gap::Boomerang => segs.push(Segment::hold(t0, t1, b)),
            Gap::Stream | Gap::Jump => {
                // every stream gap is inside a run, so only jumps land here:
                // the ordinary curve, with the dance's belly
                let l_max = 2.0 * field.bound * L_MAX_FACTOR;
                let l = ((CRUISE as f64 * t) as f32).clamp(d, l_max);
                let mut belly = 0.0f32;
                if dance {
                    let n = (-(b.1 - a.1) / d, (b.0 - a.0) / d);
                    let mid = ((a.0 + b.0) / 2.0, (a.1 + b.1) / 2.0);
                    let s = side(dice, i);
                    let mut room = field.room(mid, (n.0 * s, n.1 * s));
                    let mut sign = s;
                    if room < 0.05 {
                        room = field.room(mid, (-n.0 * s, -n.1 * s));
                        sign = -s;
                    }
                    let want = ((l - d) / 2.0).max(BELLY_PER_UNIT * d) * factor;
                    belly = want.min(room) * sign;
                }
                let kind = if belly.abs() > 1e-4 { SegmentKind::Curve } else { SegmentKind::Straight };
                segs.push(chord_curve(t0, t1, a, b, belly, kind));
            }
        }
        i += 1;
    }
    // after the last note: ease to the centre within one real second
    let last = points.last().copied().unwrap_or(START);
    let a = (last.x, last.y);
    let ease_ms = 1000.0 * f64::from(settings.speed_mod);
    segs.push(chord_curve(last.t_ms, last.t_ms + ease_ms, a, centre, 0.0, SegmentKind::RestOut));
    Plan { segments: segs, visits }
}

/// The cursor at `t_ms`, clamped to the field.
pub fn sample(plan: &Plan, t_ms: f64, field: &Field) -> (f32, f32) {
    let segs = &plan.segments;
    if segs.is_empty() {
        return (0.0, 0.0);
    }
    if t_ms <= segs[0].t0 {
        return field.clamp(segs[0].p[0]);
    }
    let i = match segs.binary_search_by(|s| s.t0.partial_cmp(&t_ms).unwrap()) {
        Ok(i) => i,
        Err(i) => i.saturating_sub(1),
    };
    let s = &segs[i];
    if t_ms >= s.t1 {
        if i + 1 < segs.len() {
            return field.clamp(segs[i + 1].p[0]);
        }
        return field.clamp(s.p[3]);
    }
    field.clamp(s.at(t_ms))
}
