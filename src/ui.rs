use std::collections::VecDeque;
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

pub enum TransferEvent {
    DeviceUnplugged,
    TransferStarted { bytes_total: u64 },
    TransferSamples(Vec<(u64, u64)>),   // (unix_ms, bytes_done)
    UserQuery { question: String },
}

pub enum TransferStatus {
    NotStarted,
    InProgress,
    Finished,
}

pub struct Transfer {
    pub name: String,
    pub camera_name: String,
    pub bytes_total: u64,
    pub samples: Vec<(u64, u64)>,
    pub status: TransferStatus,
    pub alive: bool,
    pub rx_control: Receiver<TransferEvent>,
}

#[derive(Debug)]
pub enum LogicToUiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    NewTransfer { name: String, camera_name: String, rx_control: Receiver<TransferEvent> },
    Quit,
}

pub enum UiToLogicMessage {
    Quit,
}

pub fn init(rx: Receiver<LogicToUiMessage>, tx: Sender<UiToLogicMessage>) -> JoinHandle<()> {
    color_eyre::install().unwrap();
    thread::spawn(|| {
        ratatui::run(|terminal| { app(terminal, rx, tx) }).unwrap();
    })
}

fn app(terminal: &mut DefaultTerminal, rx: Receiver<LogicToUiMessage>, tx: Sender<UiToLogicMessage>) -> std::io::Result<()> {
    let mut l_allow: Vec<String> = vec![];
    let mut l_ignore: Vec<String> = vec![];
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut query_queue: VecDeque<String> = VecDeque::new();
    let mut selected_action = SelectedAction::Quit;

    loop {
        // Process events on each transfer's control channel
        for transfer in &mut transfers {
            while let Ok(event) = transfer.rx_control.try_recv() {
                match event {
                    TransferEvent::DeviceUnplugged => {
                        transfer.alive = false;
                    }
                    TransferEvent::TransferStarted { bytes_total } => {
                        transfer.bytes_total = bytes_total;
                        transfer.status = TransferStatus::InProgress;
                    }
                    TransferEvent::TransferSamples(new_samples) => {
                        transfer.samples.extend(new_samples);
                        if matches!(transfer.status, TransferStatus::InProgress) {
                            let bytes_done = transfer.samples.last().map(|&(_, b)| b).unwrap_or(0);
                            if transfer.bytes_total > 0 && bytes_done >= transfer.bytes_total {
                                transfer.status = TransferStatus::Finished;
                            }
                        }
                    }
                    TransferEvent::UserQuery { question } => {
                        query_queue.push_back(question);
                    }
                }
            }
        }

        terminal.draw(|frame| {
            render(frame, &selected_action, &query_queue, &transfers)
        })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LogicToUiMessage::AddConfig { allow, ignore } => {
                    l_allow = allow;
                    l_ignore = ignore;
                }
                LogicToUiMessage::Quit => return Ok(()),
                LogicToUiMessage::NewTransfer { name, camera_name, rx_control } => {
                    transfers.push(Transfer {
                        name,
                        camera_name,
                        bytes_total: 0,
                        samples: Vec::new(),
                        status: TransferStatus::NotStarted,
                        alive: true,
                        rx_control,
                    });
                }
            }
        }

        if event::poll(Duration::from_millis(16))? && let Event::Key(key) = event::read()? {
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

        let _ = (l_allow.as_slice(), l_ignore.as_slice()); // suppress unused warnings until wired up
    }
}

fn render(frame: &mut Frame, selected_action: &SelectedAction, query_queue: &VecDeque<String>, transfers: &[Transfer]) {
    let bg = Block::default().style(Style::default().bg(Color::Blue));
    frame.render_widget(bg, frame.area());

    let layout = Layout::default()
        .direction(Direction::Vertical)
        .margin(0)
        .constraints(vec![
            Constraint::Length(1),
            Constraint::Percentage(100)])
        .split(frame.area());

    let show_user_queries = !query_queue.is_empty();
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

    status_bar::render(frame, layout[0]);

    let mut window_index = 0;

    transfers_window::render(frame, windows[window_index], transfers);
    window_index += 2;

    if show_user_queries {
        user_queries_window::render(frame, windows[window_index], query_queue);
        window_index += 2;
    }

    user_actions_window::render(frame, windows[window_index], selected_action);
}
