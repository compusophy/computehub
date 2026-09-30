//! The WebGL2 side of the gfx contract: one program, one VAO, one instance
//! buffer, one instanced draw per frame.

use gfx::{DrawList, INSTANCE_BYTES, Rgba};
use wasm_bindgen::JsCast;
use web_sys::WebGl2RenderingContext as Gl;
use web_sys::{
    HtmlCanvasElement, WebGlBuffer, WebGlContextAttributes, WebGlPowerPreference, WebGlProgram,
    WebGlShader, WebGlUniformLocation, WebGlVertexArrayObject,
};

/// The instanced attributes as (location, GL type, normalized, byte offset);
/// each has 4 components, divisor 1 and stride [`INSTANCE_BYTES`], matching
/// the `layout(location = N)` declarations in [`gfx::VERTEX_SHADER`].
pub(crate) const ATTRIBS: [(u32, u32, bool, i32); 3] = [
    (0, Gl::FLOAT, false, 0),
    (1, Gl::FLOAT, false, 16),
    (2, Gl::UNSIGNED_BYTE, true, 32),
];

/// The smallest GL instance buffer, in bytes (a power of two; 455 instances).
pub(crate) const MIN_CAPACITY: usize = 16 * 1024;

/// Draws [`gfx::DrawList`]s into the `<canvas>` with WebGL2.
///
/// The platform owns it and lends it to [`crate::App::frame`]; the canvas
/// size and device pixel ratio it draws at are the ones the last
/// [`crate::Event::Resize`] reported.
pub struct Renderer {
    gl: Gl,
    canvas: HtmlCanvasElement,
    program: WebGlProgram,
    vao: WebGlVertexArrayObject,
    buffer: WebGlBuffer,
    u_viewport: Option<WebGlUniformLocation>,
    u_dpr: Option<WebGlUniformLocation>,
    bytes: Vec<u8>,      // the reused encode buffer
    capacity: usize,     // the GL buffer's size in bytes
    backing: (u32, u32), // the size last written to the canvas
    css: (f32, f32),
    dpr: f32,
}

impl Renderer {
    /// Creates the WebGL2 context on `canvas` and the GPU objects; on failure
    /// the error names the step, with the info log for shader errors.
    pub(crate) fn new(canvas: &HtmlCanvasElement) -> Result<Renderer, String> {
        let attrs = WebGlContextAttributes::new();
        attrs.set_alpha(false);
        attrs.set_antialias(false);
        attrs.set_depth(false);
        attrs.set_stencil(false);
        attrs.set_premultiplied_alpha(true);
        attrs.set_preserve_drawing_buffer(false);
        attrs.set_power_preference(WebGlPowerPreference::LowPower);
        let gl: Gl = canvas
            .get_context_with_context_options("webgl2", &attrs)
            .map_err(|e| format!("getContext(\"webgl2\") threw: {e:?}"))?
            .ok_or("WebGL2 is not available")?
            .dyn_into()
            .map_err(|_| "getContext(\"webgl2\") returned a non-WebGL2 context")?;

        let program = link(&gl)?;
        let u_viewport = gl.get_uniform_location(&program, "u_viewport");
        let u_dpr = gl.get_uniform_location(&program, "u_dpr");

        let vao = gl.create_vertex_array().ok_or("createVertexArray failed")?;
        let buffer = gl.create_buffer().ok_or("createBuffer failed")?;
        gl.bind_vertex_array(Some(&vao));
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&buffer));
        for (loc, ty, normalized, offset) in ATTRIBS {
            gl.enable_vertex_attrib_array(loc);
            let stride = INSTANCE_BYTES as i32;
            gl.vertex_attrib_pointer_with_i32(loc, 4, ty, normalized, stride, offset);
            gl.vertex_attrib_divisor(loc, 1);
        }
        gl.bind_vertex_array(None);

        Ok(Renderer {
            gl,
            canvas: canvas.clone(),
            program,
            vao,
            buffer,
            u_viewport,
            u_dpr,
            bytes: Vec::new(),
            capacity: 0,
            backing: (0, 0),
            css: (0.0, 0.0),
            dpr: 1.0,
        })
    }

    /// The canvas size in CSS pixels, as last measured.
    pub fn css_size(&self) -> (f32, f32) {
        self.css
    }

    /// The device pixel ratio, as last measured.
    pub fn dpr(&self) -> f32 {
        self.dpr
    }

    pub(crate) fn set_size(&mut self, w: f32, h: f32, dpr: f32) {
        self.css = (w, h);
        self.dpr = dpr;
    }

    /// Draws one frame: resizes the canvas backing store to
    /// `round(css * dpr)` if that changed, clears to `clear` (opaque), then
    /// draws every instance of `list` in one instanced call. An empty list
    /// only clears.
    pub fn draw(&mut self, list: &DrawList, clear: Rgba) {
        let gl = &self.gl;
        let size = backing_size(self.css, self.dpr);
        if size != self.backing {
            self.canvas.set_width(size.0);
            self.canvas.set_height(size.1);
            self.backing = size;
        }
        gl.viewport(0, 0, gl.drawing_buffer_width(), gl.drawing_buffer_height());
        let [r, g, b] = clear_rgb(clear);
        gl.clear_color(r, g, b, 1.0);
        gl.clear(Gl::COLOR_BUFFER_BIT);
        if list.is_empty() {
            return;
        }

        list.encode_into(&mut self.bytes);
        gl.bind_buffer(Gl::ARRAY_BUFFER, Some(&self.buffer));
        let len = self.bytes.len();
        if len > self.capacity {
            // Grow with headroom: upload the bytes zero-padded to the new
            // capacity, so later frames fit and take bufferSubData.
            self.capacity = grow_capacity(len);
            self.bytes.resize(self.capacity, 0);
            gl.buffer_data_with_u8_array(Gl::ARRAY_BUFFER, &self.bytes, Gl::DYNAMIC_DRAW);
            self.bytes.truncate(len);
        } else {
            gl.buffer_sub_data_with_i32_and_u8_array(Gl::ARRAY_BUFFER, 0, &self.bytes);
        }

        gl.enable(Gl::BLEND);
        gl.blend_func(Gl::ONE, Gl::ONE_MINUS_SRC_ALPHA);
        gl.use_program(Some(&self.program));
        gl.uniform2f(self.u_viewport.as_ref(), self.css.0, self.css.1);
        gl.uniform1f(self.u_dpr.as_ref(), self.dpr);
        gl.bind_vertex_array(Some(&self.vao));
        let n = i32::try_from(list.len()).unwrap_or(i32::MAX);
        gl.draw_arrays_instanced(Gl::TRIANGLE_STRIP, 0, 4, n);
        gl.bind_vertex_array(None);
    }
}

/// Compiles both gfx shaders and links them. Compile status is read only when
/// the link fails, so a good program costs no extra round trips.
fn link(gl: &Gl) -> Result<WebGlProgram, String> {
    let vs = shader(gl, Gl::VERTEX_SHADER, gfx::VERTEX_SHADER)?;
    let fs = shader(gl, Gl::FRAGMENT_SHADER, gfx::FRAGMENT_SHADER)?;
    let program = gl.create_program().ok_or("createProgram failed")?;
    gl.attach_shader(&program, &vs);
    gl.attach_shader(&program, &fs);
    gl.link_program(&program);
    let linked = gl
        .get_program_parameter(&program, Gl::LINK_STATUS)
        .is_truthy();
    let result = if linked {
        Ok(program)
    } else {
        let compiled = |s: &WebGlShader| gl.get_shader_parameter(s, Gl::COMPILE_STATUS).is_truthy();
        let log = |s: &WebGlShader| gl.get_shader_info_log(s).unwrap_or_default();
        Err(if !compiled(&vs) {
            format!("vertex shader failed to compile: {}", log(&vs))
        } else if !compiled(&fs) {
            format!("fragment shader failed to compile: {}", log(&fs))
        } else {
            let log = gl.get_program_info_log(&program).unwrap_or_default();
            format!("shader program failed to link: {log}")
        })
    };
    gl.delete_shader(Some(&vs));
    gl.delete_shader(Some(&fs));
    result
}

fn shader(gl: &Gl, ty: u32, src: &str) -> Result<WebGlShader, String> {
    let s = gl.create_shader(ty).ok_or("createShader failed")?;
    gl.shader_source(&s, src);
    gl.compile_shader(&s);
    Ok(s)
}

/// The canvas backing-store size for a CSS size and pixel ratio:
/// `round(css * dpr)` per axis, at least 1.
pub(crate) fn backing_size(css: (f32, f32), dpr: f32) -> (u32, u32) {
    let px = |v: f32| {
        let r = (v * dpr).round();
        if r >= 1.0 { r as u32 } else { 1 }
    };
    (px(css.0), px(css.1))
}

/// A color's sRGB channels as GL floats in `[0, 1]`; alpha is dropped.
pub(crate) fn clear_rgb(c: Rgba) -> [f32; 3] {
    [c.0, c.1, c.2].map(|v| f32::from(v) / 255.0)
}

/// The GL buffer size for `needed` bytes: the next power of two, at least
/// [`MIN_CAPACITY`].
pub(crate) fn grow_capacity(needed: usize) -> usize {
    needed
        .checked_next_power_of_two()
        .unwrap_or(needed)
        .max(MIN_CAPACITY)
}
