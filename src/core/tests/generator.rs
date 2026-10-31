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
