//! Command-line interface: replay inspection, integrity checks, and
//! frame/video rendering.

mod manifest;

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::ExitCode;

use anyhow::Context;
use clap::{Parser, Subcommand};
use rhythia_formats::{map::Map, rhr::Replay};
use rhythia_sim::integrity;
use serde::Deserialize;

#[derive(Parser)]
#[command(
    // The binary is rhythr-cli; the old name here was the render crate's and
    // put a name in --version and --help that exists nowhere else.
    name = "rhythr-cli",
    version,
    about = "Rhythia replay renderer (read-only)"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Measure which way of handing frames to ffmpeg is fastest here, and
    /// print what each one managed.
    BenchTransport {
        /// Output width to measure at; the best transport depends on it.
        #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(320..=7680))]
        width: u32,
        #[arg(long, default_value_t = 1080, value_parser = clap::value_parser!(u32).range(240..=4320))]
        height: u32,
        #[arg(long, default_value = "ffmpeg")]
        ffmpeg: String,
    },
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
        /// Output width in pixels (320-7680).
        #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(320..=7680))]
        width: u32,
        /// Output height in pixels (240-4320).
        #[arg(long, default_value_t = 1080, value_parser = clap::value_parser!(u32).range(240..=4320))]
        height: u32,
        /// The player's config.json or exported .rhs skin (adopts their
        /// note skin, camera, colours). Defaults applied when omitted.
        #[arg(long)]
        config: Option<PathBuf>,
        /// Directory with the game's built-in assets (builtin_colorsets.json
        /// + notes/borders/cursors) to resolve built-in skin references.
        #[arg(long)]
        game_assets: Option<PathBuf>,
        /// Apply the HUD layout arranged in the desktop app (element
        /// positions, sizes, visibility, error/aim meters). Without an app
        /// settings file the render is unchanged.
        #[arg(long)]
        app_layout: bool,
        /// Take that layout from this settings.json instead of the app's
        /// own; implies --app-layout.
        #[arg(long, value_name = "FILE")]
        app_layout_file: Option<PathBuf>,
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
        /// Frames per second (24-240).
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(24..=240))]
        fps: u32,
        /// Output width in pixels (320-7680).
        #[arg(long, default_value_t = 1920, value_parser = clap::value_parser!(u32).range(320..=7680))]
        width: u32,
        /// Output height in pixels (240-4320).
        #[arg(long, default_value_t = 1080, value_parser = clap::value_parser!(u32).range(240..=4320))]
        height: u32,
        /// Render quality 0-100, higher is better. Mapped onto whichever
        /// encoder ends up being used; --crf overrides it with a raw CRF.
        #[arg(long, default_value_t = rhythia_render::quality::DEFAULT)]
        quality: u32,
        /// Raw x264 CRF, for scripts written before --quality existed. Wins
        /// over --quality when both are given.
        #[arg(long)]
        crf: Option<u32>,
        /// Feed frames to ffmpeg over its stdin instead of a loopback
        /// socket. The socket is the default and is probed before it is
        /// used; this is the way out if it ever misbehaves.
        #[arg(long)]
        no_tcp_feed: bool,
        /// Bytes handed to that socket per write; 0 = the whole frame at
        /// once. The best value differs by platform.
        #[arg(long, default_value_t = 256 * 1024)]
        socket_chunk: usize,
        /// Diagnostic: run everything but encode nothing and write no file,
        /// so the reported feed time is the transport by itself.
        #[arg(long)]
        dry_run: bool,
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
        /// Apply the HUD layout arranged in the desktop app (see
        /// `frame --app-layout`).
        #[arg(long)]
        app_layout: bool,
        /// Take that layout from this settings.json instead of the app's
        /// own; implies --app-layout.
        #[arg(long, value_name = "FILE")]
        app_layout_file: Option<PathBuf>,
        /// Ghost races: hide the score-lead widget (numbers + bar) and the
        /// results delta graph.
        #[arg(long)]
        no_racing_delta: bool,
        /// Custom playfield background: an image or a video file (videos
        /// play muted and looped). Replaces the skin's background; the
        /// results screen keeps its own look.
        #[arg(long)]
        background: Option<PathBuf>,
        /// How much the custom background is darkened, 0-100 percent.
        #[arg(long, default_value_t = 60, value_parser = clap::value_parser!(u32).range(0..=100))]
        background_dim: u32,
        /// Zoom on the custom background, percent (100 = exactly covering
        /// the frame).
        #[arg(long, default_value_t = 100)]
        background_zoom: u32,
        /// Horizontal shift of the custom background in percent of the
        /// frame width (positive = right); clamped so the frame stays
        /// covered.
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        background_x: i32,
        /// Vertical shift in percent of the frame height (positive =
        /// down).
        #[arg(long, default_value_t = 0, allow_negative_numbers = true)]
        background_y: i32,
        /// Video backgrounds: start (and loop) playback from this second.
        #[arg(long, default_value_t = 0.0)]
        background_start: f64,
        /// Video backgrounds in clip renders (--start): "song" plays the
        /// video as if it ran since 0:00 of the song, "clip" restarts it
        /// at the clip start.
        #[arg(long, default_value = "song", value_parser = ["song", "clip"])]
        background_sync: String,
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

// -------------------------------------------------------------- app layout

/// The layout half of the desktop app's settings file: where the drag editor
/// put each HUD element, how large, which ones are on, and the optional
/// overlay meters. The rest of what the app stores (resolution, encoder,
/// output paths, background) already has its own flag here and is ignored:
/// a CLI render must stay driven by its command line.
///
/// This mirrors `Settings` in crates/gui/src/main.rs by FIELD NAME; the app
/// is a binary crate, so its types cannot be imported. Any rename there has
/// to be repeated here (see also its `apply_overrides`).
#[derive(Deserialize, Default)]
#[serde(default)]
struct AppLayout {
    hud_overrides: BTreeMap<String, bool>,
    hud_positions: BTreeMap<String, [f32; 2]>,
    hud_scales: BTreeMap<String, f32>,
    /// Absent = the app never wrote this meter; keep the config's own.
    error_meter: Option<MeterSettings>,
    aim_meter: Option<MeterSettings>,
    race_delta: Option<MeterSettings>,
}

/// One overlay meter's placement, as the app stores it.
#[derive(Deserialize, Clone, Copy)]
#[serde(default)]
struct MeterSettings {
    enabled: bool,
    x: f32,
    y: f32,
    ghost_x: Option<f32>,
    ghost_y: Option<f32>,
    scale: f32,
    alpha: f32,
}

impl Default for MeterSettings {
    fn default() -> Self {
        MeterSettings {
            enabled: false,
            x: 0.5,
            y: 0.88,
            ghost_x: None,
            ghost_y: None,
            scale: 1.0,
            alpha: 0.9,
        }
    }
}

impl MeterSettings {
    fn apply(self, target: &mut rhythia_render::config::ErrorMeter) {
        target.enabled = self.enabled;
        target.x = self.x.clamp(0.0, 1.0);
        target.y = self.y.clamp(0.0, 1.0);
        target.ghost_x = self.ghost_x.map(|v| v.clamp(0.0, 1.0));
        target.ghost_y = self.ghost_y.map(|v| v.clamp(0.0, 1.0));
        target.scale = self.scale.clamp(0.4, 2.5);
        target.alpha = self.alpha.clamp(0.05, 1.0);
    }
}

impl AppLayout {
    /// Applies the layout onto a loaded config, in the app's own order. An
    /// element the user never overrode keeps the skin config's value, so
    /// `--config` still decides everything the app was not asked about.
    fn apply(&self, cfg: &mut rhythia_render::SkinConfig) {
        for (key, &on) in &self.hud_overrides {
            let h = &mut cfg.hud;
            match key.as_str() {
                "song_info" => h.song_info = on,
                "song_progress" => h.song_progress_bar = on,
                "combo_ring" => h.combo_ring = on,
                "pauses" => h.pauses = on,
                "grade" => h.grade = on,
                "accuracy" => h.accuracy = on,
                "score" => h.score = on,
                "points" => h.points = on,
                "misses" => h.misses = on,
                "notes" => h.notes = on,
                "health_bar" => h.health_bar = on,
                // Both of these are hidden in the game by zeroing an
                // opacity, so switching them back on needs a visible value.
                "combo_text" => {
                    h.playfield_combo_text = on;
                    if on && h.combo_text_opacity <= 0.0 {
                        h.combo_text_opacity = 0.05;
                    }
                }
                "miss_marker" => {
                    if !on {
                        h.miss_effect_opacity = 0.0;
                    } else if h.miss_effect_opacity <= 0.0 {
                        h.miss_effect_opacity = 1.0;
                    }
                }
                "speed_label" => h.speed_label = on,
                _ => {}
            }
        }
        // The app clamps drags and resizes as they happen; a hand-edited or
        // older file carries no such promise.
        cfg.hud.positions = self
            .hud_positions
            .iter()
            .map(|(k, p)| (k.clone(), [p[0].clamp(0.0, 1.0), p[1].clamp(0.0, 1.0)]))
            .collect();
        cfg.hud.scales = self
            .hud_scales
            .iter()
            .map(|(k, &s)| (k.clone(), s.clamp(0.4, 2.5)))
            .collect();
        if let Some(m) = self.error_meter {
            m.apply(&mut cfg.hud.error_meter);
        }
        if let Some(m) = self.aim_meter {
            m.apply(&mut cfg.hud.aim_meter);
        }
        if let Some(m) = self.race_delta {
            m.apply(&mut cfg.hud.race_delta);
        }
    }
}

/// Where the desktop app keeps its settings. Spelled out rather than pulled
/// in via the app's `dirs` dependency: one platform rule is cheaper here
/// than a crate, but it has to keep matching `config_dir()` over there.
fn app_settings_path() -> PathBuf {
    let base = if cfg!(windows) {
        std::env::var_os("APPDATA").map(PathBuf::from)
    } else if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            // The spec says a relative XDG_CONFIG_HOME is to be ignored.
            .filter(|p| p.is_absolute())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    };
    base.unwrap_or_else(|| PathBuf::from("."))
        .join("rhythr")
        .join("settings.json")
}

/// None = nothing to apply: neither flag given, or the app has never written
/// its settings file. Somebody who only ever used the CLI renders exactly as
/// before.
fn load_app_layout(use_app: bool, file: &Option<PathBuf>) -> anyhow::Result<Option<AppLayout>> {
    let (path, named) = match file {
        Some(p) => (p.clone(), true),
        None if use_app => (app_settings_path(), false),
        None => return Ok(None),
    };
    let text = match std::fs::read_to_string(&path) {
        Ok(t) => t,
        // A file the user named is a typo when it's missing; the app's own
        // is simply absent until the app has run once.
        Err(e) if named || e.kind() != std::io::ErrorKind::NotFound => {
            return Err(
                anyhow::Error::new(e).context(format!("reading app layout {}", path.display()))
            )
        }
        Err(_) => {
            eprintln!(
                "note: no app settings at {}, rendering the config's own HUD layout",
                path.display()
            );
            return Ok(None);
        }
    };
    // Loud on a broken file: silently rendering the wrong look costs an
    // encode, and the app keeps a `.broken` copy for exactly this case.
    let layout: AppLayout = serde_json::from_str(&text)
        .with_context(|| format!("parsing app layout {}", path.display()))?;
    Ok(Some(layout))
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
            // The same code the desktop app would print for this failure, so
            // a report is worth the same whichever one produced it.
            eprintln!("error: {}", rhythia_errcode::stamp(&format!("{err:#}")));
            ExitCode::FAILURE
        }
    }
}

fn run() -> anyhow::Result<bool> {
    match Cli::parse().command {
        Command::BenchTransport {
            width,
            height,
            ffmpeg,
        } => {
            // Distinguish "ffmpeg cannot run" from "every transport failed":
            // without this the whole table printed "failed" and the command
            // still exited 0, so a script could not tell a broken ffmpeg from
            // a measurement.
            if !rhythia_render::video::ffmpeg_runs(&ffmpeg) {
                anyhow::bail!("ffmpeg could not be run ({ffmpeg}); nothing to measure");
            }
            eprintln!("measuring at {width}x{height} (a few seconds)…");
            let b = rhythia_render::transport::benchmark(&ffmpeg, width, height);
            for m in &b.results {
                match m.fps {
                    Some(f) => println!("  {:<22} {:>8.0} frames/s", m.transport.label(), f),
                    None => println!("  {:<22} {:>8}", m.transport.label(), "failed"),
                }
            }
            println!();
            println!("=> {}", b.summary());
            // Non-zero exit if nothing could actually be measured.
            Ok(b.best.is_some())
        }
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
            let mut r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            let m = Map::from_path(&map).with_context(|| format!("reading {}", map.display()))?;
            // Wall-clock replays are legitimate; don't flag them as
            // tampered just for their time base.
            rhythia_sim::timebase::normalize(&mut r, &m);
            let report = integrity::verify_replay(&r, &m);
            print_report(&report, r.hits);
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
            app_layout,
            app_layout_file,
        } => {
            let song_ms = parse_time_ms(&at).context("parsing --at")?;
            let r = Replay::from_path(&replay)
                .with_context(|| format!("reading {}", replay.display()))?;
            let m = Map::from_path(&map).with_context(|| format!("reading {}", map.display()))?;
            let mut r = r;
            rhythia_sim::timebase::normalize(&mut r, &m);
            let mut cfg = load_config(&config, &game_assets)?;
            if let Some(layout) = load_app_layout(app_layout, &app_layout_file)? {
                layout.apply(&mut cfg);
            }

            // Surface tampering before spending time rendering.
            let report = integrity::verify_replay(&r, &m);
            if !report.consistent() {
                // Loading the wrong chart is a mistake, not tampering, and it
                // is by far the likelier of the two. Saying "possibly
                // manipulated" for it accused people of something they had
                // not done.
                if integrity::looks_like_the_wrong_map(r.hits, &report, false) {
                    eprintln!(
                        "warning: most recorded hits find no note on this map. This is \
                         probably not the map that was played"
                    );
                } else {
                    eprintln!("warning: replay data is inconsistent (possibly manipulated)");
                }
            }

            let mut params = rhythia_render::scene::SceneParams::from(&cfg);
            let renderer = rhythia_render::Renderer::new(width, height, cfg.hud_font.as_deref())
                .context("initialising GPU renderer")?;
            let skin = renderer.prepare_skin(&cfg);
            // Show the field the player actually saw (mirror/hardrock).
            let (m, mods) = rhythia_render::mods::map_for_replay(&m, &r);
            params.apply_mods(&mods);
            params.apply_speed(r.speed);
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
            quality,
            crf,
            no_tcp_feed,
            socket_chunk,
            dry_run,
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
            app_layout,
            app_layout_file,
            no_racing_delta,
            background,
            background_dim,
            background_zoom,
            background_x,
            background_y,
            background_start,
            background_sync,
        } => {
            // h.264 refuses an odd side, and the GPU colour conversion needs
            // a width divisible by four (four pixels per 32-bit word), so an
            // unrounded size either fails minutes in or runs ten times slower
            // at the conversion with nothing said. The GUI rounds for you;
            // here it is worth saying out loud, since a script may be passing
            // the number.
            let (width, height) = {
                let w = (width / 4 * 4).clamp(320, 7680);
                let h = (height / 2 * 2).clamp(240, 4320);
                if (w, h) != (width, height) {
                    eprintln!("note: rendering at {w}x{h} (width to a multiple of 4, height to an even number)");
                }
                (w, h)
            };
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
            let mut cfg = load_config(&config, &game_assets)?;
            if let Some(layout) = load_app_layout(app_layout, &app_layout_file)? {
                layout.apply(&mut cfg);
            }
            // The opt-out wins over the app layout; the widget is otherwise
            // on by default (HudConfig::default), as it always was.
            if no_racing_delta {
                cfg.hud.race_delta.enabled = false;
            }
            let mut background_video = None;
            if let Some(bg) = &background {
                let bg_opts = rhythia_render::background::BackgroundOptions {
                    dim: background_dim.min(100) as f32 / 100.0,
                    zoom: background_zoom.clamp(100, 400) as f32 / 100.0,
                    offset: [
                        background_x.clamp(-100, 100) as f32 / 100.0,
                        background_y.clamp(-100, 100) as f32 / 100.0,
                    ],
                    start_secs: background_start.max(0.0),
                    sync_offset_secs: 0.0,
                };
                let kind = rhythia_render::background::apply_background(&mut cfg, bg, &bg_opts)
                    .with_context(|| format!("reading background {}", bg.display()))?;
                if kind == rhythia_render::background::BackgroundKind::Video {
                    background_video = Some(rhythia_render::video::BackgroundVideo {
                        path: bg.clone(),
                        opts: bg_opts,
                    });
                }
            }

            // Normalize BEFORE deriving the range: a wall-clock replay's raw
            // fail time / length is 1/speed of the song and would truncate the
            // render (render_video normalizes again, which is idempotent).
            let mut r = r;
            rhythia_sim::timebase::normalize(&mut r, &m);

            let report = integrity::verify_replay(&r, &m);
            if !report.consistent() {
                // Loading the wrong chart is a mistake, not tampering, and it
                // is by far the likelier of the two. Saying "possibly
                // manipulated" for it accused people of something they had
                // not done.
                if integrity::looks_like_the_wrong_map(r.hits, &report, false) {
                    eprintln!(
                        "warning: most recorded hits find no note on this map. This is \
                         probably not the map that was played"
                    );
                } else {
                    eprintln!("warning: replay data is inconsistent (possibly manipulated)");
                }
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
            if background_sync == "song" && start_ms > 0.0 {
                if let Some(bv) = background_video.as_mut() {
                    let dur = rhythia_render::background::probe_duration(&ffmpeg, &bv.path);
                    let speed = f64::from(r.speed).clamp(0.25, 3.0);
                    bv.opts.sync_offset_secs = rhythia_render::background::sync_offset(
                        start_ms / 1000.0 / speed,
                        bv.opts.start_secs,
                        dur,
                    );
                }
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
                "auto" => rhythia_render::video::hardware_encoders()
                    .iter()
                    .copied()
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
                    "amf" => "h264_amf (AMD hardware)",
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
                    if g.map_id != r.map_id
                        && !g.beatmap_hash.is_empty()
                        && g.beatmap_hash != r.beatmap_hash
                    {
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
                extra_output_args: Vec::new(),
                fps,
                start_ms,
                end_ms,
                ffmpeg,
                audio: audio_path,
                tcp_feed: !no_tcp_feed,
                socket_chunk,
                discard_output: dry_run,
                quality: match crf {
                    // A raw CRF from an older script still has to mean what
                    // it always meant, so it is converted rather than read as
                    // a point on the new, inverted scale. The new scale does
                    // not reach past CRF 14 or 34, so anything outside that
                    // is coerced, and says so, rather than quietly encoding
                    // something other than what was asked for.
                    Some(c) => {
                        let q = rhythia_render::quality::from_legacy_crf(c);
                        let landed = rhythia_render::quality::x264_crf(q);
                        if landed != c {
                            eprintln!(
                                "note: --crf {c} is outside the range this build can express; \
                                 encoding at CRF {landed} (--quality {q})"
                            );
                        }
                        q
                    }
                    None => {
                        quality.clamp(rhythia_render::quality::MIN, rhythia_render::quality::MAX)
                    }
                },
                preset,
                encoder,
                results_secs,
                motion_blur,
                music_volume: music_volume.min(150) as f32 / 100.0,
                hitsounds,
                ghost,
                background_video,
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
            let stats = rhythia_render::video::render_video(
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
            eprintln!("  {}", stats.summary());
            // --dry-run encodes nothing and writes no file, so it must not
            // claim "done -> <path>" for a path it never created.
            if dry_run {
                println!(
                    "measured in {:.1}s (diagnostic run: no file written)",
                    start_t.elapsed().as_secs_f64()
                );
            } else {
                println!(
                    "done in {:.1}s -> {}",
                    start_t.elapsed().as_secs_f64(),
                    out.display()
                );
            }
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

fn print_report(report: &integrity::IntegrityReport, replay_hits: i32) {
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
    } else if integrity::looks_like_the_wrong_map(replay_hits, &report, false) {
        println!("=> PROBABLY THE WRONG MAP: most recorded hits find no note here");
    } else {
        println!("=> REPLAY DATA INCONSISTENT (possibly manipulated)");
    }
}
