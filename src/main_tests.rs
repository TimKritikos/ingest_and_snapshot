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
}
