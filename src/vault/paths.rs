//! Default paths and allocation policy for local desktop profiles.

pub(crate) fn new_profile_path(home: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let directory = private_root(home)?;
    let mut random = [0u8; 16];
    getrandom::fill(&mut random).map_err(std::io::Error::other)?;
    let id = u128::from_be_bytes(random);
    Ok(directory.join(format!("profile-{id:032x}.lbx")))
}

/// `~/.lockbox`, private to its owner. Older versions created it world-readable,
/// so an existing directory loses group and other access; the owner keeps all.
pub(crate) fn private_root(home: &std::path::Path) -> std::io::Result<std::path::PathBuf> {
    let directory = home.join(".lockbox");
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        builder.mode(0o700);
        builder.create(&directory)?;
        let mode = std::fs::metadata(&directory)?.permissions().mode();
        if mode & 0o077 != 0 {
            std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(mode & 0o700))?;
        }
    }
    #[cfg(not(unix))]
    builder.create(&directory)?;
    Ok(directory)
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn profile_root_is_private_and_existing_roots_are_tightened() {
        let home = tempfile::tempdir().unwrap();
        let mode =
            |path: &std::path::Path| std::fs::metadata(path).unwrap().permissions().mode() & 0o777;

        let root = super::private_root(home.path()).unwrap();
        assert_eq!(mode(&root), 0o700);

        std::fs::set_permissions(&root, std::fs::Permissions::from_mode(0o755)).unwrap();
        let profile = super::new_profile_path(home.path()).unwrap();
        assert_eq!(profile.parent(), Some(root.as_path()));
        assert_eq!(mode(&root), 0o700);
    }
}
