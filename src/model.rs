use serde_json::Value;

#[derive(Clone, Debug)]
pub struct App {
    pub id: String,
    pub title: String,
    pub icon_path: Option<String>,
    /// Background colour LG declares for the icon, if any.
    pub bg_color: Option<String>,
    pub params: Option<Value>,
    /// Large 16:9 artwork for full-bleed tiles: the launch splash (logo on brand colour)
    /// when the app has one, else LG's launcher background image.
    pub art_path: Option<String>,
    pub art_is_splash: bool,
}

#[derive(Clone, Debug)]
pub struct Input {
    pub id: String,
    pub label: String,
    pub app_id: String,
    pub connected: bool,
}

/// RGBA8 image (an icon ≤128 px or 320×180 tile artwork) with its dominant colour.
pub struct Icon {
    pub app_id: String,
    pub w: u32,
    pub h: u32,
    pub pixels: Vec<u8>,
    pub color: Option<[u8; 3]>,
}

impl Icon {
    /// The icon's own background: the mean border colour when the border is opaque and
    /// uniform. A tile filled with it makes the icon blend into a full-bleed logo tile.
    pub fn edge_color(&self) -> Option<[u8; 3]> {
        let (w, h) = (self.w as usize, self.h as usize);
        if w < 8 || h < 8 {
            return None;
        }
        let mut samples = Vec::new();
        for i in 0..w.max(h) {
            for (x, y) in [(i.min(w - 1), 1), (i.min(w - 1), h - 2), (1, i.min(h - 1)), (w - 2, i.min(h - 1))] {
                let p = &self.pixels[(y * w + x) * 4..][..4];
                if p[3] < 250 {
                    return None;
                }
                samples.push([p[0] as f32, p[1] as f32, p[2] as f32]);
            }
        }
        let n = samples.len() as f32;
        let mean = samples.iter().fold([0.0; 3], |a, p| [a[0] + p[0] / n, a[1] + p[1] / n, a[2] + p[2] / n]);
        let spread = samples.iter().map(|p| (p[0] - mean[0]).abs() + (p[1] - mean[1]).abs() + (p[2] - mean[2]).abs()).sum::<f32>() / n;
        (spread < 18.0).then(|| [mean[0] as u8, mean[1] as u8, mean[2] as u8])
    }
}

/// Messages for the user; the UI picks the localised text.
pub enum Notice {
    ServiceUnavailable,
    AppListUnavailable,
    LaunchFailed,
    #[cfg_attr(all(target_os = "linux", target_arch = "arm"), allow(dead_code))]
    PreviewLaunch(String),
}

pub enum Update {
    Apps(Vec<App>),
    Inputs(Vec<Input>),
    Icon(Icon),
    /// Tile artwork; `app_id` is an app id or `plex:<ratingKey>`.
    Art(Icon),
    Notice(Notice),
    Weather(Option<Weather>),
    Entities(Vec<Entity>),
    Media(Vec<MediaItem>),
    /// Every selectable Home Assistant entity, plus the current choice (for the picker).
    HaCatalog { entities: Vec<Entity>, selected: Vec<String> },
    Status(Status),
    /// TiviOne's snapshot (None: integration off or nothing published yet).
    Tv(Option<TvFeed>),
    /// Integration switched on: launch TiviOne once with this key to pair.
    TvPair(String),
    /// TiviOne on this TV (None: not installed).
    TiviOne(Option<Installed>),
    /// The newest release on TiviOne's website (None: unknown or unreachable).
    Release(Option<Release>),
    Install(InstallState),
    Picture(Picture),
    /// The Glass wallpaper, decoded (None: it could not be loaded).
    Wallpaper(Option<Wallpaper>),
    /// The Homebrew repository, or why it could not be loaded.
    Homebrew(Result<Vec<HomebrewApp>, String>),
    HomebrewIcon(Icon),
    HomebrewInstall { app: String, state: InstallState },
    /// Installed version of every app (id, version).
    Versions(Vec<(String, String)>),
}

/// TiviOne's home-screen snapshot (see tivione/src/core/launcher.ts).
#[derive(Clone, Debug, Default)]
pub struct TvFeed {
    pub playlist_id: i64,
    pub channels: std::collections::HashMap<i64, TvChannel>,
    pub favorites: Vec<i64>,
    pub recent: Vec<i64>,
    pub reminders: Vec<TvReminder>,
    pub catchup: Vec<TvCatchup>,
    /// Live stream per channel for the hero preview (Premium TiviOne only).
    pub streams: std::collections::HashMap<i64, crate::video::Stream>,
    pub guide: std::collections::HashMap<i64, Vec<Programme>>,
    pub numbers: std::collections::HashMap<u32, i64>,
}

#[derive(Clone, Debug)]
pub struct TvChannel {
    pub name: String,
    pub number: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct TvReminder {
    pub channel: i64,
    pub title: String,
    pub start: i64,
}

#[derive(Clone, Debug)]
pub struct TvCatchup {
    pub channel: i64,
    pub title: String,
    pub start: i64,
    pub end: i64,
}

#[derive(Clone, Debug)]
pub struct Programme {
    pub title: String,
    pub start: i64,
    pub end: i64,
}

impl TvFeed {
    pub fn parse(value: &Value) -> Option<Self> {
        let ids = |key: &str| -> Vec<i64> { value.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_i64).collect()).unwrap_or_default() };
        let channels = value
            .get("channels")?
            .as_array()?
            .iter()
            .filter_map(|c| {
                Some((c.get("id")?.as_i64()?, TvChannel {
                    name: c.get("name")?.as_str()?.to_string(),
                    number: c.get("number").and_then(Value::as_u64).map(|n| n as u32),
                }))
            })
            .collect();
        let guide = value
            .get("guide")
            .and_then(Value::as_object)
            .map(|g| {
                g.iter()
                    .filter_map(|(id, list)| {
                        let programmes = list
                            .as_array()?
                            .iter()
                            .filter_map(|p| Some(Programme { title: p.get("t")?.as_str()?.to_string(), start: p.get("s")?.as_i64()?, end: p.get("e")?.as_i64()? }))
                            .collect();
                        Some((id.parse().ok()?, programmes))
                    })
                    .collect()
            })
            .unwrap_or_default();
        Some(Self {
            playlist_id: value.get("playlistId").and_then(Value::as_i64).unwrap_or(0),
            channels,
            favorites: ids("favorites"),
            recent: ids("recent"),
            reminders: value
                .get("reminders")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|r| Some(TvReminder { channel: r.get("id")?.as_i64()?, title: r.get("title")?.as_str()?.to_string(), start: r.get("s")?.as_i64()? }))
                        .collect()
                })
                .unwrap_or_default(),
            guide,
            catchup: value
                .get("catchup")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            Some(TvCatchup { channel: c.get("id")?.as_i64()?, title: c.get("t")?.as_str()?.to_string(), start: c.get("s")?.as_i64()?, end: c.get("e")?.as_i64()? })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            streams: value
                .get("streams")
                .and_then(Value::as_object)
                .map(|m| {
                    m.iter()
                        .filter_map(|(id, s)| {
                            let text = |k: &str| s.get(k).and_then(Value::as_str).filter(|v| !v.is_empty()).map(str::to_string);
                            Some((id.parse().ok()?, crate::video::Stream { url: text("url")?, user_agent: text("ua"), referer: text("ref") }))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            numbers: value
                .get("numbers")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|p| Some((p.get(0)?.as_u64()? as u32, p.get(1)?.as_i64()?))).collect())
                .unwrap_or_default(),
        })
    }

    /// The programme on air at `now_ms` and the one after it.
    pub fn now_next(&self, channel: i64, now_ms: i64) -> (Option<&Programme>, Option<&Programme>) {
        let Some(list) = self.guide.get(&channel) else { return (None, None) };
        let now = list.iter().find(|p| p.start <= now_ms && p.end > now_ms);
        let after = now.map_or(now_ms, |p| p.end);
        (now, list.iter().find(|p| p.start >= after))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Weather {
    pub temperature: f32,
    /// WMO weather interpretation code.
    pub code: u32,
}

/// A Home Assistant entity shown as a tile or in the picker.
#[derive(Clone, Debug)]
pub struct Entity {
    pub id: String,
    pub name: String,
    pub category: HaCategory,
    /// Area (room) from Home Assistant, empty when unassigned.
    pub area: String,
    pub state: String,
    pub on: Option<bool>,
    pub brightness: Option<u8>,
}

/// Picker and Home-row grouping, in display order. Locks and alarms are deliberately absent:
/// a stray OK must never unlock a door.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum HaCategory {
    Scenes,
    Scripts,
    Groups,
    Lights,
    Switches,
    Toggles,
    Fans,
    Covers,
    Media,
}

impl HaCategory {
    pub const ALL: [HaCategory; 9] = [
        HaCategory::Scenes,
        HaCategory::Scripts,
        HaCategory::Groups,
        HaCategory::Lights,
        HaCategory::Switches,
        HaCategory::Toggles,
        HaCategory::Fans,
        HaCategory::Covers,
        HaCategory::Media,
    ];

    /// Groups are `group.*` plus light and switch groups (entities listing members).
    pub fn of(domain: &str, attributes: Option<&Value>) -> Option<Self> {
        let is_group = attributes.and_then(|a| a.get("entity_id")).and_then(Value::as_array).is_some_and(|m| !m.is_empty());
        Some(match domain {
            "scene" => Self::Scenes,
            "script" => Self::Scripts,
            "group" => Self::Groups,
            "light" | "switch" if is_group => Self::Groups,
            "light" => Self::Lights,
            "switch" => Self::Switches,
            "input_boolean" => Self::Toggles,
            "fan" => Self::Fans,
            "cover" => Self::Covers,
            "media_player" => Self::Media,
            _ => return None,
        })
    }
}

/// A Plex "Continue watching" item.
#[derive(Clone, Debug)]
pub struct MediaItem {
    pub id: String,
    pub title: String,
    pub subtitle: String,
    pub progress: f32,
    pub remaining_min: u32,
}

/// Connection state of the optional services, for the Settings row and setup panels.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Status {
    /// Connected Home Assistant address.
    pub ha: Option<String>,
    /// Phone setup address while pairing.
    pub ha_setup: Option<String>,
    pub ha_error: Option<String>,
    /// How many entities the user chose for the Home row (0: automatic).
    pub ha_selected: usize,
    /// Connected Plex account.
    pub plex: Option<String>,
    /// plex.tv/link code while linking.
    pub plex_code: Option<String>,
    pub plex_error: Option<String>,
    pub tivione: Option<bool>,
    /// Custom weather location (None: from Home Assistant).
    pub location: Option<String>,
    /// Phone page address while choosing a weather location.
    pub location_setup: Option<String>,
    /// Phone page address while sending a wallpaper photo.
    pub wallpaper_setup: Option<String>,
    /// A photo from the phone is saved as the wallpaper `custom`.
    pub wallpaper_custom: bool,
    /// Counts photos received, so the screen can switch to a new one.
    pub wallpaper_version: u32,
}

pub enum ServiceRequest {
    Visible(bool),
    HaAction(String),
    HaSetup(bool),
    HaConfigured { url: String, token: String },
    HaDisconnect,
    HaCatalog,
    HaSelect(Vec<String>),
    PlexLink(bool),
    PlexDisconnect,
    TiviOne(bool),
    LocationSetup(bool),
    LocationSet { name: String, lat: f64, lon: f64 },
    LocationClear,
    WallpaperSetup(bool),
    /// A 1920×1080 JPEG from the phone page.
    WallpaperUploaded(Vec<u8>),
}

pub const TIVIONE_APP: &str = "com.tivione.tv";
/// The first TiviOne with the launcher feed; older versions only open.
pub const TIVIONE_FEED_VERSION: &str = "1.0.1";

/// A TiviOne release for direct install, from the signed manifest on TiviOne's website.
#[derive(Clone, Debug, PartialEq)]
pub struct Release {
    pub version: String,
    pub url: String,
    pub sha256: [u8; 32],
    pub size: u64,
    /// Pictures for the store page: the banner, then screenshots.
    pub pictures: Vec<String>,
}

impl Release {
    pub fn package(&self) -> Package {
        Package { app: TIVIONE_APP.into(), version: self.version.clone(), url: self.url.clone(), sha256: self.sha256, size: Some(self.size) }
    }
}

/// Something to download, verify and install.
#[derive(Clone, Debug, PartialEq)]
pub struct Package {
    pub app: String,
    pub version: String,
    pub url: String,
    pub sha256: [u8; 32],
    pub size: Option<u64>,
}

pub const HOMEBREW_CHANNEL: &str = "org.webosbrew.hbchannel";

/// An app in the Homebrew Channel repository (repo.webosbrew.org).
#[derive(Clone, Debug)]
pub struct HomebrewApp {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub description: String,
    pub icon_url: String,
    /// Runs with root rights on the TV.
    pub root: bool,
    pub featured: bool,
    pub source: String,
    pub package: Package,
}

/// App ids end up in shell commands (install, uninstall): letters, digits, '.', '-', '_' only.
pub fn valid_app_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// The repository index (`api/apps.json`), featured apps first; entries without a valid id,
/// package address or checksum are left out.
pub fn homebrew_apps(index: &Value) -> Vec<HomebrewApp> {
    let mut apps: Vec<HomebrewApp> = index
        .get("packages")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|entry| {
            let manifest = entry.get("manifest")?;
            let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
            let id = text(entry, "id");
            let url = text(manifest, "ipkUrl");
            let hex = manifest.pointer("/ipkHash/sha256").and_then(Value::as_str)?;
            if !valid_app_id(&id) || !url.starts_with("https://") || hex.len() != 64 {
                return None;
            }
            let mut sha256 = [0u8; 32];
            for (i, byte) in sha256.iter_mut().enumerate() {
                *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok()?;
            }
            let title = text(entry, "title");
            Some(HomebrewApp {
                summary: text(entry, "shortDescription"),
                description: text(manifest, "appDescription"),
                icon_url: Some(text(entry, "iconUri")).filter(|u| !u.is_empty()).unwrap_or_else(|| text(manifest, "iconUri")),
                root: manifest.get("rootRequired").and_then(Value::as_bool).unwrap_or(false),
                featured: entry.get("featured").and_then(Value::as_bool).unwrap_or(false),
                source: text(manifest, "sourceUrl"),
                package: Package {
                    app: id.clone(),
                    version: text(manifest, "version"),
                    url,
                    sha256,
                    size: manifest.get("ipkSize").and_then(Value::as_u64),
                },
                title: if title.is_empty() { id.clone() } else { title },
                id,
            })
        })
        .collect();
    apps.sort_by_key(|a| !a.featured);
    apps
}

/// TiviOne as installed: its version, and whether it is a developer install (from the
/// website, kept up to date by Vision) rather than LG's store.
#[derive(Clone, Debug, PartialEq)]
pub struct Installed {
    pub version: String,
    pub direct: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub enum InstallState {
    Idle,
    Downloading(f32),
    Installing,
    Removing,
    Done,
    Failed(InstallFailure),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum InstallFailure {
    Network,
    /// The download did not match the signed manifest.
    Damaged,
    /// No root: Homebrew Channel cannot run the installer.
    NeedsRoot,
    /// The TV's installer refused the package.
    Refused,
}

/// One store-page picture, decoded: `index` 0 is the banner, then the screenshots.
pub struct Picture {
    pub index: usize,
    pub rgba: Vec<u8>,
    pub w: usize,
    pub h: usize,
}

/// TiviOne in `listApps`: version, and a developer install when it lives in the developer
/// app directory (where Vision's own installs land).
pub fn tivione_from_apps(reply: &Value) -> Option<Installed> {
    let app = reply.get("apps")?.as_array()?.iter().find(|a| a.get("id").and_then(Value::as_str) == Some(TIVIONE_APP))?;
    Some(Installed {
        version: app.get("version").and_then(Value::as_str).unwrap_or_default().to_string(),
        direct: app.get("folderPath").and_then(Value::as_str).is_some_and(|p| p.starts_with("/media/developer/")),
    })
}

/// Wallpapers are 1920×1080 RGB; the blurred copy (for the frosted dock) is a quarter of that.
pub const WALLPAPER: (usize, usize) = (1920, 1080);
pub const WALLPAPER_BLUR: (usize, usize) = (480, 270);

pub struct Wallpaper {
    pub rgb: Vec<u8>,
    pub blur: Vec<u8>,
}

/// Decodes a 1920×1080 JPEG and prepares its blurred quarter-size copy (a few box-blur passes,
/// once, so the dock never needs a real-time blur).
pub fn decode_wallpaper(bytes: &[u8]) -> Result<Wallpaper, String> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("jpeg without info")?;
    if (info.width as usize, info.height as usize) != WALLPAPER {
        return Err(format!("wallpaper is {}x{}, not 1920x1080", info.width, info.height));
    }
    let rgb = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels,
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g]).collect(),
        _ => return Err("unsupported jpeg format".into()),
    };
    let (w, h) = WALLPAPER;
    let (bw, bh) = WALLPAPER_BLUR;
    // 4×4 average down to a quarter, as floats for the blur passes.
    let mut small = vec![0f32; bw * bh * 3];
    for y in 0..bh {
        for x in 0..bw {
            for c in 0..3 {
                let mut sum = 0u32;
                for dy in 0..4 {
                    for dx in 0..4 {
                        sum += rgb[((y * 4 + dy) * w + x * 4 + dx) * 3 + c] as u32;
                    }
                }
                small[(y * bw + x) * 3 + c] = sum as f32 / 16.0;
            }
        }
    }
    for _ in 0..3 {
        box_blur(&mut small, bw, bh, 6, true);
        box_blur(&mut small, bw, bh, 6, false);
    }
    let blur = small.iter().map(|v| v.round().clamp(0.0, 255.0) as u8).collect();
    let _ = h;
    Ok(Wallpaper { rgb, blur })
}

/// One running-sum box blur along rows (`horizontal`) or columns, edges clamped.
fn box_blur(data: &mut [f32], w: usize, h: usize, radius: usize, horizontal: bool) {
    let (lines, len) = if horizontal { (h, w) } else { (w, h) };
    let at = |line: usize, i: usize| if horizontal { (line * w + i) * 3 } else { (i * w + line) * 3 };
    let mut out = vec![0f32; len * 3];
    let window = (2 * radius + 1) as f32;
    for line in 0..lines {
        for c in 0..3 {
            let sample = |i: isize| data[at(line, i.clamp(0, len as isize - 1) as usize) + c];
            let mut sum: f32 = (-(radius as isize)..=radius as isize).map(sample).sum();
            for i in 0..len {
                out[i * 3 + c] = sum / window;
                sum += sample(i as isize + radius as isize + 1) - sample(i as isize - radius as isize);
            }
        }
        for i in 0..len {
            data[at(line, i)..at(line, i) + 3].copy_from_slice(&out[i * 3..i * 3 + 3]);
        }
    }
}

/// Every app's version from `listApps`.
pub fn versions_from_apps(reply: &Value) -> Vec<(String, String)> {
    reply
        .get("apps")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a.get("id")?.as_str()?.to_string(), a.get("version")?.as_str()?.to_string())))
        .collect()
}

/// Dotted version comparison ("1.0.10" > "1.0.9"); missing parts count as 0.
pub fn version_newer(a: &str, b: &str) -> bool {
    let parts = |v: &str| -> Vec<u64> { v.split('.').map(|p| p.trim().parse().unwrap_or(0)).collect() };
    let (a, b) = (parts(a), parts(b));
    for i in 0..a.len().max(b.len()) {
        let (x, y) = (a.get(i).copied().unwrap_or(0), b.get(i).copied().unwrap_or(0));
        if x != y {
            return x > y;
        }
    }
    false
}

pub enum Request {
    Refresh,
    /// Switch the Home key between Vision and LG Home (root, via tv/home.sh).
    HomeTakeover(bool),
    /// Drop the icon and artwork caches and load everything again.
    RefreshImages,
    Launch { app_id: String, params: Option<Value> },
    SavePrefs(crate::storage::Prefs),
    /// Download, verify and install this TiviOne release (root, via Homebrew Channel).
    Install(Release),
    /// Fetch and decode the store page's pictures.
    Pictures(Release),
    /// Decode a wallpaper JPEG.
    Wallpaper(std::path::PathBuf),
    /// Load the Homebrew repository and its icons.
    Homebrew,
    HomebrewInstall(Package),
    HomebrewRemove(String),
}

/// Normalises `listLaunchPoints` (LG's Home bar, already in the user's order), taking each
/// app's launch splash from `listApps` when available.
pub fn apps_from_launch_points(reply: &Value, apps: Option<&Value>, own_id: &str) -> Vec<App> {
    let Some(points) = reply.get("launchPoints").and_then(Value::as_array) else {
        return Vec::new();
    };
    let splashes: std::collections::HashMap<&str, String> = apps
        .and_then(|apps| apps.get("apps"))
        .and_then(Value::as_array)
        .map(|apps| {
            apps.iter()
                .filter_map(|app| {
                    let id = app.get("id")?.as_str()?;
                    let folder = app.get("folderPath")?.as_str()?;
                    let splash = app.get("splashBackground")?.as_str()?;
                    let path = if splash.starts_with('/') { splash.to_string() } else { format!("{folder}/{splash}") };
                    path.to_ascii_lowercase().ends_with(".png").then(|| (id, path))
                })
                .collect()
        })
        .unwrap_or_default();
    points
        .iter()
        .filter_map(|point| {
            let id = point.get("id")?.as_str()?.to_string();
            if id == own_id || id == "com.webos.app.home" || id.starts_with("com.webos.app.hdmi") {
                return None;
            }
            let title = point.get("title").and_then(Value::as_str).unwrap_or(&id).trim().to_string();
            let icon_path = ["largeIcon", "icon"]
                .iter()
                .find_map(|key| point.get(*key).and_then(Value::as_str))
                .filter(|path| path.starts_with('/'))
                .map(str::to_string);
            let splash = splashes.get(id.as_str()).cloned();
            let background = point
                .get("bgImage")
                .and_then(Value::as_str)
                .filter(|path| path.starts_with('/') && path.to_ascii_lowercase().ends_with(".png"))
                .map(str::to_string);
            Some(App {
                art_is_splash: splash.is_some(),
                art_path: splash.or(background),
                id,
                title,
                icon_path,
                bg_color: point.get("bgColor").and_then(Value::as_str).map(str::to_string),
                params: point.get("params").filter(|p| p.is_object()).cloned(),
            })
        })
        .collect()
}

/// Normalises EIM `getAllInputStatus`.
pub fn inputs_from_status(reply: &Value) -> Vec<Input> {
    let Some(devices) = reply.get("devices").and_then(Value::as_array) else {
        return Vec::new();
    };
    devices
        .iter()
        .filter_map(|device| {
            let id = device.get("id")?.as_str()?.to_string();
            let app_id = device.get("appId")?.as_str()?.to_string();
            let label = device
                .get("label")
                .and_then(Value::as_str)
                .filter(|label| !label.is_empty())
                .unwrap_or(&id)
                .to_string();
            Some(Input {
                id,
                label,
                app_id,
                connected: device.get("connected").and_then(Value::as_bool).unwrap_or(false),
            })
        })
        .collect()
}

/// Decodes a PNG, fits it into 128 px and finds its most saturated dominant colour.
pub fn decode_icon(app_id: &str, bytes: &[u8]) -> Result<Icon, String> {
    let (rgba, w, h) = decode_png(bytes)?;
    let (pixels, w, h) = fit(&rgba, w, h, 128);
    Ok(Icon {
        app_id: app_id.to_string(),
        w: w as u32,
        h: h as u32,
        color: dominant_color(&pixels),
        pixels,
    })
}

pub const ART_W: usize = 320;
pub const ART_H: usize = 180;

/// Decodes 16:9 artwork into a 320×180 tile image. Splash screens are a logo on a plain
/// background: the logo is located and the crop chosen so it fills the tile. A splash
/// without a detectable logo is rejected (the tile then uses the app icon).
pub fn decode_art(app_id: &str, bytes: &[u8], splash: bool) -> Result<Icon, String> {
    let (rgba, w, h) = decode_png(bytes)?;
    let (x0, y0, cw, ch) = if splash { logo_crop(&rgba, w, h)? } else { cover_crop(w, h) };
    let pixels = resample(&rgba, w, (x0, y0, cw, ch), ART_W, ART_H);
    Ok(Icon {
        app_id: app_id.to_string(),
        w: ART_W as u32,
        h: ART_H as u32,
        color: dominant_color(&pixels),
        pixels,
    })
}

/// Largest centred 16:9 region.
fn cover_crop(w: usize, h: usize) -> (usize, usize, usize, usize) {
    let (cw, ch) = if w * 9 >= h * 16 { (h * 16 / 9, h) } else { (w, w * 9 / 16) };
    ((w - cw) / 2, (h - ch) / 2, cw, ch)
}

/// Finds the logo (pixels clearly unlike the corner background) and frames it so it spans
/// about 58 % of the tile width or 52 % of its height.
fn logo_crop(rgba: &[u8], w: usize, h: usize) -> Result<(usize, usize, usize, usize), String> {
    let pixel = |x: usize, y: usize| &rgba[(y * w + x) * 4..][..4];
    let mut bg = [0f32; 3];
    for (cx, cy) in [(8, 8), (w - 9, 8), (8, h - 9), (w - 9, h - 9)] {
        let p = pixel(cx, cy);
        for c in 0..3 {
            bg[c] += p[c] as f32 / 4.0;
        }
    }
    let (mut x0, mut y0, mut x1, mut y1, mut hits) = (w, h, 0, 0, 0usize);
    for y in (0..h).step_by(4) {
        for x in (0..w).step_by(4) {
            let p = pixel(x, y);
            let diff = (0..3).map(|c| (p[c] as f32 - bg[c]).abs()).sum::<f32>() * p[3] as f32 / 255.0;
            if diff > 110.0 {
                hits += 1;
                x0 = x0.min(x);
                y0 = y0.min(y);
                x1 = x1.max(x);
                y1 = y1.max(y);
            }
        }
    }
    if hits < 40 {
        return Err("splash has no detectable logo".into());
    }
    let (bw, bh) = ((x1 - x0 + 4) as f32, (y1 - y0 + 4) as f32);
    if bw > w as f32 * 0.85 && bh > h as f32 * 0.85 {
        return Ok(cover_crop(w, h));
    }
    // Never zoom past 1:1 (blur) nor beyond the image.
    let cw = (bw / 0.58).max(bh / 0.52 * 16.0 / 9.0).clamp(ART_W as f32 * 1.4, w as f32).min(h as f32 * 16.0 / 9.0);
    let ch = cw * 9.0 / 16.0;
    let cx = (x0 + x1) as f32 / 2.0;
    let cy = (y0 + y1) as f32 / 2.0;
    let left = (cx - cw / 2.0).clamp(0.0, w as f32 - cw);
    let top = (cy - ch / 2.0).clamp(0.0, h as f32 - ch);
    Ok((left as usize, top as usize, cw as usize, ch as usize))
}

/// Fits any RGBA image into a 320×180 tile image by centred cover-cropping.
pub fn art_from_rgba(app_id: &str, rgba: &[u8], w: usize, h: usize) -> Icon {
    let crop = cover_crop(w, h);
    let pixels = resample(rgba, w, crop, ART_W, ART_H);
    Icon { app_id: app_id.to_string(), w: ART_W as u32, h: ART_H as u32, color: dominant_color(&pixels), pixels }
}

/// Decodes a JPEG (Plex artwork) to RGBA.
pub fn decode_jpeg(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let mut decoder = jpeg_decoder::Decoder::new(bytes);
    let pixels = decoder.decode().map_err(|e| e.to_string())?;
    let info = decoder.info().ok_or("jpeg without info")?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba = match info.pixel_format {
        jpeg_decoder::PixelFormat::RGB24 => pixels.chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        jpeg_decoder::PixelFormat::L8 => pixels.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        _ => return Err("unsupported jpeg format".into()),
    };
    Ok((rgba, w, h))
}

pub fn decode_png_rgba(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    decode_png(bytes)
}

fn decode_png(bytes: &[u8]) -> Result<(Vec<u8>, usize, usize), String> {
    let mut decoder = png::Decoder::new(bytes);
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder.read_info().map_err(|e| e.to_string())?;
    let mut buffer = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut buffer).map_err(|e| e.to_string())?;
    let (w, h) = (info.width as usize, info.height as usize);
    let rgba: Vec<u8> = match info.color_type {
        png::ColorType::Rgba => buffer[..w * h * 4].to_vec(),
        png::ColorType::Rgb => buffer[..w * h * 3].chunks_exact(3).flat_map(|p| [p[0], p[1], p[2], 255]).collect(),
        png::ColorType::GrayscaleAlpha => buffer[..w * h * 2].chunks_exact(2).flat_map(|p| [p[0], p[0], p[0], p[1]]).collect(),
        png::ColorType::Grayscale => buffer[..w * h].iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return Err("unexpanded palette".into()),
    };
    Ok((rgba, w, h))
}

/// Box-filter resample of a source region to nw×nh (premultiplied, so transparency is clean).
fn resample(rgba: &[u8], stride_w: usize, (sx, sy, sw, sh): (usize, usize, usize, usize), nw: usize, nh: usize) -> Vec<u8> {
    let mut out = vec![0u8; nw * nh * 4];
    for y in 0..nh {
        let (y0, y1) = (sy + y * sh / nh, (sy + (y + 1) * sh / nh).max(sy + y * sh / nh + 1));
        for x in 0..nw {
            let (x0, x1) = (sx + x * sw / nw, (sx + (x + 1) * sw / nw).max(sx + x * sw / nw + 1));
            let mut sum = [0u32; 4];
            for py in y0..y1 {
                for px in x0..x1 {
                    let p = &rgba[(py * stride_w + px) * 4..][..4];
                    let a = p[3] as u32;
                    sum[0] += p[0] as u32 * a;
                    sum[1] += p[1] as u32 * a;
                    sum[2] += p[2] as u32 * a;
                    sum[3] += a;
                }
            }
            let n = ((y1 - y0) * (x1 - x0)) as u32;
            let o = &mut out[(y * nw + x) * 4..][..4];
            if sum[3] > 0 {
                o[0] = (sum[0] / sum[3]) as u8;
                o[1] = (sum[1] / sum[3]) as u8;
                o[2] = (sum[2] / sum[3]) as u8;
            }
            o[3] = (sum[3] / n) as u8;
        }
    }
    out
}

/// Fits an image into max×max, keeping its aspect ratio.
fn fit(rgba: &[u8], w: usize, h: usize, max: usize) -> (Vec<u8>, usize, usize) {
    if w <= max && h <= max {
        return (rgba.to_vec(), w, h);
    }
    let scale = max as f32 / w.max(h) as f32;
    let (nw, nh) = (((w as f32 * scale).round() as usize).max(1), ((h as f32 * scale).round() as usize).max(1));
    (resample(rgba, w, (0, 0, w, h), nw, nh), nw, nh)
}

fn dominant_color(rgba: &[u8]) -> Option<[u8; 3]> {
    let (mut acc, mut weight) = ([0f32; 3], 0f32);
    let (mut plain, mut plain_n) = ([0f32; 3], 0f32);
    for p in rgba.chunks_exact(4) {
        if p[3] < 128 {
            continue;
        }
        let (r, g, b) = (p[0] as f32 / 255.0, p[1] as f32 / 255.0, p[2] as f32 / 255.0);
        let max = r.max(g).max(b);
        let min = r.min(g).min(b);
        plain = [plain[0] + r, plain[1] + g, plain[2] + b];
        plain_n += 1.0;
        if max < 0.12 {
            continue;
        }
        let saturation = (max - min) / max;
        let w = saturation * saturation * max;
        acc = [acc[0] + r * w, acc[1] + g * w, acc[2] + b * w];
        weight += w;
    }
    let pick = |c: [f32; 3], n: f32| [(c[0] / n * 255.0) as u8, (c[1] / n * 255.0) as u8, (c[2] / n * 255.0) as u8];
    if weight > 0.5 {
        Some(pick(acc, weight))
    } else if plain_n > 0.0 {
        Some(pick(plain, plain_n))
    } else {
        None
    }
}
