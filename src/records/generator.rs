//! Random passwords from the operating system's random generator.

use std::ops::RangeInclusive;

use zeroize::Zeroizing;

pub const LENGTH: RangeInclusive<usize> = 8..=128;

const LOWER: &str = "abcdefghijklmnopqrstuvwxyz";
const UPPER: &str = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const DIGITS: &str = "0123456789";
/// No quotes, backslash, backtick or space: they break pasting into shells and forms.
const SYMBOLS: &str = "!#$%&()*+,-./:;<=>?@[]^_{|}~";
/// Characters easy to confuse when a password is read or typed by hand.
pub const AMBIGUOUS: &str = "0Oo1lI|";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Options {
    pub length: usize,
    pub lower: bool,
    pub upper: bool,
    pub digits: bool,
    pub symbols: bool,
    pub avoid_ambiguous: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            length: 24,
            lower: true,
            upper: true,
            digits: true,
            symbols: true,
            avoid_ambiguous: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum GeneratorError {
    #[error("select at least one character set")]
    NoCharacters,
    #[error("length must be between 8 and 128 characters")]
    InvalidLength,
}

impl Options {
    /// The chosen character sets, without ambiguous characters if requested.
    fn sets(&self) -> Vec<Vec<char>> {
        [
            (self.lower, LOWER),
            (self.upper, UPPER),
            (self.digits, DIGITS),
            (self.symbols, SYMBOLS),
        ]
        .into_iter()
        .filter(|(chosen, _)| *chosen)
        .map(|(_, set)| {
            set.chars()
                .filter(|c| !(self.avoid_ambiguous && AMBIGUOUS.contains(*c)))
                .collect()
        })
        .collect()
    }

    /// Counts the available characters after applying the ambiguity filter.
    pub fn alphabet_size(&self) -> usize {
        self.sets().iter().map(Vec::len).sum()
    }

    /// Strength of a uniformly random password with these options.
    pub fn entropy_bits(&self) -> f64 {
        match self.alphabet_size() {
            0 => 0.0,
            size => self.length as f64 * (size as f64).log2(),
        }
    }
}

/// A password with at least one character from every chosen set, in random order.
pub fn generate(options: &Options) -> Result<Zeroizing<String>, GeneratorError> {
    if !LENGTH.contains(&options.length) {
        return Err(GeneratorError::InvalidLength);
    }

    let sets = options.sets();

    if sets.is_empty() {
        return Err(GeneratorError::NoCharacters);
    }

    let alphabet: Vec<char> = sets.iter().flatten().copied().collect();
    let mut chars = Zeroizing::new(Vec::with_capacity(options.length));

    for set in &sets {
        chars.push(set[uniform(set.len())]);
    }

    while chars.len() < options.length {
        chars.push(alphabet[uniform(alphabet.len())]);
    }
    // Fisher–Yates, so the guaranteed characters do not sit at the start.
    for i in (1..chars.len()).rev() {
        chars.swap(i, uniform(i + 1));
    }

    Ok(Zeroizing::new(chars.iter().collect()))
}

/// Uniform in `0..bound`: values from the incomplete last range are redrawn
/// instead of folded in with `%`, which would favour small results.
fn uniform(bound: usize) -> usize {
    let bound = u32::try_from(bound).expect("alphabets are small");
    let limit = u32::MAX - u32::MAX % bound;
    loop {
        let mut bytes = [0u8; 4];
        getrandom::fill(&mut bytes).expect("operating system random generator");
        let value = u32::from_le_bytes(bytes);

        if value < limit {
            return (value % bound) as usize;
        }
    }
}

#[cfg(test)]
#[path = "tests/generator.rs"]
mod tests;
