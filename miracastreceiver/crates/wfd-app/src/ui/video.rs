//! Draws decoded I420 frames. The fast path uploads the three planes as
//! single-channel GL textures and converts YUV->RGB in a fragment shader
//! (1.5 bytes/pixel upload, no CPU conversion). If the GL context can't run
//! the shader (very old GL), frames are converted on the CPU and shown as a
//! normal egui texture instead.

use std::sync::{Arc, Mutex};

use egui_glow::glow::{self, HasContext};
use wfd_media::{ColorMatrix, DecodedFrame};

pub struct GlVideo {
    program: glow::Program,
    vao: glow::VertexArray,
    textures: [glow::Texture; 3],
    sizes: [(i32, i32); 3],
    u_bt709: Option<glow::UniformLocation>,
    u_full: Option<glow::UniformLocation>,
    pending: Option<Arc<DecodedFrame>>,
    current: Option<(bool, bool)>,
}

const VERTEX: &str = r#"
const vec2 VERTS[4] = vec2[4](vec2(-1.0, -1.0), vec2(1.0, -1.0), vec2(-1.0, 1.0), vec2(1.0, 1.0));
out vec2 v_uv;
void main() {
    vec2 p = VERTS[gl_VertexID];
    v_uv = vec2((p.x + 1.0) * 0.5, 1.0 - (p.y + 1.0) * 0.5);
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

const FRAGMENT: &str = r#"
precision mediump float;
uniform sampler2D u_y;
uniform sampler2D u_u;
uniform sampler2D u_v;
uniform int u_bt709;
uniform int u_full;
in vec2 v_uv;
out vec4 out_color;
void main() {
    float y = texture(u_y, v_uv).r;
    float u = texture(u_u, v_uv).r - 0.5;
    float v = texture(u_v, v_uv).r - 0.5;
    if (u_full == 0) {
        y = (y - 16.0 / 255.0) * (255.0 / 219.0);
        u = u * (255.0 / 224.0);
        v = v * (255.0 / 224.0);
    }
    vec3 rgb = (u_bt709 == 1)
        ? vec3(y + 1.5748 * v, y - 0.1873 * u - 0.4681 * v, y + 1.8556 * u)
        : vec3(y + 1.402 * v, y - 0.3441 * u - 0.7141 * v, y + 1.772 * u);
    out_color = vec4(clamp(rgb, 0.0, 1.0), 1.0);
}
"#;

impl GlVideo {
    pub fn new(gl: &glow::Context) -> Result<Self, String> {
        let version = egui_glow::ShaderVersion::get(gl);
        if !version.is_new_shader_interface() {
            return Err(format!("GL shader version {version:?} too old for the YUV shader"));
        }
        let decl = version.version_declaration();
        unsafe {
            let program = gl.create_program()?;
            let mut shaders = Vec::new();
            for (kind, src) in [(glow::VERTEX_SHADER, VERTEX), (glow::FRAGMENT_SHADER, FRAGMENT)] {
                let sh = gl.create_shader(kind)?;
                gl.shader_source(sh, &format!("{decl}\n{src}"));
                gl.compile_shader(sh);
                if !gl.get_shader_compile_status(sh) {
                    return Err(format!("YUV shader: {}", gl.get_shader_info_log(sh)));
                }
                gl.attach_shader(program, sh);
                shaders.push(sh);
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                return Err(format!("YUV program: {}", gl.get_program_info_log(program)));
            }
            for sh in shaders {
                gl.detach_shader(program, sh);
                gl.delete_shader(sh);
            }
            let vao = gl.create_vertex_array()?;
            let mut textures = Vec::new();
            for _ in 0..3 {
                let t = gl.create_texture()?;
                gl.bind_texture(glow::TEXTURE_2D, Some(t));
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MIN_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_MAG_FILTER, glow::LINEAR as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE as i32);
                gl.tex_parameter_i32(glow::TEXTURE_2D, glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE as i32);
                textures.push(t);
            }
            gl.use_program(Some(program));
            for (i, name) in ["u_y", "u_u", "u_v"].iter().enumerate() {
                let loc = gl.get_uniform_location(program, name);
                gl.uniform_1_i32(loc.as_ref(), i as i32);
            }
            let u_bt709 = gl.get_uniform_location(program, "u_bt709");
            let u_full = gl.get_uniform_location(program, "u_full");
            gl.use_program(None);
            Ok(Self {
                program,
                vao,
                textures: [textures[0], textures[1], textures[2]],
                sizes: [(0, 0); 3],
                u_bt709,
                u_full,
                pending: None,
                current: None,
            })
        }
    }

    pub fn set_frame(&mut self, f: Arc<DecodedFrame>) {
        self.pending = Some(f);
    }

    pub fn has_frame(&self) -> bool {
        self.current.is_some() || self.pending.is_some()
    }

    fn upload(&mut self, gl: &glow::Context, f: &DecodedFrame) {
        let sizes = &mut self.sizes;
        let textures = self.textures;
        f.with_planes(|planes| unsafe {
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            for (i, p) in planes.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + i as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(textures[i]));
                gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, p.stride as i32);
                let (w, h) = (p.width as i32, p.height as i32);
                let needed = p.stride * (p.height as usize - 1) + p.width as usize;
                if p.data.len() < needed {
                    continue;
                }
                let data = glow::PixelUnpackData::Slice(Some(p.data));
                if sizes[i] != (w, h) {
                    gl.tex_image_2d(glow::TEXTURE_2D, 0, glow::R8 as i32, w, h, 0, glow::RED, glow::UNSIGNED_BYTE, data);
                    sizes[i] = (w, h);
                } else {
                    gl.tex_sub_image_2d(glow::TEXTURE_2D, 0, 0, 0, w, h, glow::RED, glow::UNSIGNED_BYTE, data);
                }
            }
            gl.pixel_store_i32(glow::UNPACK_ROW_LENGTH, 0);
            gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 4);
            gl.active_texture(glow::TEXTURE0);
        });
        self.current = Some((f.matrix == ColorMatrix::Bt709, f.full_range));
    }

    /// Called from egui's paint callback with the viewport set to our rect.
    pub fn paint(&mut self, gl: &glow::Context) {
        if let Some(f) = self.pending.take() {
            self.upload(gl, &f);
        }
        let Some((bt709, full)) = self.current else { return };
        unsafe {
            gl.use_program(Some(self.program));
            gl.uniform_1_i32(self.u_bt709.as_ref(), bt709 as i32);
            gl.uniform_1_i32(self.u_full.as_ref(), full as i32);
            for (i, t) in self.textures.iter().enumerate() {
                gl.active_texture(glow::TEXTURE0 + i as u32);
                gl.bind_texture(glow::TEXTURE_2D, Some(*t));
            }
            gl.disable(glow::BLEND);
            gl.bind_vertex_array(Some(self.vao));
            gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
            gl.bind_vertex_array(None);
            gl.active_texture(glow::TEXTURE0);
            gl.enable(glow::BLEND);
        }
    }

    pub fn destroy(&self, gl: &glow::Context) {
        unsafe {
            gl.delete_program(self.program);
            gl.delete_vertex_array(self.vao);
            for t in self.textures {
                gl.delete_texture(t);
            }
        }
    }
}

/// The renderer the UI holds: GPU shader when possible, CPU fallback otherwise.
pub enum VideoRenderer {
    Gl(Arc<Mutex<GlVideo>>),
    Cpu { texture: Option<egui::TextureHandle> },
}

impl VideoRenderer {
    pub fn new(gl: Option<&Arc<glow::Context>>) -> (Self, Option<String>) {
        if std::env::var_os("MIRACAST_CPU_RENDER").is_some() {
            return (Self::Cpu { texture: None }, Some("MIRACAST_CPU_RENDER is set".into()));
        }
        match gl.map(|g| GlVideo::new(g)) {
            Some(Ok(v)) => (Self::Gl(Arc::new(Mutex::new(v))), None),
            Some(Err(e)) => (Self::Cpu { texture: None }, Some(e)),
            None => (Self::Cpu { texture: None }, Some("no OpenGL context".into())),
        }
    }

    pub fn is_gpu(&self) -> bool {
        matches!(self, Self::Gl(_))
    }

    pub fn set_frame(&mut self, ctx: &egui::Context, f: Arc<DecodedFrame>) {
        match self {
            Self::Gl(v) => v.lock().unwrap().set_frame(f),
            Self::Cpu { texture } => {
                if let Some(rgba) = f.to_rgba() {
                    let img = egui::ColorImage::from_rgba_unmultiplied([f.width as usize, f.height as usize], &rgba);
                    match texture {
                        Some(t) => t.set(img, egui::TextureOptions::LINEAR),
                        None => *texture = Some(ctx.load_texture("video", img, egui::TextureOptions::LINEAR)),
                    }
                }
            }
        }
    }

    pub fn clear(&mut self) {
        match self {
            Self::Gl(v) => {
                let mut v = v.lock().unwrap();
                v.pending = None;
                v.current = None;
            }
            Self::Cpu { texture } => *texture = None,
        }
    }

    pub fn has_frame(&self) -> bool {
        match self {
            Self::Gl(v) => v.lock().unwrap().has_frame(),
            Self::Cpu { texture } => texture.is_some(),
        }
    }

    pub fn paint(&self, painter: &egui::Painter, rect: egui::Rect) {
        match self {
            Self::Gl(v) => {
                let v = v.clone();
                painter.add(egui::PaintCallback {
                    rect,
                    callback: Arc::new(egui_glow::CallbackFn::new(move |_info, p| {
                        v.lock().unwrap().paint(p.gl());
                    })),
                });
            }
            Self::Cpu { texture: Some(t) } => {
                painter.image(t.id(), rect, egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)), egui::Color32::WHITE);
            }
            Self::Cpu { texture: None } => {}
        }
    }

    pub fn destroy(&self, gl: Option<&glow::Context>) {
        if let (Self::Gl(v), Some(gl)) = (self, gl) {
            v.lock().unwrap().destroy(gl);
        }
    }
}
