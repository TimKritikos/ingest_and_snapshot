use ratatui::Frame;
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::style::{Color, Style, Modifier};
use ratatui::widgets::Paragraph;
use ratatui::prelude::Stylize;
use ratatui::text::{Span, Line};
use sysinfo::System;
use time_format::now;

const ZFS_VERSION_FILE: &str = "/sys/module/zfs/version";

pub fn render(frame: &mut Frame, area: Rect, sys: &System, #[cfg(feature = "fps-counter")] fps: f64) {
    let current_time = now().unwrap();
    let timestamp = time_format::strftime_utc("%a, %d %b %Y %T %Z", current_time).unwrap();

    let zfs_version = match std::fs::read_to_string(ZFS_VERSION_FILE) {
        Ok(version) => version,
        Err(_e) => "unavailable".to_string(),
    };

    let key_style = Style::default().fg(Color::White).bg(Color::Black);
    let value_style = Style::default().fg(Color::Cyan).bg(Color::Black);

    let right_status = Line::from(
        vec![
            #[cfg(feature = "fps-counter")]
            Span::styled("FPS:", key_style),
            #[cfg(feature = "fps-counter")]
            Span::styled(format!("{:.1}   ", fps), value_style),
            Span::styled("RAM:", key_style),
            Span::styled(format!("{:.1}/{:.1} GiB",(sys.used_memory() as f64) / (1024.0 * 1024.0 * 1024.0), (sys.total_memory() as f64) / (1024.0 * 1024.0 * 1024.0)), value_style),
            Span::styled("   NAME:", key_style),
            Span::styled(System::host_name().unwrap(), value_style),
            Span::styled("   ZFS:", key_style),
            Span::styled(zfs_version, value_style),
            Span::styled("   ", key_style),
        ]
    ).right_aligned();

    // More layout setting
    let status_items = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Fill(1),
            Constraint::Length(right_status.width().try_into().unwrap()),
        ])
        .split(area);

    // Status bars
    frame.render_widget(Paragraph::new(right_status).bg(Color::Black).add_modifier(Modifier::BOLD), status_items[1]);
    frame.render_widget(Paragraph::new(format!(" {}", timestamp)).bg(Color::Black).fg(Color::White).add_modifier(Modifier::BOLD), status_items[0]);
}
