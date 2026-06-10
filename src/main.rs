/* main.rs

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

use std::collections::HashMap;
use std::path::PathBuf;
use std::io;
use std::io::Write;
use std::process;
use std::env;
use std::fs::File;
use std::{thread, time};
use std::sync::{Arc, Mutex};
use crossbeam_channel;
use clap::Parser;
use home::home_dir;
use anyhow::{Result};
use nix::sys::statvfs::statvfs;
use nix::sys::statvfs::FsFlags;
use serde::{Deserialize, Serialize};
use udev::{Enumerator, MonitorBuilder};
use std::ffi::OsStr;

mod ui;
mod ui_api;
mod transfer_logic;
mod transfer_registry;
#[cfg(feature = "dummy-ui-data")]
mod dummy_ui_data;

const DEVICES_JSON_EXPECTED_MAJOR: u32 = 0;
const DEVICES_JSON_MIN_CAPABILITY_LEVEL: u32 = 0;

const SOURCE_MEDIA_DIR_NAME: &str = "source_media";

const SOURCE_MEDIA_CONFIG_EXPECTED_MAJOR: u32 = 0;
const SOURCE_MEDIA_CONFIG_MIN_CAPABILITY_LEVEL: u32 = 1;
const SOURCE_MEDIA_DATA_FILENAME: &str = "source_media_data.json";
const EXPECTED_SOURCE_MEDIA_DATA_TYPE: &str = "source_media_config";

const BACKUP_LOG_DATA_DIR_NAME: &str = "backup_log_data";
const EXPECTED_BACKUP_LOG_DATA_TYPE: &str = "backup_log_data";
const BACKUP_LOG_JSON_EXPECTED_MAJOR: u32 = 0;
const BACKUP_LOG_JSON_MIN_CAPABILITY_LEVEL: u32 = 0;

#[derive(Deserialize, Serialize)]
struct DataStructureVersion {
    major: u32,
    capability_level: u32,
}

#[derive(Deserialize)]
struct DeviceEntry {
    names: Vec<String>,
    bought: Option<u64>,
    id: String,
    exhaustive: bool,
    manual_update: bool,
    device_type: Vec<String>,
}

#[derive(Deserialize)]
struct DevicesConfig {
    data_type: String,
    data_structure_version: DataStructureVersion,
    devices: Vec<DeviceEntry>,
}

#[derive(Deserialize)]
struct SourceMediaUniqueIdentification {
    serial_number: String,
}

#[derive(Clone, Debug)]
pub enum CardNamingScheme {
    /// Sequential CARD#### IDs, auto-generated. JSON value: "CARD%04d"
    Card,
    /// User must always supply the ID manually. JSON value: "freeform"
    Freeform,
}

impl<'de> serde::Deserialize<'de> for CardNamingScheme {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "CARD%04d" => Ok(CardNamingScheme::Card),
            "freeform"  => Ok(CardNamingScheme::Freeform),
            _ => Err(serde::de::Error::custom(format!("Unknown card naming scheme: '{}'", s))),
        }
    }
}

#[derive(Deserialize)]
struct SourceMediaInfo {
    device_make_name: String,
    device_model_name: String,
    device_model_name_pretty: Option<String>,
    device_unique_identification: SourceMediaUniqueIdentification,
    new_card_naming_scheme: CardNamingScheme,
}

#[derive(Deserialize)]
struct SourceMediaConfigHeader {
    data_type: String,
    data_structure_version: DataStructureVersion,
}

#[derive(Deserialize)]
struct SourceMediaConfig {
    source_media_info: SourceMediaInfo,
}

#[derive(Clone)]
pub struct StorageDeviceEntry {
    pub id: String,
    pub display_name: String,
}

#[derive(Clone)]
struct SourceMediaEntry {
    device_make_name: String,
    device_model_name: String,
    device_model_name_pretty: Option<String>,
    serial_number: String,
    new_card_naming_scheme: CardNamingScheme,
    directory: PathBuf, // The directory from which this source media configuration was loaded.
}

// backup logs files

#[derive(Deserialize)]
struct BackupLogHeader {
    data_type: String,
    data_structure_version: DataStructureVersion,
}

#[derive(Deserialize)]
struct BackupLogEntry {
    previous_uuidv7: Option<String>,
    current_uuidv7: String,
    next_uuidv7: Option<String>,
    comment: Option<String>,
    completed_backup: bool,
    new_transfers: Vec<BackupLogTransfer>,
}

#[derive(Deserialize)]
struct BackupLogTransfer {
    card_path: PathBuf,
    medium_uuidv7: Option<String>,
}

enum BackupLogState {
    UseExistingEntry(BackupLogEntry),
    CreateNewEntry { previous_uuidv7: Option<String> },
}

#[derive(Deserialize, Serialize)]
struct MainConfig {
    data_type: String,
    data_structure_version: String,
    allow_device_list: Vec<String>,
    ignore_device_list: Vec<String>,
}

fn parse_config_file(config_file_path:PathBuf) -> Result<MainConfig> {
    if ! config_file_path.exists(){
        print!("Config file doesn't exist. Create an empty one? (y/n): ");
        let _ = io::stdout().flush();
        let mut buffer = String::new();
        io::stdin().read_line(&mut buffer)?;
        if buffer != "y\n" {
            process::exit(0);
        }
        let new_config = MainConfig{
            data_type: "ingest_and_snapshot_config".to_string(),
            data_structure_version: "v0.0".to_string(),
            allow_device_list: [].to_vec(),
            ignore_device_list: [].to_vec(),
        };

        let mut config_file = File::create(config_file_path)?;
        config_file.write_all( serde_json::to_string(&new_config)?.as_bytes())?;
        Ok(new_config)
    }else{
        match std::fs::read_to_string(&config_file_path) {
            Ok(data) => Ok(serde_json::from_str(&data)?),
            Err(e) => {
                eprintln!("Failed to read config file {:?}: {}", config_file_path, e);
                Err(e.into())
            }
        }
    }
}

fn is_read_only(path: PathBuf) -> nix::Result<bool> {
    let stats = statvfs(&path)?;
    Ok(stats.flags().contains(FsFlags::ST_RDONLY))
}

#[derive(Parser)]
#[clap(author, version, about)]
struct Cli {
    #[arg(short='c', long="config")]
    config: Option<PathBuf>,

    #[arg(short='m', long="media-dir")]
    media_dir: Option<PathBuf>,
}

fn parse_backup_log_file(path: &PathBuf) -> Result<BackupLogEntry, String> {
    let raw_json = std::fs::read_to_string(path)
        .map_err(|e| format!("{}: failed to read: {}", path.display(), e))?;

    let header = serde_json::from_str::<BackupLogHeader>(&raw_json)
        .map_err(|e| format!("{}: failed to parse JSON: {}", path.display(), e))?;

    if header.data_type != EXPECTED_BACKUP_LOG_DATA_TYPE {
        return Err(format!("{}: unexpected data_type '{}' (expected '{}')",
            path.display(), header.data_type, EXPECTED_BACKUP_LOG_DATA_TYPE));
    }
    if header.data_structure_version.major != BACKUP_LOG_JSON_EXPECTED_MAJOR {
        return Err(format!(
            "{}: unsupported data_structure_version: major {} is not supported (expected {})",
            path.display(), header.data_structure_version.major, BACKUP_LOG_JSON_EXPECTED_MAJOR
        ));
    }
    if header.data_structure_version.capability_level < BACKUP_LOG_JSON_MIN_CAPABILITY_LEVEL {
        return Err(format!(
            "{}: unsupported data_structure_version: capability_level {} is below minimum {}",
            path.display(), header.data_structure_version.capability_level, BACKUP_LOG_JSON_MIN_CAPABILITY_LEVEL
        ));
    }

    serde_json::from_str::<BackupLogEntry>(&raw_json)
        .map_err(|e| format!("{}: failed to parse entry: {}", path.display(), e))
}

fn load_backup_log(media_dir: &PathBuf) -> Result<BackupLogState, String> {
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

fn scan_source_media(media_dir: &PathBuf) -> Result<(Vec<SourceMediaEntry>, Vec<String>), String> {
    let source_media_dir = media_dir.join(SOURCE_MEDIA_DIR_NAME);

    if !source_media_dir.exists() {
        return Err(format!("{}: directory not found", source_media_dir.display()));
    }

    let subdirs = match std::fs::read_dir(&source_media_dir) {
        Ok(entries) => entries,
        Err(e) => {
            return Err(format!("{}: failed to read directory: {}", source_media_dir.display(), e));
        }
    };

    let mut warnings: Vec<String> = Vec::new();
    let mut entries: Vec<SourceMediaEntry> = Vec::new();

    for subdir_result in subdirs {
        let subdir = match subdir_result {
            Ok(entry) => entry,
            Err(e) => {
                warnings.push(format!("Failed to read item in directory {}: {}", source_media_dir.display(), e));
                continue;
            }
        };

        let json_path = subdir.path().join(SOURCE_MEDIA_DATA_FILENAME);
        if !json_path.exists() {
            continue;
        }

        let raw_json = match std::fs::read_to_string(&json_path) {
            Ok(data) => data,
            Err(e) => {
                warnings.push(format!("{}: failed to read: {}", json_path.display(), e));
                continue;
            }
        };

        let header = match serde_json::from_str::<SourceMediaConfigHeader>(&raw_json) {
            Ok(h) => h,
            Err(e) => {
                warnings.push(format!("{}: failed to parse JSON: {}", json_path.display(), e));
                continue;
            }
        };

        if header.data_type != EXPECTED_SOURCE_MEDIA_DATA_TYPE {
            warnings.push(format!(
                "{}: unexpected data_type '{}' (expected '{}')",
                json_path.display(), header.data_type, EXPECTED_SOURCE_MEDIA_DATA_TYPE
            ));
            continue;
        }

        if header.data_structure_version.major != SOURCE_MEDIA_CONFIG_EXPECTED_MAJOR {
            warnings.push(format!(
                "{}: unsupported major version {} (expected {})",
                json_path.display(),
                header.data_structure_version.major,
                SOURCE_MEDIA_CONFIG_EXPECTED_MAJOR
            ));
            continue;
        }

        if header.data_structure_version.capability_level < SOURCE_MEDIA_CONFIG_MIN_CAPABILITY_LEVEL {
            warnings.push(format!(
                "{}: capability_level {} is below minimum {}",
                json_path.display(),
                header.data_structure_version.capability_level,
                SOURCE_MEDIA_CONFIG_MIN_CAPABILITY_LEVEL
            ));
            continue;
        }

        let config = match serde_json::from_str::<SourceMediaConfig>(&raw_json) {
            Ok(c) => c,
            Err(e) => {
                warnings.push(format!("{}: failed to parse JSON: {}", json_path.display(), e));
                continue;
            }
        };

        entries.push(SourceMediaEntry {
            device_make_name:         config.source_media_info.device_make_name,
            device_model_name:        config.source_media_info.device_model_name,
            device_model_name_pretty: config.source_media_info.device_model_name_pretty,
            serial_number:            config.source_media_info.device_unique_identification.serial_number,
            new_card_naming_scheme:   config.source_media_info.new_card_naming_scheme,
            directory:                subdir.path(),
        });
    }

    Ok((entries, warnings))
}

/// Returns true if `by_id_name` (a `/dev/disk/by-id/` entry without the directory prefix)
/// matches an `allow_entry` from the config list.
///
/// Matching rules:
/// - Exact match always works.
/// - Prefix match: if the entry does NOT contain `:` (i.e. has no LUN suffix), then any
///   by-id name that starts with `{entry}-` also matches. This lets a single entry cover
///   all LUN variants of an unidentifiable device (e.g. `usb-Generic_Card_Reader` matches
///   both `usb-Generic_Card_Reader-0:0` and `usb-Generic_Card_Reader-0:1`).
///   Entries that already include a `:` are treated as fully-qualified and only match exactly,
///   which prevents accidentally matching partition entries like `usb-Foo-0:0-part1`.
fn device_name_matches(by_id_name: &str, allow_entry: &str) -> bool {
    if by_id_name == allow_entry {
        return true;
    }
    if !allow_entry.contains(':') {
        return by_id_name.starts_with(&format!("{}-", allow_entry));
    }
    false
}

/// Returns the `/dev/disk/by-id/` name to use for a device if it is on the allow list
/// and not on the ignore list. Returns `None` if the device should be skipped.
fn device_by_id_name(device: &udev::Device, allow_list: &[String], ignore_list: &[String]) -> Option<String> {
    let devlinks = device.property_value("DEVLINKS")?;
    let links = devlinks.to_string_lossy();

    let by_id_names: Vec<String> = links
        .split_whitespace()
        .filter_map(|link| link.strip_prefix("/dev/disk/by-id/").map(str::to_owned))
        .collect();

    // Ignore list wins: if any by-id name matches an ignore entry, skip the device
    for name in &by_id_names {
        if ignore_list.iter().any(|entry| device_name_matches(name, entry)) {
            return None;
        }
    }

    // Return the first by-id name that matches an allow entry
    for name in &by_id_names {
        if allow_list.iter().any(|entry| device_name_matches(name, entry)) {
            return Some(name.clone());
        }
    }

    None
}

#[cfg_attr(feature = "dummy-ui-data", allow(unreachable_code))]
fn main() {
    #[cfg(feature = "dummy-ui-data")]
    dummy_ui_data::run();

    let cli = Cli::parse();

    let config_file_path = match cli.config {
        Some(file) => file,
        None => {
            let home = home_dir().expect("Could not determine home directory");
            home.join("ingest_and_snapshot_config.json")
        }
    };

    let media_dir = match cli.media_dir {
        Some(path) => path,
        None => env::current_dir().unwrap(),
    };

    let media_version_path = media_dir.join("structure_version");
    if ! media_version_path.exists() {
        eprintln!("Invalid media directory");
        process::exit(1);
    }
    let media_version = std::fs::read_to_string(media_version_path).unwrap();
    if media_version != "v3.0-dev\n" &&
        media_version != "v2.1\n" &&
        media_version != "v2.0\n"
    {
        eprintln!("Invalid media version");
        process::exit(1);
    }

    if is_read_only(media_dir.clone()).unwrap() {
        eprintln!("media is mounted read-only");
        process::exit(1);
    }

    let config = parse_config_file(config_file_path).unwrap();

    let (ui_to_logic_tx, ui_to_logic_rx) = crossbeam_channel::unbounded::<ui_api::UiToLogicMessage>();
    let tui_backend = ui::TuiBackend::new(ui_to_logic_tx);

    let mut ui: Arc<Mutex<Box<dyn ui_api::UiBackend>>> = Arc::new(Mutex::new(Box::new(tui_backend)));

    let registry = Arc::new(Mutex::new(transfer_registry::PendingTransferRegistry::new()));

    let allow_device_list = config.allow_device_list.clone();
    let ignore_device_list = config.ignore_device_list.clone();
    ui.lock().unwrap().add_config(config.allow_device_list, config.ignore_device_list).unwrap();

    let devices_config: DevicesConfig = {
        let devices_path = media_dir.join("metadata/devices.json");

        let result: Result<DevicesConfig, String> = (|| {
            if !devices_path.exists() {
                return Err(format!("{}: file not found",devices_path.display()));
            }
            let data = std::fs::read_to_string(&devices_path)
                .map_err(|e| format!("{}: failed to read file: {}", devices_path.display(), e))?;
            let dc = serde_json::from_str::<DevicesConfig>(&data)
                .map_err(|e| format!("{}: failed to parse json: {}", devices_path.display(), e))?;
            if dc.data_type != "media_devices" {
                return Err(format!("{}: wrong data type: expected 'media_devices', found '{}'", devices_path.display(), dc.data_type));
            }
            if dc.data_structure_version.major != DEVICES_JSON_EXPECTED_MAJOR {
                return Err(format!("{}: unsupported data_structure_version: major {} is not supported (expected {})",devices_path.display(), dc.data_structure_version.major, DEVICES_JSON_EXPECTED_MAJOR));
            }
            if dc.data_structure_version.capability_level < DEVICES_JSON_MIN_CAPABILITY_LEVEL {
                return Err(format!("{}: unsupported data_structure_version: capability_level {} is below minimum {}",devices_path.display(), dc.data_structure_version.capability_level, DEVICES_JSON_MIN_CAPABILITY_LEVEL));
            }
            Ok(dc)
        })();

        match result {
            Ok(dc) => dc,
            Err(msg) => {
                let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
                ui.lock().unwrap().user_query(ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
                    error: ui_api::FatalErrorKind::DevicesJson(msg),
                    response_tx,
                }), true).unwrap();
                let _ = response_rx.recv();
                ui.lock().unwrap().quit().unwrap();
                if let Ok(mutex) = Arc::try_unwrap(ui) { mutex.into_inner().unwrap().join(); }
                process::exit(1);
            }
        }
    };

    let storage_devices: Vec<StorageDeviceEntry> = devices_config.devices.iter()
        .filter(|d| d.device_type.iter().any(|t| t == "storage"))
        .map(|d| StorageDeviceEntry {
            id:           d.id.clone(),
            display_name: d.names.join(", "),
        })
        .collect();

    let (source_media_entries, source_media_warnings) = match scan_source_media(&media_dir) {
        Ok(result) => result,
        Err(msg) => {
            let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
            ui.lock().unwrap().user_query(ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
                error: ui_api::FatalErrorKind::SourceMedia(msg),
                response_tx,
            }), true).unwrap();
            let _ = response_rx.recv();
            ui.lock().unwrap().quit().unwrap();
            if let Ok(mutex) = Arc::try_unwrap(ui) { mutex.into_inner().unwrap().join(); }
            process::exit(1);
        }
    };

    if !source_media_warnings.is_empty() {
        let (response_tx, _response_rx) = crossbeam_channel::unbounded::<()>();
        ui.lock().unwrap().user_query(ui_api::UserQuery::SourceMediaWarnings(ui_api::SourceMediaWarningsQuery {
            warnings: source_media_warnings,
            response_tx,
        }), false).unwrap();
    }

    if source_media_entries.is_empty() {
        let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
        ui.lock().unwrap().user_query(ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
            error: ui_api::FatalErrorKind::SourceMedia(format!(
                "{}: no valid source media configurations found",
                media_dir.join(SOURCE_MEDIA_DIR_NAME).display()
            )),
            response_tx,
        }), true).unwrap();
        let _ = response_rx.recv();
        ui.lock().unwrap().quit().unwrap();
        if let Ok(mutex) = Arc::try_unwrap(ui) { mutex.into_inner().unwrap().join(); }
        process::exit(1);
    }

    ui.lock().unwrap().set_available_devices(source_media_entries.clone()).unwrap();

    let backup_log_state = match load_backup_log(&media_dir) {
        Ok(state) => state,
        Err(msg) => {
            let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
            ui.lock().unwrap().user_query(ui_api::UserQuery::FatalError(ui_api::FatalErrorQuery {
                error: ui_api::FatalErrorKind::BackupLog(msg),
                response_tx,
            }), true).unwrap();
            let _ = response_rx.recv();
            ui.lock().unwrap().quit().unwrap();
            if let Ok(mutex) = Arc::try_unwrap(ui) { mutex.into_inner().unwrap().join(); }
            process::exit(1);
        }
    };
    match backup_log_state {
        BackupLogState::UseExistingEntry(entry) => {
            for transfer in entry.new_transfers {
                let source_media_dir = source_media_entries
                    .iter()
                    .find(|sme| media_dir.join(&transfer.card_path).starts_with(&sme.directory))
                    .map(|sme| sme.directory.to_string_lossy().into_owned()); //TODO: report to the user if it didn't get found

                let (transfer_event_tx, transfer_event_rx) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
                ui.lock().unwrap().new_transfer(source_media_dir, transfer_event_rx).unwrap();
                transfer_event_tx.send(ui_api::TransferEvent::TransferStarted { bytes_total: 1 }).unwrap();
                transfer_event_tx.send(ui_api::TransferEvent::TransferSamples(vec![
                    ui_api::TransferSample { timestamp_ms: 0, bytes_done: 1 },
                ])).unwrap();
            }
        }
        _ => {}
    }

    let monitor = MonitorBuilder::new()
        .unwrap()
        .match_subsystem("block")
        .unwrap()
        .listen()
        .unwrap();

    // by-id names of currently connected allowed block devices (the "device location" picker source)
    let mut allowed_connected_devices: Vec<String> = Vec::new();
    // maps udev syspath → by-id name so we can clean up on device removal
    let mut syspath_to_by_id: HashMap<String, String> = HashMap::new();

    // Enumerate devices that were already connected before the monitor started
    {
        let mut enumerator = Enumerator::new().unwrap();
        enumerator.match_subsystem("block").unwrap();
        let startup_devices: Vec<String> = enumerator.scan_devices().unwrap()
            .filter_map(|device| {
                device_by_id_name(&device, &allow_device_list, &ignore_device_list).map(|by_id_name| {
                    let syspath = device.syspath().to_string_lossy().into_owned();
                    syspath_to_by_id.insert(syspath, by_id_name.clone());
                    by_id_name
                })
            })
            .collect();
        for by_id_name in &startup_devices {
            if !allowed_connected_devices.contains(by_id_name) {
                allowed_connected_devices.push(by_id_name.clone());
            }
        }
        for by_id_name in startup_devices {
            transfer_logic::spawn_transfer(
                Arc::clone(&ui),
                Arc::clone(&registry),
                source_media_entries.clone(),
                storage_devices.clone(),
                allowed_connected_devices.clone(),
                transfer_logic::DetectedTransferInfo {
                    source_media:  None,
                    card_id:       None,
                    source_device: None,
                    device_location: Some(by_id_name),
                },
            );
        }
    }

    'outer: loop {
        thread::sleep(time::Duration::from_millis(50));
        if let Ok(msg) = ui_to_logic_rx.try_recv() {
            match msg {
                ui_api::UiToLogicMessage::Quit => {
                    ui.lock().unwrap().quit().unwrap();
                    break 'outer;
                }
                ui_api::UiToLogicMessage::StartManualTransfer => {
                    transfer_logic::spawn_transfer(
                        Arc::clone(&ui),
                        Arc::clone(&registry),
                        source_media_entries.clone(),
                        storage_devices.clone(),
                        allowed_connected_devices.clone(),
                        transfer_logic::DetectedTransferInfo {
                            source_media:  None,
                            card_id:       None,
                            source_device: None,
                            device_location: None,
                        },
                    );
                }
            }
        }

        for event in monitor.iter() {
            let device = event.device();
            let syspath = device.syspath().to_string_lossy().into_owned();
            if device.action() == Some(OsStr::new("add")) {
                if let Some(by_id_name) = device_by_id_name(&device, &allow_device_list, &ignore_device_list) {
                    if !allowed_connected_devices.contains(&by_id_name) {
                        allowed_connected_devices.push(by_id_name.clone());
                    }
                    syspath_to_by_id.insert(syspath, by_id_name.clone());
                    transfer_logic::spawn_transfer(
                        Arc::clone(&ui),
                        Arc::clone(&registry),
                        source_media_entries.clone(),
                        storage_devices.clone(),
                        allowed_connected_devices.clone(),
                        transfer_logic::DetectedTransferInfo {
                            source_media:  None,
                            card_id:       None,
                            source_device: None,
                            device_location: Some(by_id_name),
                        },
                    );
                }
            } else if device.action() == Some(OsStr::new("remove")) {
                if let Some(by_id_name) = syspath_to_by_id.remove(&syspath) {
                    // Only remove from the available list if no other connected device shares the same by-id name
                    if !syspath_to_by_id.values().any(|id| id == &by_id_name) {
                        allowed_connected_devices.retain(|id| id != &by_id_name);
                    }
                }
            }
        }
    }
    loop {
        match Arc::try_unwrap(ui) {
            Ok(mutex) => {
                mutex.into_inner().unwrap().join();
                break;
            }
            Err(arc) => {
                ui = arc;
                thread::sleep(time::Duration::from_millis(10));
            }
        }
    }

}
