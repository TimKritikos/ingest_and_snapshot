use crate::transfer_logic::*;

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};
    use crate::ui_api::{self, UiBackend, UiError, UserQuery};
    use crate::transfer_registry::PendingTransferRegistry;
    use crate::mount_manager::MountManager;
    use crate::backup_log::BackupLogManager;
    use crate::{SourceMediaEntry, CardNamingScheme};

    struct MockUiBackend {
        done_tx: crossbeam_channel::Sender<bool>,
    }

    impl UiBackend for MockUiBackend {
        fn add_config(&mut self, _: Vec<String>, _: Vec<String>) -> Result<(), UiError> { Ok(()) }
        fn set_available_devices(&mut self, _: Vec<SourceMediaEntry>) -> Result<(), UiError> { Ok(()) }
        fn new_transfer(&mut self, _: Option<String>, _: crossbeam_channel::Receiver<ui_api::TransferEvent>) -> Result<(), UiError> { Ok(()) }
        fn mount_update(&mut self, _: ui_api::MountUpdate) -> Result<(), UiError> { Ok(()) }
        fn quit(&mut self) -> Result<(), UiError> { Ok(()) }
        fn join(self: Box<Self>) {}

        fn user_query(&mut self, query: UserQuery, _priority: bool) -> Result<(), UiError> {
            match query {
                UserQuery::ApproveTransfer(q) => {
                    let _ = q.response_tx.send(ui_api::ApproveTransferResponse::Approved);
                }
                UserQuery::CardIdInLogWarning(q) => {
                    let _ = q.response_tx.send(ui_api::CardIdInLogWarningResponse::Cancel);
                    let _ = self.done_tx.send(true);
                }
                UserQuery::FatalError(q) => {
                    let _ = q.response_tx.send(());
                    let _ = self.done_tx.send(false);
                }
                _ => {}
            }
            Ok(())
        }
    }

    #[test]
    fn test_approved_transfer_with_card_id_in_log_shows_warning() {
        let tempdir = tempfile::tempdir().unwrap();
        let media_dir = tempdir.path().to_path_buf();

        // Create source media directory with an empty DATA subdirectory so CARD0000 is
        // auto-generated as the first available card ID.
        let source_media_subdir = media_dir.join("source_media").join("test_cam");
        std::fs::create_dir_all(source_media_subdir.join("DATA")).unwrap();

        // Pre-populate the backup log with a transfer for CARD0000 on this source media,
        // simulating the scenario where the card directory was deleted and the ID reused.
        let backup_log_dir = media_dir.join("metadata").join(crate::backup_log::BACKUP_LOG_DATA_DIR_NAME);
        std::fs::create_dir_all(&backup_log_dir).unwrap();
        let mut log_manager = BackupLogManager::create_new(backup_log_dir, None).unwrap();
        log_manager.add_transfer(
            std::path::PathBuf::from("source_media/test_cam/DATA/CARD0000"),
            false, false, None, false, None, false, None, false,
        ).unwrap();
        let backup_log_manager = Arc::new(Mutex::new(log_manager));

        let (done_tx, done_rx) = crossbeam_channel::unbounded::<bool>();
        let ui: Arc<Mutex<Box<dyn UiBackend>>> =
            Arc::new(Mutex::new(Box::new(MockUiBackend { done_tx })));

        let source_media_entry = SourceMediaEntry {
            device_make_name: "Test".to_owned(),
            device_model_name: "Camera".to_owned(),
            device_model_name_pretty: None,
            serial_number: "SN001".to_owned(),
            new_card_naming_scheme: CardNamingScheme::Card,
            directory: source_media_subdir.clone(),
        };

        spawn_transfer(
            Arc::clone(&ui),
            Arc::new(Mutex::new(PendingTransferRegistry::new())),
            Arc::new(Mutex::new(MountManager::new())),
            vec![source_media_entry.clone()],
            vec![],
            vec![LOCAL_FILESYSTEM_DEVICE_LOCATION.to_owned()],
            DetectedTransferInfo {
                source_media:     Some(source_media_entry),
                card_id:          None,
                source_device:    None,
                device_location:  Some(LOCAL_FILESYSTEM_DEVICE_LOCATION.to_owned()),
                real_device_path: None,
            },
            Arc::clone(&backup_log_manager),
            media_dir,
        );

        let warning_shown = done_rx
            .recv_timeout(std::time::Duration::from_secs(10))
            .expect("Transfer thread did not complete within timeout");
        assert!(warning_shown, "Expected CardIdInLogWarning but the transfer ended with a different query");
    }
}
