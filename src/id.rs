/// Stable random identifiers shared by records and folders.
pub(crate) fn new() -> String {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).expect("operating system random generator");
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
