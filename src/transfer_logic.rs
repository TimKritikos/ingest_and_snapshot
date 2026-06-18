use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use crate::ui_api::{self, UiBackend};
use crate::SourceMediaEntry;
use crate::CardNamingScheme;
use crate::transfer_registry::{PendingTransferRegistry, PendingCardId, TransferId};
use crate::mount_manager::MountManager;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Sentinel device location meaning "use the local filesystem directly, no block device required".
/// This is always included as a picker option and skips the /dev/disk/by-id/ existence check.
pub const LOCAL_FILESYSTEM_DEVICE_LOCATION: &str = "local-filesystem";

/// A single progress sample recorded during a data transfer.
#[derive(Serialize, Deserialize, Clone)]
pub struct TransferSample {
    pub timestamp_ms: u64,
    pub bytes_done: u64,
}

/// Describes the state of a single transfer field relative to its auto-detected value.
///
/// The auto-detected value (if any) lives alongside this in the `*_detected` field of
/// [`TransferFields`]; this enum only records the user's *choice*. Use [`TransferFieldState::resolve`]
/// to obtain the effective value by combining the choice with the detected value.
#[derive(Clone)]
pub enum TransferFieldState<T> {
    /// Use the auto-detected value. Only meaningful when a detected value exists.
    AutoSelected,
    /// The user has manually set the field to this value.
    Overridden(T),
    /// Nothing was auto-detected and the user has not chosen anything yet.
    NotSelected,
}

impl<T> TransferFieldState<T> {
    /// The effective value: the override if present, otherwise the detected value.
    /// `NotSelected` always resolves to `None`.
    pub fn resolve<'a>(&'a self, detected: Option<&'a T>) -> Option<&'a T> {
        match self {
            TransferFieldState::Overridden(value) => Some(value),
            TransferFieldState::AutoSelected      => detected,
            TransferFieldState::NotSelected       => None,
        }
    }

    pub fn is_overridden(&self) -> bool {
        matches!(self, TransferFieldState::Overridden(_))
    }
}

pub enum TransferResult {
    Succeeded,
    Failed(String),
}

type SourceMediaId  = PathBuf;           // the relative path from media
type CardId         = String;            // the format depends on what the source media config specifies
type StorageId      = Uuid;              // uuidv7 that should exist on the device list
/// An absolute path to the real device and the name it has on /dev/disk/by-id.
pub type DeviceLocation = (PathBuf, String);
type InputPath      = PathBuf;           // An absolute path but taken from the root of the mountpoint
type Comment        = String;            // free-form note the user can optionally attach to a transfer

/// The user-facing selection state of a transfer: every field that the approval dialog shows or
/// lets the user override. Each field is a `*_detected` auto value plus a `*_selected` choice.
///
/// This is the slice of [`TransferEntry`] that is sent to the UI, so it deliberately excludes
/// outcome/telemetry data. The UI resolves each field with [`TransferFieldState::resolve`] and
/// derives "is an auto value available?" from whether the matching `*_detected` is `Some`.
#[derive(Clone)]
pub struct TransferFields {
    pub card_id_detected:         Option<CardId>,
    pub card_id_selected:         TransferFieldState<CardId>,
    pub source_media_detected:    Option<SourceMediaId>,
    pub source_media_selected:    TransferFieldState<SourceMediaId>,
    pub storage_device_detected:  Option<StorageId>,
    pub storage_device_selected:  TransferFieldState<StorageId>,
    pub device_location_detected: Option<DeviceLocation>,
    pub device_location_selected: TransferFieldState<DeviceLocation>,
    pub input_path_detected:      Option<InputPath>,
    pub input_path_selected:      TransferFieldState<InputPath>,
    /// Optional free-form comment the user can attach to the transfer. Unlike the other fields it
    /// has no auto-detected value and no override tracking — it is simply present or absent.
    pub comment: Option<Comment>,
    /// Actual OS mountpoint of the source block device, if one is mounted.
    /// `None` for local-filesystem transfers (where the virtual input path IS the actual path).
    pub mount_root: Option<PathBuf>,
}

impl TransferFields {
    pub fn card_id(&self)         -> Option<&CardId>         { self.card_id_selected.resolve(self.card_id_detected.as_ref()) }
    pub fn source_media(&self)    -> Option<&SourceMediaId>  { self.source_media_selected.resolve(self.source_media_detected.as_ref()) }
    pub fn storage_device(&self)  -> Option<&StorageId>      { self.storage_device_selected.resolve(self.storage_device_detected.as_ref()) }
    pub fn device_location(&self) -> Option<&DeviceLocation> { self.device_location_selected.resolve(self.device_location_detected.as_ref()) }
    pub fn input_path(&self)      -> Option<&InputPath>      { self.input_path_selected.resolve(self.input_path_detected.as_ref()) }

    /// The by-id name of the selected device location (e.g. `usb-Foo-0:0` or the local-filesystem sentinel).
    pub fn device_location_name(&self) -> Option<&str> {
        self.device_location().map(|(_, name)| name.as_str())
    }

    /// The absolute device node of the selected location, but only when it is a real block device
    /// (i.e. not the local-filesystem sentinel). `None` for local-filesystem or no selection.
    fn real_device_path(&self) -> Option<&Path> {
        match self.device_location() {
            Some((path, name)) if name != LOCAL_FILESYSTEM_DEVICE_LOCATION => Some(path.as_path()),
            _ => None,
        }
    }

    /// The effective card id as an owned string, or empty when none is selected.
    fn card_id_string(&self) -> String {
        self.card_id().cloned().unwrap_or_default()
    }

    /// Absolute filesystem path of the selected source media directory, built on demand from
    /// `media_dir`. The source media is stored as a relative id; this is the only place an
    /// absolute path is formed for registry/filesystem use.
    fn source_media_dir_abs(&self, media_dir: &Path) -> Option<PathBuf> {
        self.source_media().map(|relative| media_dir.join(relative))
    }

    /// Records an auto-generated card id: the value becomes the detected default and the choice
    /// reverts to auto.
    fn set_card_id_auto(&mut self, id: CardId) {
        self.card_id_detected = Some(id);
        self.card_id_selected = TransferFieldState::AutoSelected;
    }

    /// Records a manually-entered card id.
    fn set_card_id_manual(&mut self, id: CardId) {
        self.card_id_selected = TransferFieldState::Overridden(id);
    }
}

/// All state for a single in-flight transfer. Helper functions take `&mut TransferEntry` instead
/// of a long list of individual parameters, and `BackupLogManager::add_transfer` consumes one
/// to record the transfer (so the persistence layer owns the "prepare for writing" mapping).
#[allow(dead_code)] // some record fields are kept for the on-disk record / a future summary view
pub struct TransferEntry {
    /// Unique id for the transfer, used as the backup-log transfer id.
    pub transfer_uuidv7: String,
    /// The fields shown in (and editable from) the approval dialog.
    pub fields: TransferFields,
    /// Hostname recorded in the backup log for this transfer.
    pub system_hostname: String,

    // --- Outcome data, recorded as the transfer runs (also persisted to the backup log) ---
    /// Card directory path relative to `media_dir`; set once the destination is known.
    pub card_path: Option<PathBuf>,
    pub bytes_total_measured: Option<u64>,
    pub transfer_samples: Option<Vec<TransferSample>>,
    pub transfer_result: Option<TransferResult>,
}

// Detected info provided at transfer start.
pub struct DetectedTransferInfo {
    pub source_media: Option<SourceMediaId>,
    pub card_id: Option<String>,
    pub source_device: Option<StorageId>,
    /// Absolute device node paired with its /dev/disk/by-id name, or the local-filesystem sentinel.
    pub device_location: Option<DeviceLocation>,
    pub input_path: Option<InputPath>,
}

/// Picks the initial choice for a field: use the auto value if one was detected, otherwise nothing.
fn initial_field_state<T>(detected: &Option<T>) -> TransferFieldState<T> {
    if detected.is_some() {
        TransferFieldState::AutoSelected
    } else {
        TransferFieldState::NotSelected
    }
}

#[allow(clippy::too_many_arguments)]
pub fn spawn_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    mount_manager: Arc<Mutex<MountManager>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    detected: DetectedTransferInfo,
    backup_log_manager: Arc<std::sync::Mutex<crate::backup_log::BackupLogManager>>,
    media_dir: std::path::PathBuf,
    system_hostname: String,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        run_transfer(ui, registry, mount_manager, all_source_media, all_storage_devices, detected, backup_log_manager, media_dir, system_hostname);
    })
}

#[allow(clippy::too_many_arguments)]
fn run_transfer(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    registry: Arc<Mutex<PendingTransferRegistry>>,
    mount_manager: Arc<Mutex<MountManager>>,
    all_source_media: Vec<SourceMediaEntry>,
    all_storage_devices: Vec<crate::StorageDeviceEntry>,
    detected: DetectedTransferInfo,
    backup_log_manager: Arc<std::sync::Mutex<crate::backup_log::BackupLogManager>>,
    media_dir: std::path::PathBuf,
    system_hostname: String,
) {
    // The input path defaults to the card root ("/") when a block device was auto-detected but no
    // explicit input path was supplied.
    let detected_input_path: Option<InputPath> = detected.input_path.clone()
        .or_else(|| detected.device_location.as_ref().map(|_| PathBuf::from("/")));

    let mut transfer_data = TransferEntry {
        transfer_uuidv7: Uuid::now_v7().to_string(),
        fields: TransferFields {
            card_id_detected:         None,
            source_media_detected:    detected.source_media.clone(),
            storage_device_detected:  detected.source_device,
            device_location_detected: detected.device_location.clone(),
            input_path_detected:      detected_input_path.clone(),

            card_id_selected:         initial_field_state(&detected.card_id),
            source_media_selected:    initial_field_state(&detected.source_media),
            storage_device_selected:  initial_field_state(&detected.source_device),
            device_location_selected: initial_field_state(&detected.device_location),
            input_path_selected:      initial_field_state(&detected_input_path),

            comment:                  None,

            mount_root:               None,
        },
        system_hostname,
        card_path:            None,
        bytes_total_measured: None,
        transfer_samples:     None,
        transfer_result:      None,
    };

    // Assign a unique internal ID for this transfer in the registry. TODO: maybe use the transfer uuidv7
    let transfer_id: TransferId = registry.lock().unwrap().new_transfer_internal_id();

    // Compute the initial card ID and register with the registry.
    let initial_source_dir = transfer_data.fields.source_media_dir_abs(&media_dir);
    let initial_card_id = match initial_card_id_and_register(
        &registry,
        transfer_id,
        initial_source_dir.as_deref(),
        detected.card_id.as_deref(),
        &all_source_media,
        &media_dir,
    ) {
        Ok(id) => id,
        Err(e) => {
            show_card_id_error(&ui, None, format!("Failed to compute initial card ID: {}", e));
            return;
        }
    };
    if detected.card_id.as_deref().is_some_and(|id| !id.is_empty()) {
        transfer_data.fields.set_card_id_manual(initial_card_id);
    } else if !initial_card_id.is_empty() {
        transfer_data.fields.set_card_id_auto(initial_card_id);
    }
    // else: freeform with nothing supplied yet — card_id_selected stays NotSelected.

    // Register the transfer in the UI.
    let (transfer_event_tx, transfer_event_rx) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
    if ui.lock().unwrap().new_transfer(
        transfer_data.fields.source_media().map(|p| p.to_string_lossy().into_owned()),
        transfer_event_rx,
    ).is_err() {
        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
        return;
    }

    // Subscribe to registry change notifications for the cardid of our source media dir.
    // `never()` is used when there is no source media dir — an inert branch that never fires.
    let mut notify_rx = subscribe_or_never(&registry, transfer_data.fields.source_media_dir_abs(&media_dir).as_deref());

    let mut is_re_approval = false; // becomes true after BackToQuery loops back

    // Resolve the mount root — the device is already mounted when this transfer starts.
    let initial_mount_root = transfer_data.fields.real_device_path()
        .and_then(|rp| crate::mount_manager::get_mountpoint_for_real_device(rp, &mount_manager));
    transfer_data.fields.mount_root = initial_mount_root;
    if let (Some(rp), Some(_)) = (transfer_data.fields.real_device_path(), &transfer_data.fields.mount_root) {
        crate::mount_manager::register_mount_user(rp, transfer_id, &mount_manager);
    }

    'approval_loop: loop {

        // Create approve transfer window

        let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::ApproveTransferResponse>();
        let (update_tx, update_rx)     = crossbeam_channel::unbounded::<TransferFields>();

        let show_priority = is_re_approval;
        if ui.lock().unwrap().user_query(
            ui_api::UserQuery::ApproveTransfer(Box::new(ui_api::ApproveTransferQuery {
                fields: transfer_data.fields.clone(),
                response_tx,
                update_rx,
                available_storage_devices: all_storage_devices.clone(),
                available_device_locations: {
                    let mut locations = crate::mount_manager::get_mounted_device_locations(&mount_manager);
                    locations.insert(0, LOCAL_FILESYSTEM_DEVICE_LOCATION.to_owned());
                    locations
                },
            })),
            show_priority,
        ).is_err() {
            unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
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
                            let (new_dir, device_overridden): (Option<SourceMediaId>, bool) = match selection {
                                ui_api::SourceMediaSelection::Auto => (
                                    transfer_data.fields.source_media_detected.clone(),
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
                                    &mut transfer_data,
                                    new_dir,
                                    device_overridden,
                                    &all_source_media,
                                    &transfer_event_tx,
                                    &update_tx,
                                    &mut notify_rx,
                                    &media_dir,
                                );
                            }
                        }
                        Ok(ui_api::ApproveTransferResponse::CardIdChanged(new_id)) => {
                            handle_card_id_changed(
                                &ui,
                                &registry,
                                transfer_id,
                                &mut transfer_data,
                                &all_source_media,
                                new_id,
                                &update_tx,
                                &media_dir,
                            );
                        }
                        Ok(ui_api::ApproveTransferResponse::StorageDeviceChanged(device_id)) => {
                            transfer_data.fields.storage_device_selected = TransferFieldState::Overridden(device_id);
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Ok(ui_api::ApproveTransferResponse::StorageDeviceAuto) => {
                            transfer_data.fields.storage_device_selected = TransferFieldState::AutoSelected;
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Ok(ui_api::ApproveTransferResponse::InputPathChanged(new_virtual_path)) => {
                            transfer_data.fields.input_path_selected = TransferFieldState::Overridden(new_virtual_path);
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Ok(ui_api::ApproveTransferResponse::CommentChanged(new_comment)) => {
                            // An empty comment clears the field rather than storing an empty string.
                            transfer_data.fields.comment =
                                if new_comment.is_empty() { None } else { Some(new_comment) };
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Ok(ui_api::ApproveTransferResponse::DeviceLocationChanged(name)) => {
                            let real_path = if name != LOCAL_FILESYSTEM_DEVICE_LOCATION {
                                let by_id_path = Path::new("/dev/disk/by-id").join(&name);
                                std::fs::canonicalize(&by_id_path).unwrap_or_default()
                            } else {
                                PathBuf::new()
                            };
                            transfer_data.fields.device_location_selected =
                                TransferFieldState::Overridden((real_path, name));
                            let new_mount_root = transfer_data.fields.real_device_path()
                                .and_then(|rp| crate::mount_manager::get_mountpoint_for_real_device(rp, &mount_manager));
                            if let (Some(rp), Some(_)) = (transfer_data.fields.real_device_path(), &new_mount_root) {
                                crate::mount_manager::register_mount_user(rp, transfer_id, &mount_manager);
                            }
                            reset_input_path_to_card_root_if_auto(&mut transfer_data);
                            transfer_data.fields.mount_root = new_mount_root;
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Ok(ui_api::ApproveTransferResponse::DeviceLocationAuto) => {
                            transfer_data.fields.device_location_selected = TransferFieldState::AutoSelected;
                            let new_mount_root = transfer_data.fields.real_device_path()
                                .and_then(|rp| crate::mount_manager::get_mountpoint_for_real_device(rp, &mount_manager));
                            if let (Some(rp), Some(_)) = (transfer_data.fields.real_device_path(), &new_mount_root) {
                                crate::mount_manager::register_mount_user(rp, transfer_id, &mount_manager);
                            }
                            reset_input_path_to_card_root_if_auto(&mut transfer_data);
                            transfer_data.fields.mount_root = new_mount_root;
                            let _ = update_tx.send(transfer_data.fields.clone());
                        }
                        Err(_) => {
                            unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                            return;
                        }
                    }
                }
                recv(notify_rx) -> result => {
                    if result.is_err() {
                        // Sender dropped — replace with a never-receiver to avoid a busy loop
                        notify_rx = crossbeam_channel::never();
                    } else if let Some(dir) = transfer_data.fields.source_media_dir_abs(&media_dir) {
                        if matches!(source_media_scheme(&dir, &all_source_media, &media_dir), CardNamingScheme::CardFourDigits)
                            && !transfer_data.fields.card_id_selected.is_overridden()
                        {
                            let next_card_id_result = registry.lock().unwrap().next_card_id(&dir, transfer_id);
                            match next_card_id_result {
                                Ok(new_id) if transfer_data.fields.card_id() != Some(&new_id) => {
                                    transfer_data.fields.set_card_id_auto(new_id.clone());
                                    registry.lock().unwrap().update_id(
                                        transfer_id,
                                        &dir,
                                        PendingCardId::Auto(new_id),
                                    ).expect("update_id: transfer must be registered before updating");
                                    let _ = update_tx.send(transfer_data.fields.clone());
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
            unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
            return;
        }

        // User approved — acquire the approval lock and do TOCTOU-safe conflict check
        let source_dir = match transfer_data.fields.source_media_dir_abs(&media_dir) {
            Some(dir) => dir,
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
        match transfer_data.fields.device_location() {
            None => {
                let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoDeviceLocationWarningResponse>();
                if ui.lock().unwrap().user_query(
                    ui_api::UserQuery::NoDeviceLocationWarning(ui_api::NoDeviceLocationWarningQuery {
                        reason: ui_api::NoDeviceLocationWarningReason::NoneSelected,
                        response_tx: warn_tx,
                    }),
                    true,
                ).is_err() {
                    unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                    return;
                }
                match warn_rx.recv() {
                    Ok(ui_api::NoDeviceLocationWarningResponse::BackToQuery) | Err(_) => {
                        is_re_approval = true;
                        continue 'approval_loop;
                    }
                    Ok(ui_api::NoDeviceLocationWarningResponse::Cancel) => {
                        let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                        return;
                    }
                }
            }
            Some((_, name)) if name != LOCAL_FILESYSTEM_DEVICE_LOCATION => {
                let by_id_path = std::path::Path::new("/dev/disk/by-id").join(name);
                if !by_id_path.exists() {
                    let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoDeviceLocationWarningResponse>();
                    if ui.lock().unwrap().user_query(
                        ui_api::UserQuery::NoDeviceLocationWarning(ui_api::NoDeviceLocationWarningQuery {
                            reason: ui_api::NoDeviceLocationWarningReason::NotFound,
                            response_tx: warn_tx,
                        }),
                        true,
                    ).is_err() {
                        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                        return;
                    }
                    match warn_rx.recv() {
                        Ok(ui_api::NoDeviceLocationWarningResponse::BackToQuery) | Err(_) => {
                            is_re_approval = true;
                            continue 'approval_loop;
                        }
                        Ok(ui_api::NoDeviceLocationWarningResponse::Cancel) => {
                            let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                            unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                            return;
                        }
                    }
                }
            }
            Some(_) => {} // LOCAL_FILESYSTEM_DEVICE_LOCATION — no block device check needed
        }

        // Check input path
        if transfer_data.fields.input_path().is_none() {
            let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::NoInputPathWarningResponse>();
            if ui.lock().unwrap().user_query(
                ui_api::UserQuery::NoInputPathWarning(ui_api::NoInputPathWarningQuery {
                    response_tx: warn_tx,
                }),
                true,
            ).is_err() {
                unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                return;
            }
            match warn_rx.recv() {
                Ok(ui_api::NoInputPathWarningResponse::BackToQuery) | Err(_) => {
                    is_re_approval = true;
                    continue 'approval_loop;
                }
                Ok(ui_api::NoInputPathWarningResponse::Cancel) => {
                    let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                    unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                    return;
                }
            }
        }

        let scheme = source_media_scheme(&source_dir, &all_source_media, &media_dir);
        let current_card_id = transfer_data.fields.card_id_string();
        let card_id_manually_set = transfer_data.fields.card_id_selected.is_overridden();

        // For freeform: no sequential check; only check if the ID is taken
        let approval_lock = registry.lock().unwrap().get_approval_lock(&source_dir);
        let _lock_guard = approval_lock.as_ref().map(|l| l.lock().unwrap());

        // Check if this card ID is taken on the filesystem
        let is_taken = match PendingTransferRegistry::is_card_id_taken(&source_dir, &current_card_id) {
            Ok(v) => v,
            Err(e) => {
                show_card_id_error(&ui, Some(&transfer_event_tx), format!("Error checking card ID: {}", e));
                unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                return;
            }
        };

        // For Card scheme: also check if there would be a sequence gap
        let sequence_conflict = if matches!(scheme, CardNamingScheme::CardFourDigits) && !is_taken {
            let next_card_id_result = registry.lock().unwrap().next_card_id(&source_dir, transfer_id);
            match next_card_id_result {
                Ok(next_sequential) => next_sequential != current_card_id,
                Err(e) => {
                    show_card_id_error(&ui, Some(&transfer_event_tx), format!("Error computing next card ID: {}", e));
                    unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
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
            let suggested_id = if matches!(scheme, CardNamingScheme::CardFourDigits) {
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
                unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                return;
            }

            match conflict_rx.recv() {
                Ok(ui_api::ConfirmCardIdResponse::UseNew) => {
                    if let Some(new_id) = suggested_id {
                        transfer_data.fields.set_card_id_auto(new_id.clone());
                        registry.lock().unwrap().update_id(
                            transfer_id,
                            &source_dir,
                            PendingCardId::Auto(new_id),
                        ).expect("update_id: transfer must be registered before updating");
                    }
                    // Drop the lock, create the directory, then break
                    drop(_lock_guard);
                    if let Err(e) = create_card_directory(&source_dir, &transfer_data.fields.card_id_string()) {
                        show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                        return;
                    }
                    break 'approval_loop;
                }
                Ok(ui_api::ConfirmCardIdResponse::UseOriginal) => {
                    // Lock held — original is still free (gap case only), create it
                    drop(_lock_guard);
                    if let Err(e) = create_card_directory(&source_dir, &current_card_id) {
                        show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
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
            drop(_lock_guard);

            // Reject a card ID that already has a transfer entry in the backup log.
            // This catches the case where a card directory was deleted and its ID
            // reused: without this check, update_transfer_samples would silently write
            // the new transfer's samples into the old log entry (find() returns the
            // first match), leaving the new entry with no recorded samples.
            let would_be_dest = source_dir.join("DATA").join(&current_card_id);
            let would_be_card_path = would_be_dest.strip_prefix(&media_dir)
                .map(|p| p.to_path_buf())
                .unwrap_or_else(|_| would_be_dest.clone());
            if backup_log_manager.lock().unwrap().has_transfer_for_card_path(&would_be_card_path) {
                let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::CardIdInLogWarningResponse>();
                if ui.lock().unwrap().user_query(
                    ui_api::UserQuery::CardIdInLogWarning(ui_api::CardIdInLogWarningQuery {
                        card_id: current_card_id.clone(),
                        response_tx: warn_tx,
                    }),
                    true,
                ).is_err() {
                    unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                    return;
                }
                match warn_rx.recv() {
                    Ok(ui_api::CardIdInLogWarningResponse::BackToQuery) | Err(_) => {
                        is_re_approval = true;
                        continue 'approval_loop;
                    }
                    Ok(ui_api::CardIdInLogWarningResponse::Cancel) => {
                        let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                        unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                        return;
                    }
                }
            }

            // No conflict — create the card directory
            if let Err(e) = create_card_directory(&source_dir, &current_card_id) {
                show_card_id_error(&ui, Some(&transfer_event_tx), format!("Failed to create card directory: {}", e));
                unregister_current(&registry, transfer_id, &transfer_data, &media_dir);
                return;
            }
            break 'approval_loop;
        }
    }

    // Unregister from registry — card directory now exists on filesystem
    unregister_current(&registry, transfer_id, &transfer_data, &media_dir);

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
    if let Some(real_device_path) = transfer_data.fields.real_device_path() {
        if let Err(reason) = crate::mount_manager::remount_readwrite(real_device_path, &mount_manager) {
            show_transfer_error(
                &ui,
                &transfer_event_tx,
                format!("Could not remount source device as read-write: {}", reason),
            );
            return;
        }
    }

    // Step 4: Decide what will be moved and measure its total size.
    // The approval loop only breaks after a source media dir was selected and its card
    // directory was created, so both values are valid here.
    let source_media_dir = transfer_data.fields.source_media_dir_abs(&media_dir)
        .expect("approval loop only completes with a source media dir selected");
    let current_card_id = transfer_data.fields.card_id_string();
    let destination_dir = source_media_dir.join("DATA").join(&current_card_id);

    // Plan the move (which paths get transferred and their total size) before the transfer is
    // recorded, so the authoritative total size can be written to the backup log up front —
    // before the copy begins. The move itself only adds the samples and the final result.
    let source_data_dir_result = resolve_source_data_dir(&transfer_data.fields);
    let excluded_names: &[&str] = match (source_data_dir_result.as_ref(), transfer_data.fields.mount_root.as_ref()) {
        (Ok(dir), Some(root)) if dir == root => &[crate::per_device_config::CONFIG_FILE_NAME],
        _ => &[],
    };
    let move_plan_result = source_data_dir_result
        .and_then(|source_data_dir| plan_card_move(&source_data_dir, excluded_names));
    // The total is zero when the plan could not be built; the move below then fails with the
    // same error, so this only ever records an authoritative size for transfers that proceed.
    let bytes_total_source = move_plan_result.as_ref().map(|plan| plan.bytes_total).unwrap_or(0);
    transfer_data.bytes_total_measured = Some(bytes_total_source);

    // Warn the user if the measured transfer size is zero — this is likely unintentional.
    // The card directory already exists at this point, so there is no BackToQuery option;
    // the user can either cancel (abandoning the empty card dir) or proceed anyway.
    if bytes_total_source == 0 {
        let (warn_tx, warn_rx) = crossbeam_channel::unbounded::<ui_api::ZeroSizeTransferWarningResponse>();
        if ui.lock().unwrap().user_query(
            ui_api::UserQuery::ZeroSizeTransferWarning(ui_api::ZeroSizeTransferWarningQuery {
                response_tx: warn_tx,
            }),
            true,
        ).is_err() {
            show_transfer_error(&ui, &transfer_event_tx, "Transfer aborted".to_owned());
            return;
        }
        match warn_rx.recv() {
            Ok(ui_api::ZeroSizeTransferWarningResponse::Cancel) | Err(_) => {
                let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                return;
            }
            Ok(ui_api::ZeroSizeTransferWarningResponse::Proceed) => {}
        }
    }

    // Step 5: Register the transfer in the backup log now that the card directory exists and the
    // total size is known.
    let card_path_relative = destination_dir.strip_prefix(&media_dir)
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|_| destination_dir.clone());
    transfer_data.card_path = Some(card_path_relative.clone());

    if let Err(e) = backup_log_manager.lock().unwrap().add_transfer(&transfer_data) {
        show_transfer_error(&ui, &transfer_event_tx, format!("Failed to update backup log: {}", e));
        return;
    }

    let backup_log_for_samples = Arc::clone(&backup_log_manager);
    let card_path_for_samples  = card_path_relative.clone();
    let mut on_samples = move |samples: &[ui_api::TransferSample]| {
        let log_samples: Vec<TransferSample> = samples.iter()
            .map(|s| TransferSample { timestamp_ms: s.timestamp_ms, bytes_done: s.bytes_done })
            .collect();
        let _ = backup_log_for_samples.lock().unwrap()
            .update_transfer_samples(&card_path_for_samples, log_samples);
    };

    // Step 6: Perform the move, recording only the samples and the final result afterwards.
    let move_outcome = match move_plan_result {
        Ok(plan) => move_card_data(&plan, &destination_dir, &transfer_event_tx, &mut on_samples),
        Err(e)   => Err(e),
    };

    transfer_data.transfer_result = Some(match &move_outcome {
        Ok(())       => TransferResult::Succeeded,
        Err(message) => TransferResult::Failed(message.clone()),
    });

    match &transfer_data.transfer_result {
        Some(TransferResult::Succeeded) => {
            let _ = backup_log_manager.lock().unwrap()
                .finalize_transfer(&card_path_relative, false, None);
        }
        Some(TransferResult::Failed(error_message)) => {
            let _ = backup_log_manager.lock().unwrap()
                .finalize_transfer(&card_path_relative, true, Some(error_message.clone()));
            show_transfer_error(&ui, &transfer_event_tx, error_message.clone());
        }
        None => {}
    }

    // Unmount all filesystems that were mounted for this transfer now that the transfer is done.
    crate::mount_manager::start_unmount_for_transfer(
        transfer_id,
        Arc::clone(&mount_manager),
        Arc::clone(&ui),
    );
}

/// Unregisters the transfer from the registry using its currently-selected source media dir.
/// A no-op when no source media dir is selected (the transfer was never registered).
fn unregister_current(
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    transfer_data: &TransferEntry,
    media_dir: &Path,
) {
    if let Some(dir) = transfer_data.fields.source_media_dir_abs(media_dir) {
        registry.lock().unwrap().unregister(transfer_id, &dir)
            .expect("unregister: transfer must be registered before unregistering");
    }
}

/// When the input path has not been manually overridden, reset it to the card root ("/").
/// Called after the device location changes, since the previous default no longer applies.
fn reset_input_path_to_card_root_if_auto(transfer_data: &mut TransferEntry) {
    if !transfer_data.fields.input_path_selected.is_overridden() {
        transfer_data.fields.input_path_detected = Some(PathBuf::from("/"));
        transfer_data.fields.input_path_selected = TransferFieldState::AutoSelected;
    }
}

/// Determine the initial card ID and register the transfer with the registry.
fn initial_card_id_and_register(
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    source_dir: Option<&std::path::Path>,
    detected_card_id: Option<&str>,
    all_source_media: &[SourceMediaEntry],
    media_dir: &std::path::Path,
) -> Result<String, String> {
    let dir = match source_dir {
        Some(d) => d,
        None => {
            // No source media dir yet — use empty card ID, not registered
            return Ok(String::new());
        }
    };

    let scheme = source_media_scheme(dir, all_source_media, media_dir);

    let (card_id, pending) = match detected_card_id {
        Some(manual_id) if !manual_id.is_empty() => {
            let scheme_number = if matches!(scheme, CardNamingScheme::CardFourDigits) {
                crate::transfer_registry::parse_card_number(manual_id)
            } else {
                None
            };
            (manual_id.to_owned(), PendingCardId::Manual { scheme_number })
        }
        _ => match scheme {
            CardNamingScheme::CardFourDigits => {
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

#[allow(clippy::too_many_arguments)]
fn handle_device_overwrite(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    transfer_data: &mut TransferEntry,
    new_dir: SourceMediaId,
    device_overridden: bool,
    all_source_media: &[SourceMediaEntry],
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    update_tx: &crossbeam_channel::Sender<TransferFields>,
    notify_rx: &mut crossbeam_channel::Receiver<()>,
    media_dir: &std::path::Path,
) {
    let new_dir_abs = media_dir.join(&new_dir);
    let card_id_manually_set = transfer_data.fields.card_id_selected.is_overridden();

    // Determine the card ID to carry into the new source media entry.
    // Manually set IDs are kept as-is; auto IDs are regenerated for the new dir.
    let new_card_id = match source_media_scheme(&new_dir_abs, all_source_media, media_dir) {
        CardNamingScheme::CardFourDigits if !card_id_manually_set => {
            match registry.lock().unwrap().next_card_id(&new_dir_abs, transfer_id) {
                Ok(id) => id,
                Err(e) => {
                    show_card_id_error(ui, None, format!("Failed to compute card ID for new device: {}", e));
                    String::new()
                }
            }
        }
        _ => transfer_data.fields.card_id_string(), // keep existing
    };

    let new_pending_card_id_data = if card_id_manually_set {
        PendingCardId::Manual {
            scheme_number: crate::transfer_registry::parse_card_number(&new_card_id),
        }
    } else {
        PendingCardId::Auto(new_card_id.clone())
    };

    // Move registry entry and re-subscribe to the new dir atomically
    let new_notify_rx = {
        let mut reg = registry.lock().unwrap();
        match transfer_data.fields.source_media_dir_abs(media_dir) {
            Some(old) => reg.move_source_media(transfer_id, &old, &new_dir_abs, new_pending_card_id_data),
            None      => reg.register(transfer_id, &new_dir_abs, new_pending_card_id_data),
        }
        reg.subscribe(&new_dir_abs)
    };
    *notify_rx = new_notify_rx;

    // Update the transfer state
    transfer_data.fields.source_media_selected = if device_overridden {
        TransferFieldState::Overridden(new_dir.clone())
    } else {
        TransferFieldState::AutoSelected
    };
    if card_id_manually_set {
        transfer_data.fields.set_card_id_manual(new_card_id);
    } else {
        transfer_data.fields.set_card_id_auto(new_card_id);
    }

    // Update the transfer in the UI
    let _ = transfer_event_tx.send(ui_api::TransferEvent::SourceMediaChanged(
        Some(new_dir.to_string_lossy().into_owned()),
    ));

    // Update the user query in the UI
    let _ = update_tx.send(transfer_data.fields.clone());
}

#[allow(clippy::too_many_arguments)]
fn handle_card_id_changed(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    registry: &Arc<Mutex<PendingTransferRegistry>>,
    transfer_id: TransferId,
    transfer_data: &mut TransferEntry,
    all_source_media: &[SourceMediaEntry],
    new_id: String,
    update_tx: &crossbeam_channel::Sender<TransferFields>,
    media_dir: &std::path::Path,
) { // TODO: check that if the field is now empty but no automatic id can be generated for whatever
    // reason it gets handled correctly and add relevant test case
    let source_dir = transfer_data.fields.source_media_dir_abs(media_dir);
    let pending = if new_id.is_empty() {
        // IF empty revert to auto if scheme supports it
        if let Some(dir) = source_dir.as_deref() {
            if matches!(source_media_scheme(dir, all_source_media, media_dir), CardNamingScheme::CardFourDigits) {
                match registry.lock().unwrap().next_card_id(dir, transfer_id) {
                    Ok(auto_id) => {
                        transfer_data.fields.set_card_id_auto(auto_id.clone());
                        PendingCardId::Auto(auto_id)
                    }
                    Err(e) => {
                        show_card_id_error(ui, None, format!("Failed to revert card ID to auto-generated: {}", e));
                        transfer_data.fields.set_card_id_manual(new_id);
                        PendingCardId::Manual { scheme_number: None }
                    }
                }
            } else {
                transfer_data.fields.set_card_id_manual(new_id);
                PendingCardId::Manual { scheme_number: None }
            }
        } else {
            transfer_data.fields.set_card_id_manual(new_id);
            PendingCardId::Manual { scheme_number: None }
        }
    } else {
        let scheme_number = source_dir.as_deref()
            .and_then(|dir| {
                if matches!(source_media_scheme(dir, all_source_media, media_dir), CardNamingScheme::CardFourDigits) {
                    crate::transfer_registry::parse_card_number(&new_id)
                } else {
                    None
                }
            });
        transfer_data.fields.set_card_id_manual(new_id);
        PendingCardId::Manual { scheme_number }
    };

    if let Some(dir) = source_dir.as_deref() {
        registry.lock().unwrap().update_id(transfer_id, dir, pending)
            .expect("update_id: transfer must be registered before updating");
    }

    let _ = update_tx.send(transfer_data.fields.clone());
}

fn source_media_scheme(dir: &std::path::Path, all_source_media: &[SourceMediaEntry], media_dir: &std::path::Path) -> CardNamingScheme {
    all_source_media.iter()
        .find(|e| media_dir.join(&e.directory) == dir)
        .map(|e| e.new_card_naming_scheme.clone())
        .unwrap_or(CardNamingScheme::Freeform)
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
fn resolve_source_data_dir(fields: &TransferFields) -> Result<PathBuf, String> {
    let virtual_path = fields.input_path()
        .ok_or_else(|| "No input path was selected".to_owned())?;
    match &fields.mount_root {
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

/// Total size in bytes of a pre-computed list of paths.
/// For directory entries the size is computed recursively via [`directory_size_bytes`].
/// This lets callers apply any filtering logic once (when building the list) and have
/// both the size measurement and the transfer work from exactly the same set of paths.
fn entries_size_bytes(entries: &[PathBuf]) -> std::io::Result<u64> {
    let mut total: u64 = 0;
    for path in entries {
        let metadata = path.metadata()?; // follows symlinks, matching move_card_data's source_metadata
        if metadata.is_dir() {
            total += directory_size_bytes(path)?;
        } else if metadata.is_file() {
            total += metadata.len();
        }
    }
    Ok(total)
}

/// The set of source paths a card-data move will transfer, together with their total size.
/// Produced by [`plan_card_move`] and consumed by [`move_card_data`], so both the size
/// measurement written to the backup log and the actual move work from exactly the same paths.
struct CardMovePlan {
    /// The paths handed to the transfer binary.
    source_entries: Vec<PathBuf>,
    /// Total size in bytes of `source_entries`, measured before the move began.
    bytes_total: u64,
}

/// Decides which paths a card-data move will transfer and measures their total size, *without*
/// moving anything. Running this before the transfer is recorded lets the caller write the
/// authoritative total size to the backup log up front, before the copy begins.
///
/// `source_path` may be either a directory, whose *contents* (minus `excluded_names`) are moved
/// into the card directory — like `mv source/* dest/`, so the directory itself does not become a
/// subdirectory — or a single file, which is moved into the card directory as-is.
fn plan_card_move(
    source_path: &std::path::Path,
    excluded_names: &[&str],
) -> Result<CardMovePlan, String> {
    // metadata() follows symlinks, so a symlink pointing at a directory is still treated
    // as a directory whose contents are moved.
    let source_metadata = std::fs::metadata(source_path)
        .map_err(|e| format!("Failed to inspect source path {:?}: {}", source_path, e))?;

    // A directory move transfers its contents; a file move transfers the file itself.
    if source_metadata.is_dir() {
        // Build the entry list first so the exclusion filter is applied once, then derive
        // the size from that same list — both the transfer and the measurement see identical paths.
        let directory_entries: Vec<PathBuf> = std::fs::read_dir(source_path)
            .and_then(|entries| entries.map(|entry| entry.map(|e| e.path())).collect::<std::io::Result<Vec<_>>>())
            .map_err(|e| format!("Failed to list source directory {:?}: {}", source_path, e))?
            .into_iter()
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .map(|n| !excluded_names.contains(&n))
                    .unwrap_or(true)
            })
            .collect();
        let bytes_total = entries_size_bytes(&directory_entries)
            .map_err(|e| format!("Failed to measure size of source data at {:?}: {}", source_path, e))?;
        Ok(CardMovePlan { source_entries: directory_entries, bytes_total })
    } else {
        Ok(CardMovePlan { source_entries: vec![source_path.to_path_buf()], bytes_total: source_metadata.len() })
    }
}

/// Moves the pre-planned data (see [`plan_card_move`]) into the (already-created) card directory
/// `destination_dir` by invoking the system GNU `mv` binary, while periodically reporting progress
/// to the UI transfer graph through `transfer_event_tx`.
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
    plan: &CardMovePlan,
    destination_dir: &std::path::Path,
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    on_samples: &mut impl FnMut(&[ui_api::TransferSample]),
) -> Result<(), String> {
    let source_entries = &plan.source_entries;
    let bytes_total = plan.bytes_total;

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
    let move_source_entries = source_entries.clone();
    thread::spawn(move || {
        let mut cmd = std::process::Command::new(TRANSFER_BINARY);
        #[cfg(feature = "copy-instead-of-move")]
        cmd.arg("--recursive").arg("--preserve=timestamps");
        let move_command_output = cmd
            .arg("--target-directory")
            .arg(&move_destination)
            .arg("--")
            .args(&move_source_entries)
            .output();
        let result = match move_command_output {
            Ok(output) if output.status.success() => Ok(()),
            Ok(output) => Err(String::from_utf8_lossy(&output.stderr).trim().to_owned()),
            Err(spawn_error) => Err(format!("Could not run `{}`: {}", TRANSFER_BINARY, spawn_error)),
        };
        let _ = move_result_tx.send(result);
    });

    let binary_outcome: Result<(), String> = loop {
        let maybe_result: Option<Result<(), String>> = crossbeam_channel::select! {
            recv(move_result_rx) -> result => Some(match result {
                Ok(Ok(()))       => Ok(()),
                Ok(Err(message)) => Err(format!("Failed to move data to {:?}: {}", destination_dir, message)),
                Err(_)           => Err("Move thread exited without reporting a result".to_owned()),
            }),
            default(TRANSFER_SAMPLE_INTERVAL) => None,
        };

        if let Ok(bytes_done) = directory_size_bytes(destination_dir) {
            let samples = vec![ui_api::TransferSample { timestamp_ms: current_time_ms(), bytes_done }];
            on_samples(&samples);
            let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferSamples(samples));
        }

        if let Some(result) = maybe_result {
            break result;
        }
    };

    binary_outcome
}

// Mark the transfer as failed in the UI immediately, then show the fatal error dialog.
fn show_transfer_error(
    ui: &Arc<Mutex<Box<dyn UiBackend>>>,
    transfer_event_tx: &crossbeam_channel::Sender<ui_api::TransferEvent>,
    message: String,
) {
    let _ = transfer_event_tx.send(ui_api::TransferEvent::TransferFailed);
    let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
    let _ = ui.lock().unwrap().user_query(
        ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
            error: ui_api::FatalErrorKind::Transfer(message),
            response_tx,
        }),
        true,
    );
    let _ = response_rx.recv();
}

#[cfg(test)]
#[path = "transfer_logic_tests.rs"]
mod tests;
