//! Video export: render the replay frame by frame and stream raw RGBA into
//! a single ffmpeg process (rawvideo on stdin → H.264), muxing the map
//! audio. Speed mods play back as in the game: the timeline is compressed
//! by the replay's speed factor and the song is rate-shifted (faster and
//! higher-pitched), so a 1.45x run watches like a 1.45x run.

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
    /// User-facing quality, 0..=100, HIGHER IS BETTER. Mapped onto each
    /// encoder's own scale by [`crate::quality`] — it is deliberately not
    /// the raw CRF any more, see that module.
    pub quality: u32,
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
    /// Extra ffmpeg output arguments, appended just before the output
    /// path — the Analyze window uses this for `+faststart` and a short
    /// GOP so its segments start and seek instantly.
    pub extra_output_args: Vec<String>,
    /// Hand frames to ffmpeg over a loopback socket instead of its stdin.
    /// Off by default — see [`FrameSink`] for what it is worth and why that
    /// is not obviously a win.
    pub tcp_feed: bool,
    /// Bytes per write into that socket; 0 means the whole frame in one
    /// call. See [`FrameSink::write_frame`] — the best value is not the same
    /// on every platform, so this is a setting rather than a constant.
    pub socket_chunk: usize,
    /// Diagnostic: feed ffmpeg every frame exactly as a real render does,
    /// but tell it to copy them straight to nowhere instead of encoding.
    ///
    /// This is the only way to see how the `feed` time splits. That number
    /// is the write into the transport AND everything ffmpeg was too busy to
    /// accept, and on a machine with a fast GPU it is essentially the whole
    /// frame — so knowing whether it is the transport or the encoder decides
    /// whether tuning the transport can achieve anything at all. Writes no
    /// file.
    pub discard_output: bool,
    /// Custom VIDEO background: decoded by the same ffmpeg, muted and
    /// looped from its start point, one frame per output frame. (Image
    /// backgrounds ride the config's background layers instead.) The
    /// results screen is never touched by it.
    pub background_video: Option<BackgroundVideo>,
}

/// A video background and how the user placed it.
pub struct BackgroundVideo {
    pub path: PathBuf,
    pub opts: crate::background::BackgroundOptions,
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
            quality: crate::quality::DEFAULT,
            preset: "veryfast".into(),
            encoder: "x264".into(),
            results_secs: 4.0,
            motion_blur: 0,
            music_volume: 1.0,
            hitsounds: None,
            ghost: None,
            extra_output_args: Vec::new(),
            tcp_feed: false,
            socket_chunk: 256 * 1024,
            discard_output: false,
            background_video: None,
        }
    }
}

/// Renders `[start_ms, end_ms]` of the replay to `out`, calling
/// `progress(done, total)` after each frame. `progress` returning `false`
/// cancels the render: ffmpeg is stopped, the partial output file removed
/// and [`Error::Cancelled`] returned.
#[allow(clippy::too_many_arguments)]
/// Where a finished render's time went, per frame.
///
/// Collected always rather than behind a flag, because two `Instant`s a frame
/// cost nothing next to a frame and because the question "why was that render
/// slower than the last one" is otherwise unanswerable from a GUI: the stage
/// breakdown used to go to stderr, which a desktop app on Windows discards.
#[derive(Debug, Clone, Default)]
pub struct RenderStats {
    pub frames: u64,
    /// Building the picture and handing it to the GPU.
    pub build_ms: f64,
    /// Waiting for a finished frame to come back out of VRAM.
    pub readback_ms: f64,
    /// Colour conversion on the CPU. Zero when the GPU did it.
    pub convert_ms: f64,
    /// Pushing the frame into ffmpeg, including any time ffmpeg spent not
    /// keeping up — this is the encoder's back pressure as much as the
    /// transport's cost.
    pub feed_ms: f64,
    /// Set when this was a diagnostic run that encoded nothing, so the feed
    /// figure is the transport alone.
    pub discarded: bool,
    /// Which transport carried the frames, and how they were written into
    /// it. The write size is a setting and this is where its effect can be
    /// read off — a desktop app has no console to print it to, which is
    /// exactly how an environment variable failed at the job.
    pub transport: String,
}

impl RenderStats {
    /// One line for a human: the stages, largest first in practice.
    pub fn summary(&self) -> String {
        format!(
            "per frame: build {:.2} ms · readback {:.2} ms · convert {:.2} ms · \
             feed {:.2} ms ({})",
            self.build_ms,
            self.readback_ms,
            self.convert_ms,
            self.feed_ms,
            if self.discarded {
                format!("{}, NO ENCODING — transport only", self.transport)
            } else {
                self.transport.clone()
            }
        )
    }
}

pub fn render_video(
    renderer: &Renderer,
    params: &SceneParams,
    config: &SkinConfig,
    replay: &Replay,
    map: &Map,
    out: &Path,
    opts: &VideoOptions,
    mut progress: impl FnMut(u64, u64) -> bool,
) -> Result<RenderStats, Error> {
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
    // Some wild replays store wall-clock frame times instead of song time
    // (speed already applied); normalise so the speed pipeline below never
    // doubles up or drops the speed. No-op for well-formed replays.
    let mut replay_owned;
    let replay = if rhythia_sim::timebase::time_scale(map, replay) != 1.0 {
        replay_owned = replay.clone();
        rhythia_sim::timebase::normalize(&mut replay_owned, map);
        &replay_owned
    } else {
        replay
    };
    let mut ghost_input = opts.ghost.as_ref().map(|g| {
        let mut greplay = g.replay.clone();
        rhythia_sim::timebase::normalize(&mut greplay, map);
        let g = crate::video::GhostOptions { replay: greplay, color: g.color };
        let g = &g;
        let (gmap, gmods) = crate::mods::map_for_replay(map, &g.replay);
        crate::hud::GhostInput {
            state: crate::hud::HudState::new(&gmap, &g.replay),
            replay: g.replay.clone(),
            color: g.color,
            map: gmap,
            mods: gmods,
            race: None,
        }
    });
    let (map, main_mods) = crate::mods::map_for_replay(map, replay);
    let map = &map;
    let mut params = *params;
    params.apply_mods(&main_mods);
    params.apply_speed(replay.speed);
    let params = &params;
    // Resolve every note's hit/miss once; the HUD reads running stats from it.
    let hud_state = crate::hud::HudState::new(map, replay);
    // With both sides resolved, the whole-map race series (results delta
    // graph) is fixed — build it once.
    if let Some(g) = ghost_input.as_mut() {
        g.race = Some(crate::race::RaceSeries::for_race(
            &crate::race::RaceSide { map, replay, state: &hud_state },
            &crate::race::RaceSide { map: &g.map, replay: &g.replay, state: &g.state },
        ));
    }
    let ghost_input = ghost_input;
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
        // Enumerated, not assumed: on a hybrid laptop the first render node
        // is often the display-only chip.
        if let Some(dev) = vaapi_device(&opts.ffmpeg) {
            cmd.args(["-vaapi_device", &dev]);
        }
    }
    // Input 0: raw frames on stdin. NV12 where the frame size allows it —
    // 1.5 bytes per pixel instead of 4, on a pipe that was measured to be
    // the render's entire speed limit. See crate::nv12.
    let feed_nv12 = crate::nv12::nv12_supported(width as usize, height as usize)
        && std::env::var_os("RHYTHR_NO_NV12").is_none();
    cmd.args([
        "-f",
        "rawvideo",
        "-pix_fmt",
        if feed_nv12 { "nv12" } else { "rgba" },
    ]);
    cmd.args(["-s", &format!("{width}x{height}")]);
    cmd.args(["-r", &opts.fps.to_string()]);
    // A loopback listener, when asked for. Bound to 127.0.0.1 explicitly
    // rather than to every interface: a loopback-only listener is what keeps
    // Windows Firewall from putting a dialog in front of a render.
    let listener = if opts.tcp_feed && tcp_feed_works(&opts.ffmpeg) {
        std::net::TcpListener::bind(("127.0.0.1", 0)).ok()
    } else {
        None
    };
    match &listener {
        Some(l) => {
            let port = l.local_addr().map(|a| a.port()).unwrap_or(0);
            cmd.args(["-i", &format!("tcp://127.0.0.1:{port}")]);
        }
        None => {
            cmd.args(["-i", "pipe:0"]);
        }
    }
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

    // Optional motion blur: tmix averages neighbouring frames — free at
    // encode time, no extra rendering. It must run before any hardware
    // upload in the filter chain.
    let tmix = match opts.motion_blur.min(2) {
        0 => None,
        n => Some(format!("tmix=frames={}", n + 1)),
    };
    if opts.discard_output {
        // Diagnostic: every frame still crosses the transport exactly as it
        // would in a real render, but nothing encodes it. No filter chain,
        // no encoder, no audio — what is left in the `feed` figure is the
        // transport by itself, and the difference against a real render of
        // the same clip is what the encoder costs.
        cmd.args(["-c:v", "copy", "-an"]);
    } else {
        let enc = video_encoder_args(
            &opts.encoder,
            opts.quality,
            &opts.preset,
            feed_nv12,
            vaapi_icq_supported(&opts.ffmpeg, opts.encoder == "vaapi"),
        );
        let chain = match (&tmix, enc.filter.is_empty()) {
            (Some(t), true) => t.clone(),
            (Some(t), false) => format!("{t},{}", enc.filter),
            (None, _) => enc.filter.clone(),
        };
        if !chain.is_empty() {
            cmd.args(["-vf", &chain]);
        }
        cmd.args(&enc.args);
    }

    // Audio encode: the music stops where the clip ends (a fail cuts it off);
    // silence pads the appended results screen, and the output is capped at
    // the exact video duration instead of -shortest. With hit sounds a
    // filter graph mixes the effects track on top of the (volume-scaled)
    // song; amix must not renormalise or the song would dip per overlap.
    if opts.audio.is_some() && !opts.discard_output {
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
    // Move the index to the front so the file starts playing before it has
    // finished downloading. Only the Analyze window's own segments used to
    // ask for this; the video the user actually keeps and uploads did not.
    // It costs one rewrite of the finished file.
    if !opts.extra_output_args.iter().any(|a| a.contains("faststart")) {
        cmd.args(["-movflags", "+faststart"]);
    }
    for a in &opts.extra_output_args {
        cmd.arg(a);
    }
    // A sibling of the real output rather than the output itself, so a
    // render that dies cannot take the previous file at that path with it.
    // Same directory, so the final step is a rename and not a copy across
    // filesystems.
    // The marker goes BEFORE the extension, not after it: ffmpeg picks the
    // container from the extension, and "video.mp4.rhythr-part" made it give
    // up with "Error initializing the muxer".
    let part = {
        let name = out.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let renamed = match name.rsplit_once('.') {
            Some((stem, ext)) if !stem.is_empty() => format!("{stem}.rhythr-part.{ext}"),
            _ => format!("{name}.rhythr-part"),
        };
        out.with_file_name(renamed)
    };
    if opts.discard_output {
        cmd.args(["-f", "null", "-"]);
    } else {
        cmd.arg(&part);
    }

    cmd.stdin(if listener.is_some() {
        Stdio::null()
    } else {
        Stdio::piped()
    });
    cmd.stdout(Stdio::null());
    // Captured, not inherited: a GUI has no terminal, so an inherited
    // stderr threw away the only explanation ffmpeg ever gives and every
    // failure arrived as a bare "exited with exit status: 1". It must be
    // DRAINED on a thread — ffmpeg writes progress here, and a full pipe
    // with no reader deadlocks the encode.
    cmd.stderr(Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| Error::Ffmpeg(format!("could not start ffmpeg ({}): {e}", opts.ffmpeg)))?;
    // From here on, EVERY exit except the final success must kill/reap the
    // ffmpeg child and remove the partial output — cancel, a GPU error from
    // `?`, a failed write, a bad ffmpeg exit status, even a panic. The guard's
    // Drop does exactly that unless it is defused at the end.
    //
    // The guard is built BEFORE the frame sink is opened, and that ordering
    // is load-bearing: opening the sink can fail (a socket handshake can time
    // out where a pipe cannot), and with the guard built afterwards that `?`
    // walked out past a running ffmpeg with nothing left to kill or reap it.
    let listener_used = listener.is_some();
    let stderr = child.stderr.take();
    let stdin_pipe = child.stdin.take();
    let mut guard = EncodeGuard {
        child,
        part: part.clone(),
        out,
        done: false,
    };
    let log = FfmpegLog::drain(stderr);
    let mut stdin = FrameSink::open(listener, stdin_pipe, opts.socket_chunk, &log)?;
    // Ask the GPU to convert instead. It reports back whether it took the
    // job: the compute path needs a width it can address, and everything it
    // turns down falls through to the CPU conversion unchanged.
    // RHYTHR_NO_GPU_NV12 forces that fallback, which is the way out if a
    // driver ever miscompiles the shader — the CPU path stays the reference
    // and both are held to producing the same bytes by a test.
    let gpu_nv12 = feed_nv12
        && std::env::var_os("RHYTHR_NO_GPU_NV12").is_none()
        && renderer.enable_nv12_readback(true);
    // The renderer outlives this call in the desktop app, and a slot left in
    // NV12 mode would hand a live preview 1.5 bytes per pixel where it
    // expects 4. Switched back however this returns, cancellation included.
    struct Nv12Reset<'a>(&'a Renderer);
    impl Drop for Nv12Reset<'_> {
        fn drop(&mut self) {
            self.0.enable_nv12_readback(false);
        }
    }
    let _nv12_reset = Nv12Reset(renderer);
    // Reused across frames: converting into a fresh buffer each time would
    // hand back the allocation churn this change exists to remove. The
    // results screen still comes back as RGBA, so this is needed even when
    // the GPU handles the replay's own frames.
    let mut nv12_buf = if feed_nv12 {
        vec![0u8; crate::nv12::nv12_len(width as usize, height as usize)]
    } else {
        Vec::new()
    };
    // Always on: the cost is two Instant reads a frame, and the answer is
    // worth far more than that the first time a render is unexpectedly slow.
    let timing = true;
    // Split out of the handoff separately, so the readback wait can be told
    // apart from our own colour conversion and from ffmpeg's backpressure.
    let t_conv = std::cell::Cell::new(0.0f64);
    let t_pipe = std::cell::Cell::new(0.0f64);
    let mut write_frame = |pixels: &[u8],
                           i: u64,
                           already_nv12: bool,
                           child: &mut std::process::Child| {
        let mark = std::time::Instant::now();
        let payload: &[u8] = if already_nv12 {
            pixels
        } else if feed_nv12
            && crate::nv12::rgba_to_nv12(pixels, width as usize, height as usize, &mut nv12_buf)
        {
            &nv12_buf
        } else {
            pixels
        };
        if timing {
            t_conv.set(t_conv.get() + mark.elapsed().as_secs_f64());
        }
        let mark = std::time::Instant::now();
        let wrote = stdin.write_frame(payload);
        if timing {
            t_pipe.set(t_pipe.get() + mark.elapsed().as_secs_f64());
        }
        if let Err(e) = wrote {
            // A broken pipe here means ffmpeg died; its own last words say
            // why far better than the errno does.
            let status = child.wait();
            return Err(Error::Ffmpeg(format!(
                "writing frame {i} failed: {e} (ffmpeg exit: {status:?}){}",
                log.tail()
            )));
        }
        Ok(())
    };
    // Pipelined: submit frame i to the GPU, then read out frame i-1 while
    // the GPU is busy — overlapping rendering with readback and encoding
    // roughly doubles throughput over the strictly serial loop.
    const DEPTH: u64 = crate::renderer::READBACK_SLOTS as u64 - 1;
    let slot = |i: u64| (i % crate::renderer::READBACK_SLOTS as u64) as usize;
    // Custom video background: a second ffmpeg decodes it muted, looped
    // and scaled-to-cover — one RGBA frame per output frame, streamed
    // into the skin's persistent background texture. If it dies mid-way
    // the last good frame stays. It runs on its own thread: decoding it
    // inline blocked the render loop on a pipe for a whole frame before
    // the GPU got any work at all.
    let (fw, fh) = renderer.dimensions();
    let mut bg_stream = match &opts.background_video {
        Some(bg) => {
            // Decode at the video's OWN rate, capped at the output rate.
            // Asking for the output rate made ffmpeg duplicate frames and
            // push every copy through the pipe: a 30 fps background under a
            // 60 fps render moved twice the bytes for the same picture.
            let (_, native_fps) = crate::background::probe_video(&opts.ffmpeg, &bg.path);
            let decode_fps = native_fps
                .map(|f| f.round().clamp(1.0, f64::from(opts.fps)) as u32)
                .unwrap_or(opts.fps);
            let decoder = crate::background::VideoDecoder::spawn(
                &opts.ffmpeg,
                &bg.path,
                fw,
                fh,
                decode_fps,
                &bg.opts,
            )
            .map_err(Error::Ffmpeg)?;
            Some(crate::background::BackgroundStream::spawn(
                decoder,
                (fw * fh * 4) as usize,
                f64::from(decode_fps),
                f64::from(opts.fps),
            ))
        }
        None => None,
    };
    // Where the wall clock goes, per stage, under RHYTHR_TIME_STAGES. This
    // is how the bottleneck was found and it costs one Instant per stage per
    // frame, which is nothing next to a frame: the answer was that building
    // and submitting a frame takes 0.6 ms while handing it to ffmpeg takes
    // 8.5 ms, i.e. the pipe is the render's speed limit, not the GPU.
    let (mut t_submit, mut t_hand) = (0.0f64, 0.0f64);
    for i in 0..play_frames {
        let song_ms = opts.start_ms + i as f64 * song_dt_ms;
        let mark = std::time::Instant::now();
        if let Some(frame) = bg_stream.as_mut().and_then(|s| s.next_frame()) {
            renderer.stream_background(&skin, frame);
        }
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
        if timing {
            t_submit += mark.elapsed().as_secs_f64();
        }
        let mark = std::time::Instant::now();
        // Read a frame that has DEPTH newer frames in flight behind it —
        // headroom that lets a fast GPU keep rendering while we encode.
        if i >= DEPTH {
            let j = i - DEPTH;
            renderer
                .with_slot_pixels(slot(j), |px| write_frame(px, j, gpu_nv12, &mut guard.child))??;
            if !progress(j + 1, total_frames) {
                return Err(Error::Cancelled);
            }
        }
        if timing {
            t_hand += mark.elapsed().as_secs_f64();
        }
    }
    let stats = {
        let per = |v: f64| v * 1000.0 / play_frames.max(1) as f64;
        let (conv, pipe) = (t_conv.get(), t_pipe.get());
        if std::env::var_os("RHYTHR_TIME_STAGES").is_some() {
            eprintln!(
                "\nSTAGES per frame: build+submit {:.2} ms | readback {:.2} ms | \
                 nv12 {:.2} ms | pipe->ffmpeg {:.2} ms  (handoff total {:.2} ms)",
                per(t_submit),
                per(t_hand - conv - pipe),
                per(conv),
                per(pipe),
                per(t_hand)
            );
        }
        RenderStats {
            frames: play_frames,
            build_ms: per(t_submit),
            readback_ms: per(t_hand - conv - pipe),
            convert_ms: per(conv),
            feed_ms: per(pipe),
            discarded: opts.discard_output,
            transport: if listener_used {
                match opts.socket_chunk {
                    0 => "socket, whole frame".to_string(),
                    n if n % 1024 == 0 => format!("socket, {} KiB", n / 1024),
                    n => format!("socket, {n} B"),
                }
            } else {
                "pipe".to_string()
            },
        }
    };
    for j in play_frames.saturating_sub(DEPTH.min(play_frames))..play_frames {
        renderer.with_slot_pixels(slot(j), |px| write_frame(px, j, gpu_nv12, &mut guard.child))??;
        if !progress(j + 1, total_frames) {
            return Err(Error::Cancelled);
        }
    }
    if results_frames > 0 {
        // The results screen is static: render once, repeat.
        let pixels = renderer.render_results(replay, map, &hud_state, config, ghost_input.as_ref())?;
        for i in 0..results_frames {
            write_frame(&pixels, play_frames + i, false, &mut guard.child)?;
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
        return Err(Error::Ffmpeg(format!(
            "ffmpeg exited with {status}{}",
            log.tail()
        )));
    }
    // Only now does the finished video take the name it was asked for. Up to
    // this line nothing has touched whatever was already there.
    if !opts.discard_output {
        std::fs::rename(&part, out).map_err(|e| {
            Error::Ffmpeg(format!(
                "could not move the finished render into place ({}): {e}",
                out.display()
            ))
        })?;
    }
    guard.done = true;
    Ok(stats)
}

/// The tail of ffmpeg's stderr, collected on a reader thread so the pipe
/// never fills up. Only the last few lines are kept — that is where ffmpeg
/// puts the reason, and the rest is progress spam.
struct FfmpegLog {
    lines: std::sync::Arc<std::sync::Mutex<std::collections::VecDeque<String>>>,
}

impl FfmpegLog {
    const KEEP: usize = 40;

    fn drain(stderr: Option<std::process::ChildStderr>) -> FfmpegLog {
        let lines = std::sync::Arc::new(std::sync::Mutex::new(
            std::collections::VecDeque::<String>::new(),
        ));
        if let Some(err) = stderr {
            let sink = std::sync::Arc::clone(&lines);
            std::thread::spawn(move || {
                use std::io::BufRead;
                for line in std::io::BufReader::new(err).lines().map_while(Result::ok) {
                    let line = line.trim().to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let mut q = sink.lock().unwrap_or_else(|e| e.into_inner());
                    if q.len() == Self::KEEP {
                        q.pop_front();
                    }
                    q.push_back(line);
                }
            });
        }
        FfmpegLog { lines }
    }

    /// The last lines that are not ffmpeg's periodic progress readout,
    /// formatted for appending to an error message (empty when silent).
    fn tail(&self) -> String {
        let q = self.lines.lock().unwrap_or_else(|e| e.into_inner());
        let picked: Vec<&str> = q
            .iter()
            .rev()
            .filter(|l| !l.starts_with("frame=") && !l.starts_with("size="))
            .take(3)
            .map(String::as_str)
            .collect();
        if picked.is_empty() {
            return String::new();
        }
        let mut out = String::from(" — ");
        for (i, l) in picked.iter().rev().enumerate() {
            if i > 0 {
                out.push_str(" / ");
            }
            out.push_str(l);
        }
        out
    }
}

/// Owns the ffmpeg child during encoding; unless defused (`done = true`),
/// dropping it kills/reaps the process and deletes the partial output file.
struct EncodeGuard<'a> {
    child: std::process::Child,
    /// Where ffmpeg is actually writing: a sibling of the real output, never
    /// the real output itself. This used to BE the real output, and the drop
    /// below deleted it on every unhappy exit — so a render that failed, was
    /// cancelled, or was still running when the app closed took the previous
    /// video at that path with it, and ffmpeg's own `-y` had already
    /// truncated it anyway. Answering "Replace" in the overwrite prompt was
    /// enough to lose the file being replaced.
    part: PathBuf,
    /// Where it belongs once ffmpeg has exited cleanly.
    out: &'a Path,
    done: bool,
}

impl Drop for EncodeGuard<'_> {
    fn drop(&mut self) {
        if !self.done {
            let _ = self.child.kill();
            let _ = self.child.wait();
            // Only ever its own partial file. Whatever was at `out` before
            // this render started is none of its business.
            let _ = std::fs::remove_file(&self.part);
        }
    }
}

/// Keeps spawned ffmpeg processes from flashing a console window on Windows
/// (CREATE_NO_WINDOW); no-op elsewhere.
pub(crate) fn hide_console_window(cmd: &mut Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    let _ = &cmd;
}

/// Whether this ffmpeg can be executed at all. Checked before a render so
/// a missing or broken binary is reported up front, instead of surfacing as
/// a failed encode after the user has waited through one.
pub fn ffmpeg_runs(ffmpeg: &str) -> bool {
    let mut cmd = Command::new(ffmpeg);
    hide_console_window(&mut cmd);
    cmd.arg("-version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Probes whether `ffmpeg` can actually encode with the given hardware
/// encoder on this machine by encoding a tiny synthetic clip to null.
pub fn encoder_works(ffmpeg: &str, encoder: &str) -> bool {
    encoder_error(ffmpeg, encoder).is_none()
}

/// The hardware encoders worth trying, best first, for this platform.
///
/// AMF is AMD's Windows encoder — AMD dropped it on Linux and points at
/// VA-API instead — so the two never compete for the same machine and each
/// list is ordered for the platform it runs on. Both the auto-selection and
/// the UI's availability probe walk this, so they cannot disagree about what
/// exists or in what order.
pub fn hardware_encoders() -> &'static [&'static str] {
    if cfg!(windows) {
        &["nvenc", "qsv", "amf"]
    } else {
        &["nvenc", "qsv", "vaapi", "amf"]
    }
}

/// Whether frames can be handed to THIS ffmpeg over a loopback socket.
///
/// Probed once per binary, and the reason for probing rather than simply
/// trying is that the answer arrives too late otherwise: a pipe cannot fail
/// to connect, a socket can, and by then the render has already started. So
/// the whole path — bind, spawn, connect, feed, exit cleanly — is walked
/// with a 64x64 frame first, and anything that does not survive that quietly
/// gets the pipe instead.
fn tcp_feed_works(ffmpeg: &str) -> bool {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(ffmpeg).copied()) {
        return hit;
    }
    let ok = probe_tcp_feed(ffmpeg);
    if let Ok(mut c) = cache.lock() {
        c.insert(ffmpeg.to_string(), ok);
    }
    ok
}

fn probe_tcp_feed(ffmpeg: &str) -> bool {
    let Ok(listener) = std::net::TcpListener::bind(("127.0.0.1", 0)) else {
        return false;
    };
    let Ok(port) = listener.local_addr().map(|a| a.port()) else {
        return false;
    };
    let mut cmd = Command::new(ffmpeg);
    hide_console_window(&mut cmd);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    cmd.args(["-f", "rawvideo", "-pix_fmt", "nv12", "-s", "64x64", "-r", "30"]);
    cmd.args(["-i", &format!("tcp://127.0.0.1:{port}")]);
    cmd.args(["-f", "null", "-"]);
    let Ok(mut child) = cmd.stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    // Everything past the spawn has a child to clean up on the way out.
    let handshake = (|| {
        listener.set_nonblocking(true).ok()?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let sock = loop {
            match listener.accept() {
                Ok((s, _)) => break s,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return None;
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(_) => return None,
            }
        };
        sock.set_nonblocking(false).ok()?;
        let mut sock = sock;
        let frame = vec![0u8; crate::nv12::nv12_len(64, 64)];
        for _ in 0..2 {
            std::io::Write::write_all(&mut sock, &frame).ok()?;
        }
        drop(sock);
        Some(())
    })();
    if handshake.is_none() {
        let _ = child.kill();
        let _ = child.wait();
        return false;
    }
    child.wait().map(|s| s.success()).unwrap_or(false)
}

/// Where ffmpeg reads frames from: its stdin, or a loopback socket.
///
/// What the socket is worth depends entirely on how fast the encoder is,
/// which is why it took two machines to settle. Here, where libx264 is the
/// wall, it is nothing: a full render came out the same either way. On the
/// owner's NVENC machine, where the encoder is not the wall, the same switch
/// is 160 fps to 210 fps at 3840x2160/240 — most of a third, because by then
/// the transport was most of what was left.
///
/// So it is on by default, but only after [`tcp_feed_works`] has walked the
/// whole path on the ffmpeg in question. A pipe cannot fail to connect and a
/// socket can; the probe is what turns that from a failed render into a
/// quiet fallback.
enum FrameSink {
    Pipe(std::process::ChildStdin),
    /// The stream, and how much of a frame to hand it per write.
    Tcp(std::net::TcpStream, usize),
}

impl FrameSink {
    /// Waits for ffmpeg to dial in, when a listener was opened. ffmpeg is
    /// already running by this point, so the only outcomes are a connection
    /// or a diagnosable failure — there is no going back to a pipe here,
    /// because the child was told where to read from before it started.
    fn open(
        listener: Option<std::net::TcpListener>,
        stdin: Option<std::process::ChildStdin>,
        chunk: usize,
        log: &FfmpegLog,
    ) -> Result<FrameSink, Error> {
        let Some(listener) = listener else {
            return stdin
                .map(FrameSink::Pipe)
                .ok_or_else(|| Error::Ffmpeg("ffmpeg stdin unavailable".into()));
        };
        listener
            .set_nonblocking(true)
            .map_err(|e| Error::Ffmpeg(format!("could not arm the frame socket: {e}")))?;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        loop {
            match listener.accept() {
                Ok((sock, _)) => {
                    sock.set_nonblocking(false).map_err(|e| {
                        Error::Ffmpeg(format!("could not settle the frame socket: {e}"))
                    })?;
                    // Frames are large and strictly ordered; waiting to
                    // coalesce them buys nothing and costs latency.
                    let _ = sock.set_nodelay(true);
                    return Ok(FrameSink::Tcp(sock, chunk));
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    if std::time::Instant::now() >= deadline {
                        return Err(Error::Ffmpeg(format!(
                            "ffmpeg never connected to the frame socket{}",
                            log.tail()
                        )));
                    }
                    std::thread::sleep(std::time::Duration::from_millis(5));
                }
                Err(e) => {
                    return Err(Error::Ffmpeg(format!("frame socket failed: {e}{}", log.tail())))
                }
            }
        }
    }
}

impl FrameSink {
    /// Hands one whole frame over, in whatever shape this transport wants.
    ///
    /// The two want opposite things, which is not obvious and took a
    /// measurement to find. On 4K NV12 frames into a real ffmpeg:
    ///
    /// ```text
    ///            whole    256 KiB   64 KiB   16 KiB    4 KiB
    ///   pipe      8.6 ms      —      7.5 ms   6.2 ms   7.9 ms
    ///   socket    4.25 ms   4.46 ms  4.83 ms  6.14 ms     —
    /// ```
    ///
    /// A single large write into a pipe fills it and then waits for the
    /// reader to drain enough of it, so the two processes take turns; small
    /// writes come back as soon as there is room. A socket has no such
    /// ceiling and every extra write is pure syscall, so it wants the frame
    /// in one go. Chunking both — which this did at first — left a third of
    /// the socket's advantage on the floor.
    ///
    /// Enlarging the socket's send buffer was measured too and does nothing:
    /// 5.02 ms by default against 4.99 at 1 MiB and 5.27 at 4 MiB.
    fn write_frame(&mut self, frame: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;
        match self {
            FrameSink::Pipe(p) => {
                const PIECE: usize = 16 * 1024;
                for part in frame.chunks(PIECE) {
                    p.write_all(part)?;
                }
                Ok(())
            }
            FrameSink::Tcp(s, chunk) => match *chunk {
                0 => s.write_all(frame),
                n => {
                    for part in frame.chunks(n) {
                        s.write_all(part)?;
                    }
                    Ok(())
                }
            },
        }
    }
}


impl std::io::Write for FrameSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            FrameSink::Pipe(p) => p.write(buf),
            FrameSink::Tcp(s, _) => s.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            FrameSink::Pipe(p) => p.flush(),
            FrameSink::Tcp(s, _) => s.flush(),
        }
    }
}

/// What a video encoder needs on the command line: a tail for the filter
/// chain (hardware encoders want their frames uploaded to VRAM first) and the
/// codec, quality and preset arguments themselves.
struct EncoderArgs {
    filter: String,
    args: Vec<String>,
}

/// Builds them for one encoder.
///
/// [`encoder_error`] runs this SAME function, which is the point of it
/// existing: an option that an ffmpeg build or a driver rejects then makes
/// that encoder come out as unavailable and auto-selection moves on, instead
/// of the mistake surfacing as a failed encode after someone has sat through
/// a render. That matters most for AMF, which cannot be tested on the machine
/// this was written on — a wrong flag there costs an AMD owner nothing worse
/// than the software fallback they already had.
fn video_encoder_args(
    encoder: &str,
    quality: u32,
    preset: &str,
    feed_nv12: bool,
    vaapi_icq: bool,
) -> EncoderArgs {
    let hw = crate::quality::hardware_q(quality).to_string();
    let mut filter = String::new();
    let mut args: Vec<String> = Vec::new();
    let mut add = |v: &[&str]| args.extend(v.iter().map(|s| (*s).to_string()));
    match encoder {
        "vaapi" => {
            // Already NV12 on the way in: no swscale pass to pay for.
            filter = if feed_nv12 { "hwupload" } else { "format=nv12,hwupload" }.into();
            add(&["-c:v", "h264_vaapi", "-profile:v", "high"]);
            if vaapi_icq {
                // Intelligent constant quality: the driver varies the
                // quantiser with the picture the way x264's CRF does. This
                // used to send a flat -qp, which spends bits on still frames
                // and starves busy ones.
                add(&["-rc_mode", "ICQ", "-global_quality", &hw]);
            } else {
                add(&["-rc_mode", "CQP", "-qp", &hw]);
            }
        }
        "nvenc" => {
            // nvenc takes nv12 directly. Asking for yuv420p while we hand it
            // nv12 makes ffmpeg insert a full-frame swscale pass per frame to
            // shuffle the two chroma planes apart, for nothing.
            let pix = if feed_nv12 { "nv12" } else { "yuv420p" };
            add(&["-c:v", "h264_nvenc", "-pix_fmt", pix, "-profile:v", "high"]);
            add(&["-preset", nvenc_preset(preset)]);
            // Constant quality on nvenc is VBR with no average target: the
            // -b:v 0 is not decoration, without it ffmpeg's default 2 Mbit/s
            // average applies and 4K falls apart.
            add(&["-rc", "vbr", "-cq", &hw, "-b:v", "0"]);
        }
        "qsv" => {
            add(&["-c:v", "h264_qsv", "-pix_fmt", "nv12", "-profile:v", "high"]);
            add(&["-preset", qsv_preset(preset)]);
            // ICQ, which is what QuickSync picks when global_quality is set
            // and no bitrate cap is. Never add -maxrate or -b:v here: either
            // silently demotes this to plain VBR.
            add(&["-global_quality", &hw]);
        }
        "amf" => {
            // AMD on Windows. Without this an AMD owner falls all the way
            // through to software x264, because nvenc is the wrong vendor,
            // QuickSync needs an Intel chip, and VAAPI is Linux-only.
            let pix = if feed_nv12 { "nv12" } else { "yuv420p" };
            add(&["-c:v", "h264_amf", "-pix_fmt", pix, "-profile:v", "high"]);
            add(&["-quality", amf_quality(preset)]);
            add(&["-rc", "cqp", "-qp_i", &hw, "-qp_p", &hw, "-qp_b", &hw]);
        }
        _ => {
            add(&["-c:v", "libx264", "-pix_fmt", "yuv420p", "-profile:v", "high"]);
            let crf = crate::quality::x264_crf(quality).to_string();
            add(&["-crf", &crf, "-preset", preset]);
        }
    }
    EncoderArgs { filter, args }
}

/// AMF's three-step quality knob, from the x264 preset name.
fn amf_quality(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "speed",
        "slow" | "slower" | "veryslow" | "placebo" => "quality",
        _ => "balanced",
    }
}

/// The DRM render node VAAPI should use.
///
/// renderD128 is usually right and used to be hardcoded, but on a hybrid
/// laptop the first node is often the display-only chip while the encoder
/// lives on the second — rhythr then reported "no VAAPI" on a machine that
/// has one. Enumerated and probed once per process.
pub fn vaapi_device(ffmpeg: &str) -> Option<String> {
    // Keyed by ffmpeg path, because the answer belongs to the binary and not
    // to the process: the app resolves its ffmpeg at runtime and the user can
    // repoint it in Settings. A single cached answer meant the verdict for
    // whichever binary happened to be probed first — including a "no VAAPI
    // here" that then stuck to a perfectly capable replacement until restart.
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::HashMap<String, Option<String>>>,
    > = std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(ffmpeg).cloned()) {
        return hit;
    }
    let mut nodes: Vec<String> = std::fs::read_dir("/dev/dri")
        .into_iter()
        .flatten()
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("renderD"))
        })
        .filter_map(|p| p.to_str().map(str::to_owned))
        .collect();
    nodes.sort();
    let found = nodes
        .into_iter()
        .find(|dev| vaapi_probe(ffmpeg, dev, false).is_ok());
    if let Ok(mut c) = cache.lock() {
        c.insert(ffmpeg.to_string(), found.clone());
    }
    found
}

/// Runs a tiny real encode on one render node. `icq` asks for the rate
/// control mode we would rather use, so the answer is about the mode and not
/// just about the device.
fn vaapi_probe(ffmpeg: &str, device: &str, icq: bool) -> Result<(), String> {
    let mut cmd = Command::new(ffmpeg);
    hide_console_window(&mut cmd);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    cmd.args(["-vaapi_device", device]);
    cmd.args(["-f", "lavfi", "-i", "color=black:size=256x256:rate=30:duration=0.1"]);
    cmd.args(["-vf", "format=nv12,hwupload", "-c:v", "h264_vaapi"]);
    if icq {
        cmd.args(["-rc_mode", "ICQ", "-global_quality", "23"]);
    }
    cmd.args(["-f", "null", "-"]);
    // The reason is kept rather than discarded. Throwing it away and telling
    // everyone "no render device in /dev/dri" named the wrong cause for every
    // other way this fails — an ffmpeg built without VAAPI, a broken libva
    // driver, or simply not being in the `render` group.
    match cmd.stdout(Stdio::null()).stderr(Stdio::piped()).output() {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(last_meaningful_line(&out.stderr)),
        Err(e) => Err(format!("could not run ffmpeg: {e}")),
    }
}

/// ffmpeg's own explanation, which is nearly always its last non-empty line.
fn last_meaningful_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .rev()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or("encoder test failed")
        .to_string()
}

/// Whether this VAAPI driver accepts intelligent-constant-quality. Several do
/// not, and asking for a mode a driver lacks fails the whole encode, so the
/// answer decides between ICQ and a flat quantiser. Probed once.
fn vaapi_icq_supported(ffmpeg: &str, needed: bool) -> bool {
    if !needed {
        return false;
    }
    static CACHE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, bool>>> =
        std::sync::OnceLock::new();
    let cache = CACHE.get_or_init(Default::default);
    if let Some(hit) = cache.lock().ok().and_then(|c| c.get(ffmpeg).copied()) {
        return hit;
    }
    let ok = match vaapi_device(ffmpeg) {
        Some(dev) => vaapi_probe(ffmpeg, &dev, true).is_ok(),
        None => false,
    };
    if let Ok(mut c) = cache.lock() {
        c.insert(ffmpeg.to_string(), ok);
    }
    ok
}

/// Translates the x264 speed preset the user picked into nvenc's p1..p7
/// scale. Without this the hardware encoder ignored the speed control
/// completely and always ran p5, one of the slowest quality presets — which
/// is what pins the encoder at 100% on a 4K120 render while the GPU's 3D
/// engine idles at 17%.
fn nvenc_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" => "p1",
        "veryfast" => "p2",
        "faster" => "p3",
        "fast" | "medium" => "p4",
        "slow" => "p5",
        "slower" => "p6",
        "veryslow" | "placebo" => "p7",
        _ => "p4",
    }
}

/// Same for QuickSync, which shares x264's preset names but does not accept
/// the two fastest or the slowest one. Passing an unknown name makes ffmpeg
/// abort, so the ends are clamped rather than forwarded.
fn qsv_preset(preset: &str) -> &'static str {
    match preset {
        "ultrafast" | "superfast" | "veryfast" => "veryfast",
        "faster" => "faster",
        "fast" => "fast",
        "slow" => "slow",
        "slower" | "veryslow" | "placebo" => "slower",
        _ => "medium",
    }
}

/// Like [`encoder_works`], but on failure returns ffmpeg's stderr (its last
/// meaningful line) so the UI can say WHY an encoder is unavailable — e.g.
/// nvenc rejecting an outdated NVIDIA driver.
pub fn encoder_error(ffmpeg: &str, encoder: &str) -> Option<String> {
    let mut cmd = std::process::Command::new(ffmpeg);
    hide_console_window(&mut cmd);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    match encoder {
        "vaapi" => match vaapi_device(ffmpeg) {
            Some(dev) => {
                cmd.args(["-vaapi_device", &dev]);
            }
            // Say what ffmpeg said. "No render device" is only one of the
            // reasons this fails and was being printed for all of them.
            None => {
                return Some(match vaapi_probe(ffmpeg, "/dev/dri/renderD128", false) {
                    Ok(()) => "no working VAAPI render device in /dev/dri".into(),
                    Err(why) => why,
                })
            }
        },
        "nvenc" | "qsv" | "amf" => {}
        _ => return None, // software x264 always works
    }
    cmd.args([
        "-f",
        "lavfi",
        "-i",
        "color=black:size=256x256:rate=30:duration=0.1",
    ]);
    // The SAME arguments a real render would use, at the default quality and
    // preset. Probing with a bare `-c:v` instead used to pass on options the
    // encode then choked on, which turned a wrong flag into a failed render
    // rather than an encoder that quietly does not appear in the list.
    let enc = video_encoder_args(
        encoder,
        crate::quality::DEFAULT,
        "medium",
        false,
        vaapi_icq_supported(ffmpeg, encoder == "vaapi"),
    );
    if !enc.filter.is_empty() {
        cmd.args(["-vf", &enc.filter]);
    }
    cmd.args(&enc.args);
    cmd.args(["-f", "null", "-"]);
    let output = cmd
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .output();
    match output {
        Ok(out) if out.status.success() => None,
        // The last non-empty line is usually the actual reason ("driver does
        // not support the required nvenc API version…").
        Ok(out) => Some(last_meaningful_line(&out.stderr)),
        Err(e) => Some(format!("could not run ffmpeg: {e}")),
    }
}
