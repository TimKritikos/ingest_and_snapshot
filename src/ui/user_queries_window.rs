use std::collections::VecDeque;
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Layout, Direction, Constraint};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets;
use super::{UserQuery, ApproveTransferQuery, ScanNewDeviceQuery, ApproveTransferResponse};

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
