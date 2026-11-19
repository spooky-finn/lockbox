//! Local appearance preferences, independent of vault contents.

use serde::{Deserialize, Deserializer, Serialize};

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
    pub gradient: bool,
    pub corners: CornerStyle,
    #[serde(deserialize_with = "deserialize_color")]
    pub accent: Option<RgbColor>,
    #[serde(deserialize_with = "deserialize_color")]
    pub background: Option<RgbColor>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CornerStyle {
    Square,
    Rounded,
    #[default]
    #[serde(other)]
    Standard,
}

impl CornerStyle {
    pub const ALL: [Self; 3] = [Self::Standard, Self::Square, Self::Rounded];

    pub fn code(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Square => "square",
            Self::Rounded => "rounded",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThemePreset {
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
        Self::Ocean,
        Self::Forest,
        Self::Plum,
        Self::Amber,
        Self::Rose,
        Self::Teal,
        Self::Indigo,
        Self::Slate,
        Self::Paper,
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

    pub fn colors(self, dark: bool) -> (Option<RgbColor>, Option<RgbColor>) {
        let (accent, background) = match (self, dark) {
            (Self::Classic, _) => return (None, None),
            (Self::Ocean, false) => ([0, 99, 170], [239, 246, 252]),
            (Self::Ocean, true) => ([83, 185, 245], [18, 30, 43]),
            (Self::Forest, false) => ([25, 112, 68], [240, 247, 241]),
            (Self::Forest, true) => ([100, 210, 147], [20, 34, 27]),
            (Self::Plum, false) => ([132, 56, 157], [249, 242, 250]),
            (Self::Plum, true) => ([215, 155, 240], [35, 24, 40]),
            (Self::Amber, false) => ([140, 78, 0], [255, 244, 221]),
            (Self::Amber, true) => ([250, 198, 108], [48, 37, 24]),
            (Self::Rose, false) => ([171, 52, 89], [255, 240, 243]),
            (Self::Rose, true) => ([247, 155, 179], [54, 30, 40]),
            (Self::Teal, false) => ([0, 107, 104], [234, 248, 246]),
            (Self::Teal, true) => ([102, 223, 209], [16, 46, 44]),
            (Self::Indigo, false) => ([72, 62, 172], [240, 239, 255]),
            (Self::Indigo, true) => ([172, 167, 255], [34, 36, 63]),
            (Self::Slate, false) => ([75, 96, 115], [237, 240, 242]),
            (Self::Slate, true) => ([178, 191, 203], [38, 42, 48]),
            (Self::Paper, false) => ([151, 63, 50], [239, 229, 208]),
            (Self::Paper, true) => ([232, 161, 145], [41, 37, 31]),
        };
        (Some(RgbColor(accent)), Some(RgbColor(background)))
    }
}

impl Appearance {
    /// Custom colors override the corresponding preset colors.
    pub fn colors(self, dark: bool) -> (Option<RgbColor>, Option<RgbColor>) {
        let (accent, background) = self.preset.colors(dark);
        (self.accent.or(accent), self.background.or(background))
    }
}

/// A validated RGB color; arbitrary preference strings never become CSS.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "String")]
pub struct RgbColor([u8; 3]);

impl RgbColor {
    pub fn parse(value: &str) -> Option<Self> {
        let digits = value.strip_prefix('#')?;
        if digits.len() != 6 || !digits.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let number = u32::from_str_radix(digits, 16).ok()?;
        Some(Self([
            (number >> 16) as u8,
            (number >> 8) as u8,
            number as u8,
        ]))
    }

    pub fn hex(self) -> String {
        let [red, green, blue] = self.0;
        format!("#{red:02x}{green:02x}{blue:02x}")
    }

    pub fn rgb(self) -> [u8; 3] {
        self.0
    }
}

impl From<RgbColor> for String {
    fn from(color: RgbColor) -> Self {
        color.hex()
    }
}

fn deserialize_color<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<RgbColor>, D::Error> {
    // An invalid saved color falls back without discarding other preferences.
    Ok(Option::<String>::deserialize(deserializer)?.and_then(|value| RgbColor::parse(&value)))
}
