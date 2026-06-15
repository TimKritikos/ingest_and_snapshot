/* per_device_config.rs

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
use serde::Deserialize;

pub const CONFIG_FILE_NAME: &str = "ingest_and_snapshot_per_device_config.json";
const EXPECTED_DATA_TYPE: &str = "ingest_and_snapshot_per_device_config";
pub const PER_DEVICE_CONFIG_MAJOR: u32 = 0;
pub const PER_DEVICE_CONFIG_CAPABILITY_LEVEL: u32 = 0;

#[derive(Deserialize)]
pub struct StructureVersion {
    pub major: u32,
    pub capability_level: u32,
}

#[derive(Deserialize)]
struct PerDeviceConfigHeader {
    data_type: String,
    data_structure_version: StructureVersion,
}

#[derive(Deserialize)]
struct PerDeviceConfigTransferEntry {
    #[serde(default)]
    source_media: Option<PathBuf>,
    #[serde(default)]
    storage_device: Option<String>,
    #[serde(default)]
    input_path: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PerDeviceConfigRaw {
    #[serde(default)]
    transfers: Vec<PerDeviceConfigTransferEntry>,
}

/// A single resolved transfer specification from a per-device config file.
///
/// Produced by [`load_per_device_config`]. A single JSON transfer entry with N paths in its
/// `input_path` array expands into N separate overrides, all sharing the same `source_media`
/// and `storage_device`.
#[derive(Debug)]
pub struct PerDeviceTransferOverride {
    /// Directory path of the source media entry to auto-select, if specified.
    pub source_media: Option<PathBuf>,
    /// Storage device ID to auto-select, if specified.
    pub storage_device: Option<String>,
    /// Input path relative to the device root (always starts with `/`), if specified.
    pub input_path: Option<PathBuf>,
}

/// Reads and parses the config file from the root of `mountpoint`.
///
/// Returns `None` if the file is absent, unreadable, malformed JSON, or has an unexpected
/// `data_type` value.  Returns `Some(vec)` (possibly empty) on success.
///
/// Each raw transfer entry is expanded: if `input_path` is a list of N paths, N separate
/// [`PerDeviceTransferOverride`] values are produced for that entry.  Entries with an explicit
/// empty `input_path` array are dropped entirely (they would produce zero transfers).
/// Unrecognised JSON fields are silently ignored.
pub fn load_per_device_config(
    mountpoint: &Path,
    storage_devices: Vec<crate::StorageDeviceEntry>,
    source_media_entries: Vec<crate::SourceMediaEntry>,
) -> Result<Vec<PerDeviceTransferOverride>, String> {
    let config_path = mountpoint.join(CONFIG_FILE_NAME);
    let raw_json = match std::fs::read_to_string(&config_path) {
        Ok(a) => a,
        Err(_) => {
            return Ok(Vec::new());
        },
    };

    let header: PerDeviceConfigHeader = serde_json::from_str(&raw_json)
        .map_err(|e| format!("Failed to parse header for json data: {}", e))?;

    if header.data_type != EXPECTED_DATA_TYPE {
        return Err("Per device config file has incorrect type".to_string());
    }

    #[allow(clippy::absurd_extreme_comparisons)]
    if header.data_structure_version.major != PER_DEVICE_CONFIG_MAJOR {
        return Err(format!(
            "Per device config file has unsupported major version {} (expected {})",
            header.data_structure_version.major, PER_DEVICE_CONFIG_MAJOR
        ));
    }

    #[allow(clippy::absurd_extreme_comparisons)]
    if header.data_structure_version.capability_level < PER_DEVICE_CONFIG_CAPABILITY_LEVEL {
        return Err(format!(
            "Per device config file has insufficient capability level {} (requires at least {})",
            header.data_structure_version.capability_level, PER_DEVICE_CONFIG_CAPABILITY_LEVEL
        ));
    }

    let config: PerDeviceConfigRaw = serde_json::from_str(&raw_json)
        .map_err(|e| format!("Failed to parse per device config file json data: {}", e))?;

    let mut overrides = Vec::new();

    for entry in config.transfers {
        if let Some(ref storage_device) = entry.storage_device {
            if !storage_devices.iter().any(|storage_device_iter| storage_device_iter.id == *storage_device){
                return Err(format!("There is a transfer that specifies an unknown storage device with id {}", storage_device));
            }
        }
        if let Some(ref source_media) = entry.source_media {
            if !source_media_entries.iter().any(|source_media_iter| source_media_iter.directory == *source_media){
                return Err(format!("There is a transfer that specifies an unknown storage device with id {}", source_media.display()));
            }
        }
        match entry.input_path {
            None => {
                overrides.push(PerDeviceTransferOverride {
                    source_media: entry.source_media,
                    storage_device: entry.storage_device,
                    input_path: None,
                });
            }
            Some(paths) if paths.is_empty() => {
                overrides.push(PerDeviceTransferOverride {
                    source_media: entry.source_media,
                    storage_device: entry.storage_device,
                    input_path: None,
                });
            }
            Some(paths) => {
                for path_str in paths {
                    let path = PathBuf::from(&path_str);
                    let normalized = if path.is_absolute() {
                        path
                    } else {
                        PathBuf::from("/").join(path)
                    };
                    overrides.push(PerDeviceTransferOverride {
                        source_media: entry.source_media.clone(),
                        storage_device: entry.storage_device.clone(),
                        input_path: Some(normalized),
                    });
                }
            }
        }
    }

    Ok(overrides)
}

#[cfg(test)]
#[path = "per_device_config_tests.rs"]
mod tests;
