//! Frees memory when it runs low. webOS keeps apps alive in the background (and preloads some,
//! such as Netflix and YouTube); with a large app in front the TV then swaps and everything
//! stutters. As Home, Vision watches the available memory and, below a threshold, closes the app
//! that has been in the background longest, one at a time. It never closes Vision, the app in
//! front, an app used in the last two minutes or a system app (Live TV, inputs, LG's home, voice,
//! AirPlay, Homebrew Channel).
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Below this much available memory (MemAvailable), Vision starts closing background apps…
const LOW_MB: u64 = 450;
/// …and stops once this much is available again.
const ENOUGH_MB: u64 = 550;
/// Apps used this recently stay open.
const MIN_IDLE: Duration = Duration::from_secs(120);

/// System apps stay open…
const KEEP_PREFIXES: &[&str] = &["com.webos.", "com.lge.", "com.palm.", "airplay", "org.webosbrew."];
/// …except these, which only hold memory.
const CLOSABLE_SYSTEM: &[&str] = &["com.webos.app.browser", "com.webos.app.livedmost"];

static ENABLED: AtomicBool = AtomicBool::new(true);
static USAGE: Mutex<Option<Usage>> = Mutex::new(None);

/// When each app last came to the front or left it, from SAM's app life events.
struct Usage {
    started: Instant,
    last: HashMap<String, Instant>,
}

/// The setting (Settings > System); on by default.
pub fn set_enabled(on: bool) {
    ENABLED.store(on, Ordering::Relaxed);
}

/// An event from `applicationManager/getAppLifeEvents`.
pub fn app_event(event: &str, app: &str) {
    if app.is_empty() {
        return;
    }
    let mut usage = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let usage = usage.get_or_insert_with(|| Usage { started: Instant::now(), last: HashMap::new() });
    match event {
        "launch" | "foreground" | "background" => {
            usage.last.insert(app.to_string(), Instant::now());
        }
        "close" | "stop" => {
            usage.last.remove(app);
        }
        _ => {}
    }
}

fn closable(app: &str) -> bool {
    crate::model::valid_app_id(app)
        && app != crate::platform::APP_ID
        && (CLOSABLE_SYSTEM.contains(&app) || !KEEP_PREFIXES.iter().any(|p| app.starts_with(p)))
}

/// The running app that has been in the background longest, if any may be closed. Apps not seen
/// in front since Vision started count from Vision's start (preloaded apps go first).
fn pick(running: &[String], front: Option<&str>, now: Instant) -> Option<String> {
    let usage = USAGE.lock().unwrap_or_else(|e| e.into_inner());
    let (started, last) = usage.as_ref().map_or((now, None), |u| (u.started, Some(&u.last)));
    running
        .iter()
        .filter(|app| closable(app) && Some(app.as_str()) != front)
        .map(|app| (app, last.and_then(|l| l.get(app)).copied().unwrap_or(started)))
        .filter(|(_, at)| now.duration_since(*at) >= MIN_IDLE)
        .min_by_key(|(_, at)| *at)
        .map(|(app, _)| app.clone())
}

/// MemAvailable in MB.
fn available_mb() -> Option<u64> {
    let info = std::fs::read_to_string("/proc/meminfo").ok()?;
    let line = info.lines().find(|l| l.starts_with("MemAvailable:"))?;
    Some(line.split_whitespace().nth(1)?.parse::<u64>().ok()? / 1024)
}

/// Starts the watcher thread (as Home only: it needs the app life events).
#[cfg(all(target_os = "linux", target_arch = "arm"))]
pub fn start() {
    use crate::log::log;
    use crate::luna::Luna;
    use serde_json::{json, Value};

    const POLL: Duration = Duration::from_secs(5);
    /// Time for the system to take back a closed app's memory before the next check.
    const SETTLE: Duration = Duration::from_secs(8);
    /// Nothing to close: look again much later.
    const NOTHING: Duration = Duration::from_secs(60);

    // Root luna-send through Homebrew Channel: Home's own connection may not close apps.
    fn exec(luna: &mut Luna, command: &str) -> Option<Value> {
        let reply = luna.call("luna://org.webosbrew.hbchannel.service/exec", &json!({ "command": command })).ok()?;
        serde_json::from_str(reply.get("stdoutString")?.as_str()?.trim()).ok()
    }

    let spawned = std::thread::Builder::new().name("memory".into()).spawn(move || {
        let mut luna = match Luna::connect() {
            Ok(luna) => luna,
            Err(error) => return log!("memory: {error}"),
        };
        // Between LOW_MB and ENOUGH_MB: keep closing once started.
        let mut freeing = false;
        loop {
            std::thread::sleep(POLL);
            let Some(available) = available_mb().filter(|_| ENABLED.load(Ordering::Relaxed)) else {
                freeing = false;
                continue;
            };
            if available >= ENOUGH_MB || (available >= LOW_MB && !freeing) {
                freeing = false;
                continue;
            }
            let front = exec(&mut luna, "luna-send -n 1 -w 4000 luna://com.webos.applicationManager/getForegroundAppInfo '{}'")
                .and_then(|v| v.get("appId").and_then(Value::as_str).map(str::to_string));
            let running: Vec<String> = exec(&mut luna, "luna-send -n 1 -w 4000 luna://com.webos.applicationManager/running '{}'")
                .and_then(|v| v.get("running").and_then(Value::as_array).cloned())
                .unwrap_or_default()
                .iter()
                .filter_map(|app| app.get("id").and_then(Value::as_str).map(str::to_string))
                .collect();
            let Some(app) = pick(&running, front.as_deref(), Instant::now()) else {
                freeing = false;
                std::thread::sleep(NOTHING);
                continue;
            };
            log!("memory: {available} MB available, closing {app}");
            freeing = true;
            // pick() only returns valid app ids: safe inside the shell command.
            exec(&mut luna, &format!("luna-send -n 1 -w 4000 luna://com.webos.applicationManager/closeByAppId '{{\"id\":\"{app}\"}}'"));
            std::thread::sleep(SETTLE);
        }
    });
    if let Err(error) = spawned {
        log!("memory: {error}");
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
pub fn start() {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn closes_the_oldest_closable_background_app() {
        let t0 = Instant::now();
        *USAGE.lock().unwrap() = Some(Usage { started: t0, last: HashMap::new() });
        app_event("background", "netflix");
        app_event("background", "com.tivione.tv");
        let later = Instant::now() + Duration::from_secs(600);
        let running: Vec<String> = ["com.webos.app.livetv", "airplay", "com.tivione.tv", "netflix", "youtube.leanback.v4", "com.vision.launcher"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        // YouTube was never in front since start: it goes first. System apps and Vision never.
        assert_eq!(pick(&running, Some("com.tivione.tv"), later).as_deref(), Some("youtube.leanback.v4"));
        let rest: Vec<String> = running.iter().filter(|a| *a != "youtube.leanback.v4").cloned().collect();
        assert_eq!(pick(&rest, None, later).as_deref(), Some("netflix"));
        // The app in front and recently used apps stay.
        assert_eq!(pick(&rest, Some("netflix"), later).as_deref(), Some("com.tivione.tv"));
        assert_eq!(pick(&rest, None, Instant::now()), None);
        assert!(closable("com.webos.app.browser") && !closable("com.webos.app.hdmi1") && !closable("org.webosbrew.hbchannel"));
    }
}
