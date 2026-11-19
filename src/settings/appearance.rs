//! Local appearance preferences, independent of vault contents.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemeMode {
    Light,
    Dark,
    #[default]
    #[serde(other)]
    System,
}

impl ThemeMode {
    pub fn code(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    pub fn is_dark(self, system_dark: bool) -> bool {
        match self {
            Self::System => system_dark,
            Self::Light => false,
            Self::Dark => true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Appearance {
    pub theme: ThemeMode,
    pub preset: ThemePreset,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreset {
    // Keep the saved preset IDs stable when their display names and styles change.
    Ocean,
    Forest,
    Plum,
    Amber,
    Rose,
    Teal,
    Indigo,
    Slate,
    Paper,
    #[default]
    #[serde(other)]
    Classic,
}

impl ThemePreset {
    pub const ALL: [Self; 10] = [
        Self::Classic,
        Self::Paper,
        Self::Forest,
        Self::Ocean,
        Self::Slate,
        Self::Amber,
        Self::Rose,
        Self::Teal,
        Self::Plum,
        Self::Indigo,
    ];

    pub fn code(self) -> &'static str {
        match self {
            Self::Classic => "classic",
            Self::Ocean => "ocean",
            Self::Forest => "forest",
            Self::Plum => "plum",
            Self::Amber => "amber",
            Self::Rose => "rose",
            Self::Teal => "teal",
            Self::Indigo => "indigo",
            Self::Slate => "slate",
            Self::Paper => "paper",
        }
    }
}
