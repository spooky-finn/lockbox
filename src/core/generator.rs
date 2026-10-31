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
    #[error("выберите хотя бы один набор символов")]
    NoCharacters,
    #[error("длина должна быть от 8 до 128 символов")]
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
mod tests {
    use super::*;

    #[test]
    fn default_password_has_the_length_and_only_allowed_characters() {
        let password = generate(&Options::default()).unwrap();
        assert_eq!(password.chars().count(), 24);
        let allowed = format!("{LOWER}{UPPER}{DIGITS}{SYMBOLS}");
        assert!(password.chars().all(|c| allowed.contains(c)));
    }

    #[test]
    fn every_chosen_set_is_present() {
        let options = Options {
            length: 8,
            ..Options::default()
        };
        for _ in 0..500 {
            let password = generate(&options).unwrap();
            for set in [LOWER, UPPER, DIGITS, SYMBOLS] {
                assert!(password.chars().any(|c| set.contains(c)), "{set}");
            }
        }
    }

    #[test]
    fn ambiguous_characters_can_be_avoided() {
        let options = Options {
            length: 128,
            avoid_ambiguous: true,
            ..Options::default()
        };
        for _ in 0..100 {
            let password = generate(&options).unwrap();
            assert!(!password.chars().any(|c| AMBIGUOUS.contains(c)));
        }
    }

    #[test]
    fn single_set_gives_only_its_characters() {
        let options = Options {
            lower: false,
            upper: false,
            symbols: false,
            ..Options::default()
        };
        assert!(
            generate(&options)
                .unwrap()
                .chars()
                .all(|c| c.is_ascii_digit())
        );
    }

    #[test]
    fn invalid_options_are_rejected() {
        let none = Options {
            lower: false,
            upper: false,
            digits: false,
            symbols: false,
            ..Options::default()
        };
        assert_eq!(generate(&none).unwrap_err(), GeneratorError::NoCharacters);
        for length in [7, 129] {
            let options = Options {
                length,
                ..Options::default()
            };
            assert_eq!(
                generate(&options).unwrap_err(),
                GeneratorError::InvalidLength
            );
        }
    }

    #[test]
    fn entropy_follows_length_and_alphabet() {
        let options = Options::default();
        assert_eq!(options.alphabet_size(), 26 + 26 + 10 + 28);
        assert!((options.entropy_bits() - 24.0 * 90f64.log2()).abs() < 1e-9);
        let fewer = Options {
            avoid_ambiguous: true,
            ..options
        };
        assert_eq!(fewer.alphabet_size(), 90 - AMBIGUOUS.len());
    }

    /// Loose check against bias: 100 000 digits, each within ±7 % of the expected count.
    #[test]
    fn characters_are_spread_evenly() {
        let options = Options {
            length: 100,
            lower: false,
            upper: false,
            symbols: false,
            ..Options::default()
        };
        let mut counts = [0usize; 10];
        for _ in 0..1000 {
            for c in generate(&options).unwrap().chars() {
                counts[c.to_digit(10).unwrap() as usize] += 1;
            }
        }
        for count in counts {
            assert!((9_300..=10_700).contains(&count), "{counts:?}");
        }
    }
}
