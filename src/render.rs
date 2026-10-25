//! One shader, one streamed vertex buffer. Every shape (rounded rects, soft shadows, glows,
//! radial light, strokes) is an analytic SDF evaluated per pixel, so nothing is pre-rendered
//! and a typical frame is a handful of draw calls. Textures only break a batch when a glyph
//! or icon needs a different atlas than the one bound.
use crate::color::Rgba;
use crate::gl::{self, Gl};
use std::ffi::{c_char, CString};

pub const LOGICAL_W: f32 = 1920.0;
pub const LOGICAL_H: f32 = 1080.0;

#[derive(Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { x, y, w, h }
    }

    pub fn inflate(self, by: f32) -> Self {
        Self::new(self.x - by, self.y - by, self.w + 2.0 * by, self.h + 2.0 * by)
    }

    pub fn offset(self, dx: f32, dy: f32) -> Self {
        Self::new(self.x + dx, self.y + dy, self.w, self.h)
    }

    /// Scales around a pivot given as a fraction of the rect (0.5, 1.0 = centre bottom).
    pub fn scale_about(self, s: f32, px: f32, py: f32) -> Self {
        let ox = self.x + self.w * px;
        let oy = self.y + self.h * py;
        Self::new(ox + (self.x - ox) * s, oy + (self.y - oy) * s, self.w * s, self.h * s)
    }

    pub fn visible(&self) -> bool {
        self.x < LOGICAL_W && self.y < LOGICAL_H && self.x + self.w > 0.0 && self.y + self.h > 0.0
    }
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
    local: [f32; 2],
    params: [f32; 4],
    color: [u8; 4],
    kind: f32,
}

const KIND_FILL: f32 = 0.0;
const KIND_RADIAL: f32 = 1.0;
const KIND_GLYPH: f32 = 2.0;
const KIND_IMAGE: f32 = 3.0;
const KIND_STROKE: f32 = 4.0;

const MAX_QUADS: usize = 4096;

const VERTEX_SHADER: &str = r#"
attribute vec2 a_pos;
attribute vec2 a_uv;
attribute vec2 a_local;
attribute vec4 a_params;
attribute vec4 a_color;
attribute float a_kind;
uniform vec2 u_scale;
varying vec2 v_uv;
varying vec2 v_local;
varying vec4 v_params;
varying vec4 v_color;
varying float v_kind;
void main() {
    v_uv = a_uv;
    v_local = a_local;
    v_params = a_params;
    v_color = a_color;
    v_kind = a_kind;
    gl_Position = vec4(a_pos * u_scale + vec2(-1.0, 1.0), 0.0, 1.0);
}
"#;

const FRAGMENT_SHADER: &str = r#"
#ifdef GL_ES
#ifdef GL_FRAGMENT_PRECISION_HIGH
precision highp float;
#else
precision mediump float;
#endif
#endif
uniform sampler2D u_tex;
varying vec2 v_uv;
varying vec2 v_local;
varying vec4 v_params;
varying vec4 v_color;
varying float v_kind;

float rrect(vec2 p, vec2 b, float r) {
    vec2 q = abs(p) - b + vec2(r);
    return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

// Interleaved gradient noise: breaks 8-bit banding in large OLED gradients.
float dither() {
    return fract(52.9829189 * fract(dot(gl_FragCoord.xy, vec2(0.06711056, 0.00583715)))) - 0.5;
}

void main() {
    float a;
    if (v_kind < 0.5) {
        float d = rrect(v_local, v_params.xy, v_params.z);
        a = clamp(0.5 - d / v_params.w, 0.0, 1.0);
        if (v_params.w > 1.5) {
            a = a * a * (3.0 - 2.0 * a);
        }
        a += dither() / 255.0;
    } else if (v_kind < 1.5) {
        float t = clamp(1.0 - length(v_local / v_params.xy), 0.0, 1.0);
        a = t * t * (3.0 - 2.0 * t) + dither() / 255.0;
    } else if (v_kind < 2.5) {
        a = texture2D(u_tex, v_uv).a;
    } else if (v_kind < 3.5) {
        vec4 c = texture2D(u_tex, v_uv);
        float mask = clamp(0.5 - rrect(v_local, v_params.xy, v_params.z), 0.0, 1.0);
        gl_FragColor = vec4(c.rgb * v_color.rgb, c.a * v_color.a * mask);
        return;
    } else {
        float d = abs(rrect(v_local, v_params.xy, v_params.z)) - v_params.w * 0.5;
        a = clamp(0.5 - d, 0.0, 1.0);
    }
    gl_FragColor = vec4(v_color.rgb, v_color.a * a);
}
"#;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Texture {
    pub id: u32,
    pub w: i32,
    pub h: i32,
}

#[derive(Default, Clone, Copy, Debug)]
pub struct FrameStats {
    pub draw_calls: u32,
    pub quads: u32,
}

pub struct Renderer {
    pub gl: Gl,
    program: u32,
    u_scale: i32,
    vbo: u32,
    vertices: Vec<Vertex>,
    bound: u32,
    /// Rotation applied to the next quads' positions: centre x/y, cos, sin.
    rotation: Option<[f32; 4]>,
    pub stats: FrameStats,
}

impl Renderer {
    pub fn new(gl: Gl, es: bool) -> Result<Self, String> {
        let header = if es {
            "#version 100\n"
        } else {
            "#version 120\n#define highp\n#define mediump\n#define lowp\n"
        };
        let program = link_program(
            &gl,
            &format!("{header}{VERTEX_SHADER}"),
            &format!("{header}{FRAGMENT_SHADER}"),
        )?;
        let uniform = |name: &str| {
            let name = CString::new(name).unwrap();
            // SAFETY: valid program and NUL-terminated name.
            unsafe { (gl.get_uniform_location)(program, name.as_ptr()) }
        };
        let u_scale = uniform("u_scale");
        let u_tex = uniform("u_tex");

        let mut buffers = [0u32; 2];
        // Static index buffer for MAX_QUADS quads: (0 1 2)(2 1 3) per quad.
        let indices: Vec<u16> = (0..MAX_QUADS as u16)
            .flat_map(|q| {
                let b = q * 4;
                [b, b + 1, b + 2, b + 2, b + 1, b + 3]
            })
            .collect();
        // SAFETY: plain GL setup on the current context.
        unsafe {
            (gl.gen_buffers)(2, buffers.as_mut_ptr());
            (gl.bind_buffer)(gl::ELEMENT_ARRAY_BUFFER, buffers[1]);
            (gl.buffer_data)(
                gl::ELEMENT_ARRAY_BUFFER,
                (indices.len() * 2) as isize,
                indices.as_ptr().cast(),
                gl::STATIC_DRAW,
            );
            (gl.bind_buffer)(gl::ARRAY_BUFFER, buffers[0]);
            (gl.use_program)(program);
            (gl.uniform_1i)(u_tex, 0);
            (gl.active_texture)(gl::TEXTURE0);

            let stride = std::mem::size_of::<Vertex>() as i32;
            let attribute = |index: u32, size: i32, kind: gl::Enum, normalized: bool, offset: usize| {
                (gl.enable_vertex_attrib_array)(index);
                (gl.vertex_attrib_pointer)(index, size, kind, normalized as u8, stride, offset as *const _);
            };
            attribute(0, 2, gl::FLOAT, false, 0);
            attribute(1, 2, gl::FLOAT, false, 8);
            attribute(2, 2, gl::FLOAT, false, 16);
            attribute(3, 4, gl::FLOAT, false, 24);
            attribute(4, 4, gl::UNSIGNED_BYTE, true, 40);
            attribute(5, 1, gl::FLOAT, false, 44);

            (gl.disable)(gl::DEPTH_TEST);
            (gl.disable)(gl::CULL_FACE);
            (gl.disable)(gl::DITHER);
            (gl.enable)(gl::BLEND);
            // Keep destination alpha at 1: the webOS compositor blends our surface by it.
            (gl.blend_func_separate)(gl::SRC_ALPHA, gl::ONE_MINUS_SRC_ALPHA, gl::ONE, gl::ONE_MINUS_SRC_ALPHA);
            (gl.pixel_store_i)(gl::UNPACK_ALIGNMENT, 1);
        }
        Ok(Self {
            gl,
            program,
            u_scale,
            vbo: buffers[0],
            vertices: Vec::with_capacity(MAX_QUADS * 4),
            bound: 0,
            rotation: None,
            stats: FrameStats::default(),
        })
    }

    pub fn create_texture(&self, w: i32, h: i32, format: gl::Enum) -> Texture {
        let mut id = 0;
        // SAFETY: allocates an empty texture of the given size on the current context.
        unsafe {
            (self.gl.gen_textures)(1, &mut id);
            (self.gl.bind_texture)(gl::TEXTURE_2D, id);
            (self.gl.tex_parameter_i)(gl::TEXTURE_2D, gl::TEXTURE_MIN_FILTER, gl::LINEAR);
            (self.gl.tex_parameter_i)(gl::TEXTURE_2D, gl::TEXTURE_MAG_FILTER, gl::LINEAR);
            (self.gl.tex_parameter_i)(gl::TEXTURE_2D, gl::TEXTURE_WRAP_S, gl::CLAMP_TO_EDGE);
            (self.gl.tex_parameter_i)(gl::TEXTURE_2D, gl::TEXTURE_WRAP_T, gl::CLAMP_TO_EDGE);
            (self.gl.tex_image_2d)(
                gl::TEXTURE_2D, 0, format as i32, w, h, 0, format, gl::UNSIGNED_BYTE, std::ptr::null(),
            );
            (self.gl.bind_texture)(gl::TEXTURE_2D, self.bound);
        }
        Texture { id, w, h }
    }

    /// Uploads a sub-rectangle. Safe mid-batch: queued quads read other regions.
    pub fn upload(&self, texture: Texture, x: i32, y: i32, w: i32, h: i32, format: gl::Enum, pixels: &[u8]) {
        // SAFETY: `pixels` holds w*h texels of `format` with 1-byte row alignment.
        unsafe {
            (self.gl.bind_texture)(gl::TEXTURE_2D, texture.id);
            (self.gl.tex_sub_image_2d)(gl::TEXTURE_2D, 0, x, y, w, h, format, gl::UNSIGNED_BYTE, pixels.as_ptr().cast());
            (self.gl.bind_texture)(gl::TEXTURE_2D, self.bound);
        }
    }

    pub fn begin(&mut self, drawable: (i32, i32), clear: Rgba) {
        self.stats = FrameStats::default();
        // SAFETY: per-frame state on the current context.
        unsafe {
            (self.gl.viewport)(0, 0, drawable.0, drawable.1);
            (self.gl.clear_color)(clear.r, clear.g, clear.b, 1.0);
            (self.gl.clear)(gl::COLOR_BUFFER_BIT);
            (self.gl.use_program)(self.program);
            (self.gl.uniform_2f)(self.u_scale, 2.0 / LOGICAL_W, -2.0 / LOGICAL_H);
        }
    }

    pub fn end(&mut self) {
        self.flush();
    }

    fn flush(&mut self) {
        if self.vertices.is_empty() {
            return;
        }
        let quads = self.vertices.len() / 4;
        // SAFETY: the bound VBO receives exactly the queued vertices; indices cover them.
        unsafe {
            (self.gl.bind_buffer)(gl::ARRAY_BUFFER, self.vbo);
            (self.gl.buffer_data)(
                gl::ARRAY_BUFFER,
                (self.vertices.len() * std::mem::size_of::<Vertex>()) as isize,
                self.vertices.as_ptr().cast(),
                gl::STREAM_DRAW,
            );
            (self.gl.draw_elements)(gl::TRIANGLES, (quads * 6) as i32, gl::UNSIGNED_SHORT, std::ptr::null());
        }
        self.stats.draw_calls += 1;
        self.stats.quads += quads as u32;
        self.vertices.clear();
    }

    fn bind(&mut self, texture: u32) {
        if texture != self.bound {
            self.flush();
            // SAFETY: binding an existing texture.
            unsafe { (self.gl.bind_texture)(gl::TEXTURE_2D, texture) };
            self.bound = texture;
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn quad(&mut self, r: Rect, uv: [f32; 4], center: (f32, f32), params: [f32; 4], colors: [Rgba; 2], kind: f32) {
        if self.vertices.len() + 4 > MAX_QUADS * 4 {
            self.flush();
        }
        let (top, bottom) = (colors[0].to_u8(), colors[1].to_u8());
        let corners = [
            (r.x, r.y, uv[0], uv[1], top),
            (r.x + r.w, r.y, uv[2], uv[1], top),
            (r.x, r.y + r.h, uv[0], uv[3], bottom),
            (r.x + r.w, r.y + r.h, uv[2], uv[3], bottom),
        ];
        for (x, y, u, v, color) in corners {
            // Only the position turns: `local` keeps the shape's own frame, so the SDF edge and
            // its anti-aliasing stay exact.
            let pos = match self.rotation {
                Some([cx, cy, cos, sin]) => {
                    let (dx, dy) = (x - cx, y - cy);
                    [cx + dx * cos - dy * sin, cy + dx * sin + dy * cos]
                }
                None => [x, y],
            };
            self.vertices.push(Vertex {
                pos,
                uv: [u, v],
                local: [x - center.0, y - center.1],
                params,
                color,
                kind,
            });
        }
    }

    /// Turns everything drawn next by `angle` radians around (cx, cy); None stops.
    pub fn set_rotation(&mut self, around: Option<(f32, f32, f32)>) {
        self.rotation = around.map(|(cx, cy, angle)| [cx, cy, angle.cos(), angle.sin()]);
    }

    /// Anti-aliased filled rounded rectangle.
    pub fn rect(&mut self, r: Rect, radius: f32, color: Rgba) {
        if color.a <= 0.0 || !r.visible() {
            return;
        }
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let params = [r.w * 0.5, r.h * 0.5, radius.min(r.w * 0.5).min(r.h * 0.5), 1.0];
        self.quad(r.inflate(1.0), [0.0; 4], c, params, [color; 2], KIND_FILL);
    }

    /// Box-shadow style soft rectangle; `blur` is the falloff width in px.
    pub fn shadow(&mut self, r: Rect, radius: f32, blur: f32, color: Rgba) {
        if color.a <= 0.0 {
            return;
        }
        let blur = blur.max(2.0);
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let params = [r.w * 0.5, r.h * 0.5, radius.min(r.w * 0.5).min(r.h * 0.5), blur];
        self.quad(r.inflate(blur * 0.5 + 1.0), [0.0; 4], c, params, [color; 2], KIND_FILL);
    }

    pub fn stroke(&mut self, r: Rect, radius: f32, width: f32, color: Rgba) {
        if color.a <= 0.0 {
            return;
        }
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let params = [r.w * 0.5, r.h * 0.5, radius, width];
        self.quad(r.inflate(width + 1.0), [0.0; 4], c, params, [color; 2], KIND_STROKE);
    }

    /// Elliptical radial light, full `color` in the centre fading to nothing at the rim.
    pub fn radial(&mut self, cx: f32, cy: f32, rx: f32, ry: f32, color: Rgba) {
        if color.a <= 0.0 {
            return;
        }
        let r = Rect::new(cx - rx, cy - ry, rx * 2.0, ry * 2.0);
        self.quad(r, [0.0; 4], (cx, cy), [rx, ry, 0.0, 0.0], [color; 2], KIND_RADIAL);
    }

    pub fn vgradient(&mut self, r: Rect, top: Rgba, bottom: Rgba) {
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        // Huge half-size: the SDF never clips, the quad edges do.
        let params = [r.w, r.h, 0.0, 1.0];
        self.quad(r, [0.0; 4], c, params, [top, bottom], KIND_FILL);
    }

    /// Writes fully transparent pixels: the hardware video plane below shows through.
    pub fn punch(&mut self, r: Rect) {
        self.flush();
        // SAFETY: state toggles on the current context.
        unsafe { (self.gl.disable)(gl::BLEND) };
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        self.quad(r, [0.0; 4], c, [r.w * 0.5, r.h * 0.5, 0.0, 1.0], [Rgba { r: 0.0, g: 0.0, b: 0.0, a: 0.0 }; 2], KIND_FILL);
        self.flush();
        unsafe { (self.gl.enable)(gl::BLEND) };
    }

    pub fn glyph(&mut self, texture: Texture, r: Rect, uv: [f32; 4], color: Rgba) {
        self.bind(texture.id);
        self.quad(r, uv, (0.0, 0.0), [0.0; 4], [color; 2], KIND_GLYPH);
    }

    /// Texture region tinted by `color`, clipped to a rounded rect of `radius`.
    pub fn image(&mut self, texture: Texture, r: Rect, uv: [f32; 4], radius: f32, color: Rgba) {
        if color.a <= 0.0 || !r.visible() {
            return;
        }
        self.bind(texture.id);
        let c = (r.x + r.w * 0.5, r.y + r.h * 0.5);
        let params = [r.w * 0.5, r.h * 0.5, radius.min(r.w * 0.5).min(r.h * 0.5), 1.0];
        self.quad(r, uv, c, params, [color; 2], KIND_IMAGE);
    }
}

fn compile(gl: &Gl, kind: gl::Enum, source: &str) -> Result<u32, String> {
    let source = CString::new(source).unwrap();
    // SAFETY: standard shader compilation with a valid source pointer.
    unsafe {
        let shader = (gl.create_shader)(kind);
        let pointer: *const c_char = source.as_ptr();
        (gl.shader_source)(shader, 1, &pointer, std::ptr::null());
        (gl.compile_shader)(shader);
        let mut ok = 0;
        (gl.get_shader_iv)(shader, gl::COMPILE_STATUS, &mut ok);
        if ok == 0 {
            let mut len = 0;
            (gl.get_shader_iv)(shader, gl::INFO_LOG_LENGTH, &mut len);
            let mut log = vec![0u8; len.max(1) as usize];
            (gl.get_shader_info_log)(shader, len, std::ptr::null_mut(), log.as_mut_ptr().cast());
            return Err(format!("shader: {}", String::from_utf8_lossy(&log)));
        }
        Ok(shader)
    }
}

fn link_program(gl: &Gl, vertex: &str, fragment: &str) -> Result<u32, String> {
    let vs = compile(gl, gl::VERTEX_SHADER, vertex)?;
    let fs = compile(gl, gl::FRAGMENT_SHADER, fragment)?;
    // SAFETY: standard program linking.
    unsafe {
        let program = (gl.create_program)();
        (gl.attach_shader)(program, vs);
        (gl.attach_shader)(program, fs);
        for (index, name) in ["a_pos", "a_uv", "a_local", "a_params", "a_color", "a_kind"].iter().enumerate() {
            let name = CString::new(*name).unwrap();
            (gl.bind_attrib_location)(program, index as u32, name.as_ptr());
        }
        (gl.link_program)(program);
        (gl.delete_shader)(vs);
        (gl.delete_shader)(fs);
        let mut ok = 0;
        (gl.get_program_iv)(program, gl::LINK_STATUS, &mut ok);
        if ok == 0 {
            let mut len = 0;
            (gl.get_program_iv)(program, gl::INFO_LOG_LENGTH, &mut len);
            let mut log = vec![0u8; len.max(1) as usize];
            (gl.get_program_info_log)(program, len, std::ptr::null_mut(), log.as_mut_ptr().cast());
            return Err(format!("program: {}", String::from_utf8_lossy(&log)));
        }
        Ok(program)
    }
}
