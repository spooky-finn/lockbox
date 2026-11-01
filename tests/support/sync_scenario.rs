//! The same acceptance scenario runs against memory and the live S3 adapter.

use lockbox::{
    records::entry::Entry,
    sync::{
        protocol::{Error, Hash, Role, Value},
        transport::{Page, Storage, TransportError},
    },
    vault::{Vault, VaultError},
};

pub type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn check(condition: bool, message: &'static str) -> Result<()> {
    if !condition {
        return Err(std::io::Error::other(message).into());
    }
    Ok(())
}

/// No real credentials or secret values enter messages, even on failure.
pub fn exercise(
    owner: &mut Vault,
    editor: &mut Vault,
    owner_storage: &mut dyn Storage,
    editor_storage: &mut dyn Storage,
) -> Result<()> {
    let owner_certificate = owner
        .device_certificate()
        .ok_or("missing owner certificate")?;
    let editor_certificate = editor
        .device_certificate()
        .ok_or("missing editor certificate")?;
    check(
        owner_certificate.body.device != editor_certificate.body.device,
        "devices must have independent keys",
    )?;
    let roster = |role| {
        vec![
            (owner_certificate.clone(), Role::Owner),
            (editor_certificate.clone(), role),
        ]
    };
    let mut entry = Entry::new();
    entry.title = "Disposable synchronization probe".into();
    entry.password = "fictional initial value".into();
    owner.upsert_entry(entry)?;
    let descriptor = owner.space_descriptor().ok_or("missing descriptor")?;
    let invitation = owner.issue_invitation(editor_certificate.clone(), Role::Editor, None)?;
    let admission = editor.accept_invitation(invitation, &descriptor)?;
    let receipt = owner.admit_invitation(admission, 0)?;
    editor.join_invitation(receipt)?;
    owner.synchronize(owner_storage)?;
    editor.synchronize(editor_storage)?;
    check(
        owner.entries() == editor.entries(),
        "initial admission did not converge",
    )?;

    let mut entry = owner.entries()[0].clone();
    entry.password = "fictional owner value".into();
    owner.upsert_entry(entry)?;
    let mut entry = editor.entries()[0].clone();
    entry.password = "fictional editor value".into();
    editor.upsert_entry(entry)?;
    let mut lossy = Observed {
        inner: owner_storage,
        writes: 0,
        lose_next: true,
    };
    check(
        matches!(
            owner.synchronize(&mut lossy),
            Err(VaultError::Transport(TransportError::Unavailable))
        ),
        "lost PUT acknowledgement did not report unavailability",
    )?;
    owner.synchronize(&mut lossy)?;
    let mut observed_editor = Observed {
        inner: editor_storage,
        writes: 0,
        lose_next: false,
    };
    editor.synchronize(&mut observed_editor)?;
    owner.synchronize(&mut lossy)?;
    let view = owner.sync_replay()?.ok_or("missing owner replay")?;
    let conflict = view
        .projection
        .conflicts()
        .next()
        .ok_or("concurrent edits were lost")?;
    check(
        conflict.versions.len() == 2,
        "expected two concurrent whole-record versions",
    )?;
    let peer = editor.sync_replay()?.ok_or("missing editor replay")?;
    check(
        view.projection.digest()? == peer.projection.digest()?,
        "conflict projections differ",
    )?;
    let chosen = conflict
        .versions
        .iter()
        .find(|version| {
            matches!(
                &version.value,
                Some(Value::Entry(entry)) if entry.password == "fictional editor value"
            )
        })
        .ok_or("editor revision is missing")?;
    owner.resolve_sync(conflict.entity.clone(), chosen.revision)?;
    owner.synchronize(&mut lossy)?;
    editor.synchronize(&mut observed_editor)?;
    check(
        owner.entries() == editor.entries(),
        "resolution did not converge",
    )?;

    owner.set_space_members(roster(Role::Reader))?;
    owner.synchronize(&mut lossy)?;
    editor.synchronize(&mut observed_editor)?;
    let mut entry = editor.entries()[0].clone();
    entry.title = "reader must not publish this".into();
    check(
        matches!(
            editor.upsert_entry(entry),
            Err(VaultError::Sync(Error::InsufficientRole))
        ),
        "reader was allowed to mutate the space",
    )?;
    owner.set_space_members(roster(Role::Editor))?;
    owner.synchronize(&mut lossy)?;
    editor.synchronize(&mut observed_editor)?;

    let mut entry = editor.entries()[0].clone();
    entry.password = "fictional offline draft".into();
    editor.upsert_entry(entry)?;
    owner.set_space_members(vec![(owner_certificate, Role::Owner)])?;
    owner.synchronize(&mut lossy)?;
    let before = observed_editor.writes;
    check(
        matches!(
            editor.synchronize(&mut observed_editor),
            Err(VaultError::Sync(Error::UnauthorizedDevice))
        ),
        "removed device retained authority",
    )?;
    check(
        observed_editor.writes == before,
        "removed device published obsolete history",
    )?;
    check(
        editor.excluded_edits()?.len() == 1,
        "excluded offline draft was lost",
    )?;
    check(
        owner.entries()[0].password == "fictional editor value",
        "revocation replaced the accepted state",
    )?;
    Ok(())
}

struct Observed<'a> {
    inner: &'a mut dyn Storage,
    writes: usize,
    lose_next: bool,
}

impl Storage for Observed<'_> {
    fn list(
        &mut self,
        namespace: Hash,
        cursor: Option<&str>,
    ) -> std::result::Result<Page, TransportError> {
        self.inner.list(namespace, cursor)
    }

    fn get(&mut self, namespace: Hash, hash: Hash) -> std::result::Result<Vec<u8>, TransportError> {
        self.inner.get(namespace, hash)
    }

    fn put(
        &mut self,
        namespace: Hash,
        hash: Hash,
        bytes: &[u8],
    ) -> std::result::Result<(), TransportError> {
        self.inner.put(namespace, hash, bytes)?;
        self.writes += 1;
        if std::mem::take(&mut self.lose_next) {
            Err(TransportError::Unavailable)
        } else {
            Ok(())
        }
    }
}

pub fn check_reopened(owner: &Vault, editor: &Vault) -> Result<()> {
    let owner_view = owner.sync_replay()?.ok_or("missing reopened owner state")?;
    check(owner_view.ready, "reopened owner state is incomplete")?;
    check(
        owner_view.projection.conflicts().next().is_none(),
        "resolved conflict reappeared on restart",
    )?;
    check(owner.entries().len() == 1, "reopened owner lost records")?;
    check(
        editor.excluded_edits()?.len() == 1,
        "reopened revoked device lost its draft",
    )?;
    check(
        editor
            .sync_replay()?
            .ok_or("missing reopened editor state")?
            .role
            .is_none(),
        "revocation was lost on restart",
    )
}
