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

/// This module defines the protocol between the main application logic and any UI backend.

use crossbeam_channel::{Receiver, Sender};

pub struct TransferSample {
    pub timestamp_ms: u64,
    pub bytes_done: u64,
}

pub enum TransferEvent {
    DeviceUnplugged,
    SourceMediaChanged(Option<String>),
    TransferStarted { bytes_total: u64 },
    TransferSamples(Vec<TransferSample>),
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
}

pub struct ConfirmCardIdQuery {
    pub original_id: String,
    pub suggested_id: Option<String>, // The next sequential ID (UseNew option)
    pub was_manually_set: bool,
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
    pub source_media_dir: Option<String>,
    pub source_device: String,
    pub data_size: u64,
    pub card_id: String,
    pub device_overridden: bool,
    pub storage_device_overridden: bool,
    pub card_id_overridden: bool,
}

pub struct ApproveTransferQuery {
    pub data: ApproveTransferQueryUpdate,
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
}

pub struct ScanNewDeviceQuery {
    pub device_name: String,
    pub response_tx: Sender<bool>,
}

pub enum FatalErrorKind {
    DevicesJson(String),
    SourceMedia(String),
    BackupLog(String),
    CardId(String),
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

pub enum UserQuery {
    ApproveTransfer(ApproveTransferQuery),
    ScanNewDevice(ScanNewDeviceQuery),
    FatalError(FatalErrorQuery), //XXX: This doesn't get priority in the queue but it's assumed it
                                 //will be sent before any other message anyways so it doesn't matter
    SourceMediaWarnings(SourceMediaWarningsQuery),
    ConfirmCardId(ConfirmCardIdQuery),
    NoSourceMediaWarning(NoSourceMediaWarningQuery),
}

/// Messages the UI sends back to the main logic.
pub enum UiToLogicMessage {
    Quit,
    StartManualTransfer,
}

/// Returned when a UiBackend method fails because the backend is no longer reachable.
#[derive(Debug)]
pub enum UiError {
    Disconnected,
}

/// The interface through which the main logic communicates with any UI backend.
pub trait UiBackend: Send {
    fn add_config(&mut self, allow: Vec<String>, ignore: Vec<String>) -> Result<(), UiError>;
    fn set_available_devices(&mut self, devices: Vec<crate::SourceMediaEntry>) -> Result<(), UiError>;
    fn new_transfer(&mut self, source_media_dir: Option<String>, rx_control: Receiver<TransferEvent>) -> Result<(), UiError>;
    fn user_query(&mut self, query: UserQuery, priority: bool) -> Result<(), UiError>;
    fn quit(&mut self) -> Result<(), UiError>;
    /// Block until the backend has fully shut down. Should be called after quit().
    fn join(self: Box<Self>);
}
