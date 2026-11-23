use std::sync::Arc;

use eframe::{egui, egui_glow, glow};
use glam::{Mat4, Vec3};
use glow::HasContext as _;

use super::model::{Model, EYE_RADIUS, IRIS_ANGLE, NECK_FADE, STRIDE};

const ATTRIBUTES: [&str; 5] = ["a_pos", "a_nrm", "a_col", "a_ao", "a_gloss"];
const ATTRIBUTE_SIZES: [i32; 5] = [3, 3, 3, 1, 1];
const MAX_TARGET: i32 = 4096;

const VERTEX: &str = r#"
in vec3 a_pos;
in vec3 a_nrm;
in vec3 a_col;
in float a_ao;
in float a_gloss;
uniform mat4 u_view_proj;
uniform mat4 u_model;
out vec3 v_wpos;
out vec3 v_nrm;
out vec3 v_col;
out float v_ao;
out float v_gloss;
out vec3 v_local;
void main() {
    vec4 w = u_model * vec4(a_pos, 1.0);
    v_wpos = w.xyz;
    v_nrm = mat3(u_model) * a_nrm;
    v_col = a_col;
    v_ao = a_ao;
    v_gloss = a_gloss;
    v_local = a_pos;
    gl_Position = u_view_proj * w;
}
"#;

const FRAGMENT: &str = r#"
in vec3 v_wpos;
in vec3 v_nrm;
in vec3 v_col;
in float v_ao;
in float v_gloss;
in vec3 v_local;
uniform vec3 u_camera;
uniform vec3 u_key;
uniform vec3 u_fill;
uniform vec3 u_rim;
uniform int u_mode;
uniform vec3 u_eye_center;
uniform float u_lid;
uniform vec3 u_iris;
uniform vec4 u_wire;
out vec4 o_color;

vec3 tonemap(vec3 c) {
    return clamp((c * (2.51 * c + 0.03)) / (c * (2.43 * c + 0.59) + 0.14), 0.0, 1.0);
}

void main() {
    if (u_mode == 2) {
        o_color = u_wire;
        return;
    }
    vec3 n = normalize(v_nrm);
    vec3 v = normalize(u_camera - v_wpos);
    vec3 albedo = v_col;
    float ao = v_ao;
    float gloss = v_gloss;
    float sss = 1.0;
    if (u_mode == 1) {
        vec3 d = normalize(v_local);
        float ang = acos(clamp(d.z, -1.0, 1.0));
        float t = ang / IRIS;
        float phi = atan(d.y, d.x + 1e-6);
        vec3 sclera = vec3(0.80, 0.78, 0.76) * (1.0 - 0.18 * smoothstep(0.9, 1.6, ang));
        float fib = 0.5 + 0.5 * sin(phi * 23.0 + sin(phi * 7.0) * 2.0);
        vec3 iris = mix(u_iris * 0.35, u_iris, smoothstep(0.25, 0.95, t) * 0.6 + fib * 0.25);
        iris *= 1.0 - 0.65 * smoothstep(0.8, 1.0, t);
        float iris_mask = 1.0 - smoothstep(0.97, 1.03, t);
        float pupil_mask = 1.0 - smoothstep(0.145, 0.175, ang);
        albedo = mix(mix(sclera, iris, iris_mask), vec3(0.01), pupil_mask);
        gloss = 1.0;
        sss = 0.0;
        float h = (v_wpos.y - u_eye_center.y) / EYE_R;
        float lu = sin(u_lid);
        ao = 1.0 - 0.55 * smoothstep(lu - 0.35, lu + 0.02, h);
    }
    float w = 0.35 * sss;
    float ndl = dot(n, u_key);
    float wrap = max((ndl + w) / (1.0 + w), 0.0);
    vec3 tint = mix(vec3(1.0, 0.62, 0.52), vec3(1.0), smoothstep(0.0, 0.45, max(ndl, 0.0)));
    vec3 key_color = vec3(1.0, 0.95, 0.88) * 2.2;
    vec3 diffuse = key_color * wrap * mix(vec3(1.0), tint, sss) * (0.4 + 0.6 * ao);
    vec3 fill = vec3(0.42, 0.52, 0.75) * 0.45 * max(dot(n, u_fill), 0.0) * ao;
    vec3 hemi = mix(vec3(0.10, 0.08, 0.09), vec3(0.24, 0.27, 0.34), n.y * 0.5 + 0.5) * ao;
    float ndv = max(dot(n, v), 0.0);
    float fres = pow(1.0 - ndv, 3.0);
    vec3 rim = vec3(0.55, 0.75, 1.0) * 1.4 * fres * max(dot(n, u_rim), 0.0) * ao;
    vec3 hv = normalize(u_key + v);
    float spec_pow = 16.0 + 240.0 * gloss * gloss;
    vec3 spec = pow(max(dot(n, hv), 0.0), spec_pow) * (0.04 + 0.9 * gloss * gloss) * key_color * ao;
    vec3 env = mix(vec3(0.10, 0.09, 0.10), vec3(0.35, 0.42, 0.55), n.y * 0.5 + 0.5)
        * (0.04 + 0.96 * pow(1.0 - ndv, 5.0)) * gloss * ao;
    vec3 c = (albedo * (diffuse + fill + hemi) + spec + rim + env) * 0.66;
    float fade = u_mode == 1 ? 1.0 : smoothstep(FADE_LOW, FADE_HIGH, v_wpos.y);
    o_color = vec4(pow(tonemap(c), vec3(1.0 / 2.2)) * fade, fade);
}
"#;

const COMPOSITE_VERTEX: &str = r#"
out vec2 v_uv;
void main() {
    vec2 p = vec2(float((gl_VertexID << 1) & 2), float(gl_VertexID & 2));
    v_uv = p;
    gl_Position = vec4(p * 2.0 - 1.0, 0.0, 1.0);
}
"#;

const COMPOSITE_FRAGMENT: &str = r#"
in vec2 v_uv;
uniform sampler2D u_tex;
out vec4 o_color;
void main() {
    o_color = texture(u_tex, v_uv);
}
"#;

#[derive(Clone, Copy)]
pub struct EyeDraw {
    pub model: Mat4,
    pub center: Vec3,
    pub lid_upper: f32,
}

#[derive(Clone, Copy)]
pub struct Frame {
    pub view_proj: Mat4,
    pub camera: Vec3,
    pub key: Vec3,
    pub fill: Vec3,
    pub rim: Vec3,
    pub iris: Vec3,
    pub eyes: [EyeDraw; 2],
    pub wireframe: bool,
}

#[derive(Default)]
pub struct Shared {
    model: Option<Arc<Model>>,
    pending: Option<Vec<f32>>,
    gpu: Option<Gpu>,
    error: Option<String>,
}

impl Shared {
    pub fn set_model(&mut self, model: Arc<Model>) {
        if self.model.as_ref().is_some_and(|m| Arc::ptr_eq(m, &model)) {
            return;
        }
        self.model = Some(model);
        self.gpu = None;
        self.pending = None;
    }

    pub fn submit(&mut self, vertices: Vec<f32>) {
        self.pending = Some(vertices);
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }

    pub fn paint(
        &mut self,
        info: &egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
        frame: &Frame,
    ) {
        if self.error.is_some() {
            return;
        }
        let Some(model) = self.model.clone() else {
            return;
        };
        if self.gpu.is_none() {
            match unsafe { Gpu::new(painter.gl().clone(), &model) } {
                Ok(gpu) => self.gpu = Some(gpu),
                Err(e) => {
                    log::warn!("3D preview disabled: {e}");
                    self.error = Some(e);
                    return;
                }
            }
        }
        let Some(gpu) = self.gpu.as_mut() else {
            return;
        };
        if let Some(vertices) = self.pending.take() {
            unsafe { gpu.upload(&vertices) };
        }
        if let Err(e) = unsafe { gpu.draw(info, painter, frame) } {
            log::warn!("3D preview disabled: {e}");
            self.error = Some(e);
            self.gpu = None;
        }
    }
}

struct Locations {
    view_proj: Option<glow::UniformLocation>,
    model: Option<glow::UniformLocation>,
    camera: Option<glow::UniformLocation>,
    key: Option<glow::UniformLocation>,
    fill: Option<glow::UniformLocation>,
    rim: Option<glow::UniformLocation>,
    mode: Option<glow::UniformLocation>,
    eye_center: Option<glow::UniformLocation>,
    lid: Option<glow::UniformLocation>,
    iris: Option<glow::UniformLocation>,
    wire: Option<glow::UniformLocation>,
    tex: Option<glow::UniformLocation>,
}

struct Target {
    width: i32,
    height: i32,
    msaa: Option<(glow::Framebuffer, glow::Renderbuffer, glow::Renderbuffer)>,
    fbo: glow::Framebuffer,
    tex: glow::Texture,
    depth: Option<glow::Renderbuffer>,
}

struct Gpu {
    gl: Arc<glow::Context>,
    program: glow::Program,
    composite: glow::Program,
    loc: Locations,
    arrays: [glow::VertexArray; 4],
    buffers: [glow::Buffer; 5],
    tri_count: i32,
    line_count: i32,
    eye_count: i32,
    vertex_bytes: usize,
    uploaded: bool,
    samples: i32,
    target: Option<Target>,
}

const SKIN_VAO: usize = 0;
const LINE_VAO: usize = 1;
const EYE_VAO: usize = 2;
const QUAD_VAO: usize = 3;
const SKIN_VBO: usize = 0;
const TRI_IBO: usize = 1;
const LINE_IBO: usize = 2;
const EYE_VBO: usize = 3;
const EYE_IBO: usize = 4;

fn bytes_f32(data: &[f32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data)) }
}

fn bytes_u32(data: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr().cast::<u8>(), std::mem::size_of_val(data)) }
}

fn shader_header(gl: &glow::Context) -> Result<&'static str, String> {
    let v = gl.version();
    if v.is_embedded {
        if v.major >= 3 {
            Ok("#version 300 es\nprecision highp float;\nprecision highp int;\n")
        } else {
            Err(format!(
                "OpenGL ES 3.0 is required (found {}.{})",
                v.major, v.minor
            ))
        }
    } else if v.major > 3 || (v.major == 3 && v.minor >= 3) {
        Ok("#version 330 core\n")
    } else if v.major == 3 {
        Ok(match v.minor {
            2 => "#version 150\n",
            1 => "#version 140\n",
            _ => "#version 130\n",
        })
    } else {
        Err(format!(
            "OpenGL 3.0 is required (found {}.{})",
            v.major, v.minor
        ))
    }
}

unsafe fn compile(
    gl: &glow::Context,
    header: &str,
    vertex: &str,
    fragment: &str,
    attributes: &[&str],
) -> Result<glow::Program, String> {
    unsafe {
        let program = gl.create_program()?;
        let mut shaders = Vec::new();
        let mut failure = None;
        for (kind, source) in [
            (glow::VERTEX_SHADER, vertex),
            (glow::FRAGMENT_SHADER, fragment),
        ] {
            let shader = match gl.create_shader(kind) {
                Ok(s) => s,
                Err(e) => {
                    failure = Some(e);
                    break;
                }
            };
            gl.shader_source(shader, &format!("{header}{source}"));
            gl.compile_shader(shader);
            if !gl.get_shader_compile_status(shader) {
                failure = Some(format!(
                    "shader compile failed: {}",
                    gl.get_shader_info_log(shader)
                ));
                gl.delete_shader(shader);
                break;
            }
            gl.attach_shader(program, shader);
            shaders.push(shader);
        }
        if failure.is_none() {
            for (i, name) in attributes.iter().enumerate() {
                gl.bind_attrib_location(program, i as u32, name);
            }
            if !gl.version().is_embedded {
                gl.bind_frag_data_location(program, 0, "o_color");
            }
            gl.link_program(program);
            if !gl.get_program_link_status(program) {
                failure = Some(format!(
                    "shader link failed: {}",
                    gl.get_program_info_log(program)
                ));
            }
        }
        for shader in shaders {
            gl.detach_shader(program, shader);
            gl.delete_shader(shader);
        }
        match failure {
            Some(e) => {
                gl.delete_program(program);
                Err(e)
            }
            None => Ok(program),
        }
    }
}

struct Created<'a> {
    gl: &'a glow::Context,
    programs: Vec<glow::Program>,
    arrays: Vec<glow::VertexArray>,
    buffers: Vec<glow::Buffer>,
}

impl Drop for Created<'_> {
    fn drop(&mut self) {
        unsafe {
            for p in self.programs.drain(..) {
                self.gl.delete_program(p);
            }
            for a in self.arrays.drain(..) {
                self.gl.delete_vertex_array(a);
            }
            for b in self.buffers.drain(..) {
                self.gl.delete_buffer(b);
            }
        }
    }
}

unsafe fn vertex_layout(gl: &glow::Context) {
    unsafe {
        let stride = (STRIDE * 4) as i32;
        let mut offset = 0;
        for (i, size) in ATTRIBUTE_SIZES.iter().enumerate() {
            gl.enable_vertex_attrib_array(i as u32);
            gl.vertex_attrib_pointer_f32(i as u32, *size, glow::FLOAT, false, stride, offset);
            offset += size * 4;
        }
    }
}

impl Gpu {
    unsafe fn new(gl: Arc<glow::Context>, model: &Model) -> Result<Gpu, String> {
        unsafe {
            let header = shader_header(&gl)?;
            let mut created = Created {
                gl: &gl,
                programs: Vec::new(),
                arrays: Vec::new(),
                buffers: Vec::new(),
            };
            let defines = format!(
                "#define IRIS {IRIS_ANGLE:.5}\n#define EYE_R {EYE_RADIUS:.5}\n#define FADE_LOW {:.5}\n#define FADE_HIGH {:.5}\n",
                NECK_FADE.0, NECK_FADE.1
            );
            let program = compile(
                &gl,
                header,
                &format!("{defines}{VERTEX}"),
                &format!("{defines}{FRAGMENT}"),
                &ATTRIBUTES,
            )?;
            created.programs.push(program);
            let composite = compile(&gl, header, COMPOSITE_VERTEX, COMPOSITE_FRAGMENT, &[])?;
            created.programs.push(composite);

            for _ in 0..4 {
                let a = gl.create_vertex_array()?;
                created.arrays.push(a);
            }
            for _ in 0..5 {
                let b = gl.create_buffer()?;
                created.buffers.push(b);
            }
            let arrays = [
                created.arrays[0],
                created.arrays[1],
                created.arrays[2],
                created.arrays[3],
            ];
            let buffers = [
                created.buffers[0],
                created.buffers[1],
                created.buffers[2],
                created.buffers[3],
                created.buffers[4],
            ];

            let vertex_bytes = model.vertex_count * STRIDE * 4;
            let vertex_bytes_i32 =
                i32::try_from(vertex_bytes).map_err(|_| "mesh too large".to_string())?;

            gl.bind_vertex_array(Some(arrays[SKIN_VAO]));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffers[SKIN_VBO]));
            gl.buffer_data_size(glow::ARRAY_BUFFER, vertex_bytes_i32, glow::DYNAMIC_DRAW);
            vertex_layout(&gl);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(buffers[TRI_IBO]));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_u32(&model.indices),
                glow::STATIC_DRAW,
            );

            gl.bind_vertex_array(Some(arrays[LINE_VAO]));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffers[SKIN_VBO]));
            vertex_layout(&gl);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(buffers[LINE_IBO]));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_u32(&model.lines),
                glow::STATIC_DRAW,
            );

            gl.bind_vertex_array(Some(arrays[EYE_VAO]));
            gl.bind_buffer(glow::ARRAY_BUFFER, Some(buffers[EYE_VBO]));
            gl.buffer_data_u8_slice(
                glow::ARRAY_BUFFER,
                bytes_f32(&model.eye_vertices),
                glow::STATIC_DRAW,
            );
            vertex_layout(&gl);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, Some(buffers[EYE_IBO]));
            gl.buffer_data_u8_slice(
                glow::ELEMENT_ARRAY_BUFFER,
                bytes_u32(&model.eye_indices),
                glow::STATIC_DRAW,
            );

            gl.bind_vertex_array(None);
            gl.bind_buffer(glow::ARRAY_BUFFER, None);
            gl.bind_buffer(glow::ELEMENT_ARRAY_BUFFER, None);

            let uniform = |name: &str| gl.get_uniform_location(program, name);
            let loc = Locations {
                view_proj: uniform("u_view_proj"),
                model: uniform("u_model"),
                camera: uniform("u_camera"),
                key: uniform("u_key"),
                fill: uniform("u_fill"),
                rim: uniform("u_rim"),
                mode: uniform("u_mode"),
                eye_center: uniform("u_eye_center"),
                lid: uniform("u_lid"),
                iris: uniform("u_iris"),
                wire: uniform("u_wire"),
                tex: gl.get_uniform_location(composite, "u_tex"),
            };
            let samples = gl.get_parameter_i32(glow::MAX_SAMPLES).clamp(0, 4);
            let count = |n: usize| i32::try_from(n).map_err(|_| "mesh too large".to_string());
            let gpu = Gpu {
                gl: gl.clone(),
                program,
                composite,
                loc,
                arrays,
                buffers,
                tri_count: count(model.indices.len())?,
                line_count: count(model.lines.len())?,
                eye_count: count(model.eye_indices.len())?,
                vertex_bytes,
                uploaded: false,
                samples,
                target: None,
            };
            created.programs.clear();
            created.arrays.clear();
            created.buffers.clear();
            Ok(gpu)
        }
    }

    unsafe fn upload(&mut self, vertices: &[f32]) {
        let bytes = bytes_f32(vertices);
        if bytes.len() != self.vertex_bytes {
            return;
        }
        unsafe {
            self.gl
                .bind_buffer(glow::ARRAY_BUFFER, Some(self.buffers[SKIN_VBO]));
            self.gl
                .buffer_sub_data_u8_slice(glow::ARRAY_BUFFER, 0, bytes);
            self.gl.bind_buffer(glow::ARRAY_BUFFER, None);
        }
        self.uploaded = true;
    }

    unsafe fn delete_target(&mut self) {
        let Some(t) = self.target.take() else {
            return;
        };
        let gl = &self.gl;
        unsafe {
            if let Some((fbo, color, depth)) = t.msaa {
                gl.delete_framebuffer(fbo);
                gl.delete_renderbuffer(color);
                gl.delete_renderbuffer(depth);
            }
            if let Some(depth) = t.depth {
                gl.delete_renderbuffer(depth);
            }
            gl.delete_framebuffer(t.fbo);
            gl.delete_texture(t.tex);
        }
    }

    unsafe fn create_target(&mut self, width: i32, height: i32) -> Result<(), String> {
        unsafe {
            self.delete_target();
            let gl = self.gl.clone();
            let tex = gl.create_texture()?;
            gl.bind_texture(glow::TEXTURE_2D, Some(tex));
            gl.tex_image_2d(
                glow::TEXTURE_2D,
                0,
                glow::RGBA8 as i32,
                width,
                height,
                0,
                glow::RGBA,
                glow::UNSIGNED_BYTE,
                glow::PixelUnpackData::Slice(None),
            );
            for (param, value) in [
                (glow::TEXTURE_MIN_FILTER, glow::NEAREST),
                (glow::TEXTURE_MAG_FILTER, glow::NEAREST),
                (glow::TEXTURE_WRAP_S, glow::CLAMP_TO_EDGE),
                (glow::TEXTURE_WRAP_T, glow::CLAMP_TO_EDGE),
            ] {
                gl.tex_parameter_i32(glow::TEXTURE_2D, param, value as i32);
            }
            gl.bind_texture(glow::TEXTURE_2D, None);
            let fbo = match gl.create_framebuffer() {
                Ok(f) => f,
                Err(e) => {
                    gl.delete_texture(tex);
                    return Err(e);
                }
            };
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_texture_2d(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::TEXTURE_2D,
                Some(tex),
                0,
            );
            self.target = Some(Target {
                width,
                height,
                msaa: None,
                fbo,
                tex,
                depth: None,
            });

            let mut msaa = None;
            if self.samples > 1 {
                if let Ok(m) = Self::multisample(&gl, self.samples, width, height) {
                    msaa = Some(m);
                } else {
                    self.samples = 0;
                }
            }
            if msaa.is_none() {
                let depth = gl.create_renderbuffer()?;
                gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
                gl.renderbuffer_storage(glow::RENDERBUFFER, glow::DEPTH_COMPONENT24, width, height);
                gl.bind_renderbuffer(glow::RENDERBUFFER, None);
                gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
                gl.framebuffer_renderbuffer(
                    glow::FRAMEBUFFER,
                    glow::DEPTH_ATTACHMENT,
                    glow::RENDERBUFFER,
                    Some(depth),
                );
                if let Some(t) = self.target.as_mut() {
                    t.depth = Some(depth);
                }
            }
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if let Some(t) = self.target.as_mut() {
                t.msaa = msaa;
            }
            if status != glow::FRAMEBUFFER_COMPLETE {
                return Err(format!("offscreen framebuffer incomplete (0x{status:x})"));
            }
            Ok(())
        }
    }

    unsafe fn multisample(
        gl: &glow::Context,
        samples: i32,
        width: i32,
        height: i32,
    ) -> Result<(glow::Framebuffer, glow::Renderbuffer, glow::Renderbuffer), String> {
        unsafe {
            let fbo = gl.create_framebuffer()?;
            let color = match gl.create_renderbuffer() {
                Ok(c) => c,
                Err(e) => {
                    gl.delete_framebuffer(fbo);
                    return Err(e);
                }
            };
            let depth = match gl.create_renderbuffer() {
                Ok(d) => d,
                Err(e) => {
                    gl.delete_framebuffer(fbo);
                    gl.delete_renderbuffer(color);
                    return Err(e);
                }
            };
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(color));
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                samples,
                glow::RGBA8,
                width,
                height,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, Some(depth));
            gl.renderbuffer_storage_multisample(
                glow::RENDERBUFFER,
                samples,
                glow::DEPTH_COMPONENT24,
                width,
                height,
            );
            gl.bind_renderbuffer(glow::RENDERBUFFER, None);
            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(fbo));
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::COLOR_ATTACHMENT0,
                glow::RENDERBUFFER,
                Some(color),
            );
            gl.framebuffer_renderbuffer(
                glow::FRAMEBUFFER,
                glow::DEPTH_ATTACHMENT,
                glow::RENDERBUFFER,
                Some(depth),
            );
            let status = gl.check_framebuffer_status(glow::FRAMEBUFFER);
            if status != glow::FRAMEBUFFER_COMPLETE {
                gl.delete_framebuffer(fbo);
                gl.delete_renderbuffer(color);
                gl.delete_renderbuffer(depth);
                return Err(format!("multisample framebuffer incomplete (0x{status:x})"));
            }
            Ok((fbo, color, depth))
        }
    }

    unsafe fn draw(
        &mut self,
        info: &egui::PaintCallbackInfo,
        painter: &egui_glow::Painter,
        frame: &Frame,
    ) -> Result<(), String> {
        if !self.uploaded {
            return Ok(());
        }
        let viewport = info.viewport_in_pixels();
        let width = viewport.width_px.clamp(1, MAX_TARGET);
        let height = viewport.height_px.clamp(1, MAX_TARGET);
        unsafe {
            let resize = self
                .target
                .as_ref()
                .is_none_or(|t| t.width != width || t.height != height);
            if resize {
                let result = self.create_target(width, height);
                self.gl
                    .bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
                result?;
            }
            let Some(target) = self.target.as_ref() else {
                return Ok(());
            };
            let gl = &self.gl;
            let render_fbo = target.msaa.map_or(target.fbo, |m| m.0);

            gl.bind_framebuffer(glow::FRAMEBUFFER, Some(render_fbo));
            gl.viewport(0, 0, width, height);
            gl.disable(glow::SCISSOR_TEST);
            gl.disable(glow::BLEND);
            gl.enable(glow::DEPTH_TEST);
            gl.depth_func(glow::LEQUAL);
            gl.depth_mask(true);
            gl.enable(glow::CULL_FACE);
            gl.cull_face(glow::BACK);
            gl.front_face(glow::CCW);
            gl.color_mask(true, true, true, true);
            gl.clear_color(0.0, 0.0, 0.0, 0.0);
            gl.clear_depth_f32(1.0);
            gl.clear(glow::COLOR_BUFFER_BIT | glow::DEPTH_BUFFER_BIT);

            gl.use_program(Some(self.program));
            let l = &self.loc;
            gl.uniform_matrix_4_f32_slice(
                l.view_proj.as_ref(),
                false,
                &frame.view_proj.to_cols_array(),
            );
            gl.uniform_matrix_4_f32_slice(l.model.as_ref(), false, &Mat4::IDENTITY.to_cols_array());
            for (loc, v) in [
                (&l.camera, frame.camera),
                (&l.key, frame.key),
                (&l.fill, frame.fill),
                (&l.rim, frame.rim),
                (&l.iris, frame.iris),
            ] {
                gl.uniform_3_f32(loc.as_ref(), v.x, v.y, v.z);
            }
            gl.uniform_1_i32(l.mode.as_ref(), 0);
            if frame.wireframe {
                gl.enable(glow::POLYGON_OFFSET_FILL);
                gl.polygon_offset(1.0, 1.0);
            }
            gl.bind_vertex_array(Some(self.arrays[SKIN_VAO]));
            gl.draw_elements(glow::TRIANGLES, self.tri_count, glow::UNSIGNED_INT, 0);

            gl.uniform_1_i32(l.mode.as_ref(), 1);
            gl.bind_vertex_array(Some(self.arrays[EYE_VAO]));
            for eye in &frame.eyes {
                gl.uniform_matrix_4_f32_slice(l.model.as_ref(), false, &eye.model.to_cols_array());
                gl.uniform_3_f32(
                    l.eye_center.as_ref(),
                    eye.center.x,
                    eye.center.y,
                    eye.center.z,
                );
                gl.uniform_1_f32(l.lid.as_ref(), eye.lid_upper);
                gl.draw_elements(glow::TRIANGLES, self.eye_count, glow::UNSIGNED_INT, 0);
            }
            gl.disable(glow::POLYGON_OFFSET_FILL);

            if frame.wireframe {
                gl.uniform_matrix_4_f32_slice(
                    l.model.as_ref(),
                    false,
                    &Mat4::IDENTITY.to_cols_array(),
                );
                gl.uniform_1_i32(l.mode.as_ref(), 2);
                let a = 0.22;
                gl.uniform_4_f32(l.wire.as_ref(), 0.40 * a, 0.80 * a, 0.88 * a, a);
                gl.enable(glow::BLEND);
                gl.blend_func(glow::ONE, glow::ONE_MINUS_SRC_ALPHA);
                gl.depth_mask(false);
                gl.bind_vertex_array(Some(self.arrays[LINE_VAO]));
                gl.draw_elements(glow::LINES, self.line_count, glow::UNSIGNED_INT, 0);
                gl.depth_mask(true);
                gl.disable(glow::BLEND);
            }

            if let Some((msaa, _, _)) = target.msaa {
                gl.bind_framebuffer(glow::READ_FRAMEBUFFER, Some(msaa));
                gl.bind_framebuffer(glow::DRAW_FRAMEBUFFER, Some(target.fbo));
                gl.blit_framebuffer(
                    0,
                    0,
                    width,
                    height,
                    0,
                    0,
                    width,
                    height,
                    glow::COLOR_BUFFER_BIT,
                    glow::NEAREST,
                );
            }

            gl.bind_framebuffer(glow::FRAMEBUFFER, painter.intermediate_fbo());
            gl.viewport(
                viewport.left_px,
                viewport.from_bottom_px,
                viewport.width_px,
                viewport.height_px,
            );
            let clip = info.clip_rect_in_pixels();
            gl.enable(glow::SCISSOR_TEST);
            gl.scissor(
                clip.left_px,
                clip.from_bottom_px,
                clip.width_px,
                clip.height_px,
            );
            gl.disable(glow::DEPTH_TEST);
            gl.disable(glow::CULL_FACE);
            gl.enable(glow::BLEND);
            gl.blend_equation(glow::FUNC_ADD);
            gl.blend_func_separate(
                glow::ONE,
                glow::ONE_MINUS_SRC_ALPHA,
                glow::ONE_MINUS_DST_ALPHA,
                glow::ONE,
            );
            gl.use_program(Some(self.composite));
            gl.active_texture(glow::TEXTURE0);
            gl.bind_texture(glow::TEXTURE_2D, Some(target.tex));
            gl.uniform_1_i32(self.loc.tex.as_ref(), 0);
            gl.bind_vertex_array(Some(self.arrays[QUAD_VAO]));
            gl.draw_arrays(glow::TRIANGLES, 0, 3);

            gl.bind_vertex_array(None);
            gl.bind_texture(glow::TEXTURE_2D, None);
            gl.use_program(None);
        }
        Ok(())
    }
}

impl Drop for Gpu {
    fn drop(&mut self) {
        unsafe {
            self.delete_target();
            let gl = &self.gl;
            gl.delete_program(self.program);
            gl.delete_program(self.composite);
            for a in self.arrays {
                gl.delete_vertex_array(a);
            }
            for b in self.buffers {
                gl.delete_buffer(b);
            }
        }
    }
}
