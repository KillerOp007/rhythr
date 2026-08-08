//! Finding the fastest way to hand frames to ffmpeg, on THIS machine.
//!
//! The transport is worth up to a third of a render, and which setting wins
//! is not predictable from first principles. It depends on the platform's
//! loopback and pipe implementations, on how much of the frame budget the
//! encoder is already using, and on the frame size. Measured on one machine
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
//! default, and small writes into a socket are actively harmful: 16 KiB lost
//! to the plain pipe at three of the four sizes. But those are conclusions
//! about one machine, and shipping them as constants means every other
//! machine gets somebody else's answer.
//!
//! So this measures instead. It pushes frames of the real output size through
//! each candidate into an ffmpeg that discards them, and reports what each one
//! managed. What it deliberately does NOT do is include the encoder: the
//! question is which transport moves bytes fastest, and an encoder in the way
//! would just measure the encoder on every candidate equally.
//!
//! Which is also the trap. The number that comes out is what the transport
//! can carry with nothing rendered and nothing encoded, so it is far above
//! any real render, and on a fast machine the socket sizes all land on top of
//! each other well above what the machine can render anyway. Two consecutive
//! runs on the owner's box reported 64 KiB at 407 frames/s and 1 MiB at 422,
//! a 4% spread, and moved the setting each time while the actual render sat
//! at 200 either way. So: every candidate is measured [`ROUNDS`] times with
//! the rounds interleaved, the median is taken, and anything inside
//! [`NOISE_BAND`] of the fastest counts as tied and resolves to
//! [`PREFERRED`]. A setting that persists across restarts must not be decided
//! by a coin toss.

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
    /// Frames per second this transport sustained (the median of the rounds),
    /// or None if it failed in every one of them.
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
    /// Every candidate that finished within [`NOISE_BAND`] of the fastest,
    /// including the fastest itself. More than one entry here means the
    /// measurement could not tell them apart, which is the normal outcome on
    /// a fast machine and the reason the winner is not simply the maximum.
    pub tied: Vec<Transport>,
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
        // Said out loud when the field could not be separated, because the
        // alternative is a button that answers differently every time it is
        // pressed and no way to tell that from a real change.
        let tie = if self.tied.len() > 1 {
            let others: Vec<String> = self
                .tied
                .iter()
                .filter(|t| **t != best)
                .map(|t| t.label())
                .collect();
            format!(
                " (too close to call against {}, so the safe default was kept)",
                others.join(" and ")
            )
        } else {
            String::new()
        };
        // "moves ... frames/s" rather than "wins at ... frames/s": this
        // number is the transport carrying frames into an ffmpeg that throws
        // them away, with nothing rendered and nothing encoded. A real render
        // is slower, often much slower, and reading it as a promise of render
        // speed is the obvious mistake to make.
        match pipe_fps {
            Some(p) if p > 0.0 && best != Transport::Pipe => format!(
                "{}x{}: {} moves {:.0} frames/s, against {:.0} on the pipe ({:+.0}%){}. \
                 Transport only, so a real render is slower.",
                self.width,
                self.height,
                best.label(),
                best_fps,
                p,
                100.0 * (best_fps / p - 1.0),
                tie
            ),
            _ => format!(
                "{}x{}: {} moves {:.0} frames/s{}. Transport only, so a real render is slower.",
                self.width,
                self.height,
                best.label(),
                best_fps,
                tie
            ),
        }
    }

    /// Whether the transport is even the thing worth tuning here.
    ///
    /// A render only ever goes as fast as its slowest stage, so once the
    /// transport can carry twice what the machine renders, every candidate
    /// is above the ceiling and the differences between them stop reaching
    /// the output at all. Saying so is more useful than another number.
    pub fn headroom_note(&self, render_fps: f64) -> Option<String> {
        let best_fps = self
            .best
            .and_then(|b| self.results.iter().find(|m| m.transport == b))
            .and_then(|m| m.fps)?;
        if render_fps <= 0.0 || best_fps < render_fps * 2.0 {
            return None;
        }
        Some(format!(
            "Your last render ran at {render_fps:.0} frames/s, and the transport can carry \
             {best_fps:.0}, so it is not what is holding the render back: the GPU and the \
             encoder are. Changing this setting will not make renders faster."
        ))
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

/// How long to spend on each candidate in each round.
const PER_CANDIDATE: Duration = Duration::from_millis(500);
/// How often the whole candidate list is walked. Rounds are interleaved
/// (A B C, A B C, A B C) rather than repeated per candidate, so a machine
/// that gets busier or hotter part way through spreads that across all of
/// them instead of punishing whoever went last.
const ROUNDS: usize = 3;
/// How close two candidates have to be before this refuses to call a winner.
///
/// On a fast machine the socket sizes land on top of each other: the owner's
/// box measured 64 KiB at 407 frames/s and 1 MiB at 422 in two consecutive
/// runs, a 4% spread, and picked a different "winner" each time. Anything
/// inside this band is noise, and the setting must not move for noise.
const NOISE_BAND: f64 = 0.06;
/// What a tie resolves to. 256 KiB is the value that was at or near the top
/// at every output size measured, which makes it the answer least likely to
/// be wrong on a machine or a resolution nobody has measured.
const PREFERRED: Transport = Transport::Socket(256 * 1024);
/// A frame cap that only bites if the clock never advances. It is set well
/// above what PER_CANDIDATE reaches even on a very fast machine (the owner's
/// manages ~1900 frames/s at 720p, so ~1330 in the window), so TIME is the
/// limit and a fast box is not measured over an unrepresentatively short
/// slice.
const MAX_FRAMES: usize = 20_000;
/// Frames pushed before the clock starts, so ffmpeg is already up and reading
/// on every candidate rather than only on the sockets (which wait for accept).
const WARMUP_FRAMES: usize = 8;

/// Measures every candidate at the given output size and picks a winner.
///
/// `ffmpeg` is the binary to talk to. The frames are synthetic: content is
/// irrelevant to `-c:v copy`, which is what makes this a measurement of the
/// transport rather than of the encoder.
pub fn benchmark(ffmpeg: &str, width: u32, height: u32) -> Benchmark {
    let (w, h) = (width.max(2) & !1, height.max(2) & !1);
    let frame = vec![0x40u8; crate::nv12::nv12_len(w as usize, h as usize)];
    let mut samples: Vec<Vec<f64>> = vec![Vec::new(); CANDIDATES.len()];
    for _ in 0..ROUNDS {
        for (i, &t) in CANDIDATES.iter().enumerate() {
            if let Some(fps) = measure(ffmpeg, t, w, h, &frame) {
                samples[i].push(fps);
            }
        }
    }
    let results: Vec<Measured> = CANDIDATES
        .iter()
        .zip(samples)
        .map(|(&transport, mut runs)| {
            runs.sort_by(f64::total_cmp);
            Measured {
                transport,
                // Median, not mean and not best: one round that landed while
                // something else on the machine woke up should not decide a
                // setting that then persists across restarts.
                fps: (!runs.is_empty()).then(|| runs[runs.len() / 2]),
            }
        })
        .collect();
    let (best, tied) = pick_winner(&results);
    Benchmark {
        width: w,
        height: h,
        results,
        best,
        tied,
    }
}

/// Turns measurements into a decision.
///
/// Pure, so the rule can be tested against the numbers that produced the
/// problem rather than by pressing a button and hoping.
fn pick_winner(results: &[Measured]) -> (Option<Transport>, Vec<Transport>) {
    let Some(top) = results.iter().filter_map(|m| m.fps).max_by(f64::total_cmp) else {
        return (None, Vec::new());
    };
    let tied: Vec<Transport> = results
        .iter()
        .filter(|m| m.fps.is_some_and(|f| f >= top * (1.0 - NOISE_BAND)))
        .map(|m| m.transport)
        .collect();
    // Inside the band nothing was actually measured to be better, so the
    // stable answer wins over the nominal one. Outside it, the measurement
    // means something and is followed.
    let winner = if tied.contains(&PREFERRED) {
        Some(PREFERRED)
    } else {
        results
            .iter()
            .filter_map(|m| m.fps.map(|f| (m.transport, f)))
            .max_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(t, _)| t)
    };
    (winner, tied)
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
    // whole benchmark forever, which is exactly what the sibling probe in
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
    // starts timing after accept() (i.e. after ffmpeg is up), while a pipe
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
            tied: vec![Transport::Socket(256 * 1024)],
        };
        let s = b.summary();
        assert!(s.contains("256 KiB"), "{s}");
        assert!(s.contains("245"), "{s}");
        assert!(s.contains("+44%"), "{s}");
        assert!(
            s.contains("Transport only"),
            "the number must not read as render speed: {s}"
        );
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
            tied: Vec::new(),
        };
        assert!(b.summary().contains("leaving the setting alone"));
    }

    fn measured(pairs: &[(Transport, f64)]) -> Vec<Measured> {
        pairs
            .iter()
            .map(|(t, f)| Measured {
                transport: *t,
                fps: Some(*f),
            })
            .collect()
    }

    /// The bug this rule exists for, with the numbers that produced it: two
    /// consecutive runs on the same machine reported 64 KiB at 407 and 1 MiB
    /// at 422, a 4% spread, and the setting moved each time. Neither run
    /// measured anything real, so neither may move it.
    #[test]
    fn a_four_percent_spread_does_not_move_the_setting() {
        let first = measured(&[
            (Transport::Pipe, 176.0),
            (Transport::Socket(64 * 1024), 407.0),
            (Transport::Socket(256 * 1024), 399.0),
            (Transport::Socket(1024 * 1024), 396.0),
            (Transport::Socket(0), 380.0),
        ]);
        let second = measured(&[
            (Transport::Pipe, 179.0),
            (Transport::Socket(64 * 1024), 402.0),
            (Transport::Socket(256 * 1024), 405.0),
            (Transport::Socket(1024 * 1024), 422.0),
            (Transport::Socket(0), 390.0),
        ]);
        let (a, tied_a) = pick_winner(&first);
        let (b, _) = pick_winner(&second);
        assert_eq!(a, Some(PREFERRED), "run one moved off the safe default");
        assert_eq!(b, Some(PREFERRED), "run two moved off the safe default");
        assert_eq!(a, b, "two runs of the same machine disagreed");
        assert!(tied_a.len() > 1, "the tie has to be visible to the summary");
    }

    /// The rule must not become "always 256 KiB". A candidate that is really
    /// faster, by more than the measurement can be wrong by, still wins.
    #[test]
    fn a_real_difference_still_wins() {
        let results = measured(&[
            (Transport::Pipe, 900.0),
            (Transport::Socket(64 * 1024), 700.0),
            (Transport::Socket(256 * 1024), 710.0),
            (Transport::Socket(1024 * 1024), 705.0),
            (Transport::Socket(0), 690.0),
        ]);
        let (best, tied) = pick_winner(&results);
        assert_eq!(best, Some(Transport::Pipe), "the pipe was 27% faster");
        assert_eq!(tied, vec![Transport::Pipe]);
    }

    /// The transport stops mattering once it can carry more than the machine
    /// renders, and pressing the button again cannot change that.
    #[test]
    fn a_transport_with_headroom_says_it_is_not_the_bottleneck() {
        let b = Benchmark {
            width: 3840,
            height: 2160,
            results: measured(&[(Transport::Socket(256 * 1024), 422.0)]),
            best: Some(Transport::Socket(256 * 1024)),
            tied: vec![Transport::Socket(256 * 1024)],
        };
        let note = b.headroom_note(200.0).expect("422 is more than twice 200");
        assert!(
            note.contains("not what is holding the render back"),
            "{note}"
        );
        // With the transport close to the render speed it IS worth tuning.
        assert!(b.headroom_note(390.0).is_none());
        assert!(b.headroom_note(0.0).is_none(), "no render yet, no claim");
    }
}
