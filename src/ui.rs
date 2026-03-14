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
use ratatui::widgets::Borders;
use ratatui::style::Style;
use time_format::now;
use ratatui::style::Modifier;

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
    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(vec![
            Constraint::Length(1),
            Constraint::Percentage(100)])
        .split(frame.area());

    let windows  = Layout::default()
        .direction(Direction::Vertical)
        .margin(1)
        .constraints(vec![
            Constraint::Percentage(20),
            Constraint::Percentage(40),
            Constraint::Percentage(40),
        ])
        .split(layout[1]);

    let current_time = now().unwrap();
    let timestamp = time_format::strftime_utc("%a, %d %b %Y %T %Z", current_time).unwrap();

    frame.render_widget(
       Paragraph::new( format!(" {}",timestamp)).bg(Color::Black).fg(Color::White).add_modifier(Modifier::BOLD), layout[0]
    );
    let transfer_block = Block::default()
        .title("Transfers")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    frame.render_widget( transfer_block, windows[0]);

    let user_queries_block = Block::default()
        .title("User queries")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    frame.render_widget( user_queries_block.clone(), windows[1]);
    frame.render_widget( format!("> hello from ingest and snapshot. Allow: {:?} Ignore: {:?}",allow,ignore), user_queries_block.inner(windows[1]));

    let actions_block = Block::default()
        .title("Actions")
        .borders(Borders::ALL)
        .style(Style::default().bg(Color::Black));
    frame.render_widget(actions_block, windows[2]);

}
