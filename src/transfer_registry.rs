use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

const CARD_PREFIX: &str = "CARD";
const CARD_NUMBER_WIDTH: usize = 4;
const DATA_SUBDIRECTORY: &str = "DATA";

pub type TransferId = u64;

pub enum PendingCardId {
    Auto(String),
    /// A manually set card ID. If it follows the CARD#### format, `scheme_number` holds
    /// the parsed number so it can influence auto-generation for other transfers.
    Manual { scheme_number: Option<u32> },
}


struct SourceMediaPending {
    transfers: HashMap<TransferId, PendingCardId>,
    subscribers: Vec<crossbeam_channel::Sender<()>>,
    approval_lock: Arc<Mutex<()>>,
}

pub struct PendingTransferRegistry {
    entries: HashMap<PathBuf, SourceMediaPending>,
    next_transfer_id: u64,
}

impl PendingTransferRegistry {
    pub fn new() -> Self {
        Self {
            entries: HashMap::new(),
            next_transfer_id: 0,
        }
    }

    pub fn new_transfer_internal_id(&mut self) -> TransferId {
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;
        id
    }

    pub fn register(&mut self, transfer_id: TransferId, source_media_dir: &Path, card_id: PendingCardId) {
        let entry = self.entries.entry(source_media_dir.to_owned()).or_insert_with(|| SourceMediaPending {
            transfers: HashMap::new(),
            subscribers: Vec::new(),
            approval_lock: Arc::new(Mutex::new(())),
        });
        entry.transfers.insert(transfer_id, card_id);
        self.notify_subscribers(source_media_dir);
    }

    pub fn update_id(&mut self, transfer_id: TransferId, source_media_dir: &Path, card_id: PendingCardId) -> Result<(), String> {
        match self.entries.get_mut(source_media_dir) {
            Some(entry) => {
                entry.transfers.insert(transfer_id, card_id);
                self.notify_subscribers(source_media_dir);
                Ok(())
            }
            None => Err(format!("update_id called for unregistered source media dir {:?}", source_media_dir)),
        }
    }

    /// Move a transfer's registration from one source media dir to another.
    /// Both dirs must be known. Use `register`/`unregister` when only one side exists.
    pub fn move_source_media(
        &mut self,
        transfer_id: TransferId,
        old_dir: &Path,
        new_dir: &Path,
        new_card_id: PendingCardId,
    ) {
        if let Some(entry) = self.entries.get_mut(old_dir) {
            entry.transfers.remove(&transfer_id);
        }
        self.notify_subscribers(old_dir);
        if self.entries.get(old_dir).map(|e| e.transfers.is_empty()).unwrap_or(false) {
            self.entries.remove(old_dir);
        }
        self.register(transfer_id, new_dir, new_card_id);
    }

    pub fn unregister(&mut self, transfer_id: TransferId, source_media_dir: &Path) -> Result<(), String> {
        match self.entries.get_mut(source_media_dir) {
            Some(entry) => {
                if entry.transfers.remove(&transfer_id).is_none() {
                    return Err(format!("unregister: transfer id {} not found in dir {:?}", transfer_id, source_media_dir));
                }
                self.notify_subscribers(source_media_dir);
                if self.entries.get(source_media_dir).map(|e| e.transfers.is_empty()).unwrap_or(false) {
                    self.entries.remove(source_media_dir);
                }
                Ok(())
            }
            None => Err(format!("unregister: source media dir {:?} not registered", source_media_dir)),
        }
    }

    /// Subscribe to registry change notifications for a source media dir.
    /// The returned receiver fires whenever another transfer registers, updates,
    /// or unregisters for the same dir.
    pub fn subscribe(&mut self, source_media_dir: &Path) -> crossbeam_channel::Receiver<()> {
        let (tx, rx) = crossbeam_channel::unbounded();
        let entry = self.entries.entry(source_media_dir.to_owned()).or_insert_with(|| SourceMediaPending {
            transfers: HashMap::new(),
            subscribers: Vec::new(),
            approval_lock: Arc::new(Mutex::new(())),
        });
        entry.subscribers.push(tx);
        rx
    }

    fn notify_subscribers(&mut self, source_media_dir: &Path) {
        if let Some(entry) = self.entries.get_mut(source_media_dir) {
            entry.subscribers.retain(|tx| tx.send(()).is_ok());
        }
    }

    pub fn get_approval_lock(&self, source_media_dir: &Path) -> Option<Arc<Mutex<()>>> {
        self.entries.get(source_media_dir).map(|e| Arc::clone(&e.approval_lock))
    }

    /// Generate the next sequential CARD#### ID for the given source_media_dir.
    /// Considers both existing directories in `source_media_dir/DATA/` and other pending
    /// transfers registered for the same source_media_dir (excluding `exclude_transfer`).
    ///
    /// Returns `Err` on any I/O error or if the DATA directory does not exist.
    pub fn next_card_id(
        &self,
        source_media_dir: &Path,
        exclude_transfer: TransferId,
    ) -> Result<String, String> {
        let data_dir = source_media_dir.join(DATA_SUBDIRECTORY);
        let fs_max: Option<u32> = filesystem_get_last_card_number(&data_dir)?;

        let entry = self.entries.get(source_media_dir);

        // Auto transfers (and the filesystem) define the sequential ceiling.
        let auto_registry_max: Option<u32> = entry.and_then(|e| {
            e.transfers.iter()
                .filter(|&(&id, _)| id != exclude_transfer)
                .filter_map(|(_, card_id)| match card_id {
                    PendingCardId::Auto(id) => parse_card_number(id),
                    PendingCardId::Manual { .. } => None,
                })
                .max()
        });

        // Manual transfers that conform to the scheme are obstacles: the auto system must
        // skip over them rather than using them to raise its ceiling.
        let manual_reserved: HashSet<u32> = entry.map(|e| {
            e.transfers.iter()
                .filter(|&(&id, _)| id != exclude_transfer)
                .filter_map(|(_, card_id)| match card_id {
                    PendingCardId::Manual { scheme_number: Some(n), .. } => Some(*n),
                    _ => None,
                })
                .collect()
        }).unwrap_or_default();

        let base = [fs_max, auto_registry_max].into_iter().flatten().max()
            .map(|m| m + 1)
            .unwrap_or(0);

        // Find the first slot >= base not already reserved by a manual transfer.
        let mut next = base;
        while manual_reserved.contains(&next) {
            next += 1;
        }
        format_card_id(next)
    }

    /// Returns true if a directory named `card_id` already exists inside
    /// `source_media_dir/DATA/`. Returns `Err` on any error other than NotFound.
    pub fn is_card_id_taken(source_media_dir: &Path, card_id: &str) -> Result<bool, String> {
        let path = source_media_dir.join(DATA_SUBDIRECTORY).join(card_id);
        match std::fs::metadata(&path) {
            Ok(_) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("Failed to check card ID path {:?}: {}", path, e)),
        }
    }

}

pub fn format_card_id(number: u32) -> Result<String, String> {
    if number <= 9999 {
        Ok(format!("{}{:0>width$}", CARD_PREFIX, number, width = CARD_NUMBER_WIDTH))
    } else {
        Err(format!("Numerical card id is outside of the allowable range ( {} is bigger than 9999)", number))
    }
}

/// Parse a CARD#### number from an ID string. Returns None if the string doesn't
/// start with the CARD prefix or the suffix isn't a pure decimal integer.
pub fn parse_card_number(id: &str) -> Option<u32> {
    if id.len() == CARD_PREFIX.len()+4 {
        id.strip_prefix(CARD_PREFIX)?.parse().ok()
    } else {
        None
    }
}

/// Get the highest CARD#### number present in the provided data directory.
/// Returns `Ok(None)` when the directory exists but contains no CARD#### entries.
fn filesystem_get_last_card_number(data_dir: &Path) -> Result<Option<u32>, String> {
    match std::fs::read_dir(data_dir) {
        Err(e) => Err(format!("Failed to read DATA directory {:?}: {}", data_dir, e)),
        Ok(entries) => {
            let mut max: Option<u32> = None;
            for entry in entries {
                let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
                let name = entry.file_name();
                if let Some(num) = parse_card_number(&name.to_string_lossy()) {
                    max = Some(max.map_or(num, |m| m.max(num)));
                }
            }
            Ok(max)
        }
    }
}

#[cfg(test)]
#[path = "transfer_registry_tests.rs"]
mod tests;
