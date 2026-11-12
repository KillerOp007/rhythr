//! Parser for `.phxr` replays, written by the free community client (the
//! Godot 4 rewrite that follows Sound Space Plus).
//!
//! The byte layout is documented by that client's public source
//! (`Rhythia/Client`: `ReplayManager.cs` writes the file, `Replay.cs` reads
//! it back). That project is AGPL-3.0; this module takes the layout and the
//! game's rules from it and none of its code.
//!
//! Little-endian throughout:
//!
//! ```text
//! "phxr", u8 version (1)
//! f64 speed, f64 start (ms)
//! f64 approach rate, f64 approach distance, f64 fade in
//! u8 fade out, u8 pushback
//! f64 parallax, f64 fov, f64 note size, f64 sensitivity
//! u8 status (0 passed, 1 disqualified, anything else failed)
//! string mods (names joined by "_"), string map folder name
//! u64 note count of the map, string player
//! u64 frame count, then per frame: f32 song time (ms), f32 x, f32 y
//! u64 first judged note, u64 judged notes, then one byte per judged note
//! u64 skip count, then per skip: f32 song time (ms)
//! 32 bytes: SHA-256 of everything before them
//! ```
//!
//! A string is a u32 count followed by UTF-8 bytes. The count is the
//! string's length in UTF-16 code units, not in bytes, so the two differ
//! as soon as a name leaves ASCII.
//!
//! What sets the format apart from `.rhr`:
//!
//! * Frame times are song time already, sampled at most 60 times a second.
//!   A skip simply shows as a jump in them.
//! * Hits are not flagged on frames. Each judged note has a result byte:
//!   255 for a miss, otherwise how late the hit came in quarter
//!   milliseconds of real time. Turning those into flagged frames needs
//!   the note times, so that happens once the replay meets its map
//!   (`rhythia_sim::timebase::normalize`).
//! * Cursor and notes share one space: centre at the origin, x to the
//!   right, y up, one unit per grid cell. That is the renderer's own space.
//! * There is no score, no health and no date, and the player is always
//!   stored as "You".
//!
//! Like the other replay modules this is read-only by design: there is no
//! serializer and none may be added.

use sha2::{Digest, Sha256};

use crate::reader::Reader;
use crate::rhr::{civil_ticks, Frame, Judgments, Origin, Replay, V_PITCH_LOCK};
use crate::{Error, Result};

/// The first four bytes of every `.phxr` file.
pub const SIGNATURE: [u8; 4] = *b"phxr";
/// The only format version the client writes so far.
pub const VERSION: u8 = 1;
/// What a converted replay carries in [`Replay::mode`].
pub const MODE: &str = "community_client";

/// Highest speed taken at face value; the client's own range ends far
/// below this.
const MAX_SPEED: f64 = 10.0;
/// One frame on the wire: three f32.
const FRAME_BYTES: usize = 12;
const DIGEST_BYTES: usize = 32;
/// The client has no accounts yet and writes this for every player.
const PLACEHOLDER_PLAYER: &str = "You";

/// True when the data starts with the `.phxr` signature. Neither a `.rhr`
/// (an int32 date number) nor a `.sspre` (`Ss*R`) can start with it.
pub fn has_signature(data: &[u8]) -> bool {
    data.len() >= SIGNATURE.len() && data[..SIGNATURE.len()] == SIGNATURE
}

/// How the client itself rated the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Passed,
    /// Finished alive, but paused mid-map or started from the middle.
    Disqualified,
    /// Health ran out, or the player quit or restarted.
    Failed,
}

/// The player's view settings as stored in the header.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub approach_rate: f64,
    pub approach_distance: f64,
    pub fade_in: f64,
    pub fade_out: bool,
    pub pushback: bool,
    pub parallax: f64,
    pub fov: f64,
    pub note_size: f64,
    pub sensitivity: f64,
}

/// One recorded cursor sample.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sample {
    /// Song time in ms.
    pub ms: f32,
    pub x: f32,
    pub y: f32,
}

/// A fully parsed `.phxr` replay.
#[derive(Debug, Clone, PartialEq)]
pub struct Phxr {
    pub speed: f64,
    /// Song time the run started from, in ms.
    pub start_from_ms: f64,
    pub view: ViewSettings,
    pub status: Status,
    /// Modifier names as stored, e.g. `NoFail`. The spin camera is stored
    /// among them as `Spin`.
    pub mods: Vec<String>,
    /// Name of the map's folder in the client's `maps` directory.
    pub map_id: String,
    pub map_note_count: u64,
    pub player: String,
    pub samples: Vec<Sample>,
    pub first_note: u64,
    /// One byte per judged note; see [`Judgments`].
    pub results: Vec<u8>,
    /// Song times at which the player skipped a break.
    pub skips: Vec<f32>,
    /// The SHA-256 trailer matches the bytes before it.
    pub checksum_ok: bool,
    /// Bytes left after the trailer. Always 0 for well-formed files.
    pub trailing_bytes: usize,
}

fn bad(msg: impl Into<String>) -> Error {
    Error::BadPhxr(msg.into())
}

fn sized_string(r: &mut Reader) -> Result<String> {
    let units = r.u32()? as usize;
    r.utf16_counted_string(units)
}

/// A count from the file, checked against the bytes that are left. A
/// forged count must not drive an allocation.
fn count(r: &mut Reader, item_bytes: usize, what: &str) -> Result<usize> {
    let declared = r.u64()?;
    let fits = (r.remaining() / item_bytes) as u64;
    if declared > fits {
        return Err(bad(format!(
            "{what} count {declared} is more than the file holds"
        )));
    }
    Ok(declared as usize)
}

impl Phxr {
    pub fn parse(data: &[u8]) -> Result<Phxr> {
        if !has_signature(data) {
            return Err(bad("missing the phxr signature"));
        }
        let mut r = Reader::new(data);
        r.bytes(SIGNATURE.len())?;
        let version = r.u8()?;
        if version != VERSION {
            return Err(bad(format!(
                "format version {version} is not one this build reads ({VERSION})"
            )));
        }
        let speed = r.f64()?;
        let start_from_ms = r.f64()?;
        let (approach_rate, approach_distance, fade_in) = (r.f64()?, r.f64()?, r.f64()?);
        let (fade_out, pushback) = (r.bool()?, r.bool()?);
        let view = ViewSettings {
            approach_rate,
            approach_distance,
            fade_in,
            fade_out,
            pushback,
            parallax: r.f64()?,
            fov: r.f64()?,
            note_size: r.f64()?,
            sensitivity: r.f64()?,
        };
        let status = match r.u8()? {
            0 => Status::Passed,
            1 => Status::Disqualified,
            _ => Status::Failed,
        };
        let mods = sized_string(&mut r)?
            .split('_')
            .filter(|m| !m.is_empty())
            .map(str::to_string)
            .collect();
        let map_id = sized_string(&mut r)?;
        let map_note_count = r.u64()?;
        let player = sized_string(&mut r)?;

        let frame_count = count(&mut r, FRAME_BYTES, "frame")?;
        if frame_count == 0 {
            // The client reserves the count when the run starts and writes
            // the frames when it ends, so zero means it never got there
            // (a crash, a kill).
            return Err(bad("the recording was never finished (it holds no frames)"));
        }
        let mut samples = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            samples.push(Sample {
                ms: r.f32()?,
                x: r.f32()?,
                y: r.f32()?,
            });
        }

        let first_note = r.u64()?;
        let judged = count(&mut r, 1, "note result")?;
        let results = r.bytes(judged)?.to_vec();
        let skip_count = count(&mut r, 4, "skip")?;
        let mut skips = Vec::with_capacity(skip_count);
        for _ in 0..skip_count {
            skips.push(r.f32()?);
        }

        let body_end = r.position();
        let checksum_ok = r.remaining() >= DIGEST_BYTES
            && Sha256::digest(&data[..body_end]).as_slice() == r.bytes(DIGEST_BYTES)?;

        Ok(Phxr {
            speed,
            start_from_ms,
            view,
            status,
            mods,
            map_id,
            map_note_count,
            player,
            samples,
            first_note,
            results,
            skips,
            checksum_ok,
            trailing_bytes: r.remaining(),
        })
    }

    /// The run in the renderer's own replay model.
    ///
    /// Frames and positions carry over unchanged. No frame is flagged as a
    /// hit yet and health is full throughout: both follow from the note
    /// results in [`Origin::Phxr`], which need the map to become frames.
    pub fn to_replay(&self) -> Replay {
        // A file can claim anything; a speed that is not a positive finite
        // number would poison every time computed from it.
        let speed = if self.speed.is_finite() && self.speed > 0.0 && self.speed <= MAX_SPEED {
            self.speed as f32
        } else {
            1.0
        };
        let start_from_ms = if self.start_from_ms.is_finite() {
            self.start_from_ms.round().clamp(0.0, f64::from(i32::MAX)) as i32
        } else {
            0
        };
        let has = |name: &str| self.mods.iter().any(|m| m == name);

        // A sample that is not a number cannot be placed on a timeline or
        // a playfield.
        let frames = self
            .samples
            .iter()
            .filter(|s| s.ms.is_finite() && s.x.is_finite() && s.y.is_finite())
            .map(|s| Frame {
                ms: f64::from(s.ms),
                x: s.x,
                y: s.y,
                health: 1.0,
                hit: false,
            })
            .collect();

        let hits = self
            .results
            .iter()
            .filter(|&&r| r != Judgments::MISS)
            .count();
        let judged = self.results.len();
        let as_count = |n: usize| i32::try_from(n).unwrap_or(i32::MAX);

        Replay {
            version: V_PITCH_LOCK,
            timestamp_ticks: 0,
            player_name: if self.player == PLACEHOLDER_PLAYER {
                String::new()
            } else {
                self.player.clone()
            },
            legacy_map_id: self.map_id.clone(),
            map_id: 0,
            start_from_ms,
            mode: MODE.to_string(),
            passed: self.status != Status::Failed,
            mods: mods_json(&self.mods),
            spin: has("Spin"),
            speed,
            total_score: 0,
            accuracy_pct: if judged > 0 {
                hits as f32 / judged as f32 * 100.0
            } else {
                100.0
            },
            hits: as_count(hits),
            misses: as_count(judged - hits),
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames,
            trailing_bytes: self.trailing_bytes,
            origin: Origin::Phxr(Judgments {
                first_note: usize::try_from(self.first_note).unwrap_or(usize::MAX),
                results: self.results.clone(),
                map_notes: self.map_note_count,
                flip: (has("HFlip"), has("VFlip")),
                checksum_ok: self.checksum_ok,
                off_target_hits: None,
            }),
        }
    }
}

/// When the run was recorded, from the name the client gives the file:
/// `<map>_<device id>_<year>-<month>-<day>T<hour>_<minute>_<second>`, the
/// date being UTC. The file itself stores no date. Returns .NET ticks, or
/// None when the name does not end in such a date (a renamed file).
pub fn recorded_ticks(file_stem: &str) -> Option<i64> {
    const STAMP_LEN: usize = "2026-01-31T23_59_59".len();
    let stamp = file_stem.get(file_stem.len().checked_sub(STAMP_LEN)?..)?;
    let (date, time) = stamp.split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let mut t = time.split('_').map(|p| p.parse::<i64>().ok());
    let ticks = civil_ticks(
        (d.next()??, d.next()??, d.next()??),
        (t.next()??, t.next()??, t.next()??),
    )?;
    (d.next().is_none() && t.next().is_none()).then_some(ticks)
}

/// The stored modifier names as the JSON array `.rhr` uses.
///
/// The two flips are one mirror mod with an axis there; the renderer finds
/// the axis from the run itself, so one flip maps to `mod_mirror` and both
/// to `mod_double_mirror`. The spin camera is a replay field, not a mod.
/// Names with no `.rhr` counterpart pass through unchanged so they surface
/// as unsupported instead of vanishing.
fn mods_json(names: &[String]) -> String {
    let has = |n: &str| names.iter().any(|m| m == n);
    let mut out: Vec<String> = Vec::new();
    match (has("HFlip"), has("VFlip")) {
        (true, true) => out.push("mod_double_mirror".into()),
        (true, false) | (false, true) => out.push("mod_mirror".into()),
        (false, false) => {}
    }
    for name in names {
        let mapped = match name.as_str() {
            "HFlip" | "VFlip" | "Spin" => continue,
            "NoFail" => "mod_nofail",
            "Ghost" => "mod_ghost",
            "Chaos" => "mod_chaos",
            other => other,
        };
        if !out.iter().any(|m| m == mapped) {
            out.push(mapped.to_string());
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds file bytes the way the client's recorder lays them out.
    /// Test-only on purpose: the crate itself never produces replay data.
    struct Fixture {
        speed: f64,
        start: f64,
        status: u8,
        mods: &'static str,
        map: &'static str,
        map_notes: u64,
        player: &'static str,
        frames: Vec<(f32, f32, f32)>,
        first_note: u64,
        results: Vec<u8>,
        skips: Vec<f32>,
    }

    impl Fixture {
        fn new() -> Fixture {
            Fixture {
                speed: 1.0,
                start: 0.0,
                status: 0,
                mods: "",
                map: "some_mapper_-_some_song",
                map_notes: 3,
                player: "You",
                frames: vec![
                    (-1000.0, 0.0, 0.0),
                    (500.0, 1.0, -1.0),
                    (1500.0, -0.5, 0.25),
                ],
                first_note: 0,
                results: vec![0, 255, 40],
                skips: vec![],
            }
        }

        /// Everything before the checksum.
        fn body(&self) -> Vec<u8> {
            let mut b = SIGNATURE.to_vec();
            b.push(VERSION);
            for v in [self.speed, self.start, 50.0, 50.0, 15.0] {
                b.extend(v.to_le_bytes());
            }
            b.extend([1u8, 0u8]); // fade out, pushback
            for v in [0.1f64, 70.0, 0.875, 0.5] {
                b.extend(v.to_le_bytes());
            }
            b.push(self.status);
            for text in [self.mods, self.map] {
                b.extend((text.encode_utf16().count() as u32).to_le_bytes());
                b.extend(text.as_bytes());
            }
            b.extend(self.map_notes.to_le_bytes());
            b.extend((self.player.encode_utf16().count() as u32).to_le_bytes());
            b.extend(self.player.as_bytes());
            b.extend((self.frames.len() as u64).to_le_bytes());
            for (ms, x, y) in &self.frames {
                for v in [ms, x, y] {
                    b.extend(v.to_le_bytes());
                }
            }
            b.extend(self.first_note.to_le_bytes());
            b.extend((self.results.len() as u64).to_le_bytes());
            b.extend(&self.results);
            b.extend((self.skips.len() as u64).to_le_bytes());
            for s in &self.skips {
                b.extend(s.to_le_bytes());
            }
            b
        }

        fn bytes(&self) -> Vec<u8> {
            let mut b = self.body();
            let digest = Sha256::digest(&b);
            b.extend(digest);
            b
        }
    }

    fn replay(f: &Fixture) -> Replay {
        Phxr::parse(&f.bytes()).unwrap().to_replay()
    }

    fn judgments(r: &Replay) -> &Judgments {
        match &r.origin {
            Origin::Phxr(j) => j,
            other => panic!("not a phxr origin: {other:?}"),
        }
    }

    #[test]
    fn a_file_yields_its_header_frames_results_and_skips() {
        let f = Fixture {
            speed: 1.25,
            start: 12_000.0,
            status: 1,
            mods: "NoFail_Ghost",
            skips: vec![4000.5],
            first_note: 7,
            ..Fixture::new()
        };
        let p = Phxr::parse(&f.bytes()).unwrap();
        assert_eq!((p.speed, p.start_from_ms), (1.25, 12_000.0));
        assert_eq!(p.status, Status::Disqualified);
        assert_eq!(p.mods, ["NoFail", "Ghost"]);
        assert_eq!(p.map_id, "some_mapper_-_some_song");
        assert_eq!((p.map_note_count, p.first_note), (3, 7));
        assert_eq!(p.player, "You");
        assert_eq!(
            (
                p.view.approach_rate,
                p.view.fade_out,
                p.view.pushback,
                p.view.fov
            ),
            (50.0, true, false, 70.0)
        );
        assert_eq!(p.samples.len(), 3);
        assert_eq!(
            p.samples[1],
            Sample {
                ms: 500.0,
                x: 1.0,
                y: -1.0
            }
        );
        assert_eq!(p.results, [0, 255, 40]);
        assert_eq!(p.skips, [4000.5]);
        assert!(p.checksum_ok);
        assert_eq!(p.trailing_bytes, 0);
    }

    #[test]
    fn the_replay_keeps_times_and_positions_and_counts_the_results() {
        let r = replay(&Fixture::new());
        assert_eq!(r.frames.len(), 3);
        assert_eq!(
            r.frames[1],
            Frame {
                ms: 500.0,
                x: 1.0,
                y: -1.0,
                health: 1.0,
                hit: false
            }
        );
        assert_eq!((r.hits, r.misses), (2, 1));
        assert!((r.accuracy_pct - 200.0 / 3.0).abs() < 1e-4);
        assert_eq!(
            r.flagged_frames(),
            0,
            "hits become frames only with the map"
        );
        assert_eq!(r.legacy_map_id, "some_mapper_-_some_song");
        assert_eq!((r.map_id, r.speed, r.mode.as_str()), (0, 1.0, MODE));
        assert!(r.passed && !r.failed());
        let j = judgments(&r);
        assert_eq!((j.first_note, j.map_notes), (0, 3));
        assert_eq!(j.results, [0, 255, 40]);
        assert!(j.checksum_ok);
        assert_eq!(j.off_target_hits, None);
    }

    #[test]
    fn a_result_byte_counts_lateness_in_quarter_milliseconds() {
        assert_eq!(Judgments::lateness_ms(0), Some(0.0));
        assert_eq!(Judgments::lateness_ms(41), Some(10.25));
        assert_eq!(Judgments::lateness_ms(220), Some(55.0));
        assert_eq!(Judgments::lateness_ms(255), None);
    }

    #[test]
    fn replay_parse_recognises_the_format_by_content() {
        let r = Replay::parse(&Fixture::new().bytes()).unwrap();
        assert!(matches!(r.origin, Origin::Phxr(_)));
    }

    #[test]
    fn the_placeholder_player_is_not_a_name() {
        assert_eq!(replay(&Fixture::new()).player_name, "");
        let named = Fixture {
            player: "killerop",
            ..Fixture::new()
        };
        assert_eq!(replay(&named).player_name, "killerop");
    }

    #[test]
    fn mods_map_to_the_names_the_renderer_knows() {
        let mods = |mods: &'static str| {
            replay(&Fixture {
                mods,
                ..Fixture::new()
            })
        };
        assert_eq!(mods("").mods, "[]");
        assert_eq!(mods("NoFail_Ghost").mods, r#"["mod_nofail","mod_ghost"]"#);
        assert_eq!(mods("HFlip").mods, r#"["mod_mirror"]"#);
        assert_eq!(mods("VFlip").mods, r#"["mod_mirror"]"#);
        assert_eq!(mods("VFlip_HFlip").mods, r#"["mod_double_mirror"]"#);
        assert_eq!(
            mods("Chaos_Earthquake").mods,
            r#"["mod_chaos","Earthquake"]"#,
            "an unknown name passes through so it shows as unsupported"
        );
        let spin = mods("NoFail_Spin");
        assert!(spin.spin, "the spin camera is stored among the mods");
        assert_eq!(spin.mods, r#"["mod_nofail"]"#);
        assert!(!mods("NoFail").spin);
        assert_eq!(judgments(&mods("HFlip")).flip, (true, false));
        assert_eq!(judgments(&mods("VFlip")).flip, (false, true));
    }

    #[test]
    fn the_status_decides_whether_the_run_passed() {
        let status = |status: u8| {
            replay(&Fixture {
                status,
                ..Fixture::new()
            })
            .passed
        };
        assert!(status(0));
        assert!(status(1), "disqualified is still a finished run");
        assert!(!status(2));
    }

    #[test]
    fn a_changed_byte_breaks_the_checksum_without_breaking_the_parse() {
        let f = Fixture::new();
        let mut data = f.bytes();
        let miss = f.body().len() - 8 - 2; // the 255 among the three results
        assert_eq!(data[miss], 255);
        data[miss] = 0;
        let p = Phxr::parse(&data).unwrap();
        assert!(!p.checksum_ok);
        assert_eq!(p.results, [0, 0, 40]);
        assert!(!judgments(&p.to_replay()).checksum_ok);
    }

    #[test]
    fn a_missing_trailer_and_extra_bytes_are_both_noticed() {
        let f = Fixture::new();
        let cut = Phxr::parse(&f.body()).unwrap();
        assert!(!cut.checksum_ok);
        assert_eq!(cut.trailing_bytes, 0);

        let mut long = f.bytes();
        long.extend([1, 2, 3]);
        let p = Phxr::parse(&long).unwrap();
        assert!(p.checksum_ok);
        assert_eq!(p.trailing_bytes, 3);
        assert_eq!(p.to_replay().trailing_bytes, 3);
    }

    #[test]
    fn a_name_beyond_ascii_is_read_by_characters_not_bytes() {
        let f = Fixture {
            map: "mäpper_-_sông_🎵",
            ..Fixture::new()
        };
        let p = Phxr::parse(&f.bytes()).unwrap();
        assert_eq!(p.map_id, "mäpper_-_sông_🎵");
        assert_eq!(p.map_note_count, 3);
        assert!(p.checksum_ok);
    }

    #[test]
    fn an_unfinished_recording_is_refused_with_a_reason() {
        let f = Fixture {
            frames: vec![],
            ..Fixture::new()
        };
        let err = Phxr::parse(&f.bytes()).unwrap_err().to_string();
        assert!(err.contains("never finished"), "{err}");
    }

    #[test]
    fn a_forged_count_is_refused_before_anything_is_allocated() {
        let f = Fixture::new();
        let body = f.body();
        // The frame count sits right after the player string.
        let frames_at = body.len() - (8 + 3 * 12) - (8 + 8 + 3) - 8;
        let mut data = f.bytes();
        data[frames_at..frames_at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        let err = Phxr::parse(&data).unwrap_err().to_string();
        assert!(err.contains("frame count"), "{err}");
    }

    #[test]
    fn other_versions_and_other_formats_are_refused() {
        let mut data = Fixture::new().bytes();
        data[4] = 2;
        assert!(Phxr::parse(&data)
            .unwrap_err()
            .to_string()
            .contains("version 2"));
        assert!(Phxr::parse(b"Ss*R\x04\x00").is_err());
        assert!(!has_signature(b"phx"));
    }

    #[test]
    fn a_speed_or_sample_that_is_not_a_number_is_not_carried_over() {
        let odd = |speed: f64| {
            replay(&Fixture {
                speed,
                ..Fixture::new()
            })
            .speed
        };
        assert_eq!(odd(f64::NAN), 1.0);
        assert_eq!(odd(-2.0), 1.0);
        assert_eq!(odd(1e9), 1.0);
        assert_eq!(odd(1.45), 1.45);

        let r = replay(&Fixture {
            frames: vec![
                (0.0, 0.0, 0.0),
                (f32::NAN, 0.0, 0.0),
                (20.0, f32::INFINITY, 0.0),
                (40.0, 1.0, 1.0),
            ],
            ..Fixture::new()
        });
        assert_eq!(
            r.frames.iter().map(|f| f.ms).collect::<Vec<_>>(),
            [0.0, 40.0]
        );
    }

    #[test]
    fn the_recording_date_comes_from_the_file_name() {
        // 2026-11-10 12:34:56 UTC
        let ticks = recorded_ticks("some_song_0123abcd_2026-11-10T12_34_56").unwrap();
        let unix_ms = (ticks - crate::rhr::UNIX_EPOCH_TICKS) / 10_000;
        assert_eq!(unix_ms, 1_794_314_096_000);
        assert_eq!(recorded_ticks("renamed"), None);
        assert_eq!(recorded_ticks("some_song_2026-13-40T99_99_99"), None);
        assert_eq!(recorded_ticks(""), None);
    }
}
