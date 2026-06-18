use super::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quit_guard_allows_exit_when_no_handles() {
        let mut handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
        assert!(!has_active_transfer_handles(&mut handles));
    }

    #[test]
    fn quit_guard_allows_exit_when_all_handles_finished() {
        let mut handles = vec![thread::spawn(|| {})];
        // Give the thread a moment to finish before checking.
        thread::sleep(time::Duration::from_millis(50));
        assert!(!has_active_transfer_handles(&mut handles));
        assert!(handles.is_empty(), "finished handles should be pruned");
    }

    #[test]
    fn quit_guard_blocks_exit_when_transfer_running() {
        let (gate_tx, gate_rx) = crossbeam_channel::unbounded::<()>();
        let handle = thread::spawn(move || {
            gate_rx.recv().unwrap();
        });
        let mut handles = vec![handle];

        assert!(has_active_transfer_handles(&mut handles));

        // Unblock the thread so it can finish and the test exits cleanly.
        let _ = gate_tx.send(());
        for h in handles { let _ = h.join(); }
    }

    #[test]
    fn resolve_backup_log_ownership_leaves_both_unset_when_no_names_given() {
        let ownership = resolve_backup_log_ownership(&None, &None).unwrap();
        assert!(ownership.owner_uid.is_none(), "uid must stay unset when no user is configured");
        assert!(ownership.owner_gid.is_none(), "gid must stay unset when no group is configured");
    }

    #[test]
    fn resolve_backup_log_ownership_resolves_existing_names_to_their_ids() {
        use nix::unistd::{User, Group, getuid, getgid};

        // Resolve the names of the account the test runs as, so the test relies on no specific
        // system user or group existing.
        let current_user  = User::from_uid(getuid()).unwrap().unwrap();
        let current_group = Group::from_gid(getgid()).unwrap().unwrap();

        let ownership = resolve_backup_log_ownership(
            &Some(current_user.name.clone()),
            &Some(current_group.name.clone()),
        ).unwrap();

        assert_eq!(ownership.owner_uid, Some(getuid()), "configured user should resolve to its uid");
        assert_eq!(ownership.owner_gid, Some(getgid()), "configured group should resolve to its gid");
    }

    #[test]
    fn resolve_backup_log_ownership_errors_on_unknown_user() {
        let result = resolve_backup_log_ownership(
            &Some("definitely_not_a_real_user_9f3a2b".to_owned()),
            &None,
        );
        assert!(result.is_err(), "an unknown user name must be a hard error, not a silent fallback");
    }

    #[test]
    fn resolve_backup_log_ownership_errors_on_unknown_group() {
        let result = resolve_backup_log_ownership(
            &None,
            &Some("definitely_not_a_real_group_9f3a2b".to_owned()),
        );
        assert!(result.is_err(), "an unknown group name must be a hard error, not a silent fallback");
    }
}
