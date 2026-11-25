//! Small, self-contained vault marks. Shared images contain only raster pixels.

use super::Blob;
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

pub const SPACE_ICON_SIZE: u32 = 64;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpaceGlyph {
    Folder,
    Cloud,
    Key,
    User,
    Sparkles,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SpaceIcon {
    Glyph(SpaceGlyph),
    /// Straight-alpha RGBA8, row-major, from top left.
    Image(Blob),
}

impl SpaceIcon {
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Glyph(_) => true,
            Self::Image(bytes) => bytes.0.len() == (SPACE_ICON_SIZE * SPACE_ICON_SIZE * 4) as usize,
        }
    }
}

impl Drop for SpaceIcon {
    fn drop(&mut self) {
        if let Self::Image(bytes) = self {
            bytes.0.zeroize();
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "icon",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum SpaceIconChange {
    Set(SpaceIcon),
    Clear,
}

impl SpaceIconChange {
    pub(super) fn icon(&self) -> Option<&SpaceIcon> {
        match self {
            Self::Set(icon) => Some(icon),
            Self::Clear => None,
        }
    }
}
