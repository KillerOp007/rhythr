//! Synthetic tests for the .rhr version gates, including the intermediate
//! version combinations no reference replay covers (extended + float time,
//! failTime + float time, int time without beatmap hash).
//!
//! The byte builder below exists ONLY to feed the parser in tests. It is
//! not part of any crate API, never touches the filesystem, and must never
//! be promoted into one (project hard rule #1: no replay writer).

use rhythia_formats::rhr::{Replay, V_BEATMAP_HASH};

struct B(Vec<u8>);

impl B {
    fn new() -> Self {
        B(Vec::new())
    }
    fn i32(mut self, v: i32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn i64(mut self, v: i64) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn f32(mut self, v: f32) -> Self {
        self.0.extend_from_slice(&v.to_le_bytes());
        self
    }
    fn u8(mut self, v: u8) -> Self {
        self.0.push(v);
        self
    }
    fn string(mut self, s: &str) -> Self {
        let mut len = s.len();
        loop {
            let mut byte = (len & 0x7F) as u8;
            len >>= 7;
            if len > 0 {
                byte |= 0x80;
            }
            self.0.push(byte);
            if len == 0 {
                break;
            }
        }
        self.0.extend_from_slice(s.as_bytes());
        self
    }
}

/// Header up to (and excluding) the version-gated block.
fn header(version: i32) -> B {
    B::new()
        .i32(version)
        .i64(638_000_000_000_000_000)
        .string("tester")
        .string("some_-_map")
        .i32(4711)
        .i32(0)
        .string("online_profile")
}

fn stats(b: B) -> B {
    // accuracy, hits, misses, points
    b.f32(50.0).i32(1).i32(1).f32(2.5)
}

#[test]
fn pre_extended_float_time_negated_y() {
    // v20260101 < all gates: no extended block, float time, y negated.
    let data = stats(header(20260101))
        .i32(2) // frame count
        .f32(100.5)
        .f32(0.25)
        .f32(-0.5)
        .f32(1.0)
        .u8(1)
        .f32(200.25)
        .f32(-1.0)
        .f32(0.75)
        .f32(0.875)
        .u8(0)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.version, 20260101);
    // Defaults for old versions:
    assert!(r.passed);
    assert_eq!(r.mods, "[]");
    assert_eq!(r.speed, 1.0);
    assert_eq!(r.fail_time_ms, -1);
    assert_eq!(r.beatmap_hash, "");
    // Float time keeps sub-ms precision; y comes back un-negated.
    assert_eq!(r.frames[0].ms, 100.5);
    assert_eq!(r.frames[0].y, 0.5);
    assert_eq!(r.frames[1].y, -0.75);
    assert!(r.frames[0].hit && !r.frames[1].hit);
    assert_eq!(r.trailing_bytes, 0);
}

#[test]
fn post_negate_pre_extended_keeps_y() {
    // 20260118 <= v20260120 < 20260125: y stored as-is, still no extended.
    let data = stats(header(20260120))
        .i32(1)
        .f32(10.0)
        .f32(0.0)
        .f32(-0.5)
        .f32(1.0)
        .u8(0)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.frames[0].y, -0.5);
    assert_eq!(r.speed, 1.0);
}

fn extended(b: B, speed: f32) -> B {
    // passed, mods, spin, speed, totalScore
    b.u8(1)
        .string("[\"mod_nofail\"]")
        .u8(0)
        .f32(speed)
        .i64(123_456)
}

#[test]
fn extended_with_float_time_no_fail_time() {
    // 20260125 <= v20260200 < 20260222: extended block present, float
    // time, no failTime, no hash — the combination rhr2mp4 never tests.
    let data = stats(extended(header(20260200), 1.25))
        .i32(1)
        .f32(99.75)
        .f32(0.1)
        .f32(0.2)
        .f32(1.0)
        .u8(1)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.mods, "[\"mod_nofail\"]");
    assert_eq!(r.speed, 1.25);
    assert_eq!(r.total_score, 123_456);
    assert_eq!(r.fail_time_ms, -1);
    assert_eq!(r.frames[0].ms, 99.75);
}

#[test]
fn fail_time_with_float_time() {
    // 20260222 <= v20260300 < 20260510: failTime present, time still f32.
    let data = stats(extended(header(20260300), 1.0))
        .i32(31_337) // failTime
        .i32(1)
        .f32(5.5)
        .f32(0.0)
        .f32(0.0)
        .f32(0.0)
        .u8(0)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.fail_time_ms, 31_337);
    assert!(r.failed());
    assert_eq!(r.frames[0].ms, 5.5);
    assert_eq!(r.beatmap_hash, "");
}

#[test]
fn int_time_without_hash() {
    // 20260510 <= v20260515 < 20260517: int32 time, still no hash.
    let data = stats(extended(header(20260515), 1.0))
        .i32(-1)
        .i32(1)
        .i32(1234)
        .f32(0.0)
        .f32(0.0)
        .f32(1.0)
        .u8(0)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.frames[0].ms, 1234.0);
    assert_eq!(r.beatmap_hash, "");
}

#[test]
fn zero_speed_normalizes_to_one() {
    let data = stats(extended(header(V_BEATMAP_HASH), 0.0))
        .i32(-1)
        .string("ab".repeat(32).as_str())
        .i32(0)
        .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.speed, 1.0);
    assert_eq!(r.beatmap_hash.len(), 64);
    assert!(r.frames.is_empty());
}

#[test]
fn long_string_uses_multibyte_varint() {
    // Player name > 127 bytes exercises the 2-byte LEB128 length prefix.
    let long_name = "x".repeat(300);
    let data = stats(
        B::new()
            .i32(20260101)
            .i64(0)
            .string(&long_name)
            .string("m")
            .i32(1)
            .i32(0)
            .string("online_profile"),
    )
    .i32(0)
    .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.player_name, long_name);
}

#[test]
fn truncated_file_reports_eof_not_panic() {
    let full = stats(extended(header(V_BEATMAP_HASH), 1.0))
        .i32(-1)
        .string(&"a".repeat(64))
        .i32(2)
        .i32(1)
        .f32(0.0)
        .f32(0.0)
        .f32(1.0)
        .u8(1)
        .i32(2)
        .f32(0.0)
        .f32(0.0)
        .f32(1.0)
        .u8(0)
        .0;
    for cut in [3, 20, full.len() / 2, full.len() - 1] {
        assert!(
            Replay::parse(&full[..cut]).is_err(),
            "cut at {cut} must error"
        );
    }
    assert!(Replay::parse(&full).is_ok());
}

#[test]
fn absurd_frame_count_errors_instead_of_allocating() {
    // A forged header claiming i32::MAX frames with no payload must be
    // rejected before Vec::with_capacity turns it into a ~51 GB request
    // (which aborts the whole process, uncatchable through Result).
    let data = stats(extended(header(V_BEATMAP_HASH), 1.0))
        .i32(-1)
        .string(&"a".repeat(64))
        .i32(i32::MAX)
        .0;
    assert!(matches!(
        Replay::parse(&data),
        Err(rhythia_formats::Error::BadFrameCount(_))
    ));
}

#[test]
fn frame_count_one_over_available_errors() {
    // Exactly one frame of payload present, header claims two.
    let data = stats(extended(header(V_BEATMAP_HASH), 1.0))
        .i32(-1)
        .string(&"a".repeat(64))
        .i32(2)
        .i32(0)
        .f32(0.0)
        .f32(0.0)
        .f32(1.0)
        .u8(1)
        .0;
    assert!(matches!(
        Replay::parse(&data),
        Err(rhythia_formats::Error::BadFrameCount(_))
    ));
}

#[test]
fn negative_frame_count_errors() {
    let data = stats(extended(header(V_BEATMAP_HASH), 1.0))
        .i32(-1)
        .string(&"a".repeat(64))
        .i32(-5)
        .0;
    assert!(matches!(
        Replay::parse(&data),
        Err(rhythia_formats::Error::BadFrameCount(-5))
    ));
}

#[test]
fn invalid_utf8_player_name_errors() {
    // String length 1, byte 0xFF — not valid UTF-8.
    let mut data = B::new().i32(20260101).i64(0).0;
    data.push(1); // varint length 1
    data.push(0xFF); // invalid UTF-8 byte
    assert!(matches!(
        Replay::parse(&data),
        Err(rhythia_formats::Error::InvalidUtf8 { .. })
    ));
}

#[test]
fn overlong_varint_string_length_errors() {
    // Five continuation bytes (>= 35 shift) trips the LEB128 cap.
    let mut data = B::new().i32(20260101).i64(0).0;
    data.extend_from_slice(&[0x80, 0x80, 0x80, 0x80, 0x80]);
    assert!(matches!(
        Replay::parse(&data),
        Err(rhythia_formats::Error::BadStringLength { .. })
    ));
}

#[test]
fn min_timestamp_does_not_overflow() {
    // A forged i64::MIN timestamp must not panic unix_ms() in debug builds.
    let mut b = B::new().i32(20260101);
    b.0.extend_from_slice(&i64::MIN.to_le_bytes());
    let data = stats(
        b.string("p")
            .string("m")
            .i32(1)
            .i32(0)
            .string("online_profile"),
    )
    .i32(0)
    .0;
    let r = Replay::parse(&data).unwrap();
    assert_eq!(r.unix_ms(), None);
}
