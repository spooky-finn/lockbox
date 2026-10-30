//! Entry editing, TOTP-link parsing and password generation.
use crate::{
    entry::{Entry, TotpAlgorithm},
    generator, totp,
};
use zeroize::{Zeroize, Zeroizing};

#[derive(Clone, Copy)]
pub(super) enum Field {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Totp,
}

pub(super) struct Draft {
    pub(super) entry: Entry,
    pub(super) is_new: bool,
    pub(super) show_password: bool,
    pub(super) show_totp: bool,
    pub(super) totp_error: Option<String>,
    pub(super) generator_open: bool,
    pub(super) generator_error: Option<String>,
    pub(super) icon_picker_open: bool,
    pub(super) icon_query: String,
}

impl Draft {
    /// A pasted otpauth:// link replaces the TOTP settings and fills an empty title and login.
    pub(super) fn apply_link(&mut self) {
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
}

impl Draft {
    pub(super) fn set_field(&mut self, field: Field, value: String) {
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
    pub(super) fn generate(&mut self, options: &generator::Options) {
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
pub(super) fn replace_secret(target: &mut String, value: &str) {
    target.zeroize();
    target.push_str(value);
}
pub(super) fn algorithm_name(algorithm: TotpAlgorithm) -> &'static str {
    match algorithm {
        TotpAlgorithm::Sha1 => "SHA1",
        TotpAlgorithm::Sha256 => "SHA256",
        TotpAlgorithm::Sha512 => "SHA512",
    }
}
