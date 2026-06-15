use super::*;
use std::path::{Path, PathBuf};

fn make_storage_device(id: &str) -> crate::StorageDeviceEntry {
    crate::StorageDeviceEntry {
        id: id.to_string(),
        display_name: id.to_string(),
    }
}

fn make_source_media_entry(directory: PathBuf) -> crate::SourceMediaEntry {
    crate::SourceMediaEntry {
        device_make_name: "TestMake".to_string(),
        device_model_name: "TestModel".to_string(),
        device_model_name_pretty: None,
        serial_number: "SN001".to_string(),
        new_card_naming_scheme: crate::CardNamingScheme::CardFourDigits,
        directory,
    }
}

fn write_config(dir: &Path, json: &str) {
    std::fs::write(dir.join(CONFIG_FILE_NAME), json).unwrap();
}

fn config_with_transfers(transfers_json: &str) -> String {
    format!(
        r#"{{"data_type": "ingest_and_snapshot_per_device_config", "data_structure_version": {{"major": 0, "capability_level": 0}}, "transfers": [{}]}}"#,
        transfers_json
    )
}

#[test]
fn test_config_file_not_found() {
    let temp = tempfile::tempdir().unwrap();
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_ok());
}

#[test]
fn test_malformed_json_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), "not valid json {{{");
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
}

#[test]
fn test_wrong_data_type_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_type": "some_other_type", "data_structure_version": {"major": 0, "capability_level": 0}, "transfers": []}"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("incorrect type"));
}

#[test]
fn test_missing_data_type_field_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_structure_version": {"major": 0, "capability_level": 0}, "transfers": []}"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
}

#[test]
fn test_missing_structure_version_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_type": "ingest_and_snapshot_per_device_config", "transfers": []}"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
}

#[test]
fn test_wrong_major_version_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_type": "ingest_and_snapshot_per_device_config", "data_structure_version": {"major": 1, "capability_level": 0}, "transfers": []}"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported major version"));
}

#[test]
fn test_higher_capability_level_than_required_is_accepted() {
    // A file written for a newer minor revision (capability_level=1) should still load
    // fine on software that only requires capability_level=0.
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_type": "ingest_and_snapshot_per_device_config", "data_structure_version": {"major": 0, "capability_level": 1}, "transfers": []}"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_ok());
}

#[test]
fn test_unknown_json_fields_are_silently_ignored() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{"data_type": "ingest_and_snapshot_per_device_config", "data_structure_version": {"major": 0, "capability_level": 0}, "unknown_field": 42, "transfers": []}"#,
    );
    assert!(load_per_device_config(temp.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_empty_transfers_array_produces_empty_result() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), &config_with_transfers(""));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert!(result.is_empty());
}

#[test]
fn test_transfer_with_no_input_path_produces_one_override() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), &config_with_transfers("{}"));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].input_path.is_none());
    assert!(result[0].source_media.is_none());
    assert!(result[0].storage_device.is_none());
}

#[test]
fn test_transfer_with_empty_input_path_array_produces_one_override() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), &config_with_transfers(r#"{"input_path": []}"#));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert!(result[0].input_path.is_none());
    assert!(result[0].source_media.is_none());
    assert!(result[0].storage_device.is_none());
}

#[test]
fn test_transfer_with_single_input_path_produces_one_override() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), &config_with_transfers(r#"{"input_path": ["/DCIM"]}"#));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM")));
    assert!(result[0].source_media.is_none());
    assert!(result[0].storage_device.is_none());
}

#[test]
fn test_multiple_input_paths_expand_into_separate_overrides() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"input_path": ["/DCIM", "/PRIVATE/AVCHD"]}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 2);
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM")));
    assert!(result[0].source_media.is_none());
    assert!(result[0].storage_device.is_none());
    assert_eq!(result[1].input_path, Some(PathBuf::from("/PRIVATE/AVCHD")));
    assert!(result[1].source_media.is_none());
    assert!(result[1].storage_device.is_none());
}

#[test]
fn test_multiple_transfer_entries_each_expand_independently() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        r#"{
            "data_type": "ingest_and_snapshot_per_device_config",
            "data_structure_version": {"major": 0, "capability_level": 0},
            "transfers": [
                {"input_path": ["/A", "/B"]},
                {"input_path": ["/C"]}
            ]
        }"#,
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 3);
}

#[test]
fn test_relative_input_path_gets_leading_slash() {
    let temp = tempfile::tempdir().unwrap();
    write_config(temp.path(), &config_with_transfers(r#"{"input_path": ["DCIM"]}"#));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM")));
}

#[test]
fn test_absolute_input_path_is_preserved_as_is() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"input_path": ["/DCIM/100MEDIA"]}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM/100MEDIA")));
}

#[test]
fn test_unknown_storage_device_id_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"storage_device": "missing_device_id"}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("missing_device_id"));
}

#[test]
fn test_known_storage_device_id_is_forwarded_to_override() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"storage_device": "hdd_01"}"#),
    );
    let devices = vec![make_storage_device("hdd_01")];
    let result = load_per_device_config(temp.path(), devices, vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].storage_device.as_deref(), Some("hdd_01"));
}

#[test]
fn test_unknown_source_media_path_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"source_media": "/nonexistent/camera"}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
}

#[test]
fn test_known_source_media_path_is_forwarded_to_override() {
    let temp = tempfile::tempdir().unwrap();
    let camera_dir = temp.path().join("my_camera");
    std::fs::create_dir(&camera_dir).unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(&format!(r#"{{"source_media": "{}"}}"#, camera_dir.display())),
    );
    let source_media = vec![make_source_media_entry(camera_dir.clone())];
    let result = load_per_device_config(temp.path(), vec![], source_media).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].source_media, Some(camera_dir));
}

#[test]
fn test_storage_device_and_source_media_fields_propagate_to_all_expanded_overrides() {
    let temp = tempfile::tempdir().unwrap();
    let camera_dir = temp.path().join("my_camera");
    std::fs::create_dir(&camera_dir).unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(&format!(
            r#"{{"storage_device": "hdd_01", "source_media": "{}", "input_path": ["/A", "/B"]}}"#,
            camera_dir.display()
        )),
    );
    let devices = vec![make_storage_device("hdd_01")];
    let source_media = vec![make_source_media_entry(camera_dir.clone())];
    let result = load_per_device_config(temp.path(), devices, source_media).unwrap();
    assert_eq!(result.len(), 2);
    for override_entry in &result {
        assert_eq!(override_entry.storage_device.as_deref(), Some("hdd_01"));
        assert_eq!(override_entry.source_media, Some(camera_dir.clone()));
    }
}

#[test]
fn test_separate_transfer_entries_do_not_cross_contaminate_each_other() {
    let temp = tempfile::tempdir().unwrap();
    let camera_a_dir = temp.path().join("camera_a");
    let camera_b_dir = temp.path().join("camera_b");
    std::fs::create_dir(&camera_a_dir).unwrap();
    std::fs::create_dir(&camera_b_dir).unwrap();
    write_config(
        temp.path(),
        &format!(
            r#"{{
                "data_type": "ingest_and_snapshot_per_device_config",
                "data_structure_version": {{"major": 0, "capability_level": 0}},
                "transfers": [
                    {{"storage_device": "hdd_01", "source_media": "{}", "input_path": ["/A1", "/A2"]}},
                    {{"storage_device": "hdd_02", "source_media": "{}", "input_path": ["/B1", "/B2"]}}
                ]
            }}"#,
            camera_a_dir.display(),
            camera_b_dir.display(),
        ),
    );
    let devices = vec![make_storage_device("hdd_01"), make_storage_device("hdd_02")];
    let source_media = vec![
        make_source_media_entry(camera_a_dir.clone()),
        make_source_media_entry(camera_b_dir.clone()),
    ];
    let result = load_per_device_config(temp.path(), devices, source_media).unwrap();
    assert_eq!(result.len(), 4);

    // First entry's two paths both belong to hdd_01 / camera_a
    assert_eq!(result[0].storage_device.as_deref(), Some("hdd_01"));
    assert_eq!(result[0].source_media, Some(camera_a_dir.clone()));
    assert_eq!(result[0].input_path, Some(PathBuf::from("/A1")));
    assert_eq!(result[1].storage_device.as_deref(), Some("hdd_01"));
    assert_eq!(result[1].source_media, Some(camera_a_dir.clone()));
    assert_eq!(result[1].input_path, Some(PathBuf::from("/A2")));

    // Second entry's two paths both belong to hdd_02 / camera_b
    assert_eq!(result[2].storage_device.as_deref(), Some("hdd_02"));
    assert_eq!(result[2].source_media, Some(camera_b_dir.clone()));
    assert_eq!(result[2].input_path, Some(PathBuf::from("/B1")));
    assert_eq!(result[3].storage_device.as_deref(), Some("hdd_02"));
    assert_eq!(result[3].source_media, Some(camera_b_dir.clone()));
    assert_eq!(result[3].input_path, Some(PathBuf::from("/B2")));
}
