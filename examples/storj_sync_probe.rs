//! Live acceptance probe. Uses a new random namespace and fictional records.
//! Required environment names and operational details: docs/sync-live-test.md.

#[path = "../tests/support/sync_scenario.rs"]
mod scenario;

use lockbox::{
    sync::transport::storj::Storj,
    vault::{
        KdfParams, ProfileStore, ProviderConfiguration, SharedProfile, witness::DirectoryWitness,
    },
};
use secrecy::SecretString;
use std::sync::Arc;
use zeroize::Zeroizing;

fn setting(suffix: &str) -> scenario::Result<Zeroizing<String>> {
    let name = format!("LOCKBOX_STORJ_{suffix}");
    let value = std::env::var(&name).map_err(|_| format!("missing environment setting {name}"))?;
    if value.is_empty() {
        return Err(format!("empty environment setting {name}").into());
    }
    Ok(Zeroizing::new(value))
}

fn password() -> scenario::Result<SecretString> {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut bytes = Zeroizing::new([0; 32]);
    getrandom::fill(bytes.as_mut()).map_err(|_| "operating system randomness unavailable")?;
    Ok(SecretString::from(URL_SAFE_NO_PAD.encode(bytes.as_ref())))
}

fn main() -> scenario::Result<()> {
    let endpoint = setting("ENDPOINT")?;
    let bucket = setting("BUCKET")?;
    let owner_access = setting("OWNER_ACCESS_KEY")?;
    let owner_secret = setting("OWNER_SECRET_KEY")?;
    let editor_access = setting("EDITOR_ACCESS_KEY")?;
    let editor_secret = setting("EDITOR_SECRET_KEY")?;
    if owner_access.as_str() == editor_access.as_str() {
        return Err("the probe requires independent credentials for both devices".into());
    }
    let region = std::env::var("LOCKBOX_STORJ_REGION").unwrap_or_else(|_| "us-east-1".into());
    let directory = tempfile::tempdir()?;
    let witness_directory = tempfile::tempdir()?;
    let profiles = ProfileStore::new(Arc::new(DirectoryWitness::new(
        witness_directory.path().to_owned(),
    )));
    let create = |name: &str, access: &str, secret: &str| {
        Ok::<_, Box<dyn std::error::Error>>(profiles.create_shared(
            directory.path().join(format!("{name}.lbx")),
            &password()?,
            &password()?,
            KdfParams::default(),
            SharedProfile {
                username: name.into(),
                space_name: "Disposable Storj probe".into(),
                provider: ProviderConfiguration {
                    base_url: endpoint.to_string(),
                    bucket: bucket.to_string(),
                    access_key: access.into(),
                    secret_key: secret.into(),
                },
            },
        )?)
    };
    let mut owner = create("Owner", &owner_access, &owner_secret)?;
    let mut editor = create("Editor", &editor_access, &editor_secret)?;
    let namespace = owner
        .space_descriptor()
        .ok_or("missing descriptor")?
        .namespace;
    println!("Disposable probe namespace: spaces/{namespace}/");
    let mut owner_storage =
        Storj::with_region(&endpoint, &bucket, &owner_access, &owner_secret, &region)?;
    let mut editor_storage =
        Storj::with_region(&endpoint, &bucket, &editor_access, &editor_secret, &region)?;
    scenario::exercise(
        &mut owner,
        &mut editor,
        &mut owner_storage,
        &mut editor_storage,
    )?;
    let owner_key = owner.export_key();
    let editor_key = editor.export_key();
    let owner_path = owner.path().to_owned();
    let editor_path = editor.path().to_owned();
    drop(owner);
    drop(editor);
    scenario::check_reopened(
        &profiles.open_with_key(owner_path, &owner_key)?,
        &profiles.open_with_key(editor_path, &editor_key)?,
    )?;
    println!(
        "PASS: admission, offline conflicts, exact-byte retry, resolution, reader enforcement, revocation and restart."
    );
    println!(
        "Only encrypted fixture objects remain under the probe namespace; local profiles were temporary."
    );
    Ok(())
}
