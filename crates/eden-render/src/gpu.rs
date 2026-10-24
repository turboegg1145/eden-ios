//! OpenGL 3.3 / OpenGL ES 3.0 shader backend (`glow`).
//!
//! The backend deliberately uses plain GLSL 3.30 / GLSL ES 3.00. It does not
//! require Metal 2.4, `wgpu`, or any iOS 15+ API, so it runs on iOS 12–14.4
//! through `CAEAGLLayer`. The iOS shell also contains a native Metal 2.0 path
//! in `platforms/ios/EdenApp/Shaders/eden.metal`.

use bytemuck::{Pod, Zeroable};
use eden_paz::image::RgbaImage;
use glow::HasContext;

use crate::particles::Firefly;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 2],
    uv: [f32; 2],
}

const QUAD: [Vertex; 4] = [
    Vertex {
        pos: [0.0, 0.0],
        uv: [0.0, 0.0],
    },
    Vertex {
        pos: [1.0, 0.0],
        uv: [1.0, 0.0],
    },
    Vertex {
        pos: [0.0, 1.0],
        uv: [0.0, 1.0],
    },
    Vertex {
        pos: [1.0, 1.0],
        uv: [1.0, 1.0],
    },
];

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ParticleVertex {
    pos: [f32; 2],
    size: f32,
    phase: f32,
}

const VERTEX_SHADER_BODY: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in vec2 a_uv;
uniform vec4 u_rect;     // x, y, w, h in NDC
uniform vec4 u_uv_rect;  // u0, v0, du, dv
out vec2 v_uv;
void main() {
    vec2 p = u_rect.xy + a_pos * u_rect.zw;
    v_uv = u_uv_rect.xy + a_uv * u_uv_rect.zw;
    gl_Position = vec4(p, 0.0, 1.0);
}
"#;

const FRAG_NORMAL: &str = r#"
uniform sampler2D u_tex;
uniform float u_opacity;
uniform vec4 u_tint;
in vec2 v_uv;
out vec4 frag_color;
void main() {
    vec4 c = texture(u_tex, v_uv);
    frag_color = vec4(c.rgb * u_tint.rgb, c.a * u_opacity * u_tint.a);
}
"#;

const FRAG_MASK: &str = r#"
uniform sampler2D u_tex;
uniform sampler2D u_mask;
uniform float u_opacity;
in vec2 v_uv;
out vec4 frag_color;
float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }
void main() {
    vec4 c = texture(u_tex, v_uv);
    float m = luma(texture(u_mask, v_uv).rgb);
    frag_color = vec4(c.rgb, c.a * m * u_opacity);
}
"#;

const FRAG_TRANSITION: &str = r#"
uniform sampler2D u_from;
uniform sampler2D u_to;
uniform sampler2D u_rule;
uniform float u_progress;
uniform float u_softness;
in vec2 v_uv;
out vec4 frag_color;
float luma(vec3 c) { return dot(c, vec3(0.299, 0.587, 0.114)); }
float rule_alpha(float l, float p, float s) {
    return clamp((l - (1.0 - p)) * s + 0.5, 0.0, 1.0);
}
void main() {
    float l = luma(texture(u_rule, v_uv).rgb);
    float a = rule_alpha(l, u_progress, u_softness);
    frag_color = mix(texture(u_from, v_uv), texture(u_to, v_uv), a);
}
"#;

const FRAG_YUV: &str = r#"
uniform sampler2D u_y;
uniform sampler2D u_u;
uniform sampler2D u_v;
in vec2 v_uv;
out vec4 frag_color;
void main() {
    float y = texture(u_y, v_uv).r;
    float u = texture(u_u, v_uv).r - 0.5;
    float v = texture(u_v, v_uv).r - 0.5;
    vec3 rgb = vec3(
        y + 1.402 * v,
        y - 0.344136 * u - 0.714136 * v,
        y + 1.772 * u
    );
    frag_color = vec4(clamp(rgb, 0.0, 1.0), 1.0);
}
"#;

const PARTICLE_VERT: &str = r#"
layout(location = 0) in vec2 a_pos;
layout(location = 1) in float a_size;
layout(location = 2) in float a_phase;
uniform vec2 u_viewport;
out float v_phase;
void main() {
    vec2 ndc = vec2(a_pos.x / u_viewport.x * 2.0 - 1.0,
                    1.0 - a_pos.y / u_viewport.y * 2.0);
    gl_Position = vec4(ndc, 0.0, 1.0);
    gl_PointSize = a_size;
    v_phase = a_phase;
}
"#;

const PARTICLE_FRAG: &str = r#"
in float v_phase;
out vec4 frag_color;
void main() {
    vec2 d = gl_PointCoord - vec2(0.5);
    float r = length(d) * 2.0;
    if (r > 1.0) discard;
    float a = (1.0 - r) * (1.0 - r) * (0.55 + 0.45 * sin(v_phase * 1.7));
    frag_color = vec4(vec3(1.0, 0.91, 0.47) * a, a);
}
"#;

fn shader_header(gles: bool) -> &'static str {
    if gles {
        "#version 300 es\nprecision highp float;\n"
    } else {
        "#version 330 core\n"
    }
}

struct Program {
    program: glow::Program,
    u_rect: Option<glow::UniformLocation>,
    u_uv_rect: Option<glow::UniformLocation>,
    u_opacity: Option<glow::UniformLocation>,
    u_tint: Option<glow::UniformLocation>,
}

pub struct GlowRenderer {
    gl: glow::Context,
    gles: bool,
    normal: Program,
    masked: Program,
    transition: glow::Program,
    yuv: glow::Program,
    particle: glow::Program,
    vao: glow::VertexArray,
    /// Owns the static quad buffer; GL objects live until context teardown.
    #[allow(dead_code)]
    vbo: glow::Buffer,
    particle_vao: glow::VertexArray,
    particle_vbo: glow::Buffer,
}

impl GlowRenderer {
    pub fn new(gl: glow::Context, gles: bool) -> Result<Self, String> {
        unsafe {
            let header = shader_header(gles);
            let vs = format!("{header}{VERTEX_SHADER_BODY}");
            let normal_prog = compile_program(&gl, &vs, &format!("{header}{FRAG_NORMAL}"))?;
            let masked_prog = compile_program(&gl, &vs, &format!("{header}{FRAG_MASK}"))?;
            let transition_prog = compile_program(&gl, &vs, &format!("{header}{FRAG_TRANSITION}"))?;
            let yuv_prog = compile_program(&gl, &vs, &format!("{header}{FRAG_YUV}"))?;
            let particle_prog = compile_program(
                &gl,
                &format!("{header}{PARTICLE_VERT}"),
                &format!("{header}{PARTICLE_FRAG}"),
            )?;

            let vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(vao));
            let vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(vbo));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&QUAD),
                glow::STATIC_DRAW,
            );
            let stride = std::mem::size_of::<Vertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, stride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 2, glow::FLOAT, false, stride, 8);

            let particle_vao = gl.create_vertex_array()?;
            gl.bind_vertex_array(Some(particle_vao));
            let particle_vbo = gl.create_buffer()?;
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(particle_vbo));
            let pstride = std::mem::size_of::<ParticleVertex>() as i32;
            gl.enable_vertex_attrib_array(0);
            gl.vertex_attrib_pointer_f32(0, 2, glow::FLOAT, false, pstride, 0);
            gl.enable_vertex_attrib_array(1);
            gl.vertex_attrib_pointer_f32(1, 1, glow::FLOAT, false, pstride, 8);
            gl.enable_vertex_attrib_array(2);
            gl.vertex_attrib_pointer_f32(2, 1, glow::FLOAT, false, pstride, 12);

            gl.bind_vertex_array(None);

            // Set sampler units once.
            for p in [normal_prog, masked_prog, transition_prog, yuv_prog] {
                gl.use_program(Some(p));
                set_sampler(&gl, p, "u_tex", 0);
                set_sampler(&gl, p, "u_mask", 1);
                set_sampler(&gl, p, "u_from", 0);
                set_sampler(&gl, p, "u_to", 1);
                set_sampler(&gl, p, "u_rule", 2);
                set_sampler(&gl, p, "u_y", 0);
                set_sampler(&gl, p, "u_u", 1);
                set_sampler(&gl, p, "u_v", 2);
            }

            Ok(Self {
                gl,
                gles,
                normal: Program {
                    program: normal_prog,
                    u_rect: None,
                    u_uv_rect: None,
                    u_opacity: None,
                    u_tint: None,
                },
                masked: Program {
                    program: masked_prog,
                    u_rect: None,
                    u_uv_rect: None,
                    u_opacity: None,
                    u_tint: None,
                },
                transition: transition_prog,
                yuv: yuv_prog,
                particle: particle_prog,
                vao,
                vbo,
                particle_vao,
                particle_vbo,
            })
            .map(|mut r| {
                r.normal = Program::query(&r.gl, r.normal.program);
                r.masked = Program::query(&r.gl, r.masked.program);
                r
            })
        }
    }

    pub fn is_gles(&self) -> bool {
        self.gles
    }

    pub fn begin_frame(&self, width: u32, height: u32, clear: [f32; 4]) {
        unsafe {
            self.gl.viewport(0, 0, width as i32, height as i32);
            self.gl.clear_color(clear[0], clear[1], clear[2], clear[3]);
            self.gl.clear(glow::COLOR_BUFFER_BIT);
            self.gl.enable(glow::BLEND);
            self.gl
                .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
    }

    pub fn upload_texture(&self, img: &RgbaImage) -> Result<glow::Texture, String> {
        unsafe {
            let tex = self.gl.create_texture()?;
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
            self.gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                img.width as i32,
                img.height as i32,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(Some(&img.data)),
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MIN_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_MAG_FILTER,
                glow::LINEAR as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_S,
                glow::CLAMP_TO_EDGE as i32,
            );
            self.gl.tex_parameter_i32(
                glow::TEXTURE_2D,
                glow::TEXTURE_WRAP_T,
                glow::CLAMP_TO_EDGE as i32,
            );
            Ok(tex)
        }
    }

    pub fn upload_yuv420p(
        &self,
        y: &[u8],
        u: &[u8],
        v: &[u8],
        width: u32,
        height: u32,
    ) -> Result<[glow::Texture; 3], String> {
        let cw = width.div_ceil(2);
        let ch = height.div_ceil(2);
        unsafe {
            let make = |data: &[u8], w: u32, h: u32| -> Result<glow::Texture, String> {
                let tex = self.gl.create_texture()?;
                self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
                self.gl.pixel_store_i32(glow::UNPACK_ALIGNMENT, 1);
                self.gl.tex_image_2d(
                    glow::TEXTURE_2D,
                    0,
                    glow::R8 as i32,
                    w as i32,
                    h as i32,
                    0,
                    glow::RED,
                    glow::UNSIGNED_BYTE,
                    glow::PixelUnpackData::Slice(Some(data)),
                );
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MIN_FILTER,
                    glow::LINEAR as i32,
                );
                self.gl.tex_parameter_i32(
                    glow::TEXTURE_2D,
                    glow::TEXTURE_MAG_FILTER,
                    glow::LINEAR as i32,
                );
                Ok(tex)
            };
            Ok([make(y, width, height)?, make(u, cw, ch)?, make(v, cw, ch)?])
        }
    }

    pub fn draw_texture(
        &self,
        tex: glow::Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opacity: f32,
        viewport: (u32, u32),
    ) {
        let rect = ndc_rect(x, y, w, h, viewport.0, viewport.1);
        unsafe {
            self.gl.use_program(Some(self.normal.program));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            if let Some(l) = self.normal.u_rect {
                self.gl
                    .uniform_4_f32(Some(&l), rect[0], rect[1], rect[2], rect[3]);
            }
            if let Some(l) = self.normal.u_uv_rect {
                self.gl.uniform_4_f32(Some(&l), 0.0, 0.0, 1.0, 1.0);
            }
            if let Some(l) = self.normal.u_opacity {
                self.gl.uniform_1_f32(Some(&l), opacity);
            }
            if let Some(l) = self.normal.u_tint {
                self.gl.uniform_4_f32(Some(&l), 1.0, 1.0, 1.0, 1.0);
            }
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    pub fn draw_masked(
        &self,
        tex: glow::Texture,
        mask: glow::Texture,
        x: f32,
        y: f32,
        w: f32,
        h: f32,
        opacity: f32,
        viewport: (u32, u32),
    ) {
        let rect = ndc_rect(x, y, w, h, viewport.0, viewport.1);
        unsafe {
            self.gl.use_program(Some(self.masked.program));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            self.gl.active_texture(glow::TEXTURE1);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(mask));
            if let Some(l) = self.masked.u_rect {
                self.gl
                    .uniform_4_f32(Some(&l), rect[0], rect[1], rect[2], rect[3]);
            }
            if let Some(l) = self.masked.u_uv_rect {
                self.gl.uniform_4_f32(Some(&l), 0.0, 0.0, 1.0, 1.0);
            }
            if let Some(l) = self.masked.u_opacity {
                self.gl.uniform_1_f32(Some(&l), opacity);
            }
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    pub fn draw_transition(
        &self,
        from: glow::Texture,
        to: glow::Texture,
        rule: glow::Texture,
        progress: f32,
        viewport: (u32, u32),
    ) {
        unsafe {
            self.gl.use_program(Some(self.transition));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(from));
            self.gl.active_texture(glow::TEXTURE1);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(to));
            self.gl.active_texture(glow::TEXTURE2);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(rule));
            if let Some(l) = self.gl.get_uniform_location(self.transition, "u_rect") {
                self.gl.uniform_4_f32(Some(&l), -1.0, -1.0, 2.0, 2.0);
            }
            if let Some(l) = self.gl.get_uniform_location(self.transition, "u_uv_rect") {
                self.gl.uniform_4_f32(Some(&l), 0.0, 0.0, 1.0, 1.0);
            }
            if let Some(l) = self.gl.get_uniform_location(self.transition, "u_progress") {
                self.gl.uniform_1_f32(Some(&l), progress);
            }
            if let Some(l) = self.gl.get_uniform_location(self.transition, "u_softness") {
                self.gl.uniform_1_f32(Some(&l), 8.0);
            }
            let _ = viewport;
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    pub fn draw_yuv(
        &self,
        y: glow::Texture,
        u: glow::Texture,
        v: glow::Texture,
        x: f32,
        ypos: f32,
        w: f32,
        h: f32,
        viewport: (u32, u32),
    ) {
        let rect = ndc_rect(x, ypos, w, h, viewport.0, viewport.1);
        unsafe {
            self.gl.use_program(Some(self.yuv));
            self.gl.active_texture(glow::TEXTURE0);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(y));
            self.gl.active_texture(glow::TEXTURE1);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(u));
            self.gl.active_texture(glow::TEXTURE2);
            self.gl.bind_texture(glow::TEXTURE_2D, Some(v));
            if let Some(l) = self.gl.get_uniform_location(self.yuv, "u_rect") {
                self.gl
                    .uniform_4_f32(Some(&l), rect[0], rect[1], rect[2], rect[3]);
            }
            if let Some(l) = self.gl.get_uniform_location(self.yuv, "u_uv_rect") {
                self.gl.uniform_4_f32(Some(&l), 0.0, 0.0, 1.0, 1.0);
            }
            self.gl.bind_vertex_array(Some(self.vao));
            self.gl.draw_arrays(glow::TRIANGLE_STRIP, 0, 4);
        }
    }

    pub fn draw_particles(&self, particles: &[Firefly], viewport: (u32, u32)) {
        if particles.is_empty() {
            return;
        }
        let verts: Vec<ParticleVertex> = particles
            .iter()
            .map(|p| ParticleVertex {
                pos: [p.x, p.y],
                size: p.size,
                phase: p.phase,
            })
            .collect();
        unsafe {
            self.gl.use_program(Some(self.particle));
            if let Some(l) = self.gl.get_uniform_location(self.particle, "u_viewport") {
                self.gl
                    .uniform_2_f32(Some(&l), viewport.0 as f32, viewport.1 as f32);
            }
            self.gl.bind_vertex_array(Some(self.particle_vao));
            self.gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(self.particle_vbo));
            self.gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytemuck::cast_slice(&verts),
                glow::DYNAMIC_DRAW,
            );
            self.gl.enable(glow::BLEND);
            self.gl.blend_func(glow::SRC_ALPHA, glow::ONE);
            self.gl.draw_arrays(glow::POINTS, 0, verts.len() as i32);
            self.gl
                .blend_func(glow::SRC_ALPHA, glow::ONE_MINUS_SRC_ALPHA);
        }
    }

    pub fn delete_texture(&self, tex: glow::Texture) {
        unsafe {
            self.gl.delete_texture(tex);
        }
    }
}

impl Program {
    fn query(gl: &glow::Context, program: glow::Program) -> Self {
        unsafe {
            Self {
                program,
                u_rect: gl.get_uniform_location(program, "u_rect"),
                u_uv_rect: gl.get_uniform_location(program, "u_uv_rect"),
                u_opacity: gl.get_uniform_location(program, "u_opacity"),
                u_tint: gl.get_uniform_location(program, "u_tint"),
            }
        }
    }
}

unsafe fn compile_program(
    gl: &glow::Context,
    vs_src: &str,
    fs_src: &str,
) -> Result<glow::Program, String> {
    let vs = gl.create_shader(glow::VERTEX_SHADER)?;
    gl.shader_source(vs, vs_src);
    gl.compile_shader(vs);
    if !gl.get_shader_compile_status(vs) {
        let log = gl.get_shader_info_log(vs);
        gl.delete_shader(vs);
        return Err(format!("vertex shader: {log}"));
    }
    let fs = gl.create_shader(glow::FRAGMENT_SHADER)?;
    gl.shader_source(fs, fs_src);
    gl.compile_shader(fs);
    if !gl.get_shader_compile_status(fs) {
        let log = gl.get_shader_info_log(fs);
        gl.delete_shader(fs);
        gl.delete_shader(vs);
        return Err(format!("fragment shader: {log}"));
    }
    let program = gl.create_program()?;
    gl.attach_shader(program, vs);
    gl.attach_shader(program, fs);
    gl.link_program(program);
    if !gl.get_program_link_status(program) {
        let log = gl.get_program_info_log(program);
        gl.delete_program(program);
        gl.delete_shader(vs);
        gl.delete_shader(fs);
        return Err(format!("link: {log}"));
    }
    gl.detach_shader(program, vs);
    gl.detach_shader(program, fs);
    gl.delete_shader(vs);
    gl.delete_shader(fs);
    Ok(program)
}

unsafe fn set_sampler(gl: &glow::Context, program: glow::Program, name: &str, unit: i32) {
    if let Some(loc) = gl.get_uniform_location(program, name) {
        gl.uniform_1_i32(Some(&loc), unit);
    }
}

/// Map a top-left-origin screen rectangle to NDC `(x, y, w, h)`.
pub fn ndc_rect(x: f32, y: f32, w: f32, h: f32, vw: u32, vh: u32) -> [f32; 4] {
    let vw = vw.max(1) as f32;
    let vh = vh.max(1) as f32;
    [
        x / vw * 2.0 - 1.0,
        1.0 - y / vh * 2.0,
        w / vw * 2.0,
        -h / vh * 2.0,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ndc_maps_top_left_to_minus_one_one() {
        let r = ndc_rect(0.0, 0.0, 1024.0, 640.0, 1024, 640);
        assert_eq!(r, [-1.0, 1.0, 2.0, -2.0]);
        let r = ndc_rect(512.0, 320.0, 0.0, 0.0, 1024, 640);
        assert!((r[0] - 0.0).abs() < 1e-6);
        assert!((r[1] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn shader_sources_contain_required_features() {
        assert!(FRAG_TRANSITION.contains("rule_alpha"));
        assert!(FRAG_TRANSITION.contains("u_rule"));
        assert!(FRAG_YUV.contains("1.402"));
        assert!(PARTICLE_VERT.contains("gl_PointSize"));
        assert!(FRAG_MASK.contains("luma"));
        assert!(VERTEX_SHADER_BODY.contains("u_uv_rect"));
    }

    #[test]
    fn shader_headers_target_gles3_and_gl330() {
        assert!(shader_header(true).contains("#version 300 es"));
        assert!(shader_header(true).contains("precision highp float"));
        assert!(shader_header(false).contains("#version 330 core"));
    }
}
