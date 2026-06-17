/* check_terminal_window.rs

   This file is part of the ingest_and_snapshot project

   Copyright (c) 2026 Efthymios Kritikos

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.

   You should have received a copy of the GNU General Public License
   along with this program.  If not, see <http://www.gnu.org/licenses/>.  */

//! The "makeshift terminal" shown while a snapshot's check program runs. A [`vt100::Parser`]
//! interprets the check program's raw output — including the cursor moves produced by
//! `tput cuu/cuf/cud` / column positioning and SGR colour codes such as `\033[32m` — and its
//! screen grid is rendered into the terminal window. The bottom actions window offers the
//! context-dependent choices the snapshot logic publishes (e.g. "Exit & complete", "Destroy").

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use crossterm::event::{KeyCode, KeyEvent};
use crossbeam_channel::{Receiver, Sender};
use super::tui_dialog_widgets;
use crate::ui_api::{SnapshotUpdate, SnapshotActionButton, SnapshotActionStyle};

/// Initial emulator grid size used before the first render learns the real window dimensions.
const INITIAL_TERMINAL_ROWS: u16 = 24;
const INITIAL_TERMINAL_COLS: u16 = 80;
/// Lines of scrollback kept by the emulator. The window only shows the live screen, but a buffer
/// avoids losing content when the check program scrolls.
const TERMINAL_SCROLLBACK_LINES: usize = 1000;

/// Live state for the check-terminal layout. Created when the logic enters check-terminal mode and
/// dropped when it sends [`SnapshotUpdate::Exit`].
pub struct CheckTerminalState {
    parser: vt100::Parser,
    updates_rx: Receiver<SnapshotUpdate>,
    action_tx: Sender<u32>,
    actions: Vec<SnapshotActionButton>,
    selected: usize,
    /// The (rows, cols) the emulator is currently sized to, so render only resizes on change.
    sized_to: (u16, u16),
}

impl CheckTerminalState {
    pub fn new(updates_rx: Receiver<SnapshotUpdate>, action_tx: Sender<u32>) -> Self {
        Self {
            parser: vt100::Parser::new(INITIAL_TERMINAL_ROWS, INITIAL_TERMINAL_COLS, TERMINAL_SCROLLBACK_LINES),
            updates_rx,
            action_tx,
            actions: Vec::new(),
            selected: 0,
            sized_to: (INITIAL_TERMINAL_ROWS, INITIAL_TERMINAL_COLS),
        }
    }

    /// Drains all pending updates from the logic thread. Returns true once an [`SnapshotUpdate::Exit`]
    /// has been received, signalling the caller to leave check-terminal mode.
    pub fn drain_updates(&mut self) -> bool {
        let mut should_exit = false;
        while let Ok(update) = self.updates_rx.try_recv() {
            match update {
                SnapshotUpdate::Terminal(bytes) => self.parser.process(&bytes),
                SnapshotUpdate::SetActions(actions) => {
                    self.actions = actions;
                    if self.selected >= self.actions.len() {
                        self.selected = self.actions.len().saturating_sub(1);
                    }
                }
                SnapshotUpdate::Exit => should_exit = true,
            }
        }
        should_exit
    }

    pub fn handle_key(&mut self, key: KeyEvent) {
        match key.code {
            KeyCode::Left | KeyCode::Up => {
                self.selected = self.selected.saturating_sub(1);
            }
            KeyCode::Right | KeyCode::Down => {
                if !self.actions.is_empty() {
                    self.selected = (self.selected + 1).min(self.actions.len() - 1);
                }
            }
            KeyCode::Enter => {
                if let Some(action) = self.actions.get(self.selected) {
                    let _ = self.action_tx.send(action.id);
                }
            }
            // Number keys pick a button directly (1-based to match the rendered "[n]" labels).
            KeyCode::Char(c) if c.is_ascii_digit() && c != '0' => {
                let index = c as usize - '1' as usize;
                if let Some(action) = self.actions.get(index) {
                    self.selected = index;
                    let _ = self.action_tx.send(action.id);
                }
            }
            _ => {}
        }
    }
}

/// Maps a vt100 colour to the closest ratatui colour. `Default` resolves to the terminal window's
/// own defaults (light grey on black) so unstyled output stays readable.
fn vt_color_to_ratatui(color: vt100::Color, is_foreground: bool) -> Color {
    match color {
        vt100::Color::Default      => if is_foreground { Color::Gray } else { Color::Black },
        vt100::Color::Idx(index)   => Color::Indexed(index),
        vt100::Color::Rgb(r, g, b) => Color::Rgb(r, g, b),
    }
}

/// Rows reserved at the bottom of the check-terminal window for the action buttons: a separator
/// line plus the button line itself.
const ACTION_AREA_ROWS: u16 = 2;

/// Renders the whole check-terminal window: the display-only emulator screen fills the upper part
/// and the action buttons sit across the bottom of the same bordered window. Takes `&mut` because
/// the emulator grid is resized to match the current terminal area.
pub fn render(frame: &mut Frame, area: Rect, state: &mut CheckTerminalState) {
    let block = tui_dialog_widgets::DialogBlock::default().title("Check terminal");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 { return; }

    // Reserve the bottom rows for the action buttons when the window is tall enough; otherwise the
    // whole inner area is given to the terminal and the buttons are dropped.
    let has_action_area = inner.height > ACTION_AREA_ROWS;
    let terminal_height = if has_action_area { inner.height - ACTION_AREA_ROWS } else { inner.height };
    let terminal_area = Rect { x: inner.x, y: inner.y, width: inner.width, height: terminal_height };

    render_emulator(frame, terminal_area, state);

    if has_action_area {
        let separator_y = inner.y + terminal_height;
        let buttons_area = Rect { x: inner.x, y: separator_y + 1, width: inner.width, height: 1 };
        render_separator(frame, Rect { x: inner.x, y: separator_y, width: inner.width, height: 1 });
        render_actions(frame, buttons_area, &state.actions, state.selected);
    }
}

fn render_emulator(frame: &mut Frame, area: Rect, state: &mut CheckTerminalState) {
    if area.width == 0 || area.height == 0 { return; }

    // Keep the emulator grid the same size as the visible terminal area so the check program's
    // cursor moves and line wrapping line up with what is drawn.
    if state.sized_to != (area.height, area.width) {
        state.parser.screen_mut().set_size(area.height, area.width);
        state.sized_to = (area.height, area.width);
    }

    let screen = state.parser.screen();
    let buf = frame.buffer_mut();
    for row in 0..area.height {
        for col in 0..area.width {
            let x = area.x + col;
            let y = area.y + row;
            let cell = screen.cell(row, col);

            // Wide-character continuation cells are covered by the preceding wide glyph.
            if cell.map(|c| c.is_wide_continuation()).unwrap_or(false) {
                continue;
            }

            let (symbol, fg, bg, bold, italic, underline, inverse) = match cell {
                Some(cell) if cell.has_contents() => (
                    cell.contents().to_owned(),
                    vt_color_to_ratatui(cell.fgcolor(), true),
                    vt_color_to_ratatui(cell.bgcolor(), false),
                    cell.bold(), cell.italic(), cell.underline(), cell.inverse(),
                ),
                _ => (" ".to_owned(), Color::Gray, Color::Black, false, false, false, false),
            };

            // `inverse` swaps foreground and background, matching real terminals.
            let (fg, bg) = if inverse { (bg, fg) } else { (fg, bg) };
            let mut style = Style::default().fg(fg).bg(bg);
            if bold      { style = style.add_modifier(Modifier::BOLD); }
            if italic    { style = style.add_modifier(Modifier::ITALIC); }
            if underline { style = style.add_modifier(Modifier::UNDERLINED); }

            buf[(x, y)].set_symbol(&symbol).set_style(style);
        }
    }
}

/// Draws a thin divider between the display area and the action buttons.
fn render_separator(frame: &mut Frame, area: Rect) {
    let style = Style::default().fg(Color::DarkGray).bg(Color::Gray);
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
        buf[(x, area.y)].set_symbol("─").set_style(style);
    }
}

/// Renders the snapshot action buttons side by side. The terminal above is display-only; all
/// keyboard interaction drives the selection rendered here.
fn render_actions(frame: &mut Frame, area: Rect, actions: &[SnapshotActionButton], selected: usize) {
    if area.width == 0 || area.height == 0 { return; }

    let key_style = Style::default().fg(Color::Black);
    let mut spans: Vec<Span> = Vec::new();
    for (index, action) in actions.iter().enumerate() {
        let base_color = match action.style {
            SnapshotActionStyle::Confirm => Color::Green,
            SnapshotActionStyle::Danger  => Color::Red,
        };
        let is_selected = index == selected;
        let mut label_style = Style::default().fg(base_color).add_modifier(Modifier::BOLD);
        if is_selected {
            label_style = label_style.add_modifier(Modifier::REVERSED);
        }
        spans.push(Span::styled(format!("[{}] ", index + 1), key_style));
        spans.push(Span::styled(action.label.clone(), label_style));
        spans.push(Span::styled("   ", key_style));
    }

    let line_area = Rect { x: area.x + 1, y: area.y, width: area.width.saturating_sub(2), height: 1 };
    frame.render_widget(Paragraph::new(Line::from(spans)), line_area);
}
