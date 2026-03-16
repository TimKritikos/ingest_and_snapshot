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

use std::path::PathBuf;
use std::io;
use std::io::Write;
use std::process;
use std::env;
use std::fs::File;
use std::{thread, time};
use std::sync::mpsc::{self, Sender, Receiver};
use clap::Parser;
use home::home_dir;
use anyhow::{Result};
use nix::sys::statvfs::statvfs;
use nix::sys::statvfs::FsFlags;
use serde::{Deserialize, Serialize};

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
        let  _ = config_file.write_all( serde_json::to_string(&new_config)?.as_bytes());
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
        eprintln!("media is mounter read-only");
        process::exit(1);
    }

    let config = parse_config_file(config_file_path).unwrap();

    let (tx, rx): (Sender<ui::UiMessage>, Receiver<ui::UiMessage>) = mpsc::channel();
    let ui_handle = ui::init(rx);

    tx.send(ui::UiMessage::AddConfig{allow:config.allow_device_list, ignore:config.ignore_device_list}).unwrap();

    let ten_millis = time::Duration::from_millis(5000);

    thread::sleep(ten_millis);
    tx.send(ui::UiMessage::Quit).unwrap();

    ui_handle.join().unwrap();
}
