/* ui_api.rs

   This file is part of the ingest_and_snapshot project

   Copyright (c) 2026 Efthymios Kritikos

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.

   You should have received a copy of the GNU General Public License
   along with this program.  If not, see <http://www.gnu.org/licenses/>.  */

//! This module defines the protocol between the main application logic and any UI backend.


use std::path::PathBuf;
use crossbeam_channel::{Receiver, Sender};

pub struct TransferSample {
    pub timestamp_ms: u64,
    pub bytes_done: u64,
}

/// Describes the state of a single field in a transfer approval dialog.
/// Exactly one state is valid at a time, preventing conflicting flag combinations.
pub enum TransferFieldState<T> {
    /// The system automatically detected a value. `None` means detection ran but found nothing.
    AutoSelected(Option<T>),
    /// The user has manually set the field to this value.
    Overridden(T),
}

impl<T> TransferFieldState<T> {
    /// Returns the current value if one is available, or `None` when the state is
    /// `AutoSelected(None)`.
    pub fn value(&self) -> Option<&T> {
        match self {
            Self::AutoSelected(Some(v)) | Self::Overridden(v) => Some(v),
            Self::AutoSelected(None) => None,
        }
    }

    pub fn is_overridden(&self) -> bool {
        matches!(self, Self::Overridden(_))
    }
}

pub enum TransferEvent {
    DeviceUnplugged,
    SourceMediaChanged(Option<String>),
    TransferStarted { bytes_total: u64 },
    TransferSamples(Vec<TransferSample>),
    TransferFailed,
}

pub enum SourceMediaSelection {
    Auto,
    Overridden(String),
}

pub enum ApproveTransferResponse {
    Approved,
    Denied,
    DeviceOverwrite(SourceMediaSelection),
    CardIdChanged(String),
    StorageDeviceChanged(String), // storage device ID
    StorageDeviceAuto,            // reset storage device to auto-detected
    DeviceLocationChanged(String), // new /dev/disk/by-id/ entry selected
    DeviceLocationAuto,            // reset device location to auto-detected
    InputPathChanged(PathBuf),     // user selected a new virtual input path
}

pub struct ConfirmCardIdQuery {
    pub original_id: String,
    pub suggested_id: Option<String>, // The next sequential ID (UseNew option)
    pub conflict_reason: CardIdConflictReason,
    pub response_tx: Sender<ConfirmCardIdResponse>,
}

pub enum CardIdConflictReason {
    IdTaken,
    SequenceGap,
}

pub enum ConfirmCardIdResponse {
    UseNew,
    UseOriginal,
    BackToQuery,
}

pub struct ApproveTransferQueryUpdate {
    pub source_media_dir: TransferFieldState<String>,
    pub source_device: TransferFieldState<String>,
    pub card_id: TransferFieldState<String>,
    pub device_location: TransferFieldState<String>,
    /// Virtual path on the source device (e.g. `PathBuf::from("/DCIM")`).
    /// Frozen while the block device is not yet mounted.
    pub input_path: TransferFieldState<PathBuf>,
    /// Actual OS mountpoint of the source block device, if one is mounted.
    /// `None` for local-filesystem transfers (where the virtual path IS the actual path).
    pub input_path_mount_root: Option<PathBuf>,
}

pub struct ApproveTransferQuery {
    pub initial_data: ApproveTransferQueryUpdate,
    pub response_tx: Sender<ApproveTransferResponse>,
    pub update_rx: Receiver<ApproveTransferQueryUpdate>,
    /// Whether an auto-detected source media exists for this transfer.
    /// When false, the UI should not offer an "Auto-detected" option in the device picker,
    /// since returning to Auto would mean returning to no source media.
    pub has_auto_detected_source_media: bool,
    /// Whether an auto-detected storage device exists for this transfer.
    /// When true, the picker offers an "Auto-detected" option that sends StorageDeviceAuto.
    pub has_auto_detected_storage_device: bool,
    /// All known storage devices that can be selected as the destination for this transfer.
    pub available_storage_devices: Vec<crate::StorageDeviceEntry>,
    /// Whether an auto-detected device location exists for this transfer.
    /// When false the picker offers no "Auto-detected" option.
    pub has_auto_detected_device_location: bool,
    /// All currently connected allowed device locations (by-id names) the user can pick from.
    pub available_device_locations: Vec<String>,
}

pub struct UnknownDeviceQuery {
    pub device_name: String,
    pub response_tx: Sender<UnknownDeviceResponse>,
}

pub enum UnknownDeviceResponse {
    AddToAllowList,
    AddToIgnoreList,
    AllowOnce,
    Ignore,
}

pub enum FatalErrorKind {
    DevicesJson(String),
    SourceMedia(String),
    BackupLog(String),
    CardId(String),
    Transfer(String),
    ActiveTransfers,
    PerDeviceConfig(String),
}

pub struct FatalErrorQuery {
    pub error: FatalErrorKind,
    pub response_tx: Sender<()>,
}

pub struct SourceMediaWarningsQuery {
    pub warnings: Vec<String>,
    pub response_tx: Sender<()>,
}

pub enum NoSourceMediaWarningResponse {
    BackToQuery,
    Cancel,
}

pub struct NoSourceMediaWarningQuery {
    pub response_tx: Sender<NoSourceMediaWarningResponse>,
}

pub enum NoDeviceLocationWarningReason {
    NoneSelected,
    NotFound,
}

pub enum NoDeviceLocationWarningResponse {
    BackToQuery,
    Cancel,
}

pub struct NoDeviceLocationWarningQuery {
    pub reason: NoDeviceLocationWarningReason,
    pub response_tx: Sender<NoDeviceLocationWarningResponse>,
}

pub enum NoInputPathWarningResponse {
    BackToQuery,
    Cancel,
}

pub struct NoInputPathWarningQuery {
    pub response_tx: Sender<NoInputPathWarningResponse>,
}

pub struct NewBackupLogQuery {
    pub response_tx: Sender<NewBackupLogResponse>,
}

pub enum NewBackupLogResponse {
    CreateNew,
    Quit,
}

pub struct CardIdInLogWarningQuery {
    pub card_id: String,
    pub response_tx: Sender<CardIdInLogWarningResponse>,
}

pub enum CardIdInLogWarningResponse {
    BackToQuery,
    Cancel,
}

pub enum UserQuery {
    ApproveTransfer(ApproveTransferQuery),
    UnknownDevice(UnknownDeviceQuery),
    FatalError(FatalErrorQuery), //XXX: This doesn't get priority in the queue but it's assumed it
                                 //will be sent before any other message anyways so it doesn't matter
    SourceMediaWarnings(SourceMediaWarningsQuery),
    ConfirmCardId(ConfirmCardIdQuery),
    NoSourceMediaWarning(NoSourceMediaWarningQuery),
    NoDeviceLocationWarning(NoDeviceLocationWarningQuery),
    NoInputPathWarning(NoInputPathWarningQuery),
    NewBackupLog(NewBackupLogQuery),
    CardIdInLogWarning(CardIdInLogWarningQuery),
}

/// Messages the UI sends back to the main logic.
pub enum UiToLogicMessage {
    Quit,
    StartManualTransfer,
    UnmountRequest(MountId),
}

/// Returned when a UiBackend method fails because the backend is no longer reachable.
#[derive(Debug)]
pub enum UiError {
    Disconnected,
}

pub type MountId = u32;

pub enum LoadingField<T> {
    Loading,
    Loaded(T),
}

pub enum MountEntryStatus {
    Mounting,
    Mounted,
    Failed { reason: String },
    UnmountFailed { reason: String },
}

pub struct MountEntry {
    pub id: MountId,
    pub by_id_name: String,
    pub real_device_path: std::path::PathBuf,
    pub mountpoint: std::path::PathBuf,
    pub status: MountEntryStatus,
    pub fs_type: LoadingField<String>,
}

pub enum MountUpdate {
    MountAdded(MountEntry),
    MountCompleted { id: MountId, fs_type: String },
    MountFailed { id: MountId, reason: String },
    MountRemoved { id: MountId },
    UnmountFailed { id: MountId, reason: String },
}

pub struct SystemInfo {
    pub ram_used_bytes: u64,
    pub ram_total_bytes: u64,
    pub hostname: String,
    pub zfs_version: String,
}

/// The interface through which the main logic communicates with any UI backend.
pub trait UiBackend: Send {
    fn add_config(&mut self, allow: Vec<String>, ignore: Vec<String>) -> Result<(), UiError>;
    fn set_available_devices(&mut self, devices: Vec<crate::SourceMediaEntry>) -> Result<(), UiError>;
    fn new_transfer(&mut self, source_media_dir: Option<String>, rx_control: Receiver<TransferEvent>) -> Result<(), UiError>;
    fn user_query(&mut self, query: UserQuery, priority: bool) -> Result<(), UiError>;
    fn mount_update(&mut self, update: MountUpdate) -> Result<(), UiError>;
    fn system_info(&mut self, info: SystemInfo) -> Result<(), UiError>;
    fn quit(&mut self) -> Result<(), UiError>;
    /// Block until the backend has fully shut down. Should be called after quit().
    fn join(self: Box<Self>);
}
