//! The GLSL ES 3.00 program that reads the [`crate::INSTANCE_BYTES`] layout.

/// Vertex shader: each instance's quad from `gl_VertexID` (a 4-vertex
/// `TRIANGLE_STRIP`, no vertex buffer), grown by the blur for a shadow, by
/// nothing for a glyph or glow and by one physical pixel otherwise, then
/// clamped to the clip. Uniforms: `u_viewport` (logical size) and `u_dpr`.
pub const VERTEX_SHADER: &str = r"#version 300 es
precision highp float;
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_params;
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec4 a_clip;
layout(location = 4) in vec4 a_uv;
layout(location = 5) in vec4 a_color2;
uniform vec2 u_viewport;
uniform float u_dpr;
out vec2 v_local, v_pos, v_tex;
flat out vec2 v_half;
flat out vec4 v_params, v_color, v_clip, v_color2;
void main() {
  vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
  float px = 1.0 / u_dpr;
  int kind = int(a_params.y + 0.5);
  float grow = kind == 2 ? max(a_params.z, px) : (kind == 4 || kind == 6 ? 0.0 : px);
  vec2 hs = a_rect.zw * 0.5;
  vec2 center = a_rect.xy + hs;
  vec2 pos = clamp(center + (corner * 2.0 - 1.0) * (hs + grow), a_clip.xy, a_clip.xy + a_clip.zw);
  vec2 local = pos - center;
  v_local = local;
  v_pos = pos;
  v_tex = a_uv.xy + (local + hs) / a_rect.zw * a_uv.zw;
  v_half = hs;
  v_params = a_params;
  v_color = a_color;
  v_clip = a_clip;
  v_color2 = a_color2;
  vec2 ndc = pos / u_viewport * 2.0 - 1.0;
  gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
";

/// Fragment shader: coverage from the signed distance to the rounded rect or
/// icon (antialiased over one physical pixel), the atlas texel's red channel,
/// or the glow falloff; zero outside the clip; written premultiplied.
/// Shadows, gradients and glows are dithered by under half an 8-bit step.
/// Uniforms: `u_atlas` (an `R8` `NEAREST` texture on unit 0), `u_atlas_size`.
pub const FRAGMENT_SHADER: &str = r"#version 300 es
precision highp float;
precision highp int;
in vec2 v_local, v_pos, v_tex;
flat in vec2 v_half;
flat in vec4 v_params, v_color, v_clip, v_color2;
uniform float u_dpr;
uniform sampler2D u_atlas;
uniform vec2 u_atlas_size;
out vec4 fragColor;

float rbox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float seg(vec2 p, vec2 a, vec2 b) {
  vec2 pa = p - a, ba = b - a;
  return length(pa - ba * clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0));
}

float icon(vec2 p, float s, int id, float w) {
  float hw = w * 0.5, l = 0.35 * s;
  if (id == 1) p = vec2(p.x + p.y, p.y - p.x) * 0.70710678;
  float bar = seg(p, vec2(-l, 0.0), vec2(l, 0.0));
  if (id == 0 || id == 1) return min(bar, seg(p, vec2(0.0, -l), vec2(0.0, l))) - hw;
  if (id == 2) return bar - hw;
  if (id == 3) return length(p) - 0.2 * s;
  if (id == 4) return abs(rbox(p, vec2(0.28 * s), 0.0)) - hw;
  return length(abs(p) - 0.22 * s) - 0.12 * s;
}

uint mix32(uint x) {
  x ^= x >> 16;
  x *= 0x7feb352du;
  x ^= x >> 15;
  x *= 0x846ca68bu;
  return x ^ (x >> 16);
}

uint pixel_hash(uint seed) {
  uvec2 p = uvec2(gl_FragCoord.xy);
  return mix32(p.x ^ mix32(p.y ^ mix32(seed)));
}

void main() {
  float aa = 1.0 / u_dpr;
  int kind = int(v_params.y + 0.5);
  float d = rbox(v_local, v_half, v_params.x);
  vec4 c = vec4(v_color.rgb * v_color.a, v_color.a);
  float cov = clamp(0.5 - d / aa, 0.0, 1.0);
  bool ramp = kind == 2 || kind == 5 || kind == 6;
  if (kind == 1) {
    float w = v_params.z;
    cov = clamp(0.5 - (abs(d + w * 0.5) - w * 0.5) / aa, 0.0, 1.0);
  } else if (kind == 2) {
    float b = max(v_params.z, aa);
    cov = 1.0 - smoothstep(-b, b, d);
  } else if (kind == 3) {
    float s = 2.0 * min(v_half.x, v_half.y);
    cov = clamp(0.5 - icon(v_local, s, int(v_params.z + 0.5), v_params.w) / aa, 0.0, 1.0);
  } else if (kind == 4) {
    cov = textureLod(u_atlas, v_tex / u_atlas_size, 0.0).r;
  } else if (kind == 5) {
    vec2 dir = vec2(sin(v_params.z), cos(v_params.z));
    float reach = max(dot(abs(dir), v_half), 1e-6);
    float t = clamp(0.5 + 0.5 * dot(v_local, dir) / reach, 0.0, 1.0);
    c = mix(c, vec4(v_color2.rgb * v_color2.a, v_color2.a), t);
  } else if (kind == 6) {
    vec2 q = v_local / v_half;
    float e = clamp(1.0 - dot(q, q), 0.0, 1.0);
    cov = e * e;
  } else if (kind == 7) {
    uint h = pixel_hash(floatBitsToUint(v_params.z));
    float n = (float(h & 0xffffu) + float(h >> 16)) / 65535.0 - 1.0;
    float a = abs(n) * v_color.a;
    c = vec4(vec3(n > 0.0 ? a : 0.0), a);
  }
  if (any(lessThan(v_pos, v_clip.xy)) || any(greaterThanEqual(v_pos, v_clip.xy + v_clip.zw))) {
    cov = 0.0;
  }
  fragColor = c * cov;
  if (ramp && fragColor.a > 0.0) {
    uint h = pixel_hash(uint(kind));
    fragColor.rgb += (float(h & 0xffffu) / 65536.0 - 0.5) / 256.0;
  }
}
";
