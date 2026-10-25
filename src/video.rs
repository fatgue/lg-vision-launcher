//! Live preview of a channel in the hero. On webOS the stream plays on the hardware video plane
//! through the TV's media server (`com.webos.media`); SDL-webOS exports a window whose id binds
//! that plane to our surface, and the UI leaves the preview rectangle transparent.
//! Elsewhere this is a no-op.
use crate::log::log;

#[derive(Clone, Debug, PartialEq)]
pub struct Stream {
    pub url: String,
    pub user_agent: Option<String>,
    pub referer: Option<String>,
}

#[cfg(all(target_os = "linux", target_arch = "arm"))]
mod imp {
    use super::*;
    use crate::luna::Luna;
    use crate::render::Rect;
    use serde_json::{json, Value};
    use std::ffi::{c_char, c_int, CStr};
    use std::path::PathBuf;
    use std::sync::mpsc::{self, Sender};

    #[repr(C)]
    struct SdlRect {
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
    }

    /// SDL_WEBOS_EXPORTED_WINDOW_TYPE_VIDEO
    const EXPORTED_VIDEO: c_int = 0;

    enum Command {
        Show(Stream),
        Hide,
    }

    pub struct Video {
        sdl_paths: Vec<PathBuf>,
        rect: Rect,
        app_id: &'static str,
        /// Exists while the exported window is being created; a leftover one means that
        /// attempt hung or crashed, so previews stay off instead of risking the Home key.
        guard: PathBuf,
        tx: std::cell::RefCell<Option<Sender<Command>>>,
        failed: std::cell::Cell<bool>,
    }

    impl Video {
        /// Nothing is created yet: SDL-webOS waits for the compositor to assign the exported
        /// window's id, which it only does once our surface is on screen. `app_id` is the
        /// identity the media server attributes playback to.
        pub fn start(sdl_paths: &[PathBuf], rect: Rect, app_id: &'static str, data_dir: &std::path::Path) -> Self {
            let guard = data_dir.join("preview-starting");
            let failed = guard.exists();
            if failed {
                log!("preview: off, the last attempt to start it did not finish");
            }
            Self {
                sdl_paths: sdl_paths.to_vec(),
                rect,
                app_id,
                guard,
                tx: std::cell::RefCell::new(None),
                failed: std::cell::Cell::new(failed),
            }
        }

        /// Creates the exported window and the media worker on first use (main thread).
        fn ensure(&self) -> bool {
            if self.tx.borrow().is_some() {
                return true;
            }
            if self.failed.get() {
                return false;
            }
            let _ = std::fs::write(&self.guard, b"1");
            match Self::export(&self.sdl_paths, self.rect) {
                Ok(window_id) => {
                    let _ = std::fs::remove_file(&self.guard);
                    log!("preview: exported window {window_id}");
                    let (tx, rx) = mpsc::channel();
                    let app_id = self.app_id;
                    let _ = std::thread::Builder::new().name("preview".into()).spawn(move || worker(rx, window_id, app_id));
                    *self.tx.borrow_mut() = Some(tx);
                    true
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&self.guard);
                    log!("preview unavailable: {error}");
                    self.failed.set(true);
                    false
                }
            }
        }

        fn export(sdl_paths: &[PathBuf], rect: Rect) -> Result<String, String> {
            // SAFETY: the SDL library is already loaded by the app; this only adds a reference.
            let lib = sdl_paths
                .iter()
                .find_map(|p| unsafe { libloading::Library::new(p) }.ok())
                .ok_or("SDL library not found")?;
            unsafe {
                let create = *lib
                    .get::<unsafe extern "C" fn(c_int) -> *const c_char>(b"SDL_webOSCreateExportedWindow\0")
                    .map_err(|e| e.to_string())?;
                let set = *lib
                    .get::<unsafe extern "C" fn(*const c_char, *mut SdlRect, *mut SdlRect) -> c_int>(b"SDL_webOSSetExportedWindow\0")
                    .map_err(|e| e.to_string())?;
                let id = create(EXPORTED_VIDEO);
                if id.is_null() {
                    return Err("no exported window".into());
                }
                let mut dst = SdlRect { x: rect.x as c_int, y: rect.y as c_int, w: rect.w as c_int, h: rect.h as c_int };
                set(id, std::ptr::null_mut(), &mut dst);
                let window_id = CStr::from_ptr(id).to_string_lossy().into_owned();
                // The exported window lives as long as the process; keep the library loaded.
                std::mem::forget(lib);
                Ok(window_id)
            }
        }

        pub fn show(&self, stream: Stream) {
            if self.ensure() {
                if let Some(tx) = self.tx.borrow().as_ref() {
                    let _ = tx.send(Command::Show(stream));
                }
            }
        }

        pub fn hide(&self) {
            if let Some(tx) = self.tx.borrow().as_ref() {
                let _ = tx.send(Command::Hide);
            }
        }
    }

    fn worker(rx: mpsc::Receiver<Command>, window_id: String, app_id: &'static str) {
        let mut luna = match Luna::connect() {
            Ok(luna) => luna,
            Err(error) => return log!("preview luna: {error}"),
        };
        let mut media: Option<String> = None;
        while let Ok(command) = rx.recv() {
            // Only the latest request matters: skip what is already outdated.
            let mut command = command;
            while let Ok(newer) = rx.try_recv() {
                command = newer;
            }
            if let Some(id) = media.take() {
                let _ = luna.call("luna://com.webos.media/unload", &json!({ "mediaId": id }));
            }
            let Command::Show(stream) = command else { continue };
            let transport = if stream.url.to_ascii_lowercase().contains(".m3u8") { "HLS" } else { "URI" };
            let mut option = json!({
                "appId": app_id,
                "windowId": window_id,
                "mediaTransportType": transport,
            });
            let mut headers = serde_json::Map::new();
            if let Some(ua) = &stream.user_agent {
                headers.insert("User-Agent".into(), Value::String(ua.clone()));
            }
            if let Some(referer) = &stream.referer {
                headers.insert("Referer".into(), Value::String(referer.clone()));
            }
            if !headers.is_empty() {
                option["transmission"] = json!({ "httpHeader": headers });
            }
            let payload = json!({ "uri": stream.url, "type": "media", "payload": { "option": option } });
            match luna.call("luna://com.webos.media/load", &payload) {
                Ok(reply) => match reply.get("mediaId").and_then(Value::as_str) {
                    Some(id) => {
                        let id = id.to_string();
                        // Muted: a preview should never talk over the room.
                        let _ = luna.call("luna://com.webos.media/setVolume", &json!({ "mediaId": id, "volume": 0 }));
                        if let Err(error) = luna.call("luna://com.webos.media/play", &json!({ "mediaId": id })) {
                            log!("preview play: {error}");
                        }
                        media = Some(id);
                    }
                    None => log!("preview load refused: {reply}"),
                },
                Err(error) => log!("preview load: {error}"),
            }
        }
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
mod imp {
    use super::*;
    use crate::render::Rect;
    use std::path::PathBuf;

    pub struct Video;

    impl Video {
        pub fn start(_sdl_paths: &[PathBuf], _rect: Rect, _app_id: &'static str, _data_dir: &std::path::Path) -> Self {
            Self
        }

        pub fn show(&self, stream: Stream) {
            // Never log the address: IPTV stream URLs carry the provider login.
            let _ = stream;
            log!("preview (desktop): would play a channel");
        }

        pub fn hide(&self) {}
    }
}

pub use imp::Video;
