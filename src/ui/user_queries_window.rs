use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Layout, Direction, Constraint};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::buffer::Buffer;
use ratatui::widgets::{Paragraph, Widget, Wrap};
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets::{self, TextEntryState, TextEntryOutcome};
use crate::ui_api::{UserQuery, ApproveTransferQuery, ApproveTransferQueryUpdate, TransferFieldState, UnknownDeviceQuery, UnknownDeviceResponse, ApproveTransferResponse, SourceMediaSelection, FatalErrorQuery, FatalErrorKind, SourceMediaWarningsQuery, ConfirmCardIdQuery, CardIdConflictReason, ConfirmCardIdResponse, NoSourceMediaWarningResponse, NoDeviceLocationWarningResponse, NoDeviceLocationWarningReason, NoInputPathWarningResponse, NewBackupLogQuery, NewBackupLogResponse, CardIdInLogWarningQuery, CardIdInLogWarningResponse};
use crate::{SourceMediaEntry, StorageDeviceEntry};

pub struct QueryWindowState {
    pub device_picker_open: bool,
    pub device_picker_selection: usize,
    pub device_override: Option<SourceMediaEntry>,
    pub storage_device_picker_open: bool,
    pub storage_device_picker_selection: usize,
    pub card_id_entry: Option<TextEntryState>,
    pub device_location_picker_open: bool,
    pub device_location_picker_selection: usize,
    pub input_path_picker_open: bool,
    pub input_path_picker_selection: usize,
    /// The actual OS directory currently displayed in the input path picker.
    pub input_path_picker_current_dir: Option<PathBuf>,
    /// Mount root constraint: the picker cannot navigate above this path.
    /// `None` means the full filesystem is browsable (local-filesystem transfers).
    pub input_path_picker_mount_root: Option<PathBuf>,
    /// Entries in the current picker directory: (display label, actual OS path, is_dir).
    pub input_path_picker_entries: Vec<(String, PathBuf, bool)>,
}

impl QueryWindowState {
    pub fn new() -> Self {
        Self {
            device_picker_open: false,
            device_picker_selection: 0,
            device_override: None,
            storage_device_picker_open: false,
            storage_device_picker_selection: 0,
            card_id_entry: None,
            device_location_picker_open: false,
            device_location_picker_selection: 0,
            input_path_picker_open: false,
            input_path_picker_selection: 0,
            input_path_picker_current_dir: None,
            input_path_picker_mount_root: None,
            input_path_picker_entries: Vec::new(),
        }
    }
}

fn can_approve(data: &ApproveTransferQueryUpdate) -> bool {
    data.source_media_dir.value().is_some()
        && data.card_id.value().is_some_and(|id| !id.is_empty())
        && data.source_device.value().is_some_and(|d| !d.is_empty())
        && data.device_location.value().is_some()
        && data.input_path.value().is_some()
}

pub fn handle_key(state: &mut QueryWindowState, key: KeyEvent, query_queue: &mut VecDeque<UserQuery>, available_devices: Option<&[SourceMediaEntry]>) {
    match query_queue.front() {
        Some(UserQuery::ApproveTransfer(query)) => {
            if let Some(entry) = &mut state.card_id_entry {
                match entry.handle_key(key.code) {
                    TextEntryOutcome::Confirmed(new_id) => {
                        let _ = query.response_tx.send(ApproveTransferResponse::CardIdChanged(new_id));
                        state.card_id_entry = None;
                    }
                    TextEntryOutcome::Cancelled => { state.card_id_entry = None; }
                    TextEntryOutcome::Editing   => {}
                }
            } else if state.device_picker_open {
                let has_auto_detected = query.has_auto_detected_source_media;
                let device_offset = if has_auto_detected { 1 } else { 0 };
                let device_count = available_devices.map(|d| d.len() + device_offset).unwrap_or(device_offset.max(1));
                match key.code {
                    KeyCode::Up   => { state.device_picker_selection = state.device_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.device_picker_selection = (state.device_picker_selection + 1).min(device_count.saturating_sub(1)); }
                    KeyCode::Enter => {
                        if has_auto_detected && state.device_picker_selection == 0 {
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceOverwrite(SourceMediaSelection::Auto));
                            state.device_override = None;
                        } else {
                            let picked = available_devices.and_then(|d| d.get(state.device_picker_selection - device_offset)).cloned();
                            let directory = picked.as_ref().map(|e| e.directory.to_string_lossy().into_owned()).unwrap_or_default();
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceOverwrite(SourceMediaSelection::Overridden(directory)));
                            state.device_override = picked;
                        }
                        state.device_picker_open = false;
                    }
                    KeyCode::Esc => { state.device_picker_open = false; }
                    _ => {}
                }
            } else if state.storage_device_picker_open {
                let devices = &query.available_storage_devices;
                let has_auto = query.has_auto_detected_storage_device;
                let device_offset = if has_auto { 1 } else { 0 };
                let device_count = devices.len() + device_offset;
                match key.code {
                    KeyCode::Up   => { state.storage_device_picker_selection = state.storage_device_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.storage_device_picker_selection = (state.storage_device_picker_selection + 1).min(device_count.saturating_sub(1)); }
                    KeyCode::Enter => {
                        if has_auto && state.storage_device_picker_selection == 0 {
                            let _ = query.response_tx.send(ApproveTransferResponse::StorageDeviceAuto);
                        } else if let Some(device) = devices.get(state.storage_device_picker_selection - device_offset) {
                            let _ = query.response_tx.send(ApproveTransferResponse::StorageDeviceChanged(device.id.clone()));
                        }
                        state.storage_device_picker_open = false;
                    }
                    KeyCode::Esc => { state.storage_device_picker_open = false; }
                    _ => {}
                }
            } else if state.device_location_picker_open {
                let locations = &query.available_device_locations;
                let has_auto = query.has_auto_detected_device_location;
                let location_offset = if has_auto { 1 } else { 0 };
                let location_count = locations.len() + location_offset;
                match key.code {
                    KeyCode::Up   => { state.device_location_picker_selection = state.device_location_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.device_location_picker_selection = (state.device_location_picker_selection + 1).min(location_count.saturating_sub(1)); }
                    KeyCode::Enter => {
                        if has_auto && state.device_location_picker_selection == 0 {
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceLocationAuto);
                        } else if let Some(location) = locations.get(state.device_location_picker_selection - location_offset) {
                            let _ = query.response_tx.send(ApproveTransferResponse::DeviceLocationChanged(location.clone()));
                        }
                        state.device_location_picker_open = false;
                    }
                    KeyCode::Esc => { state.device_location_picker_open = false; }
                    _ => {}
                }
            } else if state.input_path_picker_open {
                let entry_count = state.input_path_picker_entries.len();
                match key.code {
                    KeyCode::Up   => { state.input_path_picker_selection = state.input_path_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.input_path_picker_selection = (state.input_path_picker_selection + 1).min(entry_count.saturating_sub(1)); }
                    KeyCode::Char(' ') => {
                        // Select the currently browsed directory itself as the input path.
                        if let Some(ref current_dir) = state.input_path_picker_current_dir.clone() {
                            let virtual_path = actual_to_virtual(current_dir, state.input_path_picker_mount_root.as_deref());
                            let _ = query.response_tx.send(ApproveTransferResponse::InputPathChanged(virtual_path));
                            state.input_path_picker_open = false;
                        }
                    }
                    KeyCode::Enter => {
                        if let Some((_, actual_path, is_dir)) = state.input_path_picker_entries.get(state.input_path_picker_selection).cloned() {
                            if is_dir {
                                let is_parent_entry = state.input_path_picker_entries
                                    .get(state.input_path_picker_selection)
                                    .map(|(label, _, _)| label == "..")
                                    .unwrap_or(false);
                                if is_parent_entry {
                                    // Navigate up — already verified we can above the current dir
                                    let new_dir = actual_path;
                                    state.input_path_picker_entries = load_dir_entries(&new_dir, state.input_path_picker_mount_root.as_deref());
                                    state.input_path_picker_current_dir = Some(new_dir);
                                    state.input_path_picker_selection = 0;
                                } else {
                                    // Navigate into directory
                                    state.input_path_picker_entries = load_dir_entries(&actual_path, state.input_path_picker_mount_root.as_deref());
                                    state.input_path_picker_current_dir = Some(actual_path);
                                    state.input_path_picker_selection = 0;
                                }
                            } else {
                                // Select file as input path
                                let virtual_path = actual_to_virtual(&actual_path, state.input_path_picker_mount_root.as_deref());
                                let _ = query.response_tx.send(ApproveTransferResponse::InputPathChanged(virtual_path));
                                state.input_path_picker_open = false;
                            }
                        }
                    }
                    KeyCode::Esc => { state.input_path_picker_open = false; }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('s') | KeyCode::Char('S') => {
                        state.device_picker_open = true;
                        state.device_picker_selection = 0;
                    }
                    KeyCode::Char('d') | KeyCode::Char('D') => {
                        state.storage_device_picker_open = true;
                        state.storage_device_picker_selection = 0;
                    }
                    KeyCode::Char('l') | KeyCode::Char('L') => {
                        state.device_location_picker_open = true;
                        state.device_location_picker_selection = 0;
                    }
                    KeyCode::Char('i') | KeyCode::Char('I') => {
                        // Only open the picker when input_path is not frozen.
                        if !matches!(query.initial_data.input_path, TransferFieldState::Frozen) {
                            let mount_root = query.initial_data.input_path_mount_root.clone();
                            let start_dir = compute_picker_start_dir(
                                query.initial_data.input_path.value(),
                                mount_root.as_deref(),
                            );
                            state.input_path_picker_entries = load_dir_entries(&start_dir, mount_root.as_deref());
                            state.input_path_picker_mount_root = mount_root;
                            state.input_path_picker_current_dir = Some(start_dir);
                            state.input_path_picker_selection = 0;
                            state.input_path_picker_open = true;
                        }
                    }
                    KeyCode::Char('c') | KeyCode::Char('C') => {
                        state.card_id_entry = Some(TextEntryState::new(query.initial_data.card_id.value().cloned().unwrap_or_default()));
                    }
                    KeyCode::Enter if can_approve(&query.initial_data) => {
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
        Some(UserQuery::UnknownDevice(_)) => {
            match key.code {
                KeyCode::Char('1') => {
                    if let Some(UserQuery::UnknownDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AddToAllowList);
                    }
                }
                KeyCode::Char('2') => {
                    if let Some(UserQuery::UnknownDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AddToIgnoreList);
                    }
                }
                KeyCode::Char('3') => {
                    if let Some(UserQuery::UnknownDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AllowOnce);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::UnknownDevice(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::Ignore);
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
        Some(UserQuery::ConfirmCardId(query)) => {
            let can_use_new      = query.suggested_id.is_some();
            let can_use_original = !matches!(query.conflict_reason, CardIdConflictReason::IdTaken);
            match key.code {
                KeyCode::Char('n') | KeyCode::Char('N') if can_use_new => {
                    if let Some(UserQuery::ConfirmCardId(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::UseNew);
                    }
                }
                KeyCode::Char('k') | KeyCode::Char('K') if can_use_original => {
                    if let Some(UserQuery::ConfirmCardId(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::UseOriginal);
                    }
                }
                KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::ConfirmCardId(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::BackToQuery);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoSourceMediaWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoSourceMediaWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoSourceMediaWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoSourceMediaWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoSourceMediaWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoDeviceLocationWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoDeviceLocationWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoDeviceLocationWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoDeviceLocationWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoDeviceLocationWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoInputPathWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoInputPathWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoInputPathWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoInputPathWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NoInputPathWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NewBackupLog(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(UserQuery::NewBackupLog(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NewBackupLogResponse::CreateNew);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if let Some(UserQuery::NewBackupLog(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(NewBackupLogResponse::Quit);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::CardIdInLogWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::CardIdInLogWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(CardIdInLogWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::CardIdInLogWarning(query)) = query_queue.pop_front() {
                        let _ = query.response_tx.send(CardIdInLogWarningResponse::Cancel);
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
    available_devices: Option<&[SourceMediaEntry]>,
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
            render_approve_transfer(frame, padded, query, available_devices);
            if let Some(entry) = &state.card_id_entry {
                tui_dialog_widgets::TextEntry {
                    title: "Set card ID",
                    text: &entry.text,
                    cursor_pos: entry.cursor,
                }.render(frame.area(), frame.buffer_mut());
            } else if state.device_picker_open {
                render_device_picker(frame, available_devices, state.device_override.as_ref(), state.device_picker_selection, query.has_auto_detected_source_media);
            } else if state.storage_device_picker_open {
                render_storage_device_picker(frame, &query.available_storage_devices, query.initial_data.source_device.value().map(|s| s.as_str()).unwrap_or(""), query.initial_data.source_device.is_overridden(), query.has_auto_detected_storage_device, state.storage_device_picker_selection);
            } else if state.device_location_picker_open {
                render_device_location_picker(frame, &query.available_device_locations, query.initial_data.device_location.value().map(|s| s.as_str()), query.initial_data.device_location.is_overridden(), query.has_auto_detected_device_location, state.device_location_picker_selection);
            } else if state.input_path_picker_open {
                render_input_path_picker(frame, state);
            }
        }
        UserQuery::UnknownDevice(query) => render_unknown_device(frame, padded, query),
        UserQuery::FatalError(query) => render_fatal_error(frame, padded, query),
        UserQuery::SourceMediaWarnings(query) => render_source_media_warnings(frame, padded, query),
        UserQuery::ConfirmCardId(query) => render_confirm_card_id(frame, padded, query),
        UserQuery::NoSourceMediaWarning(_) => render_no_source_media_warning(frame, padded),
        UserQuery::NoDeviceLocationWarning(query) => render_no_device_location_warning(frame, padded, &query.reason),
        UserQuery::NoInputPathWarning(_) => render_no_input_path_warning(frame, padded),
        UserQuery::NewBackupLog(query) => render_new_backup_log(frame, padded, query),
        UserQuery::CardIdInLogWarning(query) => render_card_id_in_log_warning(frame, padded, query),
    }
}

fn render_approve_transfer(frame: &mut Frame, area: Rect, query: &ApproveTransferQuery, available_devices: Option<&[SourceMediaEntry]>) {
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

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Min(0),              // All info
            Constraint::Length(1),           // buttons
        ])
        .split(content_area);

    render_transfer_info(frame, rows[0], query, available_devices);

    let hint     = Style::default().fg(Color::Black);
    let ok       = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let ok_dim   = Style::default().fg(Color::DarkGray);
    let deny     = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let (approve_key_style, approve_label_style) = if can_approve(&query.initial_data) {
        (ok, hint)
    } else {
        (ok_dim, ok_dim)
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("[Enter]", approve_key_style),
            Span::styled(" Approve   ", approve_label_style),
            Span::styled("[Esc]", deny),
            Span::styled(" Deny   ", hint),
        ])),
        rows[1],
    );
}

/// Converts an actual OS path to the virtual path shown to the user.
/// When `mount_root` is set the virtual path is relative to the card root (starts with "/").
fn actual_to_virtual(actual: &Path, mount_root: Option<&Path>) -> PathBuf {
    if let Some(root) = mount_root {
        let relative = actual.strip_prefix(root).unwrap_or(Path::new(""));
        PathBuf::from("/").join(relative)
    } else {
        actual.to_owned()
    }
}

/// Computes the OS directory the input-path picker should open in.
fn compute_picker_start_dir(virtual_path: Option<&PathBuf>, mount_root: Option<&Path>) -> PathBuf {
    match (virtual_path, mount_root) {
        (Some(vp), Some(root)) => {
            let stripped = vp.strip_prefix("/").unwrap_or(vp.as_path());
            root.join(stripped)
        }
        (Some(vp), None) => vp.clone(),
        (None, Some(root)) => root.to_owned(),
        (None, None) => PathBuf::from("/"),
    }
}

/// Loads directory entries for the input-path picker from `actual_dir`.
/// The first entry is ".." when navigating above `mount_root` is still possible.
/// Returns `Vec<(display_label, actual_os_path, is_dir)>`.
fn load_dir_entries(actual_dir: &Path, mount_root: Option<&Path>) -> Vec<(String, PathBuf, bool)> {
    let mut entries: Vec<(String, PathBuf, bool)> = Vec::new();

    let can_go_up = match mount_root {
        Some(root) => actual_dir != root,
        None       => actual_dir != Path::new("/"),
    };
    if can_go_up {
        if let Some(parent) = actual_dir.parent() {
            entries.push(("..".to_owned(), parent.to_owned(), true));
        }
    }

    if let Ok(read_dir) = std::fs::read_dir(actual_dir) {
        let mut children: Vec<(String, PathBuf, bool)> = read_dir
            .flatten()
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path();
                let is_dir = path.is_dir();
                let label = if is_dir { format!("{}/", name) } else { name };
                Some((label, path, is_dir))
            })
            .collect();
        children.sort_by(|a, b| {
            // Directories before files, then alphabetically
            b.2.cmp(&a.2).then_with(|| a.0.cmp(&b.0))
        });
        entries.extend(children);
    }

    entries
}

fn render_input_path_picker(frame: &mut Frame, state: &QueryWindowState) {
    let hint_style = Style::default().fg(Color::Black);
    let ok_style   = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let esc_style  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let space_style = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let hint = Line::from(vec![
        Span::styled("[Space]", space_style),
        Span::styled(" Select dir   ", hint_style),
        Span::styled("[Enter]", ok_style),
        Span::styled(" Open/Select   ", hint_style),
        Span::styled("[Esc]", esc_style),
        Span::styled(" Cancel", hint_style),
    ]);

    // Build the title showing the virtual path currently being browsed.
    let current_virtual = state.input_path_picker_current_dir.as_deref()
        .map(|d| actual_to_virtual(d, state.input_path_picker_mount_root.as_deref()))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "/".to_owned());
    let title = format!("Browse: {}", current_virtual);

    let items: Vec<tui_dialog_widgets::DialogFloatingListItem> = state.input_path_picker_entries
        .iter()
        .map(|(label, _, _)| tui_dialog_widgets::DialogFloatingListItem {
            label: label.clone(),
            is_current: false,
        })
        .collect();

    frame.render_widget(
        tui_dialog_widgets::DialogFloatingList::new(&title)
            .items(items)
            .selected(state.input_path_picker_selection)
            .hint(hint),
        frame.area(),
    );
}

fn render_device_picker(frame: &mut Frame, available_devices: Option<&[SourceMediaEntry]>, device_override: Option<&SourceMediaEntry>, selection: usize, has_auto_detected: bool) {
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

    let mut items: Vec<tui_dialog_widgets::DialogFloatingListItem> = if has_auto_detected {
        vec![tui_dialog_widgets::DialogFloatingListItem { label: "Auto-detected".to_string(), is_current: device_override.is_none() }]
    } else {
        vec![]
    };
    match available_devices {
        None => items.push(tui_dialog_widgets::DialogFloatingListItem {
            label: "Loading available devices...".to_string(),
            is_current: false,
        }),
        Some(devices) => items.extend(devices.iter().map(|d| {
            let model = d.device_model_name_pretty.as_deref()
                .unwrap_or(&d.device_model_name);
            let display_name = format!("{} {} (SN: {})", d.device_make_name, model, d.serial_number);
            let is_selected = device_override
                .map(|ov| ov.directory == d.directory)
                .unwrap_or(false);
            tui_dialog_widgets::DialogFloatingListItem {
                label: display_name,
                is_current: is_selected,
            }
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

fn render_storage_device_picker(frame: &mut Frame, devices: &[StorageDeviceEntry], current_display_name: &str, storage_device_overridden: bool, has_auto: bool, selection: usize) {
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

    let mut items: Vec<tui_dialog_widgets::DialogFloatingListItem> = if has_auto {
        vec![tui_dialog_widgets::DialogFloatingListItem {
            label: "Auto-detected".to_string(),
            is_current: !storage_device_overridden,
        }]
    } else {
        vec![]
    };
    items.extend(devices.iter().map(|d| {
        tui_dialog_widgets::DialogFloatingListItem {
            label: d.display_name.clone(),
            is_current: storage_device_overridden && d.display_name == current_display_name,
        }
    }));

    frame.render_widget(
        tui_dialog_widgets::DialogFloatingList::new("Select storage device")
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

fn render_transfer_info(frame: &mut Frame, area: Rect, query: &ApproveTransferQuery, available_devices: Option<&[SourceMediaEntry]>) {
    let label_style  = Style::default().fg(Color::Black);
    let value_style  = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let overwritten  = Style::default().fg(Color::DarkGray);
    let none_style   = Style::default().fg(Color::DarkGray);
    let hint_style   = Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD);
    let data = &query.initial_data;
    let size_str = super::format_bytes(data.data_size);

    // Builds a display Line for one transfer field. Rendering varies by state:
    //   Frozen          → "Loading..."  in none_style   (placeholder until animated braille is added)
    //   AutoSelected(None)  → "none"        in none_style
    //   AutoSelected(Some)  → value text    in value_style
    //   Overridden          → value text    in value_style + " (overridden)" in overwritten style
    let field_line = |label: &str, hint: Option<&str>, field: &TransferFieldState<String>| -> Line<'static> {
        let mut spans = vec![Span::styled(label.to_owned(), label_style)];
        if let Some(h) = hint {
            spans.push(Span::styled(h.to_owned(), hint_style));
        }
        match field {
            TransferFieldState::Frozen                  => spans.push(Span::styled("Loading...", none_style)),
            TransferFieldState::AutoSelected(None)      => spans.push(Span::styled("none",       none_style)),
            TransferFieldState::AutoSelected(Some(val)) => spans.push(Span::styled(val.clone(),  value_style)),
            TransferFieldState::Overridden(val)         => {
                spans.push(Span::styled(val.clone(),     value_style));
                spans.push(Span::styled(" (overridden)", overwritten));
            }
        }
        Line::from(spans)
    };

    // source_media_dir stores a directory path; resolve it to a human-readable device name for display.
    let source_media_display: TransferFieldState<String> = match &data.source_media_dir {
        TransferFieldState::Frozen => TransferFieldState::Frozen,
        TransferFieldState::AutoSelected(dir_opt) => {
            let display = dir_opt.as_deref()
                .and_then(|dir| available_devices?.iter().find(|e| e.directory.to_string_lossy() == dir))
                .map(|e| {
                    let model = e.device_model_name_pretty.as_deref().unwrap_or(&e.device_model_name);
                    format!("{} {} (SN: {})", e.device_make_name, model, e.serial_number)
                });
            TransferFieldState::AutoSelected(display)
        }
        TransferFieldState::Overridden(dir) => {
            let display = available_devices
                .and_then(|devs| devs.iter().find(|e| e.directory.to_string_lossy() == dir.as_str()))
                .map(|e| {
                    let model = e.device_model_name_pretty.as_deref().unwrap_or(&e.device_model_name);
                    format!("{} {} (SN: {})", e.device_make_name, model, e.serial_number)
                })
                .unwrap_or_else(|| dir.clone());
            TransferFieldState::Overridden(display)
        }
    };

    // device_location stores a raw by-id name; translate the well-known sentinel to a readable label.
    let device_location_display: TransferFieldState<String> = match &data.device_location {
        TransferFieldState::Frozen => TransferFieldState::Frozen,
        TransferFieldState::AutoSelected(loc_opt) => {
            let display = loc_opt.as_deref().map(|loc| {
                if loc == crate::transfer_logic::LOCAL_FILESYSTEM_DEVICE_LOCATION {
                    "Local filesystem".to_owned()
                } else {
                    loc.to_owned()
                }
            });
            TransferFieldState::AutoSelected(display)
        }
        TransferFieldState::Overridden(loc) => {
            let display = if loc == crate::transfer_logic::LOCAL_FILESYSTEM_DEVICE_LOCATION {
                "Local filesystem".to_owned()
            } else {
                loc.clone()
            };
            TransferFieldState::Overridden(display)
        }
    };

    // input_path stores a PathBuf; convert to String for field_line.
    let input_path_display: TransferFieldState<String> = match &data.input_path {
        TransferFieldState::Frozen               => TransferFieldState::Frozen,
        TransferFieldState::AutoSelected(None)   => TransferFieldState::AutoSelected(None),
        TransferFieldState::AutoSelected(Some(p)) => TransferFieldState::AutoSelected(Some(p.to_string_lossy().into_owned())),
        TransferFieldState::Overridden(p)        => TransferFieldState::Overridden(p.to_string_lossy().into_owned()),
    };

    let lines: Vec<Line> = vec![
        field_line("    Source media: ", Some("[S] "), &source_media_display),
        field_line("         Card ID: ", Some("[C] "), &data.card_id),
        field_line("  Storage device: ", Some("[D] "), &data.source_device),
        field_line(" Device location: ", Some("[L] "), &device_location_display),
        field_line("      Input path: ", Some("[I] "), &input_path_display),
        Line::from(vec![
            Span::styled("  Transfer Size: ", label_style),
            Span::styled(size_str,            value_style),
        ]),
    ];

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
        FatalErrorKind::CardId(msg)      => ("Card ID handling error", msg.as_str()),
        FatalErrorKind::Transfer(msg)    => ("Data transfer error", msg.as_str()),
        FatalErrorKind::ActiveTransfers  => ("Cannot quit while transfers are in progress", "Please wait for all active transfers to complete before quitting."),
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

fn render_confirm_card_id(frame: &mut Frame, area: Rect, query: &ConfirmCardIdQuery) {
    let label   = Style::default().fg(Color::Black);
    let value   = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let hint    = Style::default().fg(Color::Black);
    let ok      = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let deny    = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let act     = Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD);

    let can_use_new      = query.suggested_id.is_some();
    let can_use_original = !matches!(query.conflict_reason, CardIdConflictReason::IdTaken);

    let (reason_text, reason_style) = match query.conflict_reason {
        CardIdConflictReason::IdTaken     => ("Card ID already exists on filesystem", warning),
        CardIdConflictReason::SequenceGap => ("Card ID would create a gap in the sequence", warning),
    };

    let mut lines: Vec<Line> = vec![
        Line::from(vec![Span::styled(reason_text, reason_style)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Current ID:  ", label),
            Span::styled(query.original_id.as_str(), value),
        ]),
    ];

    if let Some(ref suggested) = query.suggested_id {
        lines.push(Line::from(vec![
            Span::styled("Next seq ID: ", label),
            Span::styled(suggested.as_str(), value),
        ]));
    }

    lines.push(Line::from(""));

    let mut button_spans: Vec<Span> = Vec::new();
    if can_use_new {
        button_spans.push(Span::styled("[N]", ok));
        button_spans.push(Span::styled(" Use next ID   ", hint));
    }
    if can_use_original {
        button_spans.push(Span::styled("[K]", act));
        button_spans.push(Span::styled(" Keep original   ", hint));
    }
    button_spans.push(Span::styled("[B]", deny));
    button_spans.push(Span::styled(" Back to query", hint));

    lines.push(Line::from(button_spans));

    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_no_source_media_warning(frame: &mut Frame, area: Rect) {
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let hint    = Style::default().fg(Color::Black);
    let back    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let cancel  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![Span::styled("Warning: no source media selected", warning)]),
        Line::from(""),
        Line::from(vec![Span::styled("A source media device must be selected before a transfer can proceed.", label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", back),
            Span::styled(" Back to query   ", hint),
            Span::styled("[Esc]", cancel),
            Span::styled(" Cancel transfer", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_device_location_picker(frame: &mut Frame, locations: &[String], current_location: Option<&str>, location_overridden: bool, has_auto: bool, selection: usize) {
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

    let location_label = |loc: &str| -> String {
        if loc == crate::transfer_logic::LOCAL_FILESYSTEM_DEVICE_LOCATION {
            "Local filesystem".to_owned()
        } else {
            loc.to_owned()
        }
    };

    let mut items: Vec<tui_dialog_widgets::DialogFloatingListItem> = if has_auto {
        vec![tui_dialog_widgets::DialogFloatingListItem {
            label: "Auto-detected".to_string(),
            is_current: !location_overridden,
        }]
    } else {
        vec![]
    };
    items.extend(locations.iter().map(|loc| {
        tui_dialog_widgets::DialogFloatingListItem {
            label: location_label(loc),
            is_current: location_overridden && current_location == Some(loc.as_str()),
        }
    }));

    frame.render_widget(
        tui_dialog_widgets::DialogFloatingList::new("Select device location")
            .items(items)
            .selected(selection)
            .hint(hint),
        frame.area(),
    );
}

fn render_no_device_location_warning(frame: &mut Frame, area: Rect, reason: &NoDeviceLocationWarningReason) {
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let hint    = Style::default().fg(Color::Black);
    let back    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let cancel  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);

    let reason_text = match reason {
        NoDeviceLocationWarningReason::NoneSelected => "No device location selected",
        NoDeviceLocationWarningReason::NotFound     => "Selected device location not found",
    };
    let detail_text = match reason {
        NoDeviceLocationWarningReason::NoneSelected =>
            "A device location must be selected before a transfer can proceed.",
        NoDeviceLocationWarningReason::NotFound =>
            "The selected /dev/disk/by-id/ entry no longer exists. The device may have been unplugged.",
    };

    let lines = vec![
        Line::from(vec![Span::styled(reason_text, warning)]),
        Line::from(""),
        Line::from(vec![Span::styled(detail_text, label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", back),
            Span::styled(" Back to query   ", hint),
            Span::styled("[Esc]", cancel),
            Span::styled(" Cancel transfer", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_no_input_path_warning(frame: &mut Frame, area: Rect) {
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let hint    = Style::default().fg(Color::Black);
    let back    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let cancel  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![Span::styled("Warning: no input path selected", warning)]),
        Line::from(""),
        Line::from(vec![Span::styled("An input path must be selected before a transfer can proceed.", label)]),
        Line::from(vec![Span::styled("Press [I] in the transfer dialog to choose a directory or file.", label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", back),
            Span::styled(" Back to query   ", hint),
            Span::styled("[Esc]", cancel),
            Span::styled(" Cancel transfer", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_new_backup_log(frame: &mut Frame, area: Rect, _query: &NewBackupLogQuery) {
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let hint    = Style::default().fg(Color::Black);
    let ok      = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let quit    = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![Span::styled("Previous backup log is complete", warning)]),
        Line::from(""),
        Line::from(vec![Span::styled("The last backup log entry is marked as complete.", label)]),
        Line::from(vec![Span::styled("Start a new backup log entry to continue recording transfers?", label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", ok),
            Span::styled(" Create new log   ", hint),
            Span::styled("[Esc]", quit),
            Span::styled(" Quit", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_card_id_in_log_warning(frame: &mut Frame, area: Rect, query: &CardIdInLogWarningQuery) {
    let error   = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let value   = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let hint    = Style::default().fg(Color::Black);
    let back    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let cancel  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![Span::styled("Error: card ID already in backup log", error)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("Card ID ", label),
            Span::styled(query.card_id.as_str(), value),
            Span::styled(" already has a transfer recorded in the backup log.", label),
        ]),
        Line::from(vec![Span::styled("The card directory may have been deleted and its ID reused.", label)]),
        Line::from(vec![Span::styled("Go back to choose a different card ID.", label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", back),
            Span::styled(" Back to query   ", hint),
            Span::styled("[Esc]", cancel),
            Span::styled(" Cancel transfer", hint),
        ]),
    ];
    let content_height = lines.len() as u16;
    let y_offset = area.height.saturating_sub(content_height) / 2;
    let centered = Rect {
        y:      area.y + y_offset,
        height: content_height.min(area.height),
        ..area
    };
    frame.render_widget(Paragraph::new(lines), centered);
}

fn render_unknown_device(frame: &mut Frame, area: Rect, query: &UnknownDeviceQuery) {
    let label  = Style::default().fg(Color::Black);
    let value  = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let hint   = Style::default().fg(Color::Black);
    let key    = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let escape = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![
            Span::styled("Unknown device: ", label),
            Span::styled(query.device_name.as_str(), value),
        ]),
        Line::from(""),
        Line::from(vec![Span::styled("What would you like to do?", hint)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[1]", key),
            Span::styled(" Add to allow list  (saved to config)", hint),
        ]),
        Line::from(vec![
            Span::styled("[2]", key),
            Span::styled(" Add to ignore list (saved to config)", hint),
        ]),
        Line::from(vec![
            Span::styled("[3]", key),
            Span::styled(" Allow this session only", hint),
        ]),
        Line::from(vec![
            Span::styled("[Esc]", escape),
            Span::styled(" Skip", hint),
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
