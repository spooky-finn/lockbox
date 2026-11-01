//! Entry editing, TOTP-link parsing and password generation.

use crate::records::{
    entry::{Entry, TotpAlgorithm},
    generator, totp,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy)]
pub(crate) enum Field {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Totp,
}

pub(crate) struct Draft {
    pub(crate) entry: Entry,
    pub(crate) is_new: bool,
    pub(crate) show_password: bool,
    pub(crate) show_totp: bool,
    pub(crate) totp_error: Option<String>,
    pub(crate) generator_open: bool,
    pub(crate) generator_error: Option<String>,
    pub(crate) icon_picker_open: bool,
    pub(crate) icon_query: String,
}

impl Draft {
    pub(crate) fn new(entry: Entry, is_new: bool) -> Self {
        Self {
            entry,
            is_new,
            show_password: false,
            show_totp: false,
            totp_error: None,
            generator_open: false,
            generator_error: None,
            icon_picker_open: false,
            icon_query: String::new(),
        }
    }

    /// Validate a save snapshot without changing the draft or its revealed fields.
    pub(crate) fn prepare_save(&self) -> Result<Entry, String> {
        if self.entry.title.trim().is_empty() {
            return Err("Enter an entry name".into());
        }

        if let Some(totp) = &self.entry.totp {
            totp::validate(totp).map_err(|error| format!("TOTP: {error}"))?;
        }

        let mut entry = self.entry.clone();
        if let Some(totp) = &mut entry.totp {
            totp.secret = totp::normalize_secret(&totp.secret);
        }
        Ok(entry)
    }

    /// A pasted otpauth:// link replaces the TOTP settings and fills an empty title and login.
    pub(crate) fn apply_link(&mut self) {
        let Some(totp) = &self.entry.totp else {
            return;
        };

        if !totp::is_link(&totp.secret) {
            self.totp_error = None;
            return;
        }

        let text = Zeroizing::new(totp.secret.clone());

        match totp::parse_link(&text) {
            Ok(link) => {
                if self.entry.title.trim().is_empty()
                    && let Some(issuer) = &link.issuer
                {
                    self.entry.title = issuer.clone();
                }

                if self.entry.username.trim().is_empty()
                    && let Some(account) = &link.account
                {
                    self.entry.username = account.clone();
                }

                self.entry.totp = Some(link.totp);
                self.totp_error = None;
            }
            Err(error) => self.totp_error = Some(error.to_string()),
        }
    }

    pub(crate) fn set_field(&mut self, field: Field, value: String) {
        let value = Zeroizing::new(value);
        let target = match field {
            Field::Title => &mut self.entry.title,
            Field::Username => &mut self.entry.username,
            Field::Password => &mut self.entry.password,
            Field::Url => &mut self.entry.url,
            Field::Notes => &mut self.entry.notes,
            Field::Totp => {
                let Some(totp) = &mut self.entry.totp else {
                    return;
                };
                &mut totp.secret
            }
        };
        replace_secret(target, &value);

        if matches!(field, Field::Totp) {
            self.apply_link();
        }
    }

    pub(crate) fn generate(&mut self, options: &generator::Options) {
        match generator::generate(options) {
            Ok(password) => {
                replace_secret(&mut self.entry.password, &password);
                self.generator_error = None;
                self.generator_open = false;
            }
            Err(error) => self.generator_error = Some(error.to_string()),
        }
    }
}

pub(crate) fn replace_secret(target: &mut String, value: &str) {
    target.zeroize();
    target.push_str(value);
}

pub(crate) fn algorithm_name(algorithm: TotpAlgorithm) -> &'static str {
    match algorithm {
        TotpAlgorithm::Sha1 => "SHA1",
        TotpAlgorithm::Sha256 => "SHA256",
        TotpAlgorithm::Sha512 => "SHA512",
    }
}

#[cfg(test)]
#[path = "tests/editor.rs"]
mod tests;
