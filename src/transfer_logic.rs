use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::thread;
use crate::ui_api::{self, UiBackend};
use crate::SourceMediaEntry;

//Detected info before the transfer starts
pub struct DetectedTransferInfo {
    pub source_media: Option<SourceMediaEntry>,
    pub card_id: Option<String>,
    pub source_device: Option<String>,
}

pub fn spawn_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    all_source_media: Vec<SourceMediaEntry>,
    detected: DetectedTransferInfo,
) {
    thread::spawn(move || {
        run_transfer(ui, all_source_media, detected);
    });
}

fn run_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    all_source_media: Vec<SourceMediaEntry>,
    detected: DetectedTransferInfo,
) {
    let initial_source_media_dir = detected.source_media.as_ref()
        .map(|e| e.directory.to_string_lossy().into_owned());

    // Step 1: Register the transfer in the UI
    let (transfer_event_tx, transfer_event_rx) = mpsc::channel::<ui_api::TransferEvent>();
    if ui.lock().unwrap().new_transfer(
        initial_source_media_dir,
        transfer_event_rx,
    ).is_err() { return; }

    // Step 2: Ask the user to approve and optionally pick a source media device
    let initial_data = query_update_from_entry(
        detected.source_media.as_ref(),
        detected.source_device.unwrap_or_default(),
        detected.card_id.unwrap_or_default(),
        false,
    );

    let (response_tx, response_rx) = mpsc::channel::<ui_api::ApproveTransferResponse>();
    let (update_tx, update_rx) = mpsc::channel::<ui_api::ApproveTransferQueryUpdate>();
    if ui.lock().unwrap().user_query(ui_api::UserQuery::ApproveTransfer(ui_api::ApproveTransferQuery {
        data: initial_data,
        response_tx,
        update_rx,
    })).is_err() { return; }

    // Step 3: Process responses until the user approves or denies
    while let Ok(response) = response_rx.recv() {
        match response {
            ui_api::ApproveTransferResponse::DeviceOverwrite(directory_opt) => {
                if let Some(update) = build_overwrite_update(&all_source_media, directory_opt, &transfer_event_tx) {
                    let _ = update_tx.send(update);
                }
            }
            ui_api::ApproveTransferResponse::Approved => break,
            ui_api::ApproveTransferResponse::Denied => {
                let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                return;
            }
        }
    }

    // Step 4: Move the data
    // TODO

    // Step 5: Write the backup log entry
    // TODO
}

fn build_overwrite_update(
    source_media: &[SourceMediaEntry],
    directory_opt: Option<String>,
    transfer_event_tx: &mpsc::Sender<ui_api::TransferEvent>,
) -> Option<ui_api::ApproveTransferQueryUpdate> {
    let entry = match directory_opt {
        Some(directory) => Some(
            source_media.iter()
                .find(|e| e.directory.to_string_lossy() == directory.as_str())?
        ),
        None => None,
    };
    let update = query_update_from_entry(entry, String::new(), String::new(), entry.is_some());
    let _ = transfer_event_tx.send(ui_api::TransferEvent::SourceMediaChanged(
        update.source_media_dir.clone(),
    ));
    Some(update)
}

fn query_update_from_entry(
    entry: Option<&SourceMediaEntry>,
    source_device: String,
    card_id: String,
    device_overridden: bool,
) -> ui_api::ApproveTransferQueryUpdate {
    ui_api::ApproveTransferQueryUpdate {
        source_media_dir: entry.map(|e| e.directory.to_string_lossy().into_owned()),
        source_device,
        transfer_function: String::new(),
        data_size:         0,
        card_id,
        device_overridden,
    }
}
