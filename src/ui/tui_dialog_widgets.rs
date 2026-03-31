use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use ratatui::widgets::Widget;
use ratatui::buffer::Buffer;

#[derive(Clone)]
pub struct DialogBlock<'a> {
    title: Option<&'a str>,
    style: Style,
}

impl<'a> Default for DialogBlock<'a> {
    fn default() -> Self {
        Self {
            title: None,
            style: Style::default().bg(Color::Gray).fg(Color::Black),
        }
    }
}

impl<'a> DialogBlock<'a> {

    pub fn title(mut self, title: &'a str) -> Self {
        self.title = Some(title);
        self
    }

    pub fn inner(&self, area: Rect) -> Rect {
        Rect {
            x: area.x + 1,
            y: area.y + 1,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(2),
        }
    }
}

impl Widget for DialogBlock<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {

        let light = Style::default().fg(Color::White).bg(self.style.bg.unwrap_or(Color::Gray));
        let dark = Style::default().fg(Color::DarkGray).bg(self.style.bg.unwrap_or(Color::Gray));
        let fill = self.style;

        for y in area.y + 1..area.y + area.height + 1 {
            for x in area.x + 2..area.x + area.width + 2 {
                if x < buf.area.width && y < buf.area.height {
                    buf[(x, y)].set_style(Style::default().bg(Color::Black));
                }
            }
        }

        // fill background
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_style(fill);
            }
        }

        // borders
        for x in area.left()..area.right() {
            buf[(x, area.top())].set_symbol("─").set_style(light);
            buf[(x, area.bottom()-1)].set_symbol("─").set_style(dark);
        }

        for y in area.top()..area.bottom() {
            buf[(area.left(), y)].set_symbol("│").set_style(light);
            buf[(area.right()-1, y)].set_symbol("│").set_style(dark);
        }

        buf[(area.left(), area.top())].set_symbol("┌").set_style(light);
        buf[(area.right()-1, area.top())].set_symbol("┐").set_style(dark);
        buf[(area.left(), area.bottom()-1)].set_symbol("└").set_style(light);
        buf[(area.right()-1, area.bottom()-1)].set_symbol("┘").set_style(dark);

        // title
        if let Some(title) = self.title {
            let x = area.x + (area.right()-area.left())/60;
            buf.set_string(x, area.y, title, fill.add_modifier(Modifier::BOLD));
        }
    }
}
