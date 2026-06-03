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

use std::sync::mpsc::{Receiver, Sender};

pub struct TransferSample {
    pub timestamp_ms: u64,
    pub bytes_done: u64,
}

pub enum TransferEvent {
    DeviceUnplugged,
    CameraNameChanged(String),
    TransferStarted { bytes_total: u64 },
    TransferSamples(Vec<TransferSample>),
}

pub enum ApproveTransferResponse {
    Approved,
    Denied,
    DeviceOverwrite(Option<String>),
}

pub struct ApproveTransferQueryUpdate {
    pub device_product_name: String,
    pub brand: String,
    pub serial_number: String,
    pub source_device: String,
    pub transfer_function: String,
    pub archive_directory: String,
    pub data_size: u64,
    pub card_id: String,
    pub device_overridden: bool,
}

pub struct ApproveTransferQuery {
    pub data: ApproveTransferQueryUpdate,
    pub response_tx: Sender<ApproveTransferResponse>,
    pub update_rx: Receiver<ApproveTransferQueryUpdate>,
}

pub struct ScanNewDeviceQuery {
    pub device_name: String,
    pub response_tx: Sender<bool>,
}

pub enum FatalErrorKind {
    DevicesJson(String),
    SourceMedia(String),
    BackupLog(String),
}

pub struct FatalErrorQuery {
    pub error: FatalErrorKind,
    pub response_tx: Sender<()>,
}

pub struct SourceMediaWarningsQuery {
    pub warnings: Vec<String>,
    pub response_tx: Sender<()>,
}

pub enum UserQuery {
    ApproveTransfer(ApproveTransferQuery),
    ScanNewDevice(ScanNewDeviceQuery),
    FatalError(FatalErrorQuery), //XXX: This doesn't get priority in the queue but it's assumed it
                                 //will be sent before any other message anyways so it doesn't matter
    SourceMediaWarnings(SourceMediaWarningsQuery),
}

/// Messages the UI sends back to the main logic.
pub enum UiToLogicMessage {
    Quit,
}

/// Returned when a UiBackend method fails because the backend is no longer reachable.
#[derive(Debug)]
pub enum UiError {
    Disconnected,
}

/// The interface through which the main logic communicates with any UI backend.
pub trait UiBackend: Send {
    fn add_config(&mut self, allow: Vec<String>, ignore: Vec<String>) -> Result<(), UiError>;
    fn set_available_devices(&mut self, devices: Vec<String>) -> Result<(), UiError>;
    fn new_transfer(&mut self, name: String, camera_name: String, rx_control: Receiver<TransferEvent>) -> Result<(), UiError>;
    fn user_query(&mut self, query: UserQuery) -> Result<(), UiError>;
    fn quit(&mut self) -> Result<(), UiError>;
    /// Block until the backend has fully shut down. Should be called after quit().
    fn join(self: Box<Self>);
}
