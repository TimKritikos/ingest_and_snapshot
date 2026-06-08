use std::collections::HashMap;
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
    Manual { id: String, scheme_number: Option<u32> },
}


struct SourceMediaPending {
    transfers: HashMap<TransferId, PendingCardId>,
    version: u64,
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

    pub fn new_transfer_id(&mut self) -> TransferId {
        let id = self.next_transfer_id;
        self.next_transfer_id += 1;
        id
    }

    pub fn register(&mut self, transfer_id: TransferId, source_media_dir: &Path, card_id: PendingCardId) {
        let entry = self.entries.entry(source_media_dir.to_owned()).or_insert_with(|| SourceMediaPending {
            transfers: HashMap::new(),
            version: 0,
            approval_lock: Arc::new(Mutex::new(())),
        });
        entry.transfers.insert(transfer_id, card_id);
        entry.version += 1;
    }

    pub fn update_id(&mut self, transfer_id: TransferId, source_media_dir: &Path, card_id: PendingCardId) {
        if let Some(entry) = self.entries.get_mut(source_media_dir) {
            entry.transfers.insert(transfer_id, card_id);
            entry.version += 1;
        }
    }

    /// Move a transfer's registration from one source media dir to another.
    /// Pass `None` for old/new if the transfer had no source media dir on that side.
    pub fn move_source_media(
        &mut self,
        transfer_id: TransferId,
        old_dir: Option<&Path>,
        new_dir: Option<&Path>,
        new_card_id: PendingCardId,
    ) {
        if let Some(old) = old_dir {
            if let Some(entry) = self.entries.get_mut(old) {
                entry.transfers.remove(&transfer_id);
                entry.version += 1;
            }
            if self.entries.get(old).map(|e| e.transfers.is_empty()).unwrap_or(false) {
                self.entries.remove(old);
            }
        }
        if let Some(new) = new_dir {
            self.register(transfer_id, new, new_card_id);
        }
    }

    pub fn unregister(&mut self, transfer_id: TransferId, source_media_dir: Option<&Path>) {
        if let Some(dir) = source_media_dir {
            if let Some(entry) = self.entries.get_mut(dir) {
                entry.transfers.remove(&transfer_id);
                entry.version += 1;
            }
            if self.entries.get(dir).map(|e| e.transfers.is_empty()).unwrap_or(false) {
                self.entries.remove(dir);
            }
        }
    }

    pub fn get_version(&self, source_media_dir: &Path) -> u64 {
        self.entries.get(source_media_dir).map(|e| e.version).unwrap_or(0)
    }

    pub fn get_approval_lock(&self, source_media_dir: &Path) -> Option<Arc<Mutex<()>>> {
        self.entries.get(source_media_dir).map(|e| Arc::clone(&e.approval_lock))
    }

    /// Generate the next sequential CARD#### ID for the given source_media_dir.
    /// Considers both existing directories in `source_media_dir/DATA/` and other pending
    /// transfers registered for the same source_media_dir (excluding `exclude_transfer`).
    ///
    /// Returns `Err` on any I/O error. A missing DATA directory is treated as max=0
    /// since it may not exist yet before the first ingest.
    pub fn next_card_id(
        &self,
        source_media_dir: &Path,
        exclude_transfer: TransferId,
    ) -> Result<String, String> {
        let data_dir = source_media_dir.join(DATA_SUBDIRECTORY);
        let fs_max   = filesystem_max_card_number(&data_dir)?;

        let registry_max = self.entries.get(source_media_dir)
            .map(|entry| {
                entry.transfers.iter()
                    .filter(|&(&id, _)| id != exclude_transfer)
                    .filter_map(|(_, card_id)| {
                        match card_id {
                            PendingCardId::Auto(id) => parse_card_number(id),
                            PendingCardId::Manual { scheme_number, .. } => *scheme_number,
                        }
                    })
                    .max()
                    .unwrap_or(0)
            })
            .unwrap_or(0);

        format_card_id(fs_max.max(registry_max) + 1)
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

fn filesystem_max_card_number(data_dir: &Path) -> Result<u32, String> {
    match std::fs::read_dir(data_dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("Failed to read DATA directory {:?}: {}", data_dir, e)),
        Ok(entries) => {
            let mut max = 0u32;
            for entry in entries {
                let entry = entry.map_err(|e| format!("Failed to read directory entry: {}", e))?;
                let name = entry.file_name();
                if let Some(num) = parse_card_number(&name.to_string_lossy()) {
                    max = max.max(num);
                }
            }
            Ok(max)
        }
    }
}

#[cfg(test)]
#[path = "transfer_registry_tests.rs"]
mod tests;
