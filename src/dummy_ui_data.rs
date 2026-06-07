use std::sync::{Arc, Mutex};
use std::sync::mpsc;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};
use std::{thread, time, process};
use crate::ui_api;
use crate::ui::TuiBackend;
use crate::{SourceMediaEntry, CardNamingScheme};

pub fn run() -> ! {
    let now_ms = || -> u64 {
        SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
    };

    let (ui_to_logic_tx, ui_to_logic_rx) = mpsc::channel::<ui_api::UiToLogicMessage>();
    let mut ui: Arc<Mutex<Box<dyn ui_api::UiBackend>>> =
        Arc::new(Mutex::new(Box::new(TuiBackend::new(ui_to_logic_tx))));

    let dummy_source_media = vec![
                SourceMediaEntry {
                    device_make_name:         "Sony".to_string(),
                    device_model_name:        "ILCE-7M4".to_string(),
                    device_model_name_pretty: Some("A7 IV".to_string()),
                    serial_number:            "4710293".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Card,
                    directory:                PathBuf::from("/media/source_media/sony_a7iv"),
                },
                SourceMediaEntry {
                    device_make_name:         "Sony".to_string(),
                    device_model_name:        "ILCE-7RM5".to_string(),
                    device_model_name_pretty: Some("A7R V".to_string()),
                    serial_number:            "8823015".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Card,
                    directory:                PathBuf::from("/media/source_media/sony_a7rv"),
                },
                SourceMediaEntry {
                    device_make_name:         "Canon".to_string(),
                    device_model_name:        "EOS R5".to_string(),
                    device_model_name_pretty: None,
                    serial_number:            "083059002910".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Card,
                    directory:                PathBuf::from("/media/source_media/canon_eos_r5"),
                },
                SourceMediaEntry {
                    device_make_name:         "Fujifilm".to_string(),
                    device_model_name:        "GFX 100S".to_string(),
                    device_model_name_pretty: None,
                    serial_number:            "91007345".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Freeform,
                    directory:                PathBuf::from("/media/source_media/fujifilm_gfx100s"),
                },
                SourceMediaEntry {
                    device_make_name:         "Nikon".to_string(),
                    device_model_name:        "Z 9".to_string(),
                    device_model_name_pretty: Some("Z9".to_string()),
                    serial_number:            "3102948576".to_string(),
                    new_card_naming_scheme:   CardNamingScheme::Card,
                    directory:                PathBuf::from("/media/source_media/nikon_z9"),
                },
    ];

    let (cancel_tx_outer,   cancel_rx_outer)   = mpsc::channel::<()>();
    let (cancel_tx_approve, cancel_rx_approve) = mpsc::channel::<()>();
    let (cancel_tx_scan,    cancel_rx_scan)    = mpsc::channel::<()>();
    let cancel_senders = vec![cancel_tx_outer, cancel_tx_approve, cancel_tx_scan];

    {
        let ui = Arc::clone(&ui);
        let dummy_source_media = dummy_source_media.clone();
        thread::spawn(move || {
            match cancel_rx_outer.recv_timeout(time::Duration::from_millis(300)) {
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                _ => return,
            }

            ui.lock().unwrap().set_available_devices(dummy_source_media.clone()).unwrap();

            // Transfer 1: historical finished transfer (simulating a restore from saved state)
            let (tx1, rx1) = mpsc::channel::<ui_api::TransferEvent>();
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
            let (tx4, rx4) = mpsc::channel::<ui_api::TransferEvent>();
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
            let (tx3, rx3) = mpsc::channel::<ui_api::TransferEvent>();
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
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }
                let (response_tx, response_rx) = mpsc::channel::<bool>();
                ui_scan.lock().unwrap().user_query(ui_api::UserQuery::ScanNewDevice(ui_api::ScanNewDeviceQuery {
                    device_name: "Unknown USB Camera".to_string(),
                    response_tx,
                }), false).unwrap();
                let _ = response_rx.recv();
            });

            // Dummy ApproveTransfer query after 5 seconds
            let ui_approve = Arc::clone(&ui);
            thread::spawn(move || {
                let now_ms = || -> u64 {
                    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64
                };

                match cancel_rx_approve.recv_timeout(time::Duration::from_secs(5)) {
                    Err(mpsc::RecvTimeoutError::Timeout) => {}
                    _ => return,
                }

                let (tx2, rx2) = mpsc::channel::<ui_api::TransferEvent>();
                ui_approve.lock().unwrap().new_transfer(
                    Some("/media/source_media/nikon_z9".to_string()),
                    rx2,
                ).unwrap();

                let (response_tx, response_rx) = mpsc::channel::<ui_api::ApproveTransferResponse>();
                let (update_tx,   update_rx)   = mpsc::channel::<ui_api::ApproveTransferQueryUpdate>();

                ui_approve.lock().unwrap().user_query(ui_api::UserQuery::ApproveTransfer(ui_api::ApproveTransferQuery {
                    data: ui_api::ApproveTransferQueryUpdate {
                        source_media_dir:  Some("/media/source_media/nikon_z9".to_string()),
                        source_device:     "Sony SF-G 64GB (SN: 123456)".to_string(),
                        transfer_function: "rsync_archive".to_string(),
                        data_size:         12 * 1024 * 1024 * 1024,
                        card_id:           "NIKON_001".to_string(),
                        device_overridden: false,
                    },
                    response_tx,
                    update_rx,
                }), false).unwrap();

                while let Ok(msg) = response_rx.recv() {
                    match msg {
                        ui_api::ApproveTransferResponse::DeviceOverwrite(directory_opt) => {
                            let _ = tx2.send(ui_api::TransferEvent::SourceMediaChanged(directory_opt.clone()));
                            let update = match directory_opt {
                                Some(directory) => {
                                    ui_api::ApproveTransferQueryUpdate {
                                        source_media_dir:  Some(directory),
                                        source_device:     "Sony SF-G 64GB (SN: 123456)".to_string(),
                                        transfer_function: "rsync_archive".to_string(),
                                        data_size:         12 * 1024 * 1024 * 1024,
                                        card_id:           "UNKNOWN".to_string(),
                                        device_overridden: true,
                                    }
                                }
                                None => {
                                    ui_api::ApproveTransferQueryUpdate {
                                        source_media_dir:  Some("/media/source_media/nikon_z9".to_string()),
                                        source_device:     "Sony SF-G 64GB (SN: 123456)".to_string(),
                                        transfer_function: "rsync_archive".to_string(),
                                        data_size:         12 * 1024 * 1024 * 1024,
                                        card_id:           "NIKON_001".to_string(),
                                        device_overridden: false,
                                    }
                                }
                            };
                            let _ = update_tx.send(update);
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
                        ui_api::ApproveTransferResponse::CardIdChanged(_) => {}
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
                ui_api::UiToLogicMessage::Quit => {
                    drop(cancel_senders);
                    break;
                }
                ui_api::UiToLogicMessage::StartManualTransfer => {
                    let (transfer_event_tx, transfer_event_rx) = mpsc::channel::<ui_api::TransferEvent>();
                    ui.lock().unwrap().new_transfer(
                        None,
                        transfer_event_rx,
                    ).unwrap();

                    let (response_tx, response_rx) = mpsc::channel::<ui_api::ApproveTransferResponse>();
                    let (update_tx, update_rx) = mpsc::channel::<ui_api::ApproveTransferQueryUpdate>();
                    ui.lock().unwrap().user_query(ui_api::UserQuery::ApproveTransfer(ui_api::ApproveTransferQuery {
                        data: ui_api::ApproveTransferQueryUpdate {
                            source_media_dir:  None,
                            source_device:     String::new(),
                            transfer_function: String::new(),
                            data_size:         0,
                            card_id:           String::new(),
                            device_overridden: false,
                        },
                        response_tx,
                        update_rx,
                    }), false).unwrap();

                    thread::spawn(move || {
                        while let Ok(response) = response_rx.recv() {
                            match response {
                                ui_api::ApproveTransferResponse::DeviceOverwrite(directory_opt) => {
                                    let _ = transfer_event_tx.send(ui_api::TransferEvent::SourceMediaChanged(directory_opt.clone()));
                                    let update = ui_api::ApproveTransferQueryUpdate {
                                        source_media_dir:  directory_opt,
                                        source_device:     String::new(),
                                        transfer_function: String::new(),
                                        data_size:         0,
                                        card_id:           String::new(),
                                        device_overridden: true,
                                    };
                                    let _ = update_tx.send(update);
                                }
                                ui_api::ApproveTransferResponse::CardIdChanged(_) => {}
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
