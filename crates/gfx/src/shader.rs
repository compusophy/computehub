//! The GLSL ES 3.00 program that reads the instance bytes. It lives next to
//! [`crate::DrawList::encode_into`] so the byte contract and its only reader
//! change together.

/// Vertex shader: builds each instance's quad from `gl_VertexID` 0..3 (draw
/// as `TRIANGLE_STRIP`, no vertex buffer). The quad is the rect grown by the
/// blur for [`crate::Kind::Shadow`] (at least one physical pixel), not at all
/// for [`crate::Kind::Glyph`] (glyphs sit on device pixels), and by one
/// physical pixel otherwise, so antialiased edges are never cut; then it is
/// clamped to the clip rect. It passes on the logical position (for the
/// clip test) and the atlas pixel position (for glyphs).
pub const VERTEX_SHADER: &str = r"#version 300 es
precision highp float;
layout(location = 0) in vec4 a_rect;
layout(location = 1) in vec4 a_params;
layout(location = 2) in vec4 a_color;
layout(location = 3) in vec4 a_clip;
layout(location = 4) in vec4 a_uv;
uniform vec2 u_viewport;
uniform float u_dpr;
out vec2 v_local;
out vec2 v_pos;
out vec2 v_tex;
flat out vec2 v_half;
flat out vec4 v_params;
flat out vec4 v_color;
flat out vec4 v_clip;
void main() {
  vec2 corner = vec2(float(gl_VertexID & 1), float(gl_VertexID >> 1));
  float px = 1.0 / u_dpr;
  int kind = int(a_params.y + 0.5);
  float grow = kind == 2 ? max(a_params.z, px) : (kind == 4 ? 0.0 : px);
  vec2 hs = a_rect.zw * 0.5;
  vec2 center = a_rect.xy + hs;
  vec2 pos = center + (corner * 2.0 - 1.0) * (hs + grow);
  pos = clamp(pos, a_clip.xy, a_clip.xy + a_clip.zw);
  vec2 local = pos - center;
  v_local = local;
  v_pos = pos;
  v_tex = a_uv.xy + (local + hs) / a_rect.zw * a_uv.zw;
  v_half = hs;
  v_params = a_params;
  v_color = a_color;
  v_clip = a_clip;
  vec2 ndc = pos / u_viewport * 2.0 - 1.0;
  gl_Position = vec4(ndc.x, -ndc.y, 0.0, 1.0);
}
";

/// Fragment shader: coverage from the signed distance to the rounded rect
/// (or to the icon), antialiased over one physical pixel, or from the atlas
/// texel's red channel for glyphs; zero outside the clip rect; written
/// premultiplied for `blendFunc(ONE, ONE_MINUS_SRC_ALPHA)`.
pub const FRAGMENT_SHADER: &str = r"#version 300 es
precision highp float;
in vec2 v_local;
in vec2 v_pos;
in vec2 v_tex;
flat in vec2 v_half;
flat in vec4 v_params;
flat in vec4 v_color;
flat in vec4 v_clip;
uniform float u_dpr;
uniform sampler2D u_atlas;
uniform vec2 u_atlas_size;
out vec4 fragColor;

float rbox(vec2 p, vec2 b, float r) {
  vec2 q = abs(p) - b + r;
  return length(max(q, 0.0)) + min(max(q.x, q.y), 0.0) - r;
}

float seg(vec2 p, vec2 a, vec2 b) {
  vec2 pa = p - a;
  vec2 ba = b - a;
  float h = clamp(dot(pa, ba) / dot(ba, ba), 0.0, 1.0);
  return length(pa - ba * h);
}

float icon(vec2 p, float s, int id, float w) {
  float hw = w * 0.5;
  float l = 0.35 * s;
  if (id == 1) {
    p = vec2(p.x + p.y, p.y - p.x) * 0.70710678;
  }
  float bar = seg(p, vec2(-l, 0.0), vec2(l, 0.0));
  if (id == 0 || id == 1) {
    return min(bar, seg(p, vec2(0.0, -l), vec2(0.0, l))) - hw;
  }
  if (id == 2) {
    return bar - hw;
  }
  if (id == 3) {
    return length(p) - 0.2 * s;
  }
  if (id == 4) {
    return abs(rbox(p, vec2(0.28 * s), 0.0)) - hw;
  }
  return length(abs(p) - 0.22 * s) - 0.12 * s;
}

void main() {
  float aa = 1.0 / u_dpr;
  int kind = int(v_params.y + 0.5);
  float d = rbox(v_local, v_half, v_params.x);
  float cov;
  if (kind == 1) {
    float w = v_params.z;
    cov = clamp(0.5 - (abs(d + w * 0.5) - w * 0.5) / aa, 0.0, 1.0);
  } else if (kind == 2) {
    float b = max(v_params.z, aa);
    cov = 1.0 - smoothstep(-b, b, d);
  } else if (kind == 3) {
    float s = 2.0 * min(v_half.x, v_half.y);
    float di = icon(v_local, s, int(v_params.z + 0.5), v_params.w);
    cov = clamp(0.5 - di / aa, 0.0, 1.0);
  } else if (kind == 4) {
    cov = textureLod(u_atlas, v_tex / u_atlas_size, 0.0).r;
  } else {
    cov = clamp(0.5 - d / aa, 0.0, 1.0);
  }
  if (any(lessThan(v_pos, v_clip.xy)) || any(greaterThanEqual(v_pos, v_clip.xy + v_clip.zw))) {
    cov = 0.0;
  }
  float a = v_color.a * cov;
  fragColor = vec4(v_color.rgb * a, a);
}
";
