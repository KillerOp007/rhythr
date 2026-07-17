//! Hit/miss-sound mixing: builds a PCM track with the game's hit sound at
//! every registered hit (and optionally the miss sound, which the game only
//! plays when a combo of at least [`MISS_SOUND_COMBO_THRESHOLD`] breaks),
//! written as a WAV for ffmpeg to mix under the music.

use rhythia_sim::hitreg::NoteResult;

/// The game's `MissSoundComboThreshold` default: a miss is only audible
/// when it breaks a combo of at least this many hits.
pub const MISS_SOUND_COMBO_THRESHOLD: u32 = 5;

const RATE: u32 = 44_100;
const CHANNELS: usize = 2;

/// Decoded PCM: interleaved stereo f32 at 44.1 kHz.
pub struct Clip {
    samples: Vec<f32>,
}

impl Clip {
    /// Parses a RIFF/WAVE file (PCM16, mono or stereo). Sample rates other
    /// than 44.1 kHz are resampled naively (nearest) — the game's own clips
    /// are all 44.1 kHz, so this is only a safety net for custom files.
    pub fn from_wav(data: &[u8]) -> Option<Clip> {
        if data.len() < 44 || &data[0..4] != b"RIFF" || &data[8..12] != b"WAVE" {
            return None;
        }
        let mut pos = 12usize;
        let mut fmt: Option<(u16, u16, u32, u16)> = None; // format, channels, rate, bits
        let mut pcm: Option<&[u8]> = None;
        while pos + 8 <= data.len() {
            let id = &data[pos..pos + 4];
            let size = u32::from_le_bytes(data[pos + 4..pos + 8].try_into().ok()?) as usize;
            let body = data.get(pos + 8..pos + 8 + size)?;
            match id {
                b"fmt " if size >= 16 => {
                    fmt = Some((
                        u16::from_le_bytes(body[0..2].try_into().ok()?),
                        u16::from_le_bytes(body[2..4].try_into().ok()?),
                        u32::from_le_bytes(body[4..8].try_into().ok()?),
                        u16::from_le_bytes(body[14..16].try_into().ok()?),
                    ));
                }
                b"data" => pcm = Some(body),
                _ => {}
            }
            pos += 8 + size + (size & 1);
        }
        let (format, channels, rate, bits) = fmt?;
        let pcm = pcm?;
        if format != 1 || bits != 16 || channels == 0 || channels > 2 || rate == 0 {
            return None;
        }
        let frames: Vec<[f32; 2]> = pcm
            .chunks_exact(2 * channels as usize)
            .map(|c| {
                let l = i16::from_le_bytes([c[0], c[1]]) as f32 / 32768.0;
                let r = if channels == 2 {
                    i16::from_le_bytes([c[2], c[3]]) as f32 / 32768.0
                } else {
                    l
                };
                [l, r]
            })
            .collect();
        // Nearest-sample rate conversion to 44.1 kHz.
        let out_frames = (frames.len() as u64 * RATE as u64 / rate as u64) as usize;
        let mut samples = Vec::with_capacity(out_frames * CHANNELS);
        for i in 0..out_frames {
            let src = (i as u64 * rate as u64 / RATE as u64) as usize;
            let f = frames.get(src).copied().unwrap_or([0.0, 0.0]);
            samples.push(f[0]);
            samples.push(f[1]);
        }
        Some(Clip { samples })
    }
}

/// Mixes hit sounds (and optionally miss sounds) for the clip window
/// `[start_ms, end_ms]` (song time) into a WAV byte buffer. `speed` is the
/// replay's speed mod: the track is laid out in wall-clock time, so a hit
/// at song time t sounds at (t - start)/speed. `results` comes from the
/// resolved hit registration; miss times use each note's own time. Returns
/// None when nothing would sound.
#[allow(clippy::too_many_arguments)]
pub fn build_hitsound_wav(
    hit: &Clip,
    miss: Option<&Clip>,
    results: &[NoteResult],
    note_times: &[f64],
    start_ms: f64,
    end_ms: f64,
    speed: f64,
    volume: f32,
) -> Option<Vec<u8>> {
    let span_ms = end_ms - start_ms;
    if span_ms <= 0.0 || volume <= 0.0 || speed <= 0.0 {
        return None;
    }
    let frames = (span_ms / speed / 1000.0 * RATE as f64).ceil() as usize;
    let mut mix = vec![0.0f32; frames * CHANNELS];
    let mut add = |at_ms: f64, clip: &Clip| -> bool {
        let rel = at_ms - start_ms;
        if rel < 0.0 || rel >= span_ms {
            return false;
        }
        let offset = (rel / speed / 1000.0 * RATE as f64) as usize * CHANNELS;
        for (i, s) in clip.samples.iter().enumerate() {
            if let Some(slot) = mix.get_mut(offset + i) {
                *slot += s * volume;
            }
        }
        true
    };

    let mut any = false;
    let mut streak: u32 = 0;
    for r in results {
        if r.hit {
            streak += 1;
            if let Some(t) = r.hit_ms {
                any |= add(t, hit);
            }
        } else {
            // The game gates the miss sound behind a combo threshold.
            if streak >= MISS_SOUND_COMBO_THRESHOLD {
                if let (Some(m), Some(&t)) = (miss, note_times.get(r.note_index)) {
                    any |= add(t, m);
                }
            }
            streak = 0;
        }
    }
    if !any {
        return None;
    }

    // Soft-clip into i16 range: simultaneous hits can stack above 1.0.
    let mut out = Vec::with_capacity(44 + mix.len() * 2);
    let data_len = (mix.len() * 2) as u32;
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&(CHANNELS as u16).to_le_bytes());
    out.extend_from_slice(&RATE.to_le_bytes());
    out.extend_from_slice(&(RATE * CHANNELS as u32 * 2).to_le_bytes());
    out.extend_from_slice(&((CHANNELS * 2) as u16).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in mix {
        let clipped = (s.tanh() * 32767.0) as i16;
        out.extend_from_slice(&clipped.to_le_bytes());
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tiny_wav(rate: u32, channels: u16, frames: &[i16]) -> Vec<u8> {
        let data_len = (frames.len() * 2) as u32;
        let mut w = Vec::new();
        w.extend_from_slice(b"RIFF");
        w.extend_from_slice(&(36 + data_len).to_le_bytes());
        w.extend_from_slice(b"WAVEfmt ");
        w.extend_from_slice(&16u32.to_le_bytes());
        w.extend_from_slice(&1u16.to_le_bytes());
        w.extend_from_slice(&channels.to_le_bytes());
        w.extend_from_slice(&rate.to_le_bytes());
        w.extend_from_slice(&(rate * channels as u32 * 2).to_le_bytes());
        w.extend_from_slice(&(channels * 2).to_le_bytes());
        w.extend_from_slice(&16u16.to_le_bytes());
        w.extend_from_slice(b"data");
        w.extend_from_slice(&data_len.to_le_bytes());
        for f in frames {
            w.extend_from_slice(&f.to_le_bytes());
        }
        w
    }

    #[test]
    fn wav_roundtrip_and_mix() {
        let clip = Clip::from_wav(&tiny_wav(44_100, 1, &[16384, 16384])).expect("parses");
        assert_eq!(clip.samples.len(), 4); // mono → stereo

        let results = vec![
            NoteResult { note_index: 0, hit: true, hit_ms: Some(100.0) },
            NoteResult { note_index: 1, hit: false, hit_ms: None },
        ];
        let wav = build_hitsound_wav(&clip, None, &results, &[100.0, 200.0], 0.0, 1000.0, 1.0, 1.0)
            .expect("has sound");
        assert_eq!(&wav[0..4], b"RIFF");
        // Hit at 100 ms → non-zero samples at frame 4410.
        let off = 44 + 4410 * 2 * 2;
        let s = i16::from_le_bytes([wav[off], wav[off + 1]]);
        assert!(s.abs() > 8000, "expected audible hit, got {s}");
    }

    #[test]
    fn miss_sound_respects_combo_threshold() {
        let clip = Clip::from_wav(&tiny_wav(44_100, 1, &[16384])).unwrap();
        // 3-hit streak then a miss: below the threshold of 5 → silence.
        let mut results: Vec<NoteResult> = (0..3)
            .map(|i| NoteResult { note_index: i, hit: true, hit_ms: Some(10.0 + i as f64) })
            .collect();
        results.push(NoteResult { note_index: 3, hit: false, hit_ms: None });
        let times: Vec<f64> = (0..4).map(|i| 10.0 + i as f64).collect();
        let wav = build_hitsound_wav(&clip, Some(&clip), &results, &times, 500.0, 1000.0, 1.0, 1.0);
        assert!(wav.is_none(), "no hits in window and miss below threshold");

        // Speed mod: a hit at song time 400 lands at wall-clock 200 ms.
        let results = vec![NoteResult { note_index: 0, hit: true, hit_ms: Some(400.0) }];
        let wav = build_hitsound_wav(&clip, None, &results, &[400.0], 0.0, 1000.0, 2.0, 1.0)
            .expect("has sound");
        let frame = |ms: f64| 44 + (ms / 1000.0 * 44_100.0) as usize * 2 * 2;
        let s_at = |off: usize| i16::from_le_bytes([wav[off], wav[off + 1]]);
        assert!(s_at(frame(200.0)).abs() > 8000, "hit at half the song time");
        assert_eq!(s_at(frame(400.0)), 0, "nothing at the unscaled position");
    }
}
