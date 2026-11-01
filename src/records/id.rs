/// Stable random identifiers shared by records and folders.
pub(crate) fn new() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("operating system random generator");
    format!("{:032x}", u128::from_be_bytes(bytes))
}

/// Canonical IDs used by shared entities and attachments.
pub(crate) fn is_valid(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
