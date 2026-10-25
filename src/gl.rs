//! The OpenGL ES 2.0 subset Vision draws with (also valid on desktop GL 2.1 for previews).
#![allow(dead_code)]
use crate::dynlib::gl_api;
use std::ffi::{c_char, c_void};

pub type Enum = u32;
pub type Uint = u32;

pub const COLOR_BUFFER_BIT: u32 = 0x4000;
pub const BLEND: Enum = 0x0BE2;
pub const DEPTH_TEST: Enum = 0x0B71;
pub const CULL_FACE: Enum = 0x0B44;
pub const DITHER: Enum = 0x0BD0;
pub const SCISSOR_TEST: Enum = 0x0C11;
pub const ONE: Enum = 1;
pub const SRC_ALPHA: Enum = 0x0302;
pub const ONE_MINUS_SRC_ALPHA: Enum = 0x0303;
pub const TRIANGLES: Enum = 0x0004;
pub const UNSIGNED_BYTE: Enum = 0x1401;
pub const UNSIGNED_SHORT: Enum = 0x1403;
pub const FLOAT: Enum = 0x1406;
pub const ARRAY_BUFFER: Enum = 0x8892;
pub const ELEMENT_ARRAY_BUFFER: Enum = 0x8893;
pub const STREAM_DRAW: Enum = 0x88E0;
pub const STATIC_DRAW: Enum = 0x88E4;
pub const FRAGMENT_SHADER: Enum = 0x8B30;
pub const VERTEX_SHADER: Enum = 0x8B31;
pub const COMPILE_STATUS: Enum = 0x8B81;
pub const LINK_STATUS: Enum = 0x8B82;
pub const INFO_LOG_LENGTH: Enum = 0x8B84;
pub const TEXTURE_2D: Enum = 0x0DE1;
pub const TEXTURE0: Enum = 0x84C0;
pub const TEXTURE_MAG_FILTER: Enum = 0x2800;
pub const TEXTURE_MIN_FILTER: Enum = 0x2801;
pub const TEXTURE_WRAP_S: Enum = 0x2802;
pub const TEXTURE_WRAP_T: Enum = 0x2803;
pub const LINEAR: i32 = 0x2601;
pub const CLAMP_TO_EDGE: i32 = 0x812F;
pub const ALPHA: Enum = 0x1906;
pub const RGBA: Enum = 0x1908;
pub const UNPACK_ALIGNMENT: Enum = 0x0CF5;
pub const VENDOR: Enum = 0x1F00;
pub const RENDERER: Enum = 0x1F01;
pub const VERSION: Enum = 0x1F02;

gl_api! {
    pub struct Gl {
        fn viewport = "glViewport": fn(i32, i32, i32, i32);
        fn clear_color = "glClearColor": fn(f32, f32, f32, f32);
        fn clear = "glClear": fn(u32);
        fn enable = "glEnable": fn(Enum);
        fn disable = "glDisable": fn(Enum);
        fn blend_func_separate = "glBlendFuncSeparate": fn(Enum, Enum, Enum, Enum);
        fn get_string = "glGetString": fn(Enum) -> *const u8;
        fn get_error = "glGetError": fn() -> Enum;
        fn pixel_store_i = "glPixelStorei": fn(Enum, i32);
        fn create_shader = "glCreateShader": fn(Enum) -> Uint;
        fn shader_source = "glShaderSource": fn(Uint, i32, *const *const c_char, *const i32);
        fn compile_shader = "glCompileShader": fn(Uint);
        fn get_shader_iv = "glGetShaderiv": fn(Uint, Enum, *mut i32);
        fn get_shader_info_log = "glGetShaderInfoLog": fn(Uint, i32, *mut i32, *mut c_char);
        fn delete_shader = "glDeleteShader": fn(Uint);
        fn create_program = "glCreateProgram": fn() -> Uint;
        fn attach_shader = "glAttachShader": fn(Uint, Uint);
        fn bind_attrib_location = "glBindAttribLocation": fn(Uint, Uint, *const c_char);
        fn link_program = "glLinkProgram": fn(Uint);
        fn get_program_iv = "glGetProgramiv": fn(Uint, Enum, *mut i32);
        fn get_program_info_log = "glGetProgramInfoLog": fn(Uint, i32, *mut i32, *mut c_char);
        fn use_program = "glUseProgram": fn(Uint);
        fn get_uniform_location = "glGetUniformLocation": fn(Uint, *const c_char) -> i32;
        fn uniform_1i = "glUniform1i": fn(i32, i32);
        fn uniform_2f = "glUniform2f": fn(i32, f32, f32);
        fn gen_buffers = "glGenBuffers": fn(i32, *mut Uint);
        fn bind_buffer = "glBindBuffer": fn(Enum, Uint);
        fn buffer_data = "glBufferData": fn(Enum, isize, *const c_void, Enum);
        fn enable_vertex_attrib_array = "glEnableVertexAttribArray": fn(Uint);
        fn vertex_attrib_pointer = "glVertexAttribPointer": fn(Uint, i32, Enum, u8, i32, *const c_void);
        fn draw_elements = "glDrawElements": fn(Enum, i32, Enum, *const c_void);
        fn gen_textures = "glGenTextures": fn(i32, *mut Uint);
        fn bind_texture = "glBindTexture": fn(Enum, Uint);
        fn active_texture = "glActiveTexture": fn(Enum);
        fn tex_parameter_i = "glTexParameteri": fn(Enum, Enum, i32);
        fn tex_image_2d = "glTexImage2D": fn(Enum, i32, i32, i32, i32, i32, Enum, Enum, *const c_void);
        fn tex_sub_image_2d = "glTexSubImage2D": fn(Enum, i32, i32, i32, i32, i32, Enum, Enum, *const c_void);
        fn read_pixels = "glReadPixels": fn(i32, i32, i32, i32, Enum, Enum, *mut c_void);
    }
}

impl Gl {
    pub fn string(&self, name: Enum) -> String {
        // SAFETY: glGetString returns a static NUL-terminated string or null.
        let pointer = unsafe { (self.get_string)(name) };
        if pointer.is_null() {
            return String::new();
        }
        unsafe { std::ffi::CStr::from_ptr(pointer.cast()) }
            .to_string_lossy()
            .into_owned()
    }
}
