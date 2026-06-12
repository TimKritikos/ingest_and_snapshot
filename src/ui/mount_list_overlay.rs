use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use crossterm::event::{KeyCode, KeyEvent};
use crate::ui_api::{MountEntry, MountEntryStatus, MountId, LoadingField};

const OVERLAY_BG: Color       = Color::Rgb(25, 25, 40);
const SELECTED_BG: Color      = Color::Rgb(60, 60, 95);
const HINT_FG: Color          = Color::DarkGray;
const KEY_FG: Color           = Color::Cyan;
const TITLE_FG: Color         = Color::White;
const SEPARATOR_FG: Color     = Color::DarkGray;
const NORMAL_TEXT_FG: Color   = Color::White;

pub struct MountListState {
    pub selected: usize,
}

impl MountListState {
    pub fn new() -> Self {
        Self { selected: 0 }
    }
}

pub enum MountListEvent {
    Unmount(MountId),
    Close,
}

/// Height the overlay needs for the given mount list (capped at max_height).
pub fn required_height(mounts: &[MountEntry], max_height: u16) -> u16 {
    // 1 emptry row + 1 separator + 1 title row + max(1, count) entry rows + 1 separator + 1 hint row + 1 more empty row = count + 6
    // (min 1 entry row to show the "empty" message)
    let entry_rows = mounts.len().max(1) as u16;
    (entry_rows + 6).min(max_height)
}

pub fn handle_key(
    state: &mut MountListState,
    key: KeyEvent,
    mounts: &[MountEntry],
) -> Option<MountListEvent> {
    match key.code {
        KeyCode::Esc => return Some(MountListEvent::Close),
        KeyCode::Char('f') | KeyCode::Char('F') => return Some(MountListEvent::Close),
        KeyCode::Up => {
            if state.selected > 0 {
                state.selected -= 1;
            }
        }
        KeyCode::Down => {
            if !mounts.is_empty() && state.selected + 1 < mounts.len() {
                state.selected += 1;
            }
        }
        KeyCode::Char('u') | KeyCode::Char('U') => {
            if let Some(entry) = mounts.get(state.selected) {
                return Some(MountListEvent::Unmount(entry.id));
            }
        }
        _ => {}
    }
    None
}

pub fn render(frame: &mut Frame, area: Rect, mounts: &[MountEntry], state: &MountListState) {
    if area.height < 3 { return; }

    // ── Background fill ───────────────────────────────────────────────────────────
    let fill_style = Style::default().bg(OVERLAY_BG).fg(NORMAL_TEXT_FG);
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            frame.buffer_mut()[(x, y)]
                .set_char(' ')
                .reset();
            frame.buffer_mut()[(x, y)]
                .set_style(fill_style);
        }
    }

    // ── Separator ─────────────────────────────────────────────────────────────────
    let sep_style = Style::default().fg(SEPARATOR_FG).bg(OVERLAY_BG);
    let buf = frame.buffer_mut();
    for x in area.left()..area.right() {
        buf[(x, 2)].set_char('─').set_style(sep_style);
    }

    // ── Title row ─────────────────────────────────────────────────────────────────
    let title_style = Style::default().fg(TITLE_FG).bg(OVERLAY_BG).add_modifier(Modifier::BOLD);
    let title_text  = format!("    status                    mountpoint                        filesystem      Device                  by-id");
    {
        let buf = frame.buffer_mut();
        buf.set_string(area.x, area.y + 2, &title_text, title_style);
        let title_end = area.x.saturating_add(title_text.len() as u16);
        for x in title_end..area.right() {
            buf[(x, area.y+2)].set_style(title_style);
        }
    }

    if area.height < 3 { return; }

    // Layout: title(1) + entries(n) + separator(1) + hint(1)
    let entries_start_y     = area.y + 3;
    let hint_y              = area.bottom() - 2;
    let separator_y         = hint_y.saturating_sub(1);
    let max_visible_entries = separator_y.saturating_sub(entries_start_y) as usize;

    // ── Scroll so selected is visible ────────────────────────────────────────────
    let scroll_start = if max_visible_entries > 0 && state.selected >= max_visible_entries {
        state.selected + 1 - max_visible_entries
    } else {
        0
    };

    // ── Entry rows ────────────────────────────────────────────────────────────────
    if mounts.is_empty() {
        let empty_style = Style::default().fg(HINT_FG).bg(OVERLAY_BG);
        frame.buffer_mut().set_string(area.x, entries_start_y, "  No filesystems mounted", empty_style);
    } else {
        for (list_idx, entry) in mounts.iter().enumerate().skip(scroll_start) {
            let row_offset = list_idx - scroll_start;
            if row_offset >= max_visible_entries { break; }
            let y = entries_start_y + row_offset as u16;
            render_entry_row(frame, area.x, y, area.width, entry, list_idx == state.selected);
        }
    }

    // ── Separator ─────────────────────────────────────────────────────────────────
    if separator_y > entries_start_y && separator_y < area.bottom() {
        let sep_style = Style::default().fg(SEPARATOR_FG).bg(OVERLAY_BG);
        let buf = frame.buffer_mut();
        for x in area.left()..area.right() {
            buf[(x, separator_y)].set_char('─').set_style(sep_style);
        }
    }

    // ── Hint row ──────────────────────────────────────────────────────────────────
    // If the selected entry is in an error state, show the error instead of normal hints.
    if hint_y < area.bottom() {
        let hint_line = build_hint_line(mounts.get(state.selected));
        let hint_area = Rect { x: area.x, y: hint_y, width: area.width, height: 1 };
        {
            let buf = frame.buffer_mut();
            for x in hint_area.left()..hint_area.right() {
                buf[(x, hint_y)].set_style(fill_style);
            }
        }
        frame.render_widget(Paragraph::new(hint_line), hint_area);
    }
}

fn render_entry_row(
    frame: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    entry: &MountEntry,
    is_selected: bool,
) {
    let row_bg     = if is_selected { SELECTED_BG } else { OVERLAY_BG };
    let row_style  = Style::default().bg(row_bg).fg(NORMAL_TEXT_FG);

    let (badge_text, badge_color) = match &entry.status {
        MountEntryStatus::Mounting             => ("mounting", Color::Yellow),
        MountEntryStatus::Mounted              => ("mounted ", Color::Green),
        MountEntryStatus::Failed { .. }        => ("failed  ", Color::Red),
        MountEntryStatus::UnmountFailed { .. } => ("unmnt!  ", Color::Rgb(255, 100, 0)),
    };

    let fs_text = match &entry.fs_type {
        LoadingField::Loading   => "?".to_string(),
        LoadingField::Loaded(t) => t.clone(),
    };

    let selector = if is_selected { "▶" } else { " " };
    let mountpoint_text  = format!("{: >45}",entry.mountpoint.display());
    let device_node_text = entry.real_device_path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "?".to_string());
    let by_id_text = entry.by_id_name.clone();

    let badge_style = Style::default()
        .fg(badge_color)
        .bg(row_bg)
        .add_modifier(Modifier::BOLD);

    let line = Line::from(vec![
        Span::styled(format!("{} ", selector), row_style),
        Span::styled("[",          row_style),
        Span::styled(badge_text,   badge_style),
        Span::styled("]  ",        row_style),
        Span::styled(mountpoint_text,      row_style),
        Span::styled("  ",         row_style),
        Span::styled(format!("{: >11}", fs_text),           row_style),
        Span::styled("         ",                                 row_style),
        Span::styled(format!("{:<10}", device_node_text),   row_style),
        Span::styled("  ",                                   row_style),
        Span::styled(by_id_text,                             row_style),
    ]);

    let row_area = Rect { x, y, width, height: 1 };
    {
        let buf = frame.buffer_mut();
        for bx in row_area.left()..row_area.right() {
            buf[(bx, y)].set_style(row_style);
        }
    }
    frame.render_widget(Paragraph::new(line).style(row_style), row_area);
}

fn build_hint_line(selected_entry: Option<&MountEntry>) -> Line<'static> {
    let ks = Style::default().fg(KEY_FG).bg(OVERLAY_BG).add_modifier(Modifier::BOLD);
    let hs = Style::default().fg(HINT_FG).bg(OVERLAY_BG);
    let es = Style::default().fg(Color::Red).bg(OVERLAY_BG).add_modifier(Modifier::BOLD);

    match selected_entry {
        Some(MountEntry { status: MountEntryStatus::Failed { reason }, .. }) => {
            Line::from(vec![
                Span::styled("  ", hs),
                Span::styled("[f/Esc]", ks),
                Span::styled(" Close", hs),
                Span::styled("  ", hs),
                Span::styled(format!("Mount error: {}", reason), es),
            ])
        }
        Some(MountEntry { status: MountEntryStatus::UnmountFailed { reason }, .. }) => {
            Line::from(vec![
                Span::styled("  ", hs),
                Span::styled("[f/Esc]", ks),
                Span::styled(" Close ", hs),
                Span::styled("[u]", ks),
                Span::styled(" Retry ", hs),
                Span::styled("  ", hs),
                Span::styled(format!("Unmount error: {}", reason), es),
            ])
        }
        _ => {
            Line::from(vec![
                Span::styled("  ", hs),
                Span::styled("[f/Esc]", ks),
                Span::styled(" Close", hs),
                Span::styled("[↑↓]",    ks),
                Span::styled(" Navigate ", hs),
                Span::styled("[u]",    ks),
                Span::styled(" Unmount ", hs),
            ])
        }
    }
}
