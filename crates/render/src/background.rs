//! Custom playfield background: a user-chosen image or video drawn behind
//! gameplay instead of the skin's background, with an adjustable dim.
//!
//! Images ride the existing skin background-layer pipeline (a synthetic
//! cover layer). Videos are decoded by the bundled ffmpeg into a rawvideo
//! pipe (one RGBA frame per output frame, looped and muted) and streamed
//! into a persistent texture. The results screen is deliberately untouched.

use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use crate::config::{BackgroundLayer, SkinConfig};

/// What kind of file the user picked. Detection is by content (magic
/// bytes), not extension: "support as many formats as possible" means an
/// image is whatever the image decoder recognises and everything else is
/// handed to ffmpeg.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundKind {
    Image,
    Video,
}

/// Sniffs the first bytes of a file. Animated GIFs go to ffmpeg so they
/// actually play.
pub fn classify_bytes(head: &[u8]) -> BackgroundKind {
    match image::guess_format(head) {
        // Animated GIFs must play (ffmpeg's problem).
        Ok(image::ImageFormat::Gif) => BackgroundKind::Video,
        Ok(_) => BackgroundKind::Image,
        Err(_) => BackgroundKind::Video,
    }
}

/// Reads just enough of the file to classify it.
pub fn classify_file(path: &Path) -> std::io::Result<BackgroundKind> {
    use std::io::Read;
    let mut head = [0u8; 64];
    let mut f = std::fs::File::open(path)?;
    let n = f.read(&mut head)?;
    Ok(classify_bytes(&head[..n]))
}

/// Applies a custom background to the render config: replaces the skin's
/// background layers (an image becomes one frame-covering layer with the
/// user's zoom/shift, a video flips the streaming flag) and sets the dim.
/// Returns the detected kind.
pub fn apply_background(
    cfg: &mut SkinConfig,
    path: &Path,
    opts: &BackgroundOptions,
) -> std::io::Result<BackgroundKind> {
    let kind = classify_file(path)?;
    cfg.background_images.clear();
    cfg.custom_bg_dim = Some(opts.dim.clamp(0.0, 1.0));
    match kind {
        BackgroundKind::Image => {
            cfg.background_images
                .push(cover_layer(std::fs::read(path)?, opts));
            cfg.custom_bg_video = false;
        }
        BackgroundKind::Video => cfg.custom_bg_video = true,
    }
    Ok(kind)
}

/// A screen-space layer that covers the whole frame: cover-fit times the
/// user's zoom, centre shifted by the offset (the compositor clamps the
/// shift so the frame stays covered).
fn cover_layer(bytes: Vec<u8>, opts: &BackgroundOptions) -> BackgroundLayer {
    let zoom = opts.zoom.clamp(1.0, 4.0);
    BackgroundLayer {
        bytes,
        fit: 2,
        placement: 0,
        center_x: 0.5 + opts.offset[0].clamp(-1.0, 1.0),
        center_y: 0.5 + opts.offset[1].clamp(-1.0, 1.0),
        scale_x: zoom,
        scale_y: zoom,
        flip_horizontal: false,
        space_x: 0.0,
        space_y: 0.0,
        space_w: 0.0,
        space_h: 0.0,
        tint: [1.0, 1.0, 1.0, 1.0],
    }
}

/// Parses the duration in seconds out of ffmpeg's `-i` banner (we do not
/// ship ffprobe): the line looks like `  Duration: 00:01:23.45, start: …`.
pub fn parse_ffmpeg_duration(stderr: &str) -> Option<f64> {
    let idx = stderr.find("Duration: ")?;
    let field = stderr[idx + 10..].split([',', '\n']).next()?.trim();
    let mut parts = field.split(':');
    let h: f64 = parts.next()?.trim().parse().ok()?;
    let m: f64 = parts.next()?.parse().ok()?;
    let s: f64 = parts.next()?.parse().ok()?;
    Some(h * 3600.0 + m * 60.0 + s)
}

/// The video's own frame rate, from the same `-i` probe output that
/// carries the duration (`… , 30 fps, …`). None when it cannot be read.
pub fn parse_ffmpeg_fps(stderr: &str) -> Option<f64> {
    // Take the LAST match: a file with several streams lists the video one
    // after any others, and only a video stream reports fps.
    let mut best = None;
    for (i, _) in stderr.match_indices(" fps") {
        let head = &stderr[..i];
        let num: String = head
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_digit() || *c == '.')
            .collect();
        if let Ok(v) = num.chars().rev().collect::<String>().parse::<f64>() {
            if v > 0.0 && v < 1000.0 {
                best = Some(v);
            }
        }
    }
    best
}

/// Full decode check so a corrupt image errors when the user picks it,
/// not as a silently black render.
pub fn image_decodes(bytes: &[u8]) -> bool {
    image::load_from_memory(bytes).is_ok()
}

/// How the user placed the custom background. Defaults reproduce the
/// plain cover behaviour exactly.
#[derive(Debug, Clone, Copy)]
pub struct BackgroundOptions {
    /// 0..1 darkening of the background quad.
    pub dim: f32,
    /// Extra zoom on top of cover-fit (1.0 = exactly covering).
    pub zoom: f32,
    /// Shift as a fraction of the frame size (positive = image moves
    /// right/down). Clamped to the available overflow so the frame stays
    /// covered (no black bars).
    pub offset: [f32; 2],
    /// Videos: playback starts (and loops) from this point.
    pub start_secs: f64,
    /// Videos: the FIRST pass begins this far into the loop window
    /// (song-synced clip renders); later loops return to start_secs.
    pub sync_offset_secs: f64,
}

impl Default for BackgroundOptions {
    fn default() -> Self {
        BackgroundOptions {
            dim: 0.6,
            zoom: 1.0,
            offset: [0.0, 0.0],
            start_secs: 0.0,
            sync_offset_secs: 0.0,
        }
    }
}

/// Clamps a wanted shift (px) to what the content can afford beyond the
/// frame, keeping the frame fully covered.
pub fn clamp_cover_offset(wanted: f32, content: f32, frame: f32) -> f32 {
    let spare = ((content - frame) * 0.5).max(0.0);
    wanted.clamp(-spare, spare)
}

/// Scale-to-cover filter with the user's zoom and shift: fill the frame,
/// crop the overflow around the shifted window. The crop position is
/// clamped inside ffmpeg (the source size is only known at run time).
fn cover_vf(w: u32, h: u32, zoom: f32, offset: [f32; 2]) -> String {
    let zoom = zoom.clamp(1.0, 4.0);
    let (sw, sh) = (
        (w as f32 * zoom).ceil() as u32,
        (h as f32 * zoom).ceil() as u32,
    );
    let ox = (offset[0].clamp(-1.0, 1.0) * w as f32).round() as i64;
    let oy = (offset[1].clamp(-1.0, 1.0) * h as f32).round() as i64;
    format!(
        "scale={sw}:{sh}:force_original_aspect_ratio=increase,\
         crop={w}:{h}:x='clip((iw-{w})/2-({ox}),0,iw-{w})':y='clip((ih-{h})/2-({oy}),0,ih-{h})'"
    )
}

/// A muted, looping ffmpeg decode of the background video, delivering
/// exactly one frame-sized RGBA frame per output frame. Looping is done
/// by respawning the decoder at end-of-stream, so playback restarts at
/// the USER'S start point, not at an intro the start point was meant to
/// skip.
pub struct VideoDecoder {
    child: Child,
    stdout: ChildStdout,
    /// Everything needed to respawn for the next loop.
    spec: (
        String,
        std::path::PathBuf,
        u32,
        u32,
        u32,
        f64,
        f32,
        [f32; 2],
    ),
    /// Frames delivered by the current child. A child that dies without
    /// producing any is broken (or the start point is past the end), and
    /// respawning it would loop forever.
    child_frames: u64,
    /// First-pass frames, recorded while they fit LOOP_BUF_CAP. A loop
    /// that fits entirely replays from memory: a 1-frame GIF must not
    /// cost one ffmpeg spawn per loop iteration in the render hot path.
    loop_buf: Vec<Vec<u8>>,
    loop_bytes: usize,
    /// Still recording the first pass into loop_buf.
    buffering: bool,
    /// Serving loops from loop_buf; no child process anymore.
    mem_loop: bool,
    buf_idx: usize,
    /// Bytes in one frame, to reject a caller buffer of the wrong size.
    frame_len: usize,
    done: bool,
}

impl VideoDecoder {
    fn spawn_child(
        (ffmpeg, path, width, height, fps, start, zoom, offset): &(
            String,
            std::path::PathBuf,
            u32,
            u32,
            u32,
            f64,
            f32,
            [f32; 2],
        ),
    ) -> Result<(Child, ChildStdout), String> {
        let mut cmd = Command::new(ffmpeg);
        crate::video::hide_console_window(&mut cmd);
        cmd.args(["-loglevel", "error"]);
        if *start > 0.0 {
            cmd.args(["-ss", &format!("{start:.3}")]);
        }
        cmd.args(["-an", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgba"])
            .args(["-vf", &cover_vf(*width, *height, *zoom, *offset)])
            .args(["-r", &fps.to_string(), "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start ffmpeg ({ffmpeg}): {e}"))?;
        let stdout = child.stdout.take().ok_or("ffmpeg gave no stdout")?;
        Ok((child, stdout))
    }

    pub fn spawn(
        ffmpeg: &str,
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
        opts: &BackgroundOptions,
    ) -> Result<VideoDecoder, String> {
        let spec = (
            ffmpeg.to_string(),
            path.to_path_buf(),
            width,
            height,
            fps,
            opts.start_secs.max(0.0),
            opts.zoom,
            opts.offset,
        );
        let sync = opts.sync_offset_secs.max(0.0);
        let (child, stdout) = if sync > 0.0 {
            let mut first = spec.clone();
            first.5 += sync;
            Self::spawn_child(&first)?
        } else {
            Self::spawn_child(&spec)?
        };
        Ok(VideoDecoder {
            child,
            stdout,
            spec,
            child_frames: 0,
            loop_buf: Vec::new(),
            loop_bytes: 0,
            buffering: sync <= 0.0,
            mem_loop: false,
            buf_idx: 0,
            frame_len: (width * height * 4) as usize,
            done: false,
        })
    }

    /// Byte cap for the in-memory loop buffer (~7 frames at 1080p):
    /// enough for 1-frame GIFs and tiny loops, negligible for real
    /// videos, which keep the respawn path.
    const LOOP_BUF_CAP: usize = 64 * 1024 * 1024;

    /// Fills `dst` with the frame for the next output frame; `false` means
    /// the stream is finished and the caller should keep showing whatever it
    /// had. At end-of-stream the decoder respawns, looping from the start
    /// point rather than from an intro the start point was meant to skip.
    ///
    /// Reads STRAIGHT into the caller's buffer. The previous version read
    /// into a scratch buffer and then copied that into a kept frame, so
    /// every output frame paid a full frame-sized memcpy (8 MB at 1080p,
    /// 33 MB at 4K) purely to guarantee that a failed read could not
    /// corrupt the last good frame. Reporting failure and letting the
    /// caller keep its own last frame gives the same guarantee for free.
    pub fn next_frame_into(&mut self, dst: &mut [u8]) -> bool {
        use std::io::Read;
        if dst.len() != self.frame_len {
            return false;
        }
        if self.mem_loop {
            dst.copy_from_slice(&self.loop_buf[self.buf_idx]);
            self.buf_idx = (self.buf_idx + 1) % self.loop_buf.len();
            return true;
        }
        if self.done {
            return false;
        }
        match self.stdout.read_exact(dst) {
            Ok(()) => {
                self.child_frames += 1;
                if self.buffering {
                    if self.loop_bytes + dst.len() <= Self::LOOP_BUF_CAP {
                        self.loop_buf.push(dst.to_vec());
                        self.loop_bytes += dst.len();
                    } else {
                        // Too big to keep: this loop respawns instead.
                        self.buffering = false;
                        self.loop_buf.clear();
                        self.loop_bytes = 0;
                    }
                }
                true
            }
            Err(_) if self.child_frames > 0 => {
                let _ = self.child.kill();
                let _ = self.child.wait();
                if self.buffering && !self.loop_buf.is_empty() {
                    // The whole loop fit in memory: serve it from there,
                    // no more child processes.
                    self.buffering = false;
                    self.mem_loop = true;
                    dst.copy_from_slice(&self.loop_buf[0]);
                    self.buf_idx = 1 % self.loop_buf.len();
                    return true;
                }
                // End of stream: loop by respawning at the start point.
                self.child_frames = 0;
                match Self::spawn_child(&self.spec) {
                    Ok((child, stdout)) => {
                        self.child = child;
                        self.stdout = stdout;
                        if self.stdout.read_exact(dst).is_ok() {
                            self.child_frames = 1;
                            true
                        } else {
                            self.done = true;
                            false
                        }
                    }
                    Err(_) => {
                        self.done = true;
                        false
                    }
                }
            }
            Err(_) => {
                self.done = true;
                false
            }
        }
    }
}

/// Runs a [`VideoDecoder`] on its own thread and hands finished frames to
/// the render loop through a small queue.
///
/// Decoding used to sit in the middle of the render loop: it blocked on the
/// decoder's pipe for a whole frame, copied it, and only then let the GPU
/// have any work. Measured on a 15 s 1080p clip, adding a video background
/// took the render from 8.7 s to 20.1 s while the GPU sat at 41%: the
/// extra time was almost entirely waiting.
///
/// The queue is deliberately short. It exists so the decoder can run a
/// little ahead, not so it can decode the whole video into memory: when the
/// render loop falls behind, the queue fills and the decoder blocks, which
/// is exactly the back-pressure that keeps this from eating the machine.
pub struct BackgroundStream {
    /// Source frames per output frame (never above 1: the decoder is asked
    /// for at most the output rate).
    per_output: f64,
    /// Fractional source frames owed; starts at 1 so the first output frame
    /// gets a picture.
    owed: f64,
    frames: std::sync::mpsc::Receiver<Vec<u8>>,
    /// Spent buffers going back to the decoder, so a steady state allocates
    /// nothing at all.
    spare: std::sync::mpsc::Sender<Vec<u8>>,
    worker: Option<std::thread::JoinHandle<()>>,
    /// The most recent frame, kept so a decoder that stalls or ends leaves
    /// the background as it was instead of blanking it.
    last: Option<Vec<u8>>,
}

impl BackgroundStream {
    /// Frames the decoder may run ahead by. Three is enough to cover a
    /// slow decode without holding more than a few frames of memory
    /// (~100 MB at 4K, ~25 MB at 1080p).
    const QUEUE: usize = 3;

    /// `src_fps` is the rate the decoder was actually asked for and
    /// `out_fps` the video being rendered. A 30 fps background under a
    /// 60 fps render shows each frame twice. The old code made ffmpeg
    /// duplicate it and pushed both copies through the pipe and up to the
    /// GPU, two 8 MB transfers (33 MB at 4K) for one picture.
    pub fn spawn(
        mut decoder: VideoDecoder,
        frame_len: usize,
        src_fps: f64,
        out_fps: f64,
    ) -> BackgroundStream {
        let (frame_tx, frames) = std::sync::mpsc::sync_channel::<Vec<u8>>(Self::QUEUE);
        let (spare, spare_rx) = std::sync::mpsc::channel::<Vec<u8>>();
        let worker = std::thread::spawn(move || {
            loop {
                // Reuse a returned buffer when there is one; the first few
                // frames allocate, then it is recycling only.
                let mut buf = match spare_rx.try_recv() {
                    Ok(b) => b,
                    Err(_) => vec![0u8; frame_len],
                };
                if buf.len() != frame_len {
                    buf.resize(frame_len, 0);
                }
                if !decoder.next_frame_into(&mut buf) {
                    return; // stream finished: closing the channel says so
                }
                // A full queue blocks here, which is the back-pressure.
                if frame_tx.send(buf).is_err() {
                    return; // render loop went away
                }
            }
        });
        BackgroundStream {
            per_output: (src_fps / out_fps.max(1.0)).clamp(0.0, 1.0),
            owed: 1.0,
            frames,
            spare,
            worker: Some(worker),
            last: None,
        }
    }

    /// A NEW background frame for this output frame, or None when the one
    /// already on the GPU is still the right picture. The caller skips its
    /// upload then. Blocks only while the decoder is genuinely behind.
    pub fn next_frame(&mut self) -> Option<&[u8]> {
        self.owed += self.per_output;
        if self.owed < 1.0 {
            return None; // same source frame as last time
        }
        self.owed -= 1.0;
        match self.frames.recv() {
            Ok(buf) => {
                if let Some(old) = self.last.replace(buf) {
                    let _ = self.spare.send(old);
                }
                self.last.as_deref()
            }
            // Finished or failed: whatever is on the GPU stays.
            Err(_) => None,
        }
    }
}

impl Drop for BackgroundStream {
    fn drop(&mut self) {
        // Dropping the receiver makes the worker's send fail, which ends it;
        // draining first releases a worker that is blocked on a full queue.
        while self.frames.try_recv().is_ok() {}
        if let Some(w) = self.worker.take() {
            drop(std::mem::replace(
                &mut self.frames,
                std::sync::mpsc::sync_channel(0).1,
            ));
            let _ = w.join();
        }
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The video's duration via ffmpeg's `-i` banner (no ffprobe shipped).
/// Where a song-synced clip render enters the video's loop window:
/// the clip's elapsed OUTPUT time folded into [0, duration - start).
pub fn sync_offset(elapsed_out_secs: f64, start_secs: f64, duration: Option<f64>) -> f64 {
    let start = start_secs.max(0.0);
    match duration {
        Some(d) if d - start > 0.05 => elapsed_out_secs.max(0.0) % (d - start),
        _ => 0.0,
    }
}

/// Duration and native frame rate in one probe.
pub fn probe_video(ffmpeg: &str, path: &Path) -> (Option<f64>, Option<f64>) {
    let mut cmd = Command::new(ffmpeg);
    crate::video::hide_console_window(&mut cmd);
    let Ok(out) = cmd
        .args(["-hide_banner", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
    else {
        return (None, None);
    };
    let text = String::from_utf8_lossy(&out.stderr);
    (parse_ffmpeg_duration(&text), parse_ffmpeg_fps(&text))
}

pub fn probe_duration(ffmpeg: &str, path: &Path) -> Option<f64> {
    let mut cmd = Command::new(ffmpeg);
    crate::video::hide_console_window(&mut cmd);
    let out = cmd
        .args(["-hide_banner", "-i"])
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .ok()?;
    parse_ffmpeg_duration(&String::from_utf8_lossy(&out.stderr))
}

/// One RGBA frame at `t_secs` for the live preview, wrapped over the
/// loop window `[start, duration)` so scrubbing matches what the render
/// will show.
pub fn extract_frame(
    ffmpeg: &str,
    path: &Path,
    t_secs: f64,
    duration: Option<f64>,
    width: u32,
    height: u32,
    opts: &BackgroundOptions,
) -> Option<Vec<u8>> {
    use std::io::Read;
    let start = opts.start_secs.max(0.0);
    let t = match duration {
        Some(d) if d - start > 0.05 => start + (t_secs.max(0.0) % (d - start)),
        _ => start + t_secs.max(0.0),
    };
    let mut cmd = Command::new(ffmpeg);
    crate::video::hide_console_window(&mut cmd);
    let mut child = cmd
        .args(["-loglevel", "error", "-ss", &format!("{t:.3}"), "-an", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args([
            "-vf",
            &cover_vf(width, height, opts.zoom, opts.offset),
            "pipe:1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut frame = vec![0; (width * height * 4) as usize];
    let ok = child
        .stdout
        .take()
        .map(|mut s| s.read_exact(&mut frame).is_ok())
        .unwrap_or(false);
    let _ = child.kill();
    let _ = child.wait();
    ok.then_some(frame)
}

#[cfg(test)]
mod tests {

    #[test]
    fn reads_the_frame_rate_out_of_an_ffmpeg_probe() {
        let real = "  Duration: 00:00:12.00, start: 0.000000, bitrate: 6400 kb/s\n                    Stream #0:0: Video: h264, yuv420p, 1920x1080, 6397 kb/s, 30 fps, 30 tbr";
        assert_eq!(super::parse_ffmpeg_fps(real), Some(30.0));
        // Fractional rates and a leading audio stream (no fps of its own).
        let ntsc = "Stream #0:0: Audio: aac, 48000 Hz\n                    Stream #0:1: Video: h264, 1280x720, 29.97 fps, 30 tbr";
        assert_eq!(super::parse_ffmpeg_fps(ntsc), Some(29.97));
        // Nothing to find must not invent a rate.
        assert_eq!(super::parse_ffmpeg_fps("Duration: 00:00:01.00"), None);
        assert_eq!(super::parse_ffmpeg_fps(""), None);
    }

    #[test]
    fn sync_offset_folds_into_loop_window() {
        // 40 s video, no intro skip: 90 s into the song = 10 s into pass 3.
        assert!((super::sync_offset(90.0, 0.0, Some(40.0)) - 10.0).abs() < 1e-9);
        // Intro skip 5 s → 35 s window: 90 % 35 = 20.
        assert!((super::sync_offset(90.0, 5.0, Some(40.0)) - 20.0).abs() < 1e-9);
        // Unknown or degenerate durations never offset.
        assert_eq!(super::sync_offset(90.0, 0.0, None), 0.0);
        assert_eq!(super::sync_offset(90.0, 39.99, Some(40.0)), 0.0);
        assert_eq!(super::sync_offset(-3.0, 0.0, Some(40.0)), 0.0);
    }

    use super::*;

    #[test]
    fn images_are_recognised_by_content_not_extension() {
        // PNG / JPEG / WebP / BMP magic bytes → image.
        assert_eq!(
            classify_bytes(b"\x89PNG\r\n\x1a\n........"),
            BackgroundKind::Image
        );
        assert_eq!(
            classify_bytes(b"\xff\xd8\xff\xe0....JFIF"),
            BackgroundKind::Image
        );
        assert_eq!(
            classify_bytes(b"RIFF\x00\x00\x00\x00WEBPVP8 "),
            BackgroundKind::Image
        );
        assert_eq!(
            classify_bytes(b"BM\x00\x00\x00\x00\x00\x00\x00\x00"),
            BackgroundKind::Image
        );
    }

    #[test]
    fn gifs_and_unknown_containers_go_to_ffmpeg() {
        // Animated GIFs must PLAY, so they are video; mp4/webm/mkv are
        // never image formats; garbage falls through to ffmpeg, which
        // will produce the real error message.
        assert_eq!(
            classify_bytes(b"GIF89a\x00\x00\x00\x00"),
            BackgroundKind::Video
        );
        assert_eq!(
            classify_bytes(b"\x00\x00\x00\x20ftypisom...."),
            BackgroundKind::Video
        );
        assert_eq!(
            classify_bytes(b"\x1a\x45\xdf\xa3............"),
            BackgroundKind::Video
        );
        assert_eq!(
            classify_bytes(b"not a media file at all....."),
            BackgroundKind::Video
        );
    }

    #[test]
    fn applying_a_background_replaces_skin_layers_and_sets_the_dim() {
        let dir = std::env::temp_dir().join("rhythr-bg-test");
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("bg.png");
        std::fs::write(&img, b"\x89PNG\r\n\x1a\n....").unwrap();

        let mut cfg = SkinConfig::default();
        cfg.background_images
            .push(cover_layer(vec![1], &BackgroundOptions::default()));
        let opts = BackgroundOptions {
            dim: 0.6,
            zoom: 1.5,
            offset: [0.25, -0.1],
            ..Default::default()
        };
        let kind = apply_background(&mut cfg, &img, &opts).unwrap();
        assert_eq!(kind, BackgroundKind::Image);
        assert_eq!(cfg.background_images.len(), 1);
        assert!(!cfg.custom_bg_video);
        assert_eq!(cfg.custom_bg_dim, Some(0.6));
        // The layer carries the user's zoom and shift.
        let l = &cfg.background_images[0];
        assert_eq!((l.scale_x, l.scale_y), (1.5, 1.5));
        assert_eq!((l.center_x, l.center_y), (0.75, 0.4));

        let vid = dir.join("bg.mp4");
        std::fs::write(&vid, b"\x00\x00\x00\x20ftypisom").unwrap();
        let kind = apply_background(
            &mut cfg,
            &vid,
            &BackgroundOptions {
                dim: 1.5,
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(kind, BackgroundKind::Video);
        assert!(cfg.background_images.is_empty());
        assert!(cfg.custom_bg_video);
        assert_eq!(cfg.custom_bg_dim, Some(1.0));
    }

    #[test]
    fn cover_offset_clamps_to_the_available_overflow() {
        // Content 200 wide over a 100 frame: 50 px spare per side.
        assert_eq!(clamp_cover_offset(20.0, 200.0, 100.0), 20.0);
        assert_eq!(clamp_cover_offset(80.0, 200.0, 100.0), 50.0);
        assert_eq!(clamp_cover_offset(-80.0, 200.0, 100.0), -50.0);
        // Content no bigger than the frame: nothing to shift.
        assert_eq!(clamp_cover_offset(30.0, 100.0, 100.0), 0.0);
    }

    #[test]
    fn cover_filter_defaults_stay_plain_and_zoom_shift_clamp_in_expr() {
        // zoom 1 / no shift: centred crop, same behaviour as before.
        assert_eq!(
            cover_vf(1920, 1080, 1.0, [0.0, 0.0]),
            "scale=1920:1080:force_original_aspect_ratio=increase,crop=1920:1080:x='clip((iw-1920)/2-(0),0,iw-1920)':y='clip((ih-1080)/2-(0),0,ih-1080)'"
        );
        // zoom 1.5, shifted a quarter frame right: scale grows, the crop
        // window moves left by 480 px, clamped inside the source.
        let f = cover_vf(1920, 1080, 1.5, [0.25, -0.1]);
        assert!(
            f.starts_with("scale=2880:1620:force_original_aspect_ratio=increase,crop=1920:1080:")
        );
        assert!(f.contains("x='clip((iw-1920)/2-(480),0,iw-1920)'"));
        assert!(f.contains("y='clip((ih-1080)/2-(-108),0,ih-1080)'"));
    }

    #[test]
    fn duration_parses_from_the_ffmpeg_banner() {
        let banner = "Input #0, mov,mp4, from 'bg.mp4':\n  Duration: 00:01:23.45, start: 0.000000, bitrate: 3000 kb/s\n";
        assert!((parse_ffmpeg_duration(banner).unwrap() - 83.45).abs() < 1e-6);
        let hours = "  Duration: 01:02:03.50, start";
        assert!((parse_ffmpeg_duration(hours).unwrap() - 3723.5).abs() < 1e-6);
        assert_eq!(parse_ffmpeg_duration("Duration: N/A, bitrate"), None);
        assert_eq!(parse_ffmpeg_duration("no banner here"), None);
    }
}
