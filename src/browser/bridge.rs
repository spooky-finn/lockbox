//! One-use local callback. Deep links contain only a random rendezvous token.
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    path::PathBuf,
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub const TIMEOUT: Duration = Duration::from_secs(120);
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Credential {
    pub username: Zeroizing<String>,
    pub password: Zeroizing<String>,
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Hello {
    origin: String,
}

fn token_valid(token: &str) -> bool {
    token.len() == 64
        && token
            .bytes()
            .all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c))
}

#[cfg(unix)]
fn socket_path(token: &str) -> io::Result<PathBuf> {
    use std::os::unix::fs::{DirBuilderExt, MetadataExt};
    if !token_valid(token) {
        return Err(io::Error::other("invalid request token"));
    }
    #[cfg(not(test))]
    let directory = dirs::home_dir()
        .ok_or_else(|| io::Error::other("no home directory"))?
        .join(".lockbox/browser-bridge");
    #[cfg(test)]
    let directory = {
        static ROOT: std::sync::LazyLock<tempfile::TempDir> =
            std::sync::LazyLock::new(|| tempfile::tempdir().unwrap());
        ROOT.path().join("bridge")
    };
    std::fs::create_dir_all(directory.parent().unwrap())?;
    match std::fs::DirBuilder::new().mode(0o700).create(&directory) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let metadata = std::fs::symlink_metadata(&directory)?;
    if !metadata.is_dir()
        || metadata.uid() != unsafe { libc::geteuid() }
        || metadata.mode() & 0o077 != 0
    {
        return Err(io::Error::other("unsafe bridge directory"));
    }
    // Short enough for macOS sockaddr_un even with a long username.
    Ok(directory.join(&token[..32]))
}

fn write_frame<T: Serialize>(stream: &mut impl Write, value: &T) -> io::Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(value)?);
    if bytes.len() > super::MAX_MESSAGE {
        return Err(io::Error::other("reply too large"));
    }
    stream.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    stream.write_all(&bytes)?;
    stream.flush()
}
fn read_frame<T: serde::de::DeserializeOwned>(stream: &mut impl Read) -> io::Result<T> {
    let bytes =
        super::read_message(stream)?.ok_or_else(|| io::Error::other("connection closed"))?;
    Ok(serde_json::from_slice(&bytes)?)
}

#[cfg(unix)]
struct SocketFile(PathBuf);
#[cfg(unix)]
impl Drop for SocketFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

#[cfg(unix)]
pub fn request(origin: &str) -> io::Result<Credential> {
    request_with(origin, |link| {
        #[cfg(target_os = "macos")]
        let status = std::process::Command::new("/usr/bin/open")
            .arg(link)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        #[cfg(not(target_os = "macos"))]
        let status = std::process::Command::new("xdg-open")
            .arg(link)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()?;
        if status.success() {
            Ok(())
        } else {
            Err(io::Error::other("could not open Lockbox"))
        }
    })
}

#[cfg(unix)]
fn request_with(
    origin: &str,
    launch: impl FnOnce(&str) -> io::Result<()>,
) -> io::Result<Credential> {
    use std::os::unix::net::UnixListener;
    let origin = super::origin(origin)
        .map_err(io::Error::other)?
        .ascii_serialization();
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let path = socket_path(&token)?;
    let listener = UnixListener::bind(&path)?;
    let _file = SocketFile(path);
    listener.set_nonblocking(true)?;
    launch(&format!("lockbox://browser/fill?request={token}"))?;
    let deadline = Instant::now() + TIMEOUT;
    loop {
        if Instant::now() >= deadline {
            return Err(io::Error::new(io::ErrorKind::TimedOut, "consent expired"));
        }
        match listener.accept() {
            Ok((mut stream, _)) => {
                stream.set_nonblocking(false)?;
                let remaining = deadline.saturating_duration_since(Instant::now());
                stream.set_read_timeout(Some(remaining))?;
                stream.set_write_timeout(Some(Duration::from_secs(1)))?;
                // Authenticate before disclosing even the website origin.
                let received: String = read_frame(&mut stream)?;
                use subtle::ConstantTimeEq;
                if !bool::from(received.as_bytes().ct_eq(token.as_bytes())) {
                    continue;
                }
                write_frame(
                    &mut stream,
                    &Hello {
                        origin: origin.clone(),
                    },
                )?;
                let reply: Option<Credential> = read_frame(&mut stream)?;
                return reply
                    .filter(|c| c.username.len() <= 4096 && c.password.len() <= 4096)
                    .ok_or_else(|| io::Error::other("cancelled or oversized credential"));
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25))
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(not(unix))]
pub fn request(_: &str) -> io::Result<Credential> {
    Err(io::Error::other("desktop bridge requires Unix"))
}

#[cfg(unix)]
pub struct DesktopRequest {
    pub origin: String,
    pub deadline: Instant,
    stream: std::os::unix::net::UnixStream,
}
#[cfg(unix)]
impl DesktopRequest {
    pub fn connect(link: &str) -> io::Result<Self> {
        let url = url::Url::parse(link).map_err(io::Error::other)?;
        let pairs: Vec<_> = url.query_pairs().collect();
        if url.scheme() != "lockbox"
            || url.host_str() != Some("browser")
            || url.path() != "/fill"
            || !url.username().is_empty()
            || url.password().is_some()
            || url.port().is_some()
            || url.fragment().is_some()
            || pairs.len() != 1
            || pairs[0].0 != "request"
        {
            return Err(io::Error::other("invalid browser link"));
        }
        let token = &pairs[0].1;
        let mut stream = std::os::unix::net::UnixStream::connect(socket_path(token)?)?;
        stream.set_read_timeout(Some(Duration::from_secs(1)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        write_frame(&mut stream, &token)?;
        let hello: Hello = read_frame(&mut stream)?;
        let origin = super::origin(&hello.origin)
            .map_err(io::Error::other)?
            .ascii_serialization();
        Ok(Self {
            origin,
            deadline: Instant::now() + TIMEOUT,
            stream,
        })
    }
    pub fn active(&self) -> bool {
        if Instant::now() >= self.deadline {
            return false;
        }
        // Poll without consuming bytes; EOF means the browser cancelled.
        let mut descriptor = libc::pollfd {
            fd: std::os::fd::AsRawFd::as_raw_fd(&self.stream),
            events: libc::POLLIN,
            revents: 0,
        };
        unsafe { libc::poll(&mut descriptor, 1, 0) == 0 }
    }
    pub fn respond(mut self, credential: Option<Credential>) -> io::Result<()> {
        if !self.active() {
            return Err(io::Error::other("browser request expired"));
        }
        write_frame(&mut self.stream, &credential)
    }
}

// Availability reads only the local plaintext site index, even with the app closed.
pub fn has_match(origin: &str) -> bool {
    super::index::has_match(origin)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn token_rejects_paths_and_short_values() {
        for value in ["../socket", "", "ABC", &"g".repeat(64)] {
            assert!(!token_valid(value));
        }
        assert!(token_valid(&"a".repeat(64)));
    }
    #[cfg(unix)]
    #[test]
    fn desktop_consent_round_trip_and_cancel() {
        for approve in [true, false] {
            let mut used_link = String::new();
            let mut desktop = None;
            let result = request_with("https://accounts.example/login", |link| {
                used_link = link.to_owned();
                let link = link.to_owned();
                assert!(!link.contains("accounts.example"));
                desktop = Some(std::thread::spawn(move || {
                    let request = DesktopRequest::connect(&link).unwrap();
                    assert_eq!(request.origin, "https://accounts.example");
                    assert!(request.active());
                    request
                        .respond(approve.then(|| Credential {
                            username: Zeroizing::new("fictional-user".into()),
                            password: Zeroizing::new("fictional-password".into()),
                        }))
                        .unwrap();
                }));
                Ok(())
            });
            desktop.unwrap().join().unwrap();
            assert!(
                DesktopRequest::connect(&used_link).is_err(),
                "consumed link cannot be replayed"
            );
            assert_eq!(
                result.is_ok(),
                approve,
                "relay error: {:?}",
                result.as_ref().err()
            );
            if let Ok(reply) = result {
                assert_eq!(&*reply.password, "fictional-password");
            }
        }
    }
    #[cfg(unix)]
    #[test]
    fn desktop_request_rejects_disconnect_and_expiry() {
        for expired in [true, false] {
            let (stream, peer) = std::os::unix::net::UnixStream::pair().unwrap();
            let request = DesktopRequest {
                origin: "https://accounts.example".into(),
                deadline: if expired {
                    Instant::now() - Duration::from_secs(1)
                } else {
                    Instant::now() + TIMEOUT
                },
                stream,
            };
            if !expired {
                drop(peer);
            }
            assert!(!request.active());
            assert!(
                request
                    .respond(Some(Credential {
                        username: Zeroizing::new("fictional-user".into()),
                        password: Zeroizing::new("fictional-password".into())
                    }))
                    .is_err()
            );
        }
    }
    #[cfg(unix)]
    #[test]
    fn desktop_rejects_untrusted_callback_urls() {
        for link in [
            "https://browser/fill?request=bad",
            "lockbox://browser/fill?request=bad&request=other",
            "lockbox://browser/fill?request=bad&callback=https://attacker.test",
            "lockbox://user@browser/fill?request=bad",
            "lockbox://browser/fill?request=../socket",
        ] {
            assert!(DesktopRequest::connect(link).is_err());
        }
    }
}
