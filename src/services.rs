//! Network services on their own thread: weather (Open-Meteo), Home Assistant and Plex.
//! Polling runs only while Vision is visible; every request has short timeouts.
use crate::companion::{self, Companion};
use crate::config::Config;
use crate::log::log;
use crate::model::{self, Entity, Forecast, HaCategory, MediaItem, ServiceRequest, Status, TvFeed, Update, Weather};
use std::collections::HashMap;
use base64::Engine;
use crate::net;
use crate::platform::Sink;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

const WEATHER_EVERY: Duration = Duration::from_secs(15 * 60);
const HA_EVERY: Duration = Duration::from_secs(60);
const PLEX_EVERY: Duration = Duration::from_secs(5 * 60);
const PIN_POLL: Duration = Duration::from_secs(2);
const AREAS_EVERY: Duration = Duration::from_secs(10 * 60);
/// Area of every selectable entity in one round trip (the REST states carry no areas).
const AREA_TEMPLATE: &str = "{% set ns = namespace(o=[]) %}{% for s in states if s.domain in ['light','switch','scene','script','group','fan','cover','media_player','input_boolean'] %}{% set ns.o = ns.o + [[s.entity_id, area_name(s.entity_id) or '']] %}{% endfor %}{{ ns.o | tojson }}";
const PLEX_TV: &str = "https://plex.tv/api/v2";
const TV_EVERY: Duration = Duration::from_secs(20);
/// Where the TiviOne service drops the sealed snapshot (shared /tmp of both jails).
const TV_FEED: &str = "/tmp/tivione/launcher-feed.json";

pub struct Services {
    requests: Sender<ServiceRequest>,
}

impl Services {
    pub fn start(sink: Sink, data_dir: PathBuf) -> Self {
        let (requests, rx) = mpsc::channel();
        let own = requests.clone();
        std::thread::Builder::new()
            .name("services".into())
            .spawn(move || Worker::new(sink, data_dir, own).run(rx))
            .expect("services thread");
        Self { requests }
    }

    pub fn send(&self, request: ServiceRequest) {
        let _ = self.requests.send(request);
    }
}

struct Pin {
    id: u64,
    expires: Instant,
    next_poll: Instant,
}

struct Worker {
    sink: Sink,
    data_dir: PathBuf,
    requests: Sender<ServiceRequest>,
    config: Config,
    agent: ureq::Agent,
    visible: bool,
    status: Status,
    ha_location: Option<(f64, f64)>,
    areas: HashMap<String, String>,
    areas_at: Option<Instant>,
    next_weather: Instant,
    next_ha: Instant,
    next_plex: Instant,
    pin: Option<Pin>,
    companion: Option<(Companion, String)>,
    media_art: HashSet<String>,
    next_tv: Instant,
    tv_seen: Option<std::time::SystemTime>,
}

impl Worker {
    fn new(sink: Sink, data_dir: PathBuf, requests: Sender<ServiceRequest>) -> Self {
        let config = Config::load(&data_dir);
        let now = Instant::now();
        let status = Status {
            ha: config.ha_url.clone().filter(|_| config.ha_token.is_some()),
            ha_selected: config.ha_entities.len(),
            plex: config.plex_token.as_ref().map(|_| config.plex_user.clone().unwrap_or_else(|| "Plex".into())),
            tivione: config.tivione,
            location: config.location_name.clone().filter(|_| config.location.is_some()),
            wallpaper_custom: data_dir.join("wallpaper.jpg").exists(),
            ..Status::default()
        };
        Self {
            sink,
            data_dir,
            requests,
            config,
            agent: net::agent(),
            visible: true,
            status,
            ha_location: None,
            areas: HashMap::new(),
            areas_at: None,
            next_weather: now,
            next_ha: now,
            next_plex: now,
            pin: None,
            companion: None,
            media_art: HashSet::new(),
            next_tv: now,
            tv_seen: None,
        }
    }

    fn run(mut self, rx: mpsc::Receiver<ServiceRequest>) {
        self.publish_status();
        // The last snapshot survives reboots (until TiviOne publishes a fresh one).
        if self.config.tivione == Some(true) {
            if let Some(feed) = std::fs::read_to_string(self.data_dir.join("tivione-feed.json"))
                .ok()
                .and_then(|text| serde_json::from_str(&text).ok())
                .and_then(|value| TvFeed::parse(&value))
            {
                self.sink.send(Update::Tv(Some(feed)));
            }
        }
        loop {
            let wait = self.next_due().saturating_duration_since(Instant::now());
            match rx.recv_timeout(wait) {
                Ok(request) => self.handle(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return,
            }
            self.tick();
        }
    }

    fn next_due(&self) -> Instant {
        let mut due = self.next_weather;
        if self.visible {
            if self.config.ha().is_some() {
                due = due.min(self.next_ha);
            }
            if self.config.plex_token.is_some() {
                due = due.min(self.next_plex);
            }
            if self.config.tivione_key.is_some() {
                due = due.min(self.next_tv);
            }
        }
        if let Some(pin) = &self.pin {
            due = due.min(pin.next_poll);
        }
        due
    }

    fn save(&self) {
        if let Err(error) = self.config.save(&self.data_dir) {
            log!("config: {error}");
        }
    }

    fn publish_status(&self) {
        self.sink.send(Update::Status(self.status.clone()));
    }

    fn handle(&mut self, request: ServiceRequest) {
        let now = Instant::now();
        match request {
            ServiceRequest::Visible(visible) => {
                // Coming back: refresh what may have changed meanwhile.
                if visible && !self.visible {
                    self.next_ha = self.next_ha.min(now);
                    self.next_plex = self.next_plex.min(now + Duration::from_millis(300));
                    self.next_tv = now;
                }
                self.visible = visible;
            }
            ServiceRequest::HaAction(entity) => {
                self.ha_action(&entity);
                self.next_ha = now + Duration::from_millis(400);
            }
            ServiceRequest::HaSetup(true) => {
                self.status.ha_setup = self.companion_url("/");
                self.publish_status();
            }
            ServiceRequest::HaSetup(false) => {
                self.status.ha_setup = None;
                self.stop_companion_if_idle();
                self.publish_status();
            }
            ServiceRequest::LocationSetup(true) => {
                self.status.location_setup = self.companion_url("/location");
                self.publish_status();
            }
            ServiceRequest::LocationSetup(false) => {
                self.status.location_setup = None;
                self.stop_companion_if_idle();
                self.publish_status();
            }
            ServiceRequest::LocationSet { name, lat, lon } => {
                log!("weather: location {name}");
                self.config.location = Some((lat, lon));
                self.config.location_name = Some(name.clone());
                self.save();
                self.status.location = Some(name);
                self.status.location_setup = None;
                self.stop_companion_if_idle();
                self.publish_status();
                self.next_weather = now;
            }
            ServiceRequest::WallpaperSetup(true) => {
                self.status.wallpaper_setup = self.companion_url("/wallpaper");
                self.publish_status();
            }
            ServiceRequest::WallpaperSetup(false) => {
                self.status.wallpaper_setup = None;
                self.stop_companion_if_idle();
                self.publish_status();
            }
            ServiceRequest::WallpaperUploaded(bytes) => {
                let path = self.data_dir.join("wallpaper.jpg");
                let temp = path.with_extension("tmp");
                match std::fs::write(&temp, &bytes).and_then(|()| std::fs::rename(&temp, &path)) {
                    Ok(()) => {
                        log!("wallpaper: photo from the phone, {} KB", bytes.len() / 1024);
                        self.status.wallpaper_custom = true;
                        self.status.wallpaper_version += 1;
                    }
                    Err(error) => log!("wallpaper: {error}"),
                }
                self.status.wallpaper_setup = None;
                self.stop_companion_if_idle();
                self.publish_status();
            }
            ServiceRequest::LocationClear => {
                self.config.location = None;
                self.config.location_name = None;
                self.save();
                self.status.location = None;
                self.publish_status();
                self.next_weather = now;
                if self.ha_location.is_none() && self.config.ha().is_none() {
                    self.sink.send(Update::Weather(None));
                }
            }
            ServiceRequest::HaConfigured { url, token } => {
                log!("home assistant: connected to {url}");
                self.config.ha_url = Some(url.clone());
                self.config.ha_token = Some(token);
                self.save();
                self.status.ha = Some(url);
                self.status.ha_setup = None;
                self.stop_companion_if_idle();
                self.status.ha_error = None;
                self.publish_status();
                self.ha_location = None;
                self.next_ha = now;
                self.next_weather = now;
            }
            ServiceRequest::HaCatalog => {
                self.areas_at = None;
                match self.ha_entities() {
                    Ok(all) => self.sink.send(Update::HaCatalog { entities: all, selected: self.config.ha_entities.clone() }),
                    Err(error) => log!("home assistant catalog: {error}"),
                }
            }
            ServiceRequest::HaSelect(selected) => {
                self.config.ha_entities = selected;
                self.save();
                self.status.ha_selected = self.config.ha_entities.len();
                self.publish_status();
                self.next_ha = now;
            }
            ServiceRequest::HaDisconnect => {
                self.config.ha_url = None;
                self.config.ha_token = None;
                self.save();
                self.status.ha = None;
                self.status.ha_error = None;
                self.publish_status();
                self.sink.send(Update::Entities(Vec::new()));
            }
            ServiceRequest::PlexLink(true) => self.start_pin(),
            ServiceRequest::PlexLink(false) => {
                self.pin = None;
                self.status.plex_code = None;
                self.publish_status();
            }
            ServiceRequest::PlexDisconnect => {
                self.config.plex_token = None;
                self.config.plex_user = None;
                self.config.plex_server = None;
                self.config.plex_server_token = None;
                self.save();
                self.status.plex = None;
                self.status.plex_error = None;
                self.publish_status();
                self.sink.send(Update::Media(Vec::new()));
            }
            ServiceRequest::TiviOne(enabled) => {
                self.config.tivione = Some(enabled);
                self.tv_seen = None;
                if enabled {
                    let mut key = [0u8; 32];
                    if getrandom::getrandom(&mut key).is_err() {
                        log!("tivione: no randomness");
                        return;
                    }
                    let key = base64::engine::general_purpose::STANDARD.encode(key);
                    self.config.tivione_key = Some(key.clone());
                    self.sink.send(Update::TvPair(key));
                    self.next_tv = now + Duration::from_secs(3);
                } else {
                    self.config.tivione_key = None;
                    let _ = std::fs::remove_file(self.data_dir.join("tivione-feed.json"));
                    self.sink.send(Update::Tv(None));
                }
                self.save();
                self.status.tivione = Some(enabled);
                self.publish_status();
            }
        }
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.pin.as_ref().is_some_and(|pin| now >= pin.next_poll) {
            self.poll_pin();
        }
        if self.visible && self.config.ha().is_some() && now >= self.next_ha {
            self.next_ha = now + HA_EVERY;
            self.ha_refresh();
        }
        if now >= self.next_weather {
            self.next_weather = now + self.weather().map_or(Duration::from_secs(120), |_| WEATHER_EVERY);
        }
        if self.visible && self.config.tivione_key.is_some() && now >= self.next_tv {
            self.next_tv = now + TV_EVERY;
            self.read_tv_feed();
        }
        if self.visible && self.config.plex_token.is_some() && now >= self.next_plex {
            self.next_plex = now + PLEX_EVERY;
            self.plex_refresh();
        }
    }

    // ---- TiviOne ---------------------------------------------------------------------

    /// Opens the sealed TiviOne snapshot when it changed; keeps a private plaintext copy.
    fn read_tv_feed(&mut self) {
        let Some(key) = self.config.tivione_key.as_deref() else { return };
        let Ok(modified) = std::fs::metadata(TV_FEED).and_then(|m| m.modified()) else { return };
        if self.tv_seen == Some(modified) {
            return;
        }
        self.tv_seen = Some(modified);
        let opened = std::fs::read_to_string(TV_FEED)
            .map_err(|e| e.to_string())
            .and_then(|text| open_sealed(&text, key));
        match opened {
            Ok(value) => {
                let Some(feed) = TvFeed::parse(&value) else { return };
                let copy = self.data_dir.join("tivione-feed.json");
                if std::fs::write(&copy, value.to_string()).is_ok() {
                    #[cfg(unix)]
                    {
                        use std::os::unix::fs::PermissionsExt;
                        let _ = std::fs::set_permissions(&copy, std::fs::Permissions::from_mode(0o600));
                    }
                }
                log!("tivione: {} channels, {} favourites, {} recent", feed.channels.len(), feed.favorites.len(), feed.recent.len());
                self.sink.send(Update::Tv(Some(feed)));
            }
            // A snapshot sealed for another pairing (or a stale one): wait for the next.
            Err(error) => log!("tivione feed: {error}"),
        }
    }

    // ---- Weather ---------------------------------------------------------------------

    fn weather(&mut self) -> Option<()> {
        let (lat, lon) = self.config.location.or(self.ha_location)?;
        let url = format!(
            "https://api.open-meteo.com/v1/forecast?latitude={lat:.3}&longitude={lon:.3}&current=temperature_2m,weather_code,is_day\
             &daily=weather_code,temperature_2m_max,temperature_2m_min&forecast_days=6&timezone=auto"
        );
        match net::get_json(&self.agent, &url, &[]) {
            Ok(reply) => {
                let current = reply.get("current")?;
                let mut weather = Weather {
                    temperature: current.get("temperature_2m")?.as_f64()? as f32,
                    code: current.get("weather_code")?.as_u64()? as u32,
                    is_day: current.get("is_day").and_then(Value::as_u64) != Some(0),
                    days: Default::default(),
                    day_count: 0,
                };
                if let Some(daily) = reply.get("daily") {
                    let column = |key: &str| daily.get(key).and_then(Value::as_array).cloned().unwrap_or_default();
                    let (codes, max, min) = (column("weather_code"), column("temperature_2m_max"), column("temperature_2m_min"));
                    for (i, day) in weather.days.iter_mut().enumerate() {
                        let (Some(code), Some(hi), Some(lo)) = (codes.get(i).and_then(Value::as_u64), max.get(i).and_then(Value::as_f64), min.get(i).and_then(Value::as_f64)) else {
                            break;
                        };
                        *day = Forecast { code: code as u32, max: hi as f32, min: lo as f32 };
                        weather.day_count = i + 1;
                    }
                }
                self.sink.send(Update::Weather(Some(weather)));
                Some(())
            }
            Err(error) => {
                log!("weather: {error}");
                None
            }
        }
    }

    // ---- Home Assistant -------------------------------------------------------------

    fn ha_get(&self, path: &str) -> Result<Value, String> {
        let (url, token) = self.config.ha().ok_or("not configured")?;
        let auth = format!("Bearer {token}");
        net::get_json(&self.agent, &format!("{url}{path}"), &[("Authorization", &auth)])
    }

    fn ha_refresh(&mut self) {
        if self.ha_location.is_none() {
            if let Ok(config) = self.ha_get("/api/config") {
                if let (Some(lat), Some(lon)) = (config.get("latitude").and_then(Value::as_f64), config.get("longitude").and_then(Value::as_f64)) {
                    self.ha_location = Some((lat, lon));
                    self.next_weather = Instant::now();
                }
            }
        }
        match self.ha_entities() {
            Ok(all) => {
                if self.status.ha_error.take().is_some() {
                    self.publish_status();
                }
                self.sink.send(Update::Entities(choose(all, &self.config.ha_entities)));
            }
            Err(error) => {
                log!("home assistant: {error}");
                let message = if error.contains("401") { "Token rejected" } else { "Not reachable" };
                if self.status.ha_error.as_deref() != Some(message) {
                    self.status.ha_error = Some(message.into());
                    self.publish_status();
                }
            }
        }
    }

    /// Every selectable entity with its area, sorted by category, area and name.
    fn ha_entities(&mut self) -> Result<Vec<Entity>, String> {
        if self.areas_at.is_none_or(|t| t.elapsed() > AREAS_EVERY) {
            if let Some((url, token)) = self.config.ha() {
                let auth = format!("Bearer {token}");
                match net::post_json(&self.agent, &format!("{url}/api/template"), &[("Authorization", &auth)], Some(&json!({ "template": AREA_TEMPLATE }))) {
                    Ok(Value::Array(pairs)) => {
                        self.areas = pairs
                            .iter()
                            .filter_map(|p| Some((p.get(0)?.as_str()?.to_string(), p.get(1)?.as_str()?.to_string())))
                            .collect();
                        self.areas_at = Some(Instant::now());
                    }
                    Ok(_) => self.areas_at = Some(Instant::now()),
                    Err(error) => log!("home assistant areas: {error}"),
                }
            }
        }
        let states = self.ha_get("/api/states")?;
        Ok(entities_from_states(&states, &self.areas))
    }

    fn ha_action(&mut self, entity: &str) {
        let Some((url, token)) = self.config.ha() else { return };
        let domain = entity.split('.').next().unwrap_or_default();
        // Scenes and scripts run; media players play/pause; everything else toggles.
        let (domain, service) = match domain {
            "scene" | "script" => (domain, "turn_on"),
            "media_player" => (domain, "media_play_pause"),
            _ => ("homeassistant", "toggle"),
        };
        let auth = format!("Bearer {token}");
        let endpoint = format!("{url}/api/services/{domain}/{service}");
        if let Err(error) = net::post_json(&self.agent, &endpoint, &[("Authorization", &auth)], Some(&json!({ "entity_id": entity }))) {
            log!("home assistant {entity}: {error}");
        }
    }

    /// The phone page address for `path`, starting the companion server when needed. One
    /// server and key serve both setup pages.
    fn companion_url(&mut self, path: &str) -> Option<String> {
        if self.companion.is_none() {
            let key = format!("{:06}", u64::from_str_radix(&crate::config::random_id()[..8], 16).unwrap_or(0) % 1_000_000);
            match companion::start(key.clone(), self.requests.clone()) {
                Ok(server) => self.companion = Some((server, key)),
                Err(error) => {
                    log!("companion: {error}");
                    self.status.ha_error = Some("Setup page could not start".into());
                    return None;
                }
            }
        }
        let key = &self.companion.as_ref()?.1;
        let ip = net::local_ip().unwrap_or_else(|| "tv".into());
        Some(format!("http://{ip}:{}{path}?k={key}", companion::PORT))
    }

    fn stop_companion_if_idle(&mut self) {
        if self.status.ha_setup.is_none() && self.status.location_setup.is_none() && self.status.wallpaper_setup.is_none() {
            self.companion = None;
        }
    }

    // ---- Plex -----------------------------------------------------------------------

    fn plex_headers(&self) -> Vec<(&'static str, String)> {
        vec![
            ("X-Plex-Product", "Vision".into()),
            ("X-Plex-Client-Identifier", self.config.plex_client_id.clone()),
            ("X-Plex-Device-Name", "LG TV (Vision)".into()),
            ("X-Plex-Platform", "webOS".into()),
        ]
    }

    fn plex_tv(&self, path: &str, token: Option<&str>, post: bool) -> Result<Value, String> {
        let mut headers = self.plex_headers();
        if let Some(token) = token {
            headers.push(("X-Plex-Token", token.to_string()));
        }
        let headers: Vec<(&str, &str)> = headers.iter().map(|(k, v)| (*k, v.as_str())).collect();
        let url = format!("{PLEX_TV}{path}");
        if post {
            net::post_json(&self.agent, &url, &headers, None)
        } else {
            net::get_json(&self.agent, &url, &headers)
        }
    }

    fn start_pin(&mut self) {
        match self.plex_tv("/pins?strong=false", None, true) {
            Ok(pin) => {
                let (Some(id), Some(code)) = (pin.get("id").and_then(Value::as_u64), pin.get("code").and_then(Value::as_str)) else {
                    self.status.plex_error = Some("Plex did not return a code".into());
                    self.publish_status();
                    return;
                };
                let lifetime = pin.get("expiresIn").and_then(Value::as_u64).unwrap_or(900);
                self.pin = Some(Pin {
                    id,
                    expires: Instant::now() + Duration::from_secs(lifetime),
                    next_poll: Instant::now() + PIN_POLL,
                });
                self.status.plex_code = Some(code.to_uppercase());
                self.status.plex_error = None;
            }
            Err(error) => {
                log!("plex pin: {error}");
                self.status.plex_error = Some("Plex is not reachable".into());
            }
        }
        self.publish_status();
    }

    fn poll_pin(&mut self) {
        let Some(pin) = &mut self.pin else { return };
        if Instant::now() > pin.expires {
            // Codes expire; fetch a fresh one while the panel stays open.
            self.start_pin();
            return;
        }
        pin.next_poll = Instant::now() + PIN_POLL;
        let id = pin.id;
        let Ok(reply) = self.plex_tv(&format!("/pins/{id}"), None, false) else { return };
        let Some(token) = reply.get("authToken").and_then(Value::as_str).filter(|t| !t.is_empty()) else { return };
        self.pin = None;
        self.config.plex_token = Some(token.to_string());
        let user = self
            .plex_tv("/user", Some(token), false)
            .ok()
            .and_then(|u| u.get("title").or_else(|| u.get("username")).and_then(Value::as_str).map(str::to_string));
        self.config.plex_user = user.clone();
        self.config.plex_server = None;
        self.save();
        log!("plex: linked");
        self.status.plex = Some(user.unwrap_or_else(|| "Plex".into()));
        self.status.plex_code = None;
        self.status.plex_error = None;
        self.publish_status();
        self.next_plex = Instant::now();
    }

    /// Finds a reachable server: owned first, local connections before remote, relay last.
    fn discover_server(&mut self) -> bool {
        let Some(token) = self.config.plex_token.clone() else { return false };
        let Ok(resources) = self.plex_tv("/resources?includeHttps=1&includeRelay=1", Some(&token), false) else { return false };
        let mut servers: Vec<&Value> = resources
            .as_array()
            .map(|r| r.iter().filter(|d| d.get("provides").and_then(Value::as_str).is_some_and(|p| p.contains("server"))).collect())
            .unwrap_or_default();
        servers.sort_by_key(|s| !s.get("owned").and_then(Value::as_bool).unwrap_or(false));
        for server in servers {
            let access = server.get("accessToken").and_then(Value::as_str).unwrap_or(&token).to_string();
            let mut connections: Vec<&Value> = server.get("connections").and_then(Value::as_array).map(|c| c.iter().collect()).unwrap_or_default();
            connections.sort_by_key(|c| {
                let local = c.get("local").and_then(Value::as_bool).unwrap_or(false);
                let relay = c.get("relay").and_then(Value::as_bool).unwrap_or(false);
                (relay, !local)
            });
            for connection in connections {
                let Some(uri) = connection.get("uri").and_then(Value::as_str) else { continue };
                if net::get_json(&self.agent, &format!("{uri}/identity"), &[("X-Plex-Token", &access)]).is_ok() {
                    self.config.plex_server = Some(uri.to_string());
                    self.config.plex_server_token = Some(access);
                    self.save();
                    log!("plex: server {uri}");
                    return true;
                }
            }
        }
        false
    }

    fn plex_refresh(&mut self) {
        if self.config.plex_server.is_none() && !self.discover_server() {
            self.set_plex_error(Some("No Plex server reachable"));
            return;
        }
        let items = match self.continue_watching() {
            Ok(items) => items,
            // The server may have moved (new address): rediscover once.
            Err(_) if self.discover_server() => self.continue_watching().unwrap_or_default(),
            Err(error) => {
                log!("plex: {error}");
                self.set_plex_error(Some("Plex server not reachable"));
                return;
            }
        };
        self.set_plex_error(None);
        let media: Vec<MediaItem> = items.iter().filter_map(media_item).collect();
        self.sink.send(Update::Media(media));
        for item in &items {
            self.media_artwork(item);
        }
    }

    fn set_plex_error(&mut self, error: Option<&str>) {
        if self.status.plex_error.as_deref() != error {
            self.status.plex_error = error.map(str::to_string);
            self.publish_status();
        }
    }

    fn server_get(&self, path: &str) -> Result<Value, String> {
        let (Some(server), Some(token)) = (&self.config.plex_server, &self.config.plex_server_token) else {
            return Err("no server".into());
        };
        net::get_json(&self.agent, &format!("{server}{path}"), &[("X-Plex-Token", token)])
    }

    fn continue_watching(&self) -> Result<Vec<Value>, String> {
        let reply = self
            .server_get("/hubs/continueWatching/items?X-Plex-Container-Start=0&X-Plex-Container-Size=12")
            .or_else(|_| self.server_get("/library/onDeck?X-Plex-Container-Start=0&X-Plex-Container-Size=12"))?;
        Ok(reply
            .pointer("/MediaContainer/Metadata")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default())
    }

    /// Fetches 16:9 artwork through the server's transcoder (small JPEG), cached on disk.
    fn media_artwork(&mut self, item: &Value) {
        let Some(id) = item.get("ratingKey").and_then(Value::as_str) else { return };
        let art_id = format!("plex:{id}");
        if self.media_art.contains(&art_id) {
            return;
        }
        let Some(art) = ["art", "grandparentArt", "thumb"].iter().find_map(|k| item.get(*k).and_then(Value::as_str)) else { return };
        self.media_art.insert(art_id.clone());
        if let Some(image) = crate::storage::load_icon(&self.data_dir, "art", &art_id, art) {
            self.sink.send(Update::Art(image));
            return;
        }
        let (Some(server), Some(token)) = (&self.config.plex_server, &self.config.plex_server_token) else { return };
        let url = format!(
            "{server}/photo/:/transcode?width=480&height=270&minSize=1&upscale=1&url={}&X-Plex-Token={token}",
            net::encode(art)
        );
        let result = net::get_bytes(&self.agent, &url, &[], 4 << 20)
            .and_then(|bytes| model::decode_jpeg(&bytes).or_else(|_| model::decode_png_rgba(&bytes)));
        match result {
            Ok((rgba, w, h)) => {
                let image = model::art_from_rgba(&art_id, &rgba, w, h);
                let _ = crate::storage::save_icon(&self.data_dir, "art", &image, art);
                self.sink.send(Update::Art(image));
            }
            Err(error) => log!("plex art {id}: {error}"),
        }
    }
}

/// All selectable entities (unavailable ones left out), by category, area and name.
fn entities_from_states(states: &Value, areas: &HashMap<String, String>) -> Vec<Entity> {
    let mut entities: Vec<Entity> = states
        .as_array()
        .map(|states| {
            states
                .iter()
                .filter_map(|state| {
                    let id = state.get("entity_id")?.as_str()?.to_string();
                    let domain = id.split('.').next()?.to_string();
                    let attributes = state.get("attributes");
                    let category = HaCategory::of(&domain, attributes)?;
                    let value = state.get("state").and_then(Value::as_str).unwrap_or("").to_string();
                    if value == "unavailable" {
                        return None;
                    }
                    let name = attributes.and_then(|a| a.get("friendly_name")).and_then(Value::as_str).unwrap_or(&id).to_string();
                    let on = match category {
                        HaCategory::Scenes | HaCategory::Scripts | HaCategory::Media | HaCategory::Covers => None,
                        _ => Some(value == "on"),
                    };
                    let brightness = attributes
                        .and_then(|a| a.get("brightness"))
                        .and_then(Value::as_u64)
                        .map(|b| ((b as f32 / 255.0) * 100.0).round() as u8);
                    Some(Entity { area: areas.get(&id).cloned().unwrap_or_default(), id, name, category, state: value, on, brightness })
                })
                .collect()
        })
        .unwrap_or_default();
    entities.sort_by(|a, b| {
        a.category
            .cmp(&b.category)
            .then_with(|| a.area.is_empty().cmp(&b.area.is_empty()))
            .then_with(|| a.area.cmp(&b.area))
            .then_with(|| a.name.cmp(&b.name))
    });
    entities
}

/// The Home row: the user's choice in the picker's order; until then a small automatic set
/// (scenes, scripts and groups).
fn choose(all: Vec<Entity>, selected: &[String]) -> Vec<Entity> {
    if selected.is_empty() {
        return all
            .into_iter()
            .filter(|e| matches!(e.category, HaCategory::Scenes | HaCategory::Scripts | HaCategory::Groups))
            .take(12)
            .collect();
    }
    all.into_iter().filter(|e| selected.contains(&e.id)).collect()
}

fn media_item(item: &Value) -> Option<MediaItem> {
    let id = item.get("ratingKey")?.as_str()?.to_string();
    let text = |k: &str| item.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let number = |k: &str| item.get(k).and_then(Value::as_u64);
    let (title, subtitle) = if text("type") == "episode" {
        let episode = match (number("parentIndex"), number("index")) {
            (Some(s), Some(e)) => format!("S{s} · E{e} · "),
            _ => String::new(),
        };
        (text("grandparentTitle"), format!("{episode}{}", text("title")))
    } else {
        (text("title"), number("year").map(|y| y.to_string()).unwrap_or_default())
    };
    let duration = number("duration").unwrap_or(0) as f32;
    let offset = number("viewOffset").unwrap_or(0) as f32;
    Some(MediaItem {
        id,
        title,
        subtitle,
        progress: if duration > 0.0 { (offset / duration).clamp(0.0, 1.0) } else { 0.0 },
        remaining_min: ((duration - offset).max(0.0) / 60_000.0).round() as u32,
    })
}

/// Opens `{ v, nonce, box }` sealed with NaCl secretbox (XSalsa20-Poly1305).
fn open_sealed(text: &str, key: &str) -> Result<Value, String> {
    use crypto_secretbox::{aead::Aead, KeyInit, XSalsa20Poly1305};
    let b64 = base64::engine::general_purpose::STANDARD;
    let envelope: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let field = |k: &str| envelope.get(k).and_then(Value::as_str).ok_or(format!("missing {k}"));
    let nonce = b64.decode(field("nonce")?).map_err(|e| e.to_string())?;
    let sealed = b64.decode(field("box")?).map_err(|e| e.to_string())?;
    let key = b64.decode(key).map_err(|e| e.to_string())?;
    if key.len() != 32 || nonce.len() != 24 {
        return Err("bad key or nonce".into());
    }
    let cipher = XSalsa20Poly1305::new(crypto_secretbox::Key::from_slice(&key));
    let plain = cipher
        .decrypt(crypto_secretbox::Nonce::from_slice(&nonce), sealed.as_slice())
        .map_err(|_| "cannot open (not sealed for this pairing)".to_string())?;
    serde_json::from_slice(&plain).map_err(|e| e.to_string())
}
