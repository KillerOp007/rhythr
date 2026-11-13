//! Planning the cursor between the waypoints of a run and sampling it by
//! time. Every constant that shapes the look is named here.
use super::{Aim, AutoplaySettings, Movement, CRUISE};
use rhythia_sim::hitreg::{BASE_HIT_WINDOW_MS, HITBOX_HALF};

/// Below this gap in real seconds a same-spot pair is a hold, above it a
/// boomerang.
pub const HOLD_SECS: f64 = 0.35;
/// Two waypoints closer than this are the same spot (a quarter of the hit
/// area's width).
pub const SAME_SPOT: f32 = 2.0 * HITBOX_HALF * 0.25;
/// A boomerang reaches `CRUISE * t / BOOMERANG_DIVISOR` out before turning.
pub const BOOMERANG_DIVISOR: f64 = 2.2;
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
/// The same reach when the aim is the centre of every note: the path must
/// turn on the note, so it swings wider around it to spread the turn.
/// Measured on the Lionheart map at 60 fps: frames turning more than 60
/// degrees 83 at a third, 30 at a half, 73 at 0.65 (the loops get too wide).
pub const CENTRE_TANGENT: f32 = 0.5;
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
/// hop's time (real seconds), so the slack is a timing slack a player
/// would have too.
pub const SLACK_SECS: f64 = 0.025;
/// A speed change inside a sweep (a faster rhythm, a longer hop) is spread
/// over up to this many real seconds around the point where the string
/// changes its slope, instead of switching between two frames.
pub const RAMP_SECS: f64 = 0.15;
/// The jump into a run winds up: the cursor holds a slower pace (or waits
/// in the centre at the start of the map) and accelerates over the last
/// part of the jump, up to this fraction of it, so that it reaches the
/// stream's pace exactly at the stream's first note.
pub const WIND_UP_FRACTION: f64 = 0.6;
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
/// A jump taken as an arc (the flow aim and the dance) leaves and reaches
/// its notes along the circle through three notes in a row. The cubic's
/// handles are as long as a circular arc's (`2 / (3 (1 + cos a))` of the
/// chord for the angle `a` between heading and chord), but never more than
/// this fraction of it: a turn sharper than a right angle would otherwise
/// swing out to a full half circle, half the hop wide. At the cap a
/// reversal bulges 0.3 of the hop, a quarter turn 0.2.
pub const ARC_REACH_MAX: f32 = 0.4;
/// Along an arc the cursor may run this far ahead of or behind a jump's
/// note at its chart time (the tube of the taut string): less than in a
/// stream, a jump's note is hit close to its centre.
pub const JUMP_TUBE: f32 = 0.08;
/// A gap of this many real seconds or more is not a hop to keep a pace
/// through. With the flow aim it is a pause: the cursor carries its
/// momentum out of the last note, waits, and comes in late. Without it a
/// note next to such a gap is passed exactly on time.
pub const PAUSE_SECS: f64 = 0.6;
/// The way in from a pause takes this many real seconds.
pub const APPROACH_SECS: f64 = 0.3;
/// The way out of a phrase to the waiting spot takes at most this many.
pub const LEAVE_SECS: f64 = 0.4;
/// The handle of the way out along the heading the cursor had: how far
/// its momentum carries, at most.
pub const MOMENTUM_MAX: f32 = 0.9;
/// The waiting spot of a pause lies this fraction of the way from the
/// centre to the next note; a rest (and the time before the first note)
/// waits in the centre itself.
pub const WAIT_TOWARDS: f32 = 0.45;
/// While it waits the cursor drifts at this pace (units per real second)
/// along a lazy figure of eight with these half extents.
pub const DRIFT_SPEED: f32 = 0.8;
pub const DRIFT_LONG: f32 = 0.4;
pub const DRIFT_WIDE: f32 = 0.16;
/// The way in leaves the drift along its heading with a handle of at most
/// this length, so a drift heading away from the note turns in a small arc.
pub const LEAD_REACH: f32 = 0.2;
/// A note with nothing after it but another pause is reached at this pace
/// (units per real second).
pub const LAND_SPEED: f32 = 1.3;
/// A way out whose momentum points away from the waiting spot would come
/// back along its own line: its second control point moves out to the
/// side of the momentum's end instead, by this fraction of the momentum's
/// handle (and at least `HOOK_MIN`, so a momentum the barrier cuts short
/// still turns in an arc, not on the spot), so the cursor comes round in a
/// hook.
pub const HOOK_WIDTH: f32 = 0.8;
pub const HOOK_MIN: f32 = 0.35;
/// With the flow aim and in the dance, a sharp turn at a jump's note is
/// taken like a reversal: the heading through the note is the one across
/// the turn (the two hop directions added), which spreads the turn over
/// both hops, instead of the one a long hop next to a short one would
/// give (nearly the short hop's, a hairpin at the end of the long one).
/// From this cosine of the turn on (107 degrees) the heading blends over,
/// at the second (143 degrees) it is fully the one across.
pub const SHARP_FROM: f32 = -0.3;
pub const SHARP_FULL: f32 = -0.8;
/// Where the speed of a way out or a way in peaks, as a fraction of its
/// time: early for the way out (it mostly slows down), in the middle for
/// the way in.
const LEAVE_PEAK: f64 = 0.3;
/// With the flow aim, notes on one spot are not held: the cursor keeps
/// drifting through the hit area along its heading, at this pace (units
/// per real second) and at most this far either side of the centre.
pub const SPOT_SPEED: f32 = 1.5;
pub const SPOT_DRIFT: f32 = 0.2;
const _: () = assert!(SPOT_DRIFT + TUBE + RAMP_DEV < HITBOX_HALF - HIT_MARGIN);
/// The dance exaggerates the arc of a jump: where the pattern turns, the
/// handles grow by this factor times the piece's swing (the old belly as a
/// fraction of the chord); where it runs straight the heading through the
/// note is turned instead, at most this far (radians), to the side the
/// dice pick.
pub const DANCE_REACH: f32 = 1.0;
pub const DANCE_TURN_MAX: f32 = 1.75;
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
    /// The slow figure the cursor draws while it waits in a pause.
    Drift,
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

    /// The curve parameter at which the arc length reaches `want`.
    fn param_at(&self, want: f32) -> f32 {
        let i = match self.arc.binary_search_by(|v| v.partial_cmp(&want).unwrap()) {
            Ok(i) => i,
            Err(i) => i,
        };
        if i == 0 {
            return 0.0;
        }
        if i >= self.arc.len() {
            return 1.0;
        }
        let (a, b) = (self.arc[i - 1], self.arc[i]);
        let local = if b > a { (want - a) / (b - a) } else { 0.0 };
        ((i - 1) as f32 + local) / ARC_SAMPLES as f32
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
        let u = self.param_at(frac * total);
        if u <= 0.0 {
            return self.p[0];
        }
        if u >= 1.0 {
            return self.p[3];
        }
        bezier(&self.p, u)
    }

    /// The heading and the speed (arc units per ms) the segment ends with;
    /// no heading where the cursor stands.
    fn end_state(&self) -> (Option<(f32, f32)>, f32) {
        let total = self.length();
        if self.kind == SegmentKind::Hold || self.t1 <= self.t0 || total <= 1e-6 {
            return (None, 0.0);
        }
        let mut heading = None;
        for from in [self.p[2], self.p[1], self.p[0]] {
            let v = (self.p[3].0 - from.0, self.p[3].1 - from.1);
            let len = (v.0 * v.0 + v.1 * v.1).sqrt();
            if len > 1e-6 {
                heading = Some((v.0 / len, v.1 / len));
                break;
            }
        }
        let speed = if self.pace.len() >= 2 {
            // the knots give the shape, the length the scale
            let area: f64 = self.pace.windows(2).map(|w| (w[1].0 - w[0].0) * f64::from(w[0].1 + w[1].1) / 2.0).sum();
            if area > 1e-12 {
                (f64::from(self.pace[self.pace.len() - 1].1) * f64::from(total) / area) as f32
            } else {
                0.0
            }
        } else {
            total / (self.t1 - self.t0) as f32
        };
        (heading, speed)
    }
}

/// The part of a cubic before the parameter `u`.
fn bezier_head(p: &[(f32, f32); 4], u: f32) -> [(f32, f32); 4] {
    let mix = |a: (f32, f32), b: (f32, f32)| (a.0 + (b.0 - a.0) * u, a.1 + (b.1 - a.1) * u);
    let (a, b, c) = (mix(p[0], p[1]), mix(p[1], p[2]), mix(p[2], p[3]));
    let (d, e) = (mix(a, b), mix(b, c));
    [p[0], a, d, mix(d, e)]
}

/// The pace of a piece that starts at `v0`, ends at `v1` and covers `len`
/// in `span` ms: one knot between them at `peak_at` of the time, as fast as
/// the length asks for (never below a tenth of the slower end, so the
/// cursor does not stall when the piece is too short for both speeds).
fn hump(len: f32, span: f64, v0: f32, v1: f32, peak_at: f64) -> Vec<(f64, f32)> {
    let span = span.max(1e-6);
    let peak = 2.0 * f64::from(len) / span - peak_at * f64::from(v0) - (1.0 - peak_at) * f64::from(v1);
    let floor = f64::from(v0.min(v1)) * 0.1;
    vec![(0.0, v0), (span * peak_at, peak.max(floor).max(1e-9) as f32), (span, v1)]
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
/// are the directions at `a` and `b`. The control points sit `tangent`
/// of the chord along them, so collinear points give a
/// straight line and a corner a rounded one. A direction that says nothing
/// falls back to the chord. The times and the pace are filled in after
/// the taut string.
fn sweep_piece(a: (f32, f32), b: (f32, f32), da: (f32, f32), db: (f32, f32), tangent: f32) -> Segment {
    let d = dist(a, b).max(1e-6);
    let reach = d * tangent;
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

/// A jump from `a` to `b` taken as an arc: it leaves along `da` and
/// arrives along `db`, with handles as long as a circular arc's for the
/// angle between heading and chord (so two headings that mirror each other
/// across the chord give that arc of a circle), capped at `ARC_REACH_MAX`
/// of the chord, scaled by `scale` (the dance's exaggeration, per end) and
/// never longer than `cap_a` at the start. A handle stops at the barrier:
/// all four control points lie inside the field, so the curve does too
/// and never slides along the wall. The times and the pace are filled in
/// after the taut string.
fn arc_piece(a: (f32, f32), b: (f32, f32), da: (f32, f32), db: (f32, f32), cap_a: f32, scale: (f32, f32), field: &Field) -> Segment {
    let d = dist(a, b).max(1e-6);
    let chord = ((b.0 - a.0) / d, (b.1 - a.1) / d);
    let handle = |from: (f32, f32), v: (f32, f32), sign: f32, cap: f32, scale: f32| {
        let len = (v.0 * v.0 + v.1 * v.1).sqrt();
        let t = if len > 1e-9 { (v.0 / len, v.1 / len) } else { chord };
        let cos = (t.0 * chord.0 + t.1 * chord.1).clamp(-1.0, 1.0);
        let along = (t.0 * sign, t.1 * sign);
        let reach = (d * (2.0 / (3.0 * (1.0 + cos))).min(ARC_REACH_MAX) * scale).min(cap).min(field.room(from, along));
        (from.0 + along.0 * reach, from.1 + along.1 * reach)
    };
    Segment::new(0.0, 0.0, SegmentKind::Sweep, [a, handle(a, da, 1.0, cap_a, scale.0), handle(b, db, -1.0, f32::INFINITY, scale.1), b])
}

/// `v` mirrored across the line along `chord`: the heading a circular arc
/// has at one end of its chord when it has `v` at the other.
fn mirrored(v: (f32, f32), chord: (f32, f32)) -> (f32, f32) {
    let l2 = chord.0 * chord.0 + chord.1 * chord.1;
    if l2 < 1e-12 {
        return v;
    }
    let k = 2.0 * (v.0 * chord.0 + v.1 * chord.1) / l2;
    (k * chord.0 - v.0, k * chord.1 - v.1)
}

fn turned(v: (f32, f32), angle: f32) -> (f32, f32) {
    let (s, c) = angle.sin_cos();
    (v.0 * c - v.1 * s, v.0 * s + v.1 * c)
}

/// The heading `base` through a knot between the hops `u` (in) and `w`
/// (out), blended towards the heading across the turn where it is sharp.
fn across_sharp(base: (f32, f32), u: (f32, f32), w: (f32, f32)) -> (f32, f32) {
    let len = |v: (f32, f32)| (v.0 * v.0 + v.1 * v.1).sqrt();
    let (lu, lw, lb) = (len(u), len(w), len(base));
    if lu < 1e-6 || lw < 1e-6 || lb < 1e-9 {
        return base;
    }
    let cos = (u.0 * w.0 + u.1 * w.1) / (lu * lw);
    let sharp = ((SHARP_FROM - cos) / (SHARP_FROM - SHARP_FULL)).clamp(0.0, 1.0);
    let sum = (u.0 / lu + w.0 / lw, u.1 / lu + w.1 / lw);
    let ls = len(sum);
    if sharp <= 0.0 || ls < 1e-6 {
        return base;
    }
    (base.0 / lb * (1.0 - sharp) + sum.0 / ls * sharp, base.1 / lb * (1.0 - sharp) + sum.1 / ls * sharp)
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
    /// With the flow aim, a gap of `PAUSE_SECS` or more that is not yet a
    /// rest: the cursor leaves, waits near the next note and comes in late.
    Pause,
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
    let flow = settings.aim == Aim::Flow;
    let dance = settings.movement == Movement::Dance;
    if t >= settings.rest_secs {
        Gap::Rest
    } else if d < SAME_SPOT {
        // the flow aim keeps drifting through notes on one spot, so only
        // the dance has a use for the time (its boomerang)
        if t >= HOLD_SECS && (dance || !flow) {
            Gap::Boomerang
        } else if flow && t >= PAUSE_SECS {
            Gap::Pause
        } else {
            Gap::Still
        }
    } else if flow && t >= PAUSE_SECS {
        Gap::Pause
    } else if t <= STREAM_GAP_SECS {
        Gap::Stream
    } else {
        Gap::Jump
    }
}

/// What a phrase is made of: streams, jumps, holds, and the dance's
/// boomerangs (the flow aim has none without the dance).
fn joins(g: Gap) -> bool {
    matches!(g, Gap::Stream | Gap::Jump | Gap::Still | Gap::Boomerang)
}

/// The phrases of the flow aim and the dance: each is a range of segment
/// indices `a..=b` (segment `k` ends at waypoint `k`) of streams, jumps,
/// holds and boomerangs in a row, taken as one curve. A range of holds
/// alone is none.
fn phrase_runs(gaps: &[Gap]) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut k = 0;
    while k < gaps.len() {
        if !joins(gaps[k]) {
            k += 1;
            continue;
        }
        let a = k;
        while k < gaps.len() && joins(gaps[k]) {
            k += 1;
        }
        if gaps[a..k].iter().any(|g| *g != Gap::Still) {
            runs.push((a, k - 1));
        }
    }
    runs
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
    /// ramp of up to `ramp_ms` centred on the breakpoint (a symmetric ramp
    /// covers the same distance as the step, so the arc at the knots away
    /// from it is untouched), shortened so it moves the cursor at most
    /// `RAMP_DEV` off the arc and never reaches an exact knot (a step with
    /// no room stays a step). With `wind_up`, and the string's first bend
    /// at the end of the first piece towards a faster pace, that piece
    /// instead holds a slower pace and accelerates over its last part, so
    /// the pace is the stream's exactly at the stream's first note and the
    /// arc there is still the string's own. With `lead` (the pace the
    /// cursor drifts at, and the pace it lands with when nothing follows)
    /// the first piece is the way in from a pause: it starts at the drift's
    /// pace, ends at the pace behind the first note and covers exactly the
    /// string's arc up to that note.
    fn smoothed(string: &[(usize, f32)], times: &[f64], exact_times: &[f64], ramp_ms: f64, wind_up: bool, lead: Option<(f32, f32)>) -> Profile {
        let bp: Vec<(f64, f32)> = string.iter().map(|&(k, s)| (times[k], s)).collect();
        let speeds: Vec<f32> = bp.windows(2).map(|w| ((w[1].1 - w[0].1) / ((w[1].0 - w[0].0).max(1e-6)) as f32).max(0.0)).collect();
        let mut knots: Vec<(f64, f32)> = Vec::new();
        let mut first = 1;
        // no ramp reaches back into a first piece with a pace of its own
        let mut ramps_from = bp[0].0;
        if let Some((v_in, v_land)) = lead {
            let (t0, s0) = bp[0];
            let t1 = times[1];
            let span_t = (t1 - t0).max(1e-6);
            let bends = string[1].0 == 1;
            let span_s = f64::from(if bends { bp[1].1 - s0 } else { speeds[0] * span_t as f32 }).max(0.0);
            let v_out = f64::from(if !bends {
                speeds[0]
            } else if speeds.len() >= 2 {
                speeds[1]
            } else {
                v_land.min(speeds[0])
            });
            let v_in = f64::from(v_in);
            let peak = (4.0 * span_s / span_t - v_in - v_out) / 2.0;
            knots.push((t0, v_in as f32));
            if span_s >= v_in * span_t && peak >= v_in.min(v_out) {
                // speed up and settle, or one steady change
                knots.push((t0 + span_t / 2.0, peak as f32));
            } else if span_s >= v_in * span_t {
                // too short for that: keep drifting, then speed up
                let h = (2.0 * (span_s - v_in * span_t) / (v_out - v_in).max(1e-12)).clamp(0.0, span_t);
                knots.push((t1 - h, v_in as f32));
            } else {
                // shorter than the drift alone would go: slower still, and
                // the speed-up over the second half (or the last part)
                let flat = (4.0 * span_s / span_t - v_out) / 3.0;
                knots.clear();
                if flat >= 0.0 {
                    knots.push((t0, flat as f32));
                    knots.push((t0 + span_t / 2.0, flat as f32));
                } else {
                    let h = (2.0 * span_s / v_out.max(1e-12)).clamp(0.0, span_t);
                    knots.push((t0, 0.0));
                    knots.push((t1 - h, 0.0));
                }
            }
            knots.push((t1, v_out as f32));
            ramps_from = t1;
            first = if bends { 2 } else { 1 };
        } else if wind_up && speeds.len() >= 2 && string[1].0 == 1 && speeds[1] > speeds[0] {
            let (t0, s0) = bp[0];
            let (t1, s1) = bp[1];
            let span_t = t1 - t0;
            let span_s = f64::from(s1 - s0);
            let v_s = f64::from(speeds[1]);
            // as long a ramp as the piece allows without a pace below zero
            let h = (WIND_UP_FRACTION * span_t).min(2.0 * span_s / v_s.max(1e-9)).max(0.0);
            let v_flat = ((span_s - v_s * h / 2.0) / (span_t - h / 2.0).max(1e-9)).max(0.0) as f32;
            knots.push((t0, v_flat));
            knots.push((t1 - h, v_flat));
            knots.push((t1, speeds[1]));
            first = 2;
        } else {
            knots.push((bp[0].0, speeds[0]));
        }
        for i in first..speeds.len() {
            let t_b = bp[i].0;
            let mut h = (ramp_ms / 2.0).min((t_b - bp[i - 1].0) / 2.0).min((bp[i + 1].0 - t_b) / 2.0).min(t_b - ramps_from);
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
        // (a way in that is the whole run has set its own last knot)
        if knots.last().map(|k| k.0) != Some(bp[bp.len() - 1].0) {
            knots.push((bp[bp.len() - 1].0, speeds[speeds.len() - 1]));
        }
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

/// How a run begins when a pause comes before it: the heading and the pace
/// (arc units per ms) the cursor drifts with where its way in starts, and
/// the pace it lands with when the way in is all there is.
#[derive(Clone, Copy, Debug)]
struct Lead {
    dir: (f32, f32),
    speed: f32,
    land: f32,
}

/// The heading through a straight stretch turns this far in the dance for
/// a swing of `s` (the belly as a fraction of the chord): the angle at
/// which an arc of that belly leaves its chord.
fn swing_turn(s: f32) -> f32 {
    (2.0 * (1.5 * s).atan()).min(DANCE_TURN_MAX)
}

/// Plans one run: `knots` are the waypoint before it and its own, `kinds`
/// the gap between each two of them, `owners` the waypoint each knot
/// stands for (none for the first, and none for a boomerang's far end,
/// which is no note), `base` the index of its first waypoint (the dice
/// are asked by it). The knots are passed on
/// one smooth curve, the inner ones of a stream beside their centres, at a
/// speed that the taut string keeps steady wherever the notes allow and
/// that ramps where it must change. With the flow aim and in the dance a
/// jump is an arc of the circle through three notes in a row, and with
/// `lead` the first piece is the way in from a pause. Pushes the pieces and
/// fills the visits of the inner waypoints.
#[allow(clippy::too_many_arguments)]
fn plan_run(knots: &[Waypoint], kinds: &[Gap], owners: &[Option<usize>], base: usize, lead: Option<Lead>, settings: &AutoplaySettings, field: &Field, dice: &[f32], segs: &mut Vec<Segment>, visits: &mut [f64]) {
    let m = kinds.len();
    let times: Vec<f64> = knots.iter().map(|w| w.t_ms).collect();
    let centres: Vec<(f32, f32)> = knots.iter().map(|w| (w.x, w.y)).collect();
    let mut exact: Vec<bool> = (0..=m).map(|j| j == 0 || j == m || knots[j].exact).collect();
    let flow = settings.aim == Aim::Flow;
    let dance = settings.movement == Movement::Dance;
    let smooth = flow || dance;
    let tangent = if flow { STREAM_TANGENT } else { CENTRE_TANGENT };
    let to_map_ms = 1000.0 * f64::from(settings.speed_mod);
    // the note a way in from a pause lands on is reached on time, on its
    // centre: the way in is short, and its slack would carry on into the
    // phrase behind it
    if lead.is_some() && m >= 2 {
        exact[1] = true;
    }
    // a jump taken as an arc
    let leap: Vec<bool> = kinds.iter().map(|k| smooth && *k == Gap::Jump).collect();
    // the passed points: with the flow aim the inner knots beside their
    // centres (a reversal tip close to it); with the centre aim, the ends
    // and the exact ones always, on the spot
    let mut q = centres.clone();
    // notes on one spot: with the flow aim the cursor does not stand on
    // them, it drifts through the hit area: the first of them is passed a
    // little before the centre on the way in, the last a little behind it
    // on the way out, the ones between on the way from one to the other
    let mut placed = vec![false; m + 1];
    if flow {
        let mut j0 = 0;
        while j0 < m {
            if kinds[j0] != Gap::Still {
                j0 += 1;
                continue;
            }
            let mut j1 = j0 + 1;
            while j1 < m && kinds[j1] == Gap::Still {
                j1 += 1;
            }
            // knots j0..=j1 share the spot (a chord's shared spot is exact
            // and stays where it is; so do the ends of the run)
            let span = times[j1] - times[j0];
            if (j0..=j1).all(|k| !knots[k].exact) && span > 1e-6 && !(j0 == 0 && j1 == m) {
                let c = centres[j0];
                let unit = |v: (f32, f32)| {
                    let len = (v.0 * v.0 + v.1 * v.1).sqrt();
                    (len > 0.05).then(|| (v.0 / len, v.1 / len))
                };
                let s = side(dice, base + j0);
                let from = (j0 > 0).then(|| centres[j0 - 1]);
                let to = (j1 < m).then(|| centres[j1 + 1]);
                let dir_in = from.and_then(|p| unit((c.0 - p.0, c.1 - p.1)));
                let dir_out = to.and_then(|n| unit((n.0 - c.0, n.1 - c.1)));
                let (mut u, mut w) = match (dir_in, dir_out) {
                    (Some(u), Some(w)) => (u, w),
                    (Some(u), None) => (u, u),
                    (None, Some(w)) => (w, w),
                    (None, None) => ((s, 0.0), (s, 0.0)),
                };
                // (out much the way it came: the turn is taken across the
                // spot, from the outer side of the turn to the inner, the
                // way a reversal's loop goes round a single note)
                if u.0 * w.0 + u.1 * w.1 < -0.5 {
                    let cross = u.0 * w.1 - u.1 * w.0;
                    let turn = if cross.abs() > 0.05 { cross.signum() } else { s };
                    u = (-u.1 * turn, u.0 * turn);
                    w = u;
                }
                // at the pace of the hops around the spot (a double inside
                // a slide is slid through), at least SPOT_SPEED
                let pace = |p: (f32, f32), ms: f64| dist(p, c) / (ms.max(1e-6) / to_map_ms) as f32;
                let around = from.map_or(0.0, |p| pace(p, times[j0] - times[j0 - 1])).max(to.map_or(0.0, |n| pace(n, times[j1 + 1] - times[j1])));
                let reach = SPOT_DRIFT.min(0.5 * SPOT_SPEED.max(around) * (span / to_map_ms) as f32);
                for k in j0..=j1 {
                    if k == 0 || k == m {
                        continue;
                    }
                    let frac = ((times[k] - times[j0]) / span) as f32;
                    // (an end of the run stays on its centre: the drift
                    // then only leaves it, or only comes in)
                    let (back, on) = if j0 == 0 {
                        (0.0, frac)
                    } else if j1 == m {
                        (1.0 - frac, 0.0)
                    } else {
                        ((1.0 - 2.0 * frac).max(0.0), (2.0 * frac - 1.0).max(0.0))
                    };
                    let off = ((w.0 * on - u.0 * back) * reach, (w.1 * on - u.1 * back) * reach);
                    q[k] = field.clamp((centres[k].0 + off.0, centres[k].1 + off.1));
                    placed[k] = true;
                }
            }
            j0 = j1;
        }
    }
    // knots on one spot (a chord's twin) are one point of the curve
    let mut lo_of: Vec<usize> = (0..=m).collect();
    let mut hi_of: Vec<usize> = (0..=m).collect();
    for j in 1..=m {
        if dist(q[j - 1], q[j]) < 1e-4 {
            lo_of[j] = lo_of[j - 1];
        }
    }
    for j in (0..m).rev() {
        if dist(q[j], q[j + 1]) < 1e-4 {
            hi_of[j] = hi_of[j + 1];
        }
    }
    // an inner knot between two arcs lies on the circle through its
    // neighbours: passed through its centre, no cut
    let round: Vec<bool> = (0..=m).map(|j| lo_of[j] >= 1 && hi_of[j] < m && leap[lo_of[j] - 1] && leap[hi_of[j]]).collect();
    let mut tip = vec![false; m + 1];
    for j in 1..m {
        if placed[j] || round[j] {
            continue;
        }
        let (p, c, n) = (centres[j - 1], centres[j], centres[j + 1]);
        let (u, w) = ((c.0 - p.0, c.1 - p.1), (n.0 - c.0, n.1 - c.1));
        let (lu, lw) = ((u.0 * u.0 + u.1 * u.1).sqrt(), (w.0 * w.0 + w.1 * w.1).sqrt());
        // (a reversal right after the start in the centre is one too: the
        // way out and the way back share the line)
        tip[j] = lu > 1e-6 && lw > 1e-6 && (u.0 * w.0 + u.1 * w.1) / (lu * lw) < TIP_COS;
        let cap = if exact[j] || !flow { 0.0 } else if tip[j] { TIP_CUT } else { CORNER_CUT_MAX };
        // the B-spline pull of two equal hops, with the shorter hop's
        // length for both, so a corner next to a long hop is cut like any
        // other instead of being dragged into the long hop
        let mut off = if lu > 1e-6 && lw > 1e-6 {
            let scale = lu.min(lw) / 6.0;
            ((w.0 / lw - u.0 / lu) * scale, (w.1 / lw - u.1 / lu) * scale)
        } else {
            (0.0, 0.0)
        };
        let len = (off.0 * off.0 + off.1 * off.1).sqrt();
        if len > cap {
            off = (off.0 * cap / len, off.1 * cap / len);
        }
        q[j] = field.clamp((c.0 + off.0, c.1 + off.1));
    }
    // the dance's swing of each arc, as a fraction of its chord: the belly
    // the distance asks for, wider with time to spare, scaled by the amount
    let factor = settings.dance.factor();
    let l_max = 2.0 * field.bound * L_MAX_FACTOR;
    let swing: Vec<f32> = (0..m)
        .map(|j| {
            if !(dance && leap[j]) {
                return 0.0;
            }
            let d = dist(q[j], q[j + 1]).max(1e-6);
            let l = ((CRUISE as f64 * (times[j + 1] - times[j]) / to_map_ms) as f32).clamp(d, l_max.max(d));
            ((l - d) / 2.0).max(BELLY_PER_UNIT * d) * factor / d
        })
        .collect();
    // the direction at each knot: through the neighbours, or across the
    // line at a tip, alternating sides so a jack becomes an ellipse (the
    // dice pick the first side)
    let mut dir: Vec<(f32, f32)> = vec![(0.0, 0.0); m + 1];
    // how straight the pattern runs through a knot between two arcs: 1 on
    // a line, 0 from a right angle on (the dance turns the heading where
    // it is straight and widens the arc where it turns)
    let mut straight = vec![1.0f32; m + 1];
    let mut side_now = side(dice, base);
    // the line's normal in one fixed orientation, so consecutive tips on
    // the same line alternate sides
    let across = |u: (f32, f32)| {
        let mut n = (-u.1, u.0);
        if n.1 < 0.0 || (n.1 == 0.0 && n.0 < 0.0) {
            n = (-n.0, -n.1);
        }
        n
    };
    for j in 1..m {
        if round[j] {
            let (lo, hi) = (lo_of[j], hi_of[j]);
            if j > lo {
                dir[j] = dir[lo];
                straight[j] = straight[lo];
                continue;
            }
            let (p, c, n) = (q[lo - 1], q[j], q[hi + 1]);
            let (u, w) = ((c.0 - p.0, c.1 - p.1), (n.0 - c.0, n.1 - c.1));
            let (lu, lw) = ((u.0 * u.0 + u.1 * u.1).sqrt().max(1e-6), (w.0 * w.0 + w.1 * w.1).sqrt().max(1e-6));
            let cos = (u.0 * w.0 + u.1 * w.1) / (lu * lw);
            if cos < TIP_COS {
                let n = across(u);
                dir[j] = (n.0 * side_now, n.1 * side_now);
                side_now = -side_now;
                straight[j] = 0.0;
            } else {
                // the tangent of the circle through the three points: each
                // hop's direction weighted by the other hop's length
                dir[j] = across_sharp((u.0 / lu * lw + w.0 / lw * lu, u.1 / lu * lw + w.1 / lw * lu), u, w);
                straight[j] = cos.max(0.0);
                if dance {
                    let turn = swing_turn(swing[lo - 1].min(swing[hi])) * straight[j] * side(dice, base + j);
                    dir[j] = turned(dir[j], turn);
                }
            }
        } else if tip[j] {
            let n = across((q[j].0 - q[j - 1].0, q[j].1 - q[j - 1].1));
            dir[j] = (n.0 * side_now, n.1 * side_now);
            side_now = -side_now;
        } else {
            // the velocity through the knot, each hop weighted by its own
            // time, so a long jump in does not drag the heading along
            let dt_in = ((times[j] - times[j - 1]).max(1e-6)) as f32;
            let dt_out = ((times[j + 1] - times[j]).max(1e-6)) as f32;
            dir[j] = ((q[j + 1].0 - q[j].0) / dt_out + (q[j].0 - q[j - 1].0) / dt_in, (q[j + 1].1 - q[j].1) / dt_out + (q[j].1 - q[j - 1].1) / dt_in);
            // (a jump that meets a stream in a sharp turn: across it)
            if leap[j - 1] != leap[j] {
                dir[j] = across_sharp(dir[j], (q[j].0 - q[j - 1].0, q[j].1 - q[j - 1].1), (q[j + 1].0 - q[j].0, q[j + 1].1 - q[j].1));
            }
        }
    }
    // the pieces and the arc length at each knot
    let mut pieces: Vec<Segment> = Vec::with_capacity(m);
    let mut arc = vec![0.0f32; m + 1];
    for j in 0..m {
        let chord = (q[j + 1].0 - q[j].0, q[j + 1].1 - q[j].1);
        let mut da = if j == 0 { chord } else { dir[j] };
        let mut db = if j + 1 == m { chord } else { dir[j + 1] };
        let piece = if leap[j] {
            // at an end of the run the arc is the one that meets the
            // heading at its inner knot (the circle through the first or
            // last three notes); the way in from a pause leaves along the
            // drift, on a short handle
            let mut cap = f32::INFINITY;
            if j == 0 {
                if let Some(l) = &lead {
                    da = l.dir;
                    cap = LEAD_REACH;
                } else if m >= 2 {
                    da = mirrored(dir[1], chord);
                } else if dance {
                    da = turned(chord, swing_turn(swing[0]) * side(dice, base));
                }
            }
            if j + 1 == m {
                db = mirrored(da, chord);
                // (a way in that would arrive facing backwards comes
                // straight in instead)
                if lead.is_some() && j == 0 && db.0 * chord.0 + db.1 * chord.1 < 0.5 * (db.0 * db.0 + db.1 * db.1).sqrt() * (chord.0 * chord.0 + chord.1 * chord.1).sqrt() {
                    db = chord;
                }
            }
            // the dance widens the arc where the pattern turns (at an end
            // of the run: as at the inner knot next to it)
            let grow = |knot: Option<usize>| 1.0 + DANCE_REACH * swing[j] * (1.0 - knot.map_or(1.0, |k| straight[k]));
            let inner = |k: usize| (k >= 1 && k < m).then_some(k);
            let scale = (grow(inner(j).or(inner(j + 1))), grow(inner(j + 1).or(inner(j))));
            arc_piece(q[j], q[j + 1], da, db, cap, scale, field)
        } else {
            sweep_piece(q[j], q[j + 1], da, db, tangent)
        };
        arc[j + 1] = arc[j] + piece.length();
        pieces.push(piece);
    }
    // the tube: TUBE along the arc (less between two jumps), but never more
    // than SLACK_SECS of the slower neighbouring hop's time, so a slow jump
    // into the stream cannot arrive a tenth of a second early; nothing at
    // the ends and the exact knots, and nothing at a note next to a gap
    // too long to keep a pace through
    let slack_ms = (SLACK_SECS * to_map_ms) as f32;
    let mut lo = arc.clone();
    let mut hi = arc.clone();
    for j in 1..m {
        let long = |from: usize| smooth && (times[from + 1] - times[from]) / to_map_ms >= PAUSE_SECS;
        if long(j - 1) || long(j) {
            exact[j] = true;
        }
        if exact[j] {
            continue;
        }
        let v_in = (arc[j] - arc[j - 1]) / ((times[j] - times[j - 1]).max(1e-6)) as f32;
        let v_out = (arc[j + 1] - arc[j]) / ((times[j + 1] - times[j]).max(1e-6)) as f32;
        let tube = (if round[j] { JUMP_TUBE } else { TUBE }).min(slack_ms * v_in.min(v_out));
        lo[j] = arc[j] - tube;
        hi[j] = arc[j] + tube;
    }
    let string = taut_string(&times, &lo, &hi);
    let exact_times: Vec<f64> = (1..m).filter(|&j| exact[j]).map(|j| times[j]).collect();
    // the run's first piece is the jump in when it is a jump at all (the
    // start of the map counts): it winds up instead of switching pace
    // (in a phrase of the flow aim or the dance only when a stream follows
    // it: a jump before more jumps keeps its own even pace)
    let wind_up = lead.is_none() && kinds[0] == Gap::Jump && (!smooth || kinds.get(1) == Some(&Gap::Stream));
    let profile = Profile::smoothed(&string, &times, &exact_times, RAMP_SECS * to_map_ms, wind_up, lead.map(|l| (l.speed, l.land)));
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
        let Some(i) = owners[j] else { continue };
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

/// The far end of a boomerang from `a` with `t` real seconds to spare: out
/// along the direction with the most room (the dice break ties).
fn boomerang_tip(a: (f32, f32), t: f64, s: f32, factor: f32, field: &Field) -> (f32, f32) {
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let dirs: [(f32, f32); 8] = [
        (1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0),
        (d, d), (-d, d), (d, -d), (-d, -d),
    ];
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
    (a.0 + best.0 * reach, a.1 + best.1 * reach)
}

/// The knots of a phrase over the waypoints `a..=b` behind `first`, the
/// gap between each two, and the waypoint each knot stands for. A
/// boomerang (the dance's use of a same spot with time) is two arcs: out
/// to its far end halfway through the gap, a knot of its own, and back.
/// With `way_in` the first gap is the way in from a pause: an arc.
#[allow(clippy::too_many_arguments)]
fn phrase_knots(points: &[Waypoint], gaps: &[Gap], a: usize, b: usize, first: Waypoint, way_in: bool, settings: &AutoplaySettings, field: &Field, dice: &[f32]) -> (Vec<Waypoint>, Vec<Gap>, Vec<Option<usize>>) {
    let mut knots = vec![first];
    let mut kinds: Vec<Gap> = Vec::with_capacity(b - a + 1);
    let mut owners: Vec<Option<usize>> = vec![None];
    for k in a..=b {
        let prev = knots[knots.len() - 1];
        if way_in && k == a {
            kinds.push(Gap::Jump);
        } else if gaps[k] == Gap::Boomerang {
            let t = real_secs(points[k].t_ms - prev.t_ms, settings);
            let tip = boomerang_tip((prev.x, prev.y), t, side(dice, k), settings.dance.factor(), field);
            knots.push(Waypoint { t_ms: (prev.t_ms + points[k].t_ms) / 2.0, x: tip.0, y: tip.1, exact: false });
            owners.push(None);
            kinds.push(Gap::Jump);
            kinds.push(Gap::Jump);
        } else {
            kinds.push(gaps[k]);
        }
        knots.push(points[k]);
        owners.push(Some(k));
    }
    (knots, kinds, owners)
}

/// How the cursor comes out of what was planned so far: its heading and
/// its pace (arc units per ms) at `now`. A chord's twin is a piece without
/// length half a millisecond long; the piece before it says how the cursor
/// arrived.
fn last_state(segs: &[Segment], now: f64) -> (Option<(f32, f32)>, f32) {
    for s in segs.iter().rev() {
        let state = s.end_state();
        if state.0.is_some() || now - s.t0 > 5.0 {
            return state;
        }
    }
    (None, 0.0)
}

/// How far the cursor's momentum carries it out of `a`: the handle of the
/// way out along its heading, and the point it reaches.
fn carried(a: (f32, f32), state: (Option<(f32, f32)>, f32), settings: &AutoplaySettings, field: &Field) -> (f32, (f32, f32)) {
    let leave_ms = LEAVE_SECS * 1000.0 * f64::from(settings.speed_mod);
    match state.0 {
        Some(d) => {
            // (well short of the barrier: the turn back needs room too)
            let reach = (state.1 * leave_ms as f32 / 3.0).min(MOMENTUM_MAX).min(0.6 * field.room(a, d));
            (reach, (a.0 + d.0 * reach, a.1 + d.1 * reach))
        }
        None => (0.0, a),
    }
}

/// The curve of a way out: from `a` along the momentum (a handle `reach`
/// long that ends at `c1`) round to `spot`. Returns the control points and
/// the heading it arrives with. The more the momentum points away from the
/// spot, the further the second control point moves from the straight way
/// in to the side of the momentum's end (the side the spot lies on, the
/// dice's when it lies dead behind): a hook, not a line out and back.
fn way_out(a: (f32, f32), c1: (f32, f32), spot: (f32, f32), heading: Option<(f32, f32)>, reach: f32, flip: f32, field: &Field) -> ([(f32, f32); 4], (f32, f32)) {
    let far = dist(c1, spot);
    let straight = if far > 1e-4 { ((spot.0 - c1.0) / far, (spot.1 - c1.1) / far) } else { heading.unwrap_or((flip, 0.0)) };
    let mut c2 = (spot.0 - straight.0 * 0.4 * far, spot.1 - straight.1 * 0.4 * far);
    if let Some(d) = heading {
        let away = (-(d.0 * straight.0 + d.1 * straight.1)).max(0.0);
        let cross = d.0 * (spot.1 - a.1) - d.1 * (spot.0 - a.0);
        let s = if cross.abs() > 0.05 { cross.signum() } else { flip };
        let wide = (HOOK_WIDTH * reach).max(HOOK_MIN);
        let hook = field.clamp((c1.0 - d.1 * s * wide, c1.1 + d.0 * s * wide));
        c2 = (c2.0 + (hook.0 - c2.0) * away, c2.1 + (hook.1 - c2.1) * away);
    }
    let back = dist(c2, spot);
    let g = if back > 1e-4 { ((spot.0 - c2.0) / back, (spot.1 - c2.1) / back) } else { straight };
    ([a, c1, c2, spot], g)
}

/// The way out of a phrase into a pause and the wait in it, with the flow
/// aim. From `a` at `t0`, where the cursor has the heading and pace
/// `state`, one curve follows the momentum and bends round to the waiting
/// `spot`, slowing to the drift's pace. There the cursor drifts along a
/// lazy figure of eight (it never stands) until `t_in`, when its way in
/// begins. Returns where it is then, its heading and its pace.
#[allow(clippy::too_many_arguments)]
fn plan_wait(t0: f64, t_in: f64, a: (f32, f32), state: (Option<(f32, f32)>, f32), spot: (f32, f32), flip: f32, settings: &AutoplaySettings, field: &Field, segs: &mut Vec<Segment>) -> ((f32, f32), (f32, f32), f32) {
    let to_map_ms = 1000.0 * f64::from(settings.speed_mod);
    let drift = DRIFT_SPEED / to_map_ms as f32;
    let (heading, speed) = state;
    let unit = |v: (f32, f32)| {
        let len = (v.0 * v.0 + v.1 * v.1).sqrt();
        (len > 1e-4).then(|| (v.0 / len, v.1 / len))
    };
    let fallback = heading.or_else(|| unit((spot.0 - a.0, spot.1 - a.1))).unwrap_or((flip, 0.0));
    let avail = t_in - t0;
    if avail < 1.0 {
        return (a, fallback, speed);
    }
    let (reach, c1) = carried(a, state, settings, field);
    let far = dist(c1, spot);
    let (mut at, mut dir, mut t, mut pace) = (a, fallback, t0, speed);
    let leaves = far > 0.15 || reach > 0.05;
    if leaves {
        let (curve, g) = way_out(a, c1, spot, heading, reach, flip, field);
        let mut leg = Segment::new(t0, t0, SegmentKind::RestOut, curve);
        let len = leg.length();
        // as long as slowing down to the drift's pace takes, at most
        // LEAVE_SECS, at most the time there is
        let slow = 2.0 * f64::from(len) / (LEAVE_PEAK * f64::from(speed) + (2.0 - LEAVE_PEAK) * f64::from(drift)).max(1e-12);
        let span = (LEAVE_SECS * to_map_ms).min(slow).min(avail);
        leg.t1 = t0 + span;
        leg.pace = hump(len, span, speed, drift, LEAVE_PEAK);
        segs.push(leg);
        (at, dir, t, pace) = (spot, g, t0 + span, drift);
    }
    let idle = t_in - t;
    if idle < 1.0 {
        return (at, dir, pace);
    }
    // the figure of eight through where the cursor is, entered along its
    // heading; it first curls on the way the way out was bending, or as
    // the dice say
    let curl = match heading {
        Some(d) if leaves && (dir.0 * d.1 - dir.1 * d.0).abs() > 1e-3 => (dir.0 * d.1 - dir.1 * d.0).signum(),
        _ => flip,
    };
    let wide = DRIFT_WIDE * curl;
    let (sin, cos) = (dir.1.atan2(dir.0) - (2.0 * wide).atan2(DRIFT_LONG)).sin_cos();
    let origin = at;
    let point = |phi: f32| {
        let (x, y) = (DRIFT_LONG * phi.sin(), wide * (2.0 * phi).sin());
        (origin.0 + x * cos - y * sin, origin.1 + x * sin + y * cos)
    };
    let slope = |phi: f32| {
        let (x, y) = (DRIFT_LONG * phi.cos(), 2.0 * wide * (2.0 * phi).cos());
        (x * cos - y * sin, x * sin + y * cos)
    };
    let step = std::f32::consts::PI / 8.0;
    let total = drift * idle as f32;
    let mut done = 0.0f32;
    let mut k = 0usize;
    loop {
        let (f0, f1) = (k as f32 * step, (k + 1) as f32 * step);
        let (p0, p3, d0, d3) = (point(f0), point(f1), slope(f0), slope(f1));
        let h = step / 3.0;
        let ctrl = [p0, (p0.0 + d0.0 * h, p0.1 + d0.1 * h), (p3.0 - d3.0 * h, p3.1 - d3.1 * h), p3];
        let mut piece = Segment::new(t, t, SegmentKind::Drift, ctrl);
        let len = piece.length();
        if done + len >= total || k >= 4096 {
            // the wait ends inside this piece
            let u = piece.param_at(total - done);
            let head = bezier_head(&ctrl, u);
            segs.push(Segment::new(t, t_in, SegmentKind::Drift, head));
            at = head[3];
            dir = unit(slope(f0 + u * step)).unwrap_or(dir);
            break;
        }
        piece.t1 = t + f64::from(len / drift);
        t = piece.t1;
        done += len;
        k += 1;
        segs.push(piece);
    }
    (at, dir, drift)
}

/// After the last note with the flow aim: the momentum carries the cursor
/// on, and it comes to rest in the centre within one real second.
fn plan_end(t0: f64, a: (f32, f32), state: (Option<(f32, f32)>, f32), flip: f32, settings: &AutoplaySettings, field: &Field, segs: &mut Vec<Segment>) {
    let (reach, c1) = carried(a, state, settings, field);
    let (curve, _) = way_out(a, c1, (0.0, 0.0), state.0, reach, flip, field);
    let mut leg = Segment::new(t0, t0, SegmentKind::RestOut, curve);
    let len = leg.length();
    let slow = 2.0 * f64::from(len) / (LEAVE_PEAK * f64::from(state.1)).max(1e-12);
    let span = (1000.0 * f64::from(settings.speed_mod)).min(slow);
    leg.t1 = t0 + span;
    leg.pace = hump(len, span, state.1, 0.0, LEAVE_PEAK);
    segs.push(leg);
}

/// Plans every segment between consecutive waypoints, starting from the
/// centre at time 0 and ending in the centre. Streams (with the jump into
/// and out of them) are planned as one sweep each, everything else segment
/// by segment. With the flow aim and in the dance the jumps join the
/// sweeps: a phrase is one curve from pause to pause, and the flow aim
/// fills the pauses (the way out, the wait, the way in).
pub fn plan(points: &[Waypoint], settings: &AutoplaySettings, field: &Field, dice: &[f32]) -> Plan {
    let mut segs: Vec<Segment> = Vec::new();
    let mut visits: Vec<f64> = points.iter().map(|w| w.t_ms).collect();
    let centre = (0.0f32, 0.0f32);
    let dance = settings.movement == Movement::Dance;
    let flow = settings.aim == Aim::Flow;
    let to_map_ms = 1000.0 * f64::from(settings.speed_mod);
    let gaps: Vec<Gap> = (0..points.len()).map(|k| classify(points, k, settings)).collect();
    let runs = if flow || dance { phrase_runs(&gaps) } else { sweep_runs(&gaps) };
    let mut run_at = 0usize;
    let mut i = 0usize;
    while i < points.len() {
        if run_at < runs.len() && runs[run_at].0 == i {
            let (a, b) = runs[run_at];
            let (knots, kinds, owners) = phrase_knots(points, &gaps, a, b, before(points, a), false, settings, field, dice);
            plan_run(&knots, &kinds, &owners, a, None, settings, field, dice, &mut segs, &mut visits);
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
        match gaps[i] {
            Gap::Degenerate => segs.push(Segment::hold(t0, t1, b)),
            Gap::Pause | Gap::Rest if flow => {
                // out of the last note and to the waiting spot: near the
                // next note in a pause, the centre in a rest and before the
                // first note
                let spot = if gaps[i] == Gap::Rest || i == 0 { centre } else { (b.0 * WAIT_TOWARDS, b.1 * WAIT_TOWARDS) };
                let t_in = t1 - (APPROACH_SECS * to_map_ms).min((t1 - t0) / 2.0);
                let state = last_state(&segs, t0);
                let (from, heading, speed) = plan_wait(t0, t_in, a, state, spot, side(dice, i), settings, field, &mut segs);
                // the way in is the first piece of the phrase behind the
                // pause: one curve from the wait through its notes
                let mut end = i;
                while end + 1 < points.len() && joins(gaps[end + 1]) {
                    end += 1;
                }
                if run_at < runs.len() && runs[run_at].0 == i + 1 {
                    run_at += 1;
                }
                let first = Waypoint { t_ms: t_in, x: from.0, y: from.1, exact: true };
                let (knots, kinds, owners) = phrase_knots(points, &gaps, i, end, first, true, settings, field, dice);
                let lead = Lead { dir: heading, speed, land: LAND_SPEED / to_map_ms as f32 };
                plan_run(&knots, &kinds, &owners, i, Some(lead), settings, field, dice, &mut segs, &mut visits);
                i = end + 1;
                continue;
            }
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
            // (the dance's boomerangs are inside its phrases; without it a
            // same spot is held, however long)
            Gap::Still | Gap::Boomerang | Gap::Pause => segs.push(Segment::hold(t0, t1, b)),
            // every stream gap is inside a run, and so is every jump of
            // the flow aim and the dance: only the jumps of normal
            // movement with the centre aim land here, a straight line
            Gap::Stream | Gap::Jump => segs.push(chord_curve(t0, t1, a, b, 0.0, SegmentKind::Straight)),
        }
        i += 1;
    }
    let last = points.last().copied().unwrap_or(START);
    let a = (last.x, last.y);
    if flow {
        plan_end(last.t_ms, a, last_state(&segs, last.t_ms), side(dice, points.len()), settings, field, &mut segs);
    } else {
        // after the last note: ease to the centre within one real second
        let ease_ms = 1000.0 * f64::from(settings.speed_mod);
        segs.push(chord_curve(last.t_ms, last.t_ms + ease_ms, a, centre, 0.0, SegmentKind::RestOut));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn tube_walls(arc: &[f32]) -> (Vec<f32>, Vec<f32>) {
        let n = arc.len();
        let lo = arc.iter().enumerate().map(|(k, a)| if k == 0 || k + 1 == n { *a } else { a - TUBE }).collect();
        let hi = arc.iter().enumerate().map(|(k, a)| if k == 0 || k + 1 == n { *a } else { a + TUBE }).collect();
        (lo, hi)
    }

    #[test]
    fn the_taut_string_runs_straight_through_a_uniform_stream() {
        let times: Vec<f64> = (0..8).map(|k| 1000.0 + 48.0 * k as f64).collect();
        let arc: Vec<f32> = (0..8).map(|k| k as f32).collect();
        let (lo, hi) = tube_walls(&arc);
        assert_eq!(taut_string(&times, &lo, &hi), vec![(0, 0.0), (7, 7.0)]);
    }

    #[test]
    fn the_taut_string_bends_at_the_walls_of_a_long_hop() {
        // hops 1, 1, 1, 2, 1, 1 at 48 ms: the string rises to the ceiling
        // before the long hop and falls to the floor after it
        let times: Vec<f64> = (0..7).map(|k| 1000.0 + 48.0 * k as f64).collect();
        let arc = [0.0f32, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0];
        let (lo, hi) = tube_walls(&arc);
        let s = taut_string(&times, &lo, &hi);
        let knots: Vec<usize> = s.iter().map(|b| b.0).collect();
        assert_eq!(knots, vec![0, 3, 4, 6], "{s:?}");
        assert!((s[1].1 - (3.0 + TUBE)).abs() < 1e-5 && (s[2].1 - (5.0 - TUBE)).abs() < 1e-5, "{s:?}");
    }

    #[test]
    fn an_exact_knot_pins_the_string_and_the_profile_keeps_the_arc_there() {
        let times: Vec<f64> = (0..7).map(|k| 1000.0 + 48.0 * k as f64).collect();
        let arc = [0.0f32, 1.0, 2.0, 3.0, 5.0, 6.0, 7.0];
        let (mut lo, mut hi) = tube_walls(&arc);
        lo[2] = 2.0;
        hi[2] = 2.0;
        let s = taut_string(&times, &lo, &hi);
        assert!(s.iter().any(|b| b.0 == 2 && (b.1 - 2.0).abs() < 1e-6), "{s:?}");
        let p = Profile::smoothed(&s, &times, &[times[2]], 150.0, false, None);
        assert!((p.arc(times[2]) - 2.0).abs() < 1e-4, "arc at the exact knot {}", p.arc(times[2]));
        assert!((p.arc(times[6]) - 7.0).abs() < 1e-3, "arc at the end {}", p.arc(times[6]));
        for w in p.knots.windows(2) {
            assert!(w[1].0 >= w[0].0, "profile knots out of order: {:?}", p.knots);
        }
    }

    #[test]
    fn the_wind_up_waits_and_reaches_the_streams_pace_at_its_first_note() {
        // a second from the centre to the first note, then unit hops at 48 ms
        let times = [0.0f64, 1000.0, 1048.0, 1096.0, 1144.0];
        let arc = [0.0f32, 1.0, 2.0, 3.0, 4.0];
        let (mut lo, mut hi) = tube_walls(&arc);
        // the slow approach leaves the first note no slack
        lo[1] = 1.0;
        hi[1] = 1.0;
        let s = taut_string(&times, &lo, &hi);
        assert_eq!(s[1].0, 1, "{s:?}");
        let p = Profile::smoothed(&s, &times, &[], 150.0, true, None);
        let stream = 1.0 / 48.0;
        assert!(p.speed(500.0) < 0.1 * stream, "still crawling at 500 ms: {}", p.speed(500.0));
        assert!((p.speed(999.0) - stream).abs() < 0.1 * stream, "pace at the first note {} vs {stream}", p.speed(999.0));
        assert!((p.arc(1000.0) - 1.0).abs() < 1e-3, "arc at the first note {}", p.arc(1000.0));
        assert!((p.arc(1144.0) - 4.0).abs() < 1e-3, "arc at the end {}", p.arc(1144.0));
    }
}
