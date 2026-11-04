//! Planning the cursor between the waypoints of a run and sampling it by
//! time. Every constant that shapes the look is named here.
use super::{AutoplaySettings, Movement, CRUISE};
use rhythia_sim::hitreg::HITBOX_HALF;

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
/// Samples per segment for the arc-length table.
const ARC_SAMPLES: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Waypoint {
    pub t_ms: f64,
    pub x: f32,
    pub y: f32,
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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SegmentKind {
    Hold,
    Straight,
    Curve,
    Boomerang,
    RestOut,
    RestIn,
}

#[derive(Clone, Debug, Default)]
pub struct Plan {
    pub segments: Vec<Segment>,
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
        Segment { t0, t1, kind, p, arc }
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
        let frac = ((t_ms - self.t0) / (self.t1 - self.t0)).clamp(0.0, 1.0) as f32;
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

/// Real seconds of a map gap under the speed mod.
fn real_secs(ms: f64, settings: &AutoplaySettings) -> f64 {
    ms / 1000.0 / f64::from(settings.speed_mod.max(0.01))
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
/// centre at time 0 and ending with an ease back to the centre.
pub fn plan(points: &[Waypoint], settings: &AutoplaySettings, field: &Field, dice: &[f32]) -> Plan {
    let mut segs: Vec<Segment> = Vec::new();
    let centre = (0.0f32, 0.0f32);
    let mut prev = Waypoint { t_ms: 0.0, x: 0.0, y: 0.0 };
    let factor = settings.dance.factor();
    let dance = settings.movement == Movement::Dance;
    for (i, wp) in points.iter().enumerate() {
        let a = (prev.x, prev.y);
        let b = (wp.x, wp.y);
        let (t0, t1) = (prev.t_ms, wp.t_ms.max(prev.t_ms));
        let t = real_secs(t1 - t0, settings);
        let d = dist(a, b);
        if t1 <= t0 {
            segs.push(Segment::hold(t0, t1, b));
        } else if t >= settings.rest_secs {
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
        } else if d < SAME_SPOT {
            if dance && t >= HOLD_SECS {
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
            } else {
                segs.push(Segment::hold(t0, t1, b));
            }
        } else {
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
        prev = *wp;
    }
    // after the last note: ease to the centre within one real second
    let a = (prev.x, prev.y);
    let ease_ms = 1000.0 * f64::from(settings.speed_mod);
    segs.push(chord_curve(prev.t_ms, prev.t_ms + ease_ms, a, centre, 0.0, SegmentKind::RestOut));
    Plan { segments: segs }
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
