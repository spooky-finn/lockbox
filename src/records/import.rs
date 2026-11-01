//! Importing another vault file into the unlocked vault.
//!
//! Decryption (Argon2id) runs in a session worker; the merge is saved on the UI
//! thread through the regular `save_changes` path, so shared profiles journal
//! and sign it like any other edit. The source file is only read.

use crate::{
    desktop::workspace::Unlocked,
    platform::jobs::Worker,
    records::{entry::Entry, folder::Folder, import_plan as import},
    sync::protocol as sync,
    vault::{Records, Vault},
};
use secrecy::SecretString;
use std::{
    io::{self, Write},
    path::PathBuf,
};
use zeroize::Zeroizing;

/// Stay well inside one shared transaction (mutation count and encoded size).
const BATCH_MUTATIONS: usize = sync::MAX_MUTATIONS / 2;
const BATCH_BYTES: usize = sync::MAX_TRANSACTION_BYTES / 2;

#[derive(Default)]
pub(crate) struct VaultImport {
    pub path: Option<PathBuf>,
    pub password: Zeroizing<String>,
    pub notice: Option<String>,
    job: Option<Worker<Result<Records, String>>>,
}

impl VaultImport {
    pub fn busy(&self) -> bool {
        self.job.is_some()
    }

    pub fn needs_poll(&self) -> bool {
        self.job.as_ref().is_some_and(|job| job.is_finished())
    }

    pub fn file_name(&self) -> Option<String> {
        self.path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
    }
}

/// Counts shown after a completed import.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Summary {
    pub entries: usize,
    pub folders: usize,
    pub skipped: usize,
}

impl Summary {
    fn notice(&self) -> String {
        if self.entries == 0 && self.folders == 0 {
            format!("Nothing new to import; {} duplicates skipped", self.skipped)
        } else {
            format!(
                "Imported {} entries and {} folders; {} duplicates skipped",
                self.entries, self.folders, self.skipped
            )
        }
    }
}

impl Unlocked {
    pub(crate) fn choose_import(&mut self, path: PathBuf) {
        if self.import.busy() {
            return;
        }

        self.import.path = Some(path);
        self.import.notice = None;
        self.error = None;
    }

    /// Starts decrypting the chosen file. The password field is cleared immediately.
    pub(crate) fn start_import(&mut self) {
        if self.import.busy() {
            return;
        }

        self.import.notice = None;
        self.error = None;
        let Some(path) = self.import.path.clone() else {
            return;
        };

        if self.import.password.is_empty() {
            self.error = Some("Enter the password of the vault to import".into());
            return;
        }

        if same_file(&path, self.vault.path()) {
            self.error = Some("This vault is already open".into());
            return;
        }

        let password = SecretString::from(std::mem::take(&mut *self.import.password));
        self.import.job = Some(Worker::spawn(move |_| {
            Vault::read_records(&path, &password).map_err(|error| error.to_string())
        }));
    }

    /// Applies a finished job once no entry or folder draft is open.
    pub(crate) fn poll_import(&mut self) {
        if !self.import.needs_poll() || self.is_editing() {
            return;
        }

        let Some(mut job) = self.import.job.take() else {
            return;
        };
        let result = match job.take() {
            Some(Ok(result)) => result.map_err(|error| format!("Could not import: {error}")),
            _ => Err("Could not read the vault to import".to_owned()),
        };

        match result.and_then(|records| self.import_records(&records)) {
            Ok(summary) => {
                self.import.notice = Some(summary.notice());
                self.import.path = None;
            }
            Err(error) => self.error = Some(error),
        }
    }

    /// Appends the source records in batches. A failed batch is rolled back;
    /// earlier batches stay saved and a repeated import skips them as duplicates.
    pub(crate) fn import_records(&mut self, records: &Records) -> Result<Summary, String> {
        if self.is_editing() {
            return Err("Finish editing before importing".into());
        }

        let plan = import::plan(
            self.vault.folders(),
            self.vault.entries(),
            &records.folders,
            &records.entries,
        );
        let total = plan.entries.len();
        let mut saved = 0;

        for batch in plan.folders.chunks(BATCH_MUTATIONS) {
            self.save_batch(batch, &[])
                .map_err(|error| partial(saved, total, &error))?;
        }

        for batch in entry_batches(&plan.entries) {
            self.save_batch(&[], batch)
                .map_err(|error| partial(saved, total, &error))?;
            saved += batch.len();
        }

        if total > 0 || !plan.folders.is_empty() {
            self.sync.saved();
            self.search.clear();
        }

        Ok(Summary {
            entries: total,
            folders: plan.folders.len(),
            skipped: plan.skipped,
        })
    }

    fn save_batch(&mut self, folders: &[Folder], entries: &[Entry]) -> Result<(), String> {
        self.vault
            .append_records(folders, entries)
            .map_err(|error| error.to_string())
    }
}

fn partial(saved: usize, total: usize, error: &str) -> String {
    format!("Imported {saved} of {total} entries; repeat the import to continue: {error}")
}

/// Measure serializer output without retaining a plaintext copy of the entry.
#[derive(Default)]
struct ByteCount {
    bytes: usize,
}

impl Write for ByteCount {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.bytes = self
            .bytes
            .checked_add(bytes.len())
            .ok_or_else(|| io::Error::other("serialized entry size overflow"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Splits entries so that each shared transaction stays within its limits.
fn entry_batches(entries: &[Entry]) -> Vec<&[Entry]> {
    let mut batches = Vec::new();
    let mut start = 0;
    let mut bytes = 0;

    for (index, entry) in entries.iter().enumerate() {
        let mut counter = ByteCount::default();
        let size =
            serde_json::to_writer(&mut counter, entry).map_or(BATCH_BYTES, |_| counter.bytes);

        if index > start && (index - start == BATCH_MUTATIONS || bytes + size > BATCH_BYTES) {
            batches.push(&entries[start..index]);
            start = index;
            bytes = 0;
        }
        bytes += size;
    }

    if start < entries.len() {
        batches.push(&entries[start..]);
    }

    batches
}

fn same_file(a: &std::path::Path, b: &std::path::Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

#[cfg(test)]
#[path = "tests/import.rs"]
mod tests;
