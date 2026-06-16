use crate::backup_log::*;
use crate::transfer_logic::TransferSample;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::path::PathBuf;

    /// Checks that every key present in `written` also exists in `original` with the same
    /// value, and that every key present in `original` still exists in `written`
    /// This catches both silent field addition and silent field removal.
    fn assert_same_fields(original: &Value, written: &Value, path: &str) {
        match (original, written) {
            (Value::Object(orig_map), Value::Object(written_map)) => {
                for key in written_map.keys() {
                    assert!(
                        orig_map.contains_key(key),
                        "field '{}' was added at '{}'", key, path
                    );
                }
                for key in orig_map.keys() {
                    assert!(
                        written_map.contains_key(key),
                        "field '{}' was removed at '{}'", key, path
                    );
                }
                for (key, orig_val) in orig_map {
                    assert_same_fields(orig_val, &written_map[key], &format!("{}.{}", path, key));
                }
            }
            (Value::Array(orig_arr), Value::Array(written_arr)) => {
                assert_eq!(
                    orig_arr.len(), written_arr.len(),
                    "array length changed at '{}'", path
                );
                for (i, (orig, written)) in orig_arr.iter().zip(written_arr.iter()).enumerate() {
                    assert_same_fields(orig, written, &format!("{}[{}]", path, i));
                }
            }
            _ => assert_eq!(original, written, "value changed at '{}'", path),
        }
    }

    /// Reads a log file via the normal load → from_existing path, triggers a flush via
    /// update_transfer_samples (matching no transfer, so no data changes), then returns
    /// the re-written JSON for comparison.
    fn round_trip(log_json: &Value) -> Value {
        let tempdir = tempfile::tempdir().unwrap();
        let media_dir = tempdir.path().to_path_buf();
        let log_dir = media_dir.join("metadata").join(BACKUP_LOG_DATA_DIR_NAME);
        std::fs::create_dir_all(&log_dir).unwrap();

        let uuid = log_json["current_uuidv7"].as_str().unwrap();
        std::fs::write(
            log_dir.join(format!("{}.json", uuid)),
            serde_json::to_string_pretty(log_json).unwrap().as_bytes(),
        ).unwrap();

        let entry = match load_backup_log(&media_dir).unwrap() {
            BackupLogState::UseExistingEntry(e) => e,
            BackupLogState::CreateNewEntry { .. } => panic!("expected UseExistingEntry"),
        };

        let mut manager = BackupLogManager::from_existing(log_dir.clone(), entry);

        manager.update_transfer_samples(Path::new("__no_match__"), vec![]).unwrap();

        let written = std::fs::read_to_string(log_dir.join(format!("{}.json", uuid))).unwrap();
        serde_json::from_str(&written).unwrap()
    }

    /// Builds a BackupLogEntry with every field populated.
    ///
    /// This struct literal is the compile-time manifest of all BackupLogEntry and
    /// BackupLogTransfer fields. Adding a new field to either struct without updating
    /// this literal is a compile error, which forces the round-trip test and the
    /// next_uuidv7 test to cover it automatically.
    fn make_maximal_entry() -> BackupLogEntry {
        BackupLogEntry {
            data_type: "backup_log_data".to_owned(),
            data_structure_version: BackupLogStructureVersion { major: 0, capability_level: 0 },
            previous_uuidv7: Some("test-uuid-prev".to_owned()),
            current_uuidv7:  "test-uuid-maximal".to_owned(),
            // next_uuidv7 is None: load_backup_log follows the link when it is set, so
            // the round_trip helper cannot exercise an entry that carries it. Its write
            // path is covered by creating_new_entry_writes_next_uuidv7_on_previous_entry_and_nothing_else.
            next_uuidv7:      None,
            comment:          Some("My first backup session".to_owned()),
            completed_backup: false,
            new_transfers: vec![BackupLogTransferEntry {
                transfer_uuidv7:            Some("019ec37e-1b9a-73c8-b1d7-5444113e1b2e".to_owned()),
                card_path:                  PathBuf::from("source_media/cam/DATA/CARD0003"),
                card_id:                    Some("CARD0003".to_owned()),
                source_media_overridden:    Some(true),
                card_id_overridden:         Some(false),
                medium_uuidv7:              Some("018c0000-0000-7000-8000-000000000000".to_owned()),
                medium_uuidv7_overridden:   Some(false),
                device_location:            Some("usb-Generic_Card_Reader-0:0".to_owned()),
                device_location_overridden: Some(false),
                input_path:                 Some(PathBuf::from("/DCIM")),
                input_path_overridden:      Some(true),
                transfer_samples:           Some(vec![
                    TransferSample { timestamp_ms: 1000, bytes_done: 1024 },
                    TransferSample { timestamp_ms: 2000, bytes_done: 2048 },
                ]),
                transfer_performed_by:      Some("ingest_and_snapshot 0.1.0".to_owned()),
                bytes_total_measured:       Some(2048),
                transfer_failed:            Some(true),
                failure_message:            Some("test failure".to_owned()),
                system_hostname:            Some("test-host".to_owned()),
            }],
        }
    }

    /// Converts a BackupLogEntry into the on-disk JSON format.
    ///
    /// Since BackupLogEntry serialises directly to the on-disk format, this is a
    /// straight conversion. The assertion on next_uuidv7 is kept because the
    /// round-trip helper cannot exercise that link-following path (load_backup_log
    /// follows next_uuidv7 chains, so the entry returned is always the tail).
    fn entry_to_log_json(entry: &BackupLogEntry) -> Value {
        assert!(entry.next_uuidv7.is_none(),
            "entry_to_log_json only supports next_uuidv7 = None; \
             use set_next_uuidv7_on_entry to write it onto an existing file");
        serde_json::to_value(entry).unwrap()
    }

    #[test]
    fn round_trip_does_not_add_or_modify_fields() {
        // Minimal old-format entry: only the fields that have always been required.
        // All optional entry-level and transfer-level fields are absent.
        let minimal = serde_json::json!({
            "data_type": "backup_log_data",
            "data_structure_version": {"major": 0, "capability_level": 0},
            "current_uuidv7": "test-uuid-minimal",
            "completed_backup": false,
            "new_transfers": [
                {"card_path": "source_media/cam/DATA/CARD0001"}
            ]
        });

        // Entry with a representative subset of optional fields set.
        // The fields that are present must survive unchanged; absent ones must not appear.
        let partial = serde_json::json!({
            "data_type": "backup_log_data",
            "data_structure_version": {"major": 0, "capability_level": 0},
            "previous_uuidv7": "test-uuid-prev",
            "current_uuidv7": "test-uuid-partial",
            "completed_backup": false,
            "new_transfers": [
                {
                    "card_path": "source_media/cam/DATA/CARD0002",
                    "card_id": "CARD0002",
                    "device_location": "usb-Generic_Card_Reader-0:0",
                    "device_location_overridden": false,
                    "transfer_performed_by": "ingest_and_snapshot 0.1.0",
                    "transfer_samples": [
                        {"timestamp_ms": 1000, "bytes_done": 512}
                    ]
                }
            ]
        });

        // Maximal entry covering every field. Generated from make_maximal_entry() so that
        // adding a new field to the structs without updating that function is a compile
        // error — and the round-trip test automatically covers the new field.
        let maximal = entry_to_log_json(&make_maximal_entry());

        for log_json in [&minimal, &partial, &maximal] {
            let written = round_trip(log_json);
            assert_same_fields(log_json, &written, "");
        }
    }

    #[test]
    fn creating_new_entry_writes_next_uuidv7_on_previous_entry_and_nothing_else() {
        let tempdir = tempfile::tempdir().unwrap();
        let log_dir = tempdir.path().to_path_buf();
        std::fs::create_dir_all(&log_dir).unwrap();

        // Use make_maximal_entry() as the body so this test stays in sync with the
        // field manifest — a new field that isn't preserved by set_next_uuidv7_on_entry
        // will be caught here too.
        let prev_entry_data = BackupLogEntry {
            current_uuidv7:  "test-uuid-prev".to_owned(),
            previous_uuidv7: None,
            ..make_maximal_entry()
        };
        let prev_uuid = prev_entry_data.current_uuidv7.clone();
        let prev_entry_json = entry_to_log_json(&prev_entry_data);

        std::fs::write(
            log_dir.join(format!("{}.json", prev_uuid)),
            serde_json::to_string_pretty(&prev_entry_json).unwrap().as_bytes(),
        ).unwrap();

        // Creating a new entry linked to prev_uuid triggers set_next_uuidv7_on_entry.
        let _ = BackupLogManager::create_new(log_dir.clone(), Some(prev_uuid.clone())).unwrap();

        let updated_str = std::fs::read_to_string(log_dir.join(format!("{}.json", prev_uuid))).unwrap();
        let updated: serde_json::Value = serde_json::from_str(&updated_str).unwrap();

        // next_uuidv7 must now be set and point to the new entry's file
        let next_uuid = updated["next_uuidv7"].as_str()
            .expect("next_uuidv7 should have been added as a string");
        assert!(
            log_dir.join(format!("{}.json", next_uuid)).exists(),
            "next_uuidv7 must point to an existing file"
        );

        // Every other field must be unchanged — remove next_uuidv7 and compare
        let mut updated_without_next = updated.clone();
        updated_without_next.as_object_mut().unwrap().remove("next_uuidv7");
        assert_same_fields(&prev_entry_json, &updated_without_next, "");
    }
}
