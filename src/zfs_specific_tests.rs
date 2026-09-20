#[cfg(test)]
mod tests {
    use super::super::*;

    #[test]
    fn snapshot_id_joins_dataset_and_name() {
        assert_eq!(snapshot_id("tank/photos", "2026-06-17_trip"), "tank/photos@2026-06-17_trip");
    }

    #[test]
    fn stage_name_prepends_prefix() {
        assert_eq!(stage_name(SnapshotStage::Tripwire, "2026-06-17_trip"), "pre_2026-06-17_trip");
        assert_eq!(stage_name(SnapshotStage::Check, "2026-06-17_trip"), "check_2026-06-17_trip");
    }

    #[test]
    fn find_stray_stage_snapshots_ignores_clean_dataset() {
        let output = "tank/photos@2026-06-17_trip\ntank/photos@2026-06-01_older\n";
        assert!(find_stray_stage_snapshots("tank/photos", output).is_empty());
    }

    #[test]
    fn find_stray_stage_snapshots_finds_every_stage() {
        let output = "tank/photos@2026-06-01_older\ntank/photos@pre_2026-06-17_trip\ntank/photos@check_2026-06-17_trip\n";
        let stray = find_stray_stage_snapshots("tank/photos", output);
        assert_eq!(stray, vec![
            "tank/photos@pre_2026-06-17_trip".to_owned(),
            "tank/photos@check_2026-06-17_trip".to_owned(),
        ]);
    }

    #[test]
    fn find_stray_stage_snapshots_ignores_other_datasets() {
        // A sibling dataset's snapshot happening to start with a stage prefix must not match —
        // only lines under exactly "tank/photos@" are ours to worry about.
        let output = "tank/photos-archive@check_2026-06-17_trip\n";
        assert!(find_stray_stage_snapshots("tank/photos", output).is_empty());
    }
}
