//! The desktop palette and metrics: graphite neutrals with a faint cool cast
//! and one mint accent. Colors are straight sRGB; sizes are logical pixels.

use gfx::Rgba;

/// The desktop behind every window (the frame's clear color).
pub const BG: Rgba = Rgba::hex(0x121419);
/// The panel across the top of the screen.
pub const PANEL: Rgba = Rgba::hex(0x1a1d24);
/// A window's body.
pub const WINDOW: Rgba = Rgba::hex(0x1e2129);
/// The titlebar of an unfocused window.
pub const TITLEBAR: Rgba = Rgba::hex(0x252933);
/// The titlebar of the focused window.
pub const TITLEBAR_FOCUSED: Rgba = Rgba::hex(0x2d3240);
/// Window outlines and the panel's bottom edge.
pub const BORDER: Rgba = Rgba::hex(0x353a47);
/// The one accent: focus rings and the active workspace.
pub const ACCENT: Rgba = Rgba::hex(0x5fcfae);
/// The focused window's outline.
pub const BORDER_FOCUSED: Rgba = ACCENT;
/// Icons on the panel and on focused titlebars.
pub const ICON: Rgba = Rgba::hex(0xdce1e9);
/// Icons on unfocused titlebars and inactive workspace dots.
pub const ICON_DIM: Rgba = Rgba::hex(0x747b89);
/// A translucent wash behind the button under the pointer.
pub const HOVER: Rgba = Rgba(255, 255, 255, 26);
/// Window shadows; floating windows use a stronger alpha.
pub const SHADOW: Rgba = Rgba(0, 0, 0, 120);

/// Height of the panel.
pub const PANEL_H: f32 = 36.0;
/// Height of a window's titlebar.
pub const TITLEBAR_H: f32 = 30.0;
/// Corner radius of windows.
pub const RADIUS: f32 = 10.0;
/// Blur of a tiled window's shadow.
pub const SHADOW_BLUR: f32 = 18.0;
/// Outer and inner tiling gap.
pub const GAP: i32 = 10;
/// Number of workspaces.
pub const WORKSPACES: usize = 4;

/// A stable, soft color for window `id`: the hue steps by the golden angle
/// (137.508 degrees) per id at fixed saturation and value, so neighbors
/// differ and the same id always gets the same color.
pub fn app_tint(id: u32) -> Rgba {
    // Millidegrees in integers, so the hue is exact for every id.
    let h = ((u64::from(id) * 137_508) % 360_000) as f32 / 60_000.0;
    let (v, s) = (0.9, 0.45);
    let c = v * s;
    let x = c * (1.0 - (h % 2.0 - 1.0).abs());
    let (r, g, b) = match h as u32 {
        0 => (c, x, 0.0),
        1 => (x, c, 0.0),
        2 => (0.0, c, x),
        3 => (0.0, x, c),
        4 => (x, 0.0, c),
        _ => (c, 0.0, x),
    };
    let byte = |f: f32| ((f + v - c) * 255.0).round() as u8;
    Rgba(byte(r), byte(g), byte(b), 255)
}
