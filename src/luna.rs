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
        fn call_cancel = "LSCallCancel": fn(*mut c_void, c_ulong, *mut LsError) -> bool;
        fn message_payload = "LSMessageGetPayload": fn(*mut c_void) -> *const c_char;
    }
}

dynamic_api! {
    pub struct Glib {
        fn context_new = "g_main_context_new": fn() -> *mut c_void;
        fn context_iteration = "g_main_context_iteration": fn(*mut c_void, c_int) -> c_int;
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
        let ls = Ls2::open(&[PathBuf::from("libluna-service2.so.3")])?;
        let glib = Glib::open(&[PathBuf::from("libglib-2.0.so.0")])?;
        // SAFETY: GLib returns a fresh context owned by this client.
        let context = unsafe { (glib.context_new)() };
        let mut handle = std::ptr::null_mut();
        let mut error = ls.new_error();
        // Anonymous registration: authenticated by our executable's LS2 role, and it does not
        // collide with SDL-webOS's own app-named registration.
        if !unsafe { (ls.register)(std::ptr::null(), &mut handle, &mut error) } {
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
