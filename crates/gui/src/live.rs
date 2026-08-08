//! The live Analyze engine: a dedicated render thread drawing every
//! displayed frame directly to the Analyze window's surface, paced by
//! vsync. Time is a virtual clock (`t += dt · speed`), so changing the
//! playback speed, seeking and stepping are free — there is nothing to
//! buffer and nothing to invalidate.
//!
//! The webview stays on top of the surface as a transparent layer for
//! controls and overlays; it receives `live-tick` events carrying the
//! clock and the per-side screen geometry for the overlay canvas.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use tauri::Emitter;

use rhythia_formats::map::Map;
use rhythia_formats::rhr::Replay;
use rhythia_render::config::SkinConfig;
use rhythia_render::scene::SceneParams;

pub enum LiveCmd {
    Play,
    Pause,
    Seek(f64),
    Speed(f64),
    /// Window inner size changed (physical px).
    Resize(u32, u32),
    View { hide_cursor: bool, hide_notes: bool },
    /// How long resolved hit-area boxes linger (ms of song time).
    Linger(f64),
    /// Render the CURRENT clock position to PNG bytes — the overlay
    /// snapshot wants exactly what the screen shows (skin background,
    /// live resolution), not the preview pipeline's version.
    Still(Sender<Result<Vec<u8>, String>>),
    Stop,
}

pub struct LiveSession {
    pub tx: Sender<LiveCmd>,
    pub running: Arc<AtomicBool>,
}

#[derive(Default)]
pub struct LiveHandles {
    pub session: Mutex<Option<LiveSession>>,
    /// A session start is in flight: the window's close handler must not
    /// let the window die under it (the start would spawn a render thread
    /// against a destroyed window and leak the session).
    pub starting: AtomicBool,
}

/// Everything the render thread owns — cloned out of the app state once,
/// so the live loop never touches the shared preview lock.
pub struct LiveInit {
    pub replay: Replay,
    pub map: Map,
    pub ghost: Option<Replay>,
    pub cfg: SkinConfig,
    pub run_end: f64,
    pub hide_cursor: bool,
    pub hide_notes: bool,
    pub linger_ms: f64,
    pub win_w: u32,
    pub win_h: u32,
    pub settings_w: u32,
    pub settings_h: u32,
}

#[derive(Serialize, Clone)]
struct TickNoteQuad {
    i: u32,
    pts: [[f32; 2]; 4],
}

#[derive(Serialize, Clone)]
struct TickSide {
    x: u32,
    w: u32,
    m: [[f32; 4]; 4],
    notes: Vec<TickNoteQuad>,
    field: [[f32; 2]; 4],
}

#[derive(Serialize, Clone)]
struct LiveTick {
    t: f64,
    playing: bool,
    speed: f64,
    fps: f32,
    /// Letterbox rect of the frame inside the window (physical px).
    rect: [f32; 4],
    /// Render size in frame px — overlay coordinates live in this space.
    fw: u32,
    fh: u32,
    sides: Vec<TickSide>,
}

/// Fits the render resolution to the window at the output aspect.
fn fit_render_size(win_w: u32, win_h: u32, set_w: u32, set_h: u32) -> (u32, u32) {
    let aspect = set_w.max(1) as f32 / set_h.max(1) as f32;
    let (ww, wh) = (win_w.max(64) as f32, win_h.max(64) as f32);
    let (w, h) = if ww / wh > aspect {
        (wh * aspect, wh)
    } else {
        (ww, ww / aspect)
    };
    // Cap: a 4K window is fine on a strong GPU, but never explode past it.
    let scale = (2160.0 / h).min(1.0);
    (((w * scale) as u32).max(64) & !1, ((h * scale) as u32).max(64) & !1)
}

/// Spawns the live thread. The surface MUST have been created on the
/// main thread (macOS) from the same instance handed over here.
#[allow(clippy::too_many_arguments)]
pub fn spawn(
    app_handle: tauri::AppHandle,
    window_label: String,
    instance: wgpu::Instance,
    surface: wgpu::Surface<'static>,
    init: LiveInit,
    rx: Receiver<LiveCmd>,
    running: Arc<AtomicBool>,
) {
    std::thread::spawn(move || {
        // Runs on EVERY exit — early error returns, panics, clean stop.
        // The closer waits on `running`; a path that forgets to flip it
        // turns every window close into a full watchdog timeout.
        struct DoneGuard {
            running: Arc<AtomicBool>,
            app: tauri::AppHandle,
        }
        impl Drop for DoneGuard {
            fn drop(&mut self) {
                if std::thread::panicking() {
                    let _ = self.app.emit("live-error", "live engine crashed".to_string());
                }
                self.running.store(false, Ordering::SeqCst);
                let _ = self.app.emit("live-stopped", ());
            }
        }
        let _done = DoneGuard { running: running.clone(), app: app_handle.clone() };
        let emit_err = |msg: String| {
            let _ = app_handle.emit("live-error", msg);
        };
        let (rw, rh) = fit_render_size(init.win_w, init.win_h, init.settings_w, init.settings_h);
        let renderer = match rhythia_render::Renderer::new_for_surface(
            instance,
            &surface,
            rw,
            rh,
            init.cfg.hud_font.as_deref(),
        ) {
            Ok(r) => r,
            Err(e) => return emit_err(format!("live renderer: {e}")),
        };
        let mut renderer = renderer;
        let mut presenter =
            match rhythia_render::present::Presenter::new(&renderer, surface, init.win_w, init.win_h) {
                Ok(p) => p,
                Err(e) => return emit_err(format!("live presenter: {e}")),
            };

        let mut cfg = init.cfg;
        // The value to restore a toggle TO is the skin's real opacity, so it
        // must be read before the hide flags zero it. Reading it afterwards
        // meant a window opened with notes or cursor hidden could only ever
        // bring them back at 1% — max(0.01) of the zero the hide had already
        // written.
        let base_cursor_opacity = cfg.cursor_opacity.max(0.01);
        let base_note_opacity = cfg.note_opacity.max(0.01);
        if init.hide_cursor {
            cfg.cursor_opacity = 0.0;
            cfg.cursor_trail_enabled = false;
        }
        if init.hide_notes {
            cfg.note_opacity = 0.0;
        }

        let skin = renderer.prepare_skin(&cfg);
        let replay = init.replay;
        let (main_map, main_mods) = rhythia_render::mods::map_for_replay(&init.map, &replay);
        let mut params = SceneParams::from(&cfg);
        params.apply_mods(&main_mods);
        params.apply_speed(replay.speed);
        let hud = rhythia_render::hud::HudState::new(&main_map, &replay);
        let mut ghost = init.ghost.map(|g| {
            let (gmap, gmods) = rhythia_render::mods::map_for_replay(&init.map, &g);
            rhythia_render::hud::GhostInput {
                state: rhythia_render::hud::HudState::new(&gmap, &g),
                replay: g,
                color: crate::GHOST_COLOR,
                map: gmap,
                mods: gmods,
                race: None,
            }
        });
        if let Some(g) = ghost.as_mut() {
            g.race = Some(rhythia_render::race::RaceSeries::for_race(
                &rhythia_render::race::RaceSide { map: &main_map, replay: &replay, state: &hud },
                &rhythia_render::race::RaceSide { map: &g.map, replay: &g.replay, state: &g.state },
            ));
        }

        // Visual verification on platforms whose webview hides the
        // surface (Linux test rigs): dump a frame every second.
        let dump_dir = std::env::var("RHYTHR_LIVE_DUMP").ok().map(std::path::PathBuf::from);
        let mut dump_counter = 0u64;

        let mut linger_ms = init.linger_ms;
        let mut t = 0.0f64;
        let mut playing = false;
        // What the last overlay tick was computed from. While paused nothing
        // it describes can change unless a command or a resize changed it,
        // and recomputing every note's screen quad ~40 times a second for a
        // still picture is 3400 notes a tick on a dense map, for nothing.
        let mut last_tick: Option<(u64, u64, u64, u32, u32, [u32; 4], bool)> = None;
        let mut speed = 1.0f64;
        let mut last = std::time::Instant::now();
        let mut fps = 0.0f32;
        let mut tick_counter = 0u64;
        let mut pending_resize: Option<(u32, u32)> = None;
        let mut dirty = true;

        let _ = app_handle.emit("live-started", ());

        'run: loop {
            // Commands first — they are cheap and must not lag a frame.
            loop {
                match rx.try_recv() {
                    Ok(LiveCmd::Play) => {
                        if t >= init.run_end - 1.0 {
                            t = 0.0;
                        }
                        playing = true;
                        last = std::time::Instant::now();
                    }
                    Ok(LiveCmd::Pause) => playing = false,
                    Ok(LiveCmd::Seek(to)) => {
                        t = to.clamp(0.0, init.run_end);
                        dirty = true;
                    }
                    Ok(LiveCmd::Speed(s)) => speed = s.clamp(0.01, 4.0),
                    Ok(LiveCmd::Resize(w, h)) => pending_resize = Some((w, h)),
                    Ok(LiveCmd::View { hide_cursor, hide_notes }) => {
                        cfg.cursor_opacity = if hide_cursor { 0.0 } else { base_cursor_opacity };
                        cfg.cursor_trail_enabled = !hide_cursor;
                        cfg.note_opacity = if hide_notes { 0.0 } else { base_note_opacity };
                        // Note opacity lives in SceneParams, built once at
                        // startup — rebuild it or the toggle never lands.
                        let mut p = SceneParams::from(&cfg);
                        p.apply_mods(&main_mods);
                        p.apply_speed(replay.speed);
                        params = p;
                        dirty = true;
                    }
                    Ok(LiveCmd::Linger(v)) => {
                        linger_ms = v.clamp(0.0, 2000.0);
                        dirty = true;
                    }
                    Ok(LiveCmd::Still(reply)) => {
                        let res = renderer
                            .render_still_with_ghost(
                                &params, &cfg, &skin, &replay, &main_map, t, Some(&hud),
                                ghost.as_ref(),
                            )
                            .map_err(|e| e.to_string())
                            .and_then(|px| {
                                let (w, h) = renderer.dimensions();
                                png_bytes_vec(&px, w, h)
                            });
                        let _ = reply.send(res);
                    }
                    Ok(LiveCmd::Stop) => break 'run,
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => break 'run,
                }
            }

            if let Some((w, h)) = pending_resize.take() {
                let (rw, rh) = fit_render_size(w, h, init.settings_w, init.settings_h);
                renderer.resize(rw, rh);
                presenter.resize(&renderer, w, h);
                presenter.rebind(&renderer);
                dirty = true;
            }

            let now = std::time::Instant::now();
            // Cap: a debugger stall or system sleep must not leap the
            // clock to the end of the replay in one frame.
            let dt = (now.duration_since(last).as_secs_f64() * 1000.0).min(100.0);
            last = now;
            if playing {
                t += dt * speed * (replay.speed as f64).clamp(0.25, 3.0);
                if t >= init.run_end {
                    t = init.run_end;
                    playing = false;
                    let _ = app_handle.emit("live-ended", ());
                }
                // Only real frame intervals feed the meter — the first
                // loop pass after unpausing arrives ~0 ms after `last`
                // was reset and would inject a ~1000 fps spike.
                if dt >= 2.0 {
                    let sample = (1000.0 / dt) as f32;
                    fps = if fps == 0.0 { sample } else { fps * 0.9 + sample * 0.1 };
                }
            }

            if playing || dirty {
                dirty = false;
                if let Err(e) = renderer.render_live(
                    &params,
                    &cfg,
                    &skin,
                    &replay,
                    &main_map,
                    t,
                    Some(&hud),
                    ghost.as_ref(),
                ) {
                    emit_err(format!("live render: {e}"));
                    break 'run;
                }
                match presenter.present_frame(&renderer) {
                    // Skipped (occluded/outdated): no vsync block happened
                    // — sleep, or this loop spins a core at full rate
                    // while the window is minimized.
                    Ok(false) => std::thread::sleep(std::time::Duration::from_millis(8)),
                    Ok(true) => {}
                    Err(e) => {
                        emit_err(format!("present: {e}"));
                        break 'run;
                    }
                }
                if let Some(dir) = &dump_dir {
                    dump_counter += 1;
                    if dump_counter % 60 == 1 {
                        if let Ok(px) = renderer.render_still_with_ghost(
                            &params, &cfg, &skin, &replay, &main_map, t, Some(&hud),
                            ghost.as_ref(),
                        ) {
                            let (w, h) = renderer.dimensions();
                            let path = dir.join(format!("live-{:05}.png", dump_counter / 60));
                            let _ = save_png(&path, &px, w, h);
                        }
                    }
                }
            } else {
                // Paused and clean: idle without burning a core.
                std::thread::sleep(std::time::Duration::from_millis(8));
            }

            // Overlay tick at ~half display rate — plenty for the canvas.
            tick_counter += 1;
            let (fw, fh) = renderer.dimensions();
            let (rx0, ry0, rvw, rvh) = presenter.frame_rect(&renderer);
            let tick_key = (
                t.to_bits(),
                speed.to_bits(),
                linger_ms.to_bits(),
                fw,
                fh,
                [rx0.to_bits(), ry0.to_bits(), rvw.to_bits(), rvh.to_bits()],
                playing,
            );
            // Playing: half the display rate, plenty for the canvas. Paused:
            // only when something actually moved.
            let send_tick = if playing {
                tick_counter.is_multiple_of(2)
            } else {
                last_tick != Some(tick_key)
            };
            if send_tick {
                last_tick = Some(tick_key);
                let sides = renderer
                    .field_projections(
                        &params,
                        &replay,
                        ghost.as_ref().map(|g| (&g.replay, g.mods.grid_scale)),
                        t,
                    )
                    .into_iter()
                    .enumerate()
                    .map(|(i, ((x, w), m))| {
                        let (side_params, side_map, side_replay, side_hud) =
                            match (i, ghost.as_ref()) {
                                (1, Some(g)) => {
                                    let mut p = params;
                                    p.apply_mods(&g.mods);
                                    (p, &g.map, &g.replay, &g.state)
                                }
                                _ => (params, &main_map, &replay, &hud),
                            };
                        let notes = renderer
                            .note_screen_quads(
                                &side_params,
                                side_map,
                                side_replay,
                                t,
                                (x, w),
                                Some(side_hud),
                                linger_ms,
                            )
                            .into_iter()
                            .map(|(i, pts, _)| TickNoteQuad { i: i as u32, pts })
                            .collect();
                        let field =
                            renderer.playfield_quad(&side_params, side_replay, t, (x, w));
                        TickSide { x, w, m, notes, field }
                    })
                    .collect();
                let _ = app_handle.emit(
                    "live-tick",
                    LiveTick {
                        t,
                        playing,
                        speed,
                        fps,
                        rect: [rx0, ry0, rvw, rvh],
                        fw,
                        fh,
                        sides,
                    },
                );
            }

            if !playing {
                // Event pacing while paused (the render/present above only
                // runs when dirty, so this loop needs its own throttle).
                std::thread::sleep(std::time::Duration::from_millis(24));
            }
        }

        // Release every window-bound GPU object BEFORE the DoneGuard
        // signals done — the closer destroys the window the moment
        // `running` flips.
        drop(presenter);
        drop(renderer);
        fn save_png(path: &std::path::Path, rgba: &[u8], w: u32, h: u32) -> Result<(), String> {
            let bytes = png_bytes_vec(rgba, w, h)?;
            std::fs::write(path, bytes).map_err(|e| e.to_string())
        }
        let _ = window_label;
    });
}

/// Encodes RGBA pixels as PNG bytes in memory.
fn png_bytes_vec(rgba: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(std::io::Cursor::new(&mut out), w, h);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut wr = enc.write_header().map_err(|e| e.to_string())?;
        wr.write_image_data(rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
