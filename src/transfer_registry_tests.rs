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
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(None));
}

#[test]
fn test_filesystem_max_card_number_returns_max() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("CARD0001")).unwrap();
    std::fs::create_dir(dir.path().join("CARD0005")).unwrap();
    std::fs::create_dir(dir.path().join("CARD0003")).unwrap();
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(Some(5)));
}

#[test]
fn test_filesystem_max_card_number_ignores_non_card_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("CARD0002")).unwrap();
    std::fs::create_dir(dir.path().join("notacard")).unwrap();
    std::fs::create_dir(dir.path().join("CARD")).unwrap();
    assert_eq!(filesystem_get_last_card_number(dir.path()), Ok(Some(2)));
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
    for i in 0..100 {
        registry.register(id1, dir, PendingCardId::Auto(format_card_id(i).unwrap()));
    }
    for _ in 0..100 {
        assert!(rx.try_recv().is_ok());
    }
    for _ in 0..100 {
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

#[test]
fn test_move_source_media_notifies_old_dir_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx_old = registry.subscribe(old_dir);
    registry.move_source_media(id, old_dir, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(rx_old.try_recv().is_ok());
    assert!(rx_old.try_recv().is_err()); // no extra notifications
}

#[test]
fn test_move_source_media_notifies_new_dir_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx_new = registry.subscribe(new_dir);
    registry.move_source_media(id, old_dir, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(rx_new.try_recv().is_ok());
    assert!(rx_new.try_recv().is_err()); // no extra notifications
}

#[test]
fn test_move_source_media_removes_old_entry_when_empty() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.move_source_media(id, old_dir, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(registry.get_approval_lock(old_dir).is_none());
}

#[test]
fn test_move_source_media_creates_approval_lock_for_new_dir() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.move_source_media(id, old_dir, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(registry.get_approval_lock(new_dir).is_some());
}

#[test]
fn test_move_source_media_to_existing_dir_keeps_approval_lock_the_same() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    registry.register(id1, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.register(id2, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    let lock_before = registry.get_approval_lock(new_dir).unwrap();
    registry.move_source_media(id1, old_dir, new_dir, PendingCardId::Auto("CARD0003".to_string()));
    let lock_after = registry.get_approval_lock(new_dir).unwrap();
    assert!(std::sync::Arc::ptr_eq(&lock_before, &lock_after));
}

#[test]
fn test_move_source_media_keeps_old_entry_when_other_transfers_remain() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir = Path::new("/media/card_a");
    let new_dir = Path::new("/media/card_b");
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    registry.register(id1, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.register(id2, old_dir, PendingCardId::Auto("CARD0002".to_string()));
    registry.move_source_media(id1, old_dir, new_dir, PendingCardId::Auto("CARD0003".to_string()));
    assert!(registry.get_approval_lock(old_dir).is_some());
}

#[test]
fn test_move_source_media_does_not_notify_unrelated_dir_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let old_dir     = Path::new("/media/card_a");
    let new_dir     = Path::new("/media/card_b");
    let unrelated   = Path::new("/media/card_c");
    let id = registry.new_transfer_internal_id();
    registry.register(id, old_dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx_unrelated = registry.subscribe(unrelated);
    registry.move_source_media(id, old_dir, new_dir, PendingCardId::Auto("CARD0002".to_string()));
    assert!(rx_unrelated.try_recv().is_err());
}

#[test]
fn test_unregister_notifies_subscribers() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx = registry.subscribe(dir);
    registry.unregister(id, dir).unwrap();
    assert!(rx.try_recv().is_ok());
    assert!(rx.try_recv().is_err()); // no extra notifications
}

#[test]
fn test_unregister_removes_entry_when_last_transfer() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.unregister(id, dir).unwrap();
    assert!(registry.get_approval_lock(dir).is_none());
}

#[test]
fn test_unregister_keeps_entry_when_other_transfers_remain() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id1 = registry.new_transfer_internal_id();
    let id2 = registry.new_transfer_internal_id();
    registry.register(id1, dir, PendingCardId::Auto("CARD0001".to_string()));
    registry.register(id2, dir, PendingCardId::Auto("CARD0002".to_string()));
    registry.unregister(id1, dir).unwrap();
    assert!(registry.get_approval_lock(dir).is_some());
}

#[test]
fn test_unregister_does_not_notify_other_dirs() {
    let mut registry = PendingTransferRegistry::new();
    let dir_a = Path::new("/media/card_a");
    let dir_b = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir_a, PendingCardId::Auto("CARD0001".to_string()));
    let rx_b = registry.subscribe(dir_b);
    registry.unregister(id, dir_a).unwrap();
    assert!(rx_b.try_recv().is_err());
}

#[test]
fn test_unregister_unknown_transfer_id_returns_error_and_does_not_notify() {
    let mut registry = PendingTransferRegistry::new();
    let dir = Path::new("/media/card");
    let id_registered = registry.new_transfer_internal_id();
    let id_unknown    = registry.new_transfer_internal_id();
    registry.register(id_registered, dir, PendingCardId::Auto("CARD0001".to_string()));
    let rx = registry.subscribe(dir);
    assert!(registry.unregister(id_unknown, dir).is_err());
    assert!(rx.try_recv().is_err()); // dir was not notified
}

#[test]
fn test_unregister_on_unregistered_dir_returns_error_and_does_not_notify() {
    let mut registry = PendingTransferRegistry::new();
    let dir_registered = Path::new("/media/card_a");
    let dir_unknown    = Path::new("/media/card_b");
    let id = registry.new_transfer_internal_id();
    registry.register(id, dir_registered, PendingCardId::Auto("CARD0001".to_string()));
    let rx = registry.subscribe(dir_registered);
    assert!(registry.unregister(id, dir_unknown).is_err());
    assert!(rx.try_recv().is_err()); // registered dir was not notified
}

#[test]
fn test_next_card_id_missing_data_dir_returns_error() {
    let temp = tempfile::tempdir().unwrap();
    // DATA subdirectory intentionally not created
    let registry = PendingTransferRegistry::new();
    assert!(registry.next_card_id(temp.path(), 0).is_err());
}

#[test]
fn test_next_card_id_empty_data_dir_returns_card0000() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(DATA_SUBDIRECTORY)).unwrap();
    let registry = PendingTransferRegistry::new();
    assert_eq!(registry.next_card_id(temp.path(), 0).unwrap(), "CARD0000");
}

#[test]
fn test_next_card_id_with_existing_filesystem_cards() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::create_dir(data_dir.join("CARD0003")).unwrap();
    let registry = PendingTransferRegistry::new();
    assert_eq!(registry.next_card_id(temp.path(), 0).unwrap(), "CARD0004");
}

#[test]
fn test_next_card_id_with_pending_auto_transfer() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(DATA_SUBDIRECTORY)).unwrap();
    let mut registry = PendingTransferRegistry::new();
    let other_id    = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(other_id, temp.path(), PendingCardId::Auto("CARD0003".to_string()));
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0004");
}

#[test]
fn test_next_card_id_excludes_the_querying_transfer() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(DATA_SUBDIRECTORY)).unwrap();
    let mut registry = PendingTransferRegistry::new();
    let transfer_id = registry.new_transfer_internal_id();
    registry.register(transfer_id, temp.path(), PendingCardId::Auto("CARD0005".to_string()));
    // The transfer queries for its own next ID — its own CARD0005 must not count
    // this is useful when the transfer needs to find a new id so it asks for it's next id and the
    // one in currently holds shouldn't be counted.
    assert_eq!(registry.next_card_id(temp.path(), transfer_id).unwrap(), "CARD0000");
}

#[test]
fn test_next_card_id_takes_max_of_filesystem_and_registry() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::create_dir(data_dir.join("CARD0003")).unwrap();
    let mut registry = PendingTransferRegistry::new();
    let other_id    = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(other_id, temp.path(), PendingCardId::Auto("CARD0005".to_string()));
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0006");
}

#[test]
fn test_next_card_id_filesystem_beats_registry() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    std::fs::create_dir(data_dir.join("CARD0007")).unwrap();
    let mut registry = PendingTransferRegistry::new();
    let other_id    = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(other_id, temp.path(), PendingCardId::Auto("CARD0003".to_string()));
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0008");
}

#[test]
fn test_next_card_id_skips_manual_reservation_at_base() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    for n in 0..=3u32 {
        std::fs::create_dir(data_dir.join(format_card_id(n).unwrap())).unwrap();
    }
    let mut registry = PendingTransferRegistry::new();
    let manual_id   = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(manual_id, temp.path(), PendingCardId::Manual {
        scheme_number: Some(4),
    });
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0005");
}

#[test]
fn test_next_card_id_skips_consecutive_manual_reservations() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    for n in 0..=3u32 {
        std::fs::create_dir(data_dir.join(format_card_id(n).unwrap())).unwrap();
    }
    let mut registry = PendingTransferRegistry::new();
    let manual_id_a = registry.new_transfer_internal_id();
    let manual_id_b = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(manual_id_a, temp.path(), PendingCardId::Manual {
        scheme_number: Some(4),
    });
    registry.register(manual_id_b, temp.path(), PendingCardId::Manual {
        scheme_number: Some(5),
    });
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0006");
}

#[test]
fn test_next_card_id_manual_below_base_does_not_affect_result() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    for n in 0..=3u32 {
        std::fs::create_dir(data_dir.join(format_card_id(n).unwrap())).unwrap();
    }
    let mut registry = PendingTransferRegistry::new();
    let manual_id   = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(manual_id, temp.path(), PendingCardId::Manual {
        scheme_number: Some(2),
    });
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0004");
}

#[test]
fn test_next_card_does_not_count_manual_transfers() {
    let temp = tempfile::tempdir().unwrap();
    let data_dir = temp.path().join(DATA_SUBDIRECTORY);
    std::fs::create_dir(&data_dir).unwrap();
    for n in 0..=3u32 {
        std::fs::create_dir(data_dir.join(format_card_id(n).unwrap())).unwrap();
    }
    let mut registry = PendingTransferRegistry::new();
    let manual_id   = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(manual_id, temp.path(), PendingCardId::Manual {
        scheme_number: Some(6),
    });
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0004");
}

#[test]
fn test_next_card_id_ignores_manual_without_scheme_number() {
    let temp = tempfile::tempdir().unwrap();
    std::fs::create_dir(temp.path().join(DATA_SUBDIRECTORY)).unwrap();
    let mut registry = PendingTransferRegistry::new();
    let other_id    = registry.new_transfer_internal_id();
    let querying_id = registry.new_transfer_internal_id();
    registry.register(other_id, temp.path(), PendingCardId::Manual {
        scheme_number: None,
    });
    assert_eq!(registry.next_card_id(temp.path(), querying_id).unwrap(), "CARD0000");
}
