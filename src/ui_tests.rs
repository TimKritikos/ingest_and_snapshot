use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    fn make_transfer(status: TransferStatus) -> Transfer {
        let (_, rx) = crossbeam_channel::unbounded();
        Transfer {
            source_media_dir: None,
            bytes_total: 0,
            samples: Vec::new(),
            status,
            rx_control: rx,
        }
    }

    #[test]
    fn no_active_transfers_when_list_is_empty() {
        assert!(!has_active_ui_transfers(&[]));
    }

    #[test]
    fn no_active_transfers_when_all_finished_or_failed() {
        let transfers = vec![
            make_transfer(TransferStatus::Finished),
            make_transfer(TransferStatus::Failed),
        ];
        assert!(!has_active_ui_transfers(&transfers));
    }

    #[test]
    fn active_transfer_detected_for_in_progress() {
        let transfers = vec![
            make_transfer(TransferStatus::Finished),
            make_transfer(TransferStatus::InProgress),
        ];
        assert!(has_active_ui_transfers(&transfers));
    }

    #[test]
    fn active_transfer_detected_for_not_started() {
        let transfers = vec![make_transfer(TransferStatus::NotStarted)];
        assert!(has_active_ui_transfers(&transfers));
    }
}
