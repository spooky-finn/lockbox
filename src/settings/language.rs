//! Supported interface languages. English is the default for unset or unknown values.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Language {
    #[serde(rename = "ru")]
    Russian,
    #[default]
    #[serde(rename = "en", other)]
    English,
}

impl Language {
    pub fn code(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Russian => "ru",
        }
    }
}
