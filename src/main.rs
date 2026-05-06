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

    if is_read_only(media_dir).unwrap() {
        eprintln!("media is mounted read-only");
        process::exit(1);
    }

    let config = parse_config_file(config_file_path).unwrap();

    let (logic_to_ui_tx, logic_to_ui_rx): (Sender<ui::LogicToUiMessage>, Receiver<ui::LogicToUiMessage>) = mpsc::channel();
    let (ui_to_logic_tx, ui_to_logic_rx): (Sender<ui::UiToLogicMessage>, Receiver<ui::UiToLogicMessage>) = mpsc::channel();
    let ui_handle = ui::init(logic_to_ui_rx,ui_to_logic_tx);

    logic_to_ui_tx.send(ui::LogicToUiMessage::AddConfig{allow:config.allow_device_list, ignore:config.ignore_device_list}).unwrap();

    // Dummy transfers for UI development/testing
    let logic_to_ui_tx_dummy = logic_to_ui_tx.clone();
    thread::spawn(move || {
        let now_ms = || -> u64 {
            SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
        };

        thread::sleep(time::Duration::from_millis(300));

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
        let samples1: Vec<(u64, u64)> = speed_profile.iter().enumerate().map(|(i, &spd_mbps)| {
            bytes1 = (bytes1 + spd_mbps * 1_000_000 * interval_ms / 1000).min(total1);
            (t_start + i as u64 * interval_ms, bytes1)
        }).collect();
        tx1.send(ui::TransferEvent::TransferStarted { bytes_total: total1 }).unwrap();
        tx1.send(ui::TransferEvent::TransferSamples(samples1)).unwrap();
        // No TransferFinished needed — the UI transitions to Finished when bytes_done >= bytes_total

        // Transfer 2: waiting — device detected, user query pending
        let (tx2, rx2) = mpsc::channel::<ui::TransferEvent>();
        logic_to_ui_tx_dummy.send(ui::LogicToUiMessage::NewTransfer {
            name: "/dev/disk/by-id/usb-Kingston_DataTraveler_3.0_BB020406-0:0".to_string(),
            camera_name: "Nikon Z9".to_string(),
            rx_control: rx2,
        }).unwrap();
        //tx2.send(ui::TransferEvent::UserQuery {
        //    question: "Allow transfer from Kingston DataTraveler 3.0? (y/n)".to_string(),
        //}).unwrap();

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
                if tx4.send(ui::TransferEvent::TransferSamples(vec![(now_ms(), b4)])).is_err() { break; }
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
            let _tx2_keep_alive = tx2; // keeps transfer 2 alive for the duration of this thread
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
                if tx3.send(ui::TransferEvent::TransferSamples(vec![(now_ms(), bytes3)])).is_err() { break; }
                if bytes3 >= total3 { break; }
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
