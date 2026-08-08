//! Finding the fastest way to hand frames to ffmpeg, on THIS machine.
//!
//! The transport is worth up to a third of a render, and which setting wins
//! is not predictable from first principles. It depends on the platform's
//! loopback and pipe implementations, on how much of the frame budget the
//! encoder is already using, and on the frame size — measured on one machine
//! (RTX 4070 SUPER, Ryzen 7 5800X3D) at quality 100, frames per second:
//!
//! ```text
//!              pipe   16 KiB   64 KiB  256 KiB    1 MiB   whole
//!   4K/240      170      200      245      245      245     220
//!   1440p/240   480      440      530      530      530     500
//!   1080p/240   930      710      930      930      900     900
//!   720p/240   1700     1300     1800     1900     1900    1900
//! ```
//!
//! Two conclusions came out of that. 256 KiB over a socket is the best single
//! default, and small writes into a socket are actively harmful — 16 KiB lost
//! to the plain pipe at three of the four sizes. But those are conclusions
//! about one machine, and shipping them as constants means every other
//! machine gets somebody else's answer.
//!
//! So this measures instead. It pushes frames of the real output size through
//! each candidate into an ffmpeg that discards them, and reports what each one
//! managed. What it deliberately does NOT do is include the encoder: the
//! question is which transport moves bytes fastest, and an encoder in the way
//! would just measure the encoder on every candidate equally.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// One way of getting frames across.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Transport {
    /// ffmpeg's stdin. Chunked at 16 KiB, which is what a pipe wants.
    Pipe,
    /// A loopback socket, written in pieces of this many bytes; 0 is the
    /// whole frame in one call.
    Socket(usize),
}

impl Transport {
    pub fn label(&self) -> String {
        match self {
            Transport::Pipe => "pipe".into(),
            Transport::Socket(0) => "socket, whole frame".into(),
            Transport::Socket(n) if n % (1024 * 1024) == 0 => {
                format!("socket, {} MiB", n / (1024 * 1024))
            }
            Transport::Socket(n) if n % 1024 == 0 => format!("socket, {} KiB", n / 1024),
            Transport::Socket(n) => format!("socket, {n} B"),
        }
    }
}

/// What one candidate managed.
#[derive(Debug, Clone)]
pub struct Measured {
    pub transport: Transport,
    /// Frames per second this transport sustained, or None if it failed.
    pub fps: Option<f64>,
}

/// The whole run.
#[derive(Debug, Clone)]
pub struct Benchmark {
    pub width: u32,
    pub height: u32,
    pub results: Vec<Measured>,
    /// The winner, or None when nothing worked and the pipe is all there is.
    pub best: Option<Transport>,
}

impl Benchmark {
    /// One line for a human, naming the winner and what it beat.
    pub fn summary(&self) -> String {
        let Some(best) = self.best else {
            return "no transport could be measured; leaving the setting alone".into();
        };
        let best_fps = self
            .results
            .iter()
            .find(|m| m.transport == best)
            .and_then(|m| m.fps)
            .unwrap_or(0.0);
        let pipe_fps = self
            .results
            .iter()
            .find(|m| m.transport == Transport::Pipe)
            .and_then(|m| m.fps);
        match pipe_fps {
            Some(p) if p > 0.0 && best != Transport::Pipe => format!(
                "{}x{}: {} wins at {:.0} frames/s, against {:.0} on the pipe ({:+.0}%)",
                self.width,
                self.height,
                best.label(),
                best_fps,
                p,
                100.0 * (best_fps / p - 1.0)
            ),
            _ => format!(
                "{}x{}: {} at {:.0} frames/s",
                self.width,
                self.height,
                best.label(),
                best_fps
            ),
        }
    }
}

/// Candidates, in the order they are tried.
///
/// 16 KiB is deliberately absent: it lost to the plain pipe at three of four
/// output sizes on the machine this was measured on, and offering a candidate
/// that can only be chosen by noise is worse than not offering it.
const CANDIDATES: &[Transport] = &[
    Transport::Pipe,
    Transport::Socket(64 * 1024),
    Transport::Socket(256 * 1024),
    Transport::Socket(1024 * 1024),
    Transport::Socket(0),
];

/// How long to spend on each candidate. Long enough to get past the first few
/// frames, short enough that the whole run is a few seconds.
const PER_CANDIDATE: Duration = Duration::from_millis(700);
/// A frame cap that only bites if the clock never advances — set well above
/// what PER_CANDIDATE reaches even on a very fast machine (the owner's manages
/// ~1900 frames/s at 720p, so ~1330 in the window), so TIME is the limit and
/// a fast box is not measured over an unrepresentatively short slice.
const MAX_FRAMES: usize = 20_000;
/// Frames pushed before the clock starts, so ffmpeg is already up and reading
/// on every candidate rather than only on the sockets (which wait for accept).
const WARMUP_FRAMES: usize = 8;

/// Measures every candidate at the given output size and picks a winner.
///
/// `ffmpeg` is the binary to talk to. The frames are synthetic — content is
/// irrelevant to `-c:v copy`, which is what makes this a measurement of the
/// transport rather than of the encoder.
pub fn benchmark(ffmpeg: &str, width: u32, height: u32) -> Benchmark {
    let (w, h) = (width.max(2) & !1, height.max(2) & !1);
    let frame = vec![0x40u8; crate::nv12::nv12_len(w as usize, h as usize)];
    let results: Vec<Measured> = CANDIDATES
        .iter()
        .map(|&t| Measured {
            transport: t,
            fps: measure(ffmpeg, t, w, h, &frame),
        })
        .collect();
    let best = results
        .iter()
        .filter_map(|m| m.fps.map(|f| (m.transport, f)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(t, _)| t);
    Benchmark {
        width: w,
        height: h,
        results,
        best,
    }
}

fn measure(ffmpeg: &str, transport: Transport, w: u32, h: u32, frame: &[u8]) -> Option<f64> {
    let listener = match transport {
        Transport::Socket(_) => Some(std::net::TcpListener::bind(("127.0.0.1", 0)).ok()?),
        Transport::Pipe => None,
    };
    let mut cmd = Command::new(ffmpeg);
    crate::video::hide_console_window(&mut cmd);
    cmd.args(["-hide_banner", "-loglevel", "error"]);
    cmd.args(["-f", "rawvideo", "-pix_fmt", "nv12"]);
    cmd.args(["-s", &format!("{w}x{h}")]);
    cmd.args(["-r", "60"]);
    match &listener {
        Some(l) => {
            let port = l.local_addr().ok()?.port();
            cmd.args(["-i", &format!("tcp://127.0.0.1:{port}")]);
            cmd.stdin(Stdio::null());
        }
        None => {
            cmd.args(["-i", "pipe:0"]);
            cmd.stdin(Stdio::piped());
        }
    }
    // `copy` is the point: nothing is encoded, so what is timed is the
    // transport and ffmpeg's willingness to read from it.
    cmd.args(["-c:v", "copy", "-f", "null", "-"]);
    let mut child = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let outcome = measure_into(&listener, &mut child, transport, frame);

    // ALWAYS kill before reaping, on every path. If the accept loop gave up
    // but ffmpeg then connected to the still-listening backlog, it is blocked
    // reading frames that will never come, and a bare wait() would hang the
    // whole benchmark forever — which is exactly what the sibling probe in
    // video.rs already guards against and this did not.
    let _ = child.kill();
    let _ = child.wait();
    outcome
}

/// The timed part, split out so the caller can guarantee the child is killed
/// however this returns.
fn measure_into(
    listener: &Option<std::net::TcpListener>,
    child: &mut std::process::Child,
    transport: Transport,
    frame: &[u8],
) -> Option<f64> {
    let mut sink: Box<dyn Write> = match listener {
        Some(l) => {
            l.set_nonblocking(true).ok()?;
            let deadline = Instant::now() + Duration::from_secs(5);
            let sock = loop {
                match l.accept() {
                    Ok((s, _)) => break s,
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Give up early if ffmpeg has already exited (bad args,
                        // no such device): waiting out the full deadline for a
                        // process that is gone just wastes five seconds per
                        // socket candidate.
                        if matches!(child.try_wait(), Ok(Some(_))) {
                            return None;
                        }
                        if Instant::now() >= deadline {
                            return None;
                        }
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => return None,
                }
            };
            sock.set_nonblocking(false).ok()?;
            let _ = sock.set_nodelay(true);
            Box::new(sock)
        }
        None => Box::new(child.stdin.take()?),
    };
    let chunk = match transport {
        // A pipe is fastest fed in small pieces; that is not the variable
        // under test here, so it gets the value it is known to want.
        Transport::Pipe => 16 * 1024,
        Transport::Socket(n) => n,
    };
    let write_frame = |sink: &mut Box<dyn Write>| -> std::io::Result<()> {
        if chunk == 0 {
            sink.write_all(frame)
        } else {
            for part in frame.chunks(chunk) {
                sink.write_all(part)?;
            }
            Ok(())
        }
    };
    // Warm up BEFORE starting the clock, for every candidate. A socket only
    // starts timing after accept() — i.e. after ffmpeg is up — while a pipe
    // has no such barrier, so timing a pipe from the first write charged it
    // for ffmpeg's whole startup and biased the winner away from the pipe.
    // A few warm-up frames put both on the same footing: ffmpeg is reading by
    // the time the clock starts.
    for _ in 0..WARMUP_FRAMES {
        write_frame(&mut sink).ok()?;
    }
    let start = Instant::now();
    let mut frames = 0usize;
    while frames < MAX_FRAMES && start.elapsed() < PER_CANDIDATE {
        write_frame(&mut sink).ok()?;
        frames += 1;
    }
    let secs = start.elapsed().as_secs_f64();
    drop(sink);
    if frames == 0 || secs <= 0.0 {
        return None;
    }
    Some(frames as f64 / secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn labels_say_what_was_used() {
        assert_eq!(Transport::Pipe.label(), "pipe");
        assert_eq!(Transport::Socket(0).label(), "socket, whole frame");
        assert_eq!(Transport::Socket(256 * 1024).label(), "socket, 256 KiB");
        assert_eq!(Transport::Socket(1024 * 1024).label(), "socket, 1 MiB");
        assert_eq!(Transport::Socket(1000).label(), "socket, 1000 B");
    }

    /// 16 KiB over a socket lost to the plain pipe at three of four output
    /// sizes. A benchmark that can pick it is a benchmark that can make a
    /// machine slower than doing nothing.
    #[test]
    fn no_candidate_writes_a_socket_in_small_pieces() {
        for c in CANDIDATES {
            if let Transport::Socket(n) = c {
                assert!(
                    *n == 0 || *n >= 64 * 1024,
                    "candidate {} is too small",
                    c.label()
                );
            }
        }
    }

    #[test]
    fn the_summary_names_the_winner_and_what_it_beat() {
        let b = Benchmark {
            width: 3840,
            height: 2160,
            results: vec![
                Measured {
                    transport: Transport::Pipe,
                    fps: Some(170.0),
                },
                Measured {
                    transport: Transport::Socket(256 * 1024),
                    fps: Some(245.0),
                },
            ],
            best: Some(Transport::Socket(256 * 1024)),
        };
        let s = b.summary();
        assert!(s.contains("256 KiB"), "{s}");
        assert!(s.contains("245"), "{s}");
        assert!(s.contains("+44%"), "{s}");
    }

    #[test]
    fn a_run_where_nothing_worked_says_so_instead_of_choosing() {
        let b = Benchmark {
            width: 1920,
            height: 1080,
            results: vec![Measured {
                transport: Transport::Pipe,
                fps: None,
            }],
            best: None,
        };
        assert!(b.summary().contains("leaving the setting alone"));
    }
}
