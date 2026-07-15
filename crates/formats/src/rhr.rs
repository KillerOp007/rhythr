//! Parser for Rhythia `.rhr` replay files (Steam client format).
//!
//! Little-endian throughout; strings are .NET `Write7BitEncodedInt` length
//! (LEB128 varint) + UTF-8. The format is versioned by an int32 date-number
//! header. Layout verified byte-exact against 311 real replays and three
//! independent open-source parsers (rhythia.com web bundle `parseRhr`,
//! yo-ru/rhrParse, gerhaarrd/rhr2mp4 — all MIT or public reference).
//!
//! This module is read-only by design (project hard rule #1): there is no
//! serializer and none may be added.

use std::path::Path;

use crate::reader::Reader;
use crate::Result;

/// Before this version, frame `y` is stored negated.
pub const V_NEGATE_Y: i32 = 20260118;
/// Adds passed/mods/spin/speed/totalScore.
pub const V_EXTENDED: i32 = 20260125;
/// Adds failTime.
pub const V_FAIL_TIME: i32 = 20260222;
/// Frame time becomes int32 (float32 before).
pub const V_INT32_TIME: i32 = 20260510;
/// Adds beatmapHash.
pub const V_BEATMAP_HASH: i32 = 20260517;

/// Offset between .NET DateTime ticks (100 ns since 0001-01-01) and the
/// Unix epoch, in ticks.
const UNIX_EPOCH_TICKS: i64 = 621_355_968_000_000_000;

/// Bytes one frame occupies on the wire: 4 (time, i32 or f32) + 4 (x) +
/// 4 (y) + 4 (health) + 1 (hit flag).
const WIRE_FRAME_BYTES: usize = 17;

/// One recorded input frame (~60/s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frame {
    /// Song time in ms. Stored as f64 so float-time replays (pre-20260510)
    /// keep their sub-millisecond precision instead of being truncated.
    pub ms: f64,
    /// Cursor position in cursor space (observed ±1.369; ±~1.519 under
    /// hardrock, whose playfield is ~1.11× wider).
    pub x: f32,
    pub y: f32,
    /// Player health 0..1 (regenerates in 1/8 steps; drops to 0 on fail).
    pub health: f32,
    /// True on the frame where a note was hit; Σ(hit) == header hits.
    pub hit: bool,
}

/// A fully parsed `.rhr` replay.
#[derive(Debug, Clone)]
pub struct Replay {
    pub version: i32,
    /// .NET DateTime ticks (100 ns since 0001-01-01 UTC).
    pub timestamp_ticks: i64,
    pub player_name: String,
    /// Map slug, e.g. "mm1678yt_-_dragonforce_-_through_the_fire_and_flames".
    pub legacy_map_id: String,
    /// Online map id on rhythia.com (map resolution key).
    pub map_id: i32,
    /// Start offset in ms.
    pub start_from_ms: i32,
    /// Observed: "online_profile".
    pub mode: String,
    pub passed: bool,
    /// JSON array as stored, e.g. `["mod_hardrock"]`.
    pub mods: String,
    pub spin: bool,
    /// Playback speed; 1.0 for pre-extended versions and when stored as 0.
    pub speed: f32,
    pub total_score: i64,
    /// Header accuracy 0–100 as stored by the game.
    pub accuracy_pct: f32,
    pub hits: i32,
    pub misses: i32,
    /// Awarded SP — float32 in the wire format.
    pub points: f32,
    /// −1 when the run was not failed.
    pub fail_time_ms: i32,
    /// 64-hex map hash (empty before 20260517).
    pub beatmap_hash: String,
    pub frames: Vec<Frame>,
    /// Bytes left after the last frame. Always 0 for well-formed files;
    /// nonzero is surfaced by the integrity check.
    pub trailing_bytes: usize,
}

impl Replay {
    pub fn from_path(path: impl AsRef<Path>) -> Result<Replay> {
        let data = std::fs::read(path)?;
        Replay::parse(&data)
    }

    pub fn parse(data: &[u8]) -> Result<Replay> {
        let mut r = Reader::new(data);

        let version = r.i32()?;
        let timestamp_ticks = r.i64()?;
        let player_name = r.string()?;
        let legacy_map_id = r.string()?;
        let map_id = r.i32()?;
        let start_from_ms = r.i32()?;
        let mode = r.string()?;

        // Defaults for pre-extended versions, per the official parser.
        let (mut passed, mut mods, mut spin) = (true, String::from("[]"), false);
        let (mut speed, mut total_score) = (1.0f32, 0i64);
        if version >= V_EXTENDED {
            passed = r.bool()?;
            mods = r.string()?;
            spin = r.bool()?;
            let raw_speed = r.f32()?;
            // The official parser treats a stored 0 as 1.0.
            speed = if raw_speed > 0.0 { raw_speed } else { 1.0 };
            total_score = r.i64()?;
        }

        let accuracy_pct = r.f32()?;
        let hits = r.i32()?;
        let misses = r.i32()?;
        let points = r.f32()?;

        let fail_time_ms = if version >= V_FAIL_TIME { r.i32()? } else { -1 };
        let beatmap_hash = if version >= V_BEATMAP_HASH {
            r.string()?
        } else {
            String::new()
        };

        let frame_count = r.i32()?;
        // A frame is exactly WIRE_FRAME_BYTES on the wire, so a count that
        // exceeds the bytes left cannot be honest. Rejecting it here keeps
        // a forged header from driving the allocation below: Vec reserves
        // eagerly, and an out-of-memory abort is a process-wide SIGABRT
        // that no caller can catch through our Result.
        let max_frames = (r.remaining() / WIRE_FRAME_BYTES) as i64;
        if frame_count < 0 || i64::from(frame_count) > max_frames {
            return Err(crate::Error::BadFrameCount(frame_count.into()));
        }

        let int32_time = version >= V_INT32_TIME;
        let negate_y = version < V_NEGATE_Y;

        let mut frames = Vec::with_capacity(frame_count as usize);
        for _ in 0..frame_count {
            let ms = if int32_time {
                f64::from(r.i32()?)
            } else {
                f64::from(r.f32()?)
            };
            let x = r.f32()?;
            let mut y = r.f32()?;
            if negate_y {
                y = -y;
            }
            let health = r.f32()?;
            let hit = r.u8()? != 0;
            frames.push(Frame {
                ms,
                x,
                y,
                health,
                hit,
            });
        }

        Ok(Replay {
            version,
            timestamp_ticks,
            player_name,
            legacy_map_id,
            map_id,
            start_from_ms,
            mode,
            passed,
            mods,
            spin,
            speed,
            total_score,
            accuracy_pct,
            hits,
            misses,
            points,
            fail_time_ms,
            beatmap_hash,
            frames,
            trailing_bytes: r.remaining(),
        })
    }

    pub fn failed(&self) -> bool {
        self.fail_time_ms >= 0
    }

    /// Play timestamp as Unix milliseconds, or None when the stored ticks
    /// are outside the range .NET DateTime can represent (a forged header
    /// can hold any i64, and the plain subtraction would overflow).
    pub fn unix_ms(&self) -> Option<i64> {
        if self.timestamp_ticks <= 0 {
            return None;
        }
        self.timestamp_ticks
            .checked_sub(UNIX_EPOCH_TICKS)
            .map(|ticks| ticks / 10_000)
    }

    /// Song time of the last recorded frame in ms.
    pub fn length_ms(&self) -> f64 {
        self.frames.last().map_or(0.0, |f| f.ms)
    }

    /// Number of frames with the hit flag set (must equal `hits`).
    pub fn flagged_frames(&self) -> u32 {
        self.frames.iter().filter(|f| f.hit).count() as u32
    }
}
