//! TV services on a worker thread. The UI thread only sends requests and drains updates,
//! so Luna round trips, file I/O and PNG decoding never cost a frame.
use crate::log::log;
use crate::model::{Request, Update};
use crate::sdl::Waker;
use crate::storage;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};

pub const APP_ID: &str = "com.vision.launcher";

pub struct Platform {
    requests: Sender<Request>,
    sink: Sink,
    pub updates: Receiver<Update>,
}

impl Platform {
    pub fn start(waker: Waker, data_dir: PathBuf) -> Self {
        let (requests, request_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let sink = Sink { tx: update_tx, waker };
        let worker_sink = sink.clone();
        std::thread::Builder::new()
            .name("platform".into())
            .spawn(move || backend::run(request_rx, worker_sink, data_dir))
            .expect("platform thread");
        Self { requests, sink, updates }
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }

    /// Another producer for the same update queue (the network services thread).
    pub fn sink(&self) -> Sink {
        self.sink.clone()
    }
}

#[derive(Clone)]
pub struct Sink {
    tx: Sender<Update>,
    waker: Waker,
}

impl Sink {
    pub fn send(&self, update: Update) {
        if self.tx.send(update).is_ok() {
            self.waker.wake();
        }
    }
}

/// Decodes a wallpaper on a thread of its own (a 1080p JPEG takes a while on the TV).
fn load_wallpaper(path: PathBuf, sink: Sink) {
    let _ = std::thread::Builder::new().name("wallpaper".into()).spawn(move || {
        let image = std::fs::read(&path).map_err(|e| e.to_string()).and_then(|bytes| crate::model::decode_wallpaper(&bytes));
        if let Err(error) = &image {
            log!("wallpaper {}: {error}", path.display());
        }
        sink.send(Update::Wallpaper(image.ok()));
    });
}

fn save_prefs(prefs: &storage::Prefs, data_dir: &std::path::Path) {
    if let Err(error) = prefs.save(data_dir) {
        log!("prefs: {error}");
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
mod backend {
    //! Desktop preview: fixed sample inventory, never touches TV services.
    use super::*;
    use crate::model::{App, Input, MediaItem, Notice, Programme, TvCatchup, TvChannel, TvFeed, TvReminder};

    /// Plex "continue watching" for the preview.
    fn sample_media() -> Vec<MediaItem> {
        [("Spring", "Short film", 0.35, 6), ("Sintel", "Open movie", 0.6, 6), ("Tears of Steel", "Open movie", 0.2, 10)]
            .iter()
            .enumerate()
            .map(|(i, (title, subtitle, progress, left))| MediaItem {
                id: format!("sample{i}"),
                title: title.to_string(),
                subtitle: subtitle.to_string(),
                progress: *progress,
                remaining_min: *left,
            })
            .collect()
    }

    /// A TiviOne snapshot with neutral demo channels, timed around the (possibly fixed) clock.
    fn sample_tv() -> TvFeed {
        let now = crate::clock::utc_ms();
        let hour = now - now.rem_euclid(1_800_000);
        let names = ["Animation One", "Sci-Fi Channel", "Fantasy HD", "Open Cinema", "Family TV", "Future Lab"];
        let shows = ["Big Buck Bunny", "Tears of Steel", "Legends & Myths", "Sintel", "Animal Friends", "Science Hour"];
        let mut feed = TvFeed { playlist_id: 1, ..TvFeed::default() };
        for (i, (name, show)) in names.iter().zip(shows).enumerate() {
            let id = i as i64 + 1;
            feed.channels.insert(id, TvChannel { name: name.to_string(), number: Some(id as u32) });
            feed.numbers.insert(id as u32, id);
            feed.guide.insert(
                id,
                vec![
                    Programme { title: show.to_string(), start: hour, end: hour + 1_800_000 },
                    Programme { title: "Coming up next".into(), start: hour + 1_800_000, end: hour + 3_600_000 },
                ],
            );
        }
        feed.recent = vec![1, 2, 3, 4, 5, 6];
        feed.favorites = vec![2, 4];
        feed.catchup = vec![TvCatchup { channel: 4, title: "Elephants Dream".into(), start: hour - 3_600_000, end: hour - 1_800_000 }];
        feed.reminders = vec![TvReminder { channel: 3, title: "Cosmos Laundromat".into(), start: hour + 3_600_000 }];
        feed
    }

    pub fn run(requests: Receiver<Request>, sink: Sink, data_dir: PathBuf) {
        let mut release_checked = false;
        for request in requests {
            match request {
                Request::Refresh => {
                    // Desktop: TiviOne is never installed, so the store page shows; the real release
                    // manifest (or --releases) feeds it.
                    sink.send(Update::TiviOne(None));
                    if !std::mem::replace(&mut release_checked, true) {
                        crate::installer::check(sink.clone(), Default::default());
                    }
                    let apps = [
                        ("plex", "Plex", "#E5A00D"),
                        ("netflix", "Netflix", "#E50914"),
                        ("tivione", "TiviOne", "#2F6BFF"),
                        ("moonlight", "Moonlight", "#7A5CFF"),
                        ("browser", "Browser", "#21A5B8"),
                        ("piccap", "PicCap", "#8E8E99"),
                        ("screenview", "Screen Viewer", "#3FB37F"),
                        ("hbchannel", "Homebrew Channel", "#C0392B"),
                        ("settings", "Settings", "#9AA3AE"),
                    ];
                    sink.send(Update::Apps(
                        apps.iter()
                            .map(|(id, title, color)| App {
                                id: id.to_string(),
                                title: title.to_string(),
                                icon_path: None,
                                bg_color: Some(color.to_string()),
                                params: None,
                                art_path: None,
                                art_is_splash: false,
                            })
                            .collect(),
                    ));
                    sink.send(Update::Inputs(
                        [("HDMI_1", "HDMI 1", true), ("HDMI_2", "HDMI 2", true), ("HDMI_3", "HDMI 3", false), ("HDMI_4", "HDMI 4", false)]
                            .iter()
                            .map(|(id, label, connected)| Input {
                                id: id.to_string(),
                                label: label.to_string(),
                                app_id: format!("com.webos.app.{}", id.to_lowercase().replace('_', "")),
                                connected: *connected,
                            })
                            .collect(),
                    ));
                    sink.send(Update::Media(sample_media()));
                    sink.send(Update::Tv(Some(sample_tv())));
                }
                Request::Launch { app_id, .. } => {
                    log!("preview: would launch {app_id}");
                    sink.send(Update::Notice(Notice::PreviewLaunch(app_id)));
                }
                Request::HomeTakeover(on) => log!("preview: home takeover {on}"),
                Request::RefreshImages => log!("preview: refresh images"),
                Request::SavePrefs(prefs) => save_prefs(&prefs, &data_dir),
                Request::Install(release) => crate::installer::install(release, data_dir.clone(), sink.clone()),
                Request::Pictures(release) => crate::installer::pictures(release, data_dir.clone(), sink.clone()),
                Request::Wallpaper(path) => load_wallpaper(path, sink.clone()),
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "arm"))]
mod backend {
    use super::*;
    use crate::luna::Luna;
    use crate::model::{self, App, Notice};
    use serde_json::{json, Value};
    use std::collections::HashSet;
    use std::path::Path;
    use std::time::Instant;

    /// Worker-side state: which apps' images the UI already holds, and refresh throttling.
    struct State {
        data_dir: PathBuf,
        images_sent: HashSet<String>,
        last_refresh: Option<Instant>,
        images_busy: std::sync::Arc<std::sync::atomic::AtomicBool>,
        /// When TiviOne's website was last asked for a new release, and whether it answered.
        release_checked: Option<Instant>,
        release_answered: std::sync::Arc<std::sync::atomic::AtomicBool>,
    }

    pub fn run(requests: Receiver<Request>, sink: Sink, data_dir: PathBuf) {
        let mut state = State {
            data_dir,
            images_sent: HashSet::new(),
            last_refresh: None,
            images_busy: Default::default(),
            release_checked: None,
            release_answered: Default::default(),
        };
        let mut luna = match Luna::connect() {
            Ok(luna) => luna,
            Err(error) => {
                log!("luna: {error}");
                sink.send(Update::Notice(Notice::ServiceUnavailable));
                return;
            }
        };
        for request in requests {
            match request {
                Request::Refresh => {
                    // Startup and the first foreground event arrive together; one pass is enough.
                    if state.last_refresh.is_some_and(|t| t.elapsed().as_secs() < 3) {
                        continue;
                    }
                    refresh(&mut luna, &sink, &mut state);
                    state.last_refresh = Some(Instant::now());
                }
                Request::Launch { app_id, params } => launch(&mut luna, &sink, &app_id, params),
                Request::SavePrefs(prefs) => save_prefs(&prefs, &state.data_dir),
                Request::Install(release) => crate::installer::install(release, state.data_dir.clone(), sink.clone()),
                Request::Pictures(release) => crate::installer::pictures(release, state.data_dir.clone(), sink.clone()),
                Request::Wallpaper(path) => load_wallpaper(path, sink.clone()),
                Request::HomeTakeover(on) => {
                    // As root through Homebrew Channel; while Vision is Home, disabling ends
                    // this very process (LG Home takes over), enabling restarts it as Home.
                    let command = if on {
                        "sh /media/developer/apps/usr/palm/applications/com.vision.launcher/home.sh enable"
                    } else {
                        "sh /var/lib/vision-home/home.sh disable"
                    };
                    match exec(&mut luna, command) {
                        Ok(output) => log!("home takeover {on}: {}", output.trim()),
                        Err(error) => log!("home takeover {on}: {error}"),
                    }
                }
                Request::RefreshImages => {
                    for kind in ["icons", "art"] {
                        let _ = std::fs::remove_dir_all(state.data_dir.join(kind));
                    }
                    state.images_sent.clear();
                    refresh(&mut luna, &sink, &mut state);
                    state.last_refresh = Some(Instant::now());
                }
            }
        }
    }

    /// Reads system inventory directly when this process may, else through Homebrew
    /// Channel's root `exec` running luna-send.
    fn inventory(luna: &mut Luna, uri: &str) -> Result<Value, String> {
        if let Ok(reply) = luna.call(uri, &json!({})) {
            if reply.get("returnValue").and_then(Value::as_bool) == Some(true) {
                return Ok(reply);
            }
        }
        let stdout = exec(luna, &format!("luna-send -n 1 -w 4000 {uri} '{{}}'"))?;
        serde_json::from_str(stdout.trim()).map_err(|e| format!("{uri}: {e}"))
    }

    fn exec(luna: &mut Luna, command: &str) -> Result<String, String> {
        let reply = luna.call("luna://org.webosbrew.hbchannel.service/exec", &json!({ "command": command }))?;
        if reply.get("returnValue").and_then(Value::as_bool) != Some(true) {
            return Err(format!("hbchannel exec: {reply}"));
        }
        Ok(reply.get("stdoutString").and_then(Value::as_str).unwrap_or("").to_string())
    }

    fn refresh(luna: &mut Luna, sink: &Sink, state: &mut State) {
        let started = Instant::now();
        let all_apps = inventory(luna, "luna://com.webos.applicationManager/listApps").ok();
        let apps = match inventory(luna, "luna://com.webos.applicationManager/listLaunchPoints") {
            Ok(reply) => model::apps_from_launch_points(&reply, all_apps.as_ref(), APP_ID),
            Err(error) => {
                log!("launch points: {error}");
                sink.send(Update::Notice(Notice::AppListUnavailable));
                Vec::new()
            }
        };
        log!("inventory: {} apps in {} ms", apps.len(), started.elapsed().as_millis());
        sink.send(Update::Apps(apps.clone()));
        if let Some(all_apps) = &all_apps {
            sink.send(Update::TiviOne(model::tivione_from_apps(all_apps)));
        }
        // New TiviOne releases on a foreground refresh: every six hours, or after 15 minutes
        // when the last check got no answer.
        let interval = if state.release_answered.load(std::sync::atomic::Ordering::Acquire) { 6 * 3600 } else { 15 * 60 };
        if state.release_checked.is_none_or(|t| t.elapsed().as_secs() > interval) {
            state.release_checked = Some(Instant::now());
            crate::installer::check(sink.clone(), state.release_answered.clone());
        }

        match inventory(luna, "luna://com.webos.service.eim/getAllInputStatus") {
            Ok(reply) => sink.send(Update::Inputs(model::inputs_from_status(&reply))),
            Err(error) => log!("inputs: {error}"),
        }
        log!("refresh: {} ms", started.elapsed().as_millis());

        // Images load on their own thread (first-time decoding of 1080p splashes takes seconds),
        // so app launches never queue behind them.
        use std::sync::atomic::Ordering;
        if state.images_busy.swap(true, Ordering::AcqRel) {
            return;
        }
        let missing: Vec<App> = apps.into_iter().filter(|app| !state.images_sent.contains(&app.id)).collect();
        state.images_sent.extend(missing.iter().map(|app| app.id.clone()));
        let icons: Vec<Job> = missing.iter().filter_map(|a| a.icon_path.clone().map(|p| Job::icon(&a.id, p))).collect();
        let art: Vec<Job> = missing
            .iter()
            .filter_map(|a| a.art_path.clone().map(|p| Job::art(&a.id, p, a.art_is_splash)))
            .collect();
        let (sink, data_dir, busy) = (sink.clone(), state.data_dir.clone(), state.images_busy.clone());
        let _ = std::thread::Builder::new().name("images".into()).spawn(move || {
            let mut luna = None;
            load_images(&mut luna, &sink, &icons, &data_dir);
            load_images(&mut luna, &sink, &art, &data_dir);
            busy.store(false, Ordering::Release);
        });
    }

    /// One image to load: an app icon or its tile artwork.
    struct Job {
        app_id: String,
        source: String,
        kind: &'static str,
        splash: bool,
    }

    impl Job {
        fn icon(app_id: &str, source: String) -> Self {
            Self { app_id: app_id.to_string(), source, kind: "icons", splash: false }
        }

        fn art(app_id: &str, source: String, splash: bool) -> Self {
            Self { app_id: app_id.to_string(), source, kind: "art", splash }
        }

        fn decode(&self, bytes: &[u8]) -> Result<model::Icon, String> {
            if self.kind == "art" {
                model::decode_art(&self.app_id, bytes, self.splash)
            } else {
                model::decode_icon(&self.app_id, bytes)
            }
        }

        fn update(&self, image: model::Icon) -> Update {
            if self.kind == "art" { Update::Art(image) } else { Update::Icon(image) }
        }
    }

    /// Disk cache first, then direct reads. Files the native jail hides are copied by one
    /// root `exec` into our own import directory and read from there.
    fn load_images(luna: &mut Option<Luna>, sink: &Sink, jobs: &[Job], data_dir: &Path) {
        let started = Instant::now();
        let mut blocked = Vec::new();
        let mut cached = 0;
        for job in jobs {
            if crate::storage::is_unusable(data_dir, job.kind, &job.app_id, &job.source) {
                cached += 1;
                continue;
            }
            if let Some(image) = crate::storage::load_icon(data_dir, job.kind, &job.app_id, &job.source) {
                sink.send(job.update(image));
                cached += 1;
                continue;
            }
            match std::fs::read(&job.source) {
                Ok(bytes) => emit(sink, job, &bytes, data_dir),
                Err(_) => blocked.push(job),
            }
        }
        if !blocked.is_empty() {
            let import = data_dir.join("import");
            if std::fs::create_dir_all(&import).is_ok() {
                let script: String = blocked
                    .iter()
                    .enumerate()
                    .map(|(i, job)| format!("cp '{}' '{}/{i}' 2>/dev/null; chmod 644 '{}/{i}' 2>/dev/null;", job.source.replace('\'', ""), import.display(), import.display()))
                    .collect();
                // Own LS2 connection, opened only when something must come through root.
                if luna.is_none() {
                    *luna = Luna::connect().map_err(|e| log!("images luna: {e}")).ok();
                }
                if let Some(luna) = luna.as_mut() {
                    if let Err(error) = exec(luna, &script) {
                        log!("image import: {error}");
                    }
                }
                for (i, job) in blocked.iter().enumerate() {
                    let file = import.join(i.to_string());
                    if let Ok(bytes) = std::fs::read(&file) {
                        emit(sink, job, &bytes, data_dir);
                    }
                    let _ = std::fs::remove_file(file);
                }
            }
        }
        if let Some(job) = jobs.first() {
            log!("{}: {} cached, {} decoded, {} via root, {} ms", job.kind, cached, jobs.len() - cached, blocked.len(), started.elapsed().as_millis());
        }
    }

    fn emit(sink: &Sink, job: &Job, bytes: &[u8], data_dir: &Path) {
        match job.decode(bytes) {
            Ok(image) => {
                if let Err(error) = crate::storage::save_icon(data_dir, job.kind, &image, &job.source) {
                    log!("{} cache {}: {error}", job.kind, job.app_id);
                }
                sink.send(job.update(image));
            }
            Err(error) => {
                log!("{} {}: {error}", job.kind, job.app_id);
                crate::storage::mark_unusable(data_dir, job.kind, &job.app_id, &job.source);
            }
        }
    }

    fn launch(luna: &mut Luna, sink: &Sink, app_id: &str, params: Option<Value>) {
        let mut payload = json!({ "id": app_id });
        if let Some(params) = params {
            payload["params"] = params;
        }
        let uri = "luna://com.webos.applicationManager/launch";
        match luna.call(uri, &payload) {
            Ok(reply) if reply.get("returnValue").and_then(Value::as_bool) == Some(true) => return,
            Ok(reply) => log!("launch {app_id}: {reply}"),
            Err(error) => log!("launch {app_id}: {error}"),
        }
        let quoted = payload.to_string().replace('\'', "");
        if let Err(error) = exec(luna, &format!("luna-send -n 1 {uri} '{quoted}'")) {
            log!("launch {app_id} via hbchannel: {error}");
            sink.send(Update::Notice(Notice::LaunchFailed));
        }
    }
}
