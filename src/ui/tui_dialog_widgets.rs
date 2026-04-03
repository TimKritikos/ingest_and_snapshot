use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use ratatui::widgets::Widget;
use ratatui::buffer::Buffer;

pub struct DialogSelectionList<'a> {
    pub title: Option<&'a str>,
    pub items: Vec<&'a str>,
    pub selected: Option<usize>,
    pub focused: bool,
}

impl<'a> DialogSelectionList<'a> {
    pub fn new(items: Vec<&'a str>) -> Self {
        Self {
            title: None,
            items,
            selected: None,
            focused: true,
        }
    }
    pub fn title(mut self, t: &'a str) -> Self {
        self.title = Some(t);
        self
    }
    pub fn selected(mut self, idx: Option<usize>) -> Self {
        self.selected = idx;
        self
    }
    pub fn focused(mut self, f: bool) -> Self {
        self.focused = f;
        self
    }
}

impl Widget for DialogSelectionList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {

        let selected_focused = Style::default()
            .fg(Color::Gray)
            .bg(Color::Blue)
            .add_modifier(Modifier::BOLD);

        let selected_unfocused = Style::default()
            .bg(Color::White)
            .fg(Color::Black);

        // Write background
        let bg = Style::default().bg(Color::Gray).fg(Color::Black);
        for y in area.top()..area.bottom() {
            for x in area.left()..area.right() {
                buf[(x, y)].set_style(bg);
            }
        }

        // Write the title
        if let Some(title) = self.title {
            let title_style = bg.add_modifier(Modifier::BOLD);
            buf.set_string(area.x + 1, area.y, title, title_style);
        }

        // Write the box
        let box_area = Rect {
            x: area.x,
            y: area.y + 1,
            width: area.width,
            height: area.height - 1,
        };

        let light = Style::default().fg(Color::White).bg(Color::Gray);
        let dark = Style::default().fg(Color::DarkGray).bg(Color::Gray);
        for x in box_area.left()..box_area.right() {
            buf[(x, box_area.top())].set_symbol("─").set_style(dark);
            buf[(x, box_area.bottom()-1)].set_symbol("─").set_style(light);
        }

        for y in box_area.top()..box_area.bottom() {
            buf[(box_area.left(), y)].set_symbol("│").set_style(dark);
            buf[(box_area.right()-1, y)].set_symbol("│").set_style(light);
        }

        buf[(box_area.left(), box_area.top())].set_symbol("┌").set_style(dark);
        buf[(box_area.right()-1, box_area.top())].set_symbol("┐").set_style(light);
        buf[(box_area.left(), box_area.bottom()-1)].set_symbol("└").set_style(dark);
        buf[(box_area.right()-1, box_area.bottom()-1)].set_symbol("┘").set_style(light);

        // Write the items
        let inner = Rect {
            x: box_area.x + 1,
            y: box_area.y + 1,
            width: box_area.width.saturating_sub(2),
            height: box_area.height.saturating_sub(2),
        };

        for (i, item) in self.items.iter().enumerate() {
            if i as u16 >= inner.height {
                break;
            }

            let y = inner.y + i as u16;


            let style = if Some(i) == self.selected {
                if self.focused {
                    selected_focused
                } else {
                    selected_unfocused
                }
            } else {
                bg
            };

            // clear row
            if style != bg {
                for x in inner.left()..inner.right() {
                    buf[(x, y)].set_style(style);
                }
            }

            let text_width = item.len() as u16;
            let x = inner.x + (inner.width.saturating_sub(text_width)) / 2;

            buf.set_string(x, y, *item, style);
        }
    }
}

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
