//! What the apps share: their icons.

use ui::icon::Glyph;
use ui::{AppIcon, Rgba};

/// The app icons.
pub(crate) const TERMINAL: AppIcon = AppIcon { glyph: Glyph::Terminal, hue: Rgba::hex(0x2dd4bf) };
