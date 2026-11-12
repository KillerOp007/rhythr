//! Parser for legacy Sound Space Plus `.sspre` replays.
//!
//! Sound Space Plus is the free Godot 3 client Rhythia grew out of. Its
//! replay format is defined by the game's own source (MIT,
//! `Rhythia/sound-space-plus`, `scripts/content/game/Replay.gd`), which this
//! module follows field by field: little-endian, 32-bit floats, strings as
//! newline-terminated lines.
//!
//! The format differs from `.rhr` in three ways that matter to a renderer:
//!
//! * Cursor samples are stamped in WALL-CLOCK milliseconds since the run
//!   began, not in song time. Pauses, the unpause run-up and skips are
//!   separate signals, and song time has to be rebuilt from them the way
//!   the game's own playback does (`NoteManager.gd`, the `rms`/`ms` pair).
//! * Hits and misses are recorded per note index instead of as a flag on a
//!   frame, with no timestamp of their own: the game writes the result and
//!   then forces a cursor sample in the same game frame.
//! * There is no player name, no score and no health. Since format version
//!   3 the file carries the player's view settings instead.
//!
//! Like the `.rhr` module this is read-only by design: there is no
//! serializer and none may be added.

use crate::reader::Reader;
use crate::rhr::{civil_ticks, Frame, Origin, Replay, Skip, V_PITCH_LOCK};
use crate::{Error, Result};

/// `Ss*R`, the first four bytes of every `.sspre` file.
pub const SIGNATURE: [u8; 4] = [0x53, 0x73, 0x2A, 0x52];
/// The newest format version the game writes (`current_sv`).
pub const NEWEST_VERSION: u16 = 4;
/// What a converted replay carries in [`Replay::mode`].
pub const MODE: &str = "sound_space_plus";

/// Song time the game rewinds by when an unpause begins, at 1.00x; the
/// player holds the key through this run-up (`NoteManager.gd`, 750 ms
/// times the speed multiplier).
const UNPAUSE_RUN_UP_MS: f64 = 750.0;
/// Lead-in before the first note at 1.00x: the song clock starts at
/// `start_offset - 3000 * speed` (`NoteManager.gd:16`).
const LEAD_IN_MS: f64 = 3000.0;

/// Highest speed taken at face value. The game's presets end at 1.45x and
/// its custom speed is a slider; ten times is far outside both.
const MAX_SPEED: f32 = 10.0;

const RS_CURSOR: u8 = 0;
const RS_HIT: u8 = 1;
const RS_MISS: u8 = 2;
const RS_PAUSE: u8 = 3;
const RS_GIVEUP: u8 = 4;
const RS_END: u8 = 5;
const RS_START_UNPAUSE: u8 = 6;
const RS_CANCEL_UNPAUSE: u8 = 7;
const RS_FINISH_UNPAUSE: u8 = 8;
const RS_SKIP: u8 = 9;

/// True when the data starts with the `.sspre` signature. A `.rhr` file
/// starts with an int32 date number (20260118 and later), which can never
/// spell these four bytes, so the two formats are told apart by content
/// rather than by file name.
pub fn has_signature(data: &[u8]) -> bool {
    data.len() >= SIGNATURE.len() && data[..SIGNATURE.len()] == SIGNATURE
}

/// A signal that moves the song clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSignal {
    Pause,
    GiveUp,
    StartUnpause,
    CancelUnpause,
    FinishUnpause,
    Skip,
}

/// One recorded signal, in file order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// Cursor position in grid cells: (1, 1) is the centre, x grows to the
    /// right and y grows DOWNWARD (`Cursor.gd`, `rpos`).
    Cursor {
        rms: u32,
        x: f32,
        y: f32,
    },
    /// Note index into the map's note list.
    Hit {
        note: u32,
    },
    Miss {
        note: u32,
    },
    Clock {
        rms: u32,
        signal: ClockSignal,
    },
}

/// The player's view settings, stored since format version 3.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewSettings {
    pub approach_rate: f32,
    pub spawn_distance: f32,
    pub fade_length: f32,
    pub parallax: f32,
    pub ui_parallax: f32,
    pub grid_parallax: f32,
    pub fov: f32,
    pub cam_unlock: bool,
    pub edge_drift: f32,
}

/// The run's rules, decoded from the state line (`Rhythia.gd`,
/// `generate_pb_str` and `parse_pb_str`).
#[derive(Debug, Clone, PartialEq)]
pub struct State {
    /// Playback speed multiplier; 1.0 when the line names none.
    pub speed: f32,
    pub hit_window_ms: Option<f32>,
    pub hitbox: Option<f32>,
    /// Song time the run started from, in ms.
    pub start_offset_ms: f64,
    /// The `m_` tokens as stored, e.g. `m_hardrock`.
    pub mods: Vec<String>,
}

/// A fully parsed `.sspre` replay.
///
/// Format version 4 stores the recording machine's hardware id in the
/// header. It is read past and deliberately not kept: nothing in a
/// renderer needs it, and it must not end up in a log, a preview or a
/// video.
#[derive(Debug, Clone, PartialEq)]
pub struct Sspre {
    pub version: u16,
    /// `<song id>.<year>-<month>-<day>_<hour>-<minute>-<second>`.
    pub id: String,
    /// The state line as stored; see [`Sspre::state`].
    pub state_line: String,
    pub view: Option<ViewSettings>,
    /// Wall-clock time of the last cursor sample, in ms.
    pub end_ms: u32,
    /// Signal count from the header, the END signal included.
    pub declared_signals: u32,
    pub events: Vec<Event>,
    /// Bytes left after the END signal. Always 0 for well-formed files.
    pub trailing_bytes: usize,
}

fn bad(msg: impl Into<String>) -> Error {
    Error::BadSspre(msg.into())
}

impl Sspre {
    pub fn parse(data: &[u8]) -> Result<Sspre> {
        let mut r = Reader::new(data);
        if !has_signature(data) {
            return Err(bad("missing the Ss*R signature"));
        }
        for _ in 0..SIGNATURE.len() {
            r.u8()?;
        }
        let version = r.u16()?;
        if version == 0 || version > NEWEST_VERSION {
            return Err(bad(format!(
                "format version {version} is not one this build reads (1 to {NEWEST_VERSION})"
            )));
        }
        r.u64()?; // reserved
        if version >= 4 {
            r.line()?; // hardware id, never kept
        }
        let id = r.line()?;
        let state_line = r.line()?;
        let view = if version >= 3 {
            Some(ViewSettings {
                approach_rate: r.f32()?,
                spawn_distance: r.f32()?,
                fade_length: r.f32()?,
                parallax: r.f32()?,
                ui_parallax: r.f32()?,
                grid_parallax: r.f32()?,
                fov: r.f32()?,
                cam_unlock: r.u8()? == 1,
                edge_drift: r.f32()?,
            })
        } else {
            None
        };
        r.u8()?; // reserved
        let end_ms = r.u32()?;
        let declared_signals = r.u32()?;
        if declared_signals == 0 {
            // The game patches end time and count in when the run ends, so
            // zero means the recording was cut off (a crash, a kill).
            return Err(bad(
                "the recording was never finished (signal count is zero)",
            ));
        }

        // Never sized from the header: a forged count must not drive an
        // allocation. The loop is bounded by the data itself, every signal
        // being at least five bytes.
        let mut events = Vec::new();
        loop {
            let kind = r.u8()?;
            let clock = |signal| (version >= 2).then_some(signal);
            let signal = match kind {
                RS_END => break,
                RS_CURSOR => {
                    let rms = r.u32()?;
                    let (x, y) = (r.f32()?, r.f32()?);
                    events.push(Event::Cursor { rms, x, y });
                    continue;
                }
                RS_HIT if version >= 2 => {
                    events.push(Event::Hit { note: r.u32()? });
                    continue;
                }
                RS_MISS if version >= 2 => {
                    events.push(Event::Miss { note: r.u32()? });
                    continue;
                }
                RS_PAUSE => clock(ClockSignal::Pause),
                RS_GIVEUP => clock(ClockSignal::GiveUp),
                RS_START_UNPAUSE => clock(ClockSignal::StartUnpause),
                RS_CANCEL_UNPAUSE => clock(ClockSignal::CancelUnpause),
                RS_FINISH_UNPAUSE => clock(ClockSignal::FinishUnpause),
                RS_SKIP => clock(ClockSignal::Skip),
                _ => None,
            };
            let Some(signal) = signal else {
                return Err(bad(format!(
                    "signal type {kind} is not valid in format version {version}"
                )));
            };
            events.push(Event::Clock {
                rms: r.u32()?,
                signal,
            });
        }

        Ok(Sspre {
            version,
            id,
            state_line,
            view,
            end_ms,
            declared_signals,
            events,
            trailing_bytes: r.remaining(),
        })
    }

    /// The map this run was played on: the id without its date suffix.
    pub fn song_id(&self) -> &str {
        self.id
            .rsplit_once('.')
            .map_or(self.id.as_str(), |(song, _)| song)
    }

    pub fn state(&self) -> State {
        let mut state = State {
            speed: 1.0,
            hit_window_ms: None,
            hitbox: None,
            start_offset_ms: 0.0,
            mods: Vec::new(),
        };
        for token in self.state_line.split(';').filter(|t| !t.is_empty()) {
            if let Some(v) = token.strip_prefix("s:c") {
                state.speed = v.parse().unwrap_or(1.0);
            } else if let Some(v) = token.strip_prefix("hitw:") {
                state.hit_window_ms = v.parse().ok();
            } else if let Some(v) = token.strip_prefix("hbox:") {
                state.hitbox = v.parse().ok();
            } else if let Some(v) = token.strip_prefix("so:") {
                state.start_offset_ms = v.parse().unwrap_or(0.0);
            } else if token.starts_with("m_") {
                state.mods.push(token.to_string());
            } else {
                state.speed = match token {
                    "s:---" => 1.0 / 1.35,
                    "s:--" => 1.0 / 1.25,
                    "s:-" => 1.0 / 1.15,
                    "s:+" => 1.15,
                    "s:++" => 1.25,
                    "s:+++" => 1.35,
                    "s:++++" => 1.45,
                    _ => continue,
                };
            }
        }
        // A file can claim anything; a speed that is not a positive finite
        // number would poison every time computed from it, and one far
        // beyond what the game offers is not a run.
        if !(state.speed.is_finite() && state.speed > 0.0 && state.speed <= MAX_SPEED) {
            state.speed = 1.0;
        }
        state
    }

    /// The run in the renderer's own replay model.
    ///
    /// Frame times come out in song time and positions in world units (x
    /// right, y up, centre at the origin). What the format does not store
    /// stays empty: no player name, no score, full health throughout.
    ///
    /// A skip jumps the song clock to a fixed lead before the NEXT NOTE, a
    /// distance only the map knows. Those jumps are left unsized in
    /// [`Origin::Sspre`] until the map is at hand.
    pub fn to_replay(&self) -> Result<Replay> {
        if self.version < 2 {
            return Err(bad(
                "format version 1 stores no note results and cannot be rendered",
            ));
        }
        let state = self.state();
        let speed = f64::from(state.speed);

        // The song clock as a line through an anchor: `ms = anchor_ms +
        // (rms - anchor_rms) * rate`, with rate 0 while paused.
        let (mut anchor_rms, mut anchor_ms, mut rate) =
            (0.0f64, state.start_offset_ms - LEAD_IN_MS * speed, speed);
        // Stored times are floored, so the true moment lies within the
        // following millisecond. Reading every stamp one millisecond late
        // keeps a hit from landing before its own note, which the floor
        // alone does whenever a note is hit in the frame it arrives.
        let real = |rms: u32| f64::from(rms) + 1.0;

        let (mut hits, mut misses) = (0i32, 0i32);
        let (mut unflagged_hits, mut gave_up) = (0u32, false);
        let mut judged_end: Option<u32> = None;
        let mut pause_ms = anchor_ms;
        // The furthest song time the run has reached; a pause counts, so
        // the run-up that replays the last stretch before it never does.
        let mut reached = f64::NEG_INFINITY;
        let mut skips = Vec::new();
        let mut frames: Vec<Frame> = Vec::new();

        for event in &self.events {
            match *event {
                Event::Hit { note } => {
                    hits += 1;
                    unflagged_hits += 1;
                    judged_end = judged_end.max(Some(note.saturating_add(1)));
                }
                Event::Miss { note } => {
                    misses += 1;
                    judged_end = judged_end.max(Some(note.saturating_add(1)));
                }
                Event::Clock { rms, signal } => {
                    let now = real(rms);
                    let ms = anchor_ms + (now - anchor_rms) * rate;
                    match signal {
                        ClockSignal::Pause => {
                            pause_ms = ms;
                            reached = reached.max(pause_ms);
                            (anchor_rms, anchor_ms, rate) = (now, pause_ms, 0.0);
                        }
                        ClockSignal::StartUnpause => {
                            (anchor_rms, anchor_ms, rate) =
                                (now, pause_ms - UNPAUSE_RUN_UP_MS * speed, speed);
                        }
                        ClockSignal::CancelUnpause => {
                            (anchor_rms, anchor_ms, rate) = (now, pause_ms, 0.0);
                        }
                        ClockSignal::FinishUnpause => {}
                        ClockSignal::Skip => skips.push(Skip {
                            at_ms: ms,
                            jump_ms: 0.0,
                        }),
                        ClockSignal::GiveUp => gave_up = true,
                    }
                }
                Event::Cursor { rms, x, y } => {
                    let ms = anchor_ms + (real(rms) - anchor_rms) * rate;
                    let frame = |ms: f64, hit: bool| Frame {
                        ms,
                        x: x - 1.0,
                        y: 1.0 - y,
                        health: 1.0,
                        hit,
                    };
                    if unflagged_hits > 0 {
                        // One frame per hit keeps the flag count equal to
                        // the hit count when two notes land in one game
                        // frame.
                        reached = reached.max(ms);
                        for _ in 0..unflagged_hits {
                            frames.push(frame(reached, true));
                        }
                        unflagged_hits = 0;
                    } else if rate > 0.0 && ms > reached {
                        reached = ms;
                        frames.push(frame(ms, false));
                    }
                    // Anything else is a sample taken while paused or
                    // during the unpause run-up: song time stood still or
                    // ran over ground already covered. The pipeline needs
                    // frame times that only move forward, and where the
                    // cursor wandered during a pause is not part of the
                    // run.
                }
            }
        }
        // A hit with no cursor sample after it (the file ends right there)
        // still has to be flagged somewhere.
        if let Some(&last) = frames.last() {
            for _ in 0..unflagged_hits {
                frames.push(Frame { hit: true, ..last });
            }
        }

        let judged = hits + misses;
        Ok(Replay {
            version: V_PITCH_LOCK,
            timestamp_ticks: self.recorded_ticks().unwrap_or(0),
            player_name: String::new(),
            legacy_map_id: self.song_id().to_string(),
            map_id: 0,
            start_from_ms: state.start_offset_ms.round() as i32,
            mode: MODE.to_string(),
            passed: !gave_up,
            mods: mods_json(&state.mods),
            spin: self.view.is_some_and(|v| v.cam_unlock),
            speed: state.speed,
            total_score: 0,
            accuracy_pct: if judged > 0 {
                hits as f32 / judged as f32 * 100.0
            } else {
                100.0
            },
            hits,
            misses,
            points: 0.0,
            fail_time_ms: -1,
            beatmap_hash: String::new(),
            pitch_lock: false,
            frames,
            trailing_bytes: self.trailing_bytes,
            origin: Origin::Sspre { skips, judged_end },
        })
    }

    /// The recording time from the id's date suffix as .NET ticks, read as
    /// UTC because the game stores local time without a zone.
    fn recorded_ticks(&self) -> Option<i64> {
        let (_, stamp) = self.id.rsplit_once('.')?;
        let (date, time) = stamp.split_once('_')?;
        let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
        let mut t = time.split('-').map(|p| p.parse::<i64>().ok());
        civil_ticks(
            (d.next()??, d.next()??, d.next()??),
            (t.next()??, t.next()??, t.next()??),
        )
    }
}

/// The state line's mod tokens as the JSON array `.rhr` uses.
///
/// Mirror is one mod with an axis there and two switches here; the
/// renderer finds the axis from the run itself, so a single switch maps to
/// `mod_mirror` and both to `mod_double_mirror`. Tokens with no `.rhr`
/// counterpart pass through under their own name so they surface as
/// unsupported instead of vanishing.
fn mods_json(tokens: &[String]) -> String {
    let has = |t: &str| tokens.iter().any(|m| m == t);
    let mut out: Vec<String> = Vec::new();
    match (has("m_mirror_x"), has("m_mirror_y")) {
        (true, true) => out.push("mod_double_mirror".into()),
        (true, false) | (false, true) => out.push("mod_mirror".into()),
        (false, false) => {}
    }
    for token in tokens {
        let name = match token.as_str() {
            "m_mirror_x" | "m_mirror_y" => continue,
            "m_hardrock" => "mod_hardrock",
            "m_nofail" => "mod_nofail",
            "m_ghost" => "mod_ghost",
            "m_nsight" => "mod_nearsighted",
            "m_sd" => "mod_sudden_death",
            "m_chaos" => "mod_chaos",
            other => other,
        };
        if !out.iter().any(|m| m == name) {
            out.push(name.to_string());
        }
    }
    serde_json::to_string(&out).unwrap_or_else(|_| "[]".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Builds file bytes the way the game's recorder writes them. Test-only
    /// on purpose: the crate itself never produces replay data.
    struct Fixture(Vec<u8>, u32);

    impl Fixture {
        fn new(version: u16, id: &str, state: &str) -> Fixture {
            let mut b = SIGNATURE.to_vec();
            b.extend(version.to_le_bytes());
            b.extend(0u64.to_le_bytes());
            if version >= 4 {
                b.extend(b"HARDWARE-ID-0123456789\n");
            }
            b.extend(id.as_bytes());
            b.push(b'\n');
            b.extend(state.as_bytes());
            b.push(b'\n');
            if version >= 3 {
                for v in [40.0f32, 40.0, 0.5, 1.0, 0.25, 0.5, 70.0] {
                    b.extend(v.to_le_bytes());
                }
                b.push(1); // cam_unlock
                b.extend(0.0f32.to_le_bytes());
            }
            b.push(0);
            b.extend(0u32.to_le_bytes()); // end_ms, patched by finish
            b.extend(0u32.to_le_bytes()); // signal count, patched by finish
            Fixture(b, 0)
        }
        fn cursor(mut self, rms: u32, x: f32, y: f32) -> Fixture {
            self.0.push(RS_CURSOR);
            self.0.extend(rms.to_le_bytes());
            self.0.extend(x.to_le_bytes());
            self.0.extend(y.to_le_bytes());
            self.1 += 1;
            self
        }
        fn signal(mut self, kind: u8, value: u32) -> Fixture {
            self.0.push(kind);
            self.0.extend(value.to_le_bytes());
            self.1 += 1;
            self
        }
        fn finish(mut self, end_ms: u32) -> Vec<u8> {
            self.0.push(RS_END);
            let count = self.1 + 1;
            // end_ms and count sit right before the first signal
            let header_end = self.header_len();
            self.0[header_end - 8..header_end - 4].copy_from_slice(&end_ms.to_le_bytes());
            self.0[header_end - 4..header_end].copy_from_slice(&count.to_le_bytes());
            self.0
        }
        fn header_len(&self) -> usize {
            // signature, version, reserved, then the lines
            let mut pos = 4 + 2 + 8;
            let version = u16::from_le_bytes([self.0[4], self.0[5]]);
            let lines = if version >= 4 { 3 } else { 2 };
            for _ in 0..lines {
                pos += self.0[pos..].iter().position(|&b| b == b'\n').unwrap() + 1;
            }
            if version >= 3 {
                pos += 7 * 4 + 1 + 4;
            }
            pos + 1 + 4 + 4
        }
    }

    const ID: &str = "ss_archive_some_map.2024-7-27_18-5-9";
    const PLAIN: &str = "ar:1;hbox:1.14;hitw:55;s:=";

    #[test]
    fn a_v4_file_yields_header_view_settings_and_events_in_file_order() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(10, 1.0, 1.0)
            .signal(RS_HIT, 0)
            .cursor(3050, 2.0, 0.0)
            .signal(RS_MISS, 1)
            .signal(RS_PAUSE, 4000)
            .finish(3050);
        let s = Sspre::parse(&data).unwrap();
        assert_eq!(s.version, 4);
        assert_eq!(s.id, ID);
        assert_eq!(s.song_id(), "ss_archive_some_map");
        assert_eq!(s.end_ms, 3050);
        assert_eq!(s.declared_signals, 6);
        assert_eq!(s.trailing_bytes, 0);
        let view = s.view.unwrap();
        assert_eq!(
            (view.approach_rate, view.fov, view.cam_unlock),
            (40.0, 70.0, true)
        );
        assert_eq!(
            s.events,
            vec![
                Event::Cursor {
                    rms: 10,
                    x: 1.0,
                    y: 1.0
                },
                Event::Hit { note: 0 },
                Event::Cursor {
                    rms: 3050,
                    x: 2.0,
                    y: 0.0
                },
                Event::Miss { note: 1 },
                Event::Clock {
                    rms: 4000,
                    signal: ClockSignal::Pause
                },
            ]
        );
    }

    #[test]
    fn the_hardware_id_of_a_v4_file_is_not_kept_anywhere() {
        let data = Fixture::new(4, ID, PLAIN).cursor(0, 1.0, 1.0).finish(0);
        let s = Sspre::parse(&data).unwrap();
        assert!(!format!("{s:?}").contains("HARDWARE-ID"));
        assert!(!format!("{:?}", s.to_replay().unwrap()).contains("HARDWARE-ID"));
    }

    #[test]
    fn a_v2_file_has_neither_hardware_id_nor_view_settings() {
        let data = Fixture::new(2, ID, PLAIN)
            .cursor(5, 1.0, 1.0)
            .signal(RS_HIT, 0)
            .finish(5);
        let s = Sspre::parse(&data).unwrap();
        assert_eq!(s.version, 2);
        assert_eq!(s.id, ID);
        assert!(s.view.is_none());
        assert_eq!(s.events.len(), 2);
    }

    #[test]
    fn files_the_game_itself_rejects_are_rejected() {
        let good = Fixture::new(4, ID, PLAIN).cursor(0, 1.0, 1.0).finish(0);
        let mut wrong_sig = good.clone();
        wrong_sig[0] = b'X';
        assert!(Sspre::parse(&wrong_sig).is_err());
        for version in [0u16, 5] {
            let mut v = good.clone();
            v[4..6].copy_from_slice(&version.to_le_bytes());
            assert!(Sspre::parse(&v).is_err(), "version {version}");
        }
        // never finished: the count the game patches in at the end is zero
        let unfinished = Fixture::new(4, ID, PLAIN).cursor(0, 1.0, 1.0);
        let len = unfinished.header_len();
        let mut unfinished = unfinished.finish(0);
        unfinished[len - 4..len].copy_from_slice(&0u32.to_le_bytes());
        assert!(Sspre::parse(&unfinished).is_err());
        // unknown signal type
        let unknown = Fixture::new(4, ID, PLAIN).signal(10, 0).finish(0);
        assert!(Sspre::parse(&unknown).is_err());
        // a v1 file may hold cursor samples only
        let v1 = Fixture::new(1, ID, PLAIN)
            .cursor(0, 1.0, 1.0)
            .signal(RS_HIT, 0)
            .finish(0);
        assert!(Sspre::parse(&v1).is_err());
        // cut off in the middle of a signal
        assert!(matches!(
            Sspre::parse(&good[..good.len() - 3]),
            Err(Error::UnexpectedEof { .. })
        ));
    }

    #[test]
    fn the_state_line_yields_speed_window_hitbox_offset_and_mods() {
        let data = Fixture::new(
            4,
            ID,
            "ar:1;hbox:1.27;hitw:58;m_hardrock;m_mirror_x;s:++;so:12000.500000",
        )
        .cursor(0, 1.0, 1.0)
        .finish(0);
        let state = Sspre::parse(&data).unwrap().state();
        assert_eq!(state.speed, 1.25);
        assert_eq!(state.hit_window_ms, Some(58.0));
        assert_eq!(state.hitbox, Some(1.27));
        assert_eq!(state.start_offset_ms, 12000.5);
        assert_eq!(state.mods, vec!["m_hardrock", "m_mirror_x"]);

        let custom = Fixture::new(4, ID, "s:c1.60").cursor(0, 1.0, 1.0).finish(0);
        assert_eq!(Sspre::parse(&custom).unwrap().state().speed, 1.6);
        let slow = Fixture::new(4, ID, "s:---").cursor(0, 1.0, 1.0).finish(0);
        assert!((Sspre::parse(&slow).unwrap().state().speed - 1.0 / 1.35).abs() < 1e-6);
        for forged in ["s:c-3", "s:c0", "s:cnan", "s:cinf", "s:c5000"] {
            let data = Fixture::new(4, ID, forged).cursor(0, 1.0, 1.0).finish(0);
            assert_eq!(Sspre::parse(&data).unwrap().state().speed, 1.0, "{forged}");
        }
    }

    fn replay(data: &[u8]) -> Replay {
        Sspre::parse(data).unwrap().to_replay().unwrap()
    }

    #[test]
    fn frames_come_out_in_song_time_and_world_coordinates() {
        // The song clock starts 3000 ms before zero; stamps read 1 ms late.
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(2999, 1.0, 1.0) // centre
            .cursor(3999, 2.0, 0.0) // top right cell
            .cursor(4999, 0.0, 2.0) // bottom left cell
            .finish(4999);
        let r = replay(&data);
        let got: Vec<(f64, f32, f32)> = r.frames.iter().map(|f| (f.ms, f.x, f.y)).collect();
        assert_eq!(
            got,
            vec![(0.0, 0.0, 0.0), (1000.0, 1.0, 1.0), (2000.0, -1.0, -1.0)]
        );
        assert_eq!(r.legacy_map_id, "ss_archive_some_map");
        assert_eq!(r.mode, MODE);
        assert_eq!(r.speed, 1.0);
        assert!(r.spin, "cam_unlock is the spin camera");
        assert_eq!(
            r.origin,
            Origin::Sspre {
                skips: vec![],
                judged_end: None
            }
        );
    }

    #[test]
    fn the_highest_judged_note_marks_where_the_run_got_to() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(2999, 1.0, 1.0)
            .signal(RS_HIT, 0)
            .cursor(3009, 1.0, 1.0)
            .signal(RS_MISS, 2)
            .signal(RS_HIT, 1)
            .cursor(3500, 1.0, 1.0)
            .finish(3500);
        let Origin::Sspre { judged_end, .. } = replay(&data).origin else {
            panic!("not a legacy origin");
        };
        assert_eq!(judged_end, Some(3));
    }

    #[test]
    fn a_run_that_judged_nothing_reads_as_full_accuracy() {
        let data = Fixture::new(4, ID, PLAIN).cursor(10, 1.0, 1.0).finish(10);
        let r = replay(&data);
        assert_eq!((r.hits, r.misses, r.accuracy_pct), (0, 0, 100.0));
    }

    #[test]
    fn a_speed_mod_and_a_start_offset_move_the_clock() {
        let data = Fixture::new(4, ID, "s:++;so:10000.000000")
            .cursor(999, 1.0, 1.0)
            .finish(999);
        let r = replay(&data);
        // 10000 - 3000 * 1.25 + 1000 * 1.25
        assert_eq!(r.frames[0].ms, 7500.0);
        assert_eq!(r.speed, 1.25);
        assert_eq!(r.start_from_ms, 10000);
    }

    #[test]
    fn a_hit_flags_the_cursor_sample_written_in_the_same_game_frame() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(3000, 1.0, 1.0)
            .signal(RS_HIT, 0)
            .cursor(3100, 0.0, 0.0)
            .signal(RS_MISS, 1)
            .cursor(3200, 1.0, 1.0)
            .signal(RS_HIT, 2)
            .signal(RS_HIT, 3) // two notes in one game frame
            .cursor(3300, 2.0, 2.0)
            .signal(RS_HIT, 4) // the file ends right after a hit
            .finish(3300);
        let r = replay(&data);
        let flags: Vec<(f64, bool)> = r.frames.iter().map(|f| (f.ms, f.hit)).collect();
        assert_eq!(
            flags,
            vec![
                (1.0, false),
                (101.0, true),
                (201.0, false),
                (301.0, true),
                (301.0, true),
                (301.0, true)
            ]
        );
        assert_eq!((r.hits, r.misses), (4, 1));
        assert_eq!(r.flagged_frames(), 4);
        assert_eq!(r.accuracy_pct, 80.0);
        assert!(r.passed);
    }

    #[test]
    fn a_pause_freezes_song_time_and_frames_only_move_forward() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(4999, 1.0, 1.0) // song 2000
            .signal(RS_PAUSE, 5999) // song 3000
            .cursor(6499, 0.0, 0.0) // paused: dropped
            .signal(RS_START_UNPAUSE, 7999) // rewinds to 2250
            .cursor(8249, 2.0, 2.0) // run-up at 2500: dropped
            .signal(RS_CANCEL_UNPAUSE, 8499) // released early: back to 3000, frozen
            .cursor(8999, 0.0, 2.0) // dropped
            .signal(RS_START_UNPAUSE, 9999)
            .signal(RS_FINISH_UNPAUSE, 10749) // back at 3000
            .cursor(11749, 2.0, 0.0) // song 4000
            .finish(11749);
        let r = replay(&data);
        let got: Vec<(f64, f32)> = r.frames.iter().map(|f| (f.ms, f.x)).collect();
        assert_eq!(got, vec![(2000.0, 0.0), (4000.0, 1.0)]);
    }

    #[test]
    fn a_skip_stays_unsized_until_the_map_is_known() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(2999, 1.0, 1.0)
            .signal(RS_SKIP, 3999) // song 1000
            .cursor(4999, 1.0, 1.0)
            .finish(4999);
        let r = replay(&data);
        assert_eq!(
            r.origin,
            Origin::Sspre {
                skips: vec![Skip {
                    at_ms: 1000.0,
                    jump_ms: 0.0
                }],
                judged_end: None
            }
        );
        assert_eq!(
            r.frames.last().unwrap().ms,
            2000.0,
            "provisional until resolved"
        );
    }

    #[test]
    fn giving_up_is_not_a_pass_and_mods_use_the_rhr_names() {
        let data = Fixture::new(
            4,
            ID,
            "m_hardrock;m_mirror_x;m_mirror_y;m_nofail;m_earthquake;s:=",
        )
        .cursor(0, 1.0, 1.0)
        .signal(RS_GIVEUP, 10)
        .finish(0);
        let r = replay(&data);
        assert!(!r.passed);
        assert_eq!(
            r.mods,
            r#"["mod_double_mirror","mod_hardrock","mod_nofail","m_earthquake"]"#
        );
        let single = Fixture::new(4, ID, "m_mirror_y")
            .cursor(0, 1.0, 1.0)
            .finish(0);
        assert_eq!(replay(&single).mods, r#"["mod_mirror"]"#);
    }

    #[test]
    fn the_recording_time_comes_from_the_id() {
        let data = Fixture::new(4, ID, PLAIN).cursor(0, 1.0, 1.0).finish(0);
        // 2024-07-27 18:05:09 UTC
        assert_eq!(replay(&data).unix_ms(), Some(1_722_103_509_000));
        let odd = Fixture::new(4, "no_date_here", PLAIN)
            .cursor(0, 1.0, 1.0)
            .finish(0);
        assert_eq!(replay(&odd).timestamp_ticks, 0);
        assert_eq!(replay(&odd).legacy_map_id, "no_date_here");
    }

    #[test]
    fn replays_without_an_online_id_are_told_apart_by_their_map_name() {
        let on = |song: &str| {
            replay(
                &Fixture::new(4, &format!("{song}.2024-7-27_18-5-9"), PLAIN)
                    .cursor(0, 1.0, 1.0)
                    .finish(0),
            )
        };
        assert!(on("map_a").same_map(&on("map_a")));
        assert!(!on("map_a").same_map(&on("map_b")), "both have online id 0");
        let mut online = on("map_a");
        online.map_id = 13076;
        let mut other_name = on("map_b");
        other_name.map_id = 13076;
        assert!(
            online.same_map(&other_name),
            "an online id on both sides decides"
        );
        assert!(
            online.same_map(&on("map_a")),
            "one side without an id falls back to the name"
        );
    }

    #[test]
    fn a_v1_file_parses_but_cannot_be_rendered() {
        let data = Fixture::new(1, ID, PLAIN).cursor(0, 1.0, 1.0).finish(0);
        assert!(Sspre::parse(&data).unwrap().to_replay().is_err());
    }

    #[test]
    fn the_replay_entry_point_recognises_the_format_by_its_first_bytes() {
        let data = Fixture::new(4, ID, PLAIN)
            .cursor(2999, 1.0, 1.0)
            .finish(2999);
        let r = Replay::parse(&data).unwrap();
        assert_eq!(r.mode, MODE);
        assert_eq!(r.frames.len(), 1);
    }
}
