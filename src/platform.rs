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

fn save_prefs(prefs: &storage::Prefs, data_dir: &std::path::Path) {
    if let Err(error) = prefs.save(data_dir) {
        log!("prefs: {error}");
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
mod backend {
    //! Desktop preview: fixed sample inventory, never touches TV services.
    use super::*;
    use crate::model::{App, Input, Notice};

    pub fn run(requests: Receiver<Request>, sink: Sink, data_dir: PathBuf) {
        for request in requests {
            match request {
                Request::Refresh => {
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
                }
                Request::Launch { app_id, .. } => {
                    log!("preview: would launch {app_id}");
                    sink.send(Update::Notice(Notice::PreviewLaunch(app_id)));
                }
                Request::SavePrefs(prefs) => save_prefs(&prefs, &data_dir),
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
    }

    pub fn run(requests: Receiver<Request>, sink: Sink, data_dir: PathBuf) {
        let mut state = State {
            data_dir,
            images_sent: HashSet::new(),
            last_refresh: None,
            images_busy: Default::default(),
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
