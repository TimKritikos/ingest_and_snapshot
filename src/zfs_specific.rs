/* zfs_specific.rs

   This file is part of the ingest_and_snapshot project

   Copyright (c) 2026 Efthymios Kritikos

   This program is free software: you can redistribute it and/or modify
   it under the terms of the GNU General Public License as published by
   the Free Software Foundation, either version 3 of the License, or
   (at your option) any later version.

   This program is distributed in the hope that it will be useful,
   but WITHOUT ANY WARRANTY; without even the implied warranty of
   MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
   GNU General Public License for more details.

   You should have received a copy of the GNU General Public License
   along with this program.  If not, see <http://www.gnu.org/licenses/>.  */

//! The `zfs` mechanics behind "Finish backup and do snapshot".
//!
//! A ZFS snapshot freezes its data at the moment it is created — nothing written to the live
//! filesystem afterwards (including a rename, which only touches the name) is ever visible inside
//! an already-taken snapshot. So for the kept snapshot to show `completed_backup: true` in the
//! backup log, the log has to be written complete *before* that snapshot is taken.
//!
//! That is what [`do_snapshot_steps_before_check`] does, and it is why the `check` program runs
//! against a snapshot that already claims a completed backup: the snapshot it validates is, byte for
//! byte, the one [`do_snapshot_steps_after_check`] renames to its final name. Nothing is
//! re-snapshotted afterwards, so a write landing on the live media directory while the check runs
//! simply is not part of this backup — it cannot change what was already frozen and checked.
//!
//! The sequence, tracked through [`SnapshotStage`]-tagged names:
//!
//! 1. [`SnapshotStage::Tripwire`] is taken first, and its contents are never read.
//! 2. The backup log is marked complete on the live filesystem.
//! 3. [`SnapshotStage::Check`] is taken; its frozen copy of the log says complete.
//! 4. The tripwire is destroyed — the check snapshot is stage-tagged itself, so from here on it is
//!    the one keeping the guarantee below, and the tripwire has nothing left to cover.
//! 5. `snapshot_logic` runs the `check` program against the check snapshot.
//! 6. If the check resolves in the backup's favor — it passed, the user skipped it, or it failed and
//!    the user kept the snapshot anyway (an acknowledged issue that will not be fixed on this
//!    snapshot) — [`do_snapshot_steps_after_check`] renames it to its final, plain name. Otherwise
//!    [`abandon_snapshot_run`] puts the log back and destroys it.
//!
//! The guarantee all of this exists for: from the moment the log is marked complete until the rename
//! in step 6, a [`SnapshotStage`]-tagged snapshot is on disk at every instant. Step 1 covers the
//! window between steps 2 and 3, where a crash would otherwise leave a log claiming a backup that no
//! snapshot carries and nothing on disk to signal it; the check snapshot covers everything after.
//!
//! The rename is what ends that: `zfs rename` is atomic, so it is the only step that can bring a
//! plain, final-named snapshot into existence, and by then nothing stage-tagged is left beside it. A
//! plain-named snapshot existing therefore always means the whole sequence ran to completion.
//!
//! [`check_correct_shutdown`] leans on exactly that at program startup: any [`SnapshotStage`]-tagged
//! snapshot found there can only be left over from a run that was killed, crashed, or lost power,
//! since every path above ends with none. There is deliberately no automatic recovery — the dataset
//! is left untouched and a human is asked to look at it before the program starts again.

use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Mutex};
use crossbeam_channel::Sender;
use crate::ui_api::SnapshotUpdate;
use crate::backup_log::BackupLogManager;
use crate::snapshot_logic::{write_status, write_success, write_error, run_callback};

/// The names a snapshot carries before it reaches its final, plain name. See the module docs for the
/// sequence each is used in.
#[derive(Clone, Copy)]
pub enum SnapshotStage {
    /// Taken before the backup log is marked complete, purely so a crash in the window between that
    /// write and the check snapshot's creation still leaves something stage-tagged on disk for
    /// [`check_correct_shutdown`] to find. Its contents are never read.
    Tripwire,
    /// The snapshot the `check` program runs against, and — unchanged, byte for byte — the one
    /// renamed to its final name once the check resolves in the backup's favor.
    Check,
}

/// Every [`SnapshotStage`] variant, for scanning existing snapshot names in [`check_correct_shutdown`].
const ALL_STAGES: [SnapshotStage; 2] = [SnapshotStage::Tripwire, SnapshotStage::Check];

impl SnapshotStage {
    fn prefix(self) -> &'static str {
        match self {
            SnapshotStage::Tripwire => "pre_",
            SnapshotStage::Check => "check_",
        }
    }
}

fn stage_name(stage: SnapshotStage, final_name: &str) -> String {
    format!("{}{}", stage.prefix(), final_name)
}

/// What a finish-backup run has on disk once [`do_snapshot_steps_before_check`] returns: the
/// [`SnapshotStage::Check`] snapshot, which the caller locates the `check` executable inside and
/// then passes to [`do_snapshot_steps_after_check`] or [`abandon_snapshot_run`].
pub struct SnapshotRun {
    pub dataset: String,
    pub check_name: String,
}

/// Takes the tripwire snapshot, marks the backup log complete, takes the check snapshot the `check`
/// program will run against, and drops the tripwire again — in that order, for the reasons in the
/// module docs. Cleans up after itself if any step fails, so a returned error always means nothing
/// was left behind.
pub fn do_snapshot_steps_before_check(
    updates_tx: &Sender<SnapshotUpdate>,
    media_dir: &Path,
    final_name: &str,
    backup_log_manager: &Arc<Mutex<BackupLogManager>>,
) -> Result<SnapshotRun, String> {
    let dataset = detect_zfs_dataset(media_dir)
        .map_err(|e| format!("could not determine the ZFS dataset: {}", e))?;
    let tripwire_name = stage_name(SnapshotStage::Tripwire, final_name);
    let check_name    = stage_name(SnapshotStage::Check, final_name);

    run_zfs(&["snapshot", &snapshot_id(&dataset, &tripwire_name)])
        .map_err(|e| format!("failed to create the tripwire snapshot: {}", e))?;

    let completion = backup_log_manager.lock().unwrap().complete_backup();
    if let Err(error) = completion {
        unwind(updates_tx, &dataset, &[&tripwire_name], backup_log_manager);
        return Err(format!("failed to mark the backup log complete: {}", error));
    }
    write_status(updates_tx, "Backup log marked complete.");

    write_status(updates_tx, &format!("Generating snapshot {} ...", check_name));
    if let Err(error) = run_zfs(&["snapshot", &snapshot_id(&dataset, &check_name)]) {
        unwind(updates_tx, &dataset, &[&tripwire_name], backup_log_manager);
        return Err(format!("failed to create snapshot: {}", error));
    }
    write_status(updates_tx, &format!("Snapshot {} created.", check_name));

    if let Err(error) = run_zfs(&["destroy", &snapshot_id(&dataset, &tripwire_name)]) {
        unwind(updates_tx, &dataset, &[&check_name, &tripwire_name], backup_log_manager);
        return Err(format!("failed to destroy the tripwire snapshot: {}", error));
    }

    Ok(SnapshotRun { dataset, check_name })
}

/// Finalizes the run by renaming the already-checked snapshot to its final, plain name. Like
/// [`do_snapshot_steps_before_check`] it cleans up after itself if that fails.
pub fn do_snapshot_steps_after_check(
    updates_tx: &Sender<SnapshotUpdate>,
    run: &SnapshotRun,
    final_name: &str,
    media_dir: &Path,
    backup_log_manager: &Arc<Mutex<BackupLogManager>>,
    success_callback: Option<&str>,
) -> Result<(), String> {
    let rename = run_zfs(&["rename", &snapshot_id(&run.dataset, &run.check_name), &snapshot_id(&run.dataset, final_name)]);
    if let Err(error) = rename {
        unwind(updates_tx, &run.dataset, &[&run.check_name], backup_log_manager);
        return Err(format!("failed to finalize snapshot: {}", error));
    }
    write_success(updates_tx, &format!("Snapshot finalized as {}.", final_name));

    if let Some(callback) = success_callback {
        run_callback(updates_tx, callback, final_name, final_name, media_dir);
    }
    Ok(())
}

/// Gives up on a run that will not produce a final snapshot — the check failed and the user chose to
/// discard it, the user asked to stop early and remove it, or finalizing failed partway through.
pub fn abandon_snapshot_run(
    updates_tx: &Sender<SnapshotUpdate>,
    run: &SnapshotRun,
    backup_log_manager: &Arc<Mutex<BackupLogManager>>,
) {
    unwind(updates_tx, &run.dataset, &[&run.check_name], backup_log_manager);
}

/// Puts the backup log's completion flag back, then destroys the given stage-tagged snapshots.
///
/// The order is the point. A stage-tagged snapshot is what makes [`check_correct_shutdown`] halt, so
/// it is the only thing standing between a log that wrongly claims a completed backup and a silent
/// start with no snapshot to show for it. If the log cannot be put back, the snapshots are therefore
/// left alone on purpose. Destroying them is best-effort: whatever survives is caught at the next
/// startup.
fn unwind(
    updates_tx: &Sender<SnapshotUpdate>,
    dataset: &str,
    snapshot_names: &[&str],
    backup_log_manager: &Arc<Mutex<BackupLogManager>>,
) {
    let reverted = backup_log_manager.lock().unwrap().mark_backup_incomplete();
    if let Err(error) = reverted {
        write_error(updates_tx, &format!(
            "Failed to revert the backup log to incomplete: {}. Leaving {} in place so the next \
             startup refuses to continue — the log and the snapshot(s) need sorting out by hand.",
            error, snapshot_names.join(", "),
        ));
        return;
    }
    write_status(updates_tx, "Backup log reverted to incomplete.");

    for name in snapshot_names {
        match run_zfs(&["destroy", &snapshot_id(dataset, name)]) {
            Ok(())     => write_status(updates_tx, &format!("Snapshot {} destroyed.", name)),
            Err(error) => write_error(updates_tx, &format!("Failed to destroy snapshot {}: {}", name, error)),
        }
    }
}

/// Startup consistency check: verifies no [`SnapshotStage`]-tagged snapshot is left on the dataset
/// backing `media_dir`. See the module docs for why finding one here always means a previous run was
/// interrupted, and why that is a hard failure rather than something to auto-recover from.
pub fn check_correct_shutdown(media_dir: &Path) -> Result<(), String> {
    let dataset = detect_zfs_dataset(media_dir)
        .map_err(|e| format!("could not determine the ZFS dataset: {}", e))?;

    let output = Command::new("zfs")
        .args(["list", "-H", "-o", "name", "-t", "snapshot", "-r", &dataset])
        .output()
        .map_err(|e| format!("failed to run `zfs list`: {}", e))?;
    if !output.status.success() {
        return Err(format!("`zfs list`: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }

    let stray = find_stray_stage_snapshots(&dataset, &String::from_utf8_lossy(&output.stdout));
    if stray.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "the dataset {} has snapshot(s) left over from an interrupted \"Finish backup and do \
             snapshot\" run, meaning the program was killed, crashed, or the system lost power partway through \
             finishing a backup: {}. The backup log may also still claim that backup completed. \
             Both need sorting out by hand before starting this program again.",
            dataset, stray.join(", "),
        ))
    }
}

/// Pure parser behind [`check_correct_shutdown`], kept separate from the `zfs list` invocation so it
/// can be unit-tested with fabricated output instead of a real pool.
fn find_stray_stage_snapshots(dataset: &str, zfs_list_output: &str) -> Vec<String> {
    let owned_prefix = format!("{}@", dataset);
    zfs_list_output
        .lines()
        .filter(|line| line.starts_with(&owned_prefix))
        .filter(|line| {
            let name = &line[owned_prefix.len()..];
            ALL_STAGES.iter().any(|stage| name.starts_with(stage.prefix()))
        })
        .map(str::to_owned)
        .collect()
}

/// `dataset@snapshot` identifier used by every `zfs` subcommand.
fn snapshot_id(dataset: &str, snapshot_name: &str) -> String {
    format!("{}@{}", dataset, snapshot_name)
}

/// Determines the ZFS dataset backing `media_dir` via `findmnt`.
fn detect_zfs_dataset(media_dir: &Path) -> Result<String, String> {
    let output = Command::new("findmnt")
        .args(["-n", "-o", "SOURCE", "--target"])
        .arg(media_dir)
        .output()
        .map_err(|e| format!("failed to run findmnt: {}", e))?;

    if !output.status.success() {
        return Err(format!("findmnt failed: {}", String::from_utf8_lossy(&output.stderr).trim()));
    }

    let dataset = String::from_utf8_lossy(&output.stdout).trim().to_owned();
    if dataset.is_empty() {
        return Err(format!("no mount source found for {}", media_dir.display()));
    }
    Ok(dataset)
}

fn run_zfs(args: &[&str]) -> Result<(), String> {
    let output = Command::new("zfs")
        .args(args)
        .output()
        .map_err(|e| format!("failed to run `zfs {}`: {}", args.join(" "), e))?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!("`zfs {}`: {}", args.join(" "), String::from_utf8_lossy(&output.stderr).trim()))
    }
}

#[cfg(test)]
#[path = "zfs_specific_tests.rs"]
mod tests;
