/* backup_log.rs

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

use std::path::{Path, PathBuf};
use serde::{Deserialize, Serialize};
use serde_json::ser::PrettyFormatter;
use crate::transfer_logic::{TransferSample, TransferEntry};

const BACKUP_LOG_DATA_TYPE: &str = "backup_log_data";
const BACKUP_LOG_STRUCTURE_MAJOR: u32 = 0;
const BACKUP_LOG_CAPABILITY_LEVEL: u32 = 0;

pub const BACKUP_LOG_DATA_DIR_NAME: &str = "backup_log_data";

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupLogStructureVersion {
    pub major: u32,
    pub capability_level: u32,
}


#[derive(Deserialize)]
struct BackupLogHeader {
    data_type: String,
    data_structure_version: BackupLogStructureVersion,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupLogTransferEntry {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_uuidv7: Option<String>,
    pub card_path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_media_overridden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub card_id_overridden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium_uuidv7: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub medium_uuidv7_overridden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_location: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_location_overridden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_path_overridden: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_performed_by: Option<String>,
    /// Byte count of the destination directory measured once after the transfer binary exited.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes_total_measured: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_failed: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system_hostname: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transfer_samples: Option<Vec<TransferSample>>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct BackupLogEntry {
    pub data_type: String,
    pub data_structure_version: BackupLogStructureVersion,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous_uuidv7: Option<String>,
    pub current_uuidv7: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_uuidv7: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub comment: Option<String>,
    pub completed_backup: bool,
    pub new_transfers: Vec<BackupLogTransferEntry>,
}

/// Thread-safe writer for a single backup log entry.
/// All mutations flush the full entry atomically (write to a `.tmp` file then rename).
pub struct BackupLogManager {
    log_dir: PathBuf,
    entry: BackupLogEntry,
}

impl BackupLogManager {
    /// Creates a brand-new backup log entry.
    /// When `previous_uuidv7` is `Some`, the previous entry's `next_uuidv7` field is updated
    /// atomically before the new entry is written.
    pub fn create_new(log_dir: PathBuf, previous_uuidv7: Option<String>) -> Result<Self, String> {
        let new_uuid = uuid::Uuid::now_v7().to_string();

        if let Some(ref prev_uuid) = previous_uuidv7 {
            set_next_uuidv7_on_entry(&log_dir, prev_uuid, &new_uuid)?;
        }

        let entry = BackupLogEntry {
            data_type: BACKUP_LOG_DATA_TYPE.to_owned(),
            data_structure_version: BackupLogStructureVersion {
                major: BACKUP_LOG_STRUCTURE_MAJOR,
                capability_level: BACKUP_LOG_CAPABILITY_LEVEL,
            },
            previous_uuidv7,
            current_uuidv7: new_uuid,
            next_uuidv7: None,
            comment: None,
            completed_backup: false,
            new_transfers: Vec::new(),
        };

        let manager = BackupLogManager { log_dir, entry };
        manager.flush()?;
        Ok(manager)
    }

    /// Constructs a manager from a previously-written entry that is still in progress.
    /// Does NOT write to disk — the on-disk file is left untouched until the first mutation.
    pub fn from_existing(log_dir: PathBuf, entry: BackupLogEntry) -> Self {
        BackupLogManager { log_dir, entry }
    }

    /// Appends a new transfer record and flushes to disk atomically.
    ///
    /// Takes the live [`TransferEntry`] and prepares its data for on-disk storage here: the
    /// `Uuid` storage id is rendered to a string, the device-location by-id name is pulled out of
    /// its `(path, name)` pair, and the user's auto-vs-override choices are recorded as booleans.
    /// Outcome fields (bytes/failure/samples) are filled in later by `finalize_transfer` /
    /// `update_transfer_samples`.
    pub fn add_transfer(&mut self, transfer: &TransferEntry) -> Result<(), String> {
        let fields = &transfer.fields;
        self.entry.new_transfers.push(BackupLogTransferEntry {
            transfer_uuidv7:            Some(transfer.transfer_uuidv7.clone()),
            card_path:                  transfer.card_path.clone()
                                            .expect("card_path must be set before recording the transfer"),
            card_id:                    fields.card_id().cloned(),
            source_media_overridden:    Some(fields.source_media_selected.is_overridden()),
            card_id_overridden:         Some(fields.card_id_selected.is_overridden()),
            medium_uuidv7:              fields.storage_device().map(|id| id.to_string()),
            medium_uuidv7_overridden:   Some(fields.storage_device_selected.is_overridden()),
            device_location:            fields.device_location_name().map(|name| name.to_owned()),
            device_location_overridden: Some(fields.device_location_selected.is_overridden()),
            input_path:                 fields.input_path().cloned(),
            input_path_overridden:      Some(fields.input_path_selected.is_overridden()),
            transfer_samples:           Some(Vec::new()),
            transfer_performed_by:      Some(format!("ingest_and_snapshot {}", env!("CARGO_PKG_VERSION"))),
            bytes_total_measured:       None,
            transfer_failed:            None,
            failure_message:            None,
            system_hostname:            Some(transfer.system_hostname.clone()),
        });
        self.flush()
    }

    /// Records the final outcome of a transfer: the measured destination size and whether it failed.
    /// Identified by `card_path`; silently does nothing if no matching transfer is found.
    pub fn finalize_transfer(
        &mut self,
        card_path: &Path,
        bytes_total_measured: u64,
        failed: bool,
        failure_message: Option<String>,
    ) -> Result<(), String> {
        if let Some(transfer) = self.entry.new_transfers.iter_mut().find(|t| t.card_path == card_path) {
            transfer.bytes_total_measured = Some(bytes_total_measured); //TODO: remove that from here
            transfer.transfer_failed      = Some(failed);
            transfer.failure_message      = failure_message;
        }
        self.flush()
    }

    /// Returns true if the log already contains a transfer recorded for `card_path`.
    pub fn has_transfer_for_card_path(&self, card_path: &Path) -> bool {
        self.entry.new_transfers.iter().any(|t| t.card_path == card_path)
    }

    /// Appends samples to an existing transfer record and flushes to disk atomically.
    /// Identified by `card_path`; silently does nothing if no matching transfer is found.
    pub fn update_transfer_samples(&mut self, card_path: &Path, new_samples: Vec<TransferSample>) -> Result<(), String> {
        if let Some(transfer) = self.entry.new_transfers.iter_mut().find(|t| t.card_path == card_path) {
            transfer.transfer_samples.get_or_insert_with(Vec::new).extend(new_samples);
        }
        self.flush()
    }

    fn flush(&self) -> Result<(), String> {
        let mut json = Vec::new();
        let formatter = PrettyFormatter::with_indent(b"\t");
        let mut serializer = serde_json::Serializer::with_formatter(&mut json, formatter);

        self.entry.serialize(&mut serializer)
            .map_err(|e| format!("Failed to serialize backup log entry: {}", e))?;

        let file_path = self.log_dir.join(format!("{}.json", self.entry.current_uuidv7));
        let tmp_path  = self.log_dir.join(format!("{}.json.tmp", self.entry.current_uuidv7));

        std::fs::write(&tmp_path, json)
            .map_err(|e| format!("Failed to write backup log to {}: {}", tmp_path.display(), e))?;
        std::fs::rename(&tmp_path, &file_path)
            .map_err(|e| format!("Failed to finalize backup log at {}: {}", file_path.display(), e))?;
        Ok(())
    }
}


pub enum BackupLogState {
    UseExistingEntry(BackupLogEntry),
    CreateNewEntry { previous_uuidv7: Option<String> },
}

fn parse_backup_log_file(path: &PathBuf) -> Result<BackupLogEntry, String> {
    let raw_json = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: failed to read: {}", path.display(), e))?;

    let header = serde_json::from_str::<BackupLogHeader>(&raw_json)
        .map_err(|e| format!("{}: failed to parse JSON: {}", path.display(), e))?;

    if header.data_type != BACKUP_LOG_DATA_TYPE {
        return Err(format!("{}: unexpected data_type '{}' (expected '{}')",
            path.display(), header.data_type, BACKUP_LOG_DATA_TYPE));
    }
    if header.data_structure_version.major != BACKUP_LOG_STRUCTURE_MAJOR {
        return Err(format!(
            "{}: unsupported data_structure_version: major {} is not supported (expected {})",
            path.display(), header.data_structure_version.major, BACKUP_LOG_STRUCTURE_MAJOR
        ));
    }
    #[allow(clippy::absurd_extreme_comparisons)]
    if header.data_structure_version.capability_level < BACKUP_LOG_CAPABILITY_LEVEL {
        return Err(format!(
            "{}: unsupported data_structure_version: capability_level {} is below minimum {}",
            path.display(), header.data_structure_version.capability_level, BACKUP_LOG_CAPABILITY_LEVEL
        ));
    }

    serde_json::from_str::<BackupLogEntry>(&raw_json)
        .map_err(|e| format!("{}: failed to parse entry: {}", path.display(), e))
}

pub fn load_backup_log(media_dir: &Path) -> Result<BackupLogState, String> {
    let log_dir = media_dir.join("metadata").join(BACKUP_LOG_DATA_DIR_NAME);

    if !log_dir.exists() {
        return Err(format!("{}: directory not found", log_dir.display()));
    }

    let dir_entries = std::fs::read_dir(&log_dir)
        .map_err(|e| format!("{}: failed to read directory: {}", log_dir.display(), e))?;

    let mut filenames: Vec<String> = dir_entries
        .map(|entry_result| {
            entry_result
                .map_err(|e| format!("Failed to read item in directory {}: {}", log_dir.display(), e))
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
        })
        .collect::<Result<Vec<_>, _>>()?;

    if filenames.is_empty() {
        return Ok(BackupLogState::CreateNewEntry { previous_uuidv7: None });
    }

    filenames.sort();

    let last_filename = filenames.last().unwrap();
    let mut current_path = log_dir.join(last_filename);
    let mut entry = parse_backup_log_file(&current_path)?;

    while let Some(ref next_uuid) = entry.next_uuidv7.clone() {
        current_path = log_dir.join(format!("{}.json", next_uuid));
        entry = parse_backup_log_file(&current_path)?;
    }

    if entry.completed_backup {
        Ok(BackupLogState::CreateNewEntry { previous_uuidv7: Some(entry.current_uuidv7) })
    } else {
        Ok(BackupLogState::UseExistingEntry(entry))
    }
}


/// Updates the `next_uuidv7` field of an already-written entry atomically.
fn set_next_uuidv7_on_entry(log_dir: &Path, entry_uuid: &str, next_uuid: &str) -> Result<(), String> {
    let file_path = log_dir.join(format!("{}.json", entry_uuid));
    let content   = std::fs::read_to_string(&file_path)
        .map_err(|e| format!("Failed to read {}: {}", file_path.display(), e))?;

    let mut json_data: BackupLogEntry = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", file_path.display(), e))?;

    json_data.next_uuidv7 = Some(next_uuid.to_owned());

    let mut json_serialized = Vec::new();
    let formatter = PrettyFormatter::with_indent(b"\t");
    let mut serializer = serde_json::Serializer::with_formatter(&mut json_serialized, formatter);
    json_data.serialize(&mut serializer)
        .map_err(|e| format!("Failed to re-serialize backup log entry: {}: {}",file_path.display(), e))?;

    let tmp_path = log_dir.join(format!("{}.json.tmp", entry_uuid));
    std::fs::write(&tmp_path, json_serialized)
        .map_err(|e| format!("Failed to write {}: {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, &file_path)
        .map_err(|e| format!("Failed to finalize {}: {}", file_path.display(), e))?;
    Ok(())
}

#[cfg(test)]
#[path = "backup_log_tests.rs"]
mod tests;
