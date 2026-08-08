//! One quality number that means the same thing on every encoder.
//!
//! rhythr used to hand the user's number straight to whichever encoder was
//! selected: `-crf` to libx264, `-cq` to nvenc, `-global_quality` to qsv,
//! `-qp` to vaapi. Those are four incompatible scales (x264's CRF is a rate
//! factor that floats per frame with complexity, nvenc's CQ is a target hint,
//! vaapi's QP is a flat quantiser), so switching encoder silently changed the
//! output while the number on screen stayed put.
//!
//! It also ran the wrong way round. Lower was better, which is correct for
//! CRF and wrong for every expectation a slider creates: people drag it right
//! for "more" and got less. That was the owner's call to invert, and it is
//! the reason this scale exists rather than just a mapping table.
//!
//! So: **0..=100, higher is better**, mapped per encoder here.
//!
//! How exact is the mapping? Honestly: approximate, and it cannot be
//! otherwise. There is no published cross-encoder equivalence: the numbers
//! only line up if you measure VMAF at matched bitrate on your own footage.
//! The hardware offset below is a size-matching rule of thumb, not a
//! measurement. That is why the resolved native value is shown in the UI
//! instead of being hidden: an approximate mapping you can see beats an
//! approximate mapping you cannot.

/// Lowest quality the scale offers.
pub const MIN: u32 = 0;
/// Highest quality the scale offers.
pub const MAX: u32 = 100;

/// Default quality. 70 lands on x264 CRF 20, which is about where the useful
/// range ends for an upload: YouTube re-encodes everything it is given and
/// documents 53-68 Mbit/s for 4K60, while CRF 14 on the same clip produces
/// several times that. Everything below CRF ~18 is render time and disk spent
/// on bits that get thrown away.
pub const DEFAULT: u32 = 70;

/// The user's number as an x264 CRF. Linear, and picked so the ends are the
/// useful ends: 100 → 14 (visually lossless on gameplay, very large), 0 → 34
/// (soft but watchable). The default 70 → 20.
pub fn x264_crf(quality: u32) -> u32 {
    let q = quality.min(MAX);
    34 - (f64::from(q) * 0.2).round() as u32
}

/// The same point on the scale for a hardware encoder's quantiser.
///
/// Hardware encoders at a given quantiser produce a larger file than x264 at
/// the numerically equal CRF, so matching x264's *size* means asking them for
/// a slightly coarser number. Three steps is the commonly used figure and it
/// is a rule of thumb, not a measured equivalence (see the module note).
pub fn hardware_q(quality: u32) -> u32 {
    (x264_crf(quality) + 3).min(51)
}

/// Converts a setting saved before the scale was inverted (where the stored
/// number WAS the x264 CRF) into the new one, so an upgrade does not
/// silently change anybody's output.
pub fn from_legacy_crf(crf: u32) -> u32 {
    // Inverse of x264_crf, clamped to the scale.
    let crf = crf.clamp(14, 34);
    ((34 - crf) * 5).min(MAX)
}

/// Short plain-language description of what a quality value costs, for the
/// hint under the slider. The point is that "100" should not look like a free
/// win: it is the setting that produced a 700 MB clip.
pub fn describe(quality: u32) -> &'static str {
    match quality {
        90..=u32::MAX => "Near-lossless. Very large files.",
        75..=89 => "High. More than an upload needs.",
        60..=74 => "Recommended. Good for YouTube.",
        40..=59 => "Smaller files, still clean.",
        _ => "Draft. Visibly soft.",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn higher_quality_never_means_a_coarser_encode() {
        // The whole point of the scale: it must be monotone, or the slider
        // lies about which direction is better.
        for q in MIN..MAX {
            assert!(
                x264_crf(q) >= x264_crf(q + 1),
                "crf rose from quality {q} to {}",
                q + 1
            );
            assert!(hardware_q(q) >= hardware_q(q + 1));
        }
    }

    #[test]
    fn the_ends_and_the_default_are_where_they_are_documented() {
        assert_eq!(x264_crf(100), 14);
        assert_eq!(x264_crf(DEFAULT), 20);
        assert_eq!(x264_crf(0), 34);
        assert_eq!(hardware_q(DEFAULT), 23);
    }

    #[test]
    fn out_of_range_input_is_clamped_rather_than_wrapping() {
        // 34 - (huge) would underflow a u32 and produce a nonsense CRF.
        assert_eq!(x264_crf(u32::MAX), x264_crf(MAX));
        assert!(hardware_q(u32::MAX) <= 51);
    }

    #[test]
    fn a_saved_setting_survives_the_inversion() {
        // Round-tripping must land on the same encode it did before, for
        // every value the old slider could produce (it ran 14..=30).
        for crf in 14..=30u32 {
            assert_eq!(
                x264_crf(from_legacy_crf(crf)),
                crf,
                "legacy crf {crf} did not survive"
            );
        }
    }

    #[test]
    fn legacy_values_off_the_old_slider_still_land_in_range() {
        assert_eq!(from_legacy_crf(0), MAX);
        assert_eq!(from_legacy_crf(51), MIN);
    }
}
