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

/// Mount options applied to non-Unix filesystems (the FAT family and NTFS), which have no native
/// concept of Unix permissions. Without these, the kernel exposes every entry as 0777, marking
/// every file world-writable and executable. `dmask=022` masks directories down to 0755
/// (rwxr-xr-x) and `fmask=133` masks files down to 0644 (rw-r--r--), so the data we copy out
/// looks like ordinary files instead of executables.
const NON_UNIX_PERMISSION_MASK_OPTIONS: &str = "dmask=022,fmask=133";

/// A filesystem we know how to mount. Carrying this as an enum rather than a bare string gives a
/// single source of truth for the kernel name (used at mount time, surfaced in the UI, and written
/// to the logs) and for whether the filesystem has a native Unix permission model.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FilesystemType {
    Exfat,
    Vfat,
    Ntfs3,
    Ntfs,
    Ext4,
    Btrfs,
    Xfs,
    F2fs,
}

impl FilesystemType {
    /// The name the kernel knows this filesystem by: passed to the `mount` syscall as the
    /// filesystem type, surfaced in the UI, and written to the logs.
    pub fn kernel_name(self) -> &'static str {
        match self {
            FilesystemType::Exfat => "exfat",
            FilesystemType::Vfat  => "vfat",
            FilesystemType::Ntfs3 => "ntfs3",
            FilesystemType::Ntfs  => "ntfs",
            FilesystemType::Ext4  => "ext4",
            FilesystemType::Btrfs => "btrfs",
            FilesystemType::Xfs   => "xfs",
            FilesystemType::F2fs  => "f2fs",
        }
    }

    /// Whether this filesystem has no native Unix permission model (the FAT family and NTFS).
    /// Such filesystems only synthesize Unix modes from DOS attributes and mount masks, so they
    /// both need explicit permission-mask mount options and need the modes of files copied out of
    /// them normalized afterwards. Native Unix filesystems return `false`.
    pub fn lacks_native_unix_permissions(self) -> bool {
        match self {
            FilesystemType::Exfat
            | FilesystemType::Vfat
            | FilesystemType::Ntfs3
            | FilesystemType::Ntfs => true,
            FilesystemType::Ext4
            | FilesystemType::Btrfs
            | FilesystemType::Xfs
            | FilesystemType::F2fs => false,
        }
    }

    /// The mount options needed to make this filesystem present sensible Unix permissions, or
    /// `None` for native Unix filesystems, which already carry real permissions and would reject
    /// such options with EINVAL. See `NON_UNIX_PERMISSION_MASK_OPTIONS`.
    fn permission_mask_options(self) -> Option<&'static str> {
        if self.lacks_native_unix_permissions() {
            Some(NON_UNIX_PERMISSION_MASK_OPTIONS)
        } else {
            None
        }
    }
}

/// Filesystem types tried in order when auto-detecting the filesystem on a block device.
/// Camera cards are almost always exFAT or FAT32, so those come first.
const FILESYSTEM_TYPES_TO_TRY: &[FilesystemType] = &[
    FilesystemType::Exfat,
    FilesystemType::Vfat,
    FilesystemType::Ntfs3,
    FilesystemType::Ntfs,
    FilesystemType::Ext4,
    FilesystemType::Btrfs,
    FilesystemType::Xfs,
    FilesystemType::F2fs,
];

/// Everything needed to spawn transfer threads after a successful mount.
pub struct SpawnDeps {
    pub ui: Arc<Mutex<Box<dyn UiBackend>>>,
    pub registry: Arc<Mutex<crate::transfer_registry::PendingTransferRegistry>>,
    pub all_source_media: Vec<crate::SourceMediaEntry>,
    pub all_storage_devices: Vec<crate::StorageDeviceEntry>,
    pub backup_log_manager: Arc<Mutex<crate::backup_log::BackupLogManager>>,
    pub media_dir: PathBuf,
    /// Shared collection of all active transfer thread handles. Auto-triggered transfers push
    /// their handles here so the quit guard in main.rs can detect them.
    pub transfer_handles: Arc<Mutex<Vec<std::thread::JoinHandle<()>>>>,
}

struct InternalMountEntry {
    id: MountId,
    by_id_name: String,
    user_transfer_ids: Vec<TransferId>,
    real_device_path: PathBuf,
    mountpoint: PathBuf,
    /// True once the kernel mount syscall succeeded. False means mount is in progress or failed.
    /// Only entries with is_mounted=true need umount2 called on cleanup.
    is_mounted: bool,
    /// The filesystem type the mount succeeded with, or `None` while still mounting or after a
    /// failure. Recorded so the read-write remount can replay the matching permission-mask options
    /// (MS_REMOUNT re-parses them, so omitting them would revert files to 0777 right when the mv
    /// copies them out) and so the type can be written to the logs.
    filesystem_type: Option<FilesystemType>,
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
    device_location: crate::transfer_logic::DeviceLocation,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    spawn_deps: Arc<SpawnDeps>,
    storage_devices: Vec<crate::StorageDeviceEntry>,
    source_media_entries: Vec<crate::SourceMediaEntry>,
    system_hostname: String,
) -> Option<MountId> {
    let (real_device_path, by_id_name) = device_location; //TODO: Actually use this type properly
                                                          //here
    let (id, mountpoint) = {
        let mut guard = manager.lock().unwrap();

        // Dedup: if any existing entry tracks the same physical device, return it as-is.
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
            by_id_name: by_id_name.clone(),
            user_transfer_ids: vec![],
            real_device_path: real_device_path.clone(),
            mountpoint: mountpoint.clone(),
            is_mounted: false,
            filesystem_type: None,
        });
        (id, mountpoint)
    };

    let _ = ui.lock().unwrap().mount_update(MountUpdate::MountAdded(MountEntry {
        id,
        by_id_name: by_id_name.clone(),
        real_device_path: real_device_path.clone(),
        mountpoint: mountpoint.clone(),
        status: MountEntryStatus::Mounting,
        fs_type: LoadingField::Loading,
    }));

    thread::spawn(move || {
        mount_thread(id, (real_device_path,by_id_name), mountpoint, manager, ui, spawn_deps, storage_devices, source_media_entries, system_hostname);
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

/// Initiates unmounting of filesystems associated with a given transfer in background threads.
/// Uses reference-counting: only unmounts entries where the transfer was registered AND
/// the reference count drops to zero.
pub fn start_unmount_for_transfer(
    transfer_id: TransferId,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
) {
    let entries: Vec<(MountId, PathBuf, bool)> = {
        let mut guard = manager.lock().unwrap();
        let mut to_unmount = Vec::new();
        for mount in &mut guard.mounts {
            let was_registered = mount.user_transfer_ids.contains(&transfer_id);
            mount.user_transfer_ids.retain(|&id| id != transfer_id);
            if was_registered && mount.user_transfer_ids.is_empty() {
                to_unmount.push((mount.id, mount.mountpoint.clone(), mount.is_mounted));
            }
        }
        to_unmount
    };

    for (mount_id, mountpoint, is_mounted) in entries {
        let manager_clone = Arc::clone(&manager);
        let ui_clone = Arc::clone(&ui);
        thread::spawn(move || {
            if is_mounted {
                match do_unmount(&mountpoint) {
                    Ok(()) => {}
                    Err(reason) => {
                        let _ = ui_clone.lock().unwrap().mount_update(MountUpdate::UnmountFailed { id: mount_id, reason });
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

/// Registers a transfer as a user of the mount for `real_device_path`.
/// Idempotent: calling it multiple times with the same transfer_id has no effect.
pub fn register_mount_user(
    real_device_path: &Path,
    transfer_id: TransferId,
    manager: &Arc<Mutex<MountManager>>,
) {
    let mut guard = manager.lock().unwrap();
    if let Some(entry) = guard.mounts.iter_mut().find(|m| m.real_device_path == real_device_path) {
        if !entry.user_transfer_ids.contains(&transfer_id) {
            entry.user_transfer_ids.push(transfer_id);
        }
    }
}

/// Returns the by-id names of all currently-mounted block devices.
pub fn get_mounted_device_locations(manager: &Arc<Mutex<MountManager>>) -> Vec<String> {
    manager.lock().unwrap().mounts.iter()
        .filter(|m| m.is_mounted)
        .map(|m| m.by_id_name.clone())
        .collect()
}

#[allow(clippy::too_many_arguments)]
fn mount_thread(
    id: MountId,
    device_location: crate::transfer_logic::DeviceLocation,
    mountpoint: PathBuf,
    manager: Arc<Mutex<MountManager>>,
    ui: Arc<Mutex<Box<dyn UiBackend>>>,
    spawn_deps: Arc<SpawnDeps>,
    storage_devices: Vec<crate::StorageDeviceEntry>,
    source_media_entries: Vec<crate::SourceMediaEntry>,
    system_hostname: String,
) {
    let (real_device_path, by_id_name) = device_location;
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
    let mut mounted_filesystem_type: Option<FilesystemType> = None;

    'outer: for &filesystem_type in FILESYSTEM_TYPES_TO_TRY {
        match nix::mount::mount(
            Some(real_device_path.as_path()),
            mountpoint.as_path(),
            Some(filesystem_type.kernel_name()),
            MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID | MsFlags::MS_RDONLY,
            filesystem_type.permission_mask_options(),
        ) {
            Ok(()) => {
                mounted_filesystem_type = Some(filesystem_type);
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

    let mounted_filesystem_type = match mounted_filesystem_type {
        Some(filesystem_type) => filesystem_type,
        None => {
            let _ = std::fs::remove_dir(&mountpoint);
            let _ = ui.lock().unwrap().mount_update(MountUpdate::MountFailed {
                id,
                reason: format!("Could not mount: {}", last_error),
            });
            return;
        }
    };

    // Mark as successfully mounted so cleanup code knows to call umount2, and record the
    // filesystem type so the read-write remount can replay its permission-mask options.
    if let Some(entry) = manager.lock().unwrap().mounts.iter_mut().find(|m| m.id == id) {
        entry.is_mounted = true;
        entry.filesystem_type = Some(mounted_filesystem_type);
    }

    let _ = ui.lock().unwrap().mount_update(MountUpdate::MountCompleted {
        id,
        fs_type: mounted_filesystem_type.kernel_name().to_string(),
    });

    let config_overrides = match crate::per_device_config::load_per_device_config(&mountpoint,storage_devices.clone(),source_media_entries.clone()) {
        Ok(overrides) => overrides,
        Err(reason) => {
            let (response_tx, response_rx) = crossbeam_channel::unbounded::<()>();
            let _ = ui.lock().unwrap().user_query(
                crate::ui_api::UserQuery::FatalError(crate::ui_api::FatalErrorQuery {
                    error: crate::ui_api::FatalErrorKind::PerDeviceConfig(reason),
                    response_tx,
                }),
                false,
            );
            let _ = response_rx.recv();
            vec![]
        }
    };
    let effective: Vec<crate::per_device_config::PerDeviceTransferOverride> = if config_overrides.is_empty() {
        vec![crate::per_device_config::PerDeviceTransferOverride {
            source_media: None,
            storage_device: None,
            input_path: None,
        }]
    } else {
        config_overrides
    };
    for transfer_override in effective {
        // The per-device config stores the source media directory relative to media_dir,
        // which is exactly the id `DetectedTransferInfo::source_media` expects.
        let source_media = transfer_override.source_media.clone();
        let handle = crate::transfer_logic::spawn_transfer(
            Arc::clone(&spawn_deps.ui),
            Arc::clone(&spawn_deps.registry),
            Arc::clone(&manager),
            spawn_deps.all_source_media.clone(),
            spawn_deps.all_storage_devices.clone(),
            crate::transfer_logic::DetectedTransferInfo {
                source_media,
                card_id: None,
                source_device: transfer_override.storage_device,
                device_location: Some((real_device_path.clone(), by_id_name.clone())),
                input_path: transfer_override.input_path,
                filesystem_type: Some(mounted_filesystem_type),
            },
            Arc::clone(&spawn_deps.backup_log_manager),
            spawn_deps.media_dir.clone(),
            system_hostname.clone(),
        );
        spawn_deps.transfer_handles.lock().unwrap().push(handle);
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
    let (mountpoint, filesystem_type) = manager.lock().unwrap()
        .mounts.iter()
        .find(|m| m.real_device_path == real_device_path && m.is_mounted)
        .map(|m| (m.mountpoint.clone(), m.filesystem_type))
        .ok_or_else(|| format!("Device {:?} is not currently mounted", real_device_path))?;

    // MS_REMOUNT re-parses the options string, so the permission masks set at mount time must be
    // passed again — otherwise files would revert to 0777 exactly when the mv copies them out.
    // MS_NOEXEC/MS_NOSUID are likewise re-asserted: we only ever read the source media, never
    // execute it, even while it is writable.
    let permission_mask_options = filesystem_type
        .and_then(FilesystemType::permission_mask_options);

    nix::mount::mount(
        Some(real_device_path),
        mountpoint.as_path(),
        None::<&str>,
        MsFlags::MS_REMOUNT | MsFlags::MS_NOEXEC | MsFlags::MS_NOSUID,
        permission_mask_options,
    ).map_err(|e| format!("Failed to remount {:?} as read-write: {}", real_device_path, e))
}
