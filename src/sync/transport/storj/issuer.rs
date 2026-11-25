//! Prepare locally, persist the per-device grant, then register. Retried registrations
//! are descendants of the same revocation handle.
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::Write,
    process::{Command, Stdio},
};
use zeroize::Zeroizing;

#[derive(Debug)]
pub(crate) struct IssuerError;

#[derive(Deserialize)]
pub(crate) struct MemberCredentials {
    pub endpoint: String,
    pub access: Zeroizing<String>,
    pub secret: Zeroizing<String>,
}

#[derive(Serialize, Default)]
struct Request<'a> {
    op: &'a str,
    owner: &'a str,
    child: &'a str,
    bucket: &'a str,
    prefix: &'a str,
    object: &'a str,
    digest: &'a str,
}

fn run<T: DeserializeOwned>(request: Request<'_>) -> Result<T, IssuerError> {
    let binary = std::env::current_exe()
        .map_err(|_| IssuerError)?
        .with_file_name(format!(
            "lockbox-storj-issuer{}",
            std::env::consts::EXE_SUFFIX
        ));
    let body = Zeroizing::new(serde_json::to_vec(&request).map_err(|_| IssuerError)?);
    let mut helper = Command::new(binary)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| IssuerError)?;
    let wrote = helper.stdin.take().ok_or(IssuerError)?.write_all(&body);
    let response = helper.wait_with_output().map_err(|_| IssuerError)?;
    let output = Zeroizing::new(response.stdout);
    if wrote.is_err() || !response.status.success() {
        return Err(IssuerError);
    }
    serde_json::from_slice(&output).map_err(|_| IssuerError)
}

pub(crate) fn prepare(
    owner_grant: &str,
    bucket: &str,
    space_name: &str,
) -> Result<Zeroizing<String>, IssuerError> {
    let prefix = super::object_prefix(space_name).map_err(|_| IssuerError)?;
    run(Request {
        op: "prepare",
        owner: owner_grant,
        bucket,
        prefix: &prefix,
        ..Default::default()
    })
}

pub(crate) fn verified_issuer(
    owner_grant: &str,
    bucket: &str,
    space: &str,
    genesis: crate::sync::protocol::Hash,
    expected: &[u8],
) -> Result<Zeroizing<String>, IssuerError> {
    use sha2::{Digest, Sha256};
    let prefix = super::object_prefix(space).map_err(|_| IssuerError)?;
    run(Request {
        op: "verify",
        owner: owner_grant,
        bucket,
        prefix: &prefix,
        object: &format!("{genesis}.json"),
        digest: &format!("{:x}", Sha256::digest(expected)),
        ..Default::default()
    })
}

pub(crate) fn register(child_grant: &str) -> Result<MemberCredentials, IssuerError> {
    run(Request {
        op: "register",
        child: child_grant,
        ..Default::default()
    })
}

pub(crate) fn revoke(owner_grant: &str, child_grant: &str) -> Result<(), IssuerError> {
    run(Request {
        op: "revoke",
        owner: owner_grant,
        child: child_grant,
        ..Default::default()
    })
}
