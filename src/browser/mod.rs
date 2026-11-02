//! Thin native messaging relay. Vault access and consent belong to the desktop app.
use serde::{Deserialize, Serialize};
use std::{
    io::{self, Read, Write},
    time::Duration,
};
use url::{Origin, Url};
use zeroize::Zeroizing;
pub mod bridge;
pub(crate) mod index;
const MAX_MESSAGE: usize = 64 * 1024;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Message {
    id: u32,
    request: Request,
}
#[derive(Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
enum Request {
    Fill { origin: String },
    Match { origin: String },
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
    Error { error: &'a str },
}
#[derive(Serialize)]
#[serde(untagged)]
enum Data<'a> {
    Credential(&'a bridge::Credential),
    Match { matched: bool },
}
pub(crate) fn origin(value: &str) -> Result<Origin, &'static str> {
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

/// EOF cancels in-flight consent; no vault or master password enters this process.
pub fn run() -> io::Result<()> {
    let (sender, receiver) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match read_message(&mut input) {
                Ok(Some(bytes)) => {
                    if sender.send(Ok(bytes)).is_err() {
                        break;
                    }
                }
                Ok(None) => break,
                Err(error) => {
                    let _ = sender.send(Err(error));
                    break;
                }
            }
        }
    });
    let mut output = io::stdout().lock();
    while let Ok(bytes) = receiver.recv() {
        let bytes = bytes?;
        let message: Message = serde_json::from_slice(&bytes)
            .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "invalid message"))?;
        let website = match &message.request {
            Request::Fill { origin } | Request::Match { origin } => origin,
        };
        let website = origin(website)
            .map_err(io::Error::other)?
            .ascii_serialization();
        if matches!(message.request, Request::Match { .. }) {
            write_reply(
                &mut output,
                &Reply {
                    id: message.id,
                    result: Outcome::Ok {
                        data: Data::Match {
                            matched: bridge::has_match(&website),
                        },
                    },
                },
            )?;
            continue;
        }
        let (done, result) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let _ = done.send(bridge::request(&website));
        });
        let response = loop {
            match result.recv_timeout(Duration::from_millis(100)) {
                Ok(result) => break result,
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(io::Error::other("relay stopped"));
                }
                Err(_) => {}
            }
            match receiver.try_recv() {
                Err(std::sync::mpsc::TryRecvError::Disconnected) => return Ok(()),
                Ok(_) => return Err(io::Error::other("overlapping request")),
                Err(_) => {}
            }
        };
        let outcome = match &response {
            Ok(data) => Outcome::Ok {
                data: Data::Credential(data),
            },
            Err(_) => Outcome::Error {
                error: "Desktop request cancelled or timed out. Check Lockbox installation and try again.",
            },
        };
        write_reply(
            &mut output,
            &Reply {
                id: message.id,
                result: outcome,
            },
        )?;
    }
    Ok(())
}
#[cfg(test)]
mod tests;
