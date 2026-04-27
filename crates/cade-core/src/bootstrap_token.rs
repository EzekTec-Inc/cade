//! Shared API bootstrap-token helpers.
//!
//! The CADE server and CLI both need to read (and on first launch, create) a
//! persistent local-auth token.  Having the helper live in `cade-core` avoids
//! a cyclic dependency between `cade-cli` and `cade-server`.

use std::path::{Path, PathBuf};

/// Default location of the persistent API token file.
/// Returns `~/.cade/api-token` when a home directory is available.
pub fn default_token_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(".cade").join("api-token"))
}

/// Load the API token from `path`, creating it with a fresh random value when
/// it does not yet exist.
///
/// On Unix the file is created with mode `0o600`; the parent directory is
/// created with mode `0o700` when missing.  The returned token is the hex
/// encoding of 32 random bytes when freshly generated.
///
/// Returns an error only when the filesystem operation fails.  Callers who
/// want a softer "read-only" behaviour can use [`read_existing_token`].
pub fn load_or_create_token(path: &Path) -> std::io::Result<String> {
    if let Some(token) = read_existing_token(path) {
        return Ok(token);
    }

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
        && !parent.exists()
    {
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(parent)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::create_dir_all(parent)?;
        }
    }

    let mut bytes = [0u8; 32];
    getrandom::getrandom(&mut bytes)
        .map_err(|e| std::io::Error::other(format!("getrandom failed: {e}")))?;
    let token = hex_encode(&bytes);

    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(path)?;
        f.write_all(token.as_bytes())?;
    }
    #[cfg(not(unix))]
    {
        std::fs::write(path, token.as_bytes())?;
    }

    Ok(token)
}

/// Read an existing token from disk.  Returns `None` when the file is missing,
/// unreadable, or effectively empty (only whitespace).
pub fn read_existing_token(path: &Path) -> Option<String> {
    let contents = std::fs::read_to_string(path).ok()?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn token_created_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        assert!(!path.exists());

        let token = load_or_create_token(&path).expect("should create token");
        assert!(path.exists());
        assert_eq!(token.len(), 64);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));

        let on_disk = fs::read_to_string(&path).unwrap();
        assert_eq!(on_disk.trim(), token);
    }

    #[test]
    fn token_reused_on_second_call() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");

        let first = load_or_create_token(&path).unwrap();
        let second = load_or_create_token(&path).unwrap();
        assert_eq!(first, second);
    }

    #[test]
    fn token_creates_parent_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("subdir").join("api-token");
        assert!(!path.parent().unwrap().exists());

        let token = load_or_create_token(&path).unwrap();
        assert!(path.exists());
        assert_eq!(token.len(), 64);
    }

    #[cfg(unix)]
    #[test]
    fn token_file_has_mode_0600() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        load_or_create_token(&path).unwrap();

        let mode = fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }

    #[test]
    fn read_existing_returns_none_when_empty() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        fs::write(&path, "   \n").unwrap();
        assert!(read_existing_token(&path).is_none());
    }

    #[test]
    fn read_existing_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("api-token");
        fs::write(&path, "  deadbeef  \n").unwrap();
        assert_eq!(read_existing_token(&path).unwrap(), "deadbeef");
    }
}
