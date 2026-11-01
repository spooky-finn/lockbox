use super::*;

pub(super) fn identity(seed: u8) -> (DeviceSecret, Signed<Certificate>) {
    DeviceSecret::generate(&SigningKey::from_bytes(&[seed; 32])).unwrap()
}

pub(super) fn owner() -> (SyncState, DeviceSecret) {
    let (device, certificate) = identity(1);
    let state = Machine::create(
        Hash([1; 32]),
        Hash([2; 32]),
        certificate,
        &device,
        &[3; 32],
        Hash([4; 32]),
    )
    .unwrap();
    (state, device)
}

pub(super) fn entry(id: u8, password: &str) -> Entry {
    let mut entry = Entry::default();
    entry.id = format!("{id:032x}");
    entry.title = format!("Account {id}");
    entry.password = password.into();
    entry
}

pub(super) fn edit(state: &mut SyncState, device: &DeviceSecret, entries: Vec<Entry>) -> Hash {
    let replay = Machine::replay(state, device).unwrap();
    let transaction =
        Transaction::diff(&replay.projection, &entries, &replay.projection.folders()).unwrap();
    commit(state, device, transaction)
}

pub(super) fn commit(
    state: &mut SyncState,
    device: &DeviceSecret,
    transaction: Transaction,
) -> Hash {
    state.intent = Some(Machine::prepare(state, device, transaction).unwrap());
    Machine::complete_intent(state, device).unwrap()
}

pub(super) fn admit(
    state: &mut SyncState,
    owner: &DeviceSecret,
    certificate: Signed<Certificate>,
    role: Role,
) {
    let mut members = Machine::members(state, owner).unwrap();
    members.push((certificate, role));
    state.intent = Some(Machine::prepare_membership(state, owner, members).unwrap());
    Machine::complete_intent(state, owner).unwrap();
}

pub(super) fn deliver(from: &SyncState, to: &mut SyncState) {
    for (hash, bytes) in from.objects.iter().rev() {
        Machine::receive(to, *hash, bytes.0.clone()).unwrap();
    }
}

pub(super) fn encrypted_change(
    state: &SyncState,
    membership: Hash,
    device: &DeviceSecret,
    sequence: u64,
    previous: Option<Hash>,
    parents: Vec<Hash>,
    transaction: &Transaction,
) -> Object {
    let control = control_at(state, membership).unwrap();
    let key = epoch_key(&control.body, device).unwrap();
    let content_key = crypto::content_key(&key, state.space, control.body.epoch.context, false);
    let mut nonce = [0; 24];
    getrandom::fill(&mut nonce).unwrap();
    let header = ChangeHeader {
        profile: PROFILE.into(),
        space: state.space,
        membership,
        epoch: control.body.epoch.number,
        author: device.id(),
        sequence: Counter(sequence),
        previous,
        parents,
        nonce: Blob(nonce.to_vec()),
    };
    let ciphertext = crypto::encrypt(
        &content_key,
        &nonce,
        &canonical(transaction).unwrap(),
        &canonical(&header).unwrap(),
    )
    .unwrap();
    Object::Change(
        crypto::sign(
            Change { header, ciphertext },
            &device.signer(),
            CHANGE_DOMAIN,
        )
        .unwrap(),
    )
}

pub(super) fn empty_transaction() -> Transaction {
    Transaction {
        mutations: vec![],
        rebased_from: None,
    }
}
