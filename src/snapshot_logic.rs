/* snapshot_logic.rs

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

//! Drives the "Finish backup and do snapshot" workflow on its own thread.
//!
//! After the user supplies a snapshot message it creates a ZFS snapshot named
//! `temp_YYYY-MM-DD_<message>`, then runs the `check` executable found at the root of the
//! snapshot's media directory (`<media>/.zfs/snapshot/<snapshot>/check`). The check program runs
//! under a pseudo-terminal so its live, possibly cursor-controlled output (the kind `tput
//! cuu/cuf/cud` and SGR colour codes produce) is streamed verbatim to the UI's check terminal.
//!
//! If the check succeeds — or the user chooses to complete early — the `temp_` prefix is dropped
//! and the optional success callback runs. If it fails the optional failure callback runs and the
//! user is asked whether to keep (drop the suffix) or destroy the snapshot.

use std::fs::File;
use std::io::Read;
use std::os::fd::OwnedFd;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;
use crossbeam_channel::{Receiver, Sender, TryRecvError};
use nix::pty::{openpty, OpenptyResult, Winsize};
use nix::errno::Errno;
use nix::sys::signal::{killpg, Signal};
use nix::unistd::Pid;
use crate::ui_api::{
    UiBackend, SnapshotUpdate, SnapshotActionButton, SnapshotActionStyle,
    SnapshotNameResponse, UserQuery, SnapshotNameQuery, UiToLogicMessage,
};
use crate::backup_log::BackupLogManager;

/// Filename of the check executable expected at the root of the snapshot's media directory.
const CHECK_EXECUTABLE_NAME: &str = "check";
/// Prefix carried by a snapshot until its check passes (or the user keeps it anyway).
const TEMP_SNAPSHOT_PREFIX: &str = "temp_";
/// Path under the dataset mountpoint where ZFS exposes snapshots for browsing.
const ZFS_SNAPSHOT_SUBDIR: &str = ".zfs/snapshot";
/// Fallback message used when the user submits an empty snapshot name.
const DEFAULT_SNAPSHOT_MESSAGE: &str = "Added_new_data"; //TODO: I didn't want this feature, at least not like that, but i'm not don't want to spend neither the time nore the tokens to fix it and this is the case for a lot of stuff here

/// Initial pseudo-terminal grid handed to the check program. The UI's emulator reflows to the real
/// window size, but without a SIGWINCH path the program itself keeps believing in this size.
const PTY_INITIAL_ROWS: u16 = 24;
const PTY_INITIAL_COLS: u16 = 80;

/// How often the run loop polls the check process and the action channel.
const RUN_LOOP_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// How often to re-check whether the killed check process group has fully exited, and an upper
/// bound on how long to wait before giving up (e.g. a descendant stuck in uninterruptible I/O).
const GROUP_EXIT_POLL_INTERVAL: Duration = Duration::from_millis(20);
const GROUP_EXIT_MAX_POLLS: u32 = 250; // ~5s safety cap

// Identifiers for the action buttons published to the snapshot actions window.
const ACTION_EXIT_COMPLETE: u32 = 1;
const ACTION_EXIT_REMOVE: u32 = 2;
const ACTION_KEEP_ANYWAY: u32 = 3;
const ACTION_DESTROY: u32 = 4;
const ACTION_RETURN: u32 = 5;
const ACTION_COMPLETE_BACKUP: u32 = 6;

/// Optional callback executables, read from the main config, run after the check resolves.
pub struct SnapshotConfig {
    pub success_callback: Option<String>,
    pub failure_callback: Option<String>,
}

/// How the check program's run ended.
enum CheckOutcome {
    /// The check exited with a success status.
    Success,
    /// The check exited with a failure status, could not be started, or errored while waited on.
    Failure,
    /// The user asked to stop the check and complete (finalize) the snapshot anyway.
    CompleteEarly,
    /// The user asked to stop the check and remove the snapshot.
    RemoveEarly,
}

pub fn spawn_snapshot(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    media_dir: PathBuf,
    config: SnapshotConfig,
    backup_log_manager: Arc<Mutex<BackupLogManager>>,
    ui_to_logic_tx: Sender<UiToLogicMessage>,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        run_snapshot(ui, media_dir, config, backup_log_manager, ui_to_logic_tx);
    })
}

fn run_snapshot(
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    media_dir: PathBuf,
    config: SnapshotConfig,
    backup_log_manager: Arc<Mutex<BackupLogManager>>,
    ui_to_logic_tx: Sender<UiToLogicMessage>,
) {
    // Ask the user for the free-form snapshot message before taking over the screen.
    let message = match ask_snapshot_name(&ui) {
        Some(message) => sanitize_snapshot_message(&message),
        None => return, // cancelled — no snapshot
    };

    let final_name = format!("{}_{}", snapshot_date_utc(), message);
    let temp_name  = format!("{}{}", TEMP_SNAPSHOT_PREFIX, final_name);

    // Enter check-terminal mode: updates flow logic -> UI, action ids flow UI -> logic.
    let (updates_tx, updates_rx) = crossbeam_channel::unbounded::<SnapshotUpdate>();
    let (action_tx, action_rx)   = crossbeam_channel::unbounded::<u32>();
    if ui.lock().unwrap().start_check_terminal(updates_rx, action_tx).is_err() {
        return;
    }

    write_status(&updates_tx, &format!("Generating snapshot {} ...", temp_name));

    let dataset = match detect_zfs_dataset(&media_dir) {
        Ok(dataset) => dataset,
        Err(error) => {
            write_error(&updates_tx, &format!("Could not determine the ZFS dataset: {}", error));
            offer_return_and_exit(&updates_tx, &action_rx);
            return;
        }
    };

    if let Err(error) = run_zfs(&["snapshot", &snapshot_id(&dataset, &temp_name)]) {
        write_error(&updates_tx, &format!("Failed to create snapshot: {}", error));
        offer_return_and_exit(&updates_tx, &action_rx);
        return;
    }
    write_status(&updates_tx, &format!("Snapshot {} created.", temp_name));

    let snapshot_root = media_dir.join(ZFS_SNAPSHOT_SUBDIR).join(&temp_name);
    let check_executable = snapshot_root.join(CHECK_EXECUTABLE_NAME);
    write_status(&updates_tx, &format!("Executing check program: {}", check_executable.display()));

    //TODO: Don't define the UI here, man!
    set_actions(&updates_tx, vec![
        button(ACTION_EXIT_COMPLETE, "Skip check & complete snapshot", SnapshotActionStyle::Confirm),
        button(ACTION_EXIT_REMOVE,   "Skip check & remove snapshot",   SnapshotActionStyle::Danger),
    ]);

    let outcome = match spawn_check_under_pty(&check_executable, &snapshot_root) {
        Ok((mut child, master)) => {
            // The reader streams the check program's terminal output until the pty closes.
            let _reader = spawn_output_reader(master, updates_tx.clone());
            match run_check_loop(&mut child, &action_rx) {
                Some(outcome) => outcome,
                None => return, // UI disconnected (e.g. application quit)
            }
        }
        Err(error) => {
            write_error(&updates_tx, &format!("Could not start the check program: {}", error));
            CheckOutcome::Failure
        }
    };

    // Resolve the snapshot according to the outcome and the user's follow-up choices.
    match outcome {
        CheckOutcome::Success | CheckOutcome::CompleteEarly => {
            if matches!(outcome, CheckOutcome::CompleteEarly) {
                write_status(&updates_tx, "Check stopped by user; completing the snapshot.");
            } else {
                write_success(&updates_tx, "Check completed successfully.");
            }
            // The backup is done: finalize, then mark the backup complete and exit (the same
            // teardown as "Unmount and exit") instead of returning to the main screen. On a
            // finalize failure, fall through to the return-to-main option so the error is visible.
            if finalize_snapshot(&updates_tx, &dataset, &temp_name, &final_name, &media_dir, config.success_callback.as_deref()) {
                complete_backup_and_exit(&updates_tx, &action_rx, &backup_log_manager, &ui_to_logic_tx);
                return;
            }
        }
        CheckOutcome::Failure => {
            write_error(&updates_tx, "Check did not complete successfully.");
            if let Some(callback) = config.failure_callback.as_deref() {
                run_callback(&updates_tx, callback, &final_name, &temp_name, &media_dir);
            }
            set_actions(&updates_tx, vec![
                button(ACTION_KEEP_ANYWAY, "Keep snapshot anyway", SnapshotActionStyle::Confirm),
                button(ACTION_DESTROY,     "Destroy snapshot",     SnapshotActionStyle::Danger),
            ]);
            write_status(&updates_tx, "Keep the snapshot anyway, or destroy it?");
            match wait_for_action(&action_rx, &[ACTION_KEEP_ANYWAY, ACTION_DESTROY]) {
                Some(ACTION_KEEP_ANYWAY) => {
                    // Keeping drops the temp_ prefix just like the success case, but the success
                    // callback is intentionally not run because the check did not pass.
                    finalize_snapshot(&updates_tx, &dataset, &temp_name, &final_name, &media_dir, None);
                }
                Some(_) => destroy_snapshot(&updates_tx, &dataset, &temp_name),
                None => return, // UI disconnected
            }
        }
        CheckOutcome::RemoveEarly => {
            write_status(&updates_tx, "Removing the snapshot.");
            destroy_snapshot(&updates_tx, &dataset, &temp_name);
        }
    }

    offer_return_and_exit(&updates_tx, &action_rx);
}

/// Issues the snapshot-name query and blocks until the user provides a name or cancels.
fn ask_snapshot_name(ui: &Arc<Mutex<Box<dyn UiBackend>>>) -> Option<String> {
    let (response_tx, response_rx) = crossbeam_channel::unbounded::<SnapshotNameResponse>();
    if ui.lock().unwrap().user_query(UserQuery::SnapshotName(SnapshotNameQuery { response_tx }), false).is_err() {
        return None;
    }
    match response_rx.recv() {
        Ok(SnapshotNameResponse::Provided(name)) => Some(name),
        Ok(SnapshotNameResponse::Cancelled) | Err(_) => None,
    }
}

/// Polls the running check process and the action channel until the check resolves or the user
/// stops it. Returns `None` if the UI side disconnected and the thread should simply exit.
fn run_check_loop(child: &mut Child, action_rx: &Receiver<u32>) -> Option<CheckOutcome> {
    loop {
        match action_rx.try_recv() {
            Ok(ACTION_EXIT_COMPLETE) => {
                terminate_check_group(child);
                return Some(CheckOutcome::CompleteEarly);
            }
            Ok(ACTION_EXIT_REMOVE) => {
                terminate_check_group(child);
                return Some(CheckOutcome::RemoveEarly);
            }
            Ok(_) => {}
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                terminate_check_group(child);
                return None;
            }
        }

        match child.try_wait() {
            Ok(Some(status)) => {
                return Some(if status.success() { CheckOutcome::Success } else { CheckOutcome::Failure });
            }
            Ok(None) => {}
            Err(_) => return Some(CheckOutcome::Failure),
        }

        thread::sleep(RUN_LOOP_POLL_INTERVAL);
    }
}

/// Renames `dataset@temp_name` to `dataset@final_name`, dropping the `temp_` prefix, and runs the
/// optional success callback when the rename succeeds. Returns `true` when the snapshot was
/// successfully renamed.
fn finalize_snapshot(
    updates_tx: &Sender<SnapshotUpdate>,
    dataset: &str,
    temp_name: &str,
    final_name: &str,
    media_dir: &Path,
    success_callback: Option<&str>,
) -> bool {
    if let Err(error) = unmount_snapshot(dataset, temp_name) {
        write_error(updates_tx, &format!("Warning: could not unmount snapshot before rename: {}", error));
    }
    match run_zfs(&["rename", &snapshot_id(dataset, temp_name), &snapshot_id(dataset, final_name)]) {
        Ok(()) => {
            write_success(updates_tx, &format!("Snapshot finalized as {}.", final_name));
            if let Some(callback) = success_callback {
                run_callback(updates_tx, callback, final_name, final_name, media_dir);
            }
            true
        }
        Err(error) => {
            write_error(updates_tx, &format!("Failed to finalize snapshot: {}", error));
            false
        }
    }
}

fn destroy_snapshot(updates_tx: &Sender<SnapshotUpdate>, dataset: &str, temp_name: &str) {
    if let Err(error) = unmount_snapshot(dataset, temp_name) {
        write_error(updates_tx, &format!("Warning: could not unmount snapshot before destroy: {}", error));
    }
    match run_zfs(&["destroy", &snapshot_id(dataset, temp_name)]) {
        Ok(())     => write_status(updates_tx, "Snapshot destroyed."),
        Err(error) => write_error(updates_tx, &format!("Failed to destroy snapshot: {}", error)),
    }
}

/// Unmounts the snapshot's `.zfs` automount. ZFS creates this mount the moment the check program's
/// working directory is set inside `.zfs/snapshot/<snapshot>/`, and a mounted snapshot cannot be
/// renamed or destroyed ("dataset is busy"). The check child has already been reaped, so nothing
/// else holds the mount; `umount` is synchronous, so once it returns the following `zfs` operation
/// is safe — no waiting or retrying is required.
///
/// The exact mount target is read from `/proc/self/mounts` by matching the `dataset@snapshot`
/// source, rather than reconstructed from the media path, so it works regardless of symlinks or how
/// the media directory was spelled. Returns `Ok(())` when the snapshot is no longer mounted
/// (including when it never was); returns an error only if a `umount` of a present mount failed.
fn unmount_snapshot(dataset: &str, snapshot_name: &str) -> Result<(), String> {
    let source = snapshot_id(dataset, snapshot_name);
    let mounts = std::fs::read_to_string("/proc/self/mounts")
        .map_err(|e| format!("could not read /proc/self/mounts: {}", e))?;

    let mut last_error: Option<String> = None;
    for line in mounts.lines() {
        let mut fields = line.split_whitespace();
        let mount_source = fields.next().unwrap_or("");
        let mount_target = fields.next().unwrap_or("");
        if mount_source != source {
            continue;
        }
        // /proc mount fields octal-escape spaces and similar characters in the target path.
        let target = unescape_proc_mount_path(mount_target);
        let output = Command::new("umount").arg(&target).output();
        match output {
            Ok(output) if output.status.success() => {}
            Ok(output) => last_error = Some(format!("umount {}: {}", target, String::from_utf8_lossy(&output.stderr).trim())),
            Err(error)  => last_error = Some(format!("umount {}: {}", target, error)),
        }
    }

    match last_error {
        Some(error) => Err(error),
        None        => Ok(()),
    }
}

/// Decodes the octal escapes (`\040` for space, etc.) that `/proc/self/mounts` uses in path fields.
fn unescape_proc_mount_path(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut decoded = String::with_capacity(raw.len());
    let mut index = 0;
    while index < bytes.len() {
        let is_octal_escape = bytes[index] == b'\\'
            && index + 3 < bytes.len()
            && bytes[index + 1..index + 4].iter().all(|b| b.is_ascii_digit());
        if is_octal_escape {
            if let Ok(code) = u8::from_str_radix(&raw[index + 1..index + 4], 8) {
                decoded.push(code as char);
                index += 4;
                continue;
            }
        }
        decoded.push(bytes[index] as char);
        index += 1;
    }
    decoded
}

/// Runs a callback executable, passing the snapshot's final name, the name it currently has on disk
/// (which differs only while the `temp_` prefix is still present), and the media directory. The
/// callback's own output is echoed into the check terminal.
fn run_callback(
    updates_tx: &Sender<SnapshotUpdate>,
    callback: &str,
    snapshot_name: &str,
    snapshot_now: &str,
    media_dir: &Path,
) {
    write_status(updates_tx, &format!("Running callback: {}", callback));
    let output = Command::new(callback)
        .arg("--snapshot_name").arg(snapshot_name)
        .arg("--snapshot_now").arg(snapshot_now)
        .arg("--media").arg(media_dir)
        .output();
    match output {
        Ok(output) => {
            if !output.stdout.is_empty() {
                let _ = updates_tx.send(SnapshotUpdate::Terminal(normalize_newlines(&output.stdout)));
            }
            if !output.stderr.is_empty() {
                let _ = updates_tx.send(SnapshotUpdate::Terminal(normalize_newlines(&output.stderr)));
            }
            if !output.status.success() {
                write_error(updates_tx, "Callback exited with a failure status.");
            }
        }
        Err(error) => write_error(updates_tx, &format!("Failed to run callback: {}", error)),
    }
}

/// Spawns the check executable attached to a fresh pseudo-terminal so it sees a real TTY. Returns
/// the child handle and the master side of the pty, from which its output is read.
fn spawn_check_under_pty(check_executable: &Path, working_dir: &Path) -> Result<(Child, File), String> {
    let window_size = Winsize {
        ws_row: PTY_INITIAL_ROWS,
        ws_col: PTY_INITIAL_COLS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    let OpenptyResult { master, slave } =
        openpty(Some(&window_size), None).map_err(|e| format!("openpty failed: {}", e))?;

    // The child needs its own stdin/stdout/stderr handles onto the slave side.
    let slave_for_stdin  = clone_fd(&slave)?;
    let slave_for_stdout = clone_fd(&slave)?;
    let slave_for_stderr = slave;

    let child = Command::new(check_executable)
        .current_dir(working_dir)
        .env("TERM", "xterm-256color")
        .stdin(Stdio::from(slave_for_stdin))
        .stdout(Stdio::from(slave_for_stdout))
        .stderr(Stdio::from(slave_for_stderr))
        // Put the check in its own process group (leader = the child) so the whole group — the
        // check plus any descendants it spawns, such as a `sleep` in a check script — can be
        // killed together. Otherwise an orphaned descendant keeps its cwd inside the snapshot and
        // holds the mount busy. `process_group` is the safe stdlib equivalent of a setpgid call.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("{}: {}", check_executable.display(), e))?;

    // The parent keeps only the master; the slave copies were moved into (and closed by) the child.
    Ok((child, File::from(master)))
}

fn clone_fd(fd: &OwnedFd) -> Result<OwnedFd, String> {
    fd.try_clone().map_err(|e| format!("failed to duplicate pty fd: {}", e))
}

/// Streams bytes from the pty master into the UI until the check program closes its end. A closed
/// pty surfaces as EOF (`Ok(0)`) or `EIO`, both of which end the reader.
fn spawn_output_reader(mut master: File, updates_tx: Sender<SnapshotUpdate>) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match master.read(&mut buffer) {
                Ok(0) => break,
                Ok(read_count) => {
                    if updates_tx.send(SnapshotUpdate::Terminal(buffer[..read_count].to_vec())).is_err() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
    })
}

/// Kills the check program and every descendant, then waits until the whole process group has
/// actually exited.
///
/// The check runs as its own process-group leader, so a single `killpg` reaches children it spawned
/// (e.g. a `sleep`, or the processes `shellcheck` forks). `SIGKILL` is delivered synchronously but
/// the processes do not finish dying — and release the cwd/file handles that keep the snapshot's
/// `.zfs` mount busy — until a moment later. Reaping only the leader is not enough, so this polls
/// the group until it is empty (`killpg` returns `ESRCH`) before returning, ensuring the following
/// unmount cannot race a descendant that is still tearing down.
fn terminate_check_group(child: &mut Child) {
    let process_group = Pid::from_raw(child.id() as i32);
    let _ = killpg(process_group, Signal::SIGKILL);
    // Reap the leader so it leaves the group; otherwise its zombie keeps the group non-empty and
    // the emptiness check below would never succeed.
    let _ = child.wait();

    for _ in 0..GROUP_EXIT_MAX_POLLS {
        // Re-signal each round to also catch any descendant spawned right as the group was killed.
        // `ESRCH` means no members remain, i.e. every process has fully exited.
        if matches!(killpg(process_group, Signal::SIGKILL), Err(Errno::ESRCH)) {
            break;
        }
        thread::sleep(GROUP_EXIT_POLL_INTERVAL);
    }
}

/// Shows a single "Return to main screen" button and blocks until the user presses it, then leaves
/// check-terminal mode.
fn offer_return_and_exit(updates_tx: &Sender<SnapshotUpdate>, action_rx: &Receiver<u32>) {
    set_actions(updates_tx, vec![
        button(ACTION_RETURN, "Return to main screen", SnapshotActionStyle::Confirm),
    ]);
    write_status(updates_tx, "Done. Select \"Return to main screen\" to go back.");
    wait_for_action(action_rx, &[ACTION_RETURN]);
    let _ = updates_tx.send(SnapshotUpdate::Exit);
}

/// Offers the single "Complete backup and exit" button shown after a successful check. When the
/// user selects it, the current backup log entry is marked complete and the application is asked to
/// unmount everything and exit (the same teardown as "Unmount and exit"). If marking the backup
/// fails, the snapshot stays put and the user is offered the normal return-to-main option instead.
fn complete_backup_and_exit(
    updates_tx: &Sender<SnapshotUpdate>,
    action_rx: &Receiver<u32>,
    backup_log_manager: &Arc<Mutex<BackupLogManager>>,
    ui_to_logic_tx: &Sender<UiToLogicMessage>,
) {
    set_actions(updates_tx, vec![
        button(ACTION_COMPLETE_BACKUP, "Complete backup and exit", SnapshotActionStyle::Confirm),
    ]);
    write_status(updates_tx, "Select \"Complete backup and exit\" to mark the backup complete and exit.");
    if wait_for_action(action_rx, &[ACTION_COMPLETE_BACKUP]).is_none() {
        return; // UI disconnected
    }

    match backup_log_manager.lock().unwrap().complete_backup() {
        Ok(()) => {
            write_success(updates_tx, "Backup marked complete. Unmounting and exiting ...");
            // The main loop performs the unmount-and-exit teardown; the UI tears down with it, so
            // there is no need to send SnapshotUpdate::Exit here.
            let _ = ui_to_logic_tx.send(UiToLogicMessage::CompleteBackupAndExit);
        }
        Err(error) => {
            write_error(updates_tx, &format!("Failed to mark backup as complete: {}", error));
            offer_return_and_exit(updates_tx, action_rx);
        }
    }
}

/// Blocks until the user picks one of `allowed` action ids. Returns `None` if the UI disconnected.
fn wait_for_action(action_rx: &Receiver<u32>, allowed: &[u32]) -> Option<u32> {
    loop {
        match action_rx.recv() {
            Ok(id) if allowed.contains(&id) => return Some(id),
            Ok(_) => {} // ignore stale ids from a previous button set
            Err(_) => return None,
        }
    }
}

fn button(id: u32, label: &str, style: SnapshotActionStyle) -> SnapshotActionButton {
    SnapshotActionButton { id, label: label.to_owned(), style }
}

fn set_actions(updates_tx: &Sender<SnapshotUpdate>, actions: Vec<SnapshotActionButton>) {
    let _ = updates_tx.send(SnapshotUpdate::SetActions(actions));
}

/// Writes a neutral status line into the terminal (cyan, on its own line).
fn write_status(updates_tx: &Sender<SnapshotUpdate>, message: &str) {
    write_colored_line(updates_tx, message, "36");
}

/// Writes a success line into the terminal (green).
fn write_success(updates_tx: &Sender<SnapshotUpdate>, message: &str) {
    write_colored_line(updates_tx, message, "32");
}

/// Writes an error line into the terminal (red).
fn write_error(updates_tx: &Sender<SnapshotUpdate>, message: &str) {
    write_colored_line(updates_tx, message, "31");
}

fn write_colored_line(updates_tx: &Sender<SnapshotUpdate>, message: &str, sgr_color: &str) {
    // CRLF before and after so our own messages always start at column 0, regardless of where the
    // check program left the cursor.
    let line = format!("\r\n\x1b[{}m{}\x1b[0m\r\n", sgr_color, message);
    let _ = updates_tx.send(SnapshotUpdate::Terminal(line.into_bytes()));
}

/// Translates bare LF to CRLF so captured (non-pty) callback output starts each line at column 0.
fn normalize_newlines(bytes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes.len());
    let mut previous = 0u8;
    for &byte in bytes {
        if byte == b'\n' && previous != b'\r' {
            out.push(b'\r');
        }
        out.push(byte);
        previous = byte;
    }
    out
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

/// Keeps only ZFS-name-safe characters, replacing anything else with `_`. Falls back to a default
/// when the result is empty so the snapshot always has a usable name component.
fn sanitize_snapshot_message(raw: &str) -> String {
    let sanitized: String = raw
        .trim()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | ':' | '-') { c } else { '_' })
        .collect();
    if sanitized.is_empty() {
        DEFAULT_SNAPSHOT_MESSAGE.to_owned()
    } else {
        sanitized
    }
}

/// Today's date in UTC, formatted `YYYY-MM-DD`.
fn snapshot_date_utc() -> String {
    let now = time_format::now().unwrap_or(0);
    time_format::strftime_utc("%Y-%m-%d", now).unwrap_or_else(|_| "0000-00-00".to_owned())
}

#[cfg(test)]
#[path = "snapshot_logic_tests.rs"]
mod tests;
