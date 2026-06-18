use super::*;
use std::path::{Path, PathBuf};
use uuid::Uuid;

// Storage device IDs are uuidv7 values; these stand in for two distinct devices in tests.
const HDD_01: &str = "01940000-0000-7000-0000-0000000000a1";
const HDD_02: &str = "01940000-0000-7000-0000-0000000000a2";

fn uuid(id: &str) -> Uuid {
    Uuid::parse_str(id).unwrap()
}

fn make_storage_device(id: &str) -> crate::StorageDeviceEntry {
    crate::StorageDeviceEntry {
        id: uuid(id),
        display_name: id.to_string(),
        device_thumbnail: None,
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
        device_thumbnail: None,
        subdevice_id: None,
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

fn config_with_extra_fields(extra_fields_json: &str, transfers_json: &str) -> String {
    format!(
        r#"{{"data_type": "ingest_and_snapshot_per_device_config", "data_structure_version": {{"major": 0, "capability_level": 0}}, {}, "transfers": [{}]}}"#,
        extra_fields_json,
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
    std::fs::create_dir(temp.path().join("DCIM")).unwrap();
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
    std::fs::create_dir(temp.path().join("DCIM")).unwrap();
    std::fs::create_dir_all(temp.path().join("PRIVATE").join("AVCHD")).unwrap();
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
    std::fs::create_dir(temp.path().join("A")).unwrap();
    std::fs::create_dir(temp.path().join("B")).unwrap();
    std::fs::create_dir(temp.path().join("C")).unwrap();
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
    std::fs::create_dir(temp.path().join("DCIM")).unwrap();
    write_config(temp.path(), &config_with_transfers(r#"{"input_path": ["DCIM"]}"#));
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM")));
}

#[test]
fn test_absolute_input_path_is_preserved_as_is() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(temp.path().join("DCIM").join("100MEDIA")).unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"input_path": ["/DCIM/100MEDIA"]}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM/100MEDIA")));
}

#[test]
fn test_unknown_storage_device_id_returns_error() {
    // A valid uuid that is not in the (empty) device list.
    const UNKNOWN: &str = "01940000-0000-7000-0000-0000000000ff";
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(&format!(r#"{{"storage_device": "{}"}}"#, UNKNOWN)),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains(UNKNOWN));
}

#[test]
fn test_known_storage_device_id_is_forwarded_to_override() {
    let temp = tempfile::tempdir().unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(&format!(r#"{{"storage_device": "{}"}}"#, HDD_01)),
    );
    let devices = vec![make_storage_device(HDD_01)];
    let result = load_per_device_config(temp.path(), devices, vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].storage_device, Some(uuid(HDD_01)));
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
    let device = tempfile::tempdir().unwrap();
    let source_media_temp = tempfile::tempdir().unwrap();
    let source_media_dir = source_media_temp.path().join("my_camera");
    std::fs::create_dir(&source_media_dir).unwrap();
    write_config(
        device.path(),
        &config_with_transfers(&format!(r#"{{"source_media": "{}"}}"#, source_media_dir.display())),
    );
    let source_media = vec![make_source_media_entry(source_media_dir.clone())];
    let result = load_per_device_config(device.path(), vec![], source_media).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].source_media, Some(source_media_dir));
}

#[test]
fn test_storage_device_and_source_media_fields_propagate_to_all_expanded_overrides() {
    let device = tempfile::tempdir().unwrap();
    let source_media_temp = tempfile::tempdir().unwrap();
    let source_media_dir = source_media_temp.path().join("my_camera");
    std::fs::create_dir(&source_media_dir).unwrap();
    std::fs::create_dir(device.path().join("A")).unwrap();
    std::fs::create_dir(device.path().join("B")).unwrap();
    write_config(
        device.path(),
        &config_with_transfers(&format!(
            r#"{{"storage_device": "{}", "source_media": "{}", "input_path": ["/A", "/B"]}}"#,
            HDD_01,
            source_media_dir.display()
        )),
    );
    let devices = vec![make_storage_device(HDD_01)];
    let source_media = vec![make_source_media_entry(source_media_dir.clone())];
    let result = load_per_device_config(device.path(), devices, source_media).unwrap();
    assert_eq!(result.len(), 2);
    for override_entry in &result {
        assert_eq!(override_entry.storage_device, Some(uuid(HDD_01)));
        assert_eq!(override_entry.source_media, Some(source_media_dir.clone()));
    }
}

#[test]
fn test_separate_transfer_entries_do_not_cross_contaminate_each_other() {
    let device = tempfile::tempdir().unwrap();
    let source_media_a_temp = tempfile::tempdir().unwrap();
    let source_media_b_temp = tempfile::tempdir().unwrap();
    let source_media_a_dir = source_media_a_temp.path().join("camera_a");
    let source_media_b_dir = source_media_b_temp.path().join("camera_b");
    std::fs::create_dir(&source_media_a_dir).unwrap();
    std::fs::create_dir(&source_media_b_dir).unwrap();
    std::fs::create_dir(device.path().join("A1")).unwrap();
    std::fs::create_dir(device.path().join("A2")).unwrap();
    std::fs::create_dir(device.path().join("B1")).unwrap();
    std::fs::create_dir(device.path().join("B2")).unwrap();
    write_config(
        device.path(),
        &format!(
            r#"{{
                "data_type": "ingest_and_snapshot_per_device_config",
                "data_structure_version": {{"major": 0, "capability_level": 0}},
                "transfers": [
                    {{"storage_device": "{hdd_01}", "source_media": "{source_media_a}", "input_path": ["/A1", "/A2"]}},
                    {{"storage_device": "{hdd_02}", "source_media": "{source_media_b}", "input_path": ["/B1", "/B2"]}}
                ]
            }}"#,
            hdd_01 = HDD_01,
            hdd_02 = HDD_02,
            source_media_a = source_media_a_dir.display(),
            source_media_b = source_media_b_dir.display(),
        ),
    );
    let devices = vec![make_storage_device(HDD_01), make_storage_device(HDD_02)];
    let source_media = vec![
        make_source_media_entry(source_media_a_dir.clone()),
        make_source_media_entry(source_media_b_dir.clone()),
    ];
    let result = load_per_device_config(device.path(), devices, source_media).unwrap();
    assert_eq!(result.len(), 4);

    // First entry's two paths both belong to hdd_01 / source_media_a
    assert_eq!(result[0].storage_device, Some(uuid(HDD_01)));
    assert_eq!(result[0].source_media, Some(source_media_a_dir.clone()));
    assert_eq!(result[0].input_path, Some(PathBuf::from("/A1")));
    assert_eq!(result[1].storage_device, Some(uuid(HDD_01)));
    assert_eq!(result[1].source_media, Some(source_media_a_dir.clone()));
    assert_eq!(result[1].input_path, Some(PathBuf::from("/A2")));

    // Second entry's two paths both belong to hdd_02 / source_media_b
    assert_eq!(result[2].storage_device, Some(uuid(HDD_02)));
    assert_eq!(result[2].source_media, Some(source_media_b_dir.clone()));
    assert_eq!(result[2].input_path, Some(PathBuf::from("/B1")));
    assert_eq!(result[3].storage_device, Some(uuid(HDD_02)));
    assert_eq!(result[3].source_media, Some(source_media_b_dir.clone()));
    assert_eq!(result[3].input_path, Some(PathBuf::from("/B2")));
}

#[test]
fn test_unhandled_dir_triggers_error() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("MISC")).unwrap();
    write_config(device.path(), &config_with_transfers(r#"{"input_path": ["/DCIM"]}"#));
    let result = load_per_device_config(device.path(), vec![], vec![]);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("MISC"));
}

#[test]
fn test_unhandled_dir_check_suppressed_with_ignore_flag() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("MISC")).unwrap();
    write_config(
        device.path(),
        &config_with_extra_fields(
            r#""ignore_unhandled_dirs": true"#,
            r#"{"input_path": ["/DCIM"]}"#,
        ),
    );
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_unhandled_dir_suppressed_via_ignored_dirs() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("MISC")).unwrap();
    write_config(
        device.path(),
        &config_with_extra_fields(
            r#""ignored_dirs": ["MISC"]"#,
            r#"{"input_path": ["/DCIM"]}"#,
        ),
    );
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_ignored_dirs_accepts_leading_slash() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("MISC")).unwrap();
    write_config(
        device.path(),
        &config_with_extra_fields(
            r#""ignored_dirs": ["/MISC"]"#,
            r#"{"input_path": ["/DCIM"]}"#,
        ),
    );
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_root_transfer_skips_unhandled_dir_check() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("MISC")).unwrap();
    // No input_path means root transfer — check must not fire.
    write_config(device.path(), &config_with_transfers("{}"));
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_all_dirs_covered_passes_check() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::create_dir(device.path().join("PRIVATE")).unwrap();
    write_config(
        device.path(),
        &config_with_transfers(r#"{"input_path": ["/DCIM", "/PRIVATE"]}"#),
    );
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}

#[test]
fn test_nonexistent_input_path_is_silently_ignored() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join("DCIM")).unwrap();
    write_config(
        temp.path(),
        &config_with_transfers(r#"{"input_path": ["/DCIM", "/MISSING"]}"#),
    );
    let result = load_per_device_config(temp.path(), vec![], vec![]).unwrap();
    assert_eq!(result.len(), 1);
    assert_eq!(result[0].input_path, Some(PathBuf::from("/DCIM")));
}

#[test]
fn test_files_in_device_root_are_not_flagged_as_unhandled() {
    let device = tempfile::tempdir().unwrap();
    std::fs::create_dir(device.path().join("DCIM")).unwrap();
    std::fs::write(device.path().join("AUTRUN.INF"), b"data").unwrap();
    write_config(device.path(), &config_with_transfers(r#"{"input_path": ["/DCIM"]}"#));
    assert!(load_per_device_config(device.path(), vec![], vec![]).is_ok());
}
