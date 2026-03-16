use ratatui::{DefaultTerminal, Frame};
use std::thread::JoinHandle;
use std::thread;
use std::sync::mpsc::Receiver;
use ratatui::layout::Layout;
use ratatui::layout::Direction;
use ratatui::layout::Constraint;
use ratatui::style::Color;
use ratatui::widgets::Paragraph;
use ratatui::prelude::Stylize;
use ratatui::widgets::Block;
use ratatui::style::Style;
use time_format::now;
use ratatui::style::Modifier;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use sysinfo::System;
use ratatui::text::Span;
use ratatui::text::Line;

const ZFS_VERSION_FILE: &str = "/sys/module/zfs/version";

#[derive(Debug)]
pub enum UiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    Quit,
}

pub fn init(rx: Receiver<UiMessage>) -> JoinHandle<()> {
    color_eyre::install().unwrap();
    thread::spawn(|| {
        ratatui::run(|terminal| { app(terminal,rx)}).unwrap();
    })
}

fn app(terminal: &mut DefaultTerminal,rx: Receiver<UiMessage>) -> std::io::Result<()> {
    let mut l_allow:Vec<String> = [].to_vec();
    let mut l_ignore:Vec<String> = [].to_vec();
    loop {
        terminal.draw(|frame|{ render(frame, &l_allow, &l_ignore) })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                UiMessage::AddConfig { allow, ignore } => {
                    l_allow=allow;
                    l_ignore=ignore;
                }
                UiMessage::Quit => return Ok(()),
            }
        }
    }
}

fn render(frame: &mut Frame, allow:&Vec<String>, ignore:&Vec<String>) {
    let bg = Block::default().style(Style::default().bg(Color::Blue));
    frame.render_widget(bg, frame.area());

    // Setting layout
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(vec![
            Constraint::Length(1),
            Constraint::Percentage(100)])
        .split(frame.area());

    let windows  = Layout::default()
        .direction(Direction::Vertical)
        .margin(4)
        .constraints(vec![
            Constraint::Percentage(20),
            Constraint::Length(3),
            Constraint::Percentage(40),
            Constraint::Length(3),
            Constraint::Percentage(40),
        ])
        .split(layout[1]);

    // Get right status data
    let current_time = now().unwrap();
    let timestamp = time_format::strftime_utc("%a, %d %b %Y %T %Z", current_time).unwrap();
    let mut sys = System::new_all();
    sys.refresh_all();

    let zfs_version = match std::fs::read_to_string(ZFS_VERSION_FILE) {
        Ok(version) => version,
        Err(_e) => "unavailable".to_string(),
    };

    let key_style = Style::default().fg(Color::White).bg(Color::Black);
    let value_style = Style::default().fg(Color::Cyan).bg(Color::Black);

    let right_status = Line::from(
                vec![
                    Span::styled("RAM:", key_style),
                    Span::styled(format!("{:.1}/{:.1} GiB",(sys.used_memory() as f64 )/(1024.0*1024.0*1024.0), (sys.total_memory() as f64)/(1024.0*1024.0*1024.0)), value_style),
                    Span::styled("   NAME:", key_style),
                    Span::styled(System::host_name().unwrap(),value_style),
                    Span::styled("   ZFS:", key_style),
                    Span::styled(zfs_version,value_style),
                    Span::styled("   ", key_style),
                ]
           ).right_aligned();

    // More layout setting
    let status_items= Layout::default()
        .direction(Direction::Horizontal)
        .constraints(vec![
            Constraint::Fill(1),
            Constraint::Length(right_status.width().try_into().unwrap()),
        ])
        .split(layout[0]);

    // Status bars
    frame.render_widget( Paragraph::new( right_status ).bg(Color::Black).add_modifier(Modifier::BOLD), status_items[1] );
    frame.render_widget(
       Paragraph::new( format!(" {}",timestamp)).bg(Color::Black).fg(Color::White).add_modifier(Modifier::BOLD), status_items[0]
    );

    // Windows
    let transfer_window = DialogBlock::default()
        .title("Transfers");
    frame.render_widget( transfer_window.clone(), windows[0]);

    let user_queries_window = DialogBlock::default()
        .title("User queries");
    frame.render_widget( user_queries_window.clone(), windows[2]);
    frame.render_widget( format!("> hello from ingest and snapshot. Allow: {:?} Ignore: {:?}",allow,ignore), user_queries_window.inner(windows[2]));

    let actions_window = DialogBlock::default()
        .title("Actions");
    frame.render_widget(actions_window, windows[4]);

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
