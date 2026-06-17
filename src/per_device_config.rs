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

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use serde::Deserialize;
use uuid::Uuid;

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
    storage_device: Option<Uuid>,
    #[serde(default)]
    input_path: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct PerDeviceConfigRaw {
    #[serde(default)]
    transfers: Vec<PerDeviceConfigTransferEntry>,
    #[serde(default)]
    ignore_unhandled_dirs: Option<bool>,
    #[serde(default)]
    ignored_dirs: Option<Vec<String>>,
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
    pub storage_device: Option<Uuid>,
    /// Input path relative to the device root (always starts with `/`), if specified.
    pub input_path: Option<PathBuf>,
}

fn check_for_unhandled_dirs(
    mountpoint: &Path,
    overrides: &[PerDeviceTransferOverride],
    ignored_dirs: &[String],
) -> Result<(), String> {
    // A root transfer (no specific input_path) covers everything — skip the check.
    if overrides.iter().any(|o| o.input_path.is_none()) {
        return Ok(());
    }

    // Nothing configured at all — nothing to check.
    if overrides.is_empty() {
        return Ok(());
    }

    // Collect the top-level directory name from each input_path.
    let covered_dirs: HashSet<String> = overrides
        .iter()
        .filter_map(|o| o.input_path.as_ref())
        .filter_map(|path| {
            path.components()
                .find(|c| matches!(c, std::path::Component::Normal(_)))
                .map(|c| c.as_os_str().to_string_lossy().into_owned())
        })
        .collect();

    // Accept ignored_dirs with or without a leading slash.
    let normalized_ignored: HashSet<String> = ignored_dirs
        .iter()
        .map(|d| d.trim_start_matches('/').to_string())
        .collect();

    let entries = std::fs::read_dir(mountpoint)
        .map_err(|e| format!("Failed to read mountpoint for unhandled-dir check: {}", e))?;

    let mut unhandled = Vec::new();
    for entry_result in entries {
        let entry = entry_result
            .map_err(|e| format!("Failed to read mountpoint entry: {}", e))?;
        let file_type = entry
            .file_type()
            .map_err(|e| format!("Failed to get file type of mountpoint entry: {}", e))?;

        if !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().into_owned();
        if !covered_dirs.contains(&name) && !normalized_ignored.contains(&name) {
            unhandled.push(name);
        }
    }

    if !unhandled.is_empty() {
        unhandled.sort();
        return Err(format!(
            "Source device has subdirectories not covered by any transfer entry: {}. \
             Add them to the transfers list, add them to ignored_dirs, \
             or set ignore_unhandled_dirs to true to suppress this check.",
            unhandled.join(", ")
        ));
    }

    Ok(())
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
                        storage_device: entry.storage_device,
                        input_path: Some(normalized),
                    });
                }
            }
        }
    }

    if !config.ignore_unhandled_dirs.unwrap_or(false) {
        let ignored = config.ignored_dirs.unwrap_or_default();
        check_for_unhandled_dirs(mountpoint, &overrides, &ignored)?;
    }

    Ok(overrides)
}

#[cfg(test)]
#[path = "per_device_config_tests.rs"]
mod tests;
