use super::*;
use std::path::Path;

#[test]
fn test_format_card_id() {
    assert_eq!(format_card_id(0).unwrap(),    "CARD0000");
    assert_eq!(format_card_id(5).unwrap(),    "CARD0005");
    assert_eq!(format_card_id(10).unwrap(),   "CARD0010");
    assert_eq!(format_card_id(1234).unwrap(), "CARD1234");
    assert_eq!(format_card_id(9999).unwrap(), "CARD9999");
    assert!(format_card_id(10000).is_err());
    assert!(format_card_id(10001).is_err());
    assert!(format_card_id(u32::MAX).is_err());
}

#[test]
fn test_parse_card_number() {
    assert_eq!(parse_card_number("CARD0123").unwrap(), 123);
    assert_eq!(parse_card_number("CARD0000").unwrap(), 0);
    assert_eq!(parse_card_number("notacard"), None);
    assert_eq!(parse_card_number("CARD"), None);
    assert_eq!(parse_card_number(""), None);
    assert_eq!(parse_card_number("card1234"), None);
    assert_eq!(parse_card_number("CARDOOO1"), None);
    assert_eq!(parse_card_number("CARD0000a"), None);
    assert_eq!(parse_card_number("CARD0"), None);
    assert_eq!(parse_card_number("CARD12"), None);
    assert_eq!(parse_card_number("CARD12345"), None);
}

#[test]
fn test_filesystem_max_card_number_not_found() {
    assert!(filesystem_get_last_card_number(Path::new("/n/o/n/e/x/i/s/t/e/n/t/p/a/t/h")).is_err());
}

#[test]
fn test_filesystem_max_card_number_empty_dir() {
    let dir = tempfile::tempdir().unwrap();
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(0));
}

#[test]
fn test_filesystem_max_card_number_returns_max() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("CARD0001")).unwrap();
    std::fs::create_dir(dir.path().join("CARD0005")).unwrap();
    std::fs::create_dir(dir.path().join("CARD0003")).unwrap();
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(5));
}

#[test]
fn test_filesystem_max_card_number_ignores_non_card_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("CARD0002")).unwrap();
    std::fs::create_dir(dir.path().join("notacard")).unwrap();
    std::fs::create_dir(dir.path().join("CARD")).unwrap();
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(2));
}

#[test]
fn test_registry_new_transfer_ids_are_unique() {
    const N: usize = 10000;
    let mut registry = PendingTransferRegistry::new();
    let ids: Vec<TransferId> = (0..N).map(|_| registry.new_transfer_internal_id()).collect();
    let unique_ids: std::collections::HashSet<TransferId> = ids.iter().copied().collect();
    assert_eq!(unique_ids.len(), N);
}

#[test]
fn test_registry_new_version_is_zero_for_unregistered_dir() {
    let registry = PendingTransferRegistry::new();
    assert_eq!(registry.get_version(Path::new("/some/dir")), 0);
}

#[test]
fn test_registry_new_approval_lock_is_none_for_unregistered_dir() {
    let registry = PendingTransferRegistry::new();
    assert!(registry.get_approval_lock(Path::new("/some/dir")).is_none());
}
