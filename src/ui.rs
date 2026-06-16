use std::thread::JoinHandle;
use std::thread;
use crossbeam_channel::{self, Receiver, Sender};
use ratatui::DefaultTerminal;
use ratatui::Frame;
use ratatui::layout::{Layout, Direction, Constraint};
use ratatui::style::{Color, Style};
use ratatui::widgets::Block;
use crossterm::event;
use std::time::Duration;
use crossterm::event::{Event, KeyCode};
use ratatui::layout::Rect;
use crate::SourceMediaEntry;
mod tui_dialog_widgets;
mod status_bar;
mod transfers_window;
mod user_queries_window;
mod user_actions_window;
mod mount_list_overlay;

use user_actions_window::{ActionsWindowState, ActionsWindowEvent};
use crate::ui_api::{
    TransferSample, TransferEvent, UserQuery, UiToLogicMessage, UiError,
    MountEntryStatus, MountUpdate, LoadingField, SystemInfo,
};
use crate::ui_api::UiBackend;

enum TransferStatus {
    NotStarted,
    InProgress,
    Finished,
    Failed,
}

fn has_active_ui_transfers(transfers: &[Transfer]) -> bool {
    transfers.iter().any(|t| matches!(t.status, TransferStatus::InProgress | TransferStatus::NotStarted))
}

struct Transfer {
    source_media_dir: Option<String>,
    bytes_total: u64,
    samples: Vec<TransferSample>,
    status: TransferStatus,
    rx_control: Receiver<TransferEvent>,
}

enum LogicToUiMessage {
    AddConfig { allow: Vec<String>, ignore: Vec<String> },
    SetAvailableDevices(Vec<SourceMediaEntry>),
    NewTransfer { source_media_dir: Option<String>, rx_control: Receiver<TransferEvent> },
    UserQuery { query: UserQuery, priority: bool },
    MountUpdate(MountUpdate),
    SystemInfo(SystemInfo),
    Quit,
}

/// Handle to the TUI backend. Owns the channel sender and the UI thread join handle.
pub struct TuiBackend {
    tx: Sender<LogicToUiMessage>,
    handle: JoinHandle<()>,
}

impl TuiBackend {
    pub fn new(ui_to_logic_tx: Sender<UiToLogicMessage>) -> TuiBackend {
        let (tx, rx) = crossbeam_channel::unbounded::<LogicToUiMessage>();
        color_eyre::install().unwrap();
        let handle = thread::spawn(|| {
            ratatui::run(|terminal| { app(terminal, rx, ui_to_logic_tx) }).unwrap();
        });
        TuiBackend { tx, handle }
    }
}

impl UiBackend for TuiBackend {
    fn add_config(&mut self, allow: Vec<String>, ignore: Vec<String>) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::AddConfig { allow, ignore }).map_err(|_| UiError::Disconnected)
    }
    fn set_available_devices(&mut self, devices: Vec<SourceMediaEntry>) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::SetAvailableDevices(devices)).map_err(|_| UiError::Disconnected)
    }
    fn new_transfer(&mut self, source_media_dir: Option<String>, rx_control: Receiver<TransferEvent>) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::NewTransfer { source_media_dir, rx_control }).map_err(|_| UiError::Disconnected)
    }
    fn user_query(&mut self, query: UserQuery, priority: bool) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::UserQuery { query, priority }).map_err(|_| UiError::Disconnected)
    }
    fn mount_update(&mut self, update: crate::ui_api::MountUpdate) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::MountUpdate(update)).map_err(|_| UiError::Disconnected)
    }
    fn system_info(&mut self, info: SystemInfo) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::SystemInfo(info)).map_err(|_| UiError::Disconnected)
    }
    fn quit(&mut self) -> Result<(), UiError> {
        self.tx.send(LogicToUiMessage::Quit).map_err(|_| UiError::Disconnected)
    }
    fn join(self: Box<Self>) {
        self.handle.join().unwrap();
    }
}

fn app(terminal: &mut DefaultTerminal, rx: Receiver<LogicToUiMessage>, tx: Sender<UiToLogicMessage>) -> std::io::Result<()> {
    let mut l_allow: Vec<String> = Vec::new();
    let mut l_ignore: Vec<String> = Vec::new();
    let mut transfers: Vec<Transfer> = Vec::new();
    let mut available_devices: Option<Vec<SourceMediaEntry>> = None;
    let mut actions_state = ActionsWindowState::new();
    let mut query_state = user_queries_window::QueryWindowState::new();
    let mut mount_list_state = mount_list_overlay::MountListState::new();
    #[cfg(feature = "fps-counter")]
    let mut frame_times: std::collections::VecDeque<std::time::Instant> = std::collections::VecDeque::new();
    #[cfg(feature = "fps-counter")]
    const FPS_WINDOW: std::time::Duration = std::time::Duration::from_secs(2);
    let mut current_system_info: Option<SystemInfo> = None;

    loop {
        // Process events on each transfer's control channel
        let mut i = 0;
        while i < transfers.len() {
            let mut remove = false;
            while let Ok(event) = transfers[i].rx_control.try_recv() {
                match event {
                    TransferEvent::DeviceUnplugged => { remove = true; }
                    TransferEvent::SourceMediaChanged(dir) => { transfers[i].source_media_dir = dir; }
                    TransferEvent::TransferStarted { bytes_total } => {
                        transfers[i].bytes_total = bytes_total;
                        transfers[i].status = TransferStatus::InProgress;
                    }
                    TransferEvent::TransferSamples(new_samples) => {
                        transfers[i].samples.extend(new_samples);
                        if matches!(transfers[i].status, TransferStatus::InProgress) {
                            let bytes_done = transfers[i].samples.last().map(|s| s.bytes_done).unwrap_or(0);
                            if bytes_done >= transfers[i].bytes_total {
                                transfers[i].status = TransferStatus::Finished;
                            }
                        }
                    }
                    TransferEvent::TransferFailed => {
                        transfers[i].status = TransferStatus::Failed;
                    }
                }
            }
            if remove {
                transfers.remove(i);
            } else {
                i += 1;
            }
        }

        // Keep the "Unmount and exit" option greyed out while any transfer is running.
        #[cfg(not(feature = "disable-ui-safety-checks"))]
        {
            let transfers_active = has_active_ui_transfers(&transfers);
            actions_state.quit_disabled = transfers_active;
            if transfers_active && matches!(actions_state.selected, user_actions_window::SelectedAction::Quit) {
                actions_state.selected = user_actions_window::SelectedAction::ManualTransfer;
            }
        }

        //Update the data of the current transfer query if the main logic has sent anything new
        if let Some(UserQuery::ApproveTransfer(latest_query)) = query_state.query_queue.front_mut() {
            if let Ok(update) = latest_query.update_rx.try_recv() {
                latest_query.fields = update;
            }
        }

        #[cfg(feature = "fps-counter")]
        let fps = {
            let now = std::time::Instant::now();
            frame_times.push_back(now);
            while frame_times.front().is_some_and(|t| now.duration_since(*t) > FPS_WINDOW) {
                frame_times.pop_front();
            }
            frame_times.len() as f64 / FPS_WINDOW.as_secs_f64()
        };

        terminal.draw(|frame| {
            render(frame, &actions_state, &query_state, &transfers, available_devices.as_deref(), current_system_info.as_ref(), &mount_list_state,
                #[cfg(feature = "fps-counter")] fps,
            )
        })?;

        while let Ok(msg) = rx.try_recv() {
            match msg {
                LogicToUiMessage::AddConfig { allow, ignore } => {
                    l_allow = allow;
                    l_ignore = ignore;
                }
                LogicToUiMessage::SetAvailableDevices(devices) => {
                    available_devices = Some(devices);
                }
                LogicToUiMessage::Quit => return Ok(()),
                LogicToUiMessage::NewTransfer { source_media_dir, rx_control } => {
                    transfers.push(Transfer {
                        source_media_dir,
                        bytes_total: 0,
                        samples: Vec::new(),
                        status: TransferStatus::NotStarted,
                        rx_control,
                    });
                }
                LogicToUiMessage::UserQuery { query: q, priority } => {
                    if query_state.query_queue.is_empty() {
                        query_state = user_queries_window::QueryWindowState::new();
                    }
                    if priority {
                        query_state.query_queue.push_front(q);
                    } else {
                        query_state.query_queue.push_back(q);
                    }
                }
                LogicToUiMessage::SystemInfo(info) => {
                    current_system_info = Some(info);
                }
                LogicToUiMessage::MountUpdate(update) => {
                    match update {
                        MountUpdate::MountAdded(entry) => {
                            mount_list_state.mounts.push(entry);
                        }
                        MountUpdate::MountCompleted { id, fs_type } => {
                            if let Some(entry) = mount_list_state.mounts.iter_mut().find(|e| e.id == id) {
                                entry.status = MountEntryStatus::Mounted;
                                entry.fs_type = LoadingField::Loaded(fs_type);
                            }
                        }
                        MountUpdate::MountFailed { id, reason } => {
                            if let Some(entry) = mount_list_state.mounts.iter_mut().find(|e| e.id == id) {
                                entry.status = MountEntryStatus::Failed { reason };
                            }
                        }
                        MountUpdate::MountRemoved { id } => {
                            mount_list_state.mounts.retain(|e| e.id != id);
                            if !mount_list_state.mounts.is_empty() && mount_list_state.selected >= mount_list_state.mounts.len() {
                                mount_list_state.selected = mount_list_state.mounts.len() - 1;
                            }
                        }
                        MountUpdate::UnmountFailed { id, reason } => {
                            if let Some(entry) = mount_list_state.mounts.iter_mut().find(|e| e.id == id) {
                                entry.status = MountEntryStatus::UnmountFailed { reason };
                            }
                        }
                    }
                }
            }
        }

        if event::poll(Duration::from_millis(16))? && let Event::Key(key) = event::read()? {
            // The mount list key is always intercepted first so the user can always open it.
            if matches!(key.code, KeyCode::Char('f') | KeyCode::Char('F')) && !mount_list_state.open {
                mount_list_state.open = true;
                mount_list_state.selected = 0;
            } else if mount_list_state.open {
                match mount_list_overlay::handle_key(&mut mount_list_state, key) {
                    Some(mount_list_overlay::MountListEvent::Unmount(id)) => {
                        tx.send(UiToLogicMessage::UnmountRequest(id)).unwrap();
                    }
                    Some(mount_list_overlay::MountListEvent::Close) => {
                        mount_list_state.open = false;
                    }
                    None => {}
                }
            } else if !query_state.query_queue.is_empty() {
                let prev_len = query_state.query_queue.len();
                user_queries_window::handle_key(&mut query_state, key, available_devices.as_deref());
                if query_state.query_queue.len() != prev_len {
                    let remaining_queue = std::mem::take(&mut query_state.query_queue);
                    query_state = user_queries_window::QueryWindowState::new(); //TODO: check what's going on here
                    query_state.query_queue = remaining_queue;
                }
            } else {
                match user_actions_window::handle_key(&mut actions_state, key) {
                    Some(ActionsWindowEvent::Quit)                => tx.send(UiToLogicMessage::Quit).unwrap(),
                    Some(ActionsWindowEvent::StartManualTransfer) => tx.send(UiToLogicMessage::StartManualTransfer).unwrap(),
                    None => {}
                }
            }
        }

        let _ = (l_allow.as_slice(), l_ignore.as_slice()); // suppress unused warnings until wired up
    }
}

/// Terminal font cell aspect ratio: cells are approximately this many times taller than wide.
/// Used to compute visually square dimensions from cell counts.
pub const FONT_CELL_ASPECT_RATIO: u16 = 2;

// Unicode braille encodes 8 dots as consecutive bit positions 0x01–0x80, but the spatial
// layout is not top-to-bottom because braille was originally a 2×3 grid (dots 1–6) and
// dots 7 & 8 were later appended at the bottom of each column without renumbering:
//
//   Left col   Right col        bit
//    dot1        dot4       0x01  0x08
//    dot2        dot5       0x02  0x10
//    dot3        dot6       0x04  0x20
//    dot7        dot8       0x40  0x80

/// Bit mask for each dot row in the left braille column (rows 0–3 top to bottom).
pub const BRAILLE_BAR_LEFT:  [u8; 4] = [0x01, 0x02, 0x04, 0x40];
/// Bit mask for each dot row in the right braille column (rows 0–3 top to bottom).
pub const BRAILLE_BAR_RIGHT: [u8; 4] = [0x08, 0x10, 0x20, 0x80];

pub fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else if bytes != 1 {
        format!("{} Bytes", bytes)
    } else {
        format!("{} Byte", bytes)
    }
}

fn render(
    frame: &mut Frame,
    actions_state: &ActionsWindowState,
    query_state: &user_queries_window::QueryWindowState,
    transfers: &[Transfer],
    available_devices: Option<&[SourceMediaEntry]>,
    system_info: Option<&SystemInfo>,
    mount_list_state: &mount_list_overlay::MountListState,
    #[cfg(feature = "fps-counter")] fps: f64
    ) {
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

    let current_query = query_state.query_queue.front();
    let show_user_queries = current_query.is_some();

    let windows = Layout::default()
        .direction(Direction::Vertical)
        .horizontal_margin(4)
        .vertical_margin(2)
        .constraints(if show_user_queries {
            vec![
                Constraint::Percentage(100),
                Constraint::Length(3),
                Constraint::Length(14),
                Constraint::Length(3),
                Constraint::Length(3),
            ]
        } else {
            vec![
                Constraint::Percentage(70),
                Constraint::Length(3),
                Constraint::Percentage(30),
            ]
        })
        .split(layout[1]);

    // Status bar
    status_bar::render(frame, layout[0], system_info, mount_list_state.mounts.len(), #[cfg(feature = "fps-counter")] fps);

    // Windows
    let mut window_index = 0;

    transfers_window::render(frame, windows[window_index], transfers, available_devices);
    window_index += 2;

    if show_user_queries {
        user_queries_window::render(frame, windows[window_index], query_state, available_devices);
        window_index += 2;
    }

    user_actions_window::render(frame, windows[window_index], actions_state, !show_user_queries, show_user_queries);

    // Mount list overlay — rendered last so it appears on top of all other content.
    if mount_list_state.open {
        let max_overlay_height = frame.area().height.saturating_sub(1) / 2;
        let overlay_height = mount_list_overlay::required_height(mount_list_state, max_overlay_height.max(5));
        let overlay_area = Rect {
            x: 0,
            y: 1, // just below the status bar
            width: frame.area().width,
            height: overlay_height,
        };
        mount_list_overlay::render(frame, overlay_area, mount_list_state);
    }
}

#[cfg(test)]
#[path = "ui_tests.rs"]
mod tests;
