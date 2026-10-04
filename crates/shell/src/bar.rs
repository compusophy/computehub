//! The top bar (see `home::bar`), wired to the pointer; Show desktop, the mark's (and mod+D's).

use gfx::DrawList;
use ui::Theme;
use wm::Cmd;

use crate::Shell;
use crate::desktop::Target;

impl Shell {
    pub(crate) fn bar_hit(&self, x: f32, y: f32) -> Target {
        home::bar::at(self.size.0, x, y).map_or(Target::Top, Target::Bar)
    }

    /// Show desktop: closes the open folder and menu, and minimizes every window that shows (each
    /// into its tile); on a bare desktop, brings back those it last minimized that are still
    /// open, in their stacking order, the top focused. A window shown in between forgets them
    /// (`Shell::settle`), so the next press minimizes again.
    pub(crate) fn show_desktop(&mut self) {
        (self.menu, self.grid.open) = (None, None);
        let shown: Vec<_> = self.host.wm().layout().iter().map(|p| p.win).collect();
        if shown.is_empty() {
            for win in std::mem::take(&mut self.bare) {
                self.host.apply(Cmd::Focus(win));
            }
            return;
        }
        for &win in &shown {
            self.host.apply(Cmd::Minimize(win));
        }
        self.bare = shown;
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
