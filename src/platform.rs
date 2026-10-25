//! TV services on a worker thread. The UI thread only sends requests and drains updates,
//! so Luna round trips, file reads and PNG decoding never cost a frame.
use crate::log::log;
use crate::model::{Request, Update};
use crate::sdl::Waker;
use std::sync::mpsc::{self, Receiver, Sender};

pub const APP_ID: &str = "com.vision.launcher";

pub struct Platform {
    requests: Sender<Request>,
    pub updates: Receiver<Update>,
}

impl Platform {
    pub fn start(waker: Waker) -> Self {
        let (requests, request_rx) = mpsc::channel();
        let (update_tx, updates) = mpsc::channel();
        let sink = Sink { tx: update_tx, waker };
        std::thread::Builder::new()
            .name("platform".into())
            .spawn(move || backend::run(request_rx, sink))
            .expect("platform thread");
        Self { requests, updates }
    }

    pub fn send(&self, request: Request) {
        let _ = self.requests.send(request);
    }
}

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

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
mod backend {
    //! Desktop preview: fixed sample inventory, never touches TV services.
    use super::*;
    use crate::model::{App, Input};

    pub fn run(requests: Receiver<Request>, sink: Sink) {
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
                        ("settings", "Einstellungen", "#9AA3AE"),
                    ];
                    sink.send(Update::Apps(
                        apps.iter()
                            .map(|(id, title, color)| App {
                                id: id.to_string(),
                                title: title.to_string(),
                                icon_path: None,
                                bg_color: Some(color.to_string()),
                                params: None,
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
                    sink.send(Update::Notice(format!("Vorschau: {app_id} würde starten")));
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "arm"))]
mod backend {
    use super::*;
    use crate::luna::Luna;
    use crate::model::{self, App};
    use base64::Engine;
    use serde_json::{json, Value};

    /// Worker-side state: which icons the UI already holds, and refresh throttling.
    struct State {
        icons_sent: std::collections::HashSet<String>,
        last_refresh: Option<std::time::Instant>,
    }

    pub fn run(requests: Receiver<Request>, sink: Sink) {
        let mut state = State { icons_sent: Default::default(), last_refresh: None };
        let mut luna = match Luna::connect() {
            Ok(luna) => luna,
            Err(error) => {
                log!("luna: {error}");
                sink.send(Update::Notice("Keine Verbindung zum TV-Dienst".into()));
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
                    state.last_refresh = Some(std::time::Instant::now());
                }
                Request::Launch { app_id, params } => launch(&mut luna, &sink, &app_id, params),
            }
        }
    }

    /// Reads system inventory directly when this process may, else through Homebrew
    /// Channel's root `exec` running luna-send (dev apps lack the query ACGs).
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
        let started = std::time::Instant::now();
        let apps = match inventory(luna, "luna://com.webos.applicationManager/listLaunchPoints") {
            Ok(reply) => model::apps_from_launch_points(&reply, APP_ID),
            Err(error) => {
                log!("launch points: {error}");
                sink.send(Update::Notice("App-Liste nicht verfügbar".into()));
                Vec::new()
            }
        };
        log!("inventory: {} apps in {} ms", apps.len(), started.elapsed().as_millis());
        sink.send(Update::Apps(apps.clone()));

        match inventory(luna, "luna://com.webos.service.eim/getAllInputStatus") {
            Ok(reply) => sink.send(Update::Inputs(model::inputs_from_status(&reply))),
            Err(error) => log!("inputs: {error}"),
        }
        let missing: Vec<App> = apps.into_iter().filter(|app| !state.icons_sent.contains(&app.id)).collect();
        load_icons(luna, sink, &missing);
        state.icons_sent.extend(missing.into_iter().map(|app| app.id));
        log!("inventory + icons: {} ms", started.elapsed().as_millis());
    }

    fn load_icons(luna: &mut Luna, sink: &Sink, apps: &[App]) {
        let mut blocked = Vec::new();
        for app in apps {
            let Some(path) = &app.icon_path else { continue };
            match std::fs::read(path) {
                Ok(bytes) => emit_icon(sink, &app.id, &bytes),
                Err(_) => blocked.push((app.id.clone(), path.clone())),
            }
        }
        // The native jail hides most app directories; read the rest in root batches.
        // One root exec costs ~1 s regardless of size, so batch generously.
        for chunk in blocked.chunks(32) {
            let script: String = chunk
                .iter()
                .map(|(_, path)| format!("echo '@@'; base64 '{}' 2>/dev/null | tr -d '\\n'; echo;", path.replace('\'', "")))
                .collect();
            let Ok(stdout) = exec(luna, &script) else { continue };
            let mut blocks = stdout.split("@@").skip(1);
            for (app_id, _) in chunk {
                let Some(block) = blocks.next() else { break };
                if let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(block.trim()) {
                    emit_icon(sink, app_id, &bytes);
                }
            }
        }
    }

    fn emit_icon(sink: &Sink, app_id: &str, bytes: &[u8]) {
        match model::decode_icon(app_id, bytes) {
            Ok(icon) => sink.send(Update::Icon(icon)),
            Err(error) => log!("icon {app_id}: {error}"),
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
            sink.send(Update::Notice("App konnte nicht gestartet werden".into()));
        }
    }
}
