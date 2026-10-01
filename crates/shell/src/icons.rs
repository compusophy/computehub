//! The desktop's icons behind the windows (see `home::icons`).

use gfx::DrawList;
use host::rectf;
use ui::Theme;

use crate::Shell;
use crate::desktop::Target;

impl Shell {
    /// Every icon in its cell, washed while hovered (more while pressed).
    pub(crate) fn draw_icons(&mut self, list: &mut DrawList, theme: &Theme) {
        let (a, narrow) = (rectf(self.host.wm().area()), self.host.narrow());
        for (i, e) in self.icons.iter().enumerate() {
            let hot = Some(Target::Icon(i));
            let hover = (self.hover == hot).then_some(self.armed == hot);
            let r = home::icons::cell(i, a, narrow);
            home::icons::draw(list, &mut self.host.text, theme, r, (e.icon, &e.label), hover);
        }
    }
}
