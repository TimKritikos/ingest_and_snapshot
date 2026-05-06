use std::thread::JoinHandle;
use std::thread;
use std::sync::mpsc::Receiver;
use std::sync::mpsc::Sender;
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Layout, Direction, Constraint};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use crossterm::event;
use std::time::Duration;
use crossterm::event::Event;
use crossterm::event::KeyCode;

mod tui_dialog_widgets;
mod status_bar;
mod transfers_window;
mod user_queries_window;
mod user_actions_window;

use user_actions_window::SelectedAction;

pub enum DeviceEvent {
    DeviceUnplugged,
}

#[derive(Debug)]
pub enum LogicToUiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    NewTransfer { name: String, rx_control: Receiver<DeviceEvent> },
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

fn app(terminal: &mut DefaultTerminal,rx: Receiver<LogicToUiMessage>,tx: Sender<UiToLogicMessage>) -> std::io::Result<()> {
    let mut l_allow:Vec<String> = [].to_vec();
    let mut l_ignore:Vec<String> = [].to_vec();
    let mut active_transfers: Vec<(String, Receiver<DeviceEvent>)> = Vec::new();
    let mut selected_action = SelectedAction::Quit;
    loop {
        active_transfers.retain(|(_, rx_control)| {
            !matches!(rx_control.try_recv(), Ok(DeviceEvent::DeviceUnplugged))
        });
        let active_names: Vec<&str> = active_transfers.iter().map(|(n, _)| n.as_str()).collect();
        terminal.draw( |frame| { render(frame, &l_allow, &l_ignore, &selected_action, &active_names) })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LogicToUiMessage::AddConfig {allow, ignore} => {
                    l_allow = allow;
                    l_ignore = ignore;
                }
                LogicToUiMessage::Quit => return Ok(()),
                LogicToUiMessage::NewTransfer {name, rx_control} => {
                    active_transfers.push((name, rx_control));
                }
            }
        }

        if event::poll(Duration::from_millis(200))? && let Event::Key(key) = event::read()? {
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

fn render(frame: &mut Frame, allow:&[String], ignore:&[String], selected_action:&SelectedAction, active_names:&[&str]) {
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

    let show_user_queries = !active_names.is_empty();
    let windows = Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(4)
        .vertical_margin(2)
        .constraints(if show_user_queries {
            vec![
                Constraint::Percentage(20),
                Constraint::Length(3),
                Constraint::Percentage(40),
                Constraint::Length(3),
                Constraint::Percentage(40),
            ]
        } else {
            vec![
                Constraint::Percentage(60),
                Constraint::Length(3),
                Constraint::Percentage(40),
            ]
        })
        .split(layout[1]);

    // Status bars
    status_bar::render(frame, layout[0]);

    // Windows
    let mut window_index=0;

    transfers_window::render(frame, windows[window_index]);
    window_index += 2;

    if show_user_queries {
        user_queries_window::render(frame, windows[window_index], allow, ignore, active_names);
        window_index += 2;
    }

    user_actions_window::render(frame, windows[window_index], selected_action);
    //window_index+=2;
}
