/// Stable random identifiers shared by records and folders.
pub(crate) fn new() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("operating system random generator");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Canonical IDs used by shared entities and attachments.
pub(crate) fn is_valid(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
