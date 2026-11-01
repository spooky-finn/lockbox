//! Default paths and allocation policy for local desktop profiles.

pub(crate) fn new_profile_path(home: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let directory = home.join(".lockbox");
    std::fs::create_dir_all(&directory)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
    let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
    Ok(directory.join(format!("profile-{name}.lbx")))
}
