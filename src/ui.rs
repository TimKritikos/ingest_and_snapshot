use std::thread::JoinHandle;
use std::thread;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Layout, Direction, Constraint, Rect};
use ratatui::style::{Color, Style, Modifier};
use ratatui::widgets::{Paragraph, Block};
use ratatui::prelude::Stylize;
use ratatui::text::{Span,Line};
use sysinfo::System;
use time_format::now;
use crossterm::event;
use std::time::Duration;
use crossterm::event::Event;
use crossterm::event::KeyCode;

mod tui_dialog_widgets;

const ZFS_VERSION_FILE: &str = "/sys/module/zfs/version";

#[derive(Debug)]
pub enum LogicToUiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    Quit,
}
pub enum UiToLogicMessage {
    Quit,
}

pub fn init(rx: Receiver<LogicToUiMessage>, tx: Sender<UiToLogicMessage>) -> JoinHandle<()> {
    color_eyre::install().unwrap();
    thread::spawn(|| {
        ratatui::run(|terminal| { app(terminal,rx,tx)}).unwrap();
    })
}

#[derive(PartialEq)]
enum SelectedAction {
    Quit,
    Snapshot,
}

struct UiState {
    selecte_action:SelectedAction,
}

fn app(terminal: &mut DefaultTerminal,rx: Receiver<LogicToUiMessage>,tx: Sender<UiToLogicMessage>) -> std::io::Result<()> {
    let mut l_allow:Vec<String> = [].to_vec();
    let mut l_ignore:Vec<String> = [].to_vec();
    let mut selected_action = SelectedAction::Quit;
    loop {
        terminal.draw( |frame| { render(frame, &l_allow, &l_ignore, &selected_action) })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LogicToUiMessage::AddConfig {allow, ignore} => {
                    l_allow = allow;
                    l_ignore = ignore;
                }
                LogicToUiMessage::Quit => return Ok(()),
            }
        }

        if event::poll(Duration::from_millis(200))? {
            if let Event::Key(key) = event::read()? {
                match key.code {

                    KeyCode::Up => {
                        selected_action = match selected_action {
                            SelectedAction::Snapshot => SelectedAction::Quit,
                            SelectedAction::Quit => SelectedAction::Quit,
                        }
                    }

                    KeyCode::Down => {
                        selected_action = match selected_action {
                            SelectedAction::Snapshot => SelectedAction::Snapshot,
                            SelectedAction::Quit => SelectedAction::Snapshot,
                        }
                    }
                    KeyCode::Enter => {
                        match selected_action {
                            SelectedAction::Snapshot => {},
                            SelectedAction::Quit => {
                                tx.send(UiToLogicMessage::Quit).unwrap();
                            },
                        }
                    }
                    _ => {}
                }
            }
        }
    }
}

fn render(frame: &mut Frame, allow:&Vec<String>, ignore:&Vec<String>, selected_action:&SelectedAction) {
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

    let windows = Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(4)
        .vertical_margin(2)
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
        .split(layout[0]);

    // Status bars
    frame.render_widget(Paragraph::new(right_status).bg(Color::Black).add_modifier(Modifier::BOLD), status_items[1]);
    frame.render_widget(Paragraph::new(format!(" {}",timestamp)).bg(Color::Black).fg(Color::White).add_modifier(Modifier::BOLD), status_items[0]);

    // Windows
    let transfer_window = tui_dialog_widgets::DialogBlock::default()
        .title("Transfers");
    frame.render_widget(transfer_window.clone(), windows[0]);

    let user_queries_window = tui_dialog_widgets::DialogBlock::default()
        .title("User queries");
    frame.render_widget(user_queries_window.clone(), windows[2]);
    frame.render_widget(format!("> hello from ingest and snapshot. Allow: {:?} Ignore: {:?}",allow,ignore), user_queries_window.inner(windows[2]));

    let actions_window = tui_dialog_widgets::DialogBlock::default()
        .title("Actions");
    frame.render_widget(actions_window.clone(), windows[4]);

    let list = tui_dialog_widgets::DialogSelectionList::new(vec![
        "Exit",
        "Finish backup and do snapshot",
    ])
        .title("Options")
        .selected(Some(match selected_action {
                SelectedAction::Quit => 0,
                SelectedAction::Snapshot => 1,
            }
        ))
        .focused(true);

    let actions_window_content = actions_window.inner(windows[4]);

    let list_area = Rect {
        x: actions_window_content.x + actions_window_content.width/2 - 25 ,
        y: actions_window_content.y + actions_window_content.height/2 - 3 ,
        width: 50,
        height: 6,
    };

    frame.render_widget(list, list_area);

}
