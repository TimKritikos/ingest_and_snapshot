use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect, Layout, Direction, Constraint};
use ratatui::style::{Color, Style, Modifier};
use ratatui::text::{Line, Span};
use ratatui::buffer::Buffer;
use ratatui::widgets::{Paragraph, Widget, Wrap};
#[cfg(feature = "device-thumbnails")]
use ratatui::widgets::{Block, BorderType};
use crossterm::event::{KeyCode, KeyEvent};
use super::tui_dialog_widgets::{self, TextEntryState, TextEntryOutcome};
use crate::ui_api::{UserQuery, ApproveTransferQuery, UnknownDeviceQuery, UnknownDeviceResponse, ApproveTransferResponse, SourceMediaSelection, FatalErrorQuery, FatalErrorKind, SourceMediaWarningsQuery, ConfirmCardIdQuery, CardIdConflictReason, ConfirmCardIdResponse, NoSourceMediaWarningResponse, NoDeviceLocationWarningResponse, NoDeviceLocationWarningReason, NoInputPathWarningResponse, NewBackupLogQuery, NewBackupLogResponse, CardIdInLogWarningQuery, CardIdInLogWarningResponse, ZeroSizeTransferWarningResponse, SnapshotNameResponse};
use crate::{SourceMediaEntry, StorageDeviceEntry};
use crate::transfer_logic::TransferFields;
#[cfg(feature = "device-thumbnails")]
use super::thumbnails::ThumbnailRenderer;

pub struct QueryWindowState {
    pub query_queue: VecDeque<UserQuery>,
    pub device_picker_open: bool,
    pub device_picker_selection: usize,
    pub device_override: Option<SourceMediaEntry>,
    pub storage_device_picker_open: bool,
    pub storage_device_picker_selection: usize,
    pub card_id_entry: Option<TextEntryState>,
    /// Text entry buffer for the per-transfer comment, lazily created when the user opens it.
    pub comment_entry: Option<TextEntryState>,
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
    /// Text entry buffer for the snapshot-name query, lazily created when that query is shown.
    pub snapshot_name_entry: Option<TextEntryState>,
}

impl QueryWindowState {
    pub fn new() -> Self {
        Self {
            query_queue: VecDeque::new(),
            device_picker_open: false,
            device_picker_selection: 0,
            device_override: None,
            storage_device_picker_open: false,
            storage_device_picker_selection: 0,
            card_id_entry: None,
            comment_entry: None,
            device_location_picker_open: false,
            device_location_picker_selection: 0,
            input_path_picker_open: false,
            input_path_picker_selection: 0,
            input_path_picker_current_dir: None,
            input_path_picker_mount_root: None,
            input_path_picker_entries: Vec::new(),
            snapshot_name_entry: None,
        }
    }
}

fn can_approve(fields: &TransferFields) -> bool {
    fields.source_media().is_some()
        && fields.card_id().is_some_and(|id| !id.is_empty())
        && fields.storage_device().is_some()
        && fields.device_location().is_some()
        && fields.input_path().is_some()
}

pub fn handle_key(state: &mut QueryWindowState, key: KeyEvent, available_devices: Option<&[SourceMediaEntry]>) {
    match state.query_queue.front() {
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
            } else if let Some(entry) = &mut state.comment_entry {
                match entry.handle_key(key.code) {
                    TextEntryOutcome::Confirmed(new_comment) => {
                        let _ = query.response_tx.send(ApproveTransferResponse::CommentChanged(new_comment));
                        state.comment_entry = None;
                    }
                    TextEntryOutcome::Cancelled => { state.comment_entry = None; }
                    TextEntryOutcome::Editing   => {}
                }
            } else if state.device_picker_open {
                let has_auto_detected = query.fields.source_media_detected.is_some();
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
                let has_auto = query.fields.storage_device_detected.is_some();
                let device_offset = if has_auto { 1 } else { 0 };
                let device_count = devices.len() + device_offset;
                match key.code {
                    KeyCode::Up   => { state.storage_device_picker_selection = state.storage_device_picker_selection.saturating_sub(1); }
                    KeyCode::Down => { state.storage_device_picker_selection = (state.storage_device_picker_selection + 1).min(device_count.saturating_sub(1)); }
                    KeyCode::Enter => {
                        if has_auto && state.storage_device_picker_selection == 0 {
                            let _ = query.response_tx.send(ApproveTransferResponse::StorageDeviceAuto);
                        } else if let Some(device) = devices.get(state.storage_device_picker_selection - device_offset) {
                            let _ = query.response_tx.send(ApproveTransferResponse::StorageDeviceChanged(device.id));
                        }
                        state.storage_device_picker_open = false;
                    }
                    KeyCode::Esc => { state.storage_device_picker_open = false; }
                    _ => {}
                }
            } else if state.device_location_picker_open {
                let locations = &query.available_device_locations;
                let has_auto = query.fields.device_location_detected.is_some();
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
                        {
                            let mount_root = query.fields.mount_root.clone();
                            let start_dir = compute_picker_start_dir(
                                query.fields.input_path(),
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
                        state.card_id_entry = Some(TextEntryState::new(query.fields.card_id().cloned().unwrap_or_default()));
                    }
                    KeyCode::Char('m') | KeyCode::Char('M') => {
                        state.comment_entry = Some(TextEntryState::new(query.fields.comment.clone().unwrap_or_default()));
                    }
                    KeyCode::Enter if can_approve(&query.fields) => {
                        if let Some(UserQuery::ApproveTransfer(query)) = state.query_queue.pop_front() {
                            let _ = query.response_tx.send(ApproveTransferResponse::Approved);
                        }
                    }
                    KeyCode::Esc => {
                        if let Some(UserQuery::ApproveTransfer(query)) = state.query_queue.pop_front() {
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
                    if let Some(UserQuery::UnknownDevice(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AddToAllowList);
                    }
                }
                KeyCode::Char('2') => {
                    if let Some(UserQuery::UnknownDevice(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AddToIgnoreList);
                    }
                }
                KeyCode::Char('3') => {
                    if let Some(UserQuery::UnknownDevice(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::AllowOnce);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::UnknownDevice(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(UnknownDeviceResponse::Ignore);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::FatalError(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if let Some(UserQuery::FatalError(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(());
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::SourceMediaWarnings(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Esc => {
                    if let Some(UserQuery::SourceMediaWarnings(query)) = state.query_queue.pop_front() {
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
                    if let Some(UserQuery::ConfirmCardId(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::UseNew);
                    }
                }
                KeyCode::Char('k') | KeyCode::Char('K') if can_use_original => {
                    if let Some(UserQuery::ConfirmCardId(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::UseOriginal);
                    }
                }
                KeyCode::Esc | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::ConfirmCardId(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(ConfirmCardIdResponse::BackToQuery);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoSourceMediaWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoSourceMediaWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoSourceMediaWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoSourceMediaWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoSourceMediaWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoDeviceLocationWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoDeviceLocationWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoDeviceLocationWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoDeviceLocationWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoDeviceLocationWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NoInputPathWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::NoInputPathWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoInputPathWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::NoInputPathWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NoInputPathWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::NewBackupLog(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('y') | KeyCode::Char('Y') => {
                    if let Some(UserQuery::NewBackupLog(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NewBackupLogResponse::CreateNew);
                    }
                }
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('Q') => {
                    if let Some(UserQuery::NewBackupLog(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(NewBackupLogResponse::Quit);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::CardIdInLogWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('b') | KeyCode::Char('B') => {
                    if let Some(UserQuery::CardIdInLogWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(CardIdInLogWarningResponse::BackToQuery);
                    }
                }
                KeyCode::Esc => {
                    if let Some(UserQuery::CardIdInLogWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(CardIdInLogWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::ZeroSizeTransferWarning(_)) => {
            match key.code {
                KeyCode::Enter | KeyCode::Char('p') | KeyCode::Char('P') => {
                    if let Some(UserQuery::ZeroSizeTransferWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(ZeroSizeTransferWarningResponse::Proceed);
                    }
                }
                KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                    if let Some(UserQuery::ZeroSizeTransferWarning(query)) = state.query_queue.pop_front() {
                        let _ = query.response_tx.send(ZeroSizeTransferWarningResponse::Cancel);
                    }
                }
                _ => {}
            }
        }
        Some(UserQuery::SnapshotName(query)) => {
            if let Some(entry) = state.snapshot_name_entry.as_mut() {
                match entry.handle_key(key.code) {
                    TextEntryOutcome::Confirmed(name) => {
                        let _ = query.response_tx.send(SnapshotNameResponse::Provided(name));
                        state.snapshot_name_entry = None;
                        state.query_queue.pop_front();
                    }
                    TextEntryOutcome::Cancelled => {
                        let _ = query.response_tx.send(SnapshotNameResponse::Cancelled);
                        state.snapshot_name_entry = None;
                        state.query_queue.pop_front();
                    }
                    TextEntryOutcome::Editing => {}
                }
            }
        }
        None => {}
    }
}

/// Ensures entry states are consistent with the query currently at the front of the queue.
/// Call this whenever the front of the queue may have changed (after a push or after a pop
/// resets the window state). `snapshot_name_entry` must be `Some` for the duration of a
/// `SnapshotName` query so that the 'f' shortcut guard in ui.rs can reliably detect it.
pub fn prepare_for_front_query(state: &mut QueryWindowState) {
    match state.query_queue.front() {
        Some(UserQuery::SnapshotName(_)) => {
            state.snapshot_name_entry.get_or_insert_with(|| TextEntryState::new(String::new()));
        }
        _ => {
            state.snapshot_name_entry = None;
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &QueryWindowState,
    available_devices: Option<&[SourceMediaEntry]>,
    #[cfg(feature = "device-thumbnails")] thumbnail_renderer: &ThumbnailRenderer,
) {
    let query = state.query_queue.front().expect("render called with empty query queue");
    let queued_count = state.query_queue.len() - 1;
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
            render_approve_transfer(frame, padded, query, available_devices,
                #[cfg(feature = "device-thumbnails")] thumbnail_renderer);
            if let Some(entry) = &state.card_id_entry {
                tui_dialog_widgets::TextEntry {
                    title: "Set card ID",
                    text: &entry.text,
                    cursor_pos: entry.cursor,
                }.render(frame.area(), frame.buffer_mut());
            } else if let Some(entry) = &state.comment_entry {
                tui_dialog_widgets::TextEntry {
                    title: "Set comment",
                    text: &entry.text,
                    cursor_pos: entry.cursor,
                }.render(frame.area(), frame.buffer_mut());
            } else if state.device_picker_open {
                render_device_picker(frame, available_devices, state.device_override.as_ref(), state.device_picker_selection, query.fields.source_media_detected.is_some());
            } else if state.storage_device_picker_open {
                let storage_display = query.fields.storage_device()
                    .and_then(|id| query.available_storage_devices.iter().find(|d| &d.id == id))
                    .map(|d| d.display_name.as_str())
                    .unwrap_or("");
                render_storage_device_picker(frame, &query.available_storage_devices, storage_display, query.fields.storage_device_selected.is_overridden(), query.fields.storage_device_detected.is_some(), state.storage_device_picker_selection);
            } else if state.device_location_picker_open {
                render_device_location_picker(frame, &query.available_device_locations, query.fields.device_location_name(), query.fields.device_location_selected.is_overridden(), query.fields.device_location_detected.is_some(), state.device_location_picker_selection);
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
        UserQuery::ZeroSizeTransferWarning(_) => render_zero_size_transfer_warning(frame, padded),
        UserQuery::SnapshotName(_) => {
            render_snapshot_name_prompt(frame, padded);
            // The text entry floats over the whole screen, mirroring the card-ID entry.
            let (text, cursor): (&str, usize) = match &state.snapshot_name_entry {
                Some(entry) => (&entry.text, entry.cursor),
                None        => ("", 0),
            };
            tui_dialog_widgets::TextEntry {
                title: "Snapshot name",
                text,
                cursor_pos: cursor,
            }.render(frame.area(), frame.buffer_mut());
        }
    }
}

fn render_snapshot_name_prompt(frame: &mut Frame, area: Rect) {
    let title = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    let label = Style::default().fg(Color::Black);
    let lines = vec![
        Line::from(vec![Span::styled("Finish backup and create snapshot", title)]),
        Line::from(""),
        Line::from(vec![Span::styled("Enter a name for this snapshot. It is combined with today's", label)]),
        Line::from(vec![Span::styled("date as YYYY-MM-DD_<name> to form the ZFS snapshot name.", label)]),
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

fn render_approve_transfer(
    frame: &mut Frame,
    area: Rect,
    query: &ApproveTransferQuery,
    available_devices: Option<&[SourceMediaEntry]>,
    #[cfg(feature = "device-thumbnails")] thumbnail_renderer: &ThumbnailRenderer,
) {
    let content_area = render_approve_transfer_icons(
        frame, area, query, available_devices,
        #[cfg(feature = "device-thumbnails")] thumbnail_renderer,
    );

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
    let (approve_key_style, approve_label_style) = if can_approve(&query.fields) {
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
            .map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                let path = entry.path();
                let is_dir = path.is_dir();
                let label = if is_dir { format!("{}/", name) } else { name };
                (label, path, is_dir)
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

// Titles drawn above the two device icons in the approve-transfer dialog.
const SOURCE_MEDIA_ICON_TITLE:   &str = "Source media device";
const STORAGE_DEVICE_ICON_TITLE: &str = "Storage device";

// Placeholder art drawn when a device has no thumbnail (or one cannot be displayed). Both are 13
// cells wide so the two icon columns line up. The source media icon evokes a camera; the storage
// device icon evokes a stack of disk platters.
const SOURCE_MEDIA_ICON: &[&str] = &[
    "      _      ",
    "  _n_|_|_,_  ",
    " |===.-.===| ",
    " |  ((_))  | ",
    " '==='-'===' ",
    "             ",
];


const STORAGE_DEVICE_ICON: &[&str] = &[
    "  ________  ",
    " | |____| | ",
    " |   __   | ",
    " |  (__)  | ",
    " |        | ",
    " |________| ",
];


/// Resolves the absolute thumbnail path of the source media currently selected in the dialog.
#[cfg(feature = "device-thumbnails")]
fn selected_source_media_thumbnail<'a>(
    query: &ApproveTransferQuery,
    available_devices: Option<&'a [SourceMediaEntry]>,
) -> Option<&'a Path> {
    let selected_directory = query.fields.source_media()?;
    available_devices?
        .iter()
        .find(|entry| &entry.directory == selected_directory)?
        .device_thumbnail
        .as_deref()
}

/// Resolves the absolute thumbnail path of the storage device currently selected in the dialog.
#[cfg(feature = "device-thumbnails")]
fn selected_storage_device_thumbnail(query: &ApproveTransferQuery) -> Option<&Path> {
    let selected_id = query.fields.storage_device()?;
    query.available_storage_devices
        .iter()
        .find(|device| &device.id == selected_id)?
        .device_thumbnail
        .as_deref()
}

/// Lays out the source media and storage device icons on the left of the approve-transfer dialog
/// and returns the remaining area for the dialog's textual content. When the dialog is too narrow
/// for both icons plus the text it drops to a single icon, and when narrower still it drops the
/// icons entirely and hands back the whole area.
fn render_approve_transfer_icons(
    frame: &mut Frame,
    area: Rect,
    #[cfg_attr(not(feature = "device-thumbnails"), allow(unused_variables))] query: &ApproveTransferQuery,
    #[cfg_attr(not(feature = "device-thumbnails"), allow(unused_variables))] available_devices: Option<&[SourceMediaEntry]>,
    #[cfg(feature = "device-thumbnails")] thumbnail_renderer: &ThumbnailRenderer,
) -> Rect {
    // Each icon column reserves one row for its title; the icon below it is kept roughly square by
    // deriving its cell width from its cell height and the font cell aspect ratio.
    let title_height = 1;
    let icon_area_height = area.height.saturating_sub(title_height);
    let icon_cols = icon_area_height * super::FONT_CELL_ASPECT_RATIO;
    let gap_between_icons = 1;
    let gap_before_content = 2;
    let least_characters_for_text = 20; //TODO: I made this number up

    #[cfg(feature = "device-thumbnails")]
    let source_thumbnail = selected_source_media_thumbnail(query, available_devices);
    #[cfg(feature = "device-thumbnails")]
    let storage_thumbnail = selected_storage_device_thumbnail(query);

    let two_icons_width = icon_cols + gap_between_icons + icon_cols + gap_before_content;
    let one_icon_width  = icon_cols + gap_before_content;

    if area.width > two_icons_width + least_characters_for_text {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Length(icon_cols),          // source media icon
                Constraint::Length(gap_between_icons),
                Constraint::Length(icon_cols),          // storage device icon
                Constraint::Length(gap_before_content),
                Constraint::Min(0),                     // dialog content
            ])
            .split(area);
        render_titled_device_icon(frame, cols[0], SOURCE_MEDIA_ICON_TITLE, SOURCE_MEDIA_ICON,
            #[cfg(feature = "device-thumbnails")] source_thumbnail,
            #[cfg(feature = "device-thumbnails")] thumbnail_renderer);
        render_titled_device_icon(frame, cols[2], STORAGE_DEVICE_ICON_TITLE, STORAGE_DEVICE_ICON,
            #[cfg(feature = "device-thumbnails")] storage_thumbnail,
            #[cfg(feature = "device-thumbnails")] thumbnail_renderer);
        cols[4]
    } else if area.width > one_icon_width + least_characters_for_text {
        let cols = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Length(icon_cols),
                Constraint::Length(gap_before_content),
                Constraint::Min(0),
            ])
            .split(area);
        render_titled_device_icon(frame, cols[0], SOURCE_MEDIA_ICON_TITLE, SOURCE_MEDIA_ICON,
            #[cfg(feature = "device-thumbnails")] source_thumbnail,
            #[cfg(feature = "device-thumbnails")] thumbnail_renderer);
        cols[2]
    } else {
        area
    }
}

/// Draws a centered title row and, below it, the device icon: the device's JPEG thumbnail as sixel
/// when the `device-thumbnails` feature is enabled and one is available, otherwise `placeholder_art`.
fn render_titled_device_icon(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    placeholder_art: &[&str],
    #[cfg(feature = "device-thumbnails")] thumbnail: Option<&Path>,
    #[cfg(feature = "device-thumbnails")] thumbnail_renderer: &ThumbnailRenderer,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![
            Constraint::Length(1), // title
            Constraint::Min(0),    // icon
        ])
        .split(area);

    let title_style = Style::default().fg(Color::Black).add_modifier(Modifier::BOLD);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(title.to_owned(), title_style))).alignment(Alignment::Center),
        rows[0],
    );

    let icon_area = rows[1];

    // Prefer the sixel thumbnail, framed by a text border; fall back to the placeholder art when it
    // is unavailable, fails to decode, or the terminal cannot display it.
    #[cfg(feature = "device-thumbnails")]
    if let Some(path) = thumbnail {
        let border = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::Black));
        // The image is drawn into the area inside the border. The border is rendered afterwards: it
        // only touches the perimeter cells, leaving the rendered image untouched, and is skipped
        // entirely when the thumbnail could not be drawn so the placeholder takes the full area.
        let inner = border.inner(icon_area);
        if thumbnail_renderer.render_thumbnail(frame, inner, path) {
            frame.render_widget(border, icon_area);
            return;
        }
    }

    draw_centered_icon(frame, icon_area, placeholder_art);
}

/// Fills `area` with the icon background and draws `icon` centered within it.
fn draw_centered_icon(frame: &mut Frame, area: Rect, icon: &[&str]) {
    let buf = frame.buffer_mut();
    let bg   = Style::default().bg(Color::DarkGray).fg(Color::Gray);
    let body = Style::default().bg(Color::DarkGray).fg(Color::White);

    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            buf[(x, y)].set_char(' ').set_style(bg);
        }
    }

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
    let fields = &query.fields;

    // Renders one field line. `value` is the already-resolved display string (None → "none");
    // `overridden` appends a "(overridden)" marker when the user manually set the field.
    let field_line = |label: &str, hint: Option<&str>, value: Option<&str>, overridden: bool| -> Line<'static> {
        let mut spans = vec![Span::styled(label.to_owned(), label_style)];
        if let Some(h) = hint {
            spans.push(Span::styled(h.to_owned(), hint_style));
        }
        match value {
            None      => spans.push(Span::styled("none", none_style)),
            Some(val) => {
                spans.push(Span::styled(val.to_owned(), value_style));
                if overridden {
                    spans.push(Span::styled(" (overridden)", overwritten));
                }
            }
        }
        Line::from(spans)
    };

    // source media is stored as a directory id; resolve it to a human-readable device name.
    let source_media_value: Option<String> = fields.source_media().map(|dir| {
        available_devices
            .and_then(|devs| devs.iter().find(|e| &e.directory == dir))
            .map(|e| {
                let model = e.device_model_name_pretty.as_deref().unwrap_or(&e.device_model_name);
                format!("{} {} (SN: {})", e.device_make_name, model, e.serial_number)
            })
            .unwrap_or_else(|| dir.to_string_lossy().into_owned())
    });

    let card_id_value: Option<String> = fields.card_id().cloned();

    // storage device is stored as a UUID; resolve it to its display name.
    let storage_value: Option<String> = fields.storage_device().map(|id| {
        query.available_storage_devices.iter()
            .find(|d| &d.id == id)
            .map(|d| d.display_name.clone())
            .unwrap_or_else(|| id.to_string())
    });

    // device location is a by-id name; translate the well-known sentinel to a readable label.
    let device_location_value: Option<String> = fields.device_location_name().map(|name| {
        if name == crate::transfer_logic::LOCAL_FILESYSTEM_DEVICE_LOCATION {
            "Local filesystem".to_owned()
        } else {
            name.to_owned()
        }
    });

    let input_path_value: Option<String> = fields.input_path().map(|p| p.to_string_lossy().into_owned());

    // The comment has no auto/override state, so it is never marked "(overridden)".
    let comment_value: Option<String> = fields.comment.clone();

    let lines: Vec<Line> = vec![
        field_line("    Source media: ", Some("[S] "), source_media_value.as_deref(),     fields.source_media_selected.is_overridden()),
        field_line("         Card ID: ", Some("[C] "), card_id_value.as_deref(),          fields.card_id_selected.is_overridden()),
        field_line("  Storage device: ", Some("[D] "), storage_value.as_deref(),          fields.storage_device_selected.is_overridden()),
        field_line(" Device location: ", Some("[L] "), device_location_value.as_deref(),  fields.device_location_selected.is_overridden()),
        field_line("      Input path: ", Some("[I] "), input_path_value.as_deref(),        fields.input_path_selected.is_overridden()),
        field_line("         Comment: ", Some("[M] "), comment_value.as_deref(),           false),
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
        FatalErrorKind::DevicesJson(msg)    => ("Failed to load data from devices.json", msg.as_str()),
        FatalErrorKind::SourceMedia(msg)    => ("Failed to load source media configurations", msg.as_str()),
        FatalErrorKind::BackupLog(msg)      => ("Failed to load backup log", msg.as_str()),
        FatalErrorKind::CardId(msg)         => ("Card ID handling error", msg.as_str()),
        FatalErrorKind::Transfer(msg)       => ("Data transfer error", msg.as_str()),
        FatalErrorKind::ActiveTransfers     => ("Cannot quit while transfers are in progress", "Please wait for all active transfers to complete before quitting."),
        FatalErrorKind::PerDeviceConfig(msg) => ("Failed to load per-device config", msg.as_str()),
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

        #[allow(clippy::manual_range_contains)]
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

fn render_zero_size_transfer_warning(frame: &mut Frame, area: Rect) {
    let warning = Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD);
    let label   = Style::default().fg(Color::Black);
    let hint    = Style::default().fg(Color::Black);
    let proceed = Style::default().fg(Color::Green).add_modifier(Modifier::BOLD);
    let cancel  = Style::default().fg(Color::Red).add_modifier(Modifier::BOLD);
    let lines = vec![
        Line::from(vec![Span::styled("Warning: transfer size is zero bytes", warning)]),
        Line::from(""),
        Line::from(vec![Span::styled("No files were found at the selected input path.", label)]),
        Line::from(vec![Span::styled("The transfer will succeed but copy nothing.", label)]),
        Line::from(vec![Span::styled("If cancled you need to delete the directory manually", label)]),
        Line::from(""),
        Line::from(vec![
            Span::styled("[Enter]", proceed),
            Span::styled(" Proceed anyway   ", hint),
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
