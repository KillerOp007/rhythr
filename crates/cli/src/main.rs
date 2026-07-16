//! Command-line interface: replay inspection, integrity checks, and
//! frame/video rendering.

mod manifest;

use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_sim::integrity;

#[derive(Parser)]
#[command(
    name = "rhythia-render",
    version,
    about = "Rhythia replay renderer (read-only)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Print a replay's header and frame statistics.
    Info {
        replay: PathBuf,
        /// Emit machine-readable JSON instead of text.
        #[arg(long)]
        json: bool,
    },
    /// Run the integrity check for one replay against its map.
    Verify {
        replay: PathBuf,
        /// Map file: .rhm or the game's cache .json.
        #[arg(long)]
        map: PathBuf,
    },
    /// Validate every replay in a test-data folder against its manifest
    /// (testdata_manifest.json) and run the integrity check on each.
    Check {
        /// Folder containing testdata_manifest.json plus the files it lists.
        testdata: PathBuf,
    },
    /// Render a single still frame of a replay to a PNG.
    Frame {
        replay: PathBuf,
        /// Map file: .rhm or the game's cache .json.
        #[arg(long)]
        map: PathBuf,
        /// Song time to render, as milliseconds or mm:ss(.ms).
        #[arg(long)]
        at: String,
        /// Output PNG path.
        #[arg(long, short)]
        out: PathBuf,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        /// The player's config.json or exported .rhs skin (adopts their
        /// note skin, camera, colours). Defaults applied when omitted.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Directory with the game's built-in assets (builtin_colorsets.json
        /// + notes/borders/cursors) to resolve built-in skin references.
        #[arg(long)]
        game_assets: Option<PathBuf>,
    },
    /// Render a replay to an MP4 video (frames → ffmpeg + audio).
    Video {
        replay: PathBuf,
        /// Map file: .rhm or the game's cache .json.
        #[arg(long)]
        map: PathBuf,
        /// Output MP4 path.
        #[arg(long, short)]
        out: PathBuf,
        /// Audio track to mux (ms/mp3); omit for silent or when the .rhm
        /// carries embedded audio.
        #[arg(long)]
        audio: Option<PathBuf>,
        /// Clip start (ms or mm:ss); default 0.
        #[arg(long)]
        start: Option<String>,
        /// Clip end (ms or mm:ss); default the replay's end (or fail time).
        #[arg(long)]
        end: Option<String>,
        #[arg(long, default_value_t = 60)]
        fps: u32,
        #[arg(long, default_value_t = 1920)]
        width: u32,
        #[arg(long, default_value_t = 1080)]
        height: u32,
        #[arg(long, default_value_t = 18)]
        crf: u32,
        /// Second replay of the same map, rendered as a ghost overlay
        /// (cursor + trail in orange, with a versus panel).
        #[arg(long)]
        ghost_replay: Option<PathBuf>,
        /// Motion blur strength: 0 = off, 1 = light, 2 = strong (tmix).
        #[arg(long, default_value_t = 0)]
        motion_blur: u32,
        /// Music volume in percent (0-150).
        #[arg(long, default_value_t = 100)]
        music_volume: u32,
        /// Hit/miss-sound volume in percent (0 = off); needs --game-assets
        /// for the game's extracted sound files.
        #[arg(long, default_value_t = 0)]
        hitsound_volume: u32,
        /// Seconds of results screen appended when the clip reaches the end
        /// of the run (0 disables).
        #[arg(long, default_value_t = 4.0)]
        results_secs: f64,
        /// x264 speed preset (ultrafast..placebo).
        #[arg(long, default_value = "veryfast")]
        preset: String,
        /// Video encoder: auto probes the VAAPI hardware encoder and falls
        /// back to software x264; force one with x264/vaapi.
        #[arg(long, default_value = "auto")]
        encoder: String,
        /// ffmpeg executable (path or bare name on PATH).
        #[arg(long, default_value = "ffmpeg")]
        ffmpeg: String,
        /// The player's config.json or exported .rhs skin.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Directory with the game's built-in assets to resolve built-in
        /// skin references (see `frame --game-assets`).
        #[arg(long)]
        game_assets: Option<PathBuf>,
    },
}

fn load_config(
    path: &Option<PathBuf>,
    game_assets: &Option<PathBuf>,
) -> anyhow::Result<rhythia_render::SkinConfig> {
    let mut cfg = match path {
        Some(p) => rhythia_render::SkinConfig::from_path(p)
            .with_context(|| format!("reading skin config {}", p.display()))?,
        None => rhythia_render::SkinConfig::default(),
    };
    // Resolve built-in colorset/textures the config references by name from
    // the game's assets (the player's install / an extracted copy).
    if let Some(dir) = game_assets {
        cfg.resolve_builtins(&rhythia_render::BuiltinAssets::load(dir));
    }
    Ok(cfg)
}

fn parse_time_ms(text: &str) -> anyhow::Result<f64> {
    if let Some((m, s)) = text.split_once(':') {
        let m: f64 = m.trim().parse()?;
        let s: f64 = s.trim().parse()?;
        Ok((m * 60.0 + s) * 1000.0)
    } else {
        Ok(text.trim().parse()?)
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(ok) => {
            if ok {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Err(err) => {
            eprintln!("error: {err:#}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<bool> {
    match Cli::parse().command {
        Command::Info { replay, json } => {
            let r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            if json {
                print_info_json(&r);
            } else {
                print_info(&r);
            }
            Ok(true)
        }
        Command::Verify { replay, map } => {
            let r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            let m = Map::from_path(&map).with_context(|| format!("reading {}", map.display()))?;
            let report = integrity::verify_replay(&r, &m);
            print_report(&report);
            Ok(report.consistent())
        }
        Command::Check { testdata } => manifest::check_folder(&testdata),
        Command::Frame {
            replay,
            map,
            at,
            out,
            width,
            height,
            config,
            game_assets,
        } => {
            let song_ms = parse_time_ms(&at).context("parsing --at")?;
            let r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            let m = Map::from_path(&map).with_context(|| format!("reading {}", map.display()))?;
            let cfg = load_config(&config, &game_assets)?;

            // Surface tampering before spending time rendering.
            let report = integrity::verify_replay(&r, &m);
            if !report.consistent() {
                eprintln!("warning: replay data is inconsistent — possibly manipulated");
            }

            let mut params = rhythia_render::scene::SceneParams::from(&cfg);
            let renderer = rhythia_render::Renderer::new(width, height, cfg.hud_font.as_deref())
                .context("initialising GPU renderer")?;
            let skin = renderer.prepare_skin(&cfg);
            // Show the field the player actually saw (mirror/hardrock).
            let (m, mods) = rhythia_render::mods::map_for_replay(&m, &r);
            params.grid_scale = mods.grid_scale;
            let hud_state = rhythia_render::hud::HudState::new(&m, &r);
            let pixels = renderer
                .render_still(&params, &cfg, &skin, &r, &m, song_ms, Some(&hud_state))
                .context("rendering frame")?;
            rhythia_render::write_png(&out, &pixels, width, height)
                .with_context(|| format!("writing {}", out.display()))?;
            println!(
                "rendered {}x{} at {:.0} ms -> {}",
                width,
                height,
                song_ms,
                out.display()
            );
            Ok(true)
        }
        Command::Video {
            replay,
            map,
            out,
            audio,
            start,
            end,
            fps,
            width,
            height,
            crf,
            ghost_replay,
            motion_blur,
            music_volume,
            hitsound_volume,
            results_secs,
            preset,
            encoder,
            ffmpeg,
            config,
            game_assets,
        } => {
            let r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            let mut m =
                Map::from_path(&map).with_context(|| format!("reading {}", map.display()))?;
            // Cache-JSON maps don't embed their cover; look for a sibling
            // "…cover.png" sharing the map's name prefix (results screen bg).
            if m.cover.is_none() {
                let name = map.file_stem().unwrap_or_default().to_string_lossy();
                let prefix = name.trim_end_matches("map_json").trim_end_matches('_');
                let candidate = map.with_file_name(if prefix.is_empty() {
                    "cover.png".to_string()
                } else {
                    format!("{prefix}_cover.png")
                });
                if let Ok(bytes) = std::fs::read(&candidate) {
                    m.cover = Some(bytes);
                }
            }
            let cfg = load_config(&config, &game_assets)?;

            let report = integrity::verify_replay(&r, &m);
            if !report.consistent() {
                eprintln!("warning: replay data is inconsistent — possibly manipulated");
            }

            let start_ms = match &start {
                Some(s) => parse_time_ms(s).context("parsing --start")?,
                None => 0.0,
            };
            let end_ms = match &end {
                Some(s) => parse_time_ms(s).context("parsing --end")?,
                None if r.failed() => f64::from(r.fail_time_ms),
                None => r.length_ms(),
            };
            if end_ms <= start_ms {
                anyhow::bail!("end ({end_ms} ms) must be after start ({start_ms} ms)");
            }

            // Audio: explicit flag wins; otherwise use the .rhm's embedded
            // track if present. A temp file backs the embedded bytes.
            let mut _audio_tmp: Option<tempfile::NamedTempFile> = None;
            let audio_path = if let Some(a) = audio {
                Some(a)
            } else if let Some(bytes) = &m.audio {
                let mut tmp = tempfile::Builder::new()
                    .prefix("rhythia-audio-")
                    .suffix(".mp3")
                    .tempfile()
                    .context("creating audio temp file")?;
                std::io::Write::write_all(&mut tmp, bytes).context("writing audio temp file")?;
                let path = tmp.path().to_path_buf();
                _audio_tmp = Some(tmp);
                Some(path)
            } else {
                eprintln!("note: no audio (cache-JSON map has none; pass --audio for sound)");
                None
            };

            let params = rhythia_render::scene::SceneParams::from(&cfg);
            let renderer = rhythia_render::Renderer::new(width, height, cfg.hud_font.as_deref())
                .context("initialising GPU renderer")?;

            // Pick the fastest working encoder: probe the hardware encoders
            // (NVIDIA, Intel, then VAAPI) unless the user forced a choice.
            let encoder = match encoder.as_str() {
                "auto" => ["nvenc", "qsv", "vaapi"]
                    .into_iter()
                    .find(|e| rhythia_render::video::encoder_works(&ffmpeg, e))
                    .unwrap_or("x264")
                    .to_string(),
                other => other.to_string(),
            };
            eprintln!(
                "encoder: {}",
                match encoder.as_str() {
                    "nvenc" => "h264_nvenc (NVIDIA hardware)",
                    "qsv" => "h264_qsv (Intel hardware)",
                    "vaapi" => "h264_vaapi (VAAPI hardware)",
                    _ => "libx264 (software)",
                }
            );
            // Hit sounds come from the extracted game assets folder.
            let hitsounds = game_assets
                .as_ref()
                .filter(|_| hitsound_volume > 0)
                .and_then(|dir| {
                    let sounds = dir.join("builtin_assets").join("sounds");
                    let hit_wav = std::fs::read(sounds.join("hit.wav")).ok()?;
                    Some(rhythia_render::video::HitsoundOptions {
                        hit_wav,
                        miss_wav: std::fs::read(sounds.join("miss.wav")).ok(),
                        volume: hitsound_volume.min(150) as f32 / 100.0,
                    })
                });
            if hitsound_volume > 0 && hitsounds.is_none() {
                eprintln!("note: hit sounds requested but not found (need --game-assets with extracted sounds)");
            }
            let ghost = match &ghost_replay {
                Some(p) => {
                    let g = Replay::from_path(p)
                        .with_context(|| format!("reading ghost replay {}", p.display()))?;
                    if g.map_id != r.map_id && !g.beatmap_hash.is_empty() && g.beatmap_hash != r.beatmap_hash {
                        anyhow::bail!("ghost replay was played on a different map");
                    }
                    Some(rhythia_render::video::GhostOptions {
                        replay: g,
                        color: [1.0, 0.55, 0.24],
                    })
                }
                None => None,
            };
            let opts = rhythia_render::video::VideoOptions {
                fps,
                start_ms,
                end_ms,
                ffmpeg,
                audio: audio_path,
                crf,
                preset,
                encoder,
                results_secs,
                motion_blur,
                music_volume: music_volume.min(150) as f32 / 100.0,
                hitsounds,
                ghost,
            };

            println!(
                "rendering {:.1}s of {} @ {}x{}/{} (speed {:.2}) -> {}",
                (end_ms - start_ms) / 1000.0,
                replay.file_name().unwrap_or_default().to_string_lossy(),
                width,
                height,
                fps,
                r.speed,
                out.display()
            );
            let start_t = std::time::Instant::now();
            rhythia_render::video::render_video(
                &renderer,
                &params,
                &cfg,
                &r,
                &m,
                &out,
                &opts,
                |done, total| {
                    if done % 30 == 0 || done == total {
                        let pct = 100 * done / total;
                        eprint!("\r  {pct:3}%  ({done}/{total} frames)   ");
                    }
                    true
                },
            )
            .context("rendering video")?;
            eprintln!();
            println!(
                "done in {:.1}s -> {}",
                start_t.elapsed().as_secs_f64(),
                out.display()
            );
            Ok(true)
        }
    }
}

fn print_info(r: &Replay) {
    println!("version        {}", r.version);
    println!("player         {}", r.player_name);
    println!("map id         {} ({})", r.map_id, r.legacy_map_id);
    println!("mode           {}", r.mode);
    println!(
        "played (unix)  {}",
        r.unix_ms()
            .map_or_else(|| "- (invalid timestamp)".into(), |ms| format!("{ms} ms"))
    );
    println!("passed         {}", r.passed);
    println!("mods           {}", r.mods);
    println!("speed          {}", r.speed);
    println!("total score    {}", r.total_score);
    println!("accuracy       {:.4} %", r.accuracy_pct);
    println!("hits/misses    {}/{}", r.hits, r.misses);
    println!("points (SP)    {}", r.points);
    println!(
        "fail time      {}",
        if r.failed() {
            format!("{} ms", r.fail_time_ms)
        } else {
            "- (passed)".into()
        }
    );
    println!("beatmap hash   {}", r.beatmap_hash);
    println!(
        "frames         {} ({:.1} s, {} flagged)",
        r.frames.len(),
        r.length_ms() / 1000.0,
        r.flagged_frames()
    );
    if r.trailing_bytes > 0 {
        println!("!! trailing    {} unparsed bytes", r.trailing_bytes);
    }
}

fn print_info_json(r: &Replay) {
    let value = serde_json::json!({
        "version": r.version,
        "player": r.player_name,
        "map_id": r.map_id,
        "legacy_map_id": r.legacy_map_id,
        "mode": r.mode,
        "unix_ms": r.unix_ms(),
        "passed": r.passed,
        "mods": r.mods,
        "spin": r.spin,
        "speed": r.speed,
        "total_score": r.total_score,
        "accuracy_pct": r.accuracy_pct,
        "hits": r.hits,
        "misses": r.misses,
        "points": r.points,
        "fail_time_ms": r.fail_time_ms,
        "beatmap_hash": r.beatmap_hash,
        "frame_count": r.frames.len(),
        "length_ms": r.length_ms(),
        "flagged_frames": r.flagged_frames(),
        "trailing_bytes": r.trailing_bytes,
    });
    println!("{}", serde_json::to_string_pretty(&value).unwrap());
}

fn print_report(report: &integrity::IntegrityReport) {
    for check in &report.checks {
        let mark = if check.ok { "ok " } else { "FAIL" };
        let sev = match check.severity {
            integrity::Severity::Error => "",
            integrity::Severity::Warning => " (warning)",
        };
        println!(
            "{mark}  {}{sev}: expected {}, got {}",
            check.name, check.expected, check.actual
        );
    }
    if report.consistent() {
        println!("=> replay data is consistent");
    } else {
        println!("=> REPLAY DATA INCONSISTENT — possibly manipulated");
    }
}
