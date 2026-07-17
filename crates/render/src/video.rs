//! Video export: render the replay frame by frame and stream raw RGBA into
//! a single ffmpeg process (rawvideo on stdin → H.264), muxing the map
//! audio. Speed mods play back as in the game: the timeline is compressed
//! by the replay's speed factor and the song is rate-shifted (faster and
//! higher-pitched), so a 1.45x run watches like a 1.45x run.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::scene::SceneParams;
use crate::{Error, Renderer, SkinConfig};
use rhythia_formats::{map::Map, rhr::Replay};

pub struct VideoOptions {
    pub fps: u32,
    /// Song time (ms) the video starts at.
    pub start_ms: f64,
    /// Song time (ms) the video ends at.
    pub end_ms: f64,
    /// ffmpeg executable (path or bare name on PATH).
    pub ffmpeg: String,
    /// Audio track to mux; None renders a silent video.
    pub audio: Option<PathBuf>,
    /// x264 CRF (lower = higher quality); the QP for VAAPI.
    pub crf: u32,
    /// x264 speed preset (ultrafast..placebo). veryfast roughly doubles
    /// encoding throughput over medium at slightly larger files.
    pub preset: String,
    /// Encoder: "x264" (software), or hardware "nvenc" (NVIDIA), "qsv"
    /// (Intel) or "vaapi" (AMD/Intel via /dev/dri) — the ffmpeg build must
    /// support the chosen one.
    pub encoder: String,
    /// Seconds of results screen appended after the clip (0 disables). Only
    /// shown when the clip reaches the end of the run (or its fail).
    pub results_secs: f64,
    /// Motion blur strength 0..=2 (0 = off): blends each output frame with
    /// its neighbours via ffmpeg's tmix (1 → 2 frames, 2 → 3 frames).
    pub motion_blur: u32,
    /// Music (song) volume, 0..=1.
    pub music_volume: f32,
    /// Hit/miss sounds mixed onto the song at the registered hit times.
    pub hitsounds: Option<HitsoundOptions>,
    /// A second replay of the same map, rendered as a ghost overlay.
    pub ghost: Option<GhostOptions>,
}

/// Ghost-race settings: the second replay and its overlay colour (sRGB
/// 0..1).
pub struct GhostOptions {
    pub replay: Replay,
    pub color: [f32; 3],
}

/// The game's hit/miss sounds (extracted from the user's install or a
/// custom skin) plus their volume, 0..=1.
pub struct HitsoundOptions {
    pub hit_wav: Vec<u8>,
    pub miss_wav: Option<Vec<u8>>,
    pub volume: f32,
}

impl Default for VideoOptions {
    fn default() -> Self {
        VideoOptions {
            fps: 60,
            start_ms: 0.0,
            end_ms: 0.0,
            ffmpeg: "ffmpeg".into(),
            audio: None,
            crf: 18,
            preset: "veryfast".into(),
            encoder: "x264".into(),
            results_secs: 4.0,
            motion_blur: 0,
            music_volume: 1.0,
            hitsounds: None,
            ghost: None,
        }
    }
}

/// Renders `[start_ms, end_ms]` of the replay to `out`, calling
/// `progress(done, total)` after each frame. `progress` returning `false`
/// cancels the render: ffmpeg is stopped, the partial output file removed
/// and [`Error::Cancelled`] returned.
#[allow(clippy::too_many_arguments)]
pub fn render_video(
    renderer: &Renderer,
    params: &SceneParams,
    config: &SkinConfig,
    replay: &Replay,
    map: &Map,
    out: &Path,
    opts: &VideoOptions,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<(), Error> {
    let (width, height) = renderer.dimensions();
    // Upload the skin's textures once; reused for every frame.
    let skin = renderer.prepare_skin(config);
    // Each side plays on its own field: mirror/hardrock from that replay's
    // mods are applied to its copy of the notes. Speed is the exception —
    // both runs share one timeline and one audio track, so a ghost with a
    // different speed cannot race.
    if let Some(g) = &opts.ghost {
        if (g.replay.speed - replay.speed).abs() > 0.005 {
            return Err(Error::Ghost(format!(
                "speed mods must match: main {:.2}x, ghost {:.2}x",
                replay.speed, g.replay.speed
            )));
        }
    }
    let ghost_input = opts.ghost.as_ref().map(|g| {
        let (gmap, gmods) = crate::mods::map_for_replay(map, &g.replay);
        crate::hud::GhostInput {
            state: crate::hud::HudState::new(&gmap, &g.replay),
            replay: g.replay.clone(),
            color: g.color,
            map: gmap,
            grid_scale: gmods.grid_scale,
        }
    });
    let (map, main_mods) = crate::mods::map_for_replay(map, replay);
    let map = &map;
    let mut params = *params;
    params.grid_scale = main_mods.grid_scale;
    let params = &params;
    // Resolve every note's hit/miss once; the HUD reads running stats from it.
    let hud_state = crate::hud::HudState::new(map, replay);
    // Replay frame times are song time — speed mods are baked in when the
    // .rhr is recorded (the hit registration matching note times proves
    // it). The VIDEO however runs at the modded speed, like the game did:
    // a 1.45x run covers song time 1.45x faster and the audio is rate-
    // shifted (pitch up, as in the game). speed is 1.0 unless modded.
    // A failed run ends at its fail time — the game stops there.
    let run_end = if replay.failed() {
        replay.fail_time_ms as f64
    } else {
        replay.length_ms()
    };
    let end_ms = opts.end_ms.min(run_end.max(opts.start_ms));
    // Results screen only when the clip reaches the end of the run.
    let show_results = opts.results_secs > 0.0 && end_ms >= run_end - 500.0;
    let speed = (replay.speed as f64).clamp(0.25, 3.0);
    let span_ms = (end_ms - opts.start_ms).max(0.0);
    // Wall-clock length of the clip: song span compressed by the speed mod.
    let span_real_ms = span_ms / speed;
    let play_frames = (span_real_ms / 1000.0 * opts.fps as f64).ceil() as u64;
    let play_frames = play_frames.max(1);
    let results_frames = if show_results {
        (opts.results_secs * opts.fps as f64).ceil() as u64
    } else {
        0
    };
    let total_frames = play_frames + results_frames;
    // Song time advanced per output frame: at 1.45x each real frame covers
    // 1.45 frames' worth of song.
    let song_dt_ms = 1000.0 / opts.fps as f64 * speed;

    let mut cmd = Command::new(&opts.ffmpeg);
    hide_console_window(&mut cmd);
    cmd.args(["-y", "-loglevel", "error", "-nostats"]);
    if opts.encoder == "vaapi" {
        cmd.args(["-vaapi_device", "/dev/dri/renderD128"]);
    }
    // Input 0: raw frames on stdin.
    cmd.args(["-f", "rawvideo", "-pix_fmt", "rgba"]);
    cmd.args(["-s", &format!("{width}x{height}")]);
    cmd.args(["-r", &opts.fps.to_string()]);
    cmd.args(["-i", "pipe:0"]);
    // Input 1: the audio, seeked to the clip start.
    if let Some(audio) = &opts.audio {
        cmd.args(["-ss", &format!("{:.3}", opts.start_ms / 1000.0)]);
        cmd.arg("-i").arg(audio);
    }
    // Hit/miss sounds: mixed into their own PCM track at the registered
    // hit times, fed to ffmpeg as a third input.
    let mut _hits_tmp: Option<tempfile::NamedTempFile> = None;
    if let (Some(hs), true) = (&opts.hitsounds, opts.audio.is_some()) {
        let track = crate::audio::Clip::from_wav(&hs.hit_wav).and_then(|hit| {
            let miss = hs.miss_wav.as_deref().and_then(crate::audio::Clip::from_wav);
            let note_times: Vec<f64> = map.notes.iter().map(|n| n.time_ms as f64).collect();
            crate::audio::build_hitsound_wav(
                &hit,
                miss.as_ref(),
                hud_state.results(),
                &note_times,
                opts.start_ms,
                end_ms,
                speed,
                hs.volume.clamp(0.0, 1.0),
            )
        });
        if let Some(wav) = track {
            let mut tmp = tempfile::Builder::new()
                .prefix("rhythr-hits-")
                .suffix(".wav")
                .tempfile()?;
            std::io::Write::write_all(&mut tmp, &wav)?;
            cmd.arg("-i").arg(tmp.path());
            _hits_tmp = Some(tmp);
        }
    }

    // Video encode: a hardware encoder when selected, software x264
    // otherwise. Quality knobs are mapped from the x264 CRF.
    let crf = opts.crf.to_string();
    // Optional motion blur: tmix averages neighbouring frames — free at
    // encode time, no extra rendering. It must run before any hardware
    // upload in the filter chain.
    let tmix = match opts.motion_blur.min(2) {
        0 => None,
        n => Some(format!("tmix=frames={}", n + 1)),
    };
    let vf = |hw_tail: &str| -> String {
        match (&tmix, hw_tail.is_empty()) {
            (Some(t), true) => t.clone(),
            (Some(t), false) => format!("{t},{hw_tail}"),
            (None, _) => hw_tail.to_string(),
        }
    };
    match opts.encoder.as_str() {
        "vaapi" => {
            cmd.args(["-vf", &vf("format=nv12,hwupload"), "-c:v", "h264_vaapi"]);
            cmd.args(["-qp", &crf]);
        }
        "nvenc" => {
            let f = vf("");
            if !f.is_empty() {
                cmd.args(["-vf", &f]);
            }
            cmd.args(["-c:v", "h264_nvenc", "-pix_fmt", "yuv420p"]);
            cmd.args(["-preset", "p5", "-rc", "vbr", "-cq", &crf, "-b:v", "0"]);
        }
        "qsv" => {
            let f = vf("");
            if !f.is_empty() {
                cmd.args(["-vf", &f]);
            }
            cmd.args(["-c:v", "h264_qsv", "-pix_fmt", "nv12"]);
            cmd.args(["-global_quality", &crf]);
        }
        _ => {
            let f = vf("");
            if !f.is_empty() {
                cmd.args(["-vf", &f]);
            }
            cmd.args(["-c:v", "libx264", "-pix_fmt", "yuv420p"]);
            cmd.args(["-crf", &crf, "-preset", &opts.preset]);
        }
    }
    // Audio encode: the music stops where the clip ends (a fail cuts it off);
    // silence pads the appended results screen, and the output is capped at
    // the exact video duration instead of -shortest. With hit sounds a
    // filter graph mixes the effects track on top of the (volume-scaled)
    // song; amix must not renormalise or the song would dip per overlap.
    if opts.audio.is_some() {
        let play_secs = span_real_ms / 1000.0;
        let mv = opts.music_volume.clamp(0.0, 1.5);
        // Speed mod: rate-shift the song like the game does (faster AND
        // higher-pitched) — resample to a known rate first so asetrate
        // scales from a fixed base.
        let rate = if (speed - 1.0).abs() > 0.001 {
            format!("aresample=48000,asetrate={:.0},aresample=48000,", 48000.0 * speed)
        } else {
            String::new()
        };
        if _hits_tmp.is_some() {
            cmd.args([
                "-filter_complex",
                &format!(
                    "[1:a]{rate}volume={mv:.3},atrim=duration={play_secs:.3},apad[song];                     [song][2:a]amix=inputs=2:duration=first:normalize=0[aout]"
                ),
                "-map",
                "0:v",
                "-map",
                "[aout]",
            ]);
        } else if (mv - 1.0).abs() > 0.001 || !rate.is_empty() {
            cmd.args(["-af", &format!("{rate}volume={mv:.3},atrim=duration={play_secs:.3},apad")]);
        } else {
            cmd.args(["-af", &format!("atrim=duration={play_secs:.3},apad")]);
        }
        cmd.args(["-c:a", "aac", "-b:a", "192k"]);
    }
    let video_dur = total_frames as f64 / opts.fps as f64;
    cmd.args(["-t", &format!("{video_dur:.3}")]);
    cmd.arg(out);

    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::null());
    cmd.stderr(Stdio::inherit());

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Ffmpeg(format!("could not start ffmpeg ({}): {e}", opts.ffmpeg)))?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Ffmpeg("ffmpeg stdin unavailable".into()))?;

    // From here on, EVERY exit except the final success must kill/reap the
    // ffmpeg child and remove the partial output — cancel, a GPU error from
    // `?`, a failed write, a bad ffmpeg exit status, even a panic. The guard's
    // Drop does exactly that unless it is defused at the end.
    let mut guard = EncodeGuard {
        child,
        out,
        done: false,
    };
    let mut write_frame = |pixels: &[u8], i: u64, child: &mut std::process::Child| {
        if let Err(e) = stdin.write_all(pixels) {
            let status = child.wait();
            return Err(Error::Ffmpeg(format!(
                "writing frame {i} failed: {e} (ffmpeg exit: {status:?})"
            )));
        }
        Ok(())
    };
    // Pipelined: submit frame i to the GPU, then read out frame i-1 while
    // the GPU is busy — overlapping rendering with readback and encoding
    // roughly doubles throughput over the strictly serial loop.
    const DEPTH: u64 = crate::renderer::READBACK_SLOTS as u64 - 1;
    let slot = |i: u64| (i % crate::renderer::READBACK_SLOTS as u64) as usize;
    for i in 0..play_frames {
        let song_ms = opts.start_ms + i as f64 * song_dt_ms;
        renderer.submit_frame_with_ghost(
            params,
            config,
            &skin,
            replay,
            map,
            song_ms,
            Some(&hud_state),
            ghost_input.as_ref(),
            slot(i),
        )?;
        // Read a frame that has DEPTH newer frames in flight behind it —
        // headroom that lets a fast GPU keep rendering while we encode.
        if i >= DEPTH {
            let j = i - DEPTH;
            renderer.with_slot_pixels(slot(j), |px| write_frame(px, j, &mut guard.child))??;
            if !progress(j + 1, total_frames) {
                return Err(Error::Cancelled);
            }
        }
    }
    for j in play_frames.saturating_sub(DEPTH.min(play_frames))..play_frames {
        renderer.with_slot_pixels(slot(j), |px| write_frame(px, j, &mut guard.child))??;
        if !progress(j + 1, total_frames) {
            return Err(Error::Cancelled);
        }
    }
    if results_frames > 0 {
        // The results screen is static: render once, repeat.
        let pixels = renderer.render_results(replay, map, &hud_state, config, ghost_input.as_ref())?;
        for i in 0..results_frames {
            write_frame(&pixels, play_frames + i, &mut guard.child)?;
            if !progress(play_frames + i + 1, total_frames) {
                return Err(Error::Cancelled);
            }
        }
    }
    #[allow(clippy::drop_non_drop)] // releases the closure's borrow of stdin
    drop(write_frame);

    drop(stdin);
    let status = guard
        .child
        .wait()
        .map_err(|e| Error::Ffmpeg(format!("waiting for ffmpeg: {e}")))?;
    if !status.success() {
        // Guard drop removes the unusable partial file.
        return Err(Error::Ffmpeg(format!("ffmpeg exited with {status}")));
    }
    guard.done = true;
    Ok(())
}

/// Owns the ffmpeg child during encoding; unless defused (`done = true`),
/// dropping it kills/reaps the process and deletes the partial output file.
struct EncodeGuard<'a> {
    child: std::process::Child,
    out: &'a Path,
    done: bool,
}

impl Drop for EncodeGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = std::fs::remove_file(self.out);
        }
    }
}

/// Keeps spawned ffmpeg processes from flashing a console window on Windows
/// (CREATE_NO_WINDOW); no-op elsewhere.
fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = &cmd;
}

/// Probes whether `ffmpeg` can actually encode with the given hardware
/// encoder on this machine by encoding a tiny synthetic clip to null.
pub fn encoder_works(ffmpeg: &str, encoder: &str) -> bool {
    encoder_error(ffmpeg, encoder).is_none()
}

/// Like [`encoder_works`], but on failure returns ffmpeg's stderr (its last
/// meaningful line) so the UI can say WHY an encoder is unavailable — e.g.
/// nvenc rejecting an outdated NVIDIA driver.
pub fn encoder_error(ffmpeg: &str, encoder: &str) -> Option<String> {
    let mut args: Vec<&str> = vec!["-hide_banner", "-loglevel", "error"];
    match encoder {
        "vaapi" => {
            if !std::path::Path::new("/dev/dri/renderD128").exists() {
                return Some("no VAAPI render device (/dev/dri/renderD128)".into());
            }
            args.extend(["-vaapi_device", "/dev/dri/renderD128"]);
        }
        "nvenc" | "qsv" => {}
        _ => return None, // software x264 always works
    }
    args.extend([
        "-f",
        "lavfi",
        "-i",
        "color=black:size=256x256:rate=30:duration=0.1",
    ]);
    match encoder {
        "vaapi" => args.extend(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi"]),
        "nvenc" => args.extend(["-c:v", "h264_nvenc"]),
        "qsv" => args.extend(["-c:v", "h264_qsv"]),
        _ => unreachable!(),
    }
    args.extend(["-f", "null", "-"]);
    let mut cmd = std::process::Command::new(ffmpeg);
    hide_console_window(&mut cmd);
    let output = cmd
        .args(&args)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();
    match output {
        Ok(out) if out.status.success() => None,
        Ok(out) => {
            let stderr = String::from_utf8_lossy(&out.stderr);
            // The last non-empty line is usually the actual reason
            // ("driver does not support the required nvenc API version…").
            let reason = stderr
                .lines()
                .rev()
                .map(str::trim)
                .find(|l| !l.is_empty())
                .unwrap_or("encoder test failed")
                .to_string();
            Some(reason)
        }
        Err(e) => Some(format!("could not run ffmpeg: {e}")),
    }
}
