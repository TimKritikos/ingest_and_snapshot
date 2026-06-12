use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::thread;
use crossbeam_channel;
use crate::ui_api::{self, UiBackend};
use crate::SourceMediaEntry;
use crate::CardNamingScheme;
use crate::transfer_registry::{PendingTransferRegistry, PendingCardId, TransferId};
use crate::mount_manager::MountManager;

/// Sentinel device location meaning "use the local filesystem directly, no block device required".
/// This is always included as a picker option and skips the /dev/disk/by-id/ existence check.
pub const LOCAL_FILESYSTEM_DEVICE_LOCATION: &str = "local-filesystem";

/// Tracks the user-facing input path: the virtual directory from which data will be read.
/// `virtual_path` is relative to the device root (e.g. `PathBuf::from("/DCIM")`).
/// When the source is a block device, `mount_root` is the OS mountpoint that maps "/" to the card root.
/// When `is_frozen` is true the field is locked and neither sub-field should be trusted.
struct InputPathState {
    is_frozen: bool,
    virtual_path: Option<std::path::PathBuf>,
    mount_root: Option<std::path::PathBuf>,
    is_overridden: bool,
}

// Detected info provided at transfer start
pub struct DetectedTransferInfo {
    pub source_media: Option<SourceMediaEntry>, //TODO: this probably should be an Option<String>
    pub card_id: Option<String>,
    pub source_device: Option<String>,
    pub device_location: Option<String>,       // by-id name for display and allow/ignore checks
    pub real_device_path: Option<PathBuf>,     // resolved device node for mounting (e.g. /dev/sdb1)
}

pub fn spawn_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    mount_manager: Arc<Mutex<MountManager>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    all_device_locations: Vec<String>,
    detected: DetectedTransferInfo,
    backup_log_manager: Arc<std::sync::Mutex<crate::backup_log::BackupLogManager>>,
    media_dir: std::path::PathBuf,
) {
    thread::spawn(move || {
        run_transfer(ui, registry, mount_manager, all_source_media, all_storage_devices, all_device_locations, detected, backup_log_manager, media_dir);
    });
}

fn run_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    mount_manager: Arc<Mutex<MountManager>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    all_device_locations: Vec<String>,
    detected: DetectedTransferInfo,
    backup_log_manager: Arc<std::sync::Mutex<crate::backup_log::BackupLogManager>>,
    media_dir: std::path::PathBuf,
) {
    // "Local filesystem" is always a valid device location option regardless of connected hardware
    let all_device_locations = {
        let mut locations = vec![LOCAL_FILESYSTEM_DEVICE_LOCATION.to_owned()];
        locations.extend(all_device_locations);
        locations
    };

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

    let auto_detected_device_location: Option<String> = detected.device_location.clone();
    let mut current_device_location: Option<String> = auto_detected_device_location.clone();
    let mut current_device_location_overridden: bool = false;

    let auto_detected_real_device_path: Option<PathBuf> = detected.real_device_path.clone();
    let mut current_real_device_path: Option<PathBuf> = auto_detected_real_device_path.clone();

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

    // True when a real block device location was detected at spawn time (i.e. a udev-triggered
    // transfer).  In that case the approval dialog fields start frozen and are unfrozen once the
    // device has been successfully mounted.
    let needs_frozen_until_mount = auto_detected_device_location.as_deref()
        .map(|loc| loc != LOCAL_FILESYSTEM_DEVICE_LOCATION)
        .unwrap_or(false);

    // Input path state — tracks the virtual directory from which data will be read.
    let mut input_path_state = InputPathState {
        is_frozen: needs_frozen_until_mount,
        virtual_path: if !needs_frozen_until_mount {
            // Local filesystem transfers start at the root; all other cases start empty.
            match auto_detected_device_location.as_deref() {
                Some(loc) if loc == LOCAL_FILESYSTEM_DEVICE_LOCATION => Some(PathBuf::from("/")),
                _ => None,
            }
        } else {
            None
        },
        mount_root: None,
        is_overridden: false,
    };
    // Receives the OS mountpoint when an async block-device mount completes.
    // Replaced with `never()` once consumed, or when a new mount supersedes the old one.
    let mut mount_result_rx: crossbeam_channel::Receiver<PathBuf> = crossbeam_channel::never();

    'approval_loop: loop {

        // Create approve transfer window

        let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::ApproveTransferResponse>();
        let (update_tx, update_rx)     = crossbeam_channel::unbounded::<ui_api::ApproveTransferQueryUpdate>();

        let show_priority = is_re_approval;
        let initial_data = if !is_re_approval && needs_frozen_until_mount {
            ui_api::ApproveTransferQueryUpdate {
                source_media_dir: ui_api::TransferFieldState::Frozen,
                source_device:    ui_api::TransferFieldState::Frozen,
                data_size:        0,
                card_id:          ui_api::TransferFieldState::Frozen,
                device_location:  ui_api::TransferFieldState::AutoSelected(current_device_location.clone()),
                input_path:       ui_api::TransferFieldState::Frozen,
                input_path_mount_root: None,
            }
        } else {
            query_update_from_state(
                &current_source_media_dir,
                &all_source_media,
                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                &current_card_id,
                current_device_overridden,
                current_storage_device_overridden,
                card_id_manually_set,
                &current_device_location,
                current_device_location_overridden,
                &input_path_state,
            )
        };
        if ui.lock().unwrap().user_query(
            ui_api::UserQuery::ApproveTransfer(ui_api::ApproveTransferQuery {
                initial_data,
                response_tx,
                update_rx,
                has_auto_detected_source_media: detected.source_media.is_some(),
                has_auto_detected_storage_device: auto_detected_device_id.is_some(),
                available_storage_devices: all_storage_devices.clone(),
                has_auto_detected_device_location: auto_detected_device_location.is_some(),
                available_device_locations: all_device_locations.clone(),
            }),
            show_priority,
        ).is_err() {
            if let Some(dir) = current_source_media_dir.as_deref() {
                registry.lock().unwrap().unregister(transfer_id, dir)
                    .expect("unregister: transfer must be registered before unregistering");
            }
            return;
        }

        // Mount the device on the first query submission (not on re-approval loops that only
        // happen when the user goes back from a conflict dialog — location change events below
        // already handle mounts triggered by the user picking a different location).
        if !is_re_approval {
            if let Some(ref location) = current_device_location {
                if location != LOCAL_FILESYSTEM_DEVICE_LOCATION {
                    if let Some(real_path) = current_real_device_path.clone() {
                        // When fields started frozen, unfreeze them once the filesystem is mounted.
                        // A channel is used so the select! loop can react with full current state.
                        let on_mount_success: Option<Box<dyn FnOnce(PathBuf) + Send + 'static>> =
                            if needs_frozen_until_mount {
                                let (tx, rx) = crossbeam_channel::unbounded::<PathBuf>();
                                mount_result_rx = rx;
                                Some(Box::new(move |mountpoint: PathBuf| {
                                    let _ = tx.send(mountpoint);
                                }))
                            } else {
                                None
                            };
                        let _ = crate::mount_manager::start_mount(
                            real_path, location.clone(), transfer_id,
                            Arc::clone(&mount_manager), Arc::clone(&ui),
                            on_mount_success,
                        );
                    }
                }
            }
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
                                    &current_device_location,
                                    current_device_location_overridden,
                                    &input_path_state,
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
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
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
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
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
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
                            ));
                        }
                        Ok(ui_api::ApproveTransferResponse::InputPathChanged(new_virtual_path)) => {
                            input_path_state.is_overridden = true;
                            input_path_state.is_frozen = false;
                            input_path_state.virtual_path = Some(new_virtual_path);
                            let _ = update_tx.send(query_update_from_state(
                                &current_source_media_dir,
                                &all_source_media,
                                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                &current_card_id,
                                current_device_overridden,
                                current_storage_device_overridden,
                                card_id_manually_set,
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
                            ));
                        }
                        Ok(ui_api::ApproveTransferResponse::DeviceLocationChanged(location)) => {
                            if location != LOCAL_FILESYSTEM_DEVICE_LOCATION {
                                let by_id_path = PathBuf::from("/dev/disk/by-id").join(&location);
                                let real_path = std::fs::canonicalize(&by_id_path).ok();
                                current_real_device_path = real_path.clone();
                                if let Some(real_path) = real_path {
                                    if let Some(existing_mountpoint) = crate::mount_manager::get_mountpoint_for_real_device(&real_path, &mount_manager) {
                                        // Already mounted — use it immediately.
                                        input_path_state.is_frozen = false;
                                        input_path_state.virtual_path = Some(PathBuf::from("/"));
                                        input_path_state.mount_root = Some(existing_mountpoint);
                                        input_path_state.is_overridden = false;
                                        mount_result_rx = crossbeam_channel::never();
                                    } else {
                                        // Not yet mounted — start mount and wait for async notification.
                                        let (mount_tx, mount_rx) = crossbeam_channel::unbounded::<PathBuf>();
                                        mount_result_rx = mount_rx;
                                        let _ = crate::mount_manager::start_mount(
                                            real_path, location.clone(), transfer_id,
                                            Arc::clone(&mount_manager), Arc::clone(&ui),
                                            Some(Box::new(move |mountpoint: PathBuf| {
                                                let _ = mount_tx.send(mountpoint);
                                            })),
                                        );
                                        input_path_state.is_frozen = true;
                                        input_path_state.virtual_path = None;
                                        input_path_state.mount_root = None;
                                        input_path_state.is_overridden = false;
                                    }
                                } else {
                                    // Device path could not be resolved — freeze input path.
                                    input_path_state.is_frozen = true;
                                    input_path_state.virtual_path = None;
                                    input_path_state.mount_root = None;
                                    input_path_state.is_overridden = false;
                                    mount_result_rx = crossbeam_channel::never();
                                }
                            } else {
                                current_real_device_path = None;
                                input_path_state.is_frozen = false;
                                input_path_state.virtual_path = Some(PathBuf::from("/"));
                                input_path_state.mount_root = None;
                                input_path_state.is_overridden = false;
                                mount_result_rx = crossbeam_channel::never();
                            }
                            current_device_location = Some(location);
                            current_device_location_overridden = true;
                            let _ = update_tx.send(query_update_from_state(
                                &current_source_media_dir,
                                &all_source_media,
                                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                &current_card_id,
                                current_device_overridden,
                                current_storage_device_overridden,
                                card_id_manually_set,
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
                            ));
                        }
                        Ok(ui_api::ApproveTransferResponse::DeviceLocationAuto) => {
                            current_device_location = auto_detected_device_location.clone();
                            current_device_location_overridden = false;
                            current_real_device_path = auto_detected_real_device_path.clone();
                            match (&current_device_location, &current_real_device_path) {
                                (Some(loc), _) if loc == LOCAL_FILESYSTEM_DEVICE_LOCATION => {
                                    input_path_state.is_frozen = false;
                                    input_path_state.virtual_path = Some(PathBuf::from("/"));
                                    input_path_state.mount_root = None;
                                    input_path_state.is_overridden = false;
                                    mount_result_rx = crossbeam_channel::never();
                                }
                                (Some(location), Some(real_path)) => {
                                    if let Some(existing_mountpoint) = crate::mount_manager::get_mountpoint_for_real_device(real_path, &mount_manager) {
                                        input_path_state.is_frozen = false;
                                        input_path_state.virtual_path = Some(PathBuf::from("/"));
                                        input_path_state.mount_root = Some(existing_mountpoint);
                                        input_path_state.is_overridden = false;
                                        mount_result_rx = crossbeam_channel::never();
                                    } else {
                                        let (mount_tx, mount_rx) = crossbeam_channel::unbounded::<PathBuf>();
                                        mount_result_rx = mount_rx;
                                        let _ = crate::mount_manager::start_mount(
                                            real_path.clone(), location.clone(), transfer_id,
                                            Arc::clone(&mount_manager), Arc::clone(&ui),
                                            Some(Box::new(move |mountpoint: PathBuf| {
                                                let _ = mount_tx.send(mountpoint);
                                            })),
                                        );
                                        input_path_state.is_frozen = true;
                                        input_path_state.virtual_path = None;
                                        input_path_state.mount_root = None;
                                        input_path_state.is_overridden = false;
                                    }
                                }
                                _ => {
                                    // No device location — no path either.
                                    input_path_state.is_frozen = false;
                                    input_path_state.virtual_path = None;
                                    input_path_state.mount_root = None;
                                    input_path_state.is_overridden = false;
                                    mount_result_rx = crossbeam_channel::never();
                                }
                            }
                            let _ = update_tx.send(query_update_from_state(
                                &current_source_media_dir,
                                &all_source_media,
                                &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                                &current_card_id,
                                current_device_overridden,
                                current_storage_device_overridden,
                                card_id_manually_set,
                                &current_device_location,
                                current_device_location_overridden,
                                &input_path_state,
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
                recv(mount_result_rx) -> result => {
                    if let Ok(mountpoint) = result {
                        // Block device mounted — unfreeze input path and send full state update.
                        input_path_state.is_frozen = false;
                        input_path_state.virtual_path = Some(PathBuf::from("/"));
                        input_path_state.mount_root = Some(mountpoint);
                        let _ = update_tx.send(query_update_from_state(
                            &current_source_media_dir,
                            &all_source_media,
                            &storage_device_display_name(current_source_device_id.as_deref(), &all_storage_devices),
                            &current_card_id,
                            current_device_overridden,
                            current_storage_device_overridden,
                            card_id_manually_set,
                            &current_device_location,
                            current_device_location_overridden,
                            &input_path_state,
                        ));
                    }
                    mount_result_rx = crossbeam_channel::never();
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
                                        &current_device_location,
                                        current_device_location_overridden,
                                        &input_path_state,
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

        // Check device location
        match &current_device_location {
            None => {
                let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoDeviceLocationWarningResponse>();
                if ui.lock().unwrap().user_query(
                    ui_api::UserQuery::NoDeviceLocationWarning(ui_api::NoDeviceLocationWarningQuery {
                        reason: ui_api::NoDeviceLocationWarningReason::NoneSelected,
                        response_tx: warn_tx,
                    }),
                    true,
                ).is_err() {
                    registry.lock().unwrap().unregister(transfer_id, &source_dir)
                        .expect("unregister: transfer must be registered before unregistering");
                    return;
                }
                match warn_rx.recv() {
                    Ok(ui_api::NoDeviceLocationWarningResponse::BackToQuery) | Err(_) => {
                        is_re_approval = true;
                        continue 'approval_loop;
                    }
                    Ok(ui_api::NoDeviceLocationWarningResponse::Cancel) => {
                        let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                        registry.lock().unwrap().unregister(transfer_id, &source_dir)
                            .expect("unregister: transfer must be registered before unregistering");
                        return;
                    }
                }
            }
            Some(location) if location != LOCAL_FILESYSTEM_DEVICE_LOCATION => {
                let by_id_path = std::path::Path::new("/dev/disk/by-id").join(location);
                if !by_id_path.exists() {
                    let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoDeviceLocationWarningResponse>();
                    if ui.lock().unwrap().user_query(
                        ui_api::UserQuery::NoDeviceLocationWarning(ui_api::NoDeviceLocationWarningQuery {
                            reason: ui_api::NoDeviceLocationWarningReason::NotFound,
                            response_tx: warn_tx,
                        }),
                        true,
                    ).is_err() {
                        registry.lock().unwrap().unregister(transfer_id, &source_dir)
                            .expect("unregister: transfer must be registered before unregistering");
                        return;
                    }
                    match warn_rx.recv() {
                        Ok(ui_api::NoDeviceLocationWarningResponse::BackToQuery) | Err(_) => {
                            is_re_approval = true;
                            continue 'approval_loop;
                        }
                        Ok(ui_api::NoDeviceLocationWarningResponse::Cancel) => {
                            let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                            registry.lock().unwrap().unregister(transfer_id, &source_dir)
                                .expect("unregister: transfer must be registered before unregistering");
                            return;
                        }
                    }
                }
            }
            Some(_) => {} // LOCAL_FILESYSTEM_DEVICE_LOCATION — no block device check needed
        }

        // Check input path
        if input_path_state.is_frozen || input_path_state.virtual_path.is_none() {
            let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoInputPathWarningResponse>();
            if ui.lock().unwrap().user_query(
                ui_api::UserQuery::NoInputPathWarning(ui_api::NoInputPathWarningQuery {
                    response_tx: warn_tx,
                }),
                true,
            ).is_err() {
                registry.lock().unwrap().unregister(transfer_id, &source_dir)
                    .expect("unregister: transfer must be registered before unregistering");
                return;
            }
            match warn_rx.recv() {
                Ok(ui_api::NoInputPathWarningResponse::BackToQuery) | Err(_) => {
                    is_re_approval = true;
                    continue 'approval_loop;
                }
                Ok(ui_api::NoInputPathWarningResponse::Cancel) => {
                    let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                    registry.lock().unwrap().unregister(transfer_id, &source_dir)
                        .expect("unregister: transfer must be registered before unregistering");
                    return;
                }
            }
        }

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

    // The data move is performed by the system GNU `mv` binary (see move_card_data).
    // Confirm it is GNU coreutils before doing any further work, so we bail out early
    // instead of remounting and then moving with an implementation that may not
    // preserve timestamps.
    if let Err(reason) = ensure_transfer_binary_is_gnu() {
        show_transfer_error(&ui, &transfer_event_tx, reason);
        return;
    }

    // Remount the source block device read-write so data can be deleted after the move.
    // Local-filesystem transfers skip this — no block device is involved.
    if let (Some(location), Some(real_device_path)) = (&current_device_location, &current_real_device_path) {
        if location != LOCAL_FILESYSTEM_DEVICE_LOCATION {
            if let Err(reason) = crate::mount_manager::remount_readwrite(real_device_path, &mount_manager) {
                show_transfer_error(
                    &ui,
                    &transfer_event_tx,
                    format!("Could not remount source device as read-write: {}", reason),
                );
                return;
            }
        }
    }

    // Step 4: Move the data
    // The approval loop only breaks after a source media dir was selected and its card
    // directory was created, so both values are valid here.
    let source_media_dir = current_source_media_dir.clone()
        .expect("approval loop only completes with a source media dir selected");
    let destination_dir = source_media_dir.join("DATA").join(&current_card_id);

    // Step 5: Register the transfer in the backup log now that the card directory exists.
    let card_path_relative = destination_dir.strip_prefix(&media_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| destination_dir.clone());
    if let Err(e) = backup_log_manager.lock().unwrap().add_transfer(
        card_path_relative.clone(),
        current_source_device_id.clone(),
    ) {
        show_transfer_error(&ui, &transfer_event_tx, format!("Failed to update backup log: {}", e));
        return;
    }

    let backup_log_for_samples = Arc::clone(&backup_log_manager);
    let card_path_for_samples  = card_path_relative.clone();
    let mut on_samples = move |samples: &[ui_api::TransferSample]| {
        let log_samples: Vec<crate::backup_log::BackupLogSample> = samples.iter()
            .map(|s| crate::backup_log::BackupLogSample { timestamp_ms: s.timestamp_ms, bytes_done: s.bytes_done })
            .collect();
        let _ = backup_log_for_samples.lock().unwrap()
            .update_transfer_samples(&card_path_for_samples, log_samples);
    };

    let move_result = resolve_source_data_dir(&input_path_state)
        .and_then(|source_data_dir| move_card_data(&source_data_dir, &destination_dir, &transfer_event_tx, &mut on_samples));
    if let Err(error_message) = move_result {
        show_transfer_error(&ui, &transfer_event_tx, error_message);
    }

    // Unmount all filesystems that were mounted for this transfer now that the transfer is done.
    crate::mount_manager::start_unmount_for_transfer(
        transfer_id,
        Arc::clone(&mount_manager),
        Arc::clone(&ui),
    );
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
            (manual_id.to_owned(), PendingCardId::Manual { scheme_number })
        }
        _ => match scheme {
            CardNamingScheme::Card => {
                let reg = registry.lock().unwrap();
                let id = reg.next_card_id(dir, transfer_id)?;
                (id.clone(), PendingCardId::Auto(id))
            }
            CardNamingScheme::Freeform => {
                // Empty until user provides it
                (String::new(), PendingCardId::Manual { scheme_number: None })
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
    device_location: &Option<String>,
    device_location_overridden: bool,
    input_path_state: &InputPathState,
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
        device_location,
        device_location_overridden,
        input_path_state,
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
    device_location: &Option<String>,
    device_location_overridden: bool,
    input_path_state: &InputPathState,
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
                        (new_id.clone(), PendingCardId::Manual { scheme_number: None }, true)
                    }
                }
            } else {
                (new_id.clone(), PendingCardId::Manual { scheme_number: None }, true)
            }
        } else {
            (new_id.clone(), PendingCardId::Manual { scheme_number: None }, true)
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
        (new_id.clone(), PendingCardId::Manual { scheme_number }, true)
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
        device_location,
        device_location_overridden,
        input_path_state,
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
    device_location: &Option<String>,
    device_location_overridden: bool,
    input_path: &InputPathState,
) -> ui_api::ApproveTransferQueryUpdate {
    let source_media_dir_str = source_media_dir.as_ref().map(|p| p.to_string_lossy().into_owned());
    ui_api::ApproveTransferQueryUpdate {
        source_media_dir: if device_overridden {
            ui_api::TransferFieldState::Overridden(source_media_dir_str.unwrap_or_default())
        } else {
            ui_api::TransferFieldState::AutoSelected(source_media_dir_str)
        },
        source_device: if storage_device_overridden {
            ui_api::TransferFieldState::Overridden(source_device.to_owned())
        } else {
            ui_api::TransferFieldState::AutoSelected(if source_device.is_empty() { None } else { Some(source_device.to_owned()) })
        },
        data_size: 0,
        card_id: if card_id_overridden {
            ui_api::TransferFieldState::Overridden(card_id.to_owned())
        } else {
            ui_api::TransferFieldState::AutoSelected(if card_id.is_empty() { None } else { Some(card_id.to_owned()) })
        },
        device_location: if device_location_overridden {
            ui_api::TransferFieldState::Overridden(device_location.clone().unwrap_or_default())
        } else {
            ui_api::TransferFieldState::AutoSelected(device_location.clone())
        },
        input_path: if input_path.is_frozen {
            ui_api::TransferFieldState::Frozen
        } else if input_path.is_overridden {
            ui_api::TransferFieldState::Overridden(
                input_path.virtual_path.clone().unwrap_or_else(|| PathBuf::from("/")),
            )
        } else {
            ui_api::TransferFieldState::AutoSelected(input_path.virtual_path.clone())
        },
        input_path_mount_root: input_path.mount_root.clone(),
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

/// Interval between transfer progress samples sent to the UI transfer graph.
const TRANSFER_SAMPLE_INTERVAL: std::time::Duration = std::time::Duration::from_millis(500);

/// The system binary used to transfer card data (see [`move_card_data`]).
/// With the `copy-instead-of-move` feature this is `cp` (for debugging); otherwise `mv`.
#[cfg(not(feature = "copy-instead-of-move"))]
const TRANSFER_BINARY: &str = "mv";
#[cfg(feature = "copy-instead-of-move")]
const TRANSFER_BINARY: &str = "cp";

/// Marker string present in `mv --version` output for GNU coreutils' implementation.
const GNU_COREUTILS_VERSION_MARKER: &str = "GNU coreutils";

/// Confirm that the system transfer binary (`mv` or `cp`) is GNU coreutils' implementation.
///
/// Both `mv` and `cp` from GNU coreutils preserve access and modification timestamps;
/// other implementations (e.g. busybox) make no such guarantee, so we refuse to run a
/// transfer with them rather than silently lose the original timestamps.
fn ensure_transfer_binary_is_gnu() -> Result<(), String> {
    let version_output = std::process::Command::new(TRANSFER_BINARY)
        .arg("--version")
        .output()
        .map_err(|e| format!("Could not run `{} --version`: {}", TRANSFER_BINARY, e))?;

    if !version_output.status.success() {
        return Err(format!(
            "`{} --version` exited with a failure status; cannot confirm it is GNU coreutils",
            TRANSFER_BINARY,
        ));
    }

    let version_text = String::from_utf8_lossy(&version_output.stdout);
    if version_text.contains(GNU_COREUTILS_VERSION_MARKER) {
        Ok(())
    } else {
        let reported_version = version_text.lines().next().unwrap_or("<no output>");
        Err(format!(
            "System `{}` is not GNU coreutils, so access/modification timestamps would not be \
             preserved. `--version` reported: {}",
            TRANSFER_BINARY, reported_version,
        ))
    }
}

/// Milliseconds since the UNIX epoch, used to timestamp transfer graph samples.
fn current_time_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// Resolves the actual directory the data will be read from: the OS mountpoint of the
/// source block device joined with the user-facing virtual input path, or the virtual
/// path itself for local-filesystem transfers (where it already is an actual path).
fn resolve_source_data_dir(input_path_state: &InputPathState) -> Result<PathBuf, String> {
    let virtual_path = input_path_state.virtual_path.as_ref()
        .ok_or_else(|| "No input path was selected".to_owned())?;
    match &input_path_state.mount_root {
        Some(mount_root) => {
            // The virtual path is absolute relative to the card root ("/DCIM"), so the
            // leading "/" must be dropped before joining it onto the mountpoint.
            let path_within_device = virtual_path.strip_prefix("/").unwrap_or(virtual_path);
            Ok(mount_root.join(path_within_device))
        }
        None => Ok(virtual_path.clone()),
    }
}

/// Total size in bytes of all regular files under `dir`, recursively.
/// Symlinks are not followed, matching what a non-dereferencing copy transfers.
fn directory_size_bytes(dir: &std::path::Path) -> std::io::Result<u64> {
    let mut total_bytes: u64 = 0;
    let mut pending_dirs = vec![dir.to_path_buf()];
    while let Some(current_dir) = pending_dirs.pop() {
        for entry in std::fs::read_dir(&current_dir)? {
            let entry = entry?;
            let metadata = entry.metadata()?; // does not follow symlinks
            if metadata.is_dir() {
                pending_dirs.push(entry.path());
            } else if metadata.is_file() {
                total_bytes += metadata.len();
            }
        }
    }
    Ok(total_bytes)
}

/// Moves the source data at `source_path` into the (already-created) card directory
/// `destination_dir` by invoking the system GNU `mv` binary, while periodically
/// reporting progress to the UI transfer graph through `transfer_event_tx`.
///
/// `source_path` may be either a directory, whose *contents* are moved into the card
/// directory (like `mv source/* dest/`, so the directory itself does not become a
/// subdirectory), or a single file, which is moved into the card directory as-is.
///
/// The move is delegated to GNU `mv` (rather than a Rust reimplementation) because
/// the card and the destination are typically on different filesystems, which forces
/// `mv` to fall back to copy-and-delete. GNU `mv` preserves the original access and
/// modification timestamps (as well as mode and ownership) across that fallback;
/// uutils' `uu_mv` does not. Callers must have already confirmed the system `mv` is
/// GNU coreutils via [`ensure_system_mv_is_gnu`].
///
/// The change timestamp (ctime) is intentionally not preserved: Linux offers no way
/// to set it, as the kernel updates it on every inode modification.
fn move_card_data(
    source_path: &std::path::Path,
    destination_dir: &std::path::Path,
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    on_samples: &mut impl FnMut(&[ui_api::TransferSample]),
) -> Result<(), String> {
    // metadata() follows symlinks, so a symlink pointing at a directory is still treated
    // as a directory whose contents are moved.
    let source_metadata = std::fs::metadata(source_path)
        .map_err(|e| format!("Failed to inspect source path {:?}: {}", source_path, e))?;

    // A directory move transfers its contents; a file move transfers the file itself.
    let (source_entries, bytes_total): (Vec<PathBuf>, u64) = if source_metadata.is_dir() {
        let bytes_total = directory_size_bytes(source_path)
            .map_err(|e| format!("Failed to measure size of source data at {:?}: {}", source_path, e))?;
        let directory_entries: Vec<PathBuf> = std::fs::read_dir(source_path)
            .and_then(|entries| {
                entries.map(|entry| entry.map(|e| e.path())).collect()
            })
            .map_err(|e| format!("Failed to list source directory {:?}: {}", source_path, e))?;
        (directory_entries, bytes_total)
    } else {
        (vec![source_path.to_path_buf()], source_metadata.len())
    };

    let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferStarted { bytes_total });

    if source_entries.is_empty() {
        let samples = vec![ui_api::TransferSample { timestamp_ms: current_time_ms(), bytes_done: bytes_total }];
        on_samples(&samples);
        let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferSamples(samples));
        return Ok(());
    }

    // The transfer binary blocks until it is done and offers no progress callback, so it
    // runs on a worker thread while this thread samples the destination size.
    // `--target-directory` names the target directory and `--` stops option parsing so
    // source paths beginning with a dash are never mistaken for flags.
    // When copying, `--recursive` handles subdirectories and `--preserve=timestamps`
    // keeps the original access and modification times.
    let (move_result_tx, move_result_rx) = crossbeam_channel::bounded::<Result<(), String>>(1);
    let move_destination = destination_dir.to_owned();
    thread::spawn(move || {
        let mut cmd = std::process::Command::new(TRANSFER_BINARY);
        #[cfg(feature = "copy-instead-of-move")]
        cmd.arg("--recursive").arg("--preserve=timestamps");
        let move_command_output = cmd
            .arg("--target-directory")
            .arg(&move_destination)
            .arg("--")
            .args(&source_entries)
            .output();
        let result = match move_command_output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            Err(spawn_error) => Err(format!("Could not run `{}`: {}", TRANSFER_BINARY, spawn_error)),
        };
        let _ = move_result_tx.send(result);
    });

    loop {
        crossbeam_channel::select! {
            recv(move_result_rx) -> result => {
                return match result {
                    Ok(Ok(())) => {
                        let samples = vec![ui_api::TransferSample { timestamp_ms: current_time_ms(), bytes_done: bytes_total }];
                        on_samples(&samples);
                        let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferSamples(samples));
                        Ok(())
                    }
                    Ok(Err(message)) => Err(format!("Failed to move data to {:?}: {}", destination_dir, message)),
                    Err(_) => Err("Move thread exited without reporting a result".to_owned()),
                };
            }
            default(TRANSFER_SAMPLE_INTERVAL) => {
                if let Ok(bytes_done) = directory_size_bytes(destination_dir) {
                    let samples = vec![ui_api::TransferSample { timestamp_ms: current_time_ms(), bytes_done }];
                    on_samples(&samples);
                    let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferSamples(samples));
                }
            }
        }
    }
}

// Show a fatal transfer error dialog and remove the transfer from the UI.
fn show_transfer_error(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    message: String,
) {
    let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
    let _ = ui.lock().unwrap().user_query(
        ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
            error: ui_api::FatalErrorKind::Transfer(message),
            response_tx,
        }),
        true,
    );
    let _ = response_rx.recv();
    let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
}
