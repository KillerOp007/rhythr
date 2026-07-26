//! Custom playfield background: a user-chosen image or video drawn behind
//! gameplay instead of the skin's background, with an adjustable dim.
//!
//! Images ride the existing skin background-layer pipeline (a synthetic
//! cover layer). Videos are decoded by the bundled ffmpeg into a rawvideo
//! pipe — one RGBA frame per output frame, looped and muted — and streamed
//! into a persistent texture. The results screen is deliberately untouched.

use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

use crate::config::{BackgroundLayer, SkinConfig};

/// What kind of file the user picked. Detection is by content (magic
/// bytes), not extension — "support as many formats as possible" means an
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
        // Animated GIFs must play — ffmpeg's problem.
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
/// background layers (an image becomes one frame-covering layer, a video
/// flips the streaming flag) and sets the dim. Returns the detected kind.
pub fn apply_background(
    cfg: &mut SkinConfig,
    path: &Path,
    dim: f32,
) -> std::io::Result<BackgroundKind> {
    let kind = classify_file(path)?;
    cfg.background_images.clear();
    cfg.custom_bg_dim = Some(dim.clamp(0.0, 1.0));
    match kind {
        BackgroundKind::Image => {
            cfg.background_images.push(cover_layer(std::fs::read(path)?));
            cfg.custom_bg_video = false;
        }
        BackgroundKind::Video => cfg.custom_bg_video = true,
    }
    Ok(kind)
}

/// A screen-space layer that covers the whole frame (centre-cropped).
fn cover_layer(bytes: Vec<u8>) -> BackgroundLayer {
    BackgroundLayer {
        bytes,
        fit: 2,
        placement: 0,
        center_x: 0.5,
        center_y: 0.5,
        scale_x: 1.0,
        scale_y: 1.0,
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

/// Full decode check so a corrupt image errors when the user picks it,
/// not as a silently black render.
pub fn image_decodes(bytes: &[u8]) -> bool {
    image::load_from_memory(bytes).is_ok()
}

/// Scale-to-cover filter: fill the frame, centre-crop the overflow.
fn cover_vf(w: u32, h: u32) -> String {
    format!("scale={w}:{h}:force_original_aspect_ratio=increase,crop={w}:{h}")
}

/// A muted, endlessly looped ffmpeg decode of the background video,
/// delivering exactly one frame-sized RGBA frame per output frame.
pub struct VideoDecoder {
    child: Child,
    stdout: ChildStdout,
    frame: Vec<u8>,
    got_any: bool,
    done: bool,
}

impl VideoDecoder {
    pub fn spawn(
        ffmpeg: &str,
        path: &Path,
        width: u32,
        height: u32,
        fps: u32,
    ) -> Result<VideoDecoder, String> {
        let mut cmd = Command::new(ffmpeg);
        crate::video::hide_console_window(&mut cmd);
        cmd.args(["-loglevel", "error", "-stream_loop", "-1", "-an", "-i"])
            .arg(path)
            .args(["-f", "rawvideo", "-pix_fmt", "rgba", "-vf", &cover_vf(width, height)])
            .args(["-r", &fps.to_string(), "pipe:1"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("could not start ffmpeg ({ffmpeg}): {e}"))?;
        let stdout = child.stdout.take().ok_or("ffmpeg gave no stdout")?;
        Ok(VideoDecoder {
            child,
            stdout,
            frame: vec![0; (width * height * 4) as usize],
            got_any: false,
            done: false,
        })
    }

    /// The frame for the next output frame. After the stream ends or
    /// errors, keeps returning the last good frame (or None if decoding
    /// never produced one — the caller then leaves the background black).
    pub fn next_frame(&mut self) -> Option<&[u8]> {
        use std::io::Read;
        if !self.done {
            match self.stdout.read_exact(&mut self.frame) {
                Ok(()) => self.got_any = true,
                Err(_) => self.done = true,
            }
        }
        self.got_any.then_some(self.frame.as_slice())
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The video's duration via ffmpeg's `-i` banner (no ffprobe shipped).
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

/// One RGBA frame at `t_secs` (wrapped over `duration` so the preview
/// loops like the render will) for the live preview.
pub fn extract_frame(
    ffmpeg: &str,
    path: &Path,
    t_secs: f64,
    duration: Option<f64>,
    width: u32,
    height: u32,
) -> Option<Vec<u8>> {
    use std::io::Read;
    let t = match duration {
        Some(d) if d > 0.05 => t_secs.max(0.0) % d,
        _ => t_secs.max(0.0),
    };
    let mut cmd = Command::new(ffmpeg);
    crate::video::hide_console_window(&mut cmd);
    let mut child = cmd
        .args(["-loglevel", "error", "-ss", &format!("{t:.3}"), "-an", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "rawvideo", "-pix_fmt", "rgba"])
        .args(["-vf", &cover_vf(width, height), "pipe:1"])
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
    use super::*;

    #[test]
    fn images_are_recognised_by_content_not_extension() {
        // PNG / JPEG / WebP / BMP magic bytes → image.
        assert_eq!(classify_bytes(b"\x89PNG\r\n\x1a\n........"), BackgroundKind::Image);
        assert_eq!(classify_bytes(b"\xff\xd8\xff\xe0....JFIF"), BackgroundKind::Image);
        assert_eq!(classify_bytes(b"RIFF\x00\x00\x00\x00WEBPVP8 "), BackgroundKind::Image);
        assert_eq!(classify_bytes(b"BM\x00\x00\x00\x00\x00\x00\x00\x00"), BackgroundKind::Image);
    }

    #[test]
    fn gifs_and_unknown_containers_go_to_ffmpeg() {
        // Animated GIFs must PLAY, so they are video; mp4/webm/mkv are
        // never image formats; garbage falls through to ffmpeg, which
        // will produce the real error message.
        assert_eq!(classify_bytes(b"GIF89a\x00\x00\x00\x00"), BackgroundKind::Video);
        assert_eq!(classify_bytes(b"\x00\x00\x00\x20ftypisom...."), BackgroundKind::Video);
        assert_eq!(classify_bytes(b"\x1a\x45\xdf\xa3............"), BackgroundKind::Video);
        assert_eq!(classify_bytes(b"not a media file at all....."), BackgroundKind::Video);
    }

    #[test]
    fn applying_a_background_replaces_skin_layers_and_sets_the_dim() {
        let dir = std::env::temp_dir().join("rhythr-bg-test");
        std::fs::create_dir_all(&dir).unwrap();
        let img = dir.join("bg.png");
        std::fs::write(&img, b"\x89PNG\r\n\x1a\n....").unwrap();

        let mut cfg = SkinConfig::default();
        cfg.background_images.push(cover_layer(vec![1]));
        let kind = apply_background(&mut cfg, &img, 0.6).unwrap();
        assert_eq!(kind, BackgroundKind::Image);
        assert_eq!(cfg.background_images.len(), 1);
        assert!(!cfg.custom_bg_video);
        assert_eq!(cfg.custom_bg_dim, Some(0.6));

        let vid = dir.join("bg.mp4");
        std::fs::write(&vid, b"\x00\x00\x00\x20ftypisom").unwrap();
        let kind = apply_background(&mut cfg, &vid, 1.5).unwrap();
        assert_eq!(kind, BackgroundKind::Video);
        assert!(cfg.background_images.is_empty());
        assert!(cfg.custom_bg_video);
        assert_eq!(cfg.custom_bg_dim, Some(1.0));
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
