use std::sync::{Arc, Mutex};
use crossbeam_channel;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{thread, time, process};
use crate::ui_api;
use crate::ui::TuiBackend;
use crate::{SourceMediaEntry, CardNamingScheme, StorageDeviceEntry};
use crate::transfer_logic::{TransferFields, TransferFieldState};

pub fn run() -> ! {
    let now_ms = || -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
    };

    let (ui_to_logic_tx, ui_to_logic_rx) = crossbeam_channel::unbounded::<ui_api::UiToLogicMessage>();
    let mut ui: Arc<Mutex<Box<dyn ui_api::UiBackend>>> =
        Arc::new(Mutex::new(Box::new(TuiBackend::new(ui_to_logic_tx))));

    let dummy_source_media = vec![
                SourceMediaEntry {
                    device_make_name:         "Sony".to_string(),
                    device_model_name:        "ILCE-7M4".to_string(),
                    device_model_name_pretty: Some("A7 IV".to_string()),
                    serial_number:            "4710293".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::CardFourDigits,
                    directory:                PathBuf::from("/media/source_media/sony_a7iv"),
                    device_thumbnail:         None,
                },
                SourceMediaEntry {
                    device_make_name:         "Sony".to_string(),
                    device_model_name:        "ILCE-7RM5".to_string(),
                    device_model_name_pretty: Some("A7R V".to_string()),
                    serial_number:            "8823015".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::CardFourDigits,
                    directory:                PathBuf::from("/media/source_media/sony_a7rv"),
                    device_thumbnail:         None,
                },
                SourceMediaEntry {
                    device_make_name:         "Canon".to_string(),
                    device_model_name:        "EOS R5".to_string(),
                    device_model_name_pretty: None,
                    serial_number:            "083059002910".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::CardFourDigits,
                    directory:                PathBuf::from("/media/source_media/canon_eos_r5"),
                    device_thumbnail:         None,
                },
                SourceMediaEntry {
                    device_make_name:         "Fujifilm".to_string(),
                    device_model_name:        "GFX 100S".to_string(),
                    device_model_name_pretty: None,
                    serial_number:            "91007345".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Freeform,
                    directory:                PathBuf::from("/media/source_media/fujifilm_gfx100s"),
                    device_thumbnail:         None,
                },
                SourceMediaEntry {
                    device_make_name:         "Nikon".to_string(),
                    device_model_name:        "Z 9".to_string(),
                    device_model_name_pretty: Some("Z9".to_string()),
                    serial_number:            "3102948576".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::CardFourDigits,
                    directory:                PathBuf::from("/media/source_media/nikon_z9"),
                    device_thumbnail:         None,
                },
    ];

    let dummy_storage_devices = vec![
        StorageDeviceEntry {
            id:           uuid::Uuid::parse_str("01940000-0000-7000-0000-000000000001").unwrap(),
            display_name: "Sony SF-G 64GB".to_string(),
            device_thumbnail: None,
        },
        StorageDeviceEntry {
            id:           uuid::Uuid::parse_str("01940000-0000-7000-0000-000000000002").unwrap(),
            display_name: "Lexar Professional 256GB, Lexar Professional 256GB Backup".to_string(),
            device_thumbnail: None,
        },
    ];

    let dummy_storage_devices_for_manual = dummy_storage_devices.clone();

    let dummy_device_locations = vec![
        "usb-Kingston_DataTraveler_3.0_60A44C3000000000-0:0".to_string(),
        "usb-Generic_USB_Card_Reader-0:0".to_string(),
    ];
    let dummy_device_locations_for_manual = dummy_device_locations.clone();

    let (cancel_tx_outer,   cancel_rx_outer)   = crossbeam_channel::unbounded::<()>();
    let (cancel_tx_approve, cancel_rx_approve) = crossbeam_channel::unbounded::<()>();
    let (cancel_tx_scan,    cancel_rx_scan)    = crossbeam_channel::unbounded::<()>();
    let cancel_senders = vec![cancel_tx_outer, cancel_tx_approve, cancel_tx_scan];

    {
        let ui = Arc::clone(&ui);
        let dummy_source_media = dummy_source_media.clone();
        thread::spawn(move || {
            match cancel_rx_outer.recv_timeout(time::Duration::from_millis(300)) {
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                _ => return,
            }

            ui.lock().unwrap().set_available_devices(dummy_source_media.clone()).unwrap();

            // One dummy mount entry per possible status, for UI development.
            let mount_ids: [u32; 4] = [0xAABBCC01, 0xAABBCC02, 0xAABBCC03, 0xAABBCC04];

            let dummy_real_device_paths = [
                PathBuf::from("/dev/sdb1"),
                PathBuf::from("/dev/sdc1"),
                PathBuf::from("/dev/sdd1"),
                PathBuf::from("/dev/sde1"),
            ];

            for ((mount_id, device_name), real_device_path) in mount_ids.iter().zip(&[
                "usb-Kingston_DataTraveler_3.0_60A44C3000000000-0:0-part1",
                "usb-Sony_Storage_Media_012345678901-0:0-part1",
                "usb-Generic_USB_Card_Reader-0:0-part1",
                "usb-SanDisk_Ultra_20060775320BB101-0:0-part1",
            ]).zip(dummy_real_device_paths.iter()) {
                ui.lock().unwrap().mount_update(ui_api::MountUpdate::MountAdded(ui_api::MountEntry {
                    id: *mount_id,
                    by_id_name: device_name.to_string(),
                    real_device_path: real_device_path.clone(),
                    mountpoint: PathBuf::from(format!("/run/ingest_and_snapshot/mounts/{:08x}", mount_id)),
                    status: ui_api::MountEntryStatus::Mounting,
                    fs_type: ui_api::LoadingField::Loading,
                })).unwrap();
            }

            // Mounted
            ui.lock().unwrap().mount_update(ui_api::MountUpdate::MountCompleted {
                id: mount_ids[1],
                fs_type: "exfat".to_string(),
            }).unwrap();

            // Failed
            ui.lock().unwrap().mount_update(ui_api::MountUpdate::MountFailed {
                id: mount_ids[2],
                reason: "Could not mount: no filesystem types matched".to_string(),
            }).unwrap();

            // UnmountFailed
            ui.lock().unwrap().mount_update(ui_api::MountUpdate::MountCompleted {
                id: mount_ids[3],
                fs_type: "vfat".to_string(),
            }).unwrap();
            ui.lock().unwrap().mount_update(ui_api::MountUpdate::UnmountFailed {
                id: mount_ids[3],
                reason: "Lazy unmount of \"/run/ingest_and_snapshot/mounts/aabbcc04\" failed: EBUSY".to_string(),
            }).unwrap();

            // Transfer 1: historical finished transfer (simulating a restore from saved state)
            let (tx1, rx1) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
            ui.lock().unwrap().new_transfer(
                Some("/media/source_media/sony_a7iv".to_string()),
                rx1,
            ).unwrap();
            let total1: u64 = 4 * 1024 * 1024 * 1024;
            let t_end = now_ms() - 5 * 60 * 1000; // finished 5 minutes ago
            let t_start = t_end - 80 * 1000;       // took 80 seconds
            let speed_profile: &[u64] = &[
                15, 32, 58, 75, 88, 95, 100, 98, 105, 110,
                108, 115, 112, 108, 102, 110, 118, 115, 108, 95,
            ];
            let interval_ms = (t_end - t_start) / speed_profile.len() as u64;
            let mut bytes1: u64 = 0;
            let samples1: Vec<ui_api::TransferSample> = speed_profile.iter().enumerate().map(|(i, &spd_mbps)| {
                bytes1 = (bytes1 + spd_mbps * 1_000_000 * interval_ms / 1000).min(total1);
                ui_api::TransferSample { timestamp_ms: t_start + i as u64 * interval_ms, bytes_done: bytes1 }
            }).collect();
            tx1.send(ui_api::TransferEvent::TransferStarted { bytes_total: total1 }).unwrap();
            tx1.send(ui_api::TransferEvent::TransferSamples(samples1)).unwrap();
            // No TransferFinished needed — the UI transitions to Finished when bytes_done >= bytes_total

            // Transfer 4: two-phase speed test — live, visually verify x-axis is % completion.
            // First half of data at 15 MB/s (slow), second half at 120 MB/s (fast).
            // The left half of the chart should have short bars, right half tall bars.
            let (tx4, rx4) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
            ui.lock().unwrap().new_transfer(
                Some("/media/source_media/canon_eos_r5".to_string()),
                rx4,
            ).unwrap();

            // 100 MB total: 50 MB slow (~3.3 s) then 50 MB fast (~0.4 s)
            let total4:           u64 = 100 * 1024 * 1024;
            let slow_bps:         u64 = 15  * 1024 * 1024; // 15  MB/s
            let fast_bps:         u64 = 120 * 1024 * 1024; // 120 MB/s
            let bytes_per_sample: u64 = 2   * 1024 * 1024; // 2 MB per sample
            let slow_ms:          u64 = bytes_per_sample * 1000 / slow_bps; // ~133 ms
            let fast_ms:          u64 = bytes_per_sample * 1000 / fast_bps; //  ~17 ms

            tx4.send(ui_api::TransferEvent::TransferStarted { bytes_total: total4 }).unwrap();
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
                    if tx4.send(ui_api::TransferEvent::TransferSamples(vec![ui_api::TransferSample { timestamp_ms: now_ms(), bytes_done: b4 }])).is_err() { break; }
                    if b4 >= total4 { break; }
                }
            });

            // Transfer 3: live in-progress — 50 samples/sec with varied speed
            let (tx3, rx3) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
            ui.lock().unwrap().new_transfer(
                Some("/media/source_media/fujifilm_gfx100s".to_string()),
                rx3,
            ).unwrap();

            // Total sized so the bar reaches ~85% over the demo run (visually informative)
            let total3: u64 = 1024 * 1024 * 1024; // 1 GB
            tx3.send(ui_api::TransferEvent::TransferStarted { bytes_total: total3 }).unwrap();

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
                    if tx3.send(ui_api::TransferEvent::TransferSamples(vec![ui_api::TransferSample { timestamp_ms: now_ms(), bytes_done: bytes3 }])).is_err() { break; }
                    if bytes3 >= total3 { break; }
                }
            });

            // Dummy ScanNewDevice query after 10 seconds
            let ui_scan = Arc::clone(&ui);
            thread::spawn(move || {
                match cancel_rx_scan.recv_timeout(time::Duration::from_secs(10)) {
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::UnknownDeviceResponse>();
                ui_scan.lock().unwrap().user_query(ui_api::UserQuery::UnknownDevice(ui_api::UnknownDeviceQuery {
                    device_name: "usb-Unknown_USB_Camera-0:0".to_string(),
                    response_tx,
                }), false).unwrap();
                let _ = response_rx.recv();
            });

            // Dummy ApproveTransfer query after 5 seconds
            let ui_approve = Arc::clone(&ui);
            let dummy_storage_devices_approve = dummy_storage_devices.clone();
            thread::spawn(move || {
                let now_ms = || -> u64 {
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
                };

                match cancel_rx_approve.recv_timeout(time::Duration::from_secs(5)) {
                    Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }

                let (tx2, rx2) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
                ui_approve.lock().unwrap().new_transfer(
                    Some("/media/source_media/nikon_z9".to_string()),
                    rx2,
                ).unwrap();

                let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::ApproveTransferResponse>();
                let (update_tx,   update_rx)   = crossbeam_channel::unbounded::<TransferFields>();

                let dummy_device_locations_approve = dummy_device_locations.clone();
                // A dummy auto-detected block device: (real device node, by-id name).
                let auto_detected_device_location: (PathBuf, String) =
                    (PathBuf::from("/dev/sdz1"), dummy_device_locations_approve[0].clone());

                let mut fields = TransferFields {
                    card_id_detected:         Some("NIKON_001".to_string()),
                    card_id_selected:         TransferFieldState::AutoSelected,
                    source_media_detected:    Some(PathBuf::from("/media/source_media/nikon_z9")),
                    source_media_selected:    TransferFieldState::AutoSelected,
                    storage_device_detected:  Some(dummy_storage_devices_approve[0].id),
                    storage_device_selected:  TransferFieldState::AutoSelected,
                    device_location_detected: Some(auto_detected_device_location.clone()),
                    device_location_selected: TransferFieldState::AutoSelected,
                    input_path_detected:      None,
                    input_path_selected:      TransferFieldState::NotSelected,
                    comment:                  None,
                    mount_root:               None,
                };

                ui_approve.lock().unwrap().user_query(ui_api::UserQuery::ApproveTransfer(Box::new(ui_api::ApproveTransferQuery {
                    fields: fields.clone(),
                    response_tx,
                    update_rx,
                    available_storage_devices: dummy_storage_devices_approve.clone(),
                    available_device_locations: dummy_device_locations_approve.clone(),
                })), false).unwrap();

                while let Ok(msg) = response_rx.recv() {
                    match msg {
                        ui_api::ApproveTransferResponse::DeviceOverwrite(selection) => {
                            fields.source_media_selected = match selection {
                                ui_api::SourceMediaSelection::Auto                  => TransferFieldState::AutoSelected,
                                ui_api::SourceMediaSelection::Overridden(directory) => TransferFieldState::Overridden(PathBuf::from(directory)),
                            };
                            let _ = tx2.send(ui_api::TransferEvent::SourceMediaChanged(
                                fields.source_media().map(|p| p.to_string_lossy().into_owned())));
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::StorageDeviceChanged(device_id) => {
                            fields.storage_device_selected = TransferFieldState::Overridden(device_id);
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::StorageDeviceAuto => {
                            fields.storage_device_selected = TransferFieldState::AutoSelected;
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::DeviceLocationChanged(name) => {
                            fields.device_location_selected =
                                TransferFieldState::Overridden((PathBuf::from("/dev/sdz1"), name));
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::DeviceLocationAuto => {
                            fields.device_location_selected = TransferFieldState::AutoSelected;
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::InputPathChanged(new_path) => {
                            fields.input_path_selected = TransferFieldState::Overridden(new_path);
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::CommentChanged(new_comment) => {
                            fields.comment = if new_comment.is_empty() { None } else { Some(new_comment) };
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::CardIdChanged(new_id) => {
                            fields.card_id_selected = if new_id.is_empty() {
                                TransferFieldState::AutoSelected
                            } else {
                                TransferFieldState::Overridden(new_id)
                            };
                            let _ = update_tx.send(fields.clone());
                        }
                        ui_api::ApproveTransferResponse::Approved => {
                            // Start the Nikon Z9 transfer: 500 MB, steady 40–65 MB/s
                            let total2: u64 = 500 * 1024 * 1024;
                            let _ = tx2.send(ui_api::TransferEvent::TransferStarted { bytes_total: total2 });
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
                                    if tx2.send(ui_api::TransferEvent::TransferSamples(vec![ui_api::TransferSample { timestamp_ms: now_ms(), bytes_done: bytes2 }])).is_err() { break; }
                                    if bytes2 >= total2 { break; }
                                }
                            });
                            break;
                        }
                        ui_api::ApproveTransferResponse::Denied => {
                            let _ = tx2.send(ui_api::TransferEvent::DeviceUnplugged);
                            break;
                        }
                    }
                }
            });
        });
    }

    loop {
        thread::sleep(time::Duration::from_millis(50));
        if let Ok(msg) = ui_to_logic_rx.try_recv() {
            match msg {
                ui_api::UiToLogicMessage::Quit |
                ui_api::UiToLogicMessage::CompleteBackupAndExit => {
                    drop(cancel_senders);
                    break;
                }
                ui_api::UiToLogicMessage::UnmountRequest(_) => {}
                ui_api::UiToLogicMessage::StartSnapshot => {
                    // Dummy mode cannot touch ZFS, so this simulates the check-terminal flow:
                    // it asks for a name, then streams scripted output exercising SGR colours and
                    // cursor-movement control codes before offering a "Return" button.
                    let ui_for_snapshot = Arc::clone(&ui);
                    thread::spawn(move || {
                        const RETURN_ACTION_ID: u32 = 5;

                        let (name_tx, name_rx) = crossbeam_channel::unbounded::<ui_api::SnapshotNameResponse>();
                        if ui_for_snapshot.lock().unwrap().user_query(
                            ui_api::UserQuery::SnapshotName(ui_api::SnapshotNameQuery { response_tx: name_tx }),
                            false,
                        ).is_err() { return; }
                        let message = match name_rx.recv() {
                            Ok(ui_api::SnapshotNameResponse::Provided(message)) => message,
                            _ => return,
                        };

                        let (updates_tx, updates_rx) = crossbeam_channel::unbounded::<ui_api::SnapshotUpdate>();
                        let (action_tx, action_rx)   = crossbeam_channel::unbounded::<u32>();
                        if ui_for_snapshot.lock().unwrap().start_check_terminal(updates_rx, action_tx).is_err() { return; }

                        let send = |bytes: Vec<u8>| { let _ = updates_tx.send(ui_api::SnapshotUpdate::Terminal(bytes)); };

                        send(format!("\r\n\x1b[36mGenerating snapshot temp_{} ...\x1b[0m\r\n", message).into_bytes());
                        send(b"\r\n\x1b[36mExecuting check program (dummy) ...\x1b[0m\r\n\r\n".to_vec());
                        for file_number in 1..=5 {
                            thread::sleep(time::Duration::from_millis(400));
                            send(format!("Checking file {} ... \x1b[32mOK\x1b[0m\r\n", file_number).into_bytes());
                        }
                        // Demonstrate cursor moves: go up two lines, forward 20 columns, overwrite.
                        thread::sleep(time::Duration::from_millis(400));
                        send(b"\x1b[2A\x1b[20C\x1b[33m<- revisited\x1b[0m\x1b[2B\r\n".to_vec());

                        let _ = updates_tx.send(ui_api::SnapshotUpdate::SetActions(vec![
                            ui_api::SnapshotActionButton {
                                id: RETURN_ACTION_ID,
                                label: "Return to main screen".to_string(),
                                style: ui_api::SnapshotActionStyle::Confirm,
                            },
                        ]));
                        send(b"\r\n\x1b[32mDone (dummy). Select \"Return to main screen\".\x1b[0m\r\n".to_vec());

                        loop {
                            match action_rx.recv() {
                                Ok(RETURN_ACTION_ID) => break,
                                Ok(_) => {}
                                Err(_) => return,
                            }
                        }
                        let _ = updates_tx.send(ui_api::SnapshotUpdate::Exit);
                    });
                }
                ui_api::UiToLogicMessage::StartManualTransfer => {
                    let (transfer_event_tx, transfer_event_rx) = crossbeam_channel::unbounded::<ui_api::TransferEvent>();
                    ui.lock().unwrap().new_transfer(
                        None,
                        transfer_event_rx,
                    ).unwrap();

                    let (response_tx, response_rx) = crossbeam_channel::unbounded::<ui_api::ApproveTransferResponse>();
                    let (update_tx, update_rx) = crossbeam_channel::unbounded::<TransferFields>();
                    let dummy_storage_devices_manual = dummy_storage_devices_for_manual.clone();
                    let dummy_device_locations_manual = dummy_device_locations_for_manual.clone();

                    let mut fields = TransferFields {
                        card_id_detected:         None,
                        card_id_selected:         TransferFieldState::NotSelected,
                        source_media_detected:    None,
                        source_media_selected:    TransferFieldState::NotSelected,
                        storage_device_detected:  None,
                        storage_device_selected:  TransferFieldState::NotSelected,
                        device_location_detected: None,
                        device_location_selected: TransferFieldState::NotSelected,
                        input_path_detected:      None,
                        input_path_selected:      TransferFieldState::NotSelected,
                        comment:                  None,
                        mount_root:               None,
                    };

                    ui.lock().unwrap().user_query(ui_api::UserQuery::ApproveTransfer(Box::new(ui_api::ApproveTransferQuery {
                        fields: fields.clone(),
                        response_tx,
                        update_rx,
                        available_storage_devices: dummy_storage_devices_manual.clone(),
                        available_device_locations: dummy_device_locations_manual.clone(),
                    })), false).unwrap();

                    thread::spawn(move || {
                        while let Ok(response) = response_rx.recv() {
                            match response {
                                ui_api::ApproveTransferResponse::DeviceOverwrite(selection) => {
                                    fields.source_media_selected = match selection {
                                        ui_api::SourceMediaSelection::Auto                  => TransferFieldState::NotSelected,
                                        ui_api::SourceMediaSelection::Overridden(directory) => TransferFieldState::Overridden(PathBuf::from(directory)),
                                    };
                                    let _ = transfer_event_tx.send(ui_api::TransferEvent::SourceMediaChanged(
                                        fields.source_media().map(|p| p.to_string_lossy().into_owned())));
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::StorageDeviceChanged(device_id) => {
                                    fields.storage_device_selected = TransferFieldState::Overridden(device_id);
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::StorageDeviceAuto => {}
                                ui_api::ApproveTransferResponse::CardIdChanged(new_id) => {
                                    fields.card_id_selected = if new_id.is_empty() {
                                        TransferFieldState::NotSelected
                                    } else {
                                        TransferFieldState::Overridden(new_id)
                                    };
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::DeviceLocationChanged(name) => {
                                    fields.device_location_selected =
                                        TransferFieldState::Overridden((PathBuf::from("/dev/sdz1"), name));
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::DeviceLocationAuto => {
                                    fields.device_location_selected = TransferFieldState::NotSelected;
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::InputPathChanged(new_path) => {
                                    fields.input_path_selected = TransferFieldState::Overridden(new_path);
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::CommentChanged(new_comment) => {
                                    fields.comment = if new_comment.is_empty() { None } else { Some(new_comment) };
                                    let _ = update_tx.send(fields.clone());
                                }
                                ui_api::ApproveTransferResponse::Approved => {
                                    // TODO: start actual dummy transfer
                                    break;
                                }
                                ui_api::ApproveTransferResponse::Denied => {
                                    let _ = transfer_event_tx.send(ui_api::TransferEvent::DeviceUnplugged);
                                    break;
                                }
                            }
                        }
                    });
                }
            }
        }
    }

    // Wait for all spawned threads to drop their Arc references, then tell the UI to
    // quit and join its thread so ratatui can restore the terminal before we exit.
    loop {
        match Arc::try_unwrap(ui) {
            Ok(mutex) => {
                let mut backend = mutex.into_inner().unwrap();
                backend.quit().unwrap();
                backend.join();
                break;
            }
            Err(arc) => {
                ui = arc;
                thread::sleep(time::Duration::from_millis(10));
            }
        }
    }
    process::exit(0);
}
