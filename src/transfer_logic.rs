use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel;
use crate::ui_api::{self, UiBackend};
use crate::SourceMediaEntry;
use crate::CardNamingScheme;
use crate::transfer_registry::{PendingTransferRegistry, PendingCardId, TransferId};

// Detected info provided at transfer start
pub struct DetectedTransferInfo {
    pub source_media: Option<SourceMediaEntry>, //TODO: this probably should be an Option<String>
    pub card_id: Option<String>,
    pub source_device: Option<String>,
}

pub fn spawn_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    detected: DetectedTransferInfo,
) {
    thread::spawn(move || {
        run_transfer(ui, registry, all_source_media, all_storage_devices, detected);
    });
}

fn run_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    detected: DetectedTransferInfo,
) {
    // Assign a unique ID for this transfer in the registry
    let transfer_id: TransferId = registry.lock().unwrap().new_transfer_internal_id();

    // Determine initial source media and card ID
    let initial_source_media_dir = detected.source_media.as_ref()
        .map(|e| e.directory.clone());

    let auto_detected_device_id: Option<String> = detected.source_device.clone();

    let mut current_source_media_dir: Option<PathBuf> = initial_source_media_dir.clone();
    let mut current_source_device_id: Option<String> = auto_detected_device_id.clone();
    let mut current_device_overridden: bool = false;
    let mut current_storage_device_overridden: bool = false;
    let mut card_id_manually_set = false;

    // Compute the initial card ID and register with the registry
    let mut current_card_id = match initial_card_id_and_register(
        &registry,
        transfer_id,
        current_source_media_dir.as_deref(),
        detected.card_id.as_deref(),
        &all_source_media,
    ) {
        Ok(id) => id,
        Err(e) => {
            show_card_id_error(&ui, None, format!("Failed to compute initial card ID: {}", e));
            return;
        }
    };
    if detected.card_id.is_some() {
        card_id_manually_set = true; //TODO: not sure if we should handle this like that. maybe
                                     //drop card_id from detected all-together
    }

    // Register the transfer in the UI
    let (transfer_event_tx, transfer_event_rx) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
    if ui.lock().unwrap().new_transfer(
        current_source_media_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        transfer_event_rx,
    ).is_err() {
        if let Some(dir) = current_source_media_dir.as_deref() {
            registry.lock().unwrap().unregister(transfer_id, dir)
                .expect("unregister: transfer must be registered before unregistering");
        }
        return;
    }

    // Subscribe to registry change notifications for our source media dir.
    // When another transfer registers or changes its card ID, we get notified immediately
    // `never()` is used when there is no source media dir — it
    // acts as an inert branch in select! that never fires.
    let mut notify_rx = subscribe_or_never(&registry, current_source_media_dir.as_deref());

    let mut is_re_approval = false; // becomes true after BackToQuery loops back

    'approval_loop: loop {

        // Create approve transfer window

        let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::ApproveTransferResponse>();
        let (update_tx, update_rx)     = crossbeam_channel::unbounded::<ui_api::ApproveTransferQueryUpdate>();

        let show_priority = is_re_approval;
        if ui.lock().unwrap().user_query(
            ui_api::UserQuery::ApproveTransfer(ui_api::ApproveTransferQuery {
                data: query_update_from_state(
                          &current_source_media_dir,
                          &all_source_media,
                          &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                          &current_card_id,
                          current_device_overridden,
                          current_storage_device_overridden,
                          card_id_manually_set,
                      ),
                response_tx,
                update_rx,
                has_auto_detected_source_media: detected.source_media.is_some(),
                has_auto_detected_storage_device: auto_detected_device_id.is_some(),
                available_storage_devices: all_storage_devices.clone(),
            }),
            show_priority,
        ).is_err() {
            if let Some(dir) = current_source_media_dir.as_deref() {
                registry.lock().unwrap().unregister(transfer_id, dir)
                    .expect("unregister: transfer must be registered before unregistering");
            }
            return;
        }

        // Wait for approval. Select simultaneously on the UI response channel and the
        // registry notification channel so we react instantly to either.
        let approved = loop {
            crossbeam_channel::select! {
                recv(response_rx) -> msg => {
                    match msg {
                        Ok(ui_api::ApproveTransferResponse::Approved) => break true,
                        Ok(ui_api::ApproveTransferResponse::Denied) => break false,
                        Ok(ui_api::ApproveTransferResponse::DeviceOverwrite(selection)) => {
                            let (new_dir, device_overridden) = match selection {
                                ui_api::SourceMediaSelection::Auto => (
                                    detected.source_media.as_ref().map(|e| e.directory.clone()),
                                    false,
                                ),
                                ui_api::SourceMediaSelection::Overridden(dir_str) => (
                                    all_source_media.iter()
                                        .find(|e| e.directory.to_string_lossy() == dir_str)
                                        .map(|e| e.directory.clone()),
                                    true,
                                ),
                            };
                            if let Some(new_dir) = new_dir {
                                handle_device_overwrite(
                                    &ui,
                                    &registry,
                                    transfer_id,
                                    &mut current_source_media_dir,
                                    &mut current_card_id,
                                    &mut card_id_manually_set,
                                    &mut current_device_overridden,
                                    new_dir,
                                    device_overridden,
                                    &all_source_media,
                                    &transfer_event_tx,
                                    &update_tx,
                                    &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                    current_storage_device_overridden,
                                    &mut notify_rx,
                                );
                            }
                        }
                        Ok(ui_api::ApproveTransferResponse::CardIdChanged(new_id)) => {
                            handle_card_id_changed(
                                &ui,
                                &registry,
                                transfer_id,
                                &mut current_card_id,
                                &mut card_id_manually_set,
                                current_source_media_dir.as_deref(),
                                &all_source_media,
                                new_id,
                                &update_tx,
                                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                current_device_overridden,
                                current_storage_device_overridden,
                            );
                        }
                        Ok(ui_api::ApproveTransferResponse::StorageDeviceChanged(device_id)) => {
                            let display_name = storage_device_display_name(Some(&device_id), &all_storage_devices);
                            current_source_device_id = Some(device_id);
                            current_storage_device_overridden = true;
                            let _ = update_tx.send(query_update_from_state(
                                &current_source_media_dir,
                                &all_source_media,
                                &display_name,
                                &current_card_id,
                                current_device_overridden,
                                current_storage_device_overridden,
                                card_id_manually_set,
                            ));
                        }
                        Ok(ui_api::ApproveTransferResponse::StorageDeviceAuto) => {
                            current_source_device_id = auto_detected_device_id.clone();
                            current_storage_device_overridden = false;
                            let _ = update_tx.send(query_update_from_state(
                                &current_source_media_dir,
                                &all_source_media,
                                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                &current_card_id,
                                current_device_overridden,
                                current_storage_device_overridden,
                                card_id_manually_set,
                            ));
                        }
                        Err(_) => {
                            if let Some(dir) = current_source_media_dir.as_deref() {
                                registry.lock().unwrap().unregister(transfer_id, dir)
                                    .expect("unregister: transfer must be registered before unregistering");
                            }
                            return;
                        }
                    }
                }
                recv(notify_rx) -> result => {
                    if result.is_err() {
                        // Sender dropped — replace with a never-receiver to avoid a busy loop
                        notify_rx = crossbeam_channel::never();
                    } else if let Some(dir) = current_source_media_dir.clone() {
                        if matches!(source_media_scheme(&dir, &all_source_media), CardNamingScheme::Card)
                            && !card_id_manually_set
                        {
                            let next_card_id_result = registry.lock().unwrap().next_card_id(&dir, transfer_id);
                            match next_card_id_result {
                                Ok(new_id) if new_id != current_card_id => {
                                    current_card_id = new_id.clone();
                                    registry.lock().unwrap().update_id(
                                        transfer_id,
                                        &dir,
                                        PendingCardId::Auto(new_id.clone()),
                                    ).expect("update_id: transfer must be registered before updating");
                                    let _ = update_tx.send(query_update_from_state(
                                        &current_source_media_dir,
                                        &all_source_media,
                                        &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                        &new_id,
                                        current_device_overridden,
                                        current_storage_device_overridden,
                                        card_id_manually_set,
                                    ));
                                }
                                Ok(_) => {} // ID unchanged — skip to avoid self-notification loop
                                Err(e) => {
                                    show_card_id_error(&ui, None, format!("Failed to regenerate card ID: {}", e));
                                }
                            }
                        }
                    }
                }
            }
        };

        if !approved {
            let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged); //TODO: That is probably misuse of the api
            if let Some(dir) = current_source_media_dir.as_deref() {
                registry.lock().unwrap().unregister(transfer_id, dir)
                    .expect("unregister: transfer must be registered before unregistering");
            }
            return;
        }

        // User approved — acquire the approval lock and do TOCTOU-safe conflict check
        let source_dir = match &current_source_media_dir {
            Some(dir) => dir.clone(),
            None => {
                // No source media dir selected — warn the user and let them go back or cancel
                let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoSourceMediaWarningResponse>();
                if ui.lock().unwrap().user_query(
                    ui_api::UserQuery::NoSourceMediaWarning(ui_api::NoSourceMediaWarningQuery {
                        response_tx: warn_tx,
                    }),
                    true,
                ).is_err() {
                    return;
                }
                match warn_rx.recv() {
                    Ok(ui_api::NoSourceMediaWarningResponse::BackToQuery) | Err(_) => {
                        is_re_approval = true;
                        continue 'approval_loop;
                    }
                    Ok(ui_api::NoSourceMediaWarningResponse::Cancel) => {
                        let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                        return;
                    }
                }
            }
        };

        let scheme = source_media_scheme(&source_dir, &all_source_media);

        // For freeform: no sequential check; only check if the ID is taken
        let approval_lock = registry.lock().unwrap().get_approval_lock(&source_dir);
        let _lock_guard = approval_lock.as_ref().map(|l| l.lock().unwrap());

        // Check if this card ID is taken on the filesystem
        let is_taken = match PendingTransferRegistry::is_card_id_taken(&source_dir, &current_card_id) {
            Ok(v) => v,
            Err(e) => {
                show_card_id_error(&ui, Some(&transfer_event_tx), format!("Error checking card ID: {}", e));
                registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                return;
            }
        };

        // For Card scheme: also check if there would be a sequence gap
        let sequence_conflict = if matches!(scheme, CardNamingScheme::Card) && !is_taken {
            let next_card_id_result = registry.lock().unwrap().next_card_id(&source_dir, transfer_id);
            match next_card_id_result {
                Ok(next_sequential) => next_sequential != current_card_id,
                Err(e) => {
                    show_card_id_error(&ui, Some(&transfer_event_tx), format!("Error computing next card ID: {}", e));
                    registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                    return;
                }
            }
        } else {
            false
        };

        if is_taken || sequence_conflict {
            // Show conflict resolution dialog (priority, since it follows an approval)
            let conflict_reason = if is_taken {
                ui_api::CardIdConflictReason::IdTaken
            } else {
                ui_api::CardIdConflictReason::SequenceGap
            };

            // The "suggested" next sequential ID (UseNew option) —
            // only offer UseNew if auto-generation is applicable and there IS a next ID to suggest
            let suggested_id = if matches!(scheme, CardNamingScheme::Card) {
                match registry.lock().unwrap().next_card_id(&source_dir, transfer_id) {
                    Ok(next) if next != current_card_id || is_taken => Some(next),
                    _ => None,
                }
            } else {
                None
            };

            // UseNew is only offered when there's a suggestion and (auto OR gap case)
            let offer_use_new = suggested_id.is_some() && (!card_id_manually_set || sequence_conflict);
            let final_suggested = if offer_use_new { suggested_id.clone() } else { None };

            let (conflict_tx, conflict_rx) = crossbeam_channel::unbounded::<ui_api::ConfirmCardIdResponse>();
            if ui.lock().unwrap().user_query(
                ui_api::UserQuery::ConfirmCardId(ui_api::ConfirmCardIdQuery {
                    original_id: current_card_id.clone(),
                    suggested_id: final_suggested,
                    was_manually_set: card_id_manually_set,
                    conflict_reason,
                    response_tx: conflict_tx,
                }),
                true,
            ).is_err() {
                registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                return;
            }

            match conflict_rx.recv() {
                Ok(ui_api::ConfirmCardIdResponse::UseNew) => {
                    if let Some(new_id) = suggested_id {
                        current_card_id = new_id.clone();
                        registry.lock().unwrap().update_id(
                            transfer_id,
                            &source_dir,
                            PendingCardId::Auto(new_id),
                        ).expect("update_id: transfer must be registered before updating");
                    }
                    // Drop the lock, create the directory, then break
                    drop(_lock_guard);
                    if let Err(e) = create_card_directory(&source_dir, &current_card_id) {
                        show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                        registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                        return;
                    }
                    break 'approval_loop;
                }
                Ok(ui_api::ConfirmCardIdResponse::UseOriginal) => {
                    // Lock held — original is still free (gap case only), create it
                    drop(_lock_guard);
                    if let Err(e) = create_card_directory(&source_dir, &current_card_id) {
                        show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                        registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                        return;
                    }
                    break 'approval_loop;
                }
                Ok(ui_api::ConfirmCardIdResponse::BackToQuery) | Err(_) => {
                    // Drop lock, loop back to show ApproveTransfer again (with priority)
                    drop(_lock_guard);
                    is_re_approval = true;
                    continue 'approval_loop;
                }
            }
        } else {
            // No conflict — create the card directory while still holding the lock
            drop(_lock_guard);
            if let Err(e) = create_card_directory(&source_dir, &current_card_id) {
                show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                return;
            }
            break 'approval_loop;
        }
    }

    // Unregister from registry — card directory now exists on filesystem
    if let Some(dir) = current_source_media_dir.as_deref() {
        registry.lock().unwrap().unregister(transfer_id, dir)
            .expect("unregister: transfer must be registered before unregistering");
    }

    // Step 4: Move the data
    // TODO

    // Step 5: Write the backup log entry
    // TODO
}

/// Determine the initial card ID and register the transfer with the registry.
fn initial_card_id_and_register(
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    source_dir: Option<&std::path::Path>,
    detected_card_id: Option<&str>,
    all_source_media: &[SourceMediaEntry],
) -> Result<String, String> {
    let dir = match source_dir {
        Some(d) => d,
        None => {
            // No source media dir yet — use empty card ID, not registered
            return Ok(String::new());
        }
    };

    let scheme = source_media_scheme(dir, all_source_media);

    let (card_id, pending) = match detected_card_id {
        Some(manual_id) if !manual_id.is_empty() => {
            let scheme_number = if matches!(scheme, CardNamingScheme::Card) {
                crate::transfer_registry::parse_card_number(manual_id)
            } else {
                None
            };
            (manual_id.to_owned(), PendingCardId::Manual { id: manual_id.to_owned(), scheme_number })
        }
        _ => match scheme {
            CardNamingScheme::Card => {
                let reg = registry.lock().unwrap();
                let id = reg.next_card_id(dir, transfer_id)?;
                (id.clone(), PendingCardId::Auto(id))
            }
            CardNamingScheme::Freeform => {
                // Empty until user provides it
                (String::new(), PendingCardId::Manual { id: String::new(), scheme_number: None })
            }
        },
    };

    registry.lock().unwrap().register(transfer_id, dir, pending);
    Ok(card_id)
}

fn handle_device_overwrite(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    current_source_media_dir: &mut Option<PathBuf>,
    current_card_id: &mut String,
    card_id_manually_set: &mut bool,
    current_device_overridden: &mut bool,
    new_dir: PathBuf,
    device_overridden: bool,
    all_source_media: &[SourceMediaEntry],
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    update_tx: &crossbeam_channel::Sender<ui_api::ApproveTransferQueryUpdate>,
    source_device: &str,
    storage_device_overridden: bool,
    notify_rx: &mut crossbeam_channel::Receiver<()>,
) {
    // Determine the card ID to carry into the new source media entry.
    // Manually set IDs are kept as-is; auto IDs are regenerated for the new dir.
    let new_card_id = match source_media_scheme(&new_dir, all_source_media) {
        CardNamingScheme::Card if !*card_id_manually_set => {
            match registry.lock().unwrap().next_card_id(&new_dir, transfer_id) {
                Ok(id) => id,
                Err(e) => {
                    show_card_id_error(ui, None, format!("Failed to compute card ID for new device: {}", e));
                    String::new()
                }
            }
        }
        _ => current_card_id.clone(), // keep existing
    };

    let new_pending_card_id_data = if *card_id_manually_set {
        PendingCardId::Manual {
            id: new_card_id.clone(),
            scheme_number: crate::transfer_registry::parse_card_number(&new_card_id),
        }
    } else {
        PendingCardId::Auto(new_card_id.clone())
    };

    // Move registry entry and re-subscribe to the new dir atomically
    let new_notify_rx = {
        let mut reg = registry.lock().unwrap();
        match current_source_media_dir.as_deref() {
            Some(old) => reg.move_source_media(transfer_id, old, &new_dir, new_pending_card_id_data),
            None      => reg.register(transfer_id, &new_dir, new_pending_card_id_data),
        }
        reg.subscribe(&new_dir)
    };
    *notify_rx = new_notify_rx;

    // Update the caller's data
    *current_source_media_dir = Some(new_dir.clone());
    *current_card_id = new_card_id.clone();
    *current_device_overridden = device_overridden;

    // Update the transfer in the UI
    let _ = transfer_event_tx.send(ui_api::TransferEvent::SourceMediaChanged(
        Some(new_dir.to_string_lossy().into_owned()),
    ));

    // Update the user query in the UI
    let _ = update_tx.send(query_update_from_state(
        &Some(new_dir),
        all_source_media,
        source_device,
        &new_card_id,
        device_overridden,
        storage_device_overridden,
        *card_id_manually_set,
    ));
}

fn handle_card_id_changed(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    current_card_id: &mut String,
    card_id_manually_set: &mut bool,
    source_dir: Option<&std::path::Path>,
    all_source_media: &[SourceMediaEntry],
    new_id: String,
    update_tx: &crossbeam_channel::Sender<ui_api::ApproveTransferQueryUpdate>,
    source_device: &str,
    device_overridden: bool,
    storage_device_overridden: bool,
) { // TODO: check that if the field is now empty but no automatic id can be generated for whatever
    // reason it gets handled correctly and add relevant test case
    let (final_id, pending, is_manual) = if new_id.is_empty() {
        // IF empty revert to auto if scheme supports it
        if let Some(dir) = source_dir {
            if matches!(source_media_scheme(dir, all_source_media), CardNamingScheme::Card) {
                match registry.lock().unwrap().next_card_id(dir, transfer_id) {
                    Ok(auto_id) => {
                        let pending = PendingCardId::Auto(auto_id.clone());
                        (auto_id, pending, false)
                    }
                    Err(e) => {
                        show_card_id_error(ui, None, format!("Failed to revert card ID to auto-generated: {}", e));
                        (new_id.clone(), PendingCardId::Manual { id: new_id.clone(), scheme_number: None }, true)
                    }
                }
            } else {
                (new_id.clone(), PendingCardId::Manual { id: new_id.clone(), scheme_number: None }, true)
            }
        } else {
            (new_id.clone(), PendingCardId::Manual { id: new_id.clone(), scheme_number: None }, true)
        }
    } else {
        let scheme_number = source_dir
            .and_then(|dir| {
                if matches!(source_media_scheme(dir, all_source_media), CardNamingScheme::Card) {
                    crate::transfer_registry::parse_card_number(&new_id)
                } else {
                    None
                }
            });
        (new_id.clone(), PendingCardId::Manual { id: new_id.clone(), scheme_number }, true)
    };

    if let Some(dir) = source_dir {
        registry.lock().unwrap().update_id(transfer_id, dir, pending)
            .expect("update_id: transfer must be registered before updating");
    }

    *current_card_id = final_id.clone();
    *card_id_manually_set = is_manual;

    let current_source_media: Option<PathBuf> = source_dir.map(|d| d.to_owned());
    let _ = update_tx.send(query_update_from_state(
        &current_source_media,
        all_source_media,
        source_device,
        &final_id,
        device_overridden,
        storage_device_overridden,
        is_manual,
    ));
}

fn source_media_scheme(dir: &std::path::Path, all_source_media: &[SourceMediaEntry]) -> CardNamingScheme {
    all_source_media.iter()
        .find(|e| e.directory == dir)
        .map(|e| e.new_card_naming_scheme.clone())
        .unwrap_or(CardNamingScheme::Freeform)
}

fn query_update_from_state(
    source_media_dir: &Option<PathBuf>,
    _all_source_media: &[SourceMediaEntry],
    source_device: &str,
    card_id: &str,
    device_overridden: bool,
    storage_device_overridden: bool,
    card_id_overridden: bool,
) -> ui_api::ApproveTransferQueryUpdate {
    ui_api::ApproveTransferQueryUpdate {
        source_media_dir: source_media_dir.as_ref().map(|p| p.to_string_lossy().into_owned()),
        source_device: source_device.to_owned(),
        data_size: 0,
        card_id: card_id.to_owned(),
        device_overridden,
        storage_device_overridden,
        card_id_overridden,
    }
}

// Show a fatal card ID error dialog and optionally signal device unplugged.
// Call this before unregistering and returning from run_transfer.
fn show_card_id_error(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    transfer_event_tx: Option<&crossbeam_channel::Sender<ui_api::TransferEvent>>,
    message: String,
) {
    let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
    let _ = ui.lock().unwrap().user_query(
        ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
            error: ui_api::FatalErrorKind::CardId(message),
            response_tx,
        }),
        true,
    );
    let _ = response_rx.recv();
    if let Some(tx) = transfer_event_tx {
        let _ = tx.send(ui_api::TransferEvent::DeviceUnplugged);
    }
}

fn storage_device_display_name(device_id: Option<&str>, all_storage_devices: &[crate::StorageDeviceEntry]) -> String {
    device_id
        .and_then(|id| all_storage_devices.iter().find(|d| d.id == id))
        .map(|d| d.display_name.clone())
        .unwrap_or_default()
}

fn subscribe_or_never(
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    dir: Option<&std::path::Path>,
) -> crossbeam_channel::Receiver<()> {
    dir.map(|d| registry.lock().unwrap().subscribe(d))
       .unwrap_or_else(crossbeam_channel::never)
}

fn create_card_directory(source_media_dir: &std::path::Path, card_id: &str) -> Result<(), String> {
    let path = source_media_dir.join("DATA").join(card_id);
    std::fs::create_dir_all(&path)
        .map_err(|e| format!("Failed to create card directory {:?}: {}", path, e))
}
