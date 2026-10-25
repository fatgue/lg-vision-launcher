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
    use crate::luna::{Luna, Remote};
    use crate::render::Rect;
    use crate::sdl;
    use serde_json::{json, Value};
    use std::cell::{Cell, RefCell};
    use std::ffi::{c_char, c_int, c_void, CStr};
    use std::path::{Path, PathBuf};
    use std::sync::mpsc::{self, Sender};

    #[repr(C)]
    struct SdlRect {
        x: c_int,
        y: c_int,
        w: c_int,
        h: c_int,
    }

    /// SDL_SysWMinfo (SDL 2.30, SDL_syswm.h); for Wayland the union starts with the wl_display.
    #[repr(C)]
    struct WmInfo {
        version: [u8; 3],
        subsystem: c_int,
        info: [*mut c_void; 16],
    }

    /// SDL_WEBOS_EXPORTED_WINDOW_TYPE_VIDEO
    const EXPORTED_VIDEO: c_int = 0;
    /// The dev-app LS2 role only lets `com.webos.media.client.*` names reach the media server.
    const MEDIA_CLIENT: &str = "com.webos.media.client.vision";

    enum Command {
        Show(Stream),
        /// Stop; the sender (if any) hears back once the pipeline is gone.
        Hide(Option<mpsc::SyncSender<()>>),
    }

    pub struct Video {
        sdl_paths: Vec<PathBuf>,
        window: sdl::Window,
        rect: Rect,
        app_id: &'static str,
        /// As Home: the registerApp connection, the only LS2 name LG's role allows.
        bus: Option<Remote>,
        /// Exists while the exported window is being created; a leftover one means that
        /// attempt hung or crashed, so previews stay off instead of risking the Home key.
        guard: PathBuf,
        tx: RefCell<Option<Sender<Command>>>,
        failed: Cell<bool>,
    }

    impl Video {
        /// Nothing is created yet: SDL-webOS waits for the compositor to assign the exported
        /// window's id, which it only does once our surface is on screen. `app_id` is the
        /// identity the media server attributes playback to.
        pub fn start(sdl_paths: &[PathBuf], window: sdl::Window, rect: Rect, app_id: &'static str, data_dir: &Path, bus: Option<super::Bus>) -> Self {
            let guard = data_dir.join("preview-starting");
            let failed = guard.exists();
            if failed {
                log!("preview: off, the last attempt to start it did not finish");
            }
            Self {
                sdl_paths: sdl_paths.to_vec(),
                window,
                rect,
                app_id,
                bus,
                guard,
                tx: RefCell::new(None),
                failed: Cell::new(failed),
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
            let exported = self.export();
            let _ = std::fs::remove_file(&self.guard);
            match exported {
                Ok(window_id) => {
                    log!("preview: exported window {window_id}");
                    let (tx, rx) = mpsc::channel();
                    let (app_id, bus) = (self.app_id, self.bus.clone());
                    let _ = std::thread::Builder::new().name("preview".into()).spawn(move || worker(rx, window_id, app_id, bus));
                    *self.tx.borrow_mut() = Some(tx);
                    true
                }
                Err(error) => {
                    log!("preview unavailable: {error}");
                    self.failed.set(true);
                    false
                }
            }
        }

        fn export(&self) -> Result<String, String> {
            // SAFETY: the SDL library is already loaded by the app; this only adds a reference.
            let lib = self
                .sdl_paths
                .iter()
                .find_map(|p| unsafe { libloading::Library::new(p) }.ok())
                .ok_or("SDL library not found")?;
            unsafe {
                let wm_info = *lib
                    .get::<unsafe extern "C" fn(sdl::Window, *mut WmInfo) -> c_int>(b"SDL_GetWindowWMInfo\0")
                    .map_err(|e| e.to_string())?;
                let create = *lib
                    .get::<unsafe extern "C" fn(c_int) -> *const c_char>(b"SDL_webOSCreateExportedWindow\0")
                    .map_err(|e| e.to_string())?;
                let set = *lib
                    .get::<unsafe extern "C" fn(*const c_char, *mut SdlRect, *mut SdlRect) -> c_int>(b"SDL_webOSSetExportedWindow\0")
                    .map_err(|e| e.to_string())?;
                // SDL-webOS uses its export table unchecked when the compositor does not offer
                // wl_webos_foreign: ask the compositor first.
                let mut info = WmInfo { version: [2, 30, 12], subsystem: 0, info: [std::ptr::null_mut(); 16] };
                if wm_info(self.window, &mut info) == 0 || info.info[0].is_null() {
                    return Err("no Wayland display".into());
                }
                if !foreign_offered(info.info[0])? {
                    return Err("the compositor offers no exported windows".into());
                }
                let id = create(EXPORTED_VIDEO);
                if id.is_null() {
                    return Err("no exported window".into());
                }
                // Both regions spelled out: older SDL-webOS builds hand a missing one to
                // libwayland as NULL, which aborts the process.
                let mut src = SdlRect { x: 0, y: 0, w: crate::render::LOGICAL_W as c_int, h: crate::render::LOGICAL_H as c_int };
                let r = self.rect;
                let mut dst = SdlRect { x: r.x as c_int, y: r.y as c_int, w: r.w as c_int, h: r.h as c_int };
                set(id, &mut src, &mut dst);
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
                let _ = tx.send(Command::Hide(None));
            }
        }

        /// Stops the preview and waits (briefly) until the media server has let go of the
        /// decoder, so an app started next never competes with it for the video hardware.
        pub fn release(&self) {
            let Some(tx) = self.tx.borrow().as_ref().cloned() else { return };
            let (done, wait) = mpsc::sync_channel(1);
            if tx.send(Command::Hide(Some(done))).is_ok() && wait.recv_timeout(std::time::Duration::from_millis(1500)).is_err() {
                log!("preview: release timed out");
            }
        }
    }

    /// Lists the compositor's globals on SDL's own connection (main thread, where SDL
    /// dispatches it) and reports whether `wl_webos_foreign` is among them.
    unsafe fn foreign_offered(display: *mut c_void) -> Result<bool, String> {
        type Marshal = unsafe extern "C" fn(*mut c_void, u32, *const c_void, ...) -> *mut c_void;
        extern "C" fn global(data: *mut c_void, _registry: *mut c_void, _name: u32, interface: *const c_char, _version: u32) {
            // SAFETY: libwayland passes a NUL-terminated name; data is the flag below.
            if unsafe { CStr::from_ptr(interface) }.to_bytes() == b"wl_webos_foreign" {
                unsafe { *(data as *mut bool) = true };
            }
        }
        extern "C" fn global_remove(_data: *mut c_void, _registry: *mut c_void, _name: u32) {}
        // wl_registry_listener: two function pointers in event order.
        static LISTENER: [unsafe extern "C" fn(); 2] = unsafe {
            [
                std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32, *const c_char, u32), unsafe extern "C" fn()>(global),
                std::mem::transmute::<extern "C" fn(*mut c_void, *mut c_void, u32), unsafe extern "C" fn()>(global_remove),
            ]
        };
        let wl = libloading::Library::new("libwayland-client.so.0").map_err(|e| e.to_string())?;
        let marshal = *wl.get::<Marshal>(b"wl_proxy_marshal_constructor\0").map_err(|e| e.to_string())?;
        let add_listener = *wl
            .get::<unsafe extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> c_int>(b"wl_proxy_add_listener\0")
            .map_err(|e| e.to_string())?;
        let destroy = *wl.get::<unsafe extern "C" fn(*mut c_void)>(b"wl_proxy_destroy\0").map_err(|e| e.to_string())?;
        let roundtrip = *wl.get::<unsafe extern "C" fn(*mut c_void) -> c_int>(b"wl_display_roundtrip\0").map_err(|e| e.to_string())?;
        let registry_interface = *wl.get::<*const c_void>(b"wl_registry_interface\0").map_err(|e| e.to_string())?;
        let mut offered = false;
        // wl_display.get_registry is request 1; its only argument is the new object.
        let registry = marshal(display, 1, registry_interface, std::ptr::null_mut::<c_void>());
        if registry.is_null() {
            return Err("no Wayland registry".into());
        }
        add_listener(registry, LISTENER.as_ptr().cast(), (&mut offered as *mut bool).cast());
        let ok = roundtrip(display) >= 0;
        destroy(registry);
        // SDL keeps libwayland loaded anyway.
        std::mem::forget(wl);
        if ok {
            Ok(offered)
        } else {
            Err("Wayland roundtrip failed".into())
        }
    }

    enum Bus {
        Shared(Remote),
        Own(Luna),
    }

    impl Bus {
        fn call(&mut self, uri: &str, payload: &Value) -> Result<Value, String> {
            match self {
                Bus::Shared(remote) => remote.call(uri, payload),
                Bus::Own(luna) => luna.call(uri, payload),
            }
        }
    }

    fn worker(rx: mpsc::Receiver<Command>, window_id: String, app_id: &'static str, bus: Option<Remote>) {
        let mut bus = match bus {
            Some(remote) => Bus::Shared(remote),
            None => match Luna::connect_named(Some(MEDIA_CLIENT)) {
                Ok(luna) => Bus::Own(luna),
                Err(error) => return log!("preview luna: {error}"),
            },
        };
        let mut media: Option<String> = None;
        while let Ok(command) = rx.recv() {
            // Only the latest request matters: skip what is already outdated.
            let mut command = command;
            while let Ok(newer) = rx.try_recv() {
                command = newer;
            }
            if let Some(id) = media.take() {
                let _ = bus.call("luna://com.webos.media/unload", &json!({ "mediaId": id }));
            }
            let stream = match command {
                Command::Show(stream) => stream,
                Command::Hide(done) => {
                    if let Some(done) = done {
                        let _ = done.send(());
                    }
                    continue;
                }
            };
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
            match bus.call("luna://com.webos.media/load", &payload) {
                Ok(reply) => match reply.get("mediaId").and_then(Value::as_str) {
                    Some(id) => {
                        let id = id.to_string();
                        // Muted: a preview should never talk over the room.
                        let _ = bus.call("luna://com.webos.media/setVolume", &json!({ "mediaId": id, "volume": 0 }));
                        match bus.call("luna://com.webos.media/play", &json!({ "mediaId": id })) {
                            Ok(_) => log!("preview: playing"),
                            Err(error) => log!("preview play: {error}"),
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
    use std::path::{Path, PathBuf};

    pub struct Video;

    impl Video {
        pub fn start(
            _sdl_paths: &[PathBuf],
            _window: crate::sdl::Window,
            _rect: Rect,
            _app_id: &'static str,
            _data_dir: &Path,
            _bus: Option<Bus>,
        ) -> Self {
            Self
        }

        pub fn show(&self, stream: Stream) {
            // Never log the address: IPTV stream URLs carry the provider login.
            let _ = stream;
            log!("preview (desktop): would play a channel");
        }

        pub fn hide(&self) {}

        pub fn release(&self) {}
    }
}

pub use imp::Video;

/// The LS2 connection Vision shares as Home (nothing on the desktop).
#[cfg(all(target_os = "linux", target_arch = "arm"))]
pub type Bus = crate::luna::Remote;
#[cfg(not(all(target_os = "linux", target_arch = "arm")))]
pub type Bus = ();
