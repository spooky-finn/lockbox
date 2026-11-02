//! Native messaging for the browser companion. No HTTP server or browser-side vault.

use crate::{
    platform::autolock::{AutoLock, State},
    records::{entry::Entry, generator},
    vault::{Vault, VaultError},
};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    sync::mpsc,
    time::{Duration, Instant, SystemTime},
};
use url::{Origin, Url};
use zeroize::Zeroizing;

const MAX_MESSAGE: usize = 64 * 1024;
const MAX_SESSION: Duration = Duration::from_secs(15 * 60);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: u32,
    request: Request,
}

#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Status {},
    Unlock {
        path: String,
        password: Zeroizing<String>,
    },
    Lock {},
    Suggestions {
        origin: String,
    },
    Credential {
        origin: String,
        entry_id: String,
    },
    Generate {
        origin: String,
        length: usize,
        symbols: bool,
    },
    SaveGenerated {
        origin: String,
        title: String,
        username: Zeroizing<String>,
        entry_id: Option<String>,
    },
}

#[derive(Serialize)]
struct Reply<'a> {
    id: u32,
    #[serde(flatten)]
    result: Outcome<'a>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum Outcome<'a> {
    Ok { data: Data<'a> },
    Error { error: &'static str },
}

#[derive(Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Data<'a> {
    State {
        unlocked: bool,
    },
    Suggestions {
        items: Vec<Suggestion<'a>>,
    },
    Credential {
        // The vault owns and wipes this text; replies borrow it through serialization.
        username: &'a str,
        password: &'a str,
    },
    Generated {
        length: usize,
    },
    Saved {
        entry_id: String,
    },
}

#[derive(Serialize)]
struct Suggestion<'a> {
    id: &'a str,
    title: &'a str,
    username: &'a str,
}

fn preview(value: &str) -> &str {
    value
        .char_indices()
        .nth(64)
        .map_or(value, |(end, _)| &value[..end])
}

struct Generated {
    origin: Origin,
    password: Zeroizing<String>,
}

#[derive(Default)]
struct Session {
    vault: Option<Vault>,
    generated: Option<Generated>,
    clock: Option<AutoLock>,
    opened: Option<(Instant, SystemTime)>,
}

/// Exact scheme/host/effective-port matching, with no suffix or subdomain fallback.
fn origin(value: &str) -> Result<Origin, &'static str> {
    if value.len() > 4096 {
        return Err("Use an HTTPS website.");
    }
    let url = Url::parse(value).map_err(|_| "Use an HTTPS website.")?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err("Use an HTTPS website without embedded credentials.");
    }
    Ok(url.origin())
}

fn matches(entry: &Entry, expected: &Origin) -> bool {
    origin(&entry.url).is_ok_and(|actual| actual == *expected)
}

/// Credential reads and password updates must enforce the same account-origin check.
fn account<'a>(
    entries: &'a [Entry],
    id: &str,
    website: &Origin,
) -> Result<&'a Entry, &'static str> {
    entries
        .iter()
        .find(|entry| entry.id == id && matches(entry, website))
        .ok_or("No matching account for this exact origin.")
}

impl Session {
    fn lock(&mut self) {
        // Vault/Entry/Zeroizing own wiping; dropping also releases the file lease.
        *self = Self::default();
    }

    fn expire(&mut self, now: Instant, wall: SystemTime) {
        let idle = self
            .clock
            .as_ref()
            .is_some_and(|clock| clock.check(now, wall) == State::Lock);
        let lifetime = self.opened.is_some_and(|(opened, opened_wall)| {
            now.saturating_duration_since(opened)
                .max(wall.duration_since(opened_wall).unwrap_or_default())
                >= MAX_SESSION
        });
        if idle || lifetime {
            self.lock();
        }
    }

    fn handle(&mut self, request: Request) -> Result<Data<'_>, &'static str> {
        let now = Instant::now();
        let wall = SystemTime::now();
        self.expire(now, wall);
        match request {
            Request::Status {} => {
                // Polling must never extend the idle deadline.
                return Ok(Data::State {
                    unlocked: self.vault.is_some(),
                });
            }
            Request::Lock {} => {
                self.lock();
                return Ok(Data::State { unlocked: false });
            }
            Request::Unlock { path, mut password } => {
                self.lock();
                if path.len() > 4096 || password.is_empty() || password.len() > 4096 {
                    return Err("Enter a vault path and master password.");
                }
                let password = SecretString::from(std::mem::take(&mut *password));
                let vault = Vault::open(path, &password).map_err(|error| match error {
                    VaultError::ProfileInUse => {
                        "This vault is open elsewhere. Lock it in the desktop app or other browser first."
                    }
                    VaultError::WrongPassword => "Incorrect master password.",
                    _ => "Could not open the vault. Check its path, password and file integrity.",
                })?;
                self.vault = Some(vault);
                self.opened = Some((Instant::now(), SystemTime::now()));
                self.clock = Some(AutoLock::new(5, Instant::now(), SystemTime::now()));
                return Ok(Data::State { unlocked: true });
            }
            _ => {}
        }
        let vault = self.vault.as_mut().ok_or("Unlock your vault first.")?;
        let data = match request {
            Request::Suggestions { origin: website } => {
                let website = origin(&website)?;
                let items = vault
                    .entries()
                    .iter()
                    .filter(|entry| entry.id.len() <= 64 && matches(entry, &website))
                    .take(50)
                    .map(|entry| Suggestion {
                        id: &entry.id,
                        title: preview(&entry.title),
                        username: preview(&entry.username),
                    })
                    .collect();
                Data::Suggestions { items }
            }
            Request::Credential {
                origin: website,
                entry_id,
            } => {
                let website = origin(&website)?;
                let entry = account(vault.entries(), &entry_id, &website)?;
                if entry.username.len() > 4096 || entry.password.len() > 4096 {
                    return Err("This account is too large for browser autofill.");
                }
                Data::Credential {
                    username: &entry.username,
                    password: &entry.password,
                }
            }
            Request::Generate {
                origin: website,
                length,
                symbols,
            } => {
                let website = origin(&website)?;
                if !(16..=128).contains(&length) {
                    return Err("Choose a password length between 16 and 128.");
                }
                let password = generator::generate(&generator::Options {
                    length,
                    symbols,
                    ..Default::default()
                })
                .map_err(|_| "Could not generate a password.")?;
                self.generated = Some(Generated {
                    origin: website,
                    password,
                });
                Data::Generated { length }
            }
            Request::SaveGenerated {
                origin: website,
                title,
                username,
                entry_id,
            } => {
                let website = origin(&website)?;
                if title.len() > 512 || username.len() > 512 {
                    return Err("Title and username must be at most 512 bytes each.");
                }
                let generated = self
                    .generated
                    .as_ref()
                    .filter(|value| value.origin == website)
                    .ok_or("Generate a new password for this website first.")?;
                let mut entry = match entry_id {
                    Some(id) => account(vault.entries(), &id, &website)?.clone(),
                    None => {
                        let website = website.ascii_serialization();
                        let mut entry = Entry::new();
                        entry.title = if title.trim().is_empty() {
                            website.clone()
                        } else {
                            title
                        };
                        entry.username = username.to_string();
                        entry.url = website;
                        entry
                    }
                };
                entry.password = generated.password.to_string();
                let entry_id = entry.id.clone();
                vault.upsert_entry(entry)
                    .map_err(|_| "Could not save. Nothing was filled. Check vault access in the desktop app.")?;
                self.generated = None;
                Data::Saved { entry_id }
            }
            Request::Status {} | Request::Unlock { .. } | Request::Lock {} => unreachable!(),
        };
        if let Some(clock) = &mut self.clock {
            clock.activity(Instant::now(), SystemTime::now());
        }
        Ok(data)
    }
}

fn read_message(reader: &mut impl Read) -> io::Result<Option<Zeroizing<Vec<u8>>>> {
    let mut length = [0u8; 4];
    // Only a clean EOF between frames is normal. Truncation is an error.
    if reader.read(&mut length[..1])? == 0 {
        return Ok(None);
    }
    reader.read_exact(&mut length[1..])?;
    let length = u32::from_ne_bytes(length) as usize;
    if length == 0 || length > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid frame length",
        ));
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    reader.read_exact(&mut bytes)?;
    Ok(Some(bytes))
}

fn write_reply(writer: &mut impl Write, reply: &Reply<'_>) -> io::Result<()> {
    let bytes = Zeroizing::new(serde_json::to_vec(reply)?);
    if bytes.len() > MAX_MESSAGE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "reply too large",
        ));
    }
    writer.write_all(&(bytes.len() as u32).to_ne_bytes())?;
    writer.write_all(&bytes)?;
    writer.flush()
}

/// Serve one browser-owned stdio connection. Disconnects and malformed frames
/// drop the vault; timeouts run independently of browser background scheduling.
pub fn run() -> io::Result<()> {
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        while let Ok(Some(bytes)) = read_message(&mut input) {
            if sender.send(bytes).is_err() {
                break;
            }
        }
    });
    let mut session = Session::default();
    let mut output = io::stdout().lock();
    loop {
        session.expire(Instant::now(), SystemTime::now());
        let bytes = match receiver.recv_timeout(Duration::from_millis(250)) {
            Ok(bytes) => bytes,
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        };
        let message: Message = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid message"))?;
        let result = match session.handle(message.request) {
            Ok(data) => Outcome::Ok { data },
            Err(error) => Outcome::Error { error },
        };
        write_reply(
            &mut output,
            &Reply {
                id: message.id,
                result,
            },
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
