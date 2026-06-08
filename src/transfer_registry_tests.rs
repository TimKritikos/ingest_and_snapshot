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
    let temp = tempfile::tempdir().unwrap();
    let missing_dir = temp.path().join("definitely-not-created");
    assert!(filesystem_get_last_card_number(&missing_dir).is_err());
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
fn test_registry_new_transfer_internal_ids_are_unique() {
    const N: usize = 10000;
    let mut registry = PendingTransferRegistry::new();
    let ids: Vec<TransferId> = (0..N).map(|_| registry.new_transfer_internal_id()).collect();
    let unique_ids: std::collections::HashSet<TransferId> = ids.iter().copied().collect();
    assert_eq!(unique_ids.len(), N);
}

#[test]
fn test_registry_subscribe_no_pending_notifications_on_fresh_registry() {
    let mut registry = PendingTransferRegistry::new();
    let rx = registry.subscribe(Path::new("/some/dir"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_registry_new_approval_lock_is_none_for_unregistered_dir() {
    let registry = PendingTransferRegistry::new();
    assert!(registry.get_approval_lock(Path::new("/some/dir")).is_none());
}

#[test]
fn test_register_creates_approval_lock() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir, PendingCardId::Auto("CARD0001".to_string()));
    assert!(registry.get_approval_lock(dir).is_some());
}

#[test]
fn test_register_notifies_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let rx = registry.subscribe(dir);
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir, PendingCardId::Auto("CARD0001".to_string()));
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_register_second_transfer_sends_second_notification() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let rx = registry.subscribe(dir);
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    registry.register(id1, dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.register(id2, dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err());
}

#[test]
fn test_register_multiple_transfers_send_their_notification() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let rx = registry.subscribe(dir);
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    for i in 0..100 {
        registry.register(id1, dir, PendingCardId::Auto(format_card_id(i).unwrap()));
    }
    for i in 0..100 {
        assert!(rx.try_recv().is_ok());
    }
    for i in 0..100 {
        assert!(rx.try_recv().is_err());
    }
}

#[test]
fn test_register_does_not_notify_subscribers_of_other_dirs() {
    let mut registry = PendingTransferRegistry::new();
    let dir_a = Path::new("/media/card_a");
    let dir_b = Path::new("/media/card_b");
    let rx_a = registry.subscribe(dir_a);
    let rx_b = registry.subscribe(dir_b);
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir_a, PendingCardId::Auto("CARD0001".to_string()));
    assert!(rx_a.try_recv().is_ok());
    assert!(rx_a.try_recv().is_err());
    assert!(rx_b.try_recv().is_err());
}

#[test]
fn test_register_two_transfers_share_approval_lock() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    registry.register(id1, dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.register(id2, dir, PendingCardId::Auto("CARD0002".to_string()));
    let lock1 = registry.get_approval_lock(dir).unwrap();
    let lock2 = registry.get_approval_lock(dir).unwrap();
    assert!(std::sync::Arc::ptr_eq(&lock1, &lock2));
}

#[test]
fn test_update_id_notifies_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx = registry.subscribe(dir);
    assert!(registry.update_id(id, dir, PendingCardId::Auto("CARD0002".to_string())).is_ok());
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err()); // no extra notifications
}

#[test]
fn test_update_id_does_not_notify_subscribers_of_other_dirs() {
    let mut registry = PendingTransferRegistry::new();
    let dir_a = Path::new("/media/card_a");
    let dir_b = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir_a, PendingCardId::Auto("CARD0001".to_string()));
    let rx_a = registry.subscribe(dir_a);
    let rx_b = registry.subscribe(dir_b);
    assert!(registry.update_id(id, dir_a, PendingCardId::Auto("CARD0002".to_string())).is_ok());
    assert!(rx_a.try_recv().is_ok());
    assert!(rx_a.try_recv().is_err());
    assert!(rx_b.try_recv().is_err());
}

#[test]
fn test_update_id_on_unregistered_dir_returns_error_and_does_not_notify() {
    let mut registry = PendingTransferRegistry::new();
    let dir_registered = Path::new("/media/card_a");
    let dir_unknown    = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir_registered, PendingCardId::Auto("CARD0001".to_string()));
    let rx = registry.subscribe(dir_registered);
    assert!(registry.update_id(id, dir_unknown, PendingCardId::Auto("CARD0002".to_string())).is_err());
    assert!(rx.try_recv().is_err()); // registered dir was not notified
}
