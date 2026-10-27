//! TiviOne for rooted and developer-mode TVs: the signed release manifest on TiviOne's
//! website, the pictures for its store page, and downloading, verifying and installing the IPK
//! (as root, through Homebrew Channel). Everything runs on short-lived threads of its own.
use crate::log::log;
use crate::model::{self, InstallFailure, InstallState, Package, Picture, Release, Update, TIVIONE_APP};
use crate::platform::Sink;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

const MANIFEST: &str = "https://tivione.zf-labs.de/download/latest.json";
/// Ed25519 key TiviOne's releases are signed with (tivione/scripts/direct-release.mjs).
const PUBLIC_KEY: &str = "MAePJJ1hYBXY9oKRgsamIyLIM2GqVmTj5XCoTT2Yxv4=";
const MAX_IPK: u64 = 64 << 20;
const MAX_PICTURE: u64 = 2 << 20;
const PICTURE_W: usize = 960;
const PICTURE_H: usize = 540;
pub const MAX_PICTURES: usize = 6;

static MANIFEST_URL: OnceLock<String> = OnceLock::new();
/// Vision's releases: the IPK and SHA256SUMS of the newest one (built by GitHub Actions).
const VISION_RELEASES: &str = "https://api.github.com/repos/fatgue/lg-vision-launcher/releases/latest";
static VISION_VERSION: OnceLock<String> = OnceLock::new();

/// Another manifest address (debug flag `--releases`): a test server before a release is live.
pub fn set_manifest(url: String) {
    let _ = MANIFEST_URL.set(url);
}

/// Another running version (debug flag `--vision-version`), to see the update offer.
pub fn set_vision_version(version: String) {
    let _ = VISION_VERSION.set(version);
}

/// Looks for the newest release; the answer arrives as `Update::Release`. `answered` is set
/// when the website answered, so the caller can retry a failed check sooner.
pub fn check(sink: Sink, answered: Arc<AtomicBool>) {
    spawn("release-check", move || {
        let release = fetch_release().map_err(|error| log!("tivione release: {error}")).ok();
        if let Some(release) = &release {
            log!("tivione release: {}", release.version);
        }
        answered.store(release.is_some(), Ordering::Release);
        sink.send(Update::Release(release));
        // Vision itself, from its GitHub releases. A failed check changes nothing.
        match vision_release() {
            Ok(package) => sink.send(Update::VisionRelease(package)),
            Err(error) => log!("vision release: {error}"),
        }
    });
}

fn vision_release() -> Result<Option<Package>, String> {
    let agent = crate::net::agent();
    let release = crate::net::get_json(&agent, VISION_RELEASES, &[])?;
    let current = VISION_VERSION.get().map_or(env!("CARGO_PKG_VERSION"), String::as_str);
    // Up to date: no need for the checksums.
    if vision_package(&release, None, current)?.is_none() {
        return Ok(None);
    }
    let (url, _) = asset(&release, "SHA256SUMS").ok_or("SHA256SUMS missing")?;
    let sums = crate::net::get_bytes(&agent, &url, &[], 16 << 10)?;
    vision_package(&release, Some(&String::from_utf8_lossy(&sums)), current)
}

/// A release's download address and size for one of its files.
fn asset(release: &Value, name: &str) -> Option<(String, Option<u64>)> {
    let asset = release.get("assets")?.as_array()?.iter().find(|a| a.get("name").and_then(Value::as_str) == Some(name))?;
    let url = asset.get("browser_download_url")?.as_str()?;
    url.starts_with("https://").then(|| (url.to_string(), asset.get("size").and_then(Value::as_u64)))
}

/// The package of a GitHub release newer than `current`, with its checksum from SHA256SUMS
/// (`sums` None: only whether there is one; the checksum is left empty).
fn vision_package(release: &Value, sums: Option<&str>, current: &str) -> Result<Option<Package>, String> {
    let version = release.get("tag_name").and_then(Value::as_str).unwrap_or_default().trim_start_matches('v').to_string();
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(format!("bad version {version:?}"));
    }
    if !model::version_newer(&version, current) || release.get("draft").and_then(Value::as_bool) == Some(true) {
        return Ok(None);
    }
    let name = format!("{}_{version}_arm.ipk", crate::platform::APP_ID);
    let (url, size) = asset(release, &name).ok_or_else(|| format!("{name} missing"))?;
    let mut sha256 = [0u8; 32];
    let Some(sums) = sums else {
        return Ok(Some(Package { app: crate::platform::APP_ID.to_string(), version, url, sha256, size }));
    };
    let hex = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let (hash, file) = (parts.next()?, parts.next()?);
            (file.trim_start_matches('*') == name).then_some(hash)
        })
        .ok_or("no checksum")?;
    if hex.len() != 64 {
        return Err("bad checksum".into());
    }
    for (i, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| "bad checksum")?;
    }
    Ok(Some(Package { app: crate::platform::APP_ID.to_string(), version, url, sha256, size: size.filter(|&s| s > 0 && s <= MAX_IPK) }))
}

fn fetch_release() -> Result<Release, String> {
    let url = MANIFEST_URL.get().map_or(MANIFEST, String::as_str);
    let agent = crate::net::agent();
    let body = crate::net::get_bytes(&agent, url, &[], 64 << 10)?;
    let signature = crate::net::get_bytes(&agent, &format!("{url}.sig"), &[], 1024)?;
    let b64 = base64::engine::general_purpose::STANDARD;
    let signature = b64.decode(String::from_utf8_lossy(&signature).trim()).map_err(|e| e.to_string())?;
    let key = b64.decode(PUBLIC_KEY).map_err(|e| e.to_string())?;
    let key = ed25519_compact::PublicKey::from_slice(&key).map_err(|e| e.to_string())?;
    let signature = ed25519_compact::Signature::from_slice(&signature).map_err(|e| e.to_string())?;
    key.verify(&body, &signature).map_err(|_| "manifest signature does not match".to_string())?;

    let manifest: Value = serde_json::from_slice(&body).map_err(|e| e.to_string())?;
    let text = |key: &str| manifest.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    if text("app") != TIVIONE_APP {
        return Err("manifest is for another app".into());
    }
    let version = text("version");
    if version.is_empty() || !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err("bad version".into());
    }
    let hex = text("sha256");
    let mut sha256 = [0u8; 32];
    if hex.len() != 64 {
        return Err("bad checksum".into());
    }
    for (i, byte) in sha256.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).map_err(|_| "bad checksum")?;
    }
    let size = manifest.get("size").and_then(Value::as_u64).filter(|&s| s > 0 && s <= MAX_IPK).ok_or("bad size")?;
    let store = manifest.get("store");
    let banner = store.and_then(|s| s.get("banner")).and_then(Value::as_str).map(str::to_string);
    let screenshots = store
        .and_then(|s| s.get("screenshots"))
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(|v| v.as_str().map(str::to_string)).collect::<Vec<_>>())
        .unwrap_or_default();
    let pictures = banner.into_iter().chain(screenshots).take(MAX_PICTURES).collect();
    Ok(Release { version, url: text("url"), sha256, size, pictures })
}

/// Loads the store page's pictures (cached on disk) and sends them decoded, in order.
pub fn pictures(release: Release, data_dir: PathBuf, sink: Sink) {
    spawn("store-pictures", move || {
        let dir = data_dir.join("store");
        let _ = std::fs::create_dir_all(&dir);
        let agent = crate::net::agent();
        for (index, url) in release.pictures.iter().enumerate() {
            let name: String = url.rsplit('/').next().unwrap_or_default().chars().filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '-').collect();
            let path = dir.join(format!("{index}-{name}"));
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => match crate::net::get_bytes(&agent, url, &[], MAX_PICTURE) {
                    Ok(bytes) => {
                        let _ = std::fs::write(&path, &bytes);
                        bytes
                    }
                    Err(error) => {
                        log!("store picture {index}: {error}");
                        continue;
                    }
                },
            };
            match model::decode_jpeg(&bytes) {
                Ok((rgba, w, h)) if w <= PICTURE_W && h <= PICTURE_H => sink.send(Update::Picture(Picture { index, rgba, w, h })),
                Ok((_, w, h)) => log!("store picture {index}: {w}x{h} is too large"),
                Err(error) => {
                    log!("store picture {index}: {error}");
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    });
}

/// How install progress reaches the UI (TiviOne's page or the Homebrew page).
pub type Report = fn(&str, InstallState) -> Update;

/// Downloads, verifies (SHA-256) and installs a package, reporting progress through `report`.
pub fn install(package: Package, data_dir: PathBuf, sink: Sink, report: Report) {
    spawn("install", move || {
        let dir = data_dir.join("download");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("package.ipk");
        let send = |state| sink.send(report(&package.app, state));
        let result = download(&package, &path, &send).and_then(|()| {
            send(InstallState::Installing);
            install_package(&path)
        });
        let _ = std::fs::remove_file(&path);
        match result {
            Ok(()) => {
                log!("{} {} installed", package.app, package.version);
                send(InstallState::Done);
                if package.app == crate::platform::APP_ID {
                    restart_vision();
                }
            }
            Err(failure) => {
                log!("{} install: {failure:?}", package.app);
                send(InstallState::Failed(failure));
            }
        }
    });
}

fn download(package: &Package, path: &Path, send: &dyn Fn(InstallState)) -> Result<(), InstallFailure> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Vision/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent.get(&package.url).call().map_err(|error| {
        log!("download {}: {error}", package.app);
        InstallFailure::Network
    })?;
    let expected = package.size.or_else(|| response.header("Content-Length").and_then(|v| v.parse().ok()));
    let mut reader = response.into_reader().take(expected.map_or(MAX_IPK, |s| s.min(MAX_IPK)) + 1);
    let mut file = std::fs::File::create(path).map_err(|_| InstallFailure::Network)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 << 10];
    let (mut total, mut reported) = (0u64, -1i32);
    send(InstallState::Downloading(0.0));
    loop {
        let n = reader.read(&mut buffer).map_err(|_| InstallFailure::Network)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|_| InstallFailure::Network)?;
        total += n as u64;
        if let Some(size) = expected {
            let percent = (total * 100 / size.max(1)).min(100) as i32;
            if percent >= reported + 5 {
                reported = percent;
                send(InstallState::Downloading(percent as f32 / 100.0));
            }
        }
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if package.size.is_some_and(|size| total != size) || total > MAX_IPK || digest != package.sha256 {
        return Err(InstallFailure::Damaged);
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn install_package(path: &Path) -> Result<(), InstallFailure> {
    use crate::luna::Luna;
    use serde_json::json;
    // The jailed app cannot reach the installer; root can. luna-send keeps its subscription
    // open, so it runs in the background and its output is polled from a file.
    const OUTPUT: &str = "/tmp/vision-install.txt";
    const PID: &str = "/tmp/vision-install.pid";
    let exec = |luna: &mut Luna, command: &str| -> Result<(), String> {
        let reply = luna.call("luna://org.webosbrew.hbchannel.service/exec", &json!({ "command": command }))?;
        match reply.get("returnValue").and_then(Value::as_bool) {
            Some(true) => Ok(()),
            _ => Err(reply.to_string()),
        }
    };
    let mut luna = Luna::connect().map_err(|_| InstallFailure::NeedsRoot)?;
    let payload = json!({ "id": "com.ares.defaultName", "ipkUrl": path.to_string_lossy(), "subscribe": true }).to_string();
    let command = format!(
        "rm -f {OUTPUT}; (luna-send -i -w 180000 luna://com.webos.appInstallService/dev/install '{payload}' > {OUTPUT} 2>&1 & echo $! > {PID})"
    );
    exec(&mut luna, &command).map_err(|error| {
        log!("tivione install: {error}");
        InstallFailure::NeedsRoot
    })?;
    let started = std::time::Instant::now();
    let outcome = loop {
        std::thread::sleep(Duration::from_millis(500));
        let output: String = std::fs::read_to_string(OUTPUT).unwrap_or_default().chars().filter(|c| !c.is_whitespace()).collect();
        if output.contains("\"state\":\"installed\"") {
            break Ok(());
        }
        if output.contains("FAILED") || output.contains("\"returnValue\":false") {
            log!("tivione install: {output}");
            break Err(InstallFailure::Refused);
        }
        if started.elapsed() > Duration::from_secs(180) {
            break Err(InstallFailure::Refused);
        }
    };
    let _ = exec(&mut luna, &format!("kill $(cat {PID}) 2>/dev/null; rm -f {OUTPUT} {PID}"));
    outcome
}

/// Uninstalls an app (developer install) as root, reporting through `Update::HomebrewInstall`.
pub fn remove(app: String, sink: Sink) {
    spawn("uninstall", move || {
        let state = if model::valid_app_id(&app) { remove_package(&app) } else { Err(InstallFailure::Refused) };
        let state = match state {
            Ok(()) => InstallState::Done,
            Err(failure) => InstallState::Failed(failure),
        };
        log!("uninstall {app}: {state:?}");
        sink.send(Update::HomebrewInstall { app, state });
    });
}

#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn remove_package(app: &str) -> Result<(), InstallFailure> {
    use crate::luna::Luna;
    use serde_json::json;
    let mut luna = Luna::connect().map_err(|_| InstallFailure::NeedsRoot)?;
    let mut exec = |command: &str| -> Result<String, InstallFailure> {
        let reply = luna.call("luna://org.webosbrew.hbchannel.service/exec", &json!({ "command": command })).map_err(|_| InstallFailure::NeedsRoot)?;
        match reply.get("returnValue").and_then(Value::as_bool) {
            Some(true) => Ok(reply.get("stdoutString").and_then(Value::as_str).unwrap_or_default().to_string()),
            _ => Err(InstallFailure::NeedsRoot),
        }
    };
    // `app` is a checked id (letters, digits, '.', '-', '_'), safe inside the command.
    let payload = json!({ "id": app, "subscribe": true }).to_string();
    exec(&format!("luna-send -n 4 -w 30000 luna://com.webos.appInstallService/dev/remove '{payload}' >/dev/null 2>&1; true"))?;
    let dir = format!("/media/developer/apps/usr/palm/applications/{app}");
    for _ in 0..20 {
        if exec(&format!("[ -d '{dir}' ] && echo present || echo gone"))?.trim() == "gone" {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(500));
    }
    Err(InstallFailure::Refused)
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn remove_package(_app: &str) -> Result<(), InstallFailure> {
    log!("uninstall (desktop): only on the TV");
    Err(InstallFailure::NeedsRoot)
}

const HOMEBREW_INDEX: &str = "https://repo.webosbrew.org/api/apps.json";

/// Apps outside the repository, listed after its featured apps: each one's release manifest and
/// the oldest webOS (major version) it supports. On an older TV they show, greyed out.
const EXTRAS: &[(&str, u32)] = &[(
    "https://github.com/JulioFerrero/own-your-glass/releases/latest/download/org.ownyourglass.control.manifest.json",
    10,
)];

fn extras(agent: &ureq::Agent) -> Vec<model::HomebrewApp> {
    let webos = webos_major();
    EXTRAS
        .iter()
        .filter_map(|&(url, oldest)| {
            let manifest = crate::net::get_json(agent, url, &[]).map_err(|error| log!("homebrew: {url}: {error}")).ok()?;
            let unsupported = (webos.is_none_or(|major| major < oldest)).then_some((oldest, webos));
            model::extra_app(&manifest, unsupported)
        })
        .collect()
}

/// The TV's webOS major version (`webos_release` in nyx's os_info), None when unknown.
#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn webos_major() -> Option<u32> {
    let info: Value = serde_json::from_slice(&std::fs::read("/var/run/nyx/os_info.json").ok()?).ok()?;
    info.get("webos_release")?.as_str()?.split('.').next()?.trim().parse().ok()
}

/// Desktop: `VISION_WEBOS=<major>` plays a TV with that webOS.
#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn webos_major() -> Option<u32> {
    std::env::var("VISION_WEBOS").ok()?.parse().ok()
}

/// Loads the Homebrew repository, then each app's icon (cached on disk).
pub fn homebrew(data_dir: PathBuf, sink: Sink) {
    spawn("homebrew", move || {
        let agent = crate::net::agent();
        let apps = crate::net::get_json(&agent, HOMEBREW_INDEX, &[]).map(|index| {
            let mut apps = model::homebrew_apps(&index);
            let mut at = apps.iter().take_while(|a| a.featured).count();
            for extra in extras(&agent) {
                // Once the repository lists it, its entry wins.
                if !apps.iter().any(|a| a.id == extra.id) {
                    apps.insert(at, extra);
                    at += 1;
                }
            }
            apps
        });
        let icons: Vec<(String, String)> = apps.as_ref().map(|a| a.iter().map(|app| (app.id.clone(), app.icon_url.clone())).collect()).unwrap_or_default();
        match &apps {
            Ok(list) => log!("homebrew: {} apps", list.len()),
            Err(error) => log!("homebrew: {error}"),
        }
        sink.send(Update::Homebrew(apps));
        let dir = data_dir.join("homebrew-icons");
        let _ = std::fs::create_dir_all(&dir);
        for (id, url) in icons {
            if url.is_empty() {
                continue;
            }
            let path = dir.join(format!("{id}.img"));
            let bytes = match std::fs::read(&path) {
                Ok(bytes) => bytes,
                Err(_) => match crate::net::get_bytes(&agent, &url, &[], 1 << 20) {
                    Ok(bytes) => {
                        let _ = std::fs::write(&path, &bytes);
                        bytes
                    }
                    Err(_) => continue,
                },
            };
            match model::decode_icon(&id, &bytes) {
                Ok(icon) => sink.send(Update::HomebrewIcon(icon)),
                Err(_) => {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    });
}

/// After a Vision update: as Home the mount still holds the old executable, so home.sh (from the
/// new package) mounts the new one and restarts Home; otherwise Vision starts again. Detached,
/// because both end this process.
#[cfg(all(target_os = "linux", target_arch = "arm"))]
fn restart_vision() {
    use crate::luna::Luna;
    use serde_json::json;
    const APP_DIR: &str = "/media/developer/apps/usr/palm/applications/com.vision.launcher";
    let command = format!(
        "(sleep 1; if [ -f /var/run/vision-home ]; then sh {APP_DIR}/home.sh refresh; \
         else luna-send -n 1 luna://com.webos.applicationManager/launch '{{\"id\":\"com.vision.launcher\"}}'; fi) </dev/null >/dev/null 2>&1 &"
    );
    match Luna::connect().and_then(|mut luna| luna.call("luna://org.webosbrew.hbchannel.service/exec", &json!({ "command": command }))) {
        Ok(_) => log!("vision update: restarting"),
        Err(error) => log!("vision update: restart: {error}"),
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn restart_vision() {
    log!("vision update (desktop): would restart");
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
fn install_package(_path: &Path) -> Result<(), InstallFailure> {
    log!("install (desktop): only on the TV");
    Err(InstallFailure::NeedsRoot)
}

fn spawn(name: &str, job: impl FnOnce() + Send + 'static) {
    if let Err(error) = std::thread::Builder::new().name(name.into()).spawn(job) {
        log!("{name}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_a_newer_release_with_its_checksum() {
        let release: Value = serde_json::json!({
            "tag_name": "v0.2.2",
            "assets": [
                { "name": "com.vision.launcher_0.2.2_arm.ipk", "size": 3900000, "browser_download_url": "https://github.com/x/com.vision.launcher_0.2.2_arm.ipk" },
                { "name": "SHA256SUMS", "size": 100, "browser_download_url": "https://github.com/x/SHA256SUMS" },
            ],
        });
        let hash = "ab".repeat(32);
        let sums = format!("{hash}  com.vision.launcher_0.2.2_arm.ipk\n");
        let package = vision_package(&release, Some(&sums), "0.2.1").unwrap().unwrap();
        assert_eq!((package.version.as_str(), package.size, package.sha256[0]), ("0.2.2", Some(3900000), 0xab));
        assert_eq!(vision_package(&release, Some(&sums), "0.2.2").unwrap(), None);
        assert!(vision_package(&release, None, "0.2.1").unwrap().is_some());
        assert!(vision_package(&release, Some(""), "0.2.1").is_err());
    }
}
