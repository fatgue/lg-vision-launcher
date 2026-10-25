//! TiviOne for rooted and developer-mode TVs: the signed release manifest on TiviOne's
//! website, the pictures for its store page, and downloading, verifying and installing the IPK
//! (as root, through Homebrew Channel). Everything runs on short-lived threads of its own.
use crate::log::log;
use crate::model::{self, InstallFailure, InstallState, Picture, Release, Update, TIVIONE_APP};
use crate::platform::Sink;
use base64::Engine;
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
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

/// Another manifest address (debug flag `--releases`): a test server before a release is live.
pub fn set_manifest(url: String) {
    let _ = MANIFEST_URL.set(url);
}

/// Looks for the newest release; the answer arrives as `Update::Release`.
pub fn check(sink: Sink) {
    spawn("release-check", move || {
        let release = fetch_release().map_err(|error| log!("tivione release: {error}")).ok();
        if let Some(release) = &release {
            log!("tivione release: {}", release.version);
        }
        sink.send(Update::Release(release));
    });
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

/// Downloads, verifies and installs the release, reporting progress as `Update::Install`.
pub fn install(release: Release, data_dir: PathBuf, sink: Sink) {
    spawn("install", move || {
        let dir = data_dir.join("download");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("tivione.ipk");
        let result = download(&release, &path, &sink).and_then(|()| {
            sink.send(Update::Install(InstallState::Installing));
            install_package(&path)
        });
        let _ = std::fs::remove_file(&path);
        match result {
            Ok(()) => {
                log!("tivione {} installed", release.version);
                sink.send(Update::Install(InstallState::Done));
            }
            Err(failure) => {
                log!("tivione install: {failure:?}");
                sink.send(Update::Install(InstallState::Failed(failure)));
            }
        }
    });
}

fn download(release: &Release, path: &Path, sink: &Sink) -> Result<(), InstallFailure> {
    let agent = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout_read(Duration::from_secs(30))
        .user_agent(concat!("Vision/", env!("CARGO_PKG_VERSION")))
        .build();
    let response = agent.get(&release.url).call().map_err(|error| {
        log!("tivione download: {error}");
        InstallFailure::Network
    })?;
    let mut reader = response.into_reader().take(release.size + 1);
    let mut file = std::fs::File::create(path).map_err(|_| InstallFailure::Network)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; 64 << 10];
    let (mut total, mut reported) = (0u64, -1i32);
    sink.send(Update::Install(InstallState::Downloading(0.0)));
    loop {
        let n = reader.read(&mut buffer).map_err(|_| InstallFailure::Network)?;
        if n == 0 {
            break;
        }
        hasher.update(&buffer[..n]);
        file.write_all(&buffer[..n]).map_err(|_| InstallFailure::Network)?;
        total += n as u64;
        let percent = (total * 100 / release.size.max(1)).min(100) as i32;
        if percent >= reported + 5 {
            reported = percent;
            sink.send(Update::Install(InstallState::Downloading(percent as f32 / 100.0)));
        }
    }
    let digest: [u8; 32] = hasher.finalize().into();
    if total != release.size || digest != release.sha256 {
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
