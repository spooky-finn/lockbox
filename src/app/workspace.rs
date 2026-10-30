//! Entry collection, selection and transactional save/delete.
use super::{biometrics::TouchId, editor::Draft};
use crate::{entry::Entry, generator, totp, vault::Vault};

pub(super) struct Unlocked {
    pub(super) vault: Vault,
    pub(super) touch_id: TouchId,
    pub(super) search: String,
    pub(super) selected: Option<String>,
    pub(super) draft: Option<Draft>,
    pub(super) show_password: bool,
    pub(super) confirm_delete: bool,
    pub(super) error: Option<String>,
    /// Generator settings last until the window closes; they are never saved.
    pub(super) generator: generator::Options,
}

impl Unlocked {
    pub(super) fn new(vault: Vault) -> Self {
        Self {
            vault,
            touch_id: TouchId::default(),
            search: String::new(),
            selected: None,
            draft: None,
            show_password: false,
            confirm_delete: false,
            error: None,
            generator: generator::Options::default(),
        }
    }

    /// Entries matching the search, sorted by title.
    pub(super) fn visible(&self) -> Vec<&Entry> {
        let mut entries: Vec<&Entry> = self
            .vault
            .entries
            .iter()
            .filter(|entry| matches(entry, &self.search))
            .collect();
        entries.sort_by_key(|entry| entry.title.to_lowercase());
        entries
    }

    pub(super) fn selected_entry(&self) -> Option<&Entry> {
        let id = self.selected.as_ref()?;
        self.vault.entries.iter().find(|entry| &entry.id == id)
    }

    pub(super) fn select(&mut self, id: String) {
        self.selected = Some(id);
        self.show_password = false;
        self.confirm_delete = false;
        self.error = None;
    }

    pub(super) fn start_new(&mut self) {
        self.start_draft(Entry::new(), true);
    }

    pub(super) fn start_edit(&mut self) {
        if let Some(entry) = self.selected_entry().cloned() {
            self.start_draft(entry, false);
        }
    }

    pub(super) fn start_draft(&mut self, entry: Entry, is_new: bool) {
        self.draft = Some(Draft {
            entry,
            is_new,
            show_password: false,
            show_totp: false,
            totp_error: None,
            generator_open: false,
            generator_error: None,
            icon_picker_open: false,
            icon_query: String::new(),
        });
        self.confirm_delete = false;
        self.error = None;
    }

    /// Writes the draft to disk. On failure the vault is unchanged and the draft is kept.
    pub(super) fn save_draft(&mut self) -> Result<(), String> {
        let Some(draft) = &self.draft else {
            return Ok(());
        };
        if draft.entry.title.trim().is_empty() {
            return Err("Укажите название записи".into());
        }
        if let Some(totp) = &draft.entry.totp {
            totp::validate(totp).map_err(|error| format!("TOTP: {error}"))?;
        }
        let mut entry = draft.entry.clone();
        if let Some(totp) = &mut entry.totp {
            totp.secret = totp::normalize_secret(&totp.secret);
        }
        let id = entry.id.clone();
        let previous = self.vault.entries.clone();
        match self.vault.entries.iter_mut().find(|e| e.id == id) {
            Some(slot) => *slot = entry,
            None => self.vault.entries.push(entry),
        }
        if let Err(error) = self.vault.save() {
            self.vault.entries = previous;
            return Err(format!("Не удалось сохранить: {error}"));
        }
        self.draft = None;
        self.select(id);
        Ok(())
    }

    pub(super) fn delete_selected(&mut self) -> Result<(), String> {
        let Some(id) = self.selected.clone() else {
            return Ok(());
        };
        let previous = self.vault.entries.clone();
        self.vault.entries.retain(|entry| entry.id != id);
        if let Err(error) = self.vault.save() {
            self.vault.entries = previous;
            return Err(format!("Не удалось сохранить: {error}"));
        }
        self.selected = None;
        self.confirm_delete = false;
        Ok(())
    }
}

fn matches(entry: &Entry, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    query.is_empty()
        || [&entry.title, &entry.username, &entry.url]
            .iter()
            .any(|field| field.to_lowercase().contains(&query))
}
