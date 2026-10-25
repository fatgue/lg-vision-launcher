//! Vision: a fast native home launcher for rooted LG webOS TVs.
//!
//! The UI thread renders only when something changed (input, data, the clock minute or a
//! running animation) and otherwise sleeps in SDL_WaitEventTimeout: 0 % GPU when idle.
mod anim;
mod color;
mod companion;
mod config;
mod dynlib;
mod gl;
mod home;
mod log;
#[cfg(all(target_os = "linux", target_arch = "arm"))]
mod luna;
#[cfg_attr(not(all(target_os = "linux", target_arch = "arm")), allow(dead_code))]
mod model;
mod net;
mod platform;
mod render;
mod sdl;
mod services;
#[cfg_attr(not(all(target_os = "linux", target_arch = "arm")), allow(dead_code))]
mod storage;
mod strings;
mod text;
mod theme;

use crate::home::{Action, Home, Key};
use crate::log::log;
use crate::model::{Request, ServiceRequest, Update};
use crate::platform::{Platform, APP_ID};
use crate::render::Renderer;
use crate::sdl::{Event, Sdl};
use crate::services::Services;
use crate::text::Text;
use chrono::{Datelike, Timelike};
use std::ffi::{c_int, CString};
use std::path::{Path, PathBuf};
use std::time::Instant;

const WEBOS: bool = cfg!(all(target_os = "linux", target_arch = "arm"));
const HOME_ID: &str = "com.webos.app.home";
/// LG's Home executable. With Vision as Home (tv/home.sh), Vision is bind-mounted over it, so
/// SAM starts Vision under LG's own Home manifest and LS2 role (lifecycle v2).
const LG_HOME_EXE: &str = "/usr/bin/com.webos.app.home";
/// Vision's install directory, for when it runs as LG's Home executable.
const INSTALL_DIR: &str = "/media/developer/apps/usr/palm/applications/com.vision.launcher";
/// Written while Vision is Home: a normal tile launch then opens the Home instance instead.
const HOME_MARKER: &str = "/var/run/vision-home";

fn runs_as_home() -> bool {
    WEBOS && std::env::current_exe().is_ok_and(|exe| exe == Path::new(LG_HOME_EXE))
}

struct Options {
    perf: bool,
    frames: Option<u32>,
    screenshot: Option<PathBuf>,
    keys: Vec<Key>,
}

fn main() {
    let started = Instant::now();
    if WEBOS {
        log::init_file(if runs_as_home() { "/tmp/vision-home.log" } else { "/tmp/vision.log" });
    }
    if let Err(error) = run(started) {
        log!("vision: {error}");
        std::process::exit(1);
    }
}

fn run(started: Instant) -> Result<(), String> {
    let mut args: Vec<String> = std::env::args().collect();
    // SAM passes no CLI arguments; /tmp/vision-args carries debug flags on the TV. The jailed
    // process cannot delete root's file in sticky /tmp, so scripts/tv.mjs removes it.
    if WEBOS {
        if let Ok(extra) = std::fs::read_to_string("/tmp/vision-args") {
            let _ = std::fs::remove_file("/tmp/vision-args");
            args.extend(extra.split_whitespace().map(str::to_string));
            log!("debug args: {extra}");
        }
    }
    let options = Options {
        perf: args.iter().any(|a| a == "--perf") || std::env::var_os("VISION_PERF").is_some(),
        frames: args
            .iter()
            .position(|a| a == "--frames")
            .and_then(|i| args.get(i + 1))
            .and_then(|n| n.parse().ok()),
        screenshot: args
            .iter()
            .position(|a| a == "--screenshot")
            .and_then(|i| args.get(i + 1))
            .map(PathBuf::from),
        keys: args
            .iter()
            .position(|a| a == "--keys")
            .and_then(|i| args.get(i + 1))
            .map(|list| list.split(',').filter_map(parse_key).collect())
            .unwrap_or_default(),
    };
    let home_mode = runs_as_home();
    let exe_dir = if home_mode {
        PathBuf::from(INSTALL_DIR)
    } else {
        std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)).unwrap_or_default()
    };
    if WEBOS && !home_mode && std::fs::read_to_string(HOME_MARKER).is_ok_and(|m| m.trim() == APP_ID) {
        // Vision is Home: the app tile just brings that instance forward.
        open_home();
        return Ok(());
    }
    let assets = find_assets(&exe_dir)?;
    // Fonts load and UI glyphs rasterise in parallel with SDL/GL bring-up.
    let glyphs = text::prewarm(assets.join("fonts"), theme::prewarm_jobs(strings::current()));
    let data_dir = storage::data_dir(WEBOS);
    let prefs = storage::Prefs::load(&data_dir);

    if WEBOS {
        // SDL-webOS reads APPID for its shell surface and WMCLASS from the environment.
        let surface = if home_mode { HOME_ID } else { APP_ID };
        std::env::set_var("APPID", surface);
        std::env::set_var("SDL_VIDEO_WAYLAND_WMCLASS", surface);
    }
    let sdl = Sdl::open(&sdl_candidates(&exe_dir))?;
    if WEBOS {
        sdl.hint("SDL_WEBOS_ACCESS_POLICY_KEYS_BACK", "true");
        sdl.hint("SDL_WEBOS_ACCESS_POLICY_KEYS_EXIT", "true");
        // As Home, SDL cannot register under LG's role; Vision registers with SAM itself below.
        if home_mode {
            sdl.hint("SDL_WEBOS_REGISTER_APP", "0");
        }
    }
    // SAFETY: SDL calls on the main thread in documented order.
    if unsafe { (sdl.init)(sdl::INIT_VIDEO | sdl::INIT_EVENTS) } != 0 {
        return Err(format!("SDL_Init: {}", sdl.error()));
    }
    let (window, context, es) = create_window(&sdl)?;
    unsafe { (sdl.gl_set_swap_interval)(1) };
    unsafe { (sdl.show_cursor)(0) };

    let gl = gl::Gl::load(|name| {
        // SAFETY: the name is NUL-terminated (the loader macro appends it).
        unsafe { (sdl.gl_get_proc_address)(name.as_ptr().cast()) as *const _ }
    })?;
    log!(
        "vision {}: {} / {} / {}",
        env!("CARGO_PKG_VERSION"),
        gl.string(gl::VENDOR),
        gl.string(gl::RENDERER),
        gl.string(gl::VERSION)
    );
    let mut renderer = Renderer::new(gl, es)?;
    let glyph_wait = Instant::now();
    let prewarmed = glyphs.join().map_err(|_| "glyph thread panicked".to_string())??;
    let mut text = Text::new(&renderer, prewarmed);
    log!("glyphs: {} prewarmed, waited {} ms", text.glyph_count(), glyph_wait.elapsed().as_millis());
    let mut home = Home::new(&renderer, prefs);
    if home_mode {
        register_home(sdl.waker())?;
        log!("running as Home");
    }
    let platform = Platform::start(sdl.waker(), data_dir.clone());
    platform.send(Request::Refresh);
    let services = Services::start(platform.sink(), data_dir);
    let io = Io { platform: &platform, services: &services, home_mode };
    log!("init: {} ms", started.elapsed().as_millis());

    let mut perf = Perf::default();
    let mut dirty = true;
    // Hidden windows never draw: a backgrounded surface can block eglSwapBuffers indefinitely.
    let mut visible = true;
    let mut animating = false;
    let mut last_frame = Instant::now();
    let mut minute = u32::MAX;
    let mut first_frame = true;
    let mut frames = 0u32;
    let mut event = Event::zeroed();
    // OK fires on release; held past LONG_PRESS_MS it becomes LongOk instead.
    const LONG_PRESS_MS: u128 = 550;
    let mut ok_down: Option<Instant> = None;
    let mut long_fired = false;

    'main: loop {
        // Sleep until input, worker data or the next minute while nothing animates.
        // --frames is a benchmark: render continuously so timings cover every frame.
        let busy = visible && (animating || dirty || options.frames.is_some());
        let mut timeout = if busy { 0 } else { ms_to_next_minute() };
        if let (Some(down), false) = (ok_down, long_fired) {
            let left = LONG_PRESS_MS.saturating_sub(down.elapsed().as_millis()) as c_int;
            timeout = timeout.min(left.max(1));
        }
        let mut got = if timeout == 0 {
            unsafe { (sdl.poll_event)(&mut event) }
        } else {
            unsafe { (sdl.wait_event_timeout)(&mut event, timeout) }
        };
        while got != 0 {
            match event.kind() {
                sdl::QUIT | sdl::APP_TERMINATING => break 'main,
                sdl::LIFECYCLE_EVENT if event.user_code() == 2 => break 'main,
                sdl::LIFECYCLE_EVENT => {
                    // Home key: SAM relaunches the running Home, which brings itself forward.
                    unsafe {
                        (sdl.restore_window)(window);
                        (sdl.show_window)(window);
                        (sdl.raise_window)(window);
                    }
                    if !visible {
                        services.send(ServiceRequest::Visible(true));
                    }
                    visible = true;
                    dirty = true;
                    platform.send(Request::Refresh);
                }
                sdl::KEYDOWN => match map_key(event.key_sym()) {
                    Some(Key::Ok) => {
                        if !event.key_repeat() && ok_down.is_none() {
                            ok_down = Some(Instant::now());
                            long_fired = false;
                        }
                    }
                    Some(key) => {
                        if apply(&mut home, &io, key) {
                            break 'main;
                        }
                        dirty = true;
                    }
                    None => {}
                },
                // Magic Remote: the pointer focuses what it is over; its button acts as OK
                // (held = long press), the wheel steps along the row.
                sdl::MOUSEMOTION => {
                    let (x, y) = event.mouse_xy();
                    let (lx, ly) = to_logical(&sdl, window, x, y);
                    if home.pointer(lx, ly) {
                        dirty = true;
                    }
                }
                sdl::MOUSEBUTTONDOWN if event.mouse_button() == 1 => {
                    let (x, y) = event.mouse_xy();
                    let (lx, ly) = to_logical(&sdl, window, x, y);
                    home.pointer(lx, ly);
                    if home.pointer_target(lx, ly) && ok_down.is_none() {
                        ok_down = Some(Instant::now());
                        long_fired = false;
                    }
                    dirty = true;
                }
                sdl::MOUSEBUTTONUP if event.mouse_button() == 1 => {
                    if ok_down.take().is_some() && !long_fired && apply(&mut home, &io, Key::Ok) {
                        break 'main;
                    }
                    dirty = true;
                }
                sdl::MOUSEWHEEL => {
                    let key = if event.wheel_y() > 0 { Key::Left } else { Key::Right };
                    if apply(&mut home, &io, key) {
                        break 'main;
                    }
                    dirty = true;
                }
                sdl::KEYUP if map_key(event.key_sym()) == Some(Key::Ok) => {
                    if ok_down.take().is_some() && !long_fired && apply(&mut home, &io, Key::Ok) {
                        break 'main;
                    }
                    dirty = true;
                }
                sdl::WINDOWEVENT => match event.window_event() {
                    sdl::WINDOWEVENT_SHOWN
                    | sdl::WINDOWEVENT_EXPOSED
                    | sdl::WINDOWEVENT_SIZE_CHANGED
                    | sdl::WINDOWEVENT_RESTORED
                    | sdl::WINDOWEVENT_FOCUS_GAINED => {
                        if !visible {
                            services.send(ServiceRequest::Visible(true));
                        }
                        visible = true;
                        dirty = true;
                    }
                    sdl::WINDOWEVENT_FOCUS_LOST => {
                        services.send(ServiceRequest::Visible(false));
                        visible = false;
                        animating = false;
                        home.settle();
                    }
                    _ => {}
                },
                sdl::APP_DIDENTERFOREGROUND => {
                    visible = true;
                    dirty = true;
                    platform.send(Request::Refresh);
                }
                sdl::APP_DIDENTERBACKGROUND => {
                    services.send(ServiceRequest::Visible(false));
                    visible = false;
                    animating = false;
                    home.settle();
                }
                _ => {}
            }
            got = unsafe { (sdl.poll_event)(&mut event) };
        }
        if let (Some(down), false) = (ok_down, long_fired) {
            if down.elapsed().as_millis() >= LONG_PRESS_MS {
                long_fired = true;
                apply(&mut home, &io, Key::LongOk);
                dirty = true;
            }
        }
        while let Ok(update) = platform.updates.try_recv() {
            match update {
                Update::Apps(apps) => home.set_apps(apps),
                Update::Inputs(inputs) => home.set_inputs(inputs),
                Update::Icon(icon) => home.add_icon(icon, &renderer),
                Update::Art(art) => home.add_art(art, &renderer),
                Update::Notice(message) => home.notice(message),
                Update::Weather(weather) => home.set_weather(weather),
                Update::Entities(entities) => home.set_entities(entities),
                Update::HaCatalog { entities, selected } => home.set_catalog(entities, selected),
                Update::Media(items) => home.set_media(items),
                Update::Status(status) => home.set_status(status),
                Update::Tv(feed) => home.set_tv(feed),
                Update::TvPair(key) => platform.send(Request::Launch {
                    app_id: model::TIVIONE_APP.into(),
                    params: Some(serde_json::json!({ "launcherPair": { "key": key } })),
                }),
            }
            dirty = true;
        }

        let now = chrono::Local::now();
        if now.minute() != minute {
            minute = now.minute();
            home.tick();
            dirty = true;
        }
        // Scripted keys (previews/benchmarks): one every 12 frames once data has arrived.
        if home.loaded && frames >= 20 && frames % 12 == 0 {
            if let Some(key) = options.keys.get(((frames - 20) / 12) as usize) {
                apply(&mut home, &io, *key);
                dirty = true;
            }
        }
        if !visible || (!dirty && !animating && options.frames.is_none()) {
            continue;
        }

        let frame_start = Instant::now();
        let dt = frame_start.duration_since(last_frame).as_secs_f32().min(0.05);
        last_frame = frame_start;
        animating = home.update(dt);
        if let Some(action) = home.take_action() {
            dispatch(action, &io);
        }
        dirty = false;

        let (mut w, mut h): (c_int, c_int) = (0, 0);
        unsafe { (sdl.gl_get_drawable_size)(window, &mut w, &mut h) };
        let before_begin = Instant::now();
        renderer.begin((w, h), theme::BG);
        let began = Instant::now();
        let clock = format!("{:02}:{:02}", now.hour(), now.minute());
        home.draw(&mut renderer, &mut text, &clock, &format_date(&now, strings::current()));
        renderer.end();
        let submitted = Instant::now();
        let last = options.frames.is_some_and(|n| frames + 1 >= n);
        if last {
            if let Some(path) = &options.screenshot {
                save_screenshot(&renderer, (w, h), path)?;
            }
        }
        unsafe { (sdl.gl_swap_window)(window) };
        let swapped = Instant::now();

        if first_frame {
            first_frame = false;
            log!("first frame: {} ms after start", started.elapsed().as_millis());
        }
        if options.perf {
            perf.record(&renderer, [frame_start, before_begin, began, submitted, swapped], text.rasterised);
        }
        frames += 1;
        if options.frames.is_some_and(|n| frames >= n) {
            break;
        }
    }

    unsafe {
        (sdl.gl_delete_context)(context);
        (sdl.destroy_window)(window);
        (sdl.quit)();
    }
    Ok(())
}

/// Where actions go: TV services (Luna) and network services.
struct Io<'a> {
    platform: &'a Platform,
    services: &'a Services,
    home_mode: bool,
}

/// Feeds a key to the home screen and dispatches the resulting action. Returns true to quit.
fn apply(home: &mut Home, io: &Io, key: Key) -> bool {
    match home.key(key, io.home_mode) {
        Some(action) => dispatch(action, io),
        None => false,
    }
}

fn dispatch(action: Action, io: &Io) -> bool {
    let platform = io.platform;
    match Some(action) {
        Some(Action::Launch { app_id, params }) => {
            log!("launch {app_id}");
            platform.send(Request::Launch { app_id, params });
        }
        Some(Action::SavePrefs(prefs)) => platform.send(Request::SavePrefs(prefs)),
        Some(Action::Service(request)) => io.services.send(request),
        // On the TV, Back on the first tile hands over to LG's Home and Vision stays resident,
        // so the next launch is a warm relaunch instead of a cold start.
        Some(Action::Exit) if WEBOS => {
            platform.send(Request::Launch { app_id: "com.webos.app.home".into(), params: None })
        }
        Some(Action::Exit) => return true,
        None => {}
    }
    false
}

/// Registers with SAM as LG's Home (lifecycle v2): "relaunch" on the Home key, "close" to exit.
#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn register_home(waker: sdl::Waker) -> Result<(), String> {
    let sink = Box::new(move |message: serde_json::Value| {
        log!("lifecycle: {message}");
        let event = message.get("event").or_else(|| message.get("message")).and_then(|v| v.as_str());
        match event {
            Some("relaunch") => waker.lifecycle(1),
            Some("close") => waker.lifecycle(2),
            // Registration lost (SAM restarted): quit so SAM starts Home afresh.
            _ if message.get("returnValue").and_then(|v| v.as_bool()) == Some(false) => waker.lifecycle(2),
            _ => {}
        }
    });
    luna::subscribe(Some(HOME_ID), "luna://com.webos.applicationManager/registerApp", &serde_json::json!({ "id": HOME_ID }), sink)
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn register_home(_waker: sdl::Waker) -> Result<(), String> {
    Ok(())
}

/// Brings the Home instance forward (used when Vision's own tile is opened while it is Home).
#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn open_home() {
    match luna::Luna::connect() {
        Ok(mut luna) => {
            if let Err(error) = luna.call("luna://com.webos.applicationManager/launch", &serde_json::json!({ "id": HOME_ID })) {
                log!("open home: {error}");
            }
        }
        Err(error) => log!("open home: {error}"),
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn open_home() {}

/// Window coordinates to the 1920×1080 layout space.
fn to_logical(sdl: &Sdl, window: sdl::Window, x: i32, y: i32) -> (f32, f32) {
    let (mut w, mut h): (c_int, c_int) = (0, 0);
    unsafe { (sdl.get_window_size)(window, &mut w, &mut h) };
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    (x as f32 * render::LOGICAL_W / w, y as f32 * render::LOGICAL_H / h)
}

fn create_window(sdl: &Sdl) -> Result<(sdl::Window, sdl::GlContext, bool), String> {
    let title = CString::new("Vision").unwrap();
    let attempts: &[(c_int, c_int, c_int)] = if WEBOS {
        &[(sdl::GL_CONTEXT_PROFILE_ES, 2, 0)]
    } else {
        &[(sdl::GL_CONTEXT_PROFILE_ES, 2, 0), (sdl::GL_CONTEXT_PROFILE_COMPATIBILITY, 2, 1)]
    };
    let mut last_error = String::new();
    for &(profile, major, minor) in attempts {
        // SAFETY: attribute setup before window creation on the main thread.
        unsafe {
            for (attribute, value) in [
                (sdl::GL_CONTEXT_PROFILE_MASK, profile),
                (sdl::GL_CONTEXT_MAJOR_VERSION, major),
                (sdl::GL_CONTEXT_MINOR_VERSION, minor),
                (sdl::GL_RED_SIZE, 8),
                (sdl::GL_GREEN_SIZE, 8),
                (sdl::GL_BLUE_SIZE, 8),
                (sdl::GL_ALPHA_SIZE, 8),
                (sdl::GL_DEPTH_SIZE, 0),
                (sdl::GL_STENCIL_SIZE, 0),
                (sdl::GL_DOUBLEBUFFER, 1),
            ] {
                (sdl.gl_set_attribute)(attribute, value);
            }
            let (w, h, flags) = if WEBOS {
                (1920, 1080, sdl::WINDOW_OPENGL | sdl::WINDOW_FULLSCREEN | sdl::WINDOW_SHOWN)
            } else {
                (1280, 720, sdl::WINDOW_OPENGL | sdl::WINDOW_RESIZABLE | sdl::WINDOW_SHOWN | sdl::WINDOW_ALLOW_HIGHDPI)
            };
            let window = (sdl.create_window)(title.as_ptr(), sdl::WINDOWPOS_CENTERED, sdl::WINDOWPOS_CENTERED, w, h, flags);
            if window.is_null() {
                last_error = format!("SDL_CreateWindow: {}", sdl.error());
                continue;
            }
            let context = (sdl.gl_create_context)(window);
            if context.is_null() {
                last_error = format!("SDL_GL_CreateContext: {}", sdl.error());
                (sdl.destroy_window)(window);
                continue;
            }
            return Ok((window, context, profile == sdl::GL_CONTEXT_PROFILE_ES));
        }
    }
    Err(last_error)
}

fn sdl_candidates(exe_dir: &Path) -> Vec<PathBuf> {
    if WEBOS {
        vec![exe_dir.join("lib/libSDL2-2.0.so.0"), PathBuf::from("libSDL2-2.0.so.0")]
    } else {
        vec![
            exe_dir.join("SDL2.dll"),
            PathBuf::from("third_party/sdl-windows/SDL2.dll"),
            PathBuf::from("SDL2.dll"),
            PathBuf::from("libSDL2-2.0.so.0"),
        ]
    }
}

fn find_assets(exe_dir: &Path) -> Result<PathBuf, String> {
    [exe_dir.join("assets"), PathBuf::from("assets")]
        .into_iter()
        .find(|p| p.join("fonts").is_dir())
        .ok_or_else(|| "assets/fonts not found".to_string())
}

fn map_key(sym: i32) -> Option<Key> {
    Some(match sym {
        sdl::KEY_LEFT => Key::Left,
        sdl::KEY_RIGHT => Key::Right,
        sdl::KEY_UP => Key::Up,
        sdl::KEY_DOWN => Key::Down,
        sdl::KEY_RETURN | sdl::KEY_KP_ENTER => Key::Ok,
        sdl::KEY_ESCAPE | sdl::KEY_BACKSPACE | sdl::KEY_AC_BACK | sdl::KEY_WEBOS_BACK => Key::Back,
        48..=57 => Key::Digit((sym - 48) as u8),
        sdl::KEY_KP_1..=sdl::KEY_KP_9 => Key::Digit((sym - sdl::KEY_KP_1 + 1) as u8),
        sdl::KEY_KP_0 => Key::Digit(0),
        _ => return None,
    })
}

fn parse_key(name: &str) -> Option<Key> {
    Some(match name.trim() {
        "left" => Key::Left,
        "right" => Key::Right,
        "up" => Key::Up,
        "down" => Key::Down,
        "ok" => Key::Ok,
        "back" => Key::Back,
        "hold" => Key::LongOk,
        _ => return None,
    })
}

fn ms_to_next_minute() -> c_int {
    let now = chrono::Local::now();
    let ms = 60_000 - (now.second() * 1000 + now.timestamp_subsec_millis().min(999));
    ms.clamp(50, 60_000) as c_int
}

fn format_date(now: &chrono::DateTime<chrono::Local>, s: &strings::Strings) -> String {
    format!(
        "{}, {} {}",
        s.days[now.weekday().num_days_from_monday() as usize],
        now.day(),
        s.months[now.month0() as usize]
    )
}

fn save_screenshot(renderer: &Renderer, (w, h): (c_int, c_int), path: &Path) -> Result<(), String> {
    let mut pixels = vec![0u8; (w * h * 4) as usize];
    // SAFETY: the buffer holds w*h RGBA texels.
    unsafe { (renderer.gl.read_pixels)(0, 0, w, h, gl::RGBA, gl::UNSIGNED_BYTE, pixels.as_mut_ptr().cast()) };
    let row = (w * 4) as usize;
    let flipped: Vec<u8> = pixels.chunks_exact(row).rev().flatten().copied().collect();
    let file = std::fs::File::create(path).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w as u32, h as u32);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    writer.write_image_data(&flipped).map_err(|e| e.to_string())?;
    log!("screenshot: {}", path.display());
    Ok(())
}

/// Frame timing. `wait` is time blocked in the first GL call until the driver frees a back
/// buffer (vsync back-pressure on Mali); `work` is the real per-frame CPU cost.
#[derive(Default)]
struct Perf {
    frames: u32,
    wait: f32,
    work: f32,
    work_max: f32,
    swap: f32,
    since: Option<Instant>,
}

impl Perf {
    fn record(&mut self, renderer: &Renderer, [start, before_begin, began, submitted, swapped]: [Instant; 5], glyphs: u32) {
        let ms = |a: Instant, b: Instant| b.duration_since(a).as_secs_f32() * 1000.0;
        let work = ms(start, before_begin) + ms(began, submitted);
        let wait = ms(before_begin, began);
        self.frames += 1;
        self.wait += wait;
        self.work += work;
        self.work_max = self.work_max.max(work);
        self.swap += ms(submitted, swapped);
        let since = *self.since.get_or_insert(start);
        let elapsed = since.elapsed().as_secs_f32();
        if elapsed >= 2.0 {
            let n = self.frames as f32;
            log!(
                "perf: {:.1} fps, work avg {:.2} ms max {:.2} ms, vsync wait {:.2} ms, swap {:.2} ms, {} draws / {} quads, {} glyphs",
                n / elapsed,
                self.work / n,
                self.work_max,
                self.wait / n,
                self.swap / n,
                renderer.stats.draw_calls,
                renderer.stats.quads,
                glyphs
            );
            *self = Perf::default();
        }
    }
}
