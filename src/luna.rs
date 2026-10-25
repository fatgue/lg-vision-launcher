//! A Luna (LS2) client over libluna-service2 and a private GLib context, loaded at runtime.
//! Used only from the platform worker thread.
use crate::dynlib::dynamic_api;
use serde_json::Value;
use std::ffi::{c_char, c_int, c_ulong, c_void, CStr, CString};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// LSError from luna-service2/lunaservice.h.
#[repr(C)]
pub struct LsError {
    code: c_int,
    message: *mut c_char,
    file: *const c_char,
    line: c_int,
    function: *const c_char,
    padding: *mut c_void,
    magic: c_ulong,
}

pub type Filter = extern "C" fn(*mut c_void, *mut c_void, *mut c_void) -> bool;

dynamic_api! {
    pub struct Ls2 {
        fn error_init = "LSErrorInit": fn(*mut LsError) -> bool;
        fn error_free = "LSErrorFree": fn(*mut LsError);
        fn register = "LSRegister": fn(*const c_char, *mut *mut c_void, *mut LsError) -> bool;
        fn attach = "LSGmainContextAttach": fn(*mut c_void, *mut c_void, *mut LsError) -> bool;
        fn call_one_reply = "LSCallOneReply": fn(*mut c_void, *const c_char, *const c_char, Filter, *mut c_void, *mut c_ulong, *mut LsError) -> bool;
        fn call = "LSCall": fn(*mut c_void, *const c_char, *const c_char, Filter, *mut c_void, *mut c_ulong, *mut LsError) -> bool;
        fn call_cancel = "LSCallCancel": fn(*mut c_void, c_ulong, *mut LsError) -> bool;
        fn message_payload = "LSMessageGetPayload": fn(*mut c_void) -> *const c_char;
    }
}

dynamic_api! {
    pub struct Glib {
        fn context_new = "g_main_context_new": fn() -> *mut c_void;
        fn context_iteration = "g_main_context_iteration": fn(*mut c_void, c_int) -> c_int;
        fn context_wakeup = "g_main_context_wakeup": fn(*mut c_void);
    }
}

pub struct Luna {
    ls: Ls2,
    glib: Glib,
    handle: *mut c_void,
    context: *mut c_void,
}

// Owned and driven by exactly one thread at a time.
unsafe impl Send for Luna {}

impl Ls2 {
    fn new_error(&self) -> LsError {
        let mut error = LsError {
            code: 0,
            message: std::ptr::null_mut(),
            file: std::ptr::null(),
            line: 0,
            function: std::ptr::null(),
            padding: std::ptr::null_mut(),
            magic: 0,
        };
        // SAFETY: initialises a caller-owned LSError.
        unsafe { (self.error_init)(&mut error) };
        error
    }

    fn describe(&self, mut error: LsError, stage: &str) -> String {
        let message = if error.message.is_null() {
            "unknown error".to_string()
        } else {
            // SAFETY: LS2 sets a NUL-terminated message on failure.
            unsafe { CStr::from_ptr(error.message) }.to_string_lossy().into_owned()
        };
        // SAFETY: frees what LS2 allocated in this error.
        unsafe { (self.error_free)(&mut error) };
        format!("{stage}: {message}")
    }
}

extern "C" fn on_reply(_handle: *mut c_void, message: *mut c_void, userdata: *mut c_void) -> bool {
    // SAFETY: userdata is the Reply slot owned by the waiting `call`; LS2 passes a live message.
    let reply = unsafe { &mut *(userdata as *mut Reply) };
    let payload = unsafe { (reply.payload_fn)(message) };
    reply.payload = Some(if payload.is_null() {
        String::new()
    } else {
        unsafe { CStr::from_ptr(payload) }.to_string_lossy().into_owned()
    });
    true
}

struct Reply {
    payload_fn: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    payload: Option<String>,
}

impl Luna {
    pub fn connect() -> Result<Self, String> {
        Self::connect_named(None)
    }

    /// `name`: a service name this executable's LS2 role allows (SAM identifies a registerApp
    /// caller by it); None registers anonymously.
    pub fn connect_named(name: Option<&str>) -> Result<Self, String> {
        let ls = Ls2::open(&[PathBuf::from("libluna-service2.so.3")])?;
        let glib = Glib::open(&[PathBuf::from("libglib-2.0.so.0")])?;
        // SAFETY: GLib returns a fresh context owned by this client.
        let context = unsafe { (glib.context_new)() };
        let mut handle = std::ptr::null_mut();
        let mut error = ls.new_error();
        // Anonymous registration: authenticated by our executable's LS2 role, and it does not
        // collide with SDL-webOS's own app-named registration.
        let name = name.map(|n| CString::new(n).unwrap());
        if !unsafe { (ls.register)(name.as_ref().map_or(std::ptr::null(), |n| n.as_ptr()), &mut handle, &mut error) } {
            return Err(ls.describe(error, "LSRegister"));
        }
        let mut error = ls.new_error();
        if !unsafe { (ls.attach)(handle, context, &mut error) } {
            return Err(ls.describe(error, "LSGmainContextAttach"));
        }
        Ok(Self { ls, glib, handle, context })
    }

    pub fn call(&mut self, uri: &str, payload: &Value) -> Result<Value, String> {
        let c_uri = CString::new(uri).map_err(|_| "bad uri")?;
        let c_payload = CString::new(payload.to_string()).map_err(|_| "bad payload")?;
        let mut reply = Box::new(Reply { payload_fn: self.ls.message_payload, payload: None });
        let mut token: c_ulong = 0;
        let mut error = self.ls.new_error();
        // SAFETY: strings and the boxed reply outlive the call; the context is ours.
        let ok = unsafe {
            (self.ls.call_one_reply)(
                self.handle,
                c_uri.as_ptr(),
                c_payload.as_ptr(),
                on_reply,
                (reply.as_mut() as *mut Reply).cast(),
                &mut token,
                &mut error,
            )
        };
        if !ok {
            return Err(self.ls.describe(error, uri));
        }
        let deadline = Instant::now() + Duration::from_millis(8000);
        while Instant::now() < deadline {
            // SAFETY: only this thread iterates this context.
            while unsafe { (self.glib.context_iteration)(self.context, 0) } != 0 {}
            if let Some(payload) = reply.payload.take() {
                return serde_json::from_str(&payload).map_err(|e| format!("{uri}: {e}"));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
        let mut error = self.ls.new_error();
        // SAFETY: cancels our own pending call; on refusal the reply box is leaked below so a
        // late callback never writes freed memory.
        if !unsafe { (self.ls.call_cancel)(self.handle, token, &mut error) } {
            std::mem::forget(reply);
        }
        Err(format!("{uri}: timed out"))
    }
}

type Sink = Box<dyn FnMut(Value) + Send>;

struct Subscription {
    payload_fn: unsafe extern "C" fn(*mut c_void) -> *const c_char,
    sink: Sink,
}

extern "C" fn on_message(_handle: *mut c_void, message: *mut c_void, userdata: *mut c_void) -> bool {
    // SAFETY: userdata is the leaked Subscription owned by the subscription thread.
    let subscription = unsafe { &mut *(userdata as *mut Subscription) };
    let payload = unsafe { (subscription.payload_fn)(message) };
    if !payload.is_null() {
        let text = unsafe { CStr::from_ptr(payload) }.to_string_lossy();
        if let Ok(value) = serde_json::from_str(&text) {
            (subscription.sink)(value);
        }
    }
    true
}

struct Request {
    uri: String,
    payload: Value,
    reply: std::sync::mpsc::Sender<Result<Value, String>>,
}

/// Calls made on the subscription's connection from other threads. As Home, LG's LS2 role
/// allows a single service name, and only that name may reach services such as the media
/// server, so everything that needs it shares the one connection.
#[derive(Clone)]
pub struct Remote {
    tx: std::sync::mpsc::Sender<Request>,
    context: usize,
    wakeup: unsafe extern "C" fn(*mut c_void),
}

impl Remote {
    pub fn call(&self, uri: &str, payload: &Value) -> Result<Value, String> {
        let (reply, answer) = std::sync::mpsc::channel();
        self.tx
            .send(Request { uri: uri.to_string(), payload: payload.clone(), reply })
            .map_err(|_| "luna connection closed".to_string())?;
        // SAFETY: waking a GLib context is thread-safe; the context lives as long as the process.
        unsafe { (self.wakeup)(self.context as *mut c_void) };
        answer.recv_timeout(Duration::from_secs(10)).map_err(|_| format!("{uri}: no answer"))?
    }
}

/// Holds a subscription on its own connection and thread for the life of the process
/// (SAM's registerApp: the lifecycle of Vision as Home).
pub fn subscribe(name: Option<&'static str>, uri: &str, payload: &Value, sink: Sink) -> Result<Remote, String> {
    let uri = CString::new(uri).map_err(|_| "bad uri")?;
    let payload = CString::new(payload.to_string()).map_err(|_| "bad payload")?;
    let (started, handshake) = std::sync::mpsc::sync_channel::<Result<Remote, String>>(1);
    let (tx, requests) = std::sync::mpsc::channel::<Request>();
    std::thread::Builder::new()
        .name("luna-subscription".into())
        .spawn(move || {
            let mut luna = match Luna::connect_named(name) {
                Ok(luna) => luna,
                Err(error) => return drop(started.send(Err(error))),
            };
            let userdata = Box::into_raw(Box::new(Subscription { payload_fn: luna.ls.message_payload, sink }));
            let mut token: c_ulong = 0;
            let mut error = luna.ls.new_error();
            // SAFETY: strings outlive the call; the leaked subscription lives as long as the process.
            let ok = unsafe { (luna.ls.call)(luna.handle, uri.as_ptr(), payload.as_ptr(), on_message, userdata.cast(), &mut token, &mut error) };
            if !ok {
                return drop(started.send(Err(luna.ls.describe(error, "subscription"))));
            }
            let _ = started.send(Ok(Remote { tx, context: luna.context as usize, wakeup: luna.glib.context_wakeup }));
            loop {
                // SAFETY: only this thread drives this connection's context.
                unsafe { (luna.glib.context_iteration)(luna.context, 1) };
                while let Ok(request) = requests.try_recv() {
                    let _ = request.reply.send(luna.call(&request.uri, &request.payload));
                }
            }
        })
        .map_err(|e| e.to_string())?;
    handshake.recv_timeout(Duration::from_secs(5)).map_err(|_| "subscription: no answer".to_string())?
}
