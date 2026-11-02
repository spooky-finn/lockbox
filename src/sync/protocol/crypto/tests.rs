use super::*;

#[test]
fn content_keys_match_concatenated_info_for_both_domains() {
    for seed in [0u8, 1, 127, 128, 254, 255] {
        let key = std::array::from_fn(|i| seed.wrapping_add((i as u8).wrapping_mul(73)));
        let space = Hash(std::array::from_fn(|i| seed ^ (i as u8).wrapping_mul(41)));
        let context = Hash(std::array::from_fn(|i| {
            seed.wrapping_mul(29).wrapping_sub(i as u8)
        }));

        for checkpoint in [false, true] {
            let domain = if checkpoint {
                b"lockbox.checkpoint\0".as_slice()
            } else {
                b"lockbox.changes\0".as_slice()
            };
            let info = [domain, &context.0].concat();
            let mut expected = Zeroizing::new([0u8; 32]);
            Hkdf::<Sha256>::new(Some(&space.0), &key)
                .expand(&info, expected.as_mut())
                .unwrap();

            assert_eq!(*content_key(&key, space, context, checkpoint), *expected);
        }

        assert_ne!(
            *content_key(&key, space, context, false),
            *content_key(&key, space, context, true)
        );
    }
}
