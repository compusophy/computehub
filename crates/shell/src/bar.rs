//! The top bar (see `home::bar`), wired to the pointer.

use gfx::DrawList;
use ui::Theme;

use crate::Shell;
use crate::desktop::Target;

impl Shell {
    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Target {
        home::bar::at(self.size.0, x, y).map_or(Target::Top, Target::Bar)
    }

    /// The bar, its hovered button washed (more while pressed), its clock once it ticked.
    pub(crate) fn draw_bar(&mut self, list: &mut DrawList, theme: &Theme) {
        let hover = match self.hover {
            Some(Target::Bar(b)) => Some((b, self.armed == self.hover)),
            _ => None,
        };
        let (w, text) = (self.size.0, &mut self.host.text);
        home::bar::draw(list, text, theme, w, (hover, self.clock));
    }
}
