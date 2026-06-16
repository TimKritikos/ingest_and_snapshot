use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Widget, Paragraph};
use ratatui::buffer::Buffer;
use crossterm::event::KeyCode;

pub struct DialogSelectionList<'a> {
    pub title: Option<&'a str>,
    pub items: Vec<&'a str>,
    pub selected: Option<usize>,
    pub focused: bool,
    pub disabled_indices: Vec<usize>,
}

impl<'a> DialogSelectionList<'a> {
    pub fn new(items: Vec<&'a str>) -> Self {
        Self {
            title: None,
            items,
            selected: None,
            focused: true,
            disabled_indices: Vec::new(),
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
    pub fn disabled_indices(mut self, indices: Vec<usize>) -> Self {
        self.disabled_indices = indices;
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

        let disabled = Style::default()
            .fg(Color::DarkGray)
            .bg(Color::Gray);

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
        if area.height < 3 { return; }
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


            let style = if self.disabled_indices.contains(&i) {
                disabled
            } else if Some(i) == self.selected {
                if self.focused { selected_focused } else { selected_unfocused }
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
                buf[(x, y)].reset();
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

pub enum TextEntryOutcome {
    Editing,
    Confirmed(String),
    Cancelled,
}

/// Holds the mutable state for a text entry: the current string and cursor position.
pub struct TextEntryState {
    pub text: String,
    pub cursor: usize, // byte offset
}

impl TextEntryState {
    pub fn new(initial: String) -> Self {
        let cursor = initial.len();
        Self { text: initial, cursor }
    }

    pub fn handle_key(&mut self, key: KeyCode) -> TextEntryOutcome {
        match key {
            KeyCode::Enter => TextEntryOutcome::Confirmed(self.text.clone()),
            KeyCode::Esc   => TextEntryOutcome::Cancelled,
            KeyCode::Char(c) => {
                self.text.insert(self.cursor, c);
                self.cursor += c.len_utf8();
                TextEntryOutcome::Editing
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let prev = self.text[..self.cursor]
                        .char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
                    self.text.drain(prev..self.cursor);
                    self.cursor = prev;
                }
                TextEntryOutcome::Editing
            }
            KeyCode::Delete => {
                if self.cursor < self.text.len() {
                    let next_len = self.text[self.cursor..]
                        .chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                    self.text.drain(self.cursor..self.cursor + next_len);
                }
                TextEntryOutcome::Editing
            }
            KeyCode::Left => {
                if self.cursor > 0 {
                    self.cursor = self.text[..self.cursor]
                        .char_indices().next_back().map(|(i, _)| i).unwrap_or(0);
                }
                TextEntryOutcome::Editing
            }
            KeyCode::Right => {
                if self.cursor < self.text.len() {
                    self.cursor += self.text[self.cursor..]
                        .chars().next().map(|c| c.len_utf8()).unwrap_or(0);
                }
                TextEntryOutcome::Editing
            }
            KeyCode::Home => { self.cursor = 0; TextEntryOutcome::Editing }
            KeyCode::End  => { self.cursor = self.text.len(); TextEntryOutcome::Editing }
            _ => TextEntryOutcome::Editing,
        }
    }
}

/// A floating text-entry widget. Renders a centered dialog box containing the title,
/// the current text with a cursor, and a key hint line at the bottom.
pub struct TextEntry<'a> {
    pub title: &'a str,
    pub text: &'a str,
    pub cursor_pos: usize, // byte offset into text
}

impl Widget for TextEntry<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let box_w = (area.width / 2).max(40).min(area.width.saturating_sub(4));
        let box_h: u16 = 5; // title row + blank + input row + blank + hint row, inside border = 3 rows + 2 border = 5
        if area.width < box_w || area.height < box_h { return; }

        let box_area = Rect {
            x: area.x + (area.width.saturating_sub(box_w)) / 2,
            y: area.y + (area.height.saturating_sub(box_h)) / 2,
            width: box_w,
            height: box_h,
        };

        let fill = Style::default().bg(Color::Gray).fg(Color::Black);
        for y in box_area.top()..box_area.bottom() {
            for x in box_area.left()..box_area.right() {
                buf[(x, y)].reset();
                buf[(x, y)].set_style(fill);
            }
        }

        let block = DialogBlock::default().title(self.title);
        let inner = block.inner(box_area);
        block.render(box_area, buf);

        if inner.height < 3 || inner.width < 2 { return; }

        // Text input row (first inner row)
        let input_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: 1 };
        let input_style    = Style::default().fg(Color::Black).bg(Color::White);
        let cursor_style   = Style::default().fg(Color::White).bg(Color::Black).add_modifier(Modifier::BOLD);

        for x in input_area.left()..input_area.right() {
            buf[(x, input_area.y)].set_char(' ').set_style(input_style);
        }

        let display_text: Vec<char> = self.text.chars().collect();
        let cursor_char_pos = self.text[..self.cursor_pos].chars().count();
        let visible_width   = input_area.width as usize;

        // Scroll so cursor is always visible
        let scroll_start = if cursor_char_pos >= visible_width {
            cursor_char_pos + 1 - visible_width
        } else {
            0
        };

        for (i, &ch) in display_text.iter().enumerate().skip(scroll_start) {
            let x_offset = i - scroll_start;
            if x_offset >= visible_width { break; }
            let x = input_area.x + x_offset as u16;
            let style = if i == cursor_char_pos { cursor_style } else { input_style };
            buf[(x, input_area.y)].set_char(ch).set_style(style);
        }
        // Draw cursor at end if cursor is past all text
        if cursor_char_pos >= display_text.len() {
            let x_offset = display_text.len().saturating_sub(scroll_start);
            if x_offset < visible_width {
                buf[(input_area.x + x_offset as u16, input_area.y)]
                    .set_char(' ')
                    .set_style(cursor_style);
            }
        }

        // Hint row (last inner row)
        let hint_y = inner.y + inner.height - 1;
        let ok_style   = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
        let esc_style  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        let hint_style = Style::default().fg(Color::Black);
        let hint_area  = Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 };
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", ok_style),
            Span::styled(" Confirm  ", hint_style),
            Span::styled("[Esc]", esc_style),
            Span::styled(" Cancel", hint_style),
        ])).render(hint_area, buf);
    }
}

pub struct DialogFloatingListItem {
    pub label: String,
    pub is_current: bool,
}

pub struct DialogFloatingList<'a> {
    title: &'a str,
    items: Vec<DialogFloatingListItem>,
    selected: usize,
    hint: Option<Line<'a>>,
}

impl<'a> DialogFloatingList<'a> {
    pub fn new(title: &'a str) -> Self {
        Self { title, items: Vec::new(), selected: 0, hint: None }
    }
    pub fn items(mut self, items: Vec<DialogFloatingListItem>) -> Self {
        self.items = items;
        self
    }
    pub fn selected(mut self, s: usize) -> Self {
        self.selected = s;
        self
    }
    pub fn hint(mut self, hint: Line<'a>) -> Self {
        self.hint = Some(hint);
        self
    }
}

impl Widget for DialogFloatingList<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let hint_rows: u16 = if self.hint.is_some() { 1 } else { 0 };
        let picker_h = (self.items.len() as u16 + hint_rows + 2)
            .max(5)
            .min(area.height.saturating_sub(4));
        let picker_w = (area.width / 2).max(40).min(area.width.saturating_sub(4));
        let picker_area = Rect {
            x: area.x + (area.width.saturating_sub(picker_w)) / 2,
            y: area.y + (area.height.saturating_sub(picker_h)) / 2,
            width: picker_w,
            height: picker_h,
        };

        let clear_style = Style::default().bg(Color::Gray).fg(Color::Black);
        for y in picker_area.top()..picker_area.bottom() {
            for x in picker_area.left()..picker_area.right() {
                buf[(x, y)].reset();
                buf[(x, y)].set_style(clear_style);
            }
        }

        let block = DialogBlock::default().title(self.title);
        let inner = block.inner(picker_area);
        block.render(picker_area, buf);

        let item_style     = Style::default().fg(Color::Black);
        let selected_style = Style::default().fg(Color::White).bg(Color::Blue).add_modifier(Modifier::BOLD);
        let current_style  = Style::default().fg(Color::Green); //for the green tickmark

        let items_height = inner.height.saturating_sub(hint_rows);

        for (i, item) in self.items.iter().enumerate() {
            if i as u16 >= items_height { break; }
            let y = inner.y + i as u16;
            let is_selected = self.selected == i;
            let row_style = if is_selected { selected_style } else { item_style };

            for x in inner.x..inner.x + inner.width {
                buf[(x, y)].set_style(row_style);
            }

            let marker = if item.is_current { "✓ " } else { "  " };
            let marker_style = if item.is_current { current_style } else { row_style };
            let row_area = Rect { x: inner.x, y, width: inner.width, height: 1 };
            Paragraph::new(Line::from(vec![
                Span::styled(marker, marker_style),
                Span::styled(item.label.as_str(), row_style),
            ])).render(row_area, buf);
        }

        if let Some(hint) = self.hint {
            let hint_y = inner.y + inner.height.saturating_sub(1);
            if inner.height > 0 {
                let hint_area = Rect { x: inner.x, y: hint_y, width: inner.width, height: 1 };
                Paragraph::new(hint).render(hint_area, buf);
            }
        }
    }
}
