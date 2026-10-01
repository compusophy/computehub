//! The desktop palette and metrics: graphite neutrals with a faint cool cast
//! and one mint accent. Colors are straight sRGB; sizes are logical pixels.
//!
//! The shell's chrome, the widgets in [`crate::Ui`] and the terminal all read
//! this one palette.

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
/// Window outlines, separators and the panel's bottom edge.
pub const BORDER: Rgba = Rgba::hex(0x353a47);
/// The one accent: focus rings, carets, primary buttons and the active
/// workspace.
pub const ACCENT: Rgba = Rgba::hex(0x5fcfae);
/// A primary button under the pointer.
pub const ACCENT_HOVER: Rgba = Rgba::hex(0x7adcbe);
/// A primary button held down.
pub const ACCENT_PRESSED: Rgba = Rgba::hex(0x4bb394);
/// Text on an [`ACCENT`] fill.
pub const ON_ACCENT: Rgba = Rgba::hex(0x0d1f19);
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

/// Body text on [`WINDOW`].
pub const TEXT: Rgba = Rgba::hex(0xc9ced8);
/// Secondary text: captions, keys, placeholders.
pub const TEXT_DIM: Rgba = Rgba::hex(0x8a91a0);
/// Headings and text on hovered or pressed controls.
pub const TEXT_BRIGHT: Rgba = Rgba::hex(0xeef1f6);
/// A text field's well.
pub const FIELD: Rgba = Rgba::hex(0x171a20);
/// A focused text field's well.
pub const FIELD_FOCUSED: Rgba = Rgba::hex(0x13161b);
/// A button at rest.
pub const BUTTON: Rgba = Rgba::hex(0x2b303b);
/// A button under the pointer.
pub const BUTTON_HOVER: Rgba = Rgba::hex(0x363c49);
/// A button held down.
pub const BUTTON_PRESSED: Rgba = Rgba::hex(0x22262f);
/// Selected text: the accent, translucent, drawn over the glyphs' cells.
pub const SELECTION: Rgba = Rgba(95, 207, 174, 72);

/// The terminal's 16 colors, readable on [`WINDOW`]: black, red, green,
/// yellow, blue, magenta, cyan, white, then the bright eight. Black is lifted
/// off the background so black-on-default text still shows.
pub const ANSI: [Rgba; 16] = {
    let rgb: [u32; 16] = [
        0x3a3f4b, 0xe06c75, 0x7ccf9a, 0xe5c07b, 0x61afef, 0xc678dd, 0x56b6c2, 0xc9ced8, //
        0x6e7585, 0xf07f88, 0x9fe0b4, 0xf0d58c, 0x80c2f5, 0xd79bea, 0x70d3de, 0xeef1f6,
    ];
    let mut out = [Rgba(0, 0, 0, 255); 16];
    let mut i = 0;
    while i < 16 {
        out[i] = Rgba::hex(rgb[i]);
        i += 1;
    }
    out
};

/// An xterm 256-color index: 0-15 are [`ANSI`], 16-231 the 6 x 6 x 6 cube
/// (levels 0, 95, 135, 175, 215, 255), 232-255 the gray ramp from 8 to 238
/// in steps of 10.
pub fn xterm_color(i: u8) -> Rgba {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    match i {
        0..=15 => ANSI[usize::from(i)],
        16..=231 => {
            let n = usize::from(i - 16);
            Rgba(LEVELS[n / 36], LEVELS[n / 6 % 6], LEVELS[n % 6], 255)
        }
        _ => {
            let v = 8 + 10 * (i - 232);
            Rgba(v, v, v, 255)
        }
    }
}

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
