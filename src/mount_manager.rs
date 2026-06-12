/* mount_manager.rs

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

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use nix::mount::{MntFlags, MsFlags};
use crate::ui_api::{LoadingField, MountEntry, MountEntryStatus, MountId, MountUpdate, UiBackend};
use crate::transfer_registry::TransferId;

const MOUNT_BASE_DIR: &str = "/run/ingest_and_snapshot/mounts";

/// Filesystem types tried in order when auto-detecting the filesystem on a block device.
/// Camera cards are almost always exFAT or FAT32, so those come first.
const FILESYSTEM_TYPES_TO_TRY: &[&str] = &[
    "exfat", "vfat", "ntfs3", "ntfs", "ext4", "btrfs", "xfs", "f2fs",
];

struct InternalMountEntry {
    id: MountId,
    transfer_id: TransferId,
    real_device_path: PathBuf,
    mountpoint: PathBuf,
    /// True once the kernel mount syscall succeeded. False means mount is in progress or failed.
    /// Only entries with is_mounted=true need umount2 called on cleanup.
    is_mounted: bool,
}

pub struct MountManager {
    mounts: Vec<InternalMountEntry>,
    next_id_counter: u64,
}

impl MountManager {
    pub fn new() -> Self {
        Self {
            mounts: Vec::new(),
            next_id_counter: 0,
        }
    }

    fn generate_id(&mut self) -> MountId {
        let counter = self.next_id_counter;
        self.next_id_counter += 1;
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64;
        let mixed = t
            .wrapping_mul(0x9E3779B97F4A7C15)
            .wrapping_add(counter.wrapping_mul(0x6C62272E07BB0142));
        ((mixed ^ (mixed >> 32)) & 0xFFFF_FFFF) as u32
    }

    /// Unmounts all actively-mounted filesystems synchronously. Used on program exit.
    /// Failed-but-present entries are just dropped (no kernel state to clean up).
    pub fn unmount_all_sync(&mut self) {
        let mounts: Vec<_> = self.mounts.drain(..).collect();
        for entry in mounts {
            if entry.is_mounted {
                if let Err(e) = do_unmount(&entry.mountpoint) {
                    eprintln!("Warning: failed to unmount {:?} during exit: {}", entry.mountpoint, e);
                }
            }
        }
    }
}

/// Initiates a mount of `real_device_path` (e.g. `/dev/sdb1`) in a background thread.
/// `by_id_name` is the `/dev/disk/by-id/` name used for display and allow/ignore checks.
/// Returns `Some(MountId)` — either an existing ID if this physical device is already tracked
/// (including failed entries), or a freshly allocated ID for the new mount.
/// Returns `None` only when the device node is already gone at entry-creation time.
///
/// On failure the entry stays in the manager and the UI shows a Failed badge — the entry is
/// only removed when the device disconnects (see `remove_mounts_for_device`), or when the user
/// manually requests removal.
pub fn start_mount(
    real_device_path: PathBuf,
    by_id_name: String,
    transfer_id: TransferId,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    on_mount_success: Option<Box<dyn FnOnce(PathBuf) + Send + 'static>>,
) -> Option<MountId> {
    let (id, mountpoint) = {
        let mut guard = manager.lock().unwrap();

        // Dedup: if any existing entry tracks the same physical device, return it as-is.
        // The on_mount_success callback is dropped here — the mount is already in progress or done.
        if let Some(existing) = guard.mounts.iter().find(|m| m.real_device_path == real_device_path) {
            return Some(existing.id);
        }

        // Race: if the device is already gone before we create an entry, skip silently.
        if !real_device_path.exists() {
            return None;
        }

        let id = guard.generate_id();
        let mountpoint = PathBuf::from(MOUNT_BASE_DIR).join(format!("{:08x}", id));
        guard.mounts.push(InternalMountEntry {
            id,
            transfer_id,
            real_device_path: real_device_path.clone(),
            mountpoint: mountpoint.clone(),
            is_mounted: false,
        });
        (id, mountpoint)
    };

    let _ = ui.lock().unwrap().mount_update(MountUpdate::MountAdded(MountEntry {
        id,
        by_id_name,
        real_device_path: real_device_path.clone(),
        mountpoint: mountpoint.clone(),
        status: MountEntryStatus::Mounting,
        fs_type: LoadingField::Loading,
    }));

    thread::spawn(move || {
        mount_thread(id, real_device_path, mountpoint, manager, ui, on_mount_success);
    });

    Some(id)
}

/// Initiates an unmount of a single filesystem by ID in a background thread.
/// If the entry never successfully mounted (Failed status), it is just removed without
/// calling umount2. If unmount fails, the UI is updated with an UnmountFailed status.
pub fn start_unmount(
    mount_id: MountId,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
) {
    let entry_info = {
        let guard = manager.lock().unwrap();
        guard.mounts.iter()
            .find(|m| m.id == mount_id)
            .map(|m| (m.mountpoint.clone(), m.is_mounted))
    };

    if let Some((mountpoint, is_mounted)) = entry_info {
        if is_mounted {
            thread::spawn(move || {
                match do_unmount(&mountpoint) {
                    Ok(()) => {
                        manager.lock().unwrap().mounts.retain(|m| m.id != mount_id);
                        let _ = ui.lock().unwrap().mount_update(MountUpdate::MountRemoved { id: mount_id });
                    }
                    Err(reason) => {
                        let _ = ui.lock().unwrap().mount_update(MountUpdate::UnmountFailed { id: mount_id, reason });
                    }
                }
            });
        } else {
            // Failed mount — no kernel state to clean up.
            manager.lock().unwrap().mounts.retain(|m| m.id != mount_id);
            let _ = ui.lock().unwrap().mount_update(MountUpdate::MountRemoved { id: mount_id });
        }
    }
}

/// Initiates unmounting of all filesystems associated with a given transfer in background threads.
/// Called when a transfer completes.
pub fn start_unmount_for_transfer(
    transfer_id: TransferId,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
) {
    let entries: Vec<(MountId, PathBuf, bool)> = {
        let guard = manager.lock().unwrap();
        guard.mounts.iter()
            .filter(|m| m.transfer_id == transfer_id)
            .map(|m| (m.id, m.mountpoint.clone(), m.is_mounted))
            .collect()
    };

    for (mount_id, mountpoint, is_mounted) in entries {
        let manager_clone = Arc::clone(&manager);
        let ui_clone = Arc::clone(&ui);
        thread::spawn(move || {
            if is_mounted {
                match do_unmount(&mountpoint) {
                    Ok(()) => {}
                    Err(reason) => {
                        let _ = ui_clone.lock().unwrap().mount_update(MountUpdate::UnmountFailed {
                            id: mount_id,
                            reason,
                        });
                        return;
                    }
                }
            }
            manager_clone.lock().unwrap().mounts.retain(|m| m.id != mount_id);
            let _ = ui_clone.lock().unwrap().mount_update(MountUpdate::MountRemoved { id: mount_id });
        });
    }
}

/// Called when a block device is unplugged. Removes all mount entries (mounted or failed)
/// that were associated with `real_device_path` from both the manager and the UI.
/// For actively-mounted entries a best-effort unmount is attempted in a background thread.
pub fn remove_mounts_for_device(
    real_device_path: &Path,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
) {
    let entries: Vec<(MountId, PathBuf, bool)> = {
        let mut guard = manager.lock().unwrap();
        let matching: Vec<_> = guard.mounts.iter()
            .filter(|m| m.real_device_path == real_device_path)
            .map(|m| (m.id, m.mountpoint.clone(), m.is_mounted))
            .collect();
        guard.mounts.retain(|m| m.real_device_path != real_device_path);
        matching
    };

    for (mount_id, mountpoint, is_mounted) in entries {
        let ui_clone = Arc::clone(&ui);
        if is_mounted {
            thread::spawn(move || {
                // Best-effort — device may already be gone, so ignore the result.
                let _ = do_unmount(&mountpoint);
                let _ = ui_clone.lock().unwrap().mount_update(MountUpdate::MountRemoved { id: mount_id });
            });
        } else {
            // Failed mount — no kernel state to clean up, notify UI directly.
            let _ = ui.lock().unwrap().mount_update(MountUpdate::MountRemoved { id: mount_id });
        }
    }
}

fn mount_thread(
    id: MountId,
    real_device_path: PathBuf,
    mountpoint: PathBuf,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    on_mount_success: Option<Box<dyn FnOnce(PathBuf) + Send + 'static>>,
) {
    // Device absent before we even started (race between udev event and thread start)
    // — remove the entry entirely, nothing to show.
    if !real_device_path.exists() {
        manager.lock().unwrap().mounts.retain(|m| m.id != id);
        let _ = ui.lock().unwrap().mount_update(MountUpdate::MountRemoved { id });
        return;
    }

    if let Err(e) = std::fs::create_dir_all(&mountpoint) {
        let _ = ui.lock().unwrap().mount_update(MountUpdate::MountFailed {
            id,
            reason: format!("Could not create mountpoint: {}", e),
        });
        return;
    }

    let mut last_error = String::from("no filesystem types matched");
    let mut mount_succeeded = false;

    'outer: for fstype in FILESYSTEM_TYPES_TO_TRY {
        match nix::mount::mount(
            Some(real_device_path.as_path()),
            mountpoint.as_path(),
            Some(*fstype),
            MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_RDONLY,
            None::<&str>,
        ) {
            Ok(()) => {
                mount_succeeded = true;
                break;
            }
            Err(nix::errno::Errno::EACCES) | Err(nix::errno::Errno::EPERM) => {
                last_error = "permission denied".to_string();
                break 'outer;
            }
            Err(nix::errno::Errno::ENODEV) | Err(nix::errno::Errno::ENOTBLK) => {
                last_error = "not a block device".to_string();
                break 'outer;
            }
            Err(_) => {
                // EINVAL typically means wrong filesystem type — try the next one.
            }
        }
    }

    if !mount_succeeded {
        let _ = std::fs::remove_dir(&mountpoint);
        let _ = ui.lock().unwrap().mount_update(MountUpdate::MountFailed {
            id,
            reason: format!("Could not mount: {}", last_error),
        });
        return;
    }

    // Mark as successfully mounted so cleanup code knows to call umount2.
    if let Some(entry) = manager.lock().unwrap().mounts.iter_mut().find(|m| m.id == id) {
        entry.is_mounted = true;
    }

    let fs_type = detect_fs_type_from_mountinfo(&mountpoint)
        .unwrap_or_else(|| "unknown".to_string());

    let _ = ui.lock().unwrap().mount_update(MountUpdate::MountCompleted { id, fs_type });

    if let Some(callback) = on_mount_success {
        callback(mountpoint.clone());
    }

    // TODO: Read source_media_data.json and other metadata from the mounted filesystem
    // and send FieldUpdate messages to the UI for display in the mount list.
}

fn do_unmount(mountpoint: &Path) -> Result<(), String> {
    let result = match nix::mount::umount2(mountpoint, MntFlags::empty()) {
        Ok(()) => Ok(()),
        Err(nix::errno::Errno::EBUSY) => {
            nix::mount::umount2(mountpoint, MntFlags::MNT_DETACH)
                .map_err(|e| format!("Lazy unmount of {:?} failed: {}", mountpoint, e))
        }
        Err(e) => Err(format!("Failed to unmount {:?}: {}", mountpoint, e)),
    };
    if result.is_ok() {
        let _ = std::fs::remove_dir(mountpoint);
    }
    result
}

/// Returns the mountpoint of the device at `real_device_path` if it is currently mounted.
/// Returns `None` if the device is not tracked, still mounting, or failed to mount.
pub fn get_mountpoint_for_real_device(
    real_device_path: &Path,
    manager: &Arc<Mutex<MountManager>>,
) -> Option<PathBuf> {
    manager.lock().unwrap().mounts.iter()
        .find(|m| m.real_device_path == real_device_path && m.is_mounted)
        .map(|m| m.mountpoint.clone())
}

/// Remounts an already-mounted block device as read-write.
/// Finds the mountpoint by `real_device_path` and issues an `MS_REMOUNT` without `MS_RDONLY`.
/// Returns an error if the device is not currently tracked as mounted, or if the kernel rejects
/// the remount (e.g. filesystem does not support read-write, or hardware write-protection is set).
pub fn remount_readwrite(
    real_device_path: &Path,
    manager: &Arc<Mutex<MountManager>>,
) -> Result<(), String> {
    let mountpoint = manager.lock().unwrap()
        .mounts.iter()
        .find(|m| m.real_device_path == real_device_path && m.is_mounted)
        .map(|m| m.mountpoint.clone())
        .ok_or_else(|| format!("Device {:?} is not currently mounted", real_device_path))?;

    nix::mount::mount(
        Some(real_device_path),
        mountpoint.as_path(),
        None::<&str>,
        MsFlags::MS_REMOUNT,
        None::<&str>,
    ).map_err(|e| format!("Failed to remount {:?} as read-write: {}", real_device_path, e))
}

fn detect_fs_type_from_mountinfo(mountpoint: &Path) -> Option<String> {
    let mountpoint_str = mountpoint.to_str()?;
    let content = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    for line in content.lines() {
        // Mountinfo format: mount_id parent_id major:minor root mountpoint options [optional] - fstype source super_options
        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.get(4) == Some(&mountpoint_str) {
            let dash_pos = fields.iter().position(|&f| f == "-")?;
            return fields.get(dash_pos + 1).map(|s| s.to_string());
        }
    }
    None
}
