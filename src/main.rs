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
#[cfg(feature = "dummy-ui-data")]
use std::time::{SystemTime, UNIX_EPOCH};
use std::sync::mpsc::{self, Sender, Receiver};
use clap::Parser;
use home::home_dir;
use anyhow::{Result};
use nix::sys::statvfs::statvfs;
use nix::sys::statvfs::FsFlags;
use serde::{Deserialize, Serialize};
use udev::MonitorBuilder;
use std::ffi::OsStr;

mod ui;

const DEVICES_JSON_EXPECTED_MAJOR: u32 = 0;
const DEVICES_JSON_MIN_CAPABILITY_LEVEL: u32 = 0;

const SOURCE_MEDIA_DIR_NAME: &str = "source_media";

const SOURCE_MEDIA_CONFIG_EXPECTED_MAJOR: u32 = 0;
const SOURCE_MEDIA_CONFIG_MIN_CAPABILITY_LEVEL: u32 = 1;
const SOURCE_MEDIA_DATA_FILENAME: &str = "source_media_data.json";
const EXPECTED_SOURCE_MEDIA_DATA_TYPE: &str = "source_media_config";

#[derive(Deserialize)]
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

#[derive(Deserialize)]
struct SourceMediaInfo {
    id: String,
    device_make_name: String,
    device_model_name: String,
    device_model_name_pretty: Option<String>,
    device_unique_identification: SourceMediaUniqueIdentification,
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

struct SourceMediaEntry {
    id: String,
    device_make_name: String,
    device_model_name: String,
    device_model_name_pretty: Option<String>,
    serial_number: String,
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
            id:                       config.source_media_info.id,
            device_make_name:         config.source_media_info.device_make_name,
            device_model_name:        config.source_media_info.device_model_name,
            device_model_name_pretty: config.source_media_info.device_model_name_pretty,
            serial_number:            config.source_media_info.device_unique_identification.serial_number,
        });
    }

    Ok((entries, warnings))
}

fn main() {
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

    let (logic_to_ui_tx, logic_to_ui_rx): (Sender<ui::LogicToUiMessage>, Receiver<ui::LogicToUiMessage>) = mpsc::channel();
    let (ui_to_logic_tx, ui_to_logic_rx): (Sender<ui::UiToLogicMessage>, Receiver<ui::UiToLogicMessage>) = mpsc::channel();
    let ui_handle = ui::init(logic_to_ui_rx,ui_to_logic_tx);

    logic_to_ui_tx.send(ui::LogicToUiMessage::AddConfig{allow:config.allow_device_list, ignore:config.ignore_device_list}).unwrap();

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
                let (response_tx, response_rx) = mpsc::channel::<()>();
                logic_to_ui_tx.send(ui::LogicToUiMessage::UserQuery(
                    ui::UserQuery::FatalError(ui::FatalErrorQuery {
                        error: ui::FatalErrorKind::DevicesJson(msg),
                        response_tx,
                    })
                )).unwrap();
                let _ = response_rx.recv();
                logic_to_ui_tx.send(ui::LogicToUiMessage::Quit).unwrap();
                ui_handle.join().unwrap();
                process::exit(1);
            }
        }
    };

    let (source_media_entries, source_media_warnings) = match scan_source_media(&media_dir) {
        Ok(result) => result,
        Err(msg) => {
            let (response_tx, response_rx) = mpsc::channel::<()>();
            logic_to_ui_tx.send(ui::LogicToUiMessage::UserQuery(
                ui::UserQuery::FatalError(ui::FatalErrorQuery {
                    error: ui::FatalErrorKind::SourceMedia(msg),
                    response_tx,
                })
            )).unwrap();
            let _ = response_rx.recv();
            logic_to_ui_tx.send(ui::LogicToUiMessage::Quit).unwrap();
            ui_handle.join().unwrap();
            process::exit(1);
        }
    };

    if !source_media_warnings.is_empty() {
        let (response_tx, response_rx) = mpsc::channel::<()>();
        logic_to_ui_tx.send(ui::LogicToUiMessage::UserQuery(
            ui::UserQuery::SourceMediaWarnings(ui::SourceMediaWarningsQuery {
                warnings: source_media_warnings,
                response_tx,
            })
        )).unwrap();
    }

    if source_media_entries.is_empty() {
        let (response_tx, response_rx) = mpsc::channel::<()>();
        logic_to_ui_tx.send(ui::LogicToUiMessage::UserQuery(
            ui::UserQuery::FatalError(ui::FatalErrorQuery {
                error: ui::FatalErrorKind::SourceMedia(format!(
                    "{}: no valid source media configurations found",
                    media_dir.join(SOURCE_MEDIA_DIR_NAME).display()
                )),
                response_tx,
            })
        )).unwrap();
        let _ = response_rx.recv();
        logic_to_ui_tx.send(ui::LogicToUiMessage::Quit).unwrap();
        ui_handle.join().unwrap();
        process::exit(1);
    }

    // Dummy UI data for development/testing
    #[cfg(feature = "dummy-ui-data")]
    let logic_to_ui_tx_dummy = logic_to_ui_tx.clone();
    #[cfg(feature = "dummy-ui-data")]
    thread::spawn(move || {
        let now_ms = || -> u64 {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
        };

        thread::sleep(time::Duration::from_millis(300));

        logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::SetAvailableDevices(vec![
            "Sony A7 IV".to_string(),
            "Sony A7R V".to_string(),
            "Canon EOS R5".to_string(),
            "Fujifilm GFX 100S".to_string(),
            "Nikon Z9".to_string(),
        ])).unwrap();

        // Transfer 1: historical finished transfer (simulating a restore from saved state)
        let (tx1, rx1) = mpsc::channel::<ui::TransferEvent>();
        logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::NewTransfer {
            name: "/dev/disk/by-id/usb-SanDisk_Ultra_USB_3.0_AA010203-0:0".to_string(),
            camera_name: "Sony A7 IV".to_string(),
            rx_control: rx1,
        }).unwrap();
        let total1: u64 = 4 * 1024 * 1024 * 1024;
        let t_end = now_ms() - 5 * 60 * 1000; // finished 5 minutes ago
        let t_start = t_end - 80 * 1000;       // took 80 seconds
        let speed_profile: &[u64] = &[
            15, 32, 58, 75, 88, 95, 100, 98, 105, 110,
            108, 115, 112, 108, 102, 110, 118, 115, 108, 95,
        ];
        let interval_ms = (t_end - t_start) / speed_profile.len() as u64;
        let mut bytes1: u64 = 0;
        let samples1: Vec<ui::TransferSample> = speed_profile.iter().enumerate().map(|(i, &spd_mbps)| {
            bytes1 = (bytes1 + spd_mbps * 1_000_000 * interval_ms / 1000).min(total1);
            ui::TransferSample { timestamp_ms: t_start + i as u64 * interval_ms, bytes_done: bytes1 }
        }).collect();
        tx1.send(ui::TransferEvent::TransferStarted { bytes_total: total1 }).unwrap();
        tx1.send(ui::TransferEvent::TransferSamples(samples1)).unwrap();
        // No TransferFinished needed — the UI transitions to Finished when bytes_done >= bytes_total

        // Transfer 4: two-phase speed test — live, visually verify x-axis is % completion.
        // First half of data at 15 MB/s (slow), second half at 120 MB/s (fast).
        // The left half of the chart should have short bars, right half tall bars.
        let (tx4, rx4) = mpsc::channel::<ui::TransferEvent>();
        logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::NewTransfer {
            name: "/dev/disk/by-id/usb-TwoPhase_SpeedTest-0:0".to_string(),
            camera_name: "Canon EOS R5".to_string(),
            rx_control: rx4,
        }).unwrap();

        // 100 MB total: 50 MB slow (~3.3 s) then 50 MB fast (~0.4 s)
        let total4:          u64 = 100 * 1024 * 1024;
        let slow_bps:        u64 = 15  * 1024 * 1024; // 15  MB/s
        let fast_bps:        u64 = 120 * 1024 * 1024; // 120 MB/s
        let bytes_per_sample: u64 = 2 * 1024 * 1024;  // 2 MB per sample
        let slow_ms:         u64 = bytes_per_sample * 1000 / slow_bps; // ~133 ms
        let fast_ms:         u64 = bytes_per_sample * 1000 / fast_bps; //  ~17 ms

        tx4.send(ui::TransferEvent::TransferStarted { bytes_total: total4 }).unwrap();
        thread::spawn(move || {
            let now_ms = || -> u64 {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
            };
            let mut b4: u64 = 0;
            let half4 = total4 / 2;
            loop {
                let sleep_ms = if b4 < half4 { slow_ms } else { fast_ms };
                thread::sleep(time::Duration::from_millis(sleep_ms));
                b4 = (b4 + bytes_per_sample).min(total4);
                if tx4.send(ui::TransferEvent::TransferSamples(vec![ui::TransferSample { timestamp_ms: now_ms(), bytes_done: b4 }])).is_err() { break; }
                if b4 >= total4 { break; }
            }
        });

        // Transfer 3: live in-progress — 50 samples/sec with varied speed
        let (tx3, rx3) = mpsc::channel::<ui::TransferEvent>();
        logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::NewTransfer {
            name: "/dev/disk/by-id/usb-WD_Elements_25A3_CC030609-0:0".to_string(),
            camera_name: "Fujifilm GFX 100S".to_string(),
            rx_control: rx3,
        }).unwrap();

        // Total sized so the bar reaches ~85% over the demo run (visually informative)
        let total3: u64 = 1024 * 1024 * 1024; // 1 GB
        tx3.send(ui::TransferEvent::TransferStarted { bytes_total: total3 }).unwrap();

        // Speed profile in MB/s; cycled at 50 Hz with noise.
        // Ramp-up phase followed by a plateau with periodic dips, bursts, and noise.
        let speed_profile_mbs: &[u64] = &[
            // ramp-up (~0.5 s)
            12, 18, 26, 36, 46, 56, 65, 72, 78, 83, 87, 90, 92, 94, 95, 96, 97, 98, 99, 100,
            // plateau — varied (80-entry block cycled over remaining steps)
            102, 105, 108, 112, 115, 118, 115, 112, 108, 105,
            102, 100,  98,  95,  92,  88,  85,  82,  80,  78, // dip (buffer flush)
             82,  88,  92,  96, 100, 105, 110, 115, 118, 120,
            122, 125, 128, 130, 132, 130, 128, 125, 122, 118,
            115, 112, 108, 105, 102, 100,  98,  96,  94,  92,
             90,  88,  85,  82,  80,  78,  76,  74,  72,  70, // second dip
             75,  80,  86,  92,  98, 104, 110, 115, 118, 120,
            122, 125, 128, 132, 135, 132, 128, 124, 120, 116,
        ];

        thread::spawn(move || {
            let now_ms = || -> u64 {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
            };
            let mut bytes3: u64 = 0;
            let mut i = 0u64;
            loop {
                thread::sleep(time::Duration::from_millis(20));
                let base_mbs = speed_profile_mbs[i as usize % speed_profile_mbs.len()];
                let noise_pct = ((i * 11 + 7) % 25) as i64 - 12;
                let mbs = ((base_mbs as i64 + base_mbs as i64 * noise_pct / 100).max(5)) as u64;
                let step_bytes = mbs * 1024 * 1024 / 50;
                bytes3 = (bytes3 + step_bytes).min(total3);
                i += 1;
                if tx3.send(ui::TransferEvent::TransferSamples(vec![ui::TransferSample { timestamp_ms: now_ms(), bytes_done: bytes3 }])).is_err() { break; }
                if bytes3 >= total3 { break; }
            }
        });

        // Dummy ScanNewDevice query after 10 seconds
        let logic_to_ui_tx_dummy2 = logic_to_ui_tx_dummy.clone();
        thread::spawn(move || {
            thread::sleep(time::Duration::from_secs(10));
            let (response_tx, response_rx) = mpsc::channel::<bool>();
            logic_to_ui_tx_dummy2.send(ui::LogicToUiMessage::UserQuery(
                ui::UserQuery::ScanNewDevice(ui::ScanNewDeviceQuery {
                    device_name: "Unknown USB Camera".to_string(),
                    response_tx,
                })
            )).unwrap();
            let _ = response_rx.recv();
        });

        // Create Dummy query after 5 seconds
        thread::spawn(move || {
            let now_ms = || -> u64 {
                SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
            };

            thread::sleep(time::Duration::from_secs(5));

            let (tx2, rx2) = mpsc::channel::<ui::TransferEvent>();
            logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::NewTransfer {
                name:        "/dev/disk/by-id/usb-Kingston_DataTraveler_3.0_BB020406-0:0".to_string(),
                camera_name: "Nikon Z9".to_string(),
                rx_control:  rx2,
            }).unwrap();

            let (response_tx, response_rx) = mpsc::channel::<ui::ApproveTransferResponse>();
            let (update_tx,   update_rx)   = mpsc::channel::<ui::ApproveTransferQueryUpdate>();

            logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::UserQuery(
                ui::UserQuery::ApproveTransfer(ui::ApproveTransferQuery {
                    data: ui::ApproveTransferQueryUpdate {
                        device_product_name: "Nikon Z9".to_string(),
                        brand:               "Nikon".to_string(),
                        serial_number:       "3102948576".to_string(),
                        source_device:       "Sony SF-G 64GB (SN: 123456)".to_string(),
                        transfer_function:   "rsync_archive".to_string(),
                        archive_directory:   "/media/archive/2026/05/".to_string(),
                        data_size:           12 * 1024 * 1024 * 1024,
                        card_id:             "NIKON_001".to_string(),
                        device_overridden:   false,
                    },
                    response_tx,
                    update_rx,
                })
            )).unwrap();

            while let Ok(msg) = response_rx.recv() {
                match msg {
                    ui::ApproveTransferResponse::DeviceOverwrite(name_opt) => {
                        let update = match name_opt {
                            Some(name) => {
                                let _ = tx2.send(ui::TransferEvent::CameraNameChanged(name.clone()));
                                ui::ApproveTransferQueryUpdate {
                                    device_product_name: name,
                                    brand:               "Unknown".to_string(),
                                    serial_number:       "N/A".to_string(),
                                    source_device:       "Sony SF-G 64GB (SN: 123456)".to_string(),
                                    transfer_function:   "rsync_archive".to_string(),
                                    archive_directory:   "/media/archive/2026/05/".to_string(),
                                    data_size:           12 * 1024 * 1024 * 1024,
                                    card_id:             "UNKNOWN".to_string(),
                                    device_overridden:   true,
                                }
                            }
                            None => {
                                let _ = tx2.send(ui::TransferEvent::CameraNameChanged("Nikon Z9".to_string()));
                                ui::ApproveTransferQueryUpdate {
                                    device_product_name: "Nikon Z9".to_string(),
                                    brand:               "Nikon".to_string(),
                                    serial_number:       "3102948576".to_string(),
                                    source_device:       "Sony SF-G 64GB (SN: 123456)".to_string(),
                                    transfer_function:   "rsync_archive".to_string(),
                                    archive_directory:   "/media/archive/2026/05/".to_string(),
                                    data_size:           12 * 1024 * 1024 * 1024,
                                    card_id:             "NIKON_001".to_string(),
                                    device_overridden:   false,
                                }
                            }
                        };
                        let _ = update_tx.send(update);
                    }
                    ui::ApproveTransferResponse::Approved => {
                        // Start the Nikon Z9 transfer: 500 MB, steady 40–65 MB/s
                        let total2: u64 = 500 * 1024 * 1024;
                        let _ = tx2.send(ui::TransferEvent::TransferStarted { bytes_total: total2 });
                        thread::spawn(move || {
                            let speed_profile_mbs: &[u64] = &[
                                // ramp-up
                                 8, 15, 24, 33, 40, 44, 46, 48, 49, 50,
                                // steady plateau with gentle variation
                                51, 53, 55, 57, 58, 57, 55, 53, 51, 50,
                                49, 48, 47, 46, 48, 51, 54, 57, 60, 62,
                                63, 62, 60, 57, 54, 51, 48, 46, 48, 51,
                                54, 57, 60, 63, 65, 63, 60, 57, 54, 51,
                            ];
                            let mut bytes2: u64 = 0;
                            let mut j = 0u64;
                            loop {
                                thread::sleep(time::Duration::from_millis(20));
                                let base_mbs = speed_profile_mbs[j as usize % speed_profile_mbs.len()];
                                let noise_pct = ((j * 7 + 3) % 21) as i64 - 10;
                                let mbs = ((base_mbs as i64 + base_mbs as i64 * noise_pct / 100).max(5)) as u64;
                                bytes2 = (bytes2 + mbs * 1024 * 1024 / 50).min(total2);
                                j += 1;
                                if tx2.send(ui::TransferEvent::TransferSamples(vec![ui::TransferSample { timestamp_ms: now_ms(), bytes_done: bytes2 }])).is_err() { break; }
                                if bytes2 >= total2 { break; }
                            }
                        });
                        break;
                    }
                    ui::ApproveTransferResponse::Denied => {
                        let _ = tx2.send(ui::TransferEvent::DeviceUnplugged);
                        break;
                    }
                }
            }
        });
    });

    let monitor = MonitorBuilder::new()
        .unwrap()
        .match_subsystem("block")
        .unwrap()
        .listen()
        .unwrap();

    let mut device_senders: HashMap<String, Sender<ui::TransferEvent>> = HashMap::new();

    'outer: loop {
        thread::sleep(time::Duration::from_millis(50));
        if let Ok(msg) = ui_to_logic_rx.try_recv() {
            match msg {
                ui::UiToLogicMessage::Quit => {
                    logic_to_ui_tx.send(ui::LogicToUiMessage::Quit).unwrap();
                    break 'outer;
                },
            }
        }

        for event in monitor.iter() {
            let device = event.device();
            let syspath = device.syspath().to_string_lossy().into_owned();
            if device.action() == Some(OsStr::new("add")) && let Some(devlinks) = device.property_value("DEVLINKS") {
                // DEVLINKS is a space-separated list of symlinks
                let links = devlinks.to_string_lossy();
                for link in links.split_whitespace() {
                    if link.contains("/dev/disk/by-id/") {
                        let (tx_control, rx_control) = mpsc::channel::<ui::TransferEvent>();
                        device_senders.insert(syspath.clone(), tx_control);
                        logic_to_ui_tx.send(ui::LogicToUiMessage::NewTransfer{name: link.to_string(), camera_name: String::new(), rx_control}).unwrap();
                        break;
                    }
                }
            } else if device.action() == Some(OsStr::new("remove")) {
                if let Some(tx_control) = device_senders.remove(&syspath) {
                    let _ = tx_control.send(ui::TransferEvent::DeviceUnplugged);
                }
            }
        }
    }
    ui_handle.join().unwrap();

}
