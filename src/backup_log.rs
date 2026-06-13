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

const BACKUP_LOG_DATA_TYPE: &str = "backup_log_data";
const BACKUP_LOG_STRUCTURE_MAJOR: u32 = 0;
const BACKUP_LOG_CAPABILITY_LEVEL: u32 = 0;

pub const BACKUP_LOG_DATA_DIR_NAME: &str = "backup_log_data";

#[derive(Serialize, Deserialize, Clone)]
struct BackupLogStructureVersion {
    major: u32,
    capability_level: u32,
}

/// A single progress sample recorded during a data transfer.
#[derive(Serialize, Deserialize, Clone)]
pub struct BackupLogSample {
    pub timestamp_ms: u64,
    pub bytes_done: u64,
}

#[derive(Serialize, Clone)]
struct BackupLogTransferWritable {
    card_path: PathBuf,
    medium_uuidv7: Option<String>,
    transfer_samples: Vec<BackupLogSample>,
}

#[derive(Serialize, Clone)]
struct BackupLogEntryWritable {
    data_type: String,
    data_structure_version: BackupLogStructureVersion,
    previous_uuidv7: Option<String>,
    current_uuidv7: String,
    next_uuidv7: Option<String>,
    comment: Option<String>,
    completed_backup: bool,
    new_transfers: Vec<BackupLogTransferWritable>,
}

/// Thread-safe writer for a single backup log entry.
/// All mutations flush the full entry atomically (write to a `.tmp` file then rename).
pub struct BackupLogManager {
    log_dir: PathBuf,
    entry: BackupLogEntryWritable,
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

        let entry = BackupLogEntryWritable {
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
    pub fn from_existing(
        log_dir: PathBuf,
        current_uuidv7: String,
        previous_uuidv7: Option<String>,
        comment: Option<String>,
        existing_transfers: Vec<(PathBuf, Option<String>, Vec<BackupLogSample>)>,
    ) -> Self {
        let entry = BackupLogEntryWritable {
            data_type: BACKUP_LOG_DATA_TYPE.to_owned(),
            data_structure_version: BackupLogStructureVersion {
                major: BACKUP_LOG_STRUCTURE_MAJOR,
                capability_level: BACKUP_LOG_CAPABILITY_LEVEL,
            },
            previous_uuidv7,
            current_uuidv7,
            next_uuidv7: None,
            comment,
            completed_backup: false,
            new_transfers: existing_transfers.into_iter().map(|(card_path, medium_uuidv7, samples)| {
                BackupLogTransferWritable { card_path, medium_uuidv7, transfer_samples: samples }
            }).collect(),
        };
        BackupLogManager { log_dir, entry }
    }

    /// Appends a new transfer record and flushes to disk atomically.
    pub fn add_transfer(&mut self, card_path: PathBuf, medium_uuidv7: Option<String>) -> Result<(), String> {
        self.entry.new_transfers.push(BackupLogTransferWritable {
            card_path,
            medium_uuidv7,
            transfer_samples: Vec::new(),
        });
        self.flush()
    }

    /// Returns true if the log already contains a transfer recorded for `card_path`.
    pub fn has_transfer_for_card_path(&self, card_path: &Path) -> bool {
        self.entry.new_transfers.iter().any(|t| t.card_path == card_path)
    }

    /// Appends samples to an existing transfer record and flushes to disk atomically.
    /// Identified by `card_path`; silently does nothing if no matching transfer is found.
    pub fn update_transfer_samples(&mut self, card_path: &Path, new_samples: Vec<BackupLogSample>) -> Result<(), String> {
        if let Some(transfer) = self.entry.new_transfers.iter_mut().find(|t| t.card_path == card_path) {
            transfer.transfer_samples.extend(new_samples);
        }
        self.flush()
    }

    fn flush(&self) -> Result<(), String> {
        let json = serde_json::to_string_pretty(&self.entry)
            .map_err(|e| format!("Failed to serialize backup log entry: {}", e))?;
        let file_path = self.log_dir.join(format!("{}.json", self.entry.current_uuidv7));
        let tmp_path  = self.log_dir.join(format!("{}.json.tmp", self.entry.current_uuidv7));
        std::fs::write(&tmp_path, json.as_bytes())
            .map_err(|e| format!("Failed to write backup log to {}: {}", tmp_path.display(), e))?;
        std::fs::rename(&tmp_path, &file_path)
            .map_err(|e| format!("Failed to finalize backup log at {}: {}", file_path.display(), e))?;
        Ok(())
    }
}

#[derive(Deserialize)]
struct BackupLogHeader {
    data_type: String,
    data_structure_version: BackupLogStructureVersion,
}

#[derive(Deserialize)]
pub struct BackupLogEntry {
    pub previous_uuidv7: Option<String>,
    pub current_uuidv7: String,
    pub next_uuidv7: Option<String>,
    pub comment: Option<String>,
    pub completed_backup: bool,
    pub new_transfers: Vec<BackupLogTransfer>,
}

#[derive(Deserialize)]
pub struct BackupLogTransfer {
    pub card_path: PathBuf,
    pub medium_uuidv7: Option<String>,
    #[serde(default)]
    pub transfer_samples: Vec<BackupLogSample>,
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
    if header.data_structure_version.capability_level < BACKUP_LOG_CAPABILITY_LEVEL {
        return Err(format!(
            "{}: unsupported data_structure_version: capability_level {} is below minimum {}",
            path.display(), header.data_structure_version.capability_level, BACKUP_LOG_CAPABILITY_LEVEL
        ));
    }

    serde_json::from_str::<BackupLogEntry>(&raw_json)
        .map_err(|e| format!("{}: failed to parse entry: {}", path.display(), e))
}

pub fn load_backup_log(media_dir: &PathBuf) -> Result<BackupLogState, String> {
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
    let mut value: serde_json::Value = serde_json::from_str(&content)
        .map_err(|e| format!("Failed to parse {}: {}", file_path.display(), e))?;
    value["next_uuidv7"] = serde_json::Value::String(next_uuid.to_owned());
    let updated_content = serde_json::to_string(&value)
        .map_err(|e| format!("Failed to re-serialize {}: {}", file_path.display(), e))?;
    let tmp_path = log_dir.join(format!("{}.json.tmp", entry_uuid));
    std::fs::write(&tmp_path, updated_content.as_bytes())
        .map_err(|e| format!("Failed to write {}: {}", tmp_path.display(), e))?;
    std::fs::rename(&tmp_path, &file_path)
        .map_err(|e| format!("Failed to finalize {}: {}", file_path.display(), e))?;
    Ok(())
}
