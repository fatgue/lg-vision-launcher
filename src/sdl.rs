//! The slice of SDL2 Vision uses, loaded at runtime (SDL-webOS on the TV, SDL2.dll on desktop).
use crate::dynlib::dynamic_api;
use std::ffi::{c_char, c_int, c_void, CStr, CString};

pub type Window = *mut c_void;
pub type GlContext = *mut c_void;

pub const INIT_VIDEO: u32 = 0x0000_0020;
pub const INIT_EVENTS: u32 = 0x0000_4000;

pub const WINDOW_FULLSCREEN: u32 = 0x0000_0001;
pub const WINDOW_OPENGL: u32 = 0x0000_0002;
pub const WINDOW_SHOWN: u32 = 0x0000_0004;
pub const WINDOW_RESIZABLE: u32 = 0x0000_0020;
pub const WINDOW_ALLOW_HIGHDPI: u32 = 0x0000_2000;
pub const WINDOWPOS_CENTERED: c_int = 0x2FFF_0000;

// SDL_GLattr
pub const GL_RED_SIZE: c_int = 0;
pub const GL_GREEN_SIZE: c_int = 1;
pub const GL_BLUE_SIZE: c_int = 2;
pub const GL_ALPHA_SIZE: c_int = 3;
pub const GL_DOUBLEBUFFER: c_int = 5;
pub const GL_DEPTH_SIZE: c_int = 6;
pub const GL_STENCIL_SIZE: c_int = 7;
pub const GL_CONTEXT_MAJOR_VERSION: c_int = 17;
pub const GL_CONTEXT_MINOR_VERSION: c_int = 18;
pub const GL_CONTEXT_PROFILE_MASK: c_int = 21;
pub const GL_CONTEXT_PROFILE_COMPATIBILITY: c_int = 0x0002;
pub const GL_CONTEXT_PROFILE_ES: c_int = 0x0004;

// SDL_EventType
pub const QUIT: u32 = 0x100;
pub const APP_TERMINATING: u32 = 0x101;
pub const APP_DIDENTERBACKGROUND: u32 = 0x104;
pub const APP_DIDENTERFOREGROUND: u32 = 0x106;
pub const WINDOWEVENT: u32 = 0x200;
pub const KEYDOWN: u32 = 0x300;
pub const KEYUP: u32 = 0x301;
pub const USEREVENT: u32 = 0x8000;

// SDL_WindowEventID
pub const WINDOWEVENT_SHOWN: u8 = 1;
pub const WINDOWEVENT_EXPOSED: u8 = 3;
pub const WINDOWEVENT_SIZE_CHANGED: u8 = 6;
pub const WINDOWEVENT_RESTORED: u8 = 9;
pub const WINDOWEVENT_FOCUS_GAINED: u8 = 12;
pub const WINDOWEVENT_FOCUS_LOST: u8 = 13;

// SDL_Keycode
pub const KEY_RETURN: i32 = 13;
pub const KEY_ESCAPE: i32 = 27;
pub const KEY_BACKSPACE: i32 = 8;
pub const KEY_RIGHT: i32 = 0x4000_004F;
pub const KEY_LEFT: i32 = 0x4000_0050;
pub const KEY_DOWN: i32 = 0x4000_0051;
pub const KEY_UP: i32 = 0x4000_0052;
pub const KEY_KP_ENTER: i32 = 0x4000_0058;
pub const KEY_AC_BACK: i32 = 0x4000_010E;
/// SDL-webOS reports the remote's Back key outside SDL's standard scancode range.
pub const KEY_WEBOS_BACK: i32 = (1 << 30) | 482;

/// SDL_Event is a 56-byte union; Vision reads the few fields it needs by offset.
#[repr(C, align(8))]
pub struct Event {
    pub bytes: [u8; 56],
}

impl Event {
    pub fn zeroed() -> Self {
        Self { bytes: [0; 56] }
    }

    pub fn kind(&self) -> u32 {
        u32::from_ne_bytes(self.bytes[0..4].try_into().unwrap())
    }

    /// SDL_KeyboardEvent.keysym.sym
    pub fn key_sym(&self) -> i32 {
        i32::from_ne_bytes(self.bytes[20..24].try_into().unwrap())
    }

    /// SDL_KeyboardEvent.repeat
    pub fn key_repeat(&self) -> bool {
        self.bytes[13] != 0
    }

    /// SDL_WindowEvent.event
    pub fn window_event(&self) -> u8 {
        self.bytes[12]
    }
}

dynamic_api! {
    pub struct Sdl {
        fn init = "SDL_Init": fn(u32) -> c_int;
        fn quit = "SDL_Quit": fn();
        fn get_error = "SDL_GetError": fn() -> *const c_char;
        fn set_hint = "SDL_SetHint": fn(*const c_char, *const c_char) -> c_int;
        fn gl_set_attribute = "SDL_GL_SetAttribute": fn(c_int, c_int) -> c_int;
        fn create_window = "SDL_CreateWindow": fn(*const c_char, c_int, c_int, c_int, c_int, u32) -> Window;
        fn destroy_window = "SDL_DestroyWindow": fn(Window);
        fn gl_create_context = "SDL_GL_CreateContext": fn(Window) -> GlContext;
        fn gl_delete_context = "SDL_GL_DeleteContext": fn(GlContext);
        fn gl_set_swap_interval = "SDL_GL_SetSwapInterval": fn(c_int) -> c_int;
        fn gl_get_proc_address = "SDL_GL_GetProcAddress": fn(*const c_char) -> *mut c_void;
        fn gl_swap_window = "SDL_GL_SwapWindow": fn(Window);
        fn gl_get_drawable_size = "SDL_GL_GetDrawableSize": fn(Window, *mut c_int, *mut c_int);
        fn poll_event = "SDL_PollEvent": fn(*mut Event) -> c_int;
        fn wait_event_timeout = "SDL_WaitEventTimeout": fn(*mut Event, c_int) -> c_int;
        fn push_event = "SDL_PushEvent": fn(*mut Event) -> c_int;
        fn show_cursor = "SDL_ShowCursor": fn(c_int) -> c_int;
    }
}

impl Sdl {
    pub fn error(&self) -> String {
        // SAFETY: SDL returns a valid, NUL-terminated thread-local string.
        unsafe { CStr::from_ptr((self.get_error)()) }
            .to_string_lossy()
            .into_owned()
    }

    pub fn hint(&self, name: &str, value: &str) {
        let (name, value) = (CString::new(name).unwrap(), CString::new(value).unwrap());
        // SAFETY: both strings are valid for the call; SDL copies them.
        unsafe { (self.set_hint)(name.as_ptr(), value.as_ptr()) };
    }

    pub fn waker(&self) -> Waker {
        Waker(self.push_event)
    }
}

/// Wakes the UI thread's SDL_WaitEventTimeout from a worker (SDL_PushEvent is thread-safe).
#[derive(Clone, Copy)]
pub struct Waker(unsafe extern "C" fn(*mut Event) -> c_int);

impl Waker {
    pub fn wake(&self) {
        let mut event = Event::zeroed();
        event.bytes[0..4].copy_from_slice(&USEREVENT.to_ne_bytes());
        // SAFETY: a zeroed user event is valid; SDL copies it into its queue.
        unsafe { (self.0)(&mut event) };
    }
}
