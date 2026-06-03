use std::collections::VecDeque;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Layout, Direction, Constraint};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::buffer::Buffer;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets;
use crate::ui_api::{UserQuery, ApproveTransferQuery, ScanNewDeviceQuery, ApproveTransferResponse, FatalErrorQuery, FatalErrorKind, SourceMediaWarningsQuery};

pub struct QueryWindowState {
    pub device_picker_open: bool,
    pub device_picker_selection: usize,
    pub device_override: Option<String>,
}

impl QueryWindowState {
    pub fn new() -> Self {
        Self { device_picker_open: false, device_picker_selection: 0, device_override: None }
    }
}

pub fn handle_key(state: &mut QueryWindowState, key: KeyEvent, query_queue: &mut VecDeque<UserQuery>, available_devices: Option<&[String]>) {
    match query_queue.front() {
        Some(UserQuery::ApproveTransfer(query)) => {
            if state.device_picker_open {
                let device_count = available_devices.map(|d| d.len() + 1).unwrap_or(1);
                match key.code {
                    KeyCode::Up   => { state.device_picker_selection = state.device_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.device_picker_selection = (state.device_picker_selection + 1).min(device_count.saturating_sub(1)); }
                    KeyCode::Enter => {
                        if state.device_picker_selection == 0 {
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceOverwrite(None));
                            state.device_override = None;
                        } else {
                            let picked = available_devices.and_then(|d| d.get(state.device_picker_selection - 1)).cloned();
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceOverwrite(picked.clone()));
                            state.device_override = picked;
                        }
                        state.device_picker_open = false;
                    }
                    KeyCode::Esc => { state.device_picker_open = false; }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('o') | KeyCode::Char('O') => {
                        state.device_picker_open = true;
                        state.device_picker_selection = 0;
                    }
                    KeyCode::Enter => {
                        if let Some(UserQuery::ApproveTransfer(query)) = query_queue.pop_front() {
                            let _ = query.response_tx.send(ApproveTransferResponse::Approved);
                        }
                    }
                    KeyCode::Esc => {
                        if let Some(UserQuery::ApproveTransfer(query)) = query_queue.pop_front() {
                            let _ = query.response_tx.send(ApproveTransferResponse::Denied);
                        }
                    }
                    _ => {}
                }
            }
        }
        Some(UserQuery::ScanNewDevice(_)) => {
            match key.code {
                KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(UserQuery::ScanNewDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(true);
                    }
                }
                KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                    if let Some(UserQuery::ScanNewDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(false);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::FatalError(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if let Some(UserQuery::FatalError(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(());
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::SourceMediaWarnings(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if let Some(UserQuery::SourceMediaWarnings(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(());
                    }
                }
                _ => {}
            }
        }
        None => {}
    }
}


pub fn render(
    frame: &mut Frame,
    area: Rect,
    query: &UserQuery,
    queued_count: usize,
    state: &QueryWindowState,
    available_devices: Option<&[String]>,
) {
    let title = if queued_count > 0 {
        format!("User query  [{} more queued]", queued_count)
    } else {
        "User query".to_string()
    };

    let window = tui_dialog_widgets::DialogBlock::default().title(&title);
    frame.render_widget(window.clone(), area);
    let inner = window.inner(area);

    // 1-cell padding inside the dialog border on all sides
    let padded = Rect {
        x:      inner.x.saturating_add(1),
        y:      inner.y.saturating_add(1),
        width:  inner.width.saturating_sub(2),
        height: inner.height.saturating_sub(2),
    };
    if padded.width == 0 || padded.height == 0 { return; }

    match query {
        UserQuery::ApproveTransfer(query) => {
            render_approve_transfer(frame, padded, query, state.device_override.as_deref());
            if state.device_picker_open {
                render_device_picker(frame, available_devices, state.device_override.as_deref(), state.device_picker_selection);
            }
        }
        UserQuery::ScanNewDevice(query) => render_scan_new_device(frame, padded, query),
        UserQuery::FatalError(query) => render_fatal_error(frame, padded, query),
        UserQuery::SourceMediaWarnings(query) => render_source_media_warnings(frame, padded, query),
    }
}

fn render_approve_transfer(frame: &mut Frame, area: Rect, query: &ApproveTransferQuery, device_override: Option<&str>) {
    let icon_cols = area.height * super::FONT_CELL_ASPECT_RATIO;
    let least_characters_for_text = 20 ; //TODO: I made this number up

    // Split horizontally first so the icon takes the full height.
    // When not wide enough, skip the icon and use the full area for content.
    let content_area = if area.width > icon_cols + least_characters_for_text {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Length(icon_cols),
                Constraint::Length(2),
                Constraint::Min(0),
            ])
            .split(area);
        render_icon_placeholder(frame, cols[0]);
        cols[2]
    } else {
        area
    };

    let info_height = content_area.height.saturating_sub(4).max(1);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(info_height), // All info
            Constraint::Length(1),           // gap
            Constraint::Length(1),           // device overwrite query
            Constraint::Length(1),           // gap
            Constraint::Length(1),           // buttons
        ])
        .split(content_area);

    render_transfer_info(frame, rows[0], query);

    let label_style    = Style::default().fg(Color::Black);
    let key_style      = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let selected_style = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let none_style     = Style::default().fg(Color::DarkGray);

    let (override_span, override_style) = match device_override {
        Some(name) => (name, selected_style),
        None       => ("none selected", none_style),
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[O]", key_style),
            Span::styled(" Override device:  ", label_style),
            Span::styled(override_span, override_style),
        ])),
        rows[2],
    );

    let hint = Style::default().fg(Color::Black);
    let ok   = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let deny = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", ok),
            Span::styled(" Approve   ", hint),
            Span::styled("[Esc]", deny),
            Span::styled(" Deny", hint),
        ])),
        rows[4],
    );
}

fn render_device_picker(frame: &mut Frame, available_devices: Option<&[String]>, device_override: Option<&str>, selection: usize) {
    let hint = {
        let hint = Style::default().fg(Color::Black);
        let ok   = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
        let esc  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
        Line::from(vec![
            Span::styled("[Enter]", ok),
            Span::styled(" Select  ", hint),
            Span::styled("[Esc]", esc),
            Span::styled(" Cancel", hint),
        ])
    };

    let mut items = vec![
        tui_dialog_widgets::DialogFloatingListItem { label: "Auto-detected", is_current: device_override.is_none() },
    ];
    match available_devices {
        None => items.push(tui_dialog_widgets::DialogFloatingListItem {
            label: "Loading available devices...",
            is_current: false,
        }),
        Some(devices) => items.extend(devices.iter().map(|d| tui_dialog_widgets::DialogFloatingListItem {
            label: d.as_str(),
            is_current: device_override == Some(d.as_str()),
        })),
    }

    frame.render_widget(
        tui_dialog_widgets::DialogFloatingList::new("Select device")
            .items(items)
            .selected(selection)
            .hint(hint),
        frame.area(),
    );
}

fn render_icon_placeholder(frame: &mut Frame, area: Rect) {
    let buf = frame.buffer_mut();
    let bg   = Style::default().bg(Color::DarkGray).fg(Color::Gray);
    let body = Style::default().bg(Color::DarkGray).fg(Color::White);

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_style(bg);
        }
    }

    let icon: &[&str] = &[
        "   ╭──╮      ",
        "╭──╯  ╰─────╮",
        "│   ╭─────╮ │",
        "│   │  ◎  │ │",
        "│   ╰─────╯ │",
        "╰───────────╯",
    ];

    let icon_h = icon.len() as u16;
    let icon_w = icon.iter().map(|l| l.chars().count() as u16).max().unwrap_or(0);

    if area.width < icon_w || area.height < icon_h { return; }

    let x0 = area.x + (area.width.saturating_sub(icon_w)) / 2;
    let y0 = area.y + (area.height.saturating_sub(icon_h)) / 2;

    for (i, &line) in icon.iter().enumerate() {
        buf.set_string(x0, y0 + i as u16, line, body);
    }
}

fn render_transfer_info(frame: &mut Frame, area: Rect, query: &ApproveTransferQuery) {
    let label       = Style::default().fg(Color::Black);
    let value       = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let overwritten = Style::default().fg(Color::DarkGray);
    let data = &query.data;
    let size_str = super::format_bytes(data.data_size);

    let mut product_spans = vec![Span::styled("Product:  ", label), Span::styled(data.device_product_name.as_str(), value)];
    if data.device_overridden {
        product_spans.push(Span::styled(" (overwritten)", overwritten));
    }

    let mut lines: Vec<Line> = vec![
        Line::from(product_spans),
        Line::from(vec![Span::styled("Brand:    ", label), Span::styled(data.brand.as_str(), value)]),
        Line::from(vec![Span::styled("Serial:   ", label), Span::styled(data.serial_number.as_str(), value)]),
        Line::from(vec![Span::styled("Function: ", label), Span::styled(data.transfer_function.as_str(), value)]),
        Line::from(vec![Span::styled("Archive:  ", label), Span::styled(data.archive_directory.as_str(), value)]),
        Line::from(vec![
            Span::styled("Size:     ", label),
            Span::styled(size_str, value),
            Span::styled("   Card ID: ", label),
            Span::styled(data.card_id.as_str(), value),
        ]),
    ];

    // Derive column widths from the already-built spans so the strings only appear once
    let line_w = |l: &Line| -> usize { l.spans.iter().map(|s| s.content.len()).sum() };
    let max_w  = lines.iter().map(line_w).max().unwrap_or(0);
    let col0_w = line_w(&lines[0]);

    const COL_GAP: usize = 3;
    lines[0].spans.push(Span::raw(" ".repeat(max_w - col0_w + COL_GAP)));
    lines[0].spans.push(Span::styled("Source device:  ", label));
    lines[0].spans.push(Span::styled(data.source_device.as_str(), value));

    frame.render_widget(Paragraph::new(lines), area);
}

// Renders a braille-dot icon into `area`. The background is filled with `bg_style`, and lit dots
// are drawn with `icon_style`. `is_active` receives normalised coordinates (−1.0 … +1.0, where
// −1.0/+1.0 are the left/top and right/bottom edges respectively) and returns true if that dot
// should be lit.
fn render_braille_icon(frame: &mut Frame, area: Rect, bg_style: Style, icon_style: Style, is_active: impl Fn(f32, f32) -> bool) {
    if area.width == 0 || area.height == 0 { return; }

    let dot_matrix_width  = area.width  as usize * 2;
    let dot_matrix_height = area.height as usize * 4;

    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_style(bg_style);
        }
    }

    // Each braille cell covers 2 dot-columns × 4 dot-rows.
    for character_y in 0..area.height {
        for character_x in 0..area.width {
            let dot_x = character_x as usize * 2;
            let dot_y = character_y as usize * 4;
            let mut braille_bits: u8 = 0;
            for dot_row in 0..4usize {
                let nx_left  = (dot_x       as f32 / (dot_matrix_width  as f32 - 1.0)) * 2.0 - 1.0;
                let nx_right = ((dot_x + 1) as f32 / (dot_matrix_width  as f32 - 1.0)) * 2.0 - 1.0;
                let ny       = ((dot_y + dot_row) as f32 / (dot_matrix_height as f32 - 1.0)) * 2.0 - 1.0;
                if is_active(nx_left,  ny) { braille_bits |= super::BRAILLE_BAR_LEFT[dot_row]; }
                if is_active(nx_right, ny) { braille_bits |= super::BRAILLE_BAR_RIGHT[dot_row]; }
            }
            if braille_bits != 0 {
                let ch = char::from_u32(0x2800 + braille_bits as u32).unwrap_or(' ');
                buf[(area.x + character_x, area.y + character_y)].set_char(ch).set_style(icon_style);
            }
        }
    }
}

// Draws a braille icon on the left side of `area` and returns the content area to the right.
// If the area is too narrow to fit both icon and text, skips the icon and returns the full area.
fn layout_with_icon(
    frame: &mut Frame,
    area: Rect,
    bg_style: Style,
    icon_style: Style,
    is_active: impl Fn(f32, f32) -> bool,
) -> Rect {
    let icon_cols: u16    = area.height * super::FONT_CELL_ASPECT_RATIO;
    let min_text_width: u16 = 20;
    if area.width > icon_cols + min_text_width {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Length(icon_cols),
                Constraint::Length(3),
                Constraint::Min(0),
            ])
            .split(area);
        render_braille_icon(frame, cols[0], bg_style, icon_style, is_active);
        cols[2]
    } else {
        area
    }
}

fn render_fatal_error(frame: &mut Frame, area: Rect, query: &FatalErrorQuery) {
    let bg_style     = Style::default().bg(Color::Gray);
    let icon_style   = Style::default().bg(Color::Gray).fg(Color::Black).add_modifier(Modifier::BOLD);
    let error_style  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let label_style  = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let detail_style = Style::default().fg(Color::Black);
    let ok_style     = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let hint_style   = Style::default().fg(Color::Black);

    let content_area = layout_with_icon(frame, area, bg_style, icon_style, |nx, ny| {
        let radius           = 0.85_f32;
        let circle_thickness = 0.13_f32;
        let line_thickness   = 0.11_f32;

        let dist             = (nx * nx + ny * ny).sqrt();
        let on_circle        = (dist - radius).abs() < circle_thickness;
        let on_diagonal      = (ny + nx).abs() < line_thickness && dist < radius;

        on_circle || on_diagonal
    });

    let (context, detail) = match &query.error {
        FatalErrorKind::DevicesJson(msg) => ("Failed to load data from devices.json", msg.as_str()),
        FatalErrorKind::SourceMedia(msg) => ("Failed to load source media configurations", msg.as_str()),
        FatalErrorKind::BackupLog(msg)   => ("Failed to load backup log", msg.as_str()),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1), // "Fatal error"
            Constraint::Length(1), // blank
            Constraint::Min(1),    // source + detail
            Constraint::Length(1), // blank
            Constraint::Length(1), // "[Enter] OK"
        ])
        .split(content_area);

    frame.render_widget(Paragraph::new(Span::styled("Fatal error", error_style)), rows[0]);
    frame.render_widget(
        Paragraph::new(vec![
            Line::from(vec![Span::styled("Source: ", label_style), Span::styled(context, detail_style)]),
            Line::from(vec![Span::styled("Detail: ", label_style), Span::styled(detail, detail_style)]),
        ]).wrap(Wrap { trim: false }),
        rows[2],
    );
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", ok_style),
            Span::styled(" OK", hint_style),
        ])),
        rows[4],
    );
}

// Renders `lines` into a temporary off-screen buffer and counts how many rows are actually used.
// This uses ratatui's exact word-wrap algorithm, so the result is always accurate.
fn count_wrapped_lines(lines: &[Line], width: u16) -> usize {
    if width == 0 { return 0; }
    // Worst-case height: every character on its own row (width == 1 scenario)
    let max_possible_height: usize = lines.iter()
        .map(|l| l.spans.iter().map(|s| s.content.chars().count()).sum::<usize>().max(1))
        .sum();
    let tmp_area = Rect {
        x: 0, y: 0,
        width,
        height: max_possible_height.min(u16::MAX as usize) as u16,
    };
    let mut tmp_buf = Buffer::empty(tmp_area);
    Paragraph::new(lines.to_vec()).wrap(Wrap { trim: false }).render(tmp_area, &mut tmp_buf);
    (0..tmp_area.height)
        .rev()
        .find(|&row| (0..tmp_area.width).any(|col| tmp_buf[(col, row)].symbol() != " "))
        .map(|last_row| last_row as usize + 1)
        .unwrap_or(0)
}

fn render_warnings_box(frame: &mut Frame, area: Rect, warnings: &[String]) {
    let box_bg_style = Style::default().bg(Color::DarkGray);
    let text_style   = Style::default().bg(Color::DarkGray).fg(Color::White);
    let more_style   = Style::default().bg(Color::Black).fg(Color::Blue).add_modifier(Modifier::ITALIC);

    let buf = frame.buffer_mut();
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_style(box_bg_style);
        }
    }

    if area.height == 0 { return; }

    // 1-cell horizontal padding inside the box
    let inner_width = area.width.saturating_sub(2);
    if inner_width == 0 { return; }
    let inner = Rect { x: area.x + 1, width: inner_width, ..area };

    let warning_lines: Vec<Line> = warnings.iter()
        .map(|w| Line::from(Span::styled(w.as_str(), text_style)))
        .collect();

    let total_display_lines = count_wrapped_lines(&warning_lines, inner_width);

    let available_rows = area.height as usize;
    let has_overflow   = total_display_lines > available_rows;

    let content_area = if has_overflow {
        Rect { height: area.height.saturating_sub(1), ..inner }
    } else {
        inner
    };

    frame.render_widget(
        Paragraph::new(warning_lines).wrap(Wrap { trim: false }),
        content_area,
    );

    if has_overflow {
        let hidden_lines = total_display_lines - (available_rows - 1);
        let more_text = format!("{} more line{}", hidden_lines, if hidden_lines == 1 { "" } else { "s" });
        let indicator_area = Rect { y: area.y + area.height - 1, height: 1, ..area };
        let buf = frame.buffer_mut();
        for x in indicator_area.left()..indicator_area.right() {
            buf[(x, indicator_area.y)].set_char(' ').set_style(more_style);
        }
        frame.render_widget(
            Paragraph::new(Span::styled(more_text, more_style)).alignment(Alignment::Center),
            indicator_area,
        );
    }
}

fn render_source_media_warnings(frame: &mut Frame, area: Rect, query: &SourceMediaWarningsQuery) {
    let bg_style    = Style::default().bg(Color::Gray);
    let icon_style  = Style::default().bg(Color::Gray).fg(Color::Black).add_modifier(Modifier::BOLD);
    let title_style = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let text_style  = Style::default().fg(Color::Black);
    let ok_style    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let hint_style  = Style::default().fg(Color::Black);

    let content_area = layout_with_icon(frame, area, bg_style, icon_style, |x, y| {
        let top_y      = -1.0_f32; // Highest Y of icon
        let bottom_y   =  1.0_f32; // Lowest Y of icon
        let half_width =  1.0_f32; // Half of the width of the icon

        let icon_y_percentage = (y - top_y) / (bottom_y - top_y);
        if icon_y_percentage < 0.0 || icon_y_percentage > 1.0 {
            return false;
        }

        //where the theoretical line connecting the two points is
        let edge_x        = icon_y_percentage * half_width;

        //Triangle
        let thickness     = 0.12_f32;

        //plotted lines can be though of as sepperated in the middle by the theoretical line
        //connecting the two points and having the inside part which would be half the line's thickness
        //and the outer part. This is true when we are on the inner part
        let inside        = x.abs() <= edge_x;

        let on_left_edge  = (x + edge_x).abs() < thickness;
        let on_right_edge = (x - edge_x).abs() < thickness;
        let on_bottom     = (y - bottom_y).abs() < thickness && x.abs() <= half_width;

        let triangle_outline = inside && (on_left_edge || on_right_edge || on_bottom);


        //Exclamation point
        let stem = x.abs() < 0.10 && y > -0.30 && y < 0.35;
        let dot  = x.abs() < 0.13 && (y - 0.60).abs() < 0.11;

        triangle_outline || stem || dot
    });

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1), // title
            Constraint::Length(1), // blank
            Constraint::Min(1),    // warnings box
            Constraint::Length(1), // blank
            Constraint::Length(1), // "[Enter] OK"
        ])
        .split(content_area);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Warning:", title_style),
            Span::styled(
                format!(" There was a problem reading {} source media {}",
                    query.warnings.len(),
                    if query.warnings.len() == 1 { "file" } else { "files" }),
                text_style,
            ),
        ])),
        rows[0],
    );

    render_warnings_box(frame, rows[2], &query.warnings);

    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", ok_style),
            Span::styled(" OK", hint_style),
        ])),
        rows[4],
    );
}

fn render_scan_new_device(frame: &mut Frame, area: Rect, query: &ScanNewDeviceQuery) {
    let label = Style::default().fg(Color::Black);
    let value = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let hint  = Style::default().fg(Color::Black);
    let ok    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let deny  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![
            Span::styled("Unknown device: ", label),
            Span::styled(query.device_name.as_str(), value),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("Scan this device?", hint)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Y]", ok),
            Span::styled(" Yes   ", hint),
            Span::styled("[N]", deny),
            Span::styled(" No", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centered);
}
