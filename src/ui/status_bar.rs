use ratatui::Frame;
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::style::{Color, Style, Modifier};
use ratatui::widgets::Paragraph;
use ratatui::prelude::Stylize;
use ratatui::text::{Span, Line};
use time_format::now;
use crate::ui_api::SystemInfo;

const MOUNT_WIDGET_BG: Color = Color::Rgb(220, 110, 0);
const MOUNT_WIDGET_FG: Color = Color::Black;

pub fn render(frame: &mut Frame, area: Rect, system_info: Option<&SystemInfo>, mount_count: usize, #[cfg(feature = "fps-counter")] fps: f64) {
    let current_time = now().unwrap();
    let timestamp = time_format::strftime_utc("%a, %d %b %Y %T %Z", current_time).unwrap();

    let key_style = Style::default().fg(Color::White).bg(Color::Black);
    let value_style = Style::default().fg(Color::Cyan).bg(Color::Black);

    let right_status = Line::from(
        vec![
            #[cfg(feature = "fps-counter")]
            Span::styled("FPS:", key_style),
            #[cfg(feature = "fps-counter")]
            Span::styled(format!("{:.1}   ", fps), value_style),
            Span::styled("S/W VERSION:", key_style),
            Span::styled(format!("{}   ", env!("CARGO_PKG_VERSION")), value_style),
            Span::styled("RAM:", key_style),
            Span::styled(
                system_info.map(|i| format!("{:.1}/{:.1} GiB",
                    i.ram_used_bytes  as f64 / (1024.0 * 1024.0 * 1024.0),
                    i.ram_total_bytes as f64 / (1024.0 * 1024.0 * 1024.0),
                )).unwrap_or_else(|| "-.--/-.-- GiB".to_owned()),
                value_style,
            ),
            Span::styled("   NAME:", key_style),
            Span::styled(
                system_info.map(|i| i.hostname.clone()).unwrap_or_else(|| "...".to_owned()),
                value_style,
            ),
            Span::styled("   ZFS:", key_style),
            Span::styled(
                system_info.map(|i| i.zfs_version.clone()).unwrap_or_else(|| "...".to_owned()),
                value_style,
            ),
            Span::styled("   ", key_style),
        ]
    ).right_aligned();

    let mount_text = format!(" {} Filesystem{} [F] ", mount_count, if mount_count==1 { "" } else { "s" } );
    let mount_widget_width: u16 = mount_text.len().try_into().unwrap_or(30);

    let status_items = Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Fill(1),
            Constraint::Length(mount_widget_width),
            Constraint::Length(right_status.width().try_into().unwrap()),
        ])
        .split(area);

    frame.render_widget(
        Paragraph::new(format!(" {}", timestamp))
            .bg(Color::Black)
            .fg(Color::White)
            .add_modifier(Modifier::BOLD),
        status_items[0],
    );
    frame.render_widget(
        Paragraph::new(mount_text)
            .bg(MOUNT_WIDGET_BG)
            .fg(MOUNT_WIDGET_FG)
            .add_modifier(Modifier::BOLD),
        status_items[1],
    );
    frame.render_widget(
        Paragraph::new(right_status).bg(Color::Black).add_modifier(Modifier::BOLD),
        status_items[2],
    );
}
